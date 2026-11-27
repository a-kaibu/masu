//! The programs measured: 32- and 64-bit code made with iced's assembler at CODE, data at DATA, each
//! returning to END (the return address on the stack) after `scale` times its fixed work.
//! Shapes of what the games' code spends its time in (docs/bench.md's cpu-game): table-driven
//! loops, carry chains, small calls, a bytecode VM's jump table, integer SSE2, x87, sorting,
//! and memory operands between flag writers (the exits that must keep the flags).
use super::{CODE, DATA};
use iced_x86::code_asm::*;
use iced_x86::{BlockEncoderOptions, IcedError};
use koji::Gpr;

pub const NAMES: &[&str] = &["crc32", "bigadd", "calls", "vm", "sse_int", "x87_dot", "sort", "flags_mem", "avx2_int"];

pub struct Prog {
    pub long: bool,
    pub code: Vec<u8>,
    pub data: Vec<u8>,
    pub regs: Vec<(Gpr, u32)>,
}

const BUF: u32 = DATA + 0x1_0000;
const BUF_LEN: u32 = 0x1_0000;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as u32
    }
}

fn data(n: usize) -> Vec<u8> {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    (0..n).map(|_| r.next() as u8).collect()
}

fn put32(d: &mut [u8], at: u32, v: u32) {
    let at = (at - DATA) as usize;
    d[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// Times each program's work is repeated at scale 1: ~20-60 ms of translated code in node 24
const REPEAT: [u32; 9] = [200, 500, 200, 50, 500, 1000, 100, 1000, 500];

pub fn build(i: usize, scale: u32) -> Option<Prog> {
    let scale = scale * REPEAT.get(i)?;
    let r = match i {
        0 => crc32(scale),
        1 => bigadd(scale),
        2 => calls(scale),
        3 => vm(scale),
        4 => sse_int(scale),
        5 => x87_dot(scale),
        6 => sort(scale),
        7 => flags_mem(scale),
        8 => avx2_int(scale),
        _ => return None,
    };
    r.ok()
}

fn assemble(a: &mut CodeAssembler) -> Result<Vec<u8>, IcedError> {
    a.assemble(u64::from(CODE))
}

/// CRC-32 of 64 KiB, by table (movzx, xor with the table entry, shift)
fn crc32(scale: u32) -> Result<Prog, IcedError> {
    let mut d = data(0x2_0000 + BUF_LEN as usize);
    for n in 0..256u32 {
        let mut c = n;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        put32(&mut d, DATA + 4 * n, c);
    }
    let mut a = CodeAssembler::new(32)?;
    let mut outer = a.create_label();
    let mut top = a.create_label();
    a.set_label(&mut outer)?;
    a.mov(esi, BUF)?;
    a.mov(ecx, BUF_LEN)?;
    a.mov(eax, 0xffff_ffffu32)?;
    a.set_label(&mut top)?;
    a.movzx(edx, byte_ptr(esi))?;
    a.xor(dl, al)?;
    a.shr(eax, 8)?;
    a.xor(eax, dword_ptr(edx * 4 + DATA))?;
    a.inc(esi)?;
    a.dec(ecx)?;
    a.jne(top)?;
    a.add(edi, eax)?;
    a.dec(ebp)?;
    a.jne(outer)?;
    a.ret()?;
    Ok(Prog { long: false, code: assemble(&mut a)?, data: d, regs: vec![(koji::EBP, scale), (koji::EDI, 0)] })
}

/// 4096-word additions with carry (ADC chained through INC/DEC, which keep CF)
fn bigadd(scale: u32) -> Result<Prog, IcedError> {
    let d = data(0x4_0000);
    let (x, y, z) = (DATA, DATA + 0x1_0000, DATA + 0x2_0000);
    let mut a = CodeAssembler::new(32)?;
    let mut outer = a.create_label();
    let mut top = a.create_label();
    a.set_label(&mut outer)?;
    a.xor(ecx, ecx)?;
    a.mov(ebx, 4096)?;
    a.clc()?;
    a.set_label(&mut top)?;
    a.mov(eax, dword_ptr(ecx * 4 + x))?;
    a.adc(eax, dword_ptr(ecx * 4 + y))?;
    a.mov(dword_ptr(ecx * 4 + z), eax)?;
    a.inc(ecx)?;
    a.dec(ebx)?;
    a.jne(top)?;
    a.adc(edi, 0)?;
    a.dec(ebp)?;
    a.jne(outer)?;
    a.ret()?;
    Ok(Prog { long: false, code: assemble(&mut a)?, data: d, regs: vec![(koji::EBP, scale), (koji::EDI, 0)] })
}

/// Small calls: a loop calling a leaf through a middle function (cdecl, a stack argument)
fn calls(scale: u32) -> Result<Prog, IcedError> {
    let mut a = CodeAssembler::new(32)?;
    let mut outer = a.create_label();
    let mut top = a.create_label();
    let mut mid = a.create_label();
    let mut leaf = a.create_label();
    let mut done = a.create_label();
    a.set_label(&mut outer)?;
    a.mov(ecx, 20000)?;
    a.set_label(&mut top)?;
    a.push(ecx)?;
    a.call(mid)?;
    a.add(esp, 4)?;
    a.add(edi, eax)?;
    a.dec(ecx)?;
    a.jne(top)?;
    a.dec(ebp)?;
    a.jne(outer)?;
    a.jmp(done)?;
    a.set_label(&mut mid)?;
    a.push(ebx)?;
    a.mov(ebx, dword_ptr(esp + 8))?;
    a.lea(eax, ptr(ebx + ebx * 2 + 1))?;
    a.call(leaf)?;
    a.xor(eax, ebx)?;
    a.pop(ebx)?;
    a.ret()?;
    a.set_label(&mut leaf)?;
    a.imul_3(eax, eax, 0x9e37)?;
    a.shr(eax, 3)?;
    a.ret()?;
    a.set_label(&mut done)?;
    a.ret()?;
    Ok(Prog { long: false, code: assemble(&mut a)?, data: data(0x1000), regs: vec![(koji::EBP, scale), (koji::EDI, 0)] })
}

/// A bytecode VM (a script engine's shape): a byte opcode, a jump table, a value stack in
/// memory; the program sums and mixes a counter, 8 opcodes
fn vm(scale: u32) -> Result<Prog, IcedError> {
    // opcodes: 0 push imm8, 1 add, 2 xor, 3 dup, 4 dec the value below the top and jump back
    // imm8 bytes unless it got 0, 5 drop (into EDI's sum), 6 shl the top by 1, 7 halt
    let bytecode: Vec<u8> = vec![
        0, 200, // the counter
        0, 1, // the value
        // loop (offset 4): value = ((value ^ value << 1) + 7), 3 pushed and dropped
        3, 6, 2, 0, 7, 1, 0, 3, 5, //
        4, 11, // back to offset 4 (the operand read, ESI is at 15)
        5, 5, 7,
    ];
    let mut d = data(0x2000);
    let table = DATA;
    let prog = DATA + 0x100;
    let vstack = DATA + 0x1000;
    let code_at = (prog - DATA) as usize;
    d[code_at..code_at + bytecode.len()].copy_from_slice(&bytecode);
    let mut a = CodeAssembler::new(32)?;
    let mut outer = a.create_label();
    let mut next = a.create_label();
    let mut ops: Vec<CodeLabel> = (0..8).map(|_| a.create_label()).collect();
    let mut done = a.create_label();
    a.set_label(&mut outer)?;
    a.mov(esi, prog)?;
    a.mov(ebx, vstack)?;
    a.set_label(&mut next)?;
    a.movzx(eax, byte_ptr(esi))?;
    a.inc(esi)?;
    a.jmp(dword_ptr(eax * 4 + table))?;
    // push imm8
    a.set_label(&mut ops[0])?;
    a.movzx(eax, byte_ptr(esi))?;
    a.inc(esi)?;
    a.add(ebx, 4)?;
    a.mov(dword_ptr(ebx), eax)?;
    a.jmp(next)?;
    // add
    a.set_label(&mut ops[1])?;
    a.mov(eax, dword_ptr(ebx))?;
    a.sub(ebx, 4)?;
    a.add(dword_ptr(ebx), eax)?;
    a.jmp(next)?;
    // xor
    a.set_label(&mut ops[2])?;
    a.mov(eax, dword_ptr(ebx))?;
    a.sub(ebx, 4)?;
    a.xor(dword_ptr(ebx), eax)?;
    a.jmp(next)?;
    // dup
    a.set_label(&mut ops[3])?;
    a.mov(eax, dword_ptr(ebx))?;
    a.add(ebx, 4)?;
    a.mov(dword_ptr(ebx), eax)?;
    a.jmp(next)?;
    // the value below the top less 1, back by imm8 when not 0
    a.set_label(&mut ops[4])?;
    a.movzx(eax, byte_ptr(esi))?;
    a.inc(esi)?;
    a.dec(dword_ptr(ebx - 4))?;
    a.je(next)?;
    a.sub(esi, eax)?;
    a.jmp(next)?;
    // drop
    a.set_label(&mut ops[5])?;
    a.mov(eax, dword_ptr(ebx))?;
    a.add(edi, eax)?;
    a.sub(ebx, 4)?;
    a.jmp(next)?;
    // shl1
    a.set_label(&mut ops[6])?;
    a.shl(dword_ptr(ebx), 1)?;
    a.jmp(next)?;
    // halt
    a.set_label(&mut ops[7])?;
    a.dec(ebp)?;
    a.jne(outer)?;
    a.jmp(done)?;
    a.set_label(&mut done)?;
    a.ret()?;
    let r = a.assemble_options(u64::from(CODE), BlockEncoderOptions::RETURN_NEW_INSTRUCTION_OFFSETS)?;
    for (k, l) in ops.iter().enumerate() {
        put32(&mut d, table + 4 * k as u32, r.label_ip(l)? as u32);
    }
    let code = r.inner.code_buffer;
    Ok(Prog { long: false, code, data: d, regs: vec![(koji::EBP, scale * 40), (koji::EDI, 0)] })
}

/// Integer SSE2 over 64 KiB: loads, PADDD, PMULLW, PXOR, shifts, stores
fn sse_int(scale: u32) -> Result<Prog, IcedError> {
    let d = data(0x3_0000);
    let (x, y, z) = (DATA, DATA + 0x1_0000, DATA + 0x2_0000);
    let mut a = CodeAssembler::new(32)?;
    let mut outer = a.create_label();
    let mut top = a.create_label();
    a.pcmpeqd(xmm7, xmm7)?;
    a.set_label(&mut outer)?;
    a.xor(ecx, ecx)?;
    a.set_label(&mut top)?;
    a.movdqa(xmm0, xmmword_ptr(ecx + x))?;
    a.movdqa(xmm1, xmmword_ptr(ecx + y))?;
    a.movdqa(xmm2, xmm0)?;
    a.paddd(xmm0, xmm1)?;
    a.pmullw(xmm2, xmm1)?;
    a.pxor(xmm0, xmm2)?;
    a.psrld(xmm2, 3)?;
    a.pand(xmm2, xmm7)?;
    a.psubd(xmm0, xmm2)?;
    a.paddd(xmm6, xmm0)?;
    a.movdqa(xmmword_ptr(ecx + z), xmm0)?;
    a.add(ecx, 16)?;
    a.cmp(ecx, 0x1_0000)?;
    a.jb(top)?;
    a.dec(ebp)?;
    a.jne(outer)?;
    a.ret()?;
    Ok(Prog { long: false, code: assemble(&mut a)?, data: d, regs: vec![(koji::EBP, scale)] })
}

/// x87 dot product of 4096 doubles
fn x87_dot(scale: u32) -> Result<Prog, IcedError> {
    let mut d = vec![0u8; 0x1_0000];
    let mut r = Rng(7);
    for k in 0..8192 {
        let v = f64::from(r.next() % 1000) / 7.0;
        d[8 * k..8 * k + 8].copy_from_slice(&v.to_le_bytes());
    }
    let (x, y) = (DATA, DATA + 0x8000);
    let mut a = CodeAssembler::new(32)?;
    let mut outer = a.create_label();
    let mut top = a.create_label();
    a.set_label(&mut outer)?;
    a.xor(ecx, ecx)?;
    a.fldz()?;
    a.set_label(&mut top)?;
    a.fld(qword_ptr(ecx * 8 + x))?;
    a.fmul(qword_ptr(ecx * 8 + y))?;
    a.faddp(st1, st0)?;
    a.inc(ecx)?;
    a.cmp(ecx, 4096)?;
    a.jb(top)?;
    a.fstp(qword_ptr(DATA + 0x1_0000))?;
    a.dec(ebp)?;
    a.jne(outer)?;
    a.ret()?;
    Ok(Prog { long: false, code: assemble(&mut a)?, data: d, regs: vec![(koji::EBP, scale)] })
}

/// Insertion sort of 1024 ints, copied from the source first
fn sort(scale: u32) -> Result<Prog, IcedError> {
    let d = data(0x3000);
    let (src, arr) = (DATA, DATA + 0x1000);
    let mut a = CodeAssembler::new(32)?;
    let mut outer = a.create_label();
    let mut copy = a.create_label();
    let mut i_loop = a.create_label();
    let mut j_loop = a.create_label();
    let mut place = a.create_label();
    a.set_label(&mut outer)?;
    a.xor(ecx, ecx)?;
    a.set_label(&mut copy)?;
    a.mov(eax, dword_ptr(ecx * 4 + src))?;
    a.mov(dword_ptr(ecx * 4 + arr), eax)?;
    a.inc(ecx)?;
    a.cmp(ecx, 1024)?;
    a.jb(copy)?;
    a.mov(esi, 1)?;
    a.set_label(&mut i_loop)?;
    a.mov(eax, dword_ptr(esi * 4 + arr))?;
    a.mov(edx, esi)?;
    a.set_label(&mut j_loop)?;
    a.test(edx, edx)?;
    a.je(place)?;
    a.mov(ebx, dword_ptr(edx * 4 + (arr - 4)))?;
    a.cmp(ebx, eax)?;
    a.jle(place)?;
    a.mov(dword_ptr(edx * 4 + arr), ebx)?;
    a.dec(edx)?;
    a.jmp(j_loop)?;
    a.set_label(&mut place)?;
    a.mov(dword_ptr(edx * 4 + arr), eax)?;
    a.inc(esi)?;
    a.cmp(esi, 1024)?;
    a.jb(i_loop)?;
    a.dec(ebp)?;
    a.jne(outer)?;
    a.ret()?;
    Ok(Prog { long: false, code: assemble(&mut a)?, data: d, regs: vec![(koji::EBP, scale)] })
}

/// Memory operands between flag writers whose flags nothing reads (each access could fault:
/// the flags it would leave with are the previous writer's)
fn flags_mem(scale: u32) -> Result<Prog, IcedError> {
    let d = data(0x2_0000);
    let mut a = CodeAssembler::new(32)?;
    let mut outer = a.create_label();
    let mut top = a.create_label();
    a.set_label(&mut outer)?;
    a.mov(esi, DATA)?;
    a.mov(ecx, 0x4000)?;
    a.set_label(&mut top)?;
    a.add(eax, dword_ptr(esi))?;
    a.xor(edx, dword_ptr(esi + 4))?;
    a.sub(eax, edx)?;
    a.or(ebx, dword_ptr(esi + 8))?;
    a.and(ebx, eax)?;
    a.add(edi, dword_ptr(esi + 12))?;
    a.shl(edx, 1)?;
    a.add(esi, 4)?;
    a.dec(ecx)?;
    a.jne(top)?;
    a.dec(ebp)?;
    a.jne(outer)?;
    a.ret()?;
    Ok(Prog { long: false, code: assemble(&mut a)?, data: d, regs: vec![(koji::EBP, scale)] })
}

/// An AVX2 integer transform over 64 KiB, keeping independent data in each half of a YMM.
fn avx2_int(scale: u32) -> Result<Prog, IcedError> {
    let source = DATA;
    let dest = DATA + BUF_LEN;
    let mut a = CodeAssembler::new(64)?;
    let mut outer = a.create_label();
    let mut inner = a.create_label();
    a.set_label(&mut outer)?;
    a.mov(esi, source)?;
    a.mov(edi, dest)?;
    a.mov(ecx, BUF_LEN / 32)?;
    a.set_label(&mut inner)?;
    a.vmovdqu(ymm0, ymmword_ptr(rsi))?;
    a.vmovdqu(ymm1, ymmword_ptr(rdi))?;
    a.vpaddd(ymm2, ymm0, ymm1)?;
    a.vpmullw(ymm3, ymm2, ymm0)?;
    a.vpxor(ymm4, ymm3, ymm1)?;
    a.vpsrld(ymm5, ymm4, 5)?;
    a.vpand(ymm6, ymm5, ymm2)?;
    a.vpsubd(ymm7, ymm6, ymm3)?;
    a.vmovdqu(ymmword_ptr(rdi), ymm7)?;
    a.add(esi, 32)?;
    a.add(edi, 32)?;
    a.dec(ecx)?;
    a.jne(inner)?;
    a.dec(ebp)?;
    a.jne(outer)?;
    a.ret()?;
    Ok(Prog { long: true, code: assemble(&mut a)?, data: data(2 * BUF_LEN as usize), regs: vec![(koji::EBP, scale)] })
}
