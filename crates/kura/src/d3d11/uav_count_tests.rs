//! The hidden counter of a buffer UAV made with APPEND or COUNTER: the initial counts that
//! CSSetUnorderedAccessViews / OMSetRenderTargetsAndUnorderedAccessViews give it (-1 keeps it)
//! and CopyStructureCount's copy of it into a buffer, through the real entry point.

use kome::locks::Lock as _;
use super::query_tests::{cc, dc, device, made, mem, put, rd1, release_all};
use super::*;
use crate::d3d9::trace_tests::SERIAL;
use proptest::prelude::*;

/// A buffer of `n` structures of 4 bytes (bind `bind`, misc STRUCTURED).
fn structured(dev: u32, n: u32, bind: u32) -> u32 {
    let d = mem(6);
    put(d, &[4 * n, 0, bind, 0, 0x40, 4]);
    let o = mem(2);
    assert_eq!(dc(dev, 3, &[d, 0, o]), 0);
    made(rd1(o))
}

/// What the guest's bytes of buffer `b` hold at word `i`.
fn word(b: u32, i: usize) -> u32 {
    let g = D11S.locked();
    let Some(d) = g.as_ref() else { return 0 };
    match d.objs.get(&d.id_of(b)).map(|o| &o.data) {
        Some(Data::Buffer(x)) => unsafe { rd(x.mem, i) },
        _ => 0,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]
    /// Each counted view keeps the last count it was given (not -1); CopyStructureCount writes
    /// it at the offset and nothing else; a view without a counter writes nothing.
    #[test]
    fn counters_as_set(sets in prop::collection::vec((0usize..3, prop::sample::select(vec![0u32, 1, 7, 1000, u32::MAX, 0xffff_fffe]), any::<bool>()), 1..10), at in 0u32..4) {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (dev, ctx) = device();
        let src = structured(dev, 16, 0x80);
        // views: APPEND, COUNTER, none
        let views: Vec<u32> = [2u32, 4, 0].iter().map(|&flags| {
            let d = mem(5);
            put(d, &[0, 1, 0, 16, flags]);
            let o = mem(2);
            assert_eq!(dc(dev, 8, &[src, d, o]), 0);
            made(rd1(o))
        }).collect();
        let dst = structured(dev, 4, 0x80);
        let mut want = [0u32; 3];
        let (vs, counts) = (mem(1), mem(1));
        for (k, c, om) in sets {
            put(vs, &[views[k]]);
            put(counts, &[c]);
            if om {
                // OMSetRenderTargetsAndUnorderedAccessViews(keep the targets, start 1, 1 view)
                cc(ctx, 34, &[u32::MAX, 0, 0, 1, 1, vs, counts]);
            } else {
                cc(ctx, 68, &[0, 1, vs, counts]);
            }
            if c != u32::MAX && k != 2 {
                want[k] = c;
            }
            for (j, &v) in views.iter().enumerate() {
                let before: Vec<u32> = (0..4).map(|i| word(dst, i)).collect();
                cc(ctx, 49, &[dst, 4 * at, v]);
                let after: Vec<u32> = (0..4).map(|i| word(dst, i)).collect();
                let mut expect = before.clone();
                if j != 2 {
                    expect[at as usize] = want[j];
                }
                prop_assert_eq!(after, expect, "view {}", j);
            }
        }
        cc(ctx, 68, &[0, 1, 0, 0]);
        release_all();
    }
}
