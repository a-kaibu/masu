//! How much of a JIT translation is the allocator's (2026-11-21, whether an arena for the
//! translators' temporary Vecs pays): the golden's programs translated by every translator, once
//! with the system allocator and once with a bump allocator that never frees (no arena can be
//! cheaper than it), the calls and bytes counted. `cargo test --release -p koji --test
//! alloc_share -- --ignored --nocapture` prints the numbers.
mod golden;
use golden::*;
use koji::jit::{self, wasm::Module};

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

struct Counting;
static BUMP: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
/// the bump region: [START, END), NEXT the first free byte
static START: AtomicUsize = AtomicUsize::new(0);
static NEXT: AtomicUsize = AtomicUsize::new(0);
static END: AtomicUsize = AtomicUsize::new(0);
const REGION: usize = 6 << 30;

// SAFETY: every block is System's or a fresh, aligned, unshared piece of the bump region
// (System's own, never handed back); dealloc frees only System's (the region's are left).
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        CALLS.fetch_add(1, Relaxed);
        BYTES.fetch_add(l.size(), Relaxed);
        if BUMP.load(Relaxed) {
            let at = NEXT.load(Relaxed).next_multiple_of(l.align());
            if at + l.size() <= END.load(Relaxed) {
                NEXT.store(at + l.size(), Relaxed);
                return std::ptr::with_exposed_provenance_mut(at);
            }
        }
        // SAFETY: the caller's layout, as GlobalAlloc::alloc takes it
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        let a = p.addr();
        if a >= START.load(Relaxed) && a < END.load(Relaxed) {
            return;
        }
        // SAFETY: p came from System.alloc with l (above)
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn all(per_mode: usize) {
    for n in 0..per_mode {
        let prog = program(0x6a69_7421, n);
        // (an arena per translation: what one made is gone when it ends, its memory reused)
        let mark = NEXT.load(Relaxed);
        for k in 0..5 {
            let mut mem = Mem::new(&prog, CODE);
            let mut m = Module::default();
            let r = match k {
                0 => jit::translate_region(&mut m, &mut mem, &[CODE], 16, |_| true),
                1 => jit::translate_function(&mut m, &mut mem, CODE, 16, 4000, (0, 0), &|_| None, &|_| true, |_| true),
                2 => jit::long::translate_region64(&mut m, &mut mem, &[u64::from(CODE)], (0, 0), |_, _| true),
                3 => jit::long::translate_function64(&mut m, &mut mem, u64::from(CODE), (0, 0), 4000, |_, _| true),
                _ => jit::translate_region16(&mut m, &mut mem, CODE, CODE, 16, |_| true),
            };
            std::hint::black_box((r.is_some(), m.finish().len()));
            drop((r, m, mem));
            NEXT.store(mark, Relaxed);
        }
    }
}

#[test]
#[ignore = "a measurement: prints the allocator's share of JIT translation"]
fn allocator_share_of_translation() {
    const N: usize = 1024;
    // (the region taken from System before anything is timed)
    // SAFETY: a nonzero size, align 4096
    let base = unsafe { System.alloc(Layout::from_size_align(REGION, 4096).unwrap_or(Layout::new::<u8>())) };
    assert!(!base.is_null());
    START.store(base.expose_provenance(), Relaxed);
    NEXT.store(base.addr(), Relaxed);
    END.store(base.addr() + REGION, Relaxed);
    all(64);
    let mut best = [f64::MAX; 2];
    let mut counts = (0, 0);
    for round in 0..3 {
        for (k, bump) in [false, true].into_iter().enumerate() {
            NEXT.store(base.addr(), Relaxed);
            BUMP.store(bump, Relaxed);
            let (c0, b0) = (CALLS.load(Relaxed), BYTES.load(Relaxed));
            let t = std::time::Instant::now();
            all(N);
            let s = t.elapsed().as_secs_f64();
            BUMP.store(false, Relaxed);
            best[k] = best[k].min(s);
            if round == 0 && k == 0 {
                counts = (CALLS.load(Relaxed) - c0, BYTES.load(Relaxed) - b0);
            }
        }
    }
    let translations = 5 * N;
    println!("translations {translations}: {} allocations ({:.0} per translation), {:.1} MB", counts.0, counts.0 as f64 / translations as f64, counts.1 as f64 / 1e6);
    println!("system allocator {:.3} s, bump arena {:.3} s: the allocator's share at most {:.1}%", best[0], best[1], 100.0 * (best[0] - best[1]) / best[0]);
}
