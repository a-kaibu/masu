//! koji's 32-bit JIT measured in V8 (see README.md): this crate is koji built to wasm32 the way
//! kura is, in the one memory every module shares; bench.mjs loads the modules it makes as
//! host/vnjit.js does and calls `jb_*`. The guest's memory is the wasm memory itself (addresses
//! [GUEST_LO, LIMIT)); this crate's own data and heap are above 2 GiB (`--global-base`, see
//! build.sh), out of the guest's range.
//!
//! The run loop is kura's (crates/kura/src/jit.rs `run`) in short: the dispatcher module runs
//! the translations; a miss counts, and the interpreter runs to the next branch until an address
//! missed THRESHOLD times, then the function there is translated (`translate_function_fit`,
//! with the addresses the interpreter ran as `ran`) and loaded.
use koji::jit::{self, wasm::Module};
use koji::{Cpu, Exit, Fault};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering::Relaxed;

mod progs;

const CPU: u32 = 0x2000;
const HASH: u32 = 0x8000;
const HASH_N: u32 = 1 << 14;
const RING: u32 = 0x3_0000;
/// the call cache (Cpu::jit_calls) and the code page bits (Cpu::jit_code_pages)
const CALLS: u32 = 0x7000_0000;
const PAGES: u32 = 0x7100_0000;
const CODE: u32 = 0x10_0000;
/// the address a program's end jumps to
pub const END: u32 = 0x18_0000;
pub const DATA: u32 = 0x20_0000;
pub const STACK: u32 = 0x0f00_0000;
/// guest memory the interpreter accepts: [0x40000, LIMIT)
const LIMIT: u32 = 0x1000_0000;
const THRESHOLD: u32 = 50;

#[link(wasm_import_module = "jb")]
unsafe extern "C" {
    /// the dispatcher's f0 (koji::jit::dispatch) on the Cpu at `cpu`
    fn jb_dispatch(cpu: u32) -> i32;
    /// compiles and instantiates the module at `ptr` (WebAssembly.Module, as vnjit.js), its `n`
    /// functions into the table: the first's index
    fn jb_load(ptr: u32, len: u32, n: u32) -> i32;
    /// performance.now()
    fn jb_now() -> f64;
}

struct Linear;
impl koji::Memory for Linear {
    fn read8(&mut self, a: u32) -> Result<u8, Fault> {
        if !(0x40000..LIMIT).contains(&a) {
            return Err(Fault { addr: a, write: false });
        }
        // SAFETY: [0x40000, LIMIT) is the guest's part of this module's memory (build.sh puts
        // everything of this crate above 2 GiB)
        Ok(unsafe { *(a as usize as *const u8) })
    }
    fn write8(&mut self, a: u32, v: u8) -> Result<(), Fault> {
        if !(0x40000..LIMIT).contains(&a) {
            return Err(Fault { addr: a, write: true });
        }
        // SAFETY: as read8
        unsafe { *(a as usize as *mut u8) = v };
        Ok(())
    }
}

fn cpu() -> &'static mut Cpu {
    // SAFETY: Cpu is plain data at CPU (below the guest's range, never handed out twice at once:
    // one thread, each caller done with it before the next call into wasm)
    unsafe { &mut *(CPU as usize as *mut Cpu) }
}

/// Counters, read by jb_stat (in its order)
#[derive(Default, Clone, Copy)]
struct Stats {
    translate_ms: f64,
    translations: f64,
    wasm_bytes: f64,
    guest_ins: f64,
    interp_steps: f64,
    step_calls: f64,
    exit_interp: f64,
    exit_continue: f64,
    dispatches: f64,
    load_fail: f64,
}

#[derive(Default)]
struct State {
    misses: HashMap<u32, u32>,
    known: HashSet<u32>,
    ran: HashSet<u32>,
    stats: Stats,
    end_ok: bool,
}

thread_local! {
    static ST: RefCell<State> = RefCell::new(State::default());
}

/// the modules' `step` calls since jb_setup
static STEP_CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Buffer the module bytes are handed out in (bench.mjs reads `jb_buf`'s pointer)
static mut OUT: Vec<u8> = Vec::new();

#[unsafe(no_mangle)]
pub extern "C" fn jb_cpu() -> u32 {
    CPU
}

#[unsafe(no_mangle)]
pub extern "C" fn jb_end() -> u32 {
    END
}

#[unsafe(no_mangle)]
pub extern "C" fn jb_dispatch_module() -> u32 {
    let m = jit::dispatch::module();
    let n = m.len() as u32;
    // SAFETY: one thread
    unsafe {
        let out = &mut *(&raw mut OUT);
        *out = m;
    }
    n
}

#[unsafe(no_mangle)]
pub extern "C" fn jb_buf() -> u32 {
    // SAFETY: one thread
    unsafe { (*(&raw const OUT)).as_ptr() as u32 }
}

/// the interpreter alone (no translations): the reference run
static NOJIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Knobs (koji's statics): 0 FN, 1 HOIST, 2 MEMCOUNT, 3 MX_EXACT, 4 the interpreter alone, 5 SSA
#[unsafe(no_mangle)]
pub extern "C" fn jb_set(knob: u32, v: u32) {
    let on = v != 0;
    match knob {
        4 => NOJIT.store(on, Relaxed),
        0 => jit::FN.store(on, Relaxed),
        1 => jit::HOIST.store(on, Relaxed),
        2 => jit::wasm::MEMCOUNT.store(on, Relaxed),
        3 => jit::MX_EXACT.store(on, Relaxed),
        5 => jit::SSA.store(on, Relaxed),
        _ => {}
    }
}

/// Programs: progs::NAMES; `scale` repeats their outer loop
#[unsafe(no_mangle)]
pub extern "C" fn jb_programs() -> u32 {
    progs::NAMES.len() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn jb_program_name(i: u32) -> u32 {
    let s = progs::NAMES.get(i as usize).copied().unwrap_or("");
    // SAFETY: one thread
    unsafe {
        let out = &mut *(&raw mut OUT);
        *out = s.as_bytes().to_vec();
    }
    s.len() as u32
}

/// Puts program `i` (its code, data and registers) in memory; the translations made of an
/// earlier setup of the same program stay (the code is the same bytes), of another are dropped
#[unsafe(no_mangle)]
pub extern "C" fn jb_setup(i: u32, scale: u32, keep: u32) -> u32 {
    let Some(p) = progs::build(i as usize, scale) else { return 0 };
    // SAFETY: the guest's memory, [CODE, CODE + len) and [DATA, DATA + len) inside it
    unsafe {
        std::ptr::write_bytes(0x40000 as *mut u8, 0, (LIMIT - 0x40000) as usize);
        std::ptr::copy_nonoverlapping(p.code.as_ptr(), CODE as usize as *mut u8, p.code.len());
        std::ptr::copy_nonoverlapping(p.data.as_ptr(), DATA as usize as *mut u8, p.data.len());
    }
    let c = cpu();
    *c = Cpu { eip: CODE, ..Cpu::default() };
    for (r, v) in p.regs {
        c.regs[r] = v;
    }
    c.regs[koji::ESP] = STACK - 0x100;
    // the guest's return address: END
    // SAFETY: the stack's top, in the guest's range
    unsafe { *((STACK - 0x100) as usize as *mut u32) = END };
    c.eflags = 0x202;
    if keep == 0 {
        // SAFETY: the hash table, the call cache and page bits: this crate's (below the guest)
        unsafe {
            std::ptr::write_bytes(HASH as usize as *mut u8, 0, (HASH_N * 8) as usize);
            std::ptr::write_bytes(CALLS as usize as *mut u8, 0, 0x200_0000);
            std::ptr::write_bytes(RING as usize as *mut u8, 0, 0x1_0000);
        }
        ST.with(|s| *s.borrow_mut() = State::default());
    }
    ST.with(|s| s.borrow_mut().stats = Stats::default());
    STEP_CALLS.store(0, Relaxed);
    p.code.len() as u32
}

/// Counter `k` of the last runs (since jb_setup), Stats' order
#[unsafe(no_mangle)]
pub extern "C" fn jb_stat(k: u32) -> f64 {
    ST.with(|s| {
        let mut s = s.borrow().stats;
        s.step_calls = f64::from(STEP_CALLS.load(Relaxed));
        [s.translate_ms, s.translations, s.wasm_bytes, s.guest_ins, s.interp_steps, s.step_calls, s.exit_interp, s.exit_continue, s.dispatches, s.load_fail].get(k as usize).copied().unwrap_or(-1.0)
    })
}

/// A checksum of the guest's state at the end: registers, flags, the data
#[unsafe(no_mangle)]
pub extern "C" fn jb_checksum() -> u32 {
    let c = cpu();
    let mut c2 = c.clone();
    c2.materialize();
    let mut h: u32 = 0x811c_9dc5;
    let mut mix = |v: u32| h = (h ^ v).wrapping_mul(0x0100_0193);
    for i in 0..8 {
        mix(c2.regs[koji::Gpr::ALL[i]]);
    }
    mix(c2.eflags & 0x8d5);
    for x in c2.xmm.iter().take(8) {
        mix(*x as u32);
        mix((*x >> 32) as u32);
        mix((*x >> 64) as u32);
        mix((*x >> 96) as u32);
    }
    // SAFETY: the data's 1 MiB, in the guest's range
    let d = unsafe { std::slice::from_raw_parts(DATA as usize as *const u32, 0x4_0000) };
    for &w in d {
        mix(w);
    }
    h
}

/// The guest helpers the modules import (vnjit.js's env: kura's vnhost_jit_*)
#[unsafe(no_mangle)]
pub extern "C" fn jb_step(_cpu: u32) -> i32 {
    // (inside jb_run's borrow of ST: a counter of its own)
    STEP_CALLS.fetch_add(1, Relaxed);
    let c = cpu();
    let at = c.eip;
    match koji::step(c, &mut Linear) {
        Exit::Continue if c.eip == at => 2,
        Exit::Continue if !jit::sse_ok(c.mxcsr) => 2,
        Exit::Continue => 0,
        _ => 2,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jb_eflags(_cpu: u32) -> u32 {
    let c = cpu();
    c.materialize();
    c.eflags
}

#[unsafe(no_mangle)]
pub extern "C" fn jb_status(op: u32, a: u32, b: u32, r: u32, c: u32) -> u32 {
    koji::lazy::Lazy { op, a: u64::from(a), b: u64::from(b), r: u64::from(r), c }.status()
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn jb_x87(_cpu: u32, top: u32, info: u32, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64, g: f64, h: f64) {
    jit::x87_out(&mut cpu().fpu, top, info, [a, b, c, d, e, f, g, h]);
}

/// kura's vnhost_jit_callmiss: a call cache slot filled from the hash table, else the target
/// into the ring
#[unsafe(no_mangle)]
pub extern "C" fn jb_callmiss(_cpu: u32, slot: u32, target: u32) -> i32 {
    let mut h = jit::dispatch::slot(target, HASH_N - 1);
    for _ in 0..HASH_N {
        // SAFETY: the hash table (this crate's, below the guest)
        let (key, v) = unsafe {
            let e = (HASH + 8 * h) as usize as *const u32;
            (e.read(), e.add(1).read())
        };
        if key == 0 {
            break;
        }
        if key == target {
            if v & jit::FN_TAG == 0 || v >> jit::ENTRY_SHIFT & 0x7ff != 0 {
                break;
            }
            // SAFETY: a call cache slot (the module's own address of it)
            unsafe { (slot as usize as *mut [u32; 2]).write_unaligned([target, v]) };
            return v as i32;
        }
        h = (h + 1) & (HASH_N - 1);
    }
    // SAFETY: the ring (this crate's)
    unsafe {
        let ring = RING as usize as *mut u32;
        let n = *ring;
        *ring = n.wrapping_add(1);
        let i = (n & (jit::RING_SIZE - 1)) as usize;
        *ring.add(1 + 2 * i) = jit::FN_TAG;
        *ring.add(2 + 2 * i) = target;
    }
    0
}

fn hash_insert(a: u32, v: u32) -> bool {
    let mut h = jit::dispatch::slot(a, HASH_N - 1);
    for _ in 0..HASH_N {
        // SAFETY: the hash table
        unsafe {
            let e = (HASH + 8 * h) as usize as *mut u32;
            let key = e.read();
            if key == 0 || key == a {
                e.write(a);
                e.add(1).write(v);
                return true;
            }
        }
        h = (h + 1) & (HASH_N - 1);
    }
    false
}

fn translate(st: &mut State, eip: u32) -> bool {
    st.known.insert(eip);
    let fns = jit::FN.load(Relaxed);
    let t0 = unsafe { jb_now() };
    let ran = st.ran.clone();
    let accept = |a: u32| a != END;
    let built = if fns {
        jit::translate_function_fit(None, &mut Linear, eip, jit::MAX_BLOCKS, 1000, 250, (END, END + 16), &|_| None, &|a| ran.contains(&a), accept)
    } else {
        let mut m = Module::default();
        jit::translate_region(&mut m, &mut Linear, &[eip], jit::MAX_BLOCKS, accept).map(|r| (m, r))
    };
    let Some((m, region)) = built else {
        st.stats.translate_ms += unsafe { jb_now() } - t0;
        return false;
    };
    let bytes = m.finish();
    st.stats.translate_ms += unsafe { jb_now() } - t0;
    st.stats.translations += 1.0;
    st.stats.wasm_bytes += bytes.len() as f64;
    st.stats.guest_ins += region.instructions as f64;
    let n = if fns { 2 } else { 1 };
    // SAFETY: one thread; the bytes stay until jb_load returns
    let base = unsafe {
        let out = &mut *(&raw mut OUT);
        *out = bytes;
        jb_load(out.as_ptr() as u32, out.len() as u32, n)
    };
    if base < 0 {
        st.stats.load_fail += 1.0;
        return false;
    }
    let idx = base as u32;
    for (i, &(a, k)) in region.starts.iter().enumerate() {
        st.known.insert(a);
        let v = if fns { idx | (i as u32) << jit::ENTRY_SHIFT | jit::FN_TAG } else { idx | k << jit::ENTRY_SHIFT };
        if !hash_insert(a, v) {
            return false;
        }
    }
    true
}

/// Runs the program set up from EIP to END: 0 done, 1 an exception (the interpreter's), 2 a
/// status the loop does not know, 3 no end
#[unsafe(no_mangle)]
pub extern "C" fn jb_run() -> u32 {
    ST.with(|s| {
        let st = &mut *s.borrow_mut();
        for _ in 0..100_000_000u64 {
            let c = cpu();
            if c.eip == END {
                st.end_ok = true;
                return 0;
            }
            c.jit_hash = HASH;
            c.jit_mask = HASH_N - 1;
            c.jit_ring = RING;
            c.jit_calls = CALLS;
            c.jit_code_pages = PAGES;
            c.jit_budget = 1 << 16;
            if NOJIT.load(Relaxed) {
                for _ in 0..4096 {
                    st.stats.interp_steps += 1.0;
                    if koji::step(c, &mut Linear) != Exit::Continue {
                        return 1;
                    }
                    if c.eip == END {
                        break;
                    }
                }
                continue;
            }
            st.stats.dispatches += 1.0;
            // SAFETY: the dispatcher on the Cpu
            let r = unsafe { jb_dispatch(CPU) };
            let c = cpu();
            if c.jit_stale != 0 || c.jit_smc != 0 {
                return 2;
            }
            match r {
                jit::RET_CONTINUE => st.stats.exit_continue += 1.0,
                jit::RET_FN => {}
                jit::RET_INTERP | 2 => {
                    st.stats.exit_interp += 1.0;
                    if c.eip == END {
                        continue;
                    }
                    let eip = c.eip;
                    let m = st.misses.entry(eip).or_insert(0);
                    *m += 1;
                    if *m >= THRESHOLD && !st.known.contains(&eip) && translate(st, eip) {
                        continue;
                    }
                    // the interpreter up to the next branch
                    for _ in 0..4096 {
                        st.ran.insert(c.eip);
                        st.stats.interp_steps += 1.0;
                        let (r, branch) = koji::step_flow(c, &mut Linear);
                        if r != Exit::Continue {
                            return 1;
                        }
                        if branch || c.eip == END {
                            break;
                        }
                    }
                }
                _ => return 2,
            }
        }
        3
    })
}
