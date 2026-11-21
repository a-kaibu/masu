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

/// A structured buffer of `n` structures of `stride` bytes, bound for SRVs and UAVs.
fn structured_of(dev: u32, n: u32, stride: u32) -> u32 {
    let d = mem(6);
    put(d, &[stride * n, 0, 0x88, 0, 0x40, stride]);
    let o = mem(2);
    assert_eq!(dc(dev, 3, &[d, 0, o]), 0);
    made(rd1(o))
}

/// A view (`m` 7 SRV, 8 UAV) of buffer `b` from element `first`, `count` of them.
fn view_of(dev: u32, m: u32, b: u32, first: u32, count: u32) -> u32 {
    let d = mem(5);
    // (SRV: DXGI_FORMAT_UNKNOWN, BUFFEREX; UAV: BUFFER)
    put(d, &[0, if m == 7 { 11 } else { 1 }, first, count, 0]);
    let o = mem(2);
    assert_eq!(dc(dev, m, &[b, d, o]), 0);
    made(rd1(o))
}

/// The binding and range of the raw or structured view at binding `bi` of the compute stage.
fn binding_of(bi: u32) -> ([u32; 8], [u32; 4]) {
    let mut g = D11S.locked();
    let Some(d) = g.as_mut() else { return ([0; 8], [0; 4]) };
    let st = d.st.clone();
    buffer_view_binding(d, &st, 5, bi)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]
    /// A structured view bound for a shader: the whole buffer from the 256-byte boundary at or
    /// before its first byte, its range from there (first dword, bytes, structures); nothing
    /// bound: an empty range.
    #[test]
    fn view_ranges(stride in prop::sample::select(vec![4u32, 8, 12, 24, 64, 2048]), n in 1u32..200, a in any::<u32>(), b in any::<u32>(), uav in any::<bool>()) {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (dev, ctx) = device();
        let buf = structured_of(dev, n, stride);
        let first = a % n;
        let count = 1 + b % (n - first);
        let v = view_of(dev, if uav { 8 } else { 7 }, buf, first, count);
        let vs = mem(1);
        put(vs, &[v]);
        let bi = if uav {
            cc(ctx, 68, &[0, 1, vs, 0]);
            600
        } else {
            cc(ctx, 67, &[0, 1, vs]);
            500
        };
        let (e, r) = binding_of(bi);
        let off = first * stride;
        prop_assert_eq!(e[0], bi);
        prop_assert_eq!(e[1], if uav { 10 } else { 9 });
        prop_assert!(e[2] != 0);
        prop_assert_eq!(e[3] % 256, 0);
        prop_assert!(e[3] <= off && off - e[3] < 256);
        prop_assert_eq!(e[3] + 4 * r[0], off);
        prop_assert_eq!(r, [r[0], count * stride, count, 0]);
        // (the slot emptied: a null buffer, nothing in range)
        put(vs, &[0]);
        if uav { cc(ctx, 68, &[0, 1, vs, 0]); } else { cc(ctx, 67, &[0, 1, vs]); }
        let (e, r) = binding_of(bi);
        prop_assert!(e[2] != 0);
        prop_assert_eq!(r, [0; 4]);
        release_all();
    }
}

/// A counter goes to the page when a shader binds it, and back to kura's when an initial count
/// is set; CopyStructureCount writes kura's count into the guest's bytes itself.
#[test]
fn counter_moves_to_the_page_and_back() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (dev, ctx) = device();
    let src = structured(dev, 16, 0x80);
    let d = mem(5);
    put(d, &[0, 1, 0, 16, 2]);
    let o = mem(2);
    assert_eq!(dc(dev, 8, &[src, d, o]), 0);
    let v = made(rd1(o));
    let (vs, counts) = (mem(1), mem(1));
    put(vs, &[v]);
    put(counts, &[7]);
    cc(ctx, 68, &[0, 1, vs, counts]);
    let count = || {
        let g = D11S.locked();
        let d = g.as_ref().expect("device");
        match d.objs.get(&d.id_of(v)).map(|o| &o.data) {
            Some(Data::BufferUav(u)) => (u.counter, u.counter_page),
            _ => (Count::Host(u32::MAX), 0),
        }
    };
    assert_eq!(count(), (Count::Host(7), 0));
    let e = {
        let mut g = D11S.locked();
        let d = g.as_mut().expect("device");
        let st = d.st.clone();
        counter_binding(d, &st, 5, 700)
    };
    let (c, page) = count();
    assert_eq!((c, e[1], e[2]), (Count::Page, 10, page));
    assert_ne!(page, 0);
    put(counts, &[9]);
    cc(ctx, 68, &[0, 1, vs, counts]);
    assert_eq!(count(), (Count::Host(9), page));
    let dst = structured(dev, 4, 0x80);
    cc(ctx, 49, &[dst, 8, v]);
    assert_eq!(word(dst, 2), 9);
    cc(ctx, 68, &[0, 1, 0, 0]);
    release_all();
}

/// After the page's device was lost, the views' range tables and the constant buffer range
/// copies are written again when used (the page's copies of them are empty): none of what kura
/// kept of them is taken for what the page holds.
#[test]
fn range_tables_written_again_after_a_lost_device() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (dev, ctx) = device();
    let buf = structured_of(dev, 8, 16);
    let v = view_of(dev, 7, buf, 3, 2);
    let vs = mem(1);
    put(vs, &[v]);
    cc(ctx, 67, &[0, 1, vs]);
    let table = |d: &mut D11| {
        let st = d.st.clone();
        ranges_binding(d, &st, 5, &[500])[2]
    };
    let first = {
        let mut g = D11S.locked();
        let d = g.as_mut().expect("device");
        d.cb_copies.insert((1, 2, 16), (77, vec![1, 2, 3]));
        let a = table(d);
        assert_eq!(table(d), a, "the same table: one page buffer");
        a
    };
    crate::render::lose_device();
    // (the guest's next call notices it)
    cc(ctx, 67, &[0, 1, vs]);
    {
        let mut g = D11S.locked();
        let d = g.as_mut().expect("device");
        assert!(d.cb_copies.values().all(|(_, last)| last.is_empty()), "constant buffer copies written again");
        assert_ne!(table(d), first, "the range table made again");
        d.cb_copies.remove(&(1, 2, 16));
    }
    put(vs, &[0]);
    cc(ctx, 67, &[0, 1, vs]);
    release_all();
}
