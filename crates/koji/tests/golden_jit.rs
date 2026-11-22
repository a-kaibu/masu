//! The JIT's golden: the seeded random programs of tests/golden/mod.rs translated by every
//! translator (32-bit regions and functions, long-mode regions and functions, 16-bit regions);
//! the wasm module's bytes and what the Region reports (blocks, code ranges, instruction count)
//! are hashed, GROUP programs to a line of testdata/golden/jit.txt. A rewrite meant to change
//! nothing must emit the same wasm, byte for byte (that the wasm *runs* the same as the
//! interpreter is tools/jit{16,32,64}test's job).

mod golden;
use golden::*;
use koji::jit::{self, wasm::Module, Region};

const PER_MODE: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tr {
    Region32,
    Function32,
    Region64,
    Function64,
    Region16,
}
const TRS: [Tr; 5] = [Tr::Region32, Tr::Function32, Tr::Region64, Tr::Function64, Tr::Region16];

fn translate(t: Tr, n: usize) -> u64 {
    let prog = program(0x6a69_7421, n);
    let mut mem = Mem::new(&prog, CODE);
    let mut m = Module::default();
    let r: Option<Region> = match t {
        Tr::Region32 => jit::translate_region(&mut m, &mut mem, &[CODE], 16, |_| true),
        Tr::Function32 => jit::translate_function(&mut m, &mut mem, CODE, 16, 4000, (0, 0), &|_| None, &|_| true, |_| true),
        Tr::Region64 => jit::long::translate_region64(&mut m, &mut mem, &[u64::from(CODE)], (0, 0), |_, _| true),
        Tr::Function64 => jit::long::translate_function64(&mut m, &mut mem, u64::from(CODE), (0, 0), 4000, |_, _| true),
        Tr::Region16 => jit::translate_region16(&mut m, &mut mem, CODE, CODE, 16, |_| true),
    };
    let mut h = Fnv::new();
    match r {
        None => { h.u8(0); }
        Some(r) => {
            h.u8(1);
            for (a, i) in &r.starts {
                h.u32(*a);
                h.u32(*i);
            }
            h.u64(n64(r.instructions));
            for (a, l) in &r.code {
                h.u32(*a);
                h.u32(*l);
            }
            match &r.text {
                None => { h.u8(0); }
                Some(t) => {
                    h.u8(1);
                    h.bytes(t);
                }
            }
            for s in &r.sites {
                h.u32(*s);
            }
            h.u8(u8::from(r.ctx.exact()));
            h.bytes(&m.finish());
        }
    }
    h.0
}

#[test]
fn jit_golden() {
    // (the generator instruction by instruction, and with the SSA runs: jit::ssa)
    for (ssa, file) in [(false, "jit.txt"), (true, "jit_ssa.txt")] {
        jit::SSA.store(ssa, std::sync::atomic::Ordering::Relaxed);
        golden_of(file);
    }
}

fn golden_of(file: &str) {
    let show = std::env::var("MASU_GOLDEN_GROUP").ok().and_then(|s| s.parse::<usize>().ok());
    let mut lines = Vec::new();
    for t in TRS {
        for g in 0..PER_MODE / GROUP {
            let mut h = Fnv::new();
            for n in g * GROUP..(g + 1) * GROUP {
                let one = translate(t, n);
                if show == Some(g) {
                    eprintln!("{t:?} {n} {one:016x} {:02x?}", program(0x6a69_7421, n));
                }
                h.u64(one);
            }
            lines.push(format!("{t:?}/{g} {:016x}", h.0));
        }
    }
    check(file, &lines);
}
