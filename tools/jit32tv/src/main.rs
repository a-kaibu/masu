//! Translation validation of koji's 32-bit JIT (crates/koji/src/jit/mod.rs): for short integer
//! sequences (ADD ADC SUB SBB CMP, AND OR XOR TEST, INC DEC NEG NOT, SHL SHR SAR ROL ROR by an
//! immediate and by CL, MOV MOVZX MOVSX, on 8-bit (low and high), 16-bit and 32-bit
//! registers), the wasm koji generates for the region is read back and run on terms
//! (wasm.rs), and x86's semantics written apart from koji (x86.rs, from the SDM) is run on the
//! same initial registers and EFLAGS. Z3 is asked, for every path of the wasm, whether any input
//! makes the two differ in a register, EIP or a flag the SDM defines after the sequence. The
//! flags the wasm leaves in Cpu.lazy are read by the record's contract (koji::lazy::Kind: an
//! Add record's flags are those of a + b, ...), worked out with x86.rs's formulas; that
//! Lazy::status itself keeps the contract is koji's own tests' business.
//!
//! Each case ends as one of:
//!   differs     an input where they differ (printed with Z3's model: add it to jit32test)
//!   proven      no input makes them differ, on every path (for these registers, flags, EIP)
//!   undecided   a path calls an import (the interpreter's step, the eflags import: what runs
//!               there is not this wasm), an operator or address this executor does not
//!               follow, or Z3 timed out
//!
//!   cargo run --release -- [--strict] [filter]     (Z3: the z3 on PATH, or $Z3)
//! Exits 1 on a difference (and, with --strict, on an undecided case).
#![cfg_attr(not(test), warn(dead_code_pub_in_binary))]
use iced_x86::code_asm::*;
use iced_x86::{Decoder, DecoderOptions, Instruction, Mnemonic, OpKind, Register};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::Ordering::Relaxed;

mod expr;
mod wasm;
mod x86;

use expr::{self as x, E};

const CPU: u32 = 0x2000;
const CODE: u32 = 0x10_0000;
const END: u32 = 0x18_0000;
const SIDE_EXIT: u32 = END + 0x100;
const MEM: usize = 0x40_0000;
const BUDGET: u64 = 1000;

struct Mem(Vec<u8>);
impl koji::Memory for Mem {
    fn read8(&mut self, a: u32) -> Result<u8, koji::Fault> {
        self.0.get(a as usize).copied().ok_or(koji::Fault { addr: a, write: false })
    }
    fn write8(&mut self, a: u32, v: u8) -> Result<(), koji::Fault> {
        match self.0.get_mut(a as usize) {
            Some(b) => {
                *b = v;
                Ok(())
            }
            None => Err(koji::Fault { addr: a, write: true }),
        }
    }
}

type Emit = fn(&mut CodeAssembler) -> Result<(), IcedError>;

/// The cases: (name, the sequence)
fn cases() -> Vec<(String, Result<Vec<Instruction>, String>)> {
    let mut out = Vec::new();
    let mut push = |name: String, emit: &dyn Fn(&mut CodeAssembler) -> Result<(), IcedError>| {
        let assembled = CodeAssembler::new(32).and_then(|mut a| {
            emit(&mut a)?;
            a.assemble(u64::from(CODE))
        });
        out.push((name, assembled.map(|bytes| Decoder::with_ip(32, &bytes, u64::from(CODE), DecoderOptions::NONE).iter().collect()).map_err(|e| e.to_string())));
    };
    macro_rules! each_alu {
        ($($m:ident),*) => {$(
            push(format!("{} eax, ebx", stringify!($m)), &|a| a.$m(eax, ebx));
            push(format!("{} ecx, ecx", stringify!($m)), &|a| a.$m(ecx, ecx));
            push(format!("{} esi, 0x80000000", stringify!($m)), &|a| a.$m(esi, 0x8000_0000u32 as i32));
            push(format!("{} edx, -1", stringify!($m)), &|a| a.$m(edx, -1));
            push(format!("{} ax, dx", stringify!($m)), &|a| a.$m(ax, dx));
            push(format!("{} di, 0x7fff", stringify!($m)), &|a| a.$m(di, 0x7fff));
            push(format!("{} al, bl", stringify!($m)), &|a| a.$m(al, bl));
            push(format!("{} ah, cl", stringify!($m)), &|a| a.$m(ah, cl));
            push(format!("{} bh, ch", stringify!($m)), &|a| a.$m(bh, ch));
            push(format!("{} dl, 0x7f", stringify!($m)), &|a| a.$m(dl, 0x7f));
        )*};
    }
    each_alu!(add, adc, sub, sbb, cmp, and, or, xor, test);
    macro_rules! each_un {
        ($($m:ident),*) => {$(
            push(format!("{} eax", stringify!($m)), &|a| a.$m(eax));
            push(format!("{} si", stringify!($m)), &|a| a.$m(si));
            push(format!("{} cl", stringify!($m)), &|a| a.$m(cl));
            push(format!("{} dh", stringify!($m)), &|a| a.$m(dh));
        )*};
    }
    each_un!(inc, dec, neg, not);
    macro_rules! each_shift {
        ($($m:ident),*) => {$(
            push(format!("{} eax, 1", stringify!($m)), &|a| a.$m(eax, 1));
            push(format!("{} ebx, 7", stringify!($m)), &|a| a.$m(ebx, 7));
            push(format!("{} edx, 31", stringify!($m)), &|a| a.$m(edx, 31));
            push(format!("{} eax, cl", stringify!($m)), &|a| a.$m(eax, cl));
            push(format!("{} si, cl", stringify!($m)), &|a| a.$m(si, cl));
            push(format!("{} bx, 3", stringify!($m)), &|a| a.$m(bx, 3));
            push(format!("{} al, cl", stringify!($m)), &|a| a.$m(al, cl));
            push(format!("{} ah, 1", stringify!($m)), &|a| a.$m(ah, 1));
            push(format!("{} dl, 5", stringify!($m)), &|a| a.$m(dl, 5));
        )*};
    }
    each_shift!(shl, shr, sar, rol, ror);
    let ext: [(&str, Emit); 8] = [
        ("movzx eax, bl", |a| a.movzx(eax, bl)),
        ("movzx ecx, ah", |a| a.movzx(ecx, ah)),
        ("movzx edx, si", |a| a.movzx(edx, si)),
        ("movzx di, cl", |a| a.movzx(di, cl)),
        ("movsx eax, bh", |a| a.movsx(eax, bh)),
        ("movsx ebx, dx", |a| a.movsx(ebx, dx)),
        ("movsx si, al", |a| a.movsx(si, al)),
        ("mov ah, bl", |a| a.mov(ah, bl)),
    ];
    for (n, e) in ext {
        push(n.into(), &e);
    }
    // (sequences: a carry chained, flags of one read by the next, partial writes merged)
    let seq: [(&str, Emit); 10] = [
        ("add eax, ecx; adc edx, ebx", |a| { a.add(eax, ecx)?; a.adc(edx, ebx) }),
        ("sub eax, ecx; sbb edx, ebx", |a| { a.sub(eax, ecx)?; a.sbb(edx, ebx) }),
        ("cmp eax, ebx; adc ecx, 0", |a| { a.cmp(eax, ebx)?; a.adc(ecx, 0) }),
        ("shl eax, 1; adc ebx, ebx", |a| { a.shl(eax, 1)?; a.adc(ebx, ebx) }),
        ("inc eax; adc ebx, 0", |a| { a.inc(eax)?; a.adc(ebx, 0) }),
        ("mov al, bl; mov ah, cl; add ax, dx", |a| { a.mov(al, bl)?; a.mov(ah, cl)?; a.add(ax, dx) }),
        ("xor eax, eax; sub al, 1", |a| { a.xor(eax, eax)?; a.sub(al, 1) }),
        ("and ecx, 31; shr edx, cl", |a| { a.and(ecx, 31)?; a.shr(edx, cl) }),
        ("sar eax, cl; sbb ebx, ebx", |a| { a.sar(eax, cl)?; a.sbb(ebx, ebx) }),
        ("rol eax, 8; ror bx, cl", |a| { a.rol(eax, 8)?; a.ror(bx, cl) }),
    ];
    for (n, e) in seq {
        push(n.into(), &e);
    }
    push("test eax, eax; jz side; xor edx, edx".into(), &|a| {
        a.test(eax, eax)?;
        a.jz(u64::from(SIDE_EXIT))?;
        a.xor(edx, edx)
    });
    push("cmp eax, ebx; jb side; add ax, dx".into(), &|a| {
        a.cmp(eax, ebx)?;
        a.jb(u64::from(SIDE_EXIT))?;
        a.add(ax, dx)
    });
    push("cmp eax, ebx; mov ecx, [0] (before)".into(), &|a| {
        a.cmp(eax, ebx)?;
        a.mov(ecx, dword_ptr(0))
    });
    push("xor esp, esp; xor edx, edx (after)".into(), &|a| {
        a.xor(esp, esp)?;
        a.xor(edx, edx)
    });
    out
}

enum Verdict {
    Differs(String),
    Proven(usize),
    Undecided(String),
}

fn translate(code: &[u8]) -> Result<(Vec<u8>, u32), String> {
    let mut mem = Mem(vec![0; MEM]);
    let at = CODE as usize;
    mem.0[at..at + code.len()].copy_from_slice(code);
    koji::jit::FN.store(false, Relaxed);
    let mut m = koji::jit::wasm::Module::default();
    let region = koji::jit::translate_region(&mut m, &mut mem, &[CODE], 2, |a| !matches!(a, END | SIDE_EXIT)).ok_or("koji refused the region")?;
    let &(_, block) = region.starts.iter().find(|&&(at, _)| at == CODE).ok_or("missing region entry")?;
    Ok((m.finish(), block << koji::jit::ENTRY_SHIFT))
}

/// the bytes of a 4- or 8-byte field of the Cpu in memory
fn field(mem: &BTreeMap<u32, E>, at: u32, n: u32) -> E {
    let byte = |a: u32| mem.get(&a).cloned().unwrap_or_else(|| x::c(8, 0));
    let mut v = byte(at);
    for i in 1..n {
        v = x::concat(&byte(at + i), &v);
    }
    v
}

/// The flags a Cpu's lazy record and EFLAGS stand for, by koji::lazy::Kind's contract.
fn materialize(op: &E, c: &E, a: &E, b: &E, r: &E, eflags: &E) -> Result<x86::State, String> {
    use koji::lazy::{Kind, Op, Size};
    let regs = vec![x::c(32, 0); 8];
    let mut s = x86::State::new(regs, eflags);
    let op = x::konst(op).ok_or("a lazy op the paths do not fix")?;
    if op == 0 {
        return Ok(s);
    }
    let Some(Op { kind, size }) = Op::decode(op as u32) else {
        return Err(format!("lazy op {op} names no kind"));
    };
    let w = match size {
        Size::B8 => 8,
        Size::B16 => 16,
        Size::B32 => 32,
        Size::B64 => return Err("a 64-bit record from the 32-bit JIT".into()),
    };
    let (a, b, r) = (x::extract(a, w - 1, 0), x::extract(b, w - 1, 0), x::extract(r, w - 1, 0));
    let carry = x::nonzero(c);
    let zero = x::c(1, 0);
    let mut flags_of = |m: Mnemonic, a: &E, b: &E, carry: &E| -> Result<(), String> {
        let ins = iced_x86::Instruction::with2(code_of(m, w), reg_of(w, 0), reg_of(w, 1)).map_err(|e| e.to_string())?;
        let mut t = x86::State::new(vec![x::c(32, 0); 8], eflags);
        t.cf = x86::Flag { v: carry.clone(), defined: x::c(1, 1) };
        t.regs[0] = x::zext(a, 32);
        t.regs[1] = x::zext(b, 32);
        x86::step(&mut t, &ins).ok_or("no reference for the record")?;
        s = t;
        Ok(())
    };
    match kind {
        Kind::Add => flags_of(Mnemonic::Add, &a, &b, &zero)?,
        Kind::Adc => flags_of(Mnemonic::Adc, &a, &b, &carry)?,
        Kind::Sub => flags_of(Mnemonic::Sub, &a, &b, &zero)?,
        Kind::Sbb => flags_of(Mnemonic::Sbb, &a, &b, &carry)?,
        Kind::Inc | Kind::Dec => {
            let m = if kind == Kind::Inc { Mnemonic::Add } else { Mnemonic::Sub };
            flags_of(m, &a, &x::c(w, 1), &zero)?;
            s.cf = x86::Flag { v: carry, defined: x::c(1, 1) };
        }
        Kind::Shl | Kind::Shr | Kind::Sar => {
            let m = match kind { Kind::Shl => Mnemonic::Shl, Kind::Shr => Mnemonic::Shr, _ => Mnemonic::Sar };
            flags_of(m, &a, &b, &zero)?;
        }
        Kind::Logic | Kind::Mul => {
            flags_of(Mnemonic::Test, &r, &r, &zero)?;
            if kind == Kind::Mul {
                s.cf = x86::Flag { v: carry.clone(), defined: x::c(1, 1) };
                s.of = x86::Flag { v: carry, defined: x::c(1, 1) };
            }
            s.af = x86::Flag { v: zero, defined: x::c(1, 1) };
        }
    }
    Ok(s)
}

/// an instruction code for `m` reg, reg at width w (for the record's reference)
fn code_of(m: Mnemonic, w: u32) -> iced_x86::Code {
    use iced_x86::Code::*;
    match (m, w) {
        (Mnemonic::Add, 8) => Add_r8_rm8, (Mnemonic::Add, 16) => Add_r16_rm16, (Mnemonic::Add, _) => Add_r32_rm32,
        (Mnemonic::Adc, 8) => Adc_r8_rm8, (Mnemonic::Adc, 16) => Adc_r16_rm16, (Mnemonic::Adc, _) => Adc_r32_rm32,
        (Mnemonic::Sub, 8) => Sub_r8_rm8, (Mnemonic::Sub, 16) => Sub_r16_rm16, (Mnemonic::Sub, _) => Sub_r32_rm32,
        (Mnemonic::Sbb, 8) => Sbb_r8_rm8, (Mnemonic::Sbb, 16) => Sbb_r16_rm16, (Mnemonic::Sbb, _) => Sbb_r32_rm32,
        (Mnemonic::Test, 8) => Test_rm8_r8, (Mnemonic::Test, 16) => Test_rm16_r16, (Mnemonic::Test, _) => Test_rm32_r32,
        (Mnemonic::Shl, 8) => Shl_rm8_CL, (Mnemonic::Shl, 16) => Shl_rm16_CL, (Mnemonic::Shl, _) => Shl_rm32_CL,
        (Mnemonic::Shr, 8) => Shr_rm8_CL, (Mnemonic::Shr, 16) => Shr_rm16_CL, (Mnemonic::Shr, _) => Shr_rm32_CL,
        (Mnemonic::Sar, 8) => Sar_rm8_CL, (Mnemonic::Sar, 16) => Sar_rm16_CL, (_, _) => Sar_rm32_CL,
    }
}

fn reg_of(w: u32, i: usize) -> iced_x86::Register {
    use iced_x86::Register::*;
    match (w, i) {
        (8, 0) => AL, (8, _) => CL, (16, 0) => AX, (16, _) => CX, (_, 0) => EAX, (_, _) => ECX,
    }
}

const REGS: [&str; 8] = ["EAX", "ECX", "EDX", "EBX", "ESP", "EBP", "ESI", "EDI"];
const FLAGS: [&str; 6] = ["CF", "PF", "AF", "ZF", "SF", "OF"];

fn flags(s: &x86::State) -> [&x86::Flag; 6] {
    [&s.cf, &s.pf, &s.af, &s.zf, &s.sf, &s.of]
}

fn z3(query: &str) -> Result<String, String> {
    let z3 = std::env::var("Z3").unwrap_or_else(|_| "z3".into());
    let mut child = std::process::Command::new(&z3)
        .args(["-T:60", "-in"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{z3}: {e}"))?;
    use std::io::Write as _;
    child.stdin.take().ok_or("stdin")?.write_all(query.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// sat (Some(model)), unsat (None), or neither (Err)
fn sat(smt: &mut x::Smt, assert: &E) -> Result<Option<String>, String> {
    let a = smt.name(assert);
    let mut q = smt.text.clone();
    let _ = writeln!(q, "(assert (= {a} #b1))\n(check-sat)");
    let vars: Vec<String> = smt.vars.iter().map(|(n, _)| n.clone()).collect();
    if !vars.is_empty() {
        let _ = writeln!(q, "(get-value ({}))", vars.join(" "));
    }
    let r = z3(&q)?;
    match r.lines().next().unwrap_or("") {
        "unsat" => Ok(None),
        "sat" => Ok(Some(r.lines().skip(1).collect::<Vec<_>>().join(" "))),
        other => Err(format!("z3: {other}")),
    }
}

struct ReferenceExit {
    at: u32,
    status: i32,
    when: E,
    state: x86::State,
}

/// Follow a case's fallthrough symbolically, recording each taken side exit and the final END.
fn reference_exits(seq: &[Instruction], mut state: x86::State) -> Result<Vec<ReferenceExit>, String> {
    let mut exits = Vec::new();
    let mut when = x::c(1, 1);
    for ins in seq {
        let flag = match ins.mnemonic() {
            Mnemonic::Je => &state.zf,
            Mnemonic::Jb => &state.cf,
            Mnemonic::Mov if ins.op1_kind() == OpKind::Memory && ins.memory_base() == Register::None
                && ins.memory_index() == Register::None && ins.memory_displacement64() == 0 => {
                // Address zero is below the guest range. The load has not executed.
                exits.push(ReferenceExit { at: ins.ip32(), status: koji::jit::RET_INTERP, when, state });
                return Ok(exits);
            }
            _ => {
                x86::step(&mut state, ins).ok_or_else(|| format!("no reference for {ins}"))?;
                if ins.mnemonic() == Mnemonic::Xor && ins.op0_register() == Register::ESP && ins.op1_register() == Register::ESP {
                    // The stack is now zero, so the JIT returns to the dispatcher immediately.
                    exits.push(ReferenceExit { at: ins.next_ip32(), status: koji::jit::RET_INTERP, when, state });
                    return Ok(exits);
                }
                continue;
            }
        };
        if ins.near_branch32() != SIDE_EXIT || x::konst(&flag.defined) != Some(1) {
            return Err(format!("no reference for the target or condition of {ins}"));
        }
        exits.push(ReferenceExit { at: SIDE_EXIT, status: koji::jit::RET_CONTINUE, when: x::band(&when, &flag.v), state: state.clone() });
        when = x::band(&when, &x::bnot(&flag.v));
    }
    exits.push(ReferenceExit { at: END, status: koji::jit::RET_CONTINUE, when, state });
    Ok(exits)
}

fn check(seq: &[Instruction]) -> Verdict {
    let mut code = Vec::new();
    let mut a = match CodeAssembler::new(32) {
        Ok(a) => a,
        Err(e) => return Verdict::Undecided(e.to_string()),
    };
    for i in seq {
        if a.add_instruction(*i).is_err() {
            return Verdict::Undecided("assembles".into());
        }
    }
    if a.jmp(u64::from(END)).is_err() {
        return Verdict::Undecided("jmp".into());
    }
    match a.assemble(u64::from(CODE)) {
        Ok(b) => code.extend(b),
        Err(e) => return Verdict::Undecided(e.to_string()),
    }
    let (bytes, entry) = match translate(&code) {
        Ok(translation) => translation,
        Err(e) => return Verdict::Undecided(e),
    };
    let f = match wasm::parse(&bytes) {
        Ok(f) => f,
        Err(e) => return Verdict::Undecided(format!("reading the wasm: {e}")),
    };
    let regs: Vec<E> = REGS.iter().map(|r| x::var(&r.to_lowercase(), 32)).collect();
    let eflags = x::var("eflags", 32);
    // the region's arguments: the Cpu, encoded entry, the 8 registers, the lazy record and budget
    let mut params = vec![x::c(32, u64::from(CPU)), x::c(32, u64::from(entry))];
    params.extend(regs.iter().cloned());
    while params.len() < f.nparams {
        params.push(x::c(32, 0));
    }
    if let Some(p) = params.get_mut(15) {
        *p = x::c(32, BUDGET);
    }
    params.truncate(f.nparams);
    let eflags_at = CPU + core::mem::offset_of!(koji::Cpu, eflags) as u32;
    let lazy_at = CPU + core::mem::offset_of!(koji::Cpu, lazy) as u32;
    let regs_at = CPU + core::mem::offset_of!(koji::Cpu, regs) as u32;
    let eip_at = CPU + core::mem::offset_of!(koji::Cpu, eip) as u32;
    let budget_at = CPU + core::mem::offset_of!(koji::Cpu, jit_budget) as u32;
    let mut mem = BTreeMap::new();
    for i in 0..4 {
        mem.insert(eflags_at + i, x::extract(&eflags, 8 * i + 7, 8 * i));
        mem.insert(budget_at + i, x::c(8, BUDGET >> (8 * i)));
    }
    // the reference
    let references = match reference_exits(seq, x86::State::new(regs, &eflags)) {
        Ok(exits) => exits,
        Err(e) => return Verdict::Undecided(e),
    };
    let paths = f.run(params, mem, 64);
    let mut proven = 0;
    let mut undecided = Vec::new();
    for p in paths {
        let mut smt = x::Smt::new();
        let all = |cond: &[E]| cond.iter().fold(x::c(1, 1), |a, b| x::band(&a, b));
        match p {
            wasm::End::Return { cond, mem, results } => {
                let got = |at: u32| field(&mem, at, 4);
                let lazy = |off: usize, n: u32| field(&mem, lazy_at + off as u32, n);
                let w = match materialize(&lazy(0, 4), &lazy(4, 4), &lazy(8, 8), &lazy(16, 8), &lazy(24, 8), &got(eflags_at)) {
                    Ok(w) => w,
                    Err(e) => {
                        undecided.push(e);
                        continue;
                    }
                };
                let wasm_when = all(&cond);
                let mut complete = true;
                for expected in &references {
                    let when = x::band(&wasm_when, &expected.when);
                    // The dispatcher sees the target EIP, return status, and guest state.
                    let ret = match results.as_slice() {
                        [r] => x::bnot(&x::eq(r, &x::c(32, expected.status as u32 as u64))),
                        _ => x::c(1, 1),
                    };
                    let mut outs = vec![("EIP", x::bnot(&x::eq(&got(eip_at), &x::c(32, u64::from(expected.at))))), ("status", ret)];
                    for ((at, want), name) in (0u32..).map(|i| regs_at + 4 * i).zip(&expected.state.regs).zip(REGS) {
                        outs.push((name, x::bnot(&x::eq(&got(at), want))));
                    }
                    for ((want, have), name) in flags(&expected.state).iter().zip(flags(&w)).zip(FLAGS) {
                        outs.push((name, x::band(&want.defined, &x::bnot(&x::eq(&want.v, &have.v)))));
                    }
                    let differ = outs.iter().fold(x::c(1, 0), |a, (_, d)| x::bor(&a, d));
                    match sat(&mut x::Smt::new(), &x::band(&when, &differ)) {
                        Ok(None) => {}
                        Ok(Some(model)) => {
                            let which: Vec<&str> = outs.iter().filter(|(_, d)| matches!(sat(&mut x::Smt::new(), &x::band(&when, d)), Ok(Some(_)))).map(|(n, _)| *n).collect();
                            let actual = x::konst(&got(eip_at)).map_or_else(|| "symbolic EIP".into(), |at| format!("{at:#x}"));
                            return Verdict::Differs(format!("exit {:#x} (JIT {actual}): {} at {model}", expected.at, which.join(" ")));
                        }
                        Err(e) => {
                            undecided.push(e);
                            complete = false;
                        }
                    }
                }
                proven += usize::from(complete);
            }
            wasm::End::Trap { cond, why } => match sat(&mut smt, &all(&cond)) {
                Ok(None) => proven += 1,
                Ok(Some(model)) => return Verdict::Differs(format!("traps ({why}) at {model}")),
                Err(e) => undecided.push(e),
            },
            wasm::End::Opaque { cond, why } => match sat(&mut smt, &all(&cond)) {
                Ok(None) => proven += 1,
                Ok(Some(_)) | Err(_) => undecided.push(why),
            },
        }
    }
    if undecided.is_empty() {
        Verdict::Proven(proven)
    } else {
        undecided.sort();
        undecided.dedup();
        Verdict::Undecided(undecided.join("; "))
    }
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let strict = args.iter().any(|a| a == "--strict");
    // (--expect FILE: the cases known to be undecided, one name per line, # comments: a case
    // undecided and not listed, or listed and decided now, fails the run, so the list is kept
    // exact both ways)
    let expect_at = args.iter().position(|a| a == "--expect").and_then(|i| args.get(i + 1)).cloned();
    let expected: Option<std::collections::BTreeSet<String>> = match &expect_at {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(t) => Some(t.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect()),
            Err(e) => {
                eprintln!("jit32tv: {p}: {e}");
                return std::process::ExitCode::FAILURE;
            }
        },
        None => None,
    };
    let filter = args.iter().enumerate().find(|(i, a)| !a.starts_with("--") && args.get(i.wrapping_sub(1)).is_none_or(|p| p != "--expect")).map_or_default(|(_, a)| a.clone());
    let (mut differs, mut proven, mut undecided) = (0, 0, std::collections::BTreeSet::new());
    let mut seen = std::collections::BTreeSet::new();
    for (name, seq) in cases() {
        if !name.contains(&filter) {
            continue;
        }
        seen.insert(name.clone());
        let verdict = match seq {
            Ok(seq) => check(&seq),
            Err(e) => Verdict::Undecided(format!("assembling the case: {e}")),
        };
        match verdict {
            Verdict::Proven(n) => {
                proven += 1;
                println!("proven     {name} ({n} paths)");
            }
            Verdict::Differs(m) => {
                differs += 1;
                println!("DIFFERS    {name}: {m}");
            }
            Verdict::Undecided(why) => {
                println!("undecided  {name}: {why}");
                undecided.insert(name);
            }
        }
    }
    println!("jit32tv: {proven} proven, {differs} differ, {} undecided", undecided.len());
    let mut fail = differs > 0 || (strict && !undecided.is_empty());
    if let (Some(want), Some(p)) = (&expected, &expect_at) {
        for n in undecided.difference(want) {
            println!("jit32tv: {n} is undecided and not in {p}");
            fail = true;
        }
        for n in want.iter().filter(|n| seen.contains(*n) || filter.is_empty()).filter(|n| !undecided.contains(*n)) {
            println!("jit32tv: {n} is in {p} but {}: take it out", if seen.contains(n) { "decided now" } else { "no case of that name" });
            fail = true;
        }
    }
    if fail { std::process::ExitCode::FAILURE } else { std::process::ExitCode::SUCCESS }
}
