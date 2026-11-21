//! What the x86 JIT targets (x86_jit, x86_jit64, x86_jit_hle) share: the guest memory window,
//! the wasmtime engine with koji's dispatcher, and the "do both runs end up the same" check.
use koji::jit;
use koji::{Cpu, Exit, Fault, Memory};
use std::cell::UnsafeCell;
use std::sync::OnceLock;
use wasmtime::{Config, Engine, Module, OptLevel};

/// Guest memory is [LO, HI): code at LO, data above it.
pub const LO: u32 = 0x10_0000;
pub const HI: u32 = 0x14_0000;

/// [LO, HI) as the interpreter's memory: anything outside it faults.
pub struct Window(pub Vec<u8>);
impl Memory for Window {
    fn read8(&mut self, a: u32) -> Result<u8, Fault> {
        if (LO..HI).contains(&a) { Ok(self.0[(a - LO) as usize]) } else { Err(Fault { addr: a, write: false }) }
    }
    fn write8(&mut self, a: u32, v: u8) -> Result<(), Fault> {
        if (LO..HI).contains(&a) {
            self.0[(a - LO) as usize] = v;
            Ok(())
        } else {
            Err(Fault { addr: a, write: true })
        }
    }
}

/// The wasm memory as guest memory for the `step` helper (the JIT's own view: all of it, but
/// only the window is reached: a step outside it is an interpreter fault too, never compared).
/// `wrote` sees each store first (x86_jit64 marks stores into the code).
pub struct Wasm<'a>(pub &'a [UnsafeCell<u8>], pub fn(u32));
impl Memory for Wasm<'_> {
    fn read8(&mut self, a: u32) -> Result<u8, Fault> {
        if (LO..HI).contains(&a) { Ok(unsafe { *self.0[a as usize].get() }) } else { Err(Fault { addr: a, write: false }) }
    }
    fn write8(&mut self, a: u32, v: u8) -> Result<(), Fault> {
        if (LO..HI).contains(&a) {
            (self.1)(a);
            unsafe { *self.0[a as usize].get() = v };
            Ok(())
        } else {
            Err(Fault { addr: a, write: true })
        }
    }
}

/// One engine (threads, shared memory, SIMD, tail calls; no optimization: the targets run many
/// small modules) and koji's dispatcher compiled in it.
pub fn engine() -> &'static (Engine, Module) {
    static E: OnceLock<(Engine, Module)> = OnceLock::new();
    E.get_or_init(|| {
        let mut c = Config::new();
        c.wasm_threads(true).shared_memory(true).wasm_simd(true).wasm_tail_call(true).cranelift_opt_level(OptLevel::None);
        let e = Engine::new(&c).unwrap();
        let d = Module::new(&e, jit::dispatch::module()).unwrap();
        (e, d)
    })
}

/// Runs both states on in the interpreter (up to 64 instructions, or to where either stops):
/// whether they end the same by `same` (without the status flags). Status flags that differ and
/// were read make the runs part (a branch, a SETcc, an ADC); ones still different at the end
/// were never read.
pub fn converge(a: &Cpu, am: &[u8], b: &Cpu, bm: &[u8], same: impl Fn(&Cpu, &Cpu, bool) -> bool) -> bool {
    let (mut a, mut b) = (a.clone(), b.clone());
    let (mut am, mut bm) = (Window(am.to_vec()), Window(bm.to_vec()));
    for _ in 0..64 {
        let (x, y) = (koji::step(&mut a, &mut am), koji::step(&mut b, &mut bm));
        if x != y {
            return false;
        }
        if x != Exit::Continue {
            // (an exception: the handler sees the flags as they are, which the JIT does not
            // keep where nothing after reads them)
            return same(&a, &b, false) && am.0 == bm.0;
        }
    }
    a.eip == b.eip && same(&a, &b, false) && am.0 == bm.0
}
