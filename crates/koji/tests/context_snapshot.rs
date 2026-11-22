//! A translation is a function of its context and the guest's bytes (jit::TranslationContext,
//! koji redesign stage 2): translations made while another thread flips the modes (the page
//! checks, the MXCSR flag checks, hoisting, the loop slots, the SSA runs) come out the same bytes every time
//! for each context they record. None is part one mode, part another, which is what reading the
//! statics where they were used gave. Its own test binary: it sets the statics.
mod golden;
use golden::*;
use koji::jit::{self, wasm::Module};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

/// Program `n` by translator `k`: its context and module bytes
fn make(k: usize, n: usize) -> Option<(jit::TranslationContext, Vec<u8>)> {
    let prog = program(0x6374_7821, n);
    let mut mem = Mem::new(&prog, CODE);
    let mut m = Module::default();
    let r = match k {
        0 => jit::translate_region(&mut m, &mut mem, &[CODE], 16, |_| true),
        1 => jit::translate_function(&mut m, &mut mem, CODE, 16, 4000, (0, 0), &|_| None, &|_| true, |_| true),
        2 => jit::long::translate_region64(&mut m, &mut mem, &[u64::from(CODE)], (0, 0), |_, _| true),
        3 => jit::long::translate_function64(&mut m, &mut mem, u64::from(CODE), (0, 0), 4000, |_, _| true),
        _ => jit::translate_region16(&mut m, &mut mem, CODE, CODE, 16, |_| true),
    }?;
    Some((r.ctx, m.finish()))
}

type Seen = HashMap<(usize, usize, jit::TranslationContext), Vec<u8>>;

/// Every translation of `progs` programs, `rounds` times: Err at one whose bytes differ from an
/// earlier one of the same context
fn translate_all(progs: usize, rounds: usize) -> Result<Seen, String> {
    let mut seen = Seen::new();
    for _ in 0..rounds {
        for n in 0..progs {
            for k in 0..5 {
                let Some((c, b)) = make(k, n) else { continue };
                match seen.entry((k, n, c)) {
                    Entry::Occupied(e) if *e.get() != b => return Err(format!("program {n}, translator {k}: {c:?} made two different modules")),
                    Entry::Occupied(_) => {}
                    Entry::Vacant(e) => {
                        e.insert(b);
                    }
                }
            }
        }
    }
    Ok(seen)
}

#[test]
fn same_context_same_bytes_while_the_modes_flip() {
    let stop = AtomicBool::new(false);
    let results: Vec<Result<Seen, String>> = std::thread::scope(|s| {
        s.spawn(|| {
            let mut i = 0u32;
            while !stop.load(Relaxed) {
                jit::PAGE_CHECK.store(i & 1 != 0, Relaxed);
                jit::MX_EXACT.store(i & 2 != 0, Relaxed);
                jit::HOIST.store(i & 4 == 0, Relaxed);
                jit::SLOTS_MAX.store(if i & 8 != 0 { 4 } else { 0 }, Relaxed);
                jit::SSA.store(i & 16 != 0, Relaxed);
                i = i.wrapping_add(1);
                std::hint::spin_loop();
            }
        });
        let ts: Vec<_> = (0..3).map(|_| s.spawn(|| translate_all(48, 3))).collect();
        let r = ts.into_iter().map(|t| t.join().unwrap_or_else(|_| Err("a translating thread panicked".into()))).collect();
        stop.store(true, Relaxed);
        r
    });
    let mut all = Seen::new();
    for r in results {
        match r {
            Ok(seen) => {
                // (and across the threads: the same context, the same bytes)
                for (key, b) in seen {
                    if let Some(o) = all.insert(key, b.clone()) {
                        assert_eq!(o, b, "two threads made different modules under {:?}", key.2);
                    }
                }
            }
            Err(e) => panic!("{e}"),
        }
    }
    // (the flips were seen: more than one context per translator)
    let contexts: std::collections::HashSet<_> = all.keys().map(|k| k.2).collect();
    assert!(contexts.len() > 1, "the modes never changed under the translations: {contexts:?}");
}
