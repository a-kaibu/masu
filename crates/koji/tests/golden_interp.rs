//! The interpreter's golden: seeded random programs (tests/golden/mod.rs) run step by step in
//! 32-bit mode (x87 in doubles and in 80 bits, with and without the decode cache), long mode
//! and 16-bit mode; the state after every step (exit, registers, flags, x87, MMX, SSE/AVX,
//! MXCSR) and the memory written at the end are hashed, GROUP programs to a line of
//! testdata/golden/interp.txt. Pins today's behaviour for rewrites that must not change it.

mod golden;
use golden::*;
use koji::{step_cached, Cpu, DecodeCache, Exit};
use std::sync::atomic::Ordering::Relaxed;

const PER_MODE: usize = 1024;
const STEPS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    I32,
    I32X80,
    I32Cached,
    I64,
    I16,
}
const MODES: [Mode; 5] = [Mode::I32, Mode::I32X80, Mode::I32Cached, Mode::I64, Mode::I16];

fn hash_exit(h: &mut Fnv, e: Exit) {
    match e {
        Exit::Continue => { h.u8(0); }
        Exit::Trap(a) => {
            h.u8(1);
            h.u32(a);
        }
        Exit::Fault(f) => {
            h.u8(2);
            h.u32(f.addr);
            h.u8(u8::from(f.write));
        }
        Exit::DivideError => { h.u8(3); }
        Exit::InvalidOpcode => { h.u8(4); }
        Exit::Interrupt(n) => {
            h.u8(5);
            h.u8(n);
        }
        Exit::Syscall => { h.u8(6); }
        Exit::Halt => { h.u8(7); }
        Exit::SingleStep => { h.u8(8); }
        Exit::SimdException => { h.u8(9); }
    }
}

fn hash_cpu(h: &mut Fnv, c: &Cpu) {
    for i in 0..16 {
        h.u64(c.gpr(koji::Reg64::ALL[i]));
    }
    h.u32(c.eip);
    h.u32(c.rip_hi);
    h.u32(c.flags());
    h.u32(c.fs_base);
    h.u32(c.gs_base);
    for i in 0..17 {
        let [lo, hi] = c.ymm(i);
        h.u128(lo);
        h.u128(hi);
    }
    for m in c.mm {
        h.u64(m);
    }
    h.u32(c.mxcsr);
    let f = &c.fpu;
    h.u16(f.cw);
    h.u16(f.status());
    h.u16(f.tag_word());
    for i in 0..8 {
        h.bytes(&f.st80(i).to_bytes());
    }
    for (s, b) in c.sreg.values().into_iter().zip(c.sbase.values()) {
        h.u16(s);
        h.u32(b);
    }
}

fn setup(mode: Mode, n: usize) -> (Cpu, Mem) {
    let prog = program(0x6b6f_6a69, n);
    let regs = regs(0x6b6f_6a69, n);
    let mut cpu = Cpu::default();
    let mut flags_r = Rng(n64(n) ^ 0xf1a9);
    // (status flags and DF random; IF and bit 1 set)
    cpu.eflags = 0x202 | (u32::try_from(flags_r.next() & 0x4d5).unwrap());
    match mode {
        Mode::I32 | Mode::I32X80 | Mode::I32Cached => {
            for (i, r) in regs.iter().take(8).enumerate() {
                cpu.set_gpr(koji::Reg64::ALL[i], *r & 0xffff_ffff);
            }
            cpu.eip = CODE;
        }
        Mode::I64 => {
            cpu.mode64 = 1;
            for (i, r) in regs.iter().enumerate() {
                cpu.set_gpr(koji::Reg64::ALL[i], *r);
            }
            cpu.eip = CODE;
        }
        Mode::I16 => {
            cpu.mode16 = 1;
            cpu.ldt = koji::LDT_REAL;
            for (i, r) in regs.iter().take(8).enumerate() {
                cpu.set_gpr(koji::Reg64::ALL[i], *r & 0xffff);
            }
            // ES CS SS DS FS GS, real-mode style (base = selector * 16)
            cpu.sreg = koji::Segs::new([0x1800, u16::try_from(CODE >> 4).unwrap(), 0x2000, 0x1800, 0x1800, 0x1800]);
            for i in koji::Seg::ALL {
                cpu.sbase[i] = u32::from(cpu.sreg[i]) << 4;
            }
            cpu.eip = 0;
        }
    }
    let sp = if mode == Mode::I16 { 0xe000 } else { u64::from(STACK_TOP - 0x100) };
    cpu.set_gpr(koji::Reg64::Rsp, sp);
    let mut x = Rng(n64(n) ^ 0x5353);
    for i in 0..16 {
        cpu.set_xmm(i, u128::from(x.next()) | u128::from(x.next()) << 64);
    }
    for m in &mut cpu.mm {
        *m = x.next();
    }
    // (some x87 values on the stack: FLD of small integers)
    for k in 0..x.below(5) {
        cpu.fpu.push(f64::from(u32::try_from(k).unwrap()) * 1.5 - 2.0);
    }
    let at = if mode == Mode::I16 { CODE } else { cpu.eip };
    (cpu, Mem::new(&prog, at))
}

fn run(mode: Mode, n: usize, cache: &mut DecodeCache) -> u64 {
    let (mut cpu, mut mem) = setup(mode, n);
    let mut h = Fnv::new();
    for _ in 0..STEPS {
        let at = if mode == Mode::I16 { cpu.pc16() } else { cpu.eip };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| step_cached(&mut cpu, &mut mem, if mode == Mode::I32Cached { Some(&mut *cache) } else { None })));
        let Ok((e, flow)) = r else {
            let o = off(at);
            panic!("{mode:?} program {n}: the interpreter panicked at {at:#x} on {:02x?}", &mem.b[o..o + 15]);
        };
        hash_exit(&mut h, e);
        h.u8(u8::from(flow));
        hash_cpu(&mut h, &cpu);
        if e != Exit::Continue {
            break;
        }
    }
    mem.hash(&mut h);
    h.0
}

#[test]
fn interpreter_golden() {
    let show = std::env::var("MASU_GOLDEN_GROUP").ok().and_then(|s| s.parse::<usize>().ok());
    let mut lines = Vec::new();
    let mut cache = DecodeCache::new();
    for mode in MODES {
        koji::fpu::X87_80.store(mode == Mode::I32X80, Relaxed);
        for g in 0..PER_MODE / GROUP {
            let mut h = Fnv::new();
            for n in g * GROUP..(g + 1) * GROUP {
                let one = run(mode, n, &mut cache);
                if show == Some(g) {
                    eprintln!("{mode:?} {n} {one:016x} {:02x?}", program(0x6b6f_6a69, n));
                }
                h.u64(one);
            }
            lines.push(format!("{mode:?}/{g} {:016x}", h.0));
        }
    }
    koji::fpu::X87_80.store(false, Relaxed);
    check("interp.txt", &lines);
}
