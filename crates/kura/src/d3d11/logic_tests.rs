//! Which targets get logic ops, and a draw's primitives as draws of their own.
use super::*;
use dxwgsl::logic::{Logic, Mark, Op, Sign, Store};

/// targets 0..2 of R8G8B8A8_UINT, R32_UINT, R8G8B8A8_UNORM; blend state words with logic ops
fn blend(independent: bool, ops: [(u32, u32); 3]) -> Vec<u32> {
    let mut b = vec![0, u32::from(independent)];
    for _ in 0..8 {
        b.extend_from_slice(&[0, 2, 1, 1, 2, 1, 1, 0xf]);
    }
    for k in 0..8 {
        b.extend_from_slice(&ops.get(k).map_or([0, 4], |&(e, o)| [e, o]));
    }
    b
}

#[test]
fn targets_of_integer_formats_only() {
    let fmts = [f::P_RGBA8UI, f::P_R32UI, f::P_RGBA8];
    let (ops, kept) = targets(&blend(true, [(1, 10), (1, 4), (1, 10)]), &fmts, &[false, true, false], false);
    assert_eq!(ops, vec![Logic { target: 0, store: Store::Direct(Op::Xor), bits: [8, 8, 8, 8], sign: Sign::Unsigned, multisampled: false }]);
    assert_eq!(kept, vec![1]);
    // (without IndependentBlendEnable every target has target 0's; a multisampled one under a
    // pixel-rate shader a stash, its mark after the targets)
    let (ops, _) = targets(&blend(false, [(1, 6), (0, 0), (0, 0)]), &fmts, &[false, true, false], false);
    assert_eq!(ops.iter().map(|l| (l.target, l.store, l.multisampled)).collect::<Vec<_>>(), vec![(0, Store::Direct(Op::And), false), (1, Store::Stash { op: Op::And, mark: Mark::Slot(3) }, true)]);
    // (a shader of sample rate: each sample its own invocation, the op in the shader)
    let (ops, _) = targets(&blend(false, [(1, 6), (0, 0), (0, 0)]), &fmts, &[false, true, false], true);
    assert_eq!(ops.iter().map(|l| l.store).collect::<Vec<_>>(), vec![Store::Direct(Op::And), Store::Direct(Op::And)]);
    // (LogicOpEnable off: the integer targets written plainly, saturated)
    let (ops, kept) = targets(&blend(true, [(0, 10), (0, 10), (0, 10)]), &fmts, &[false; 3], false);
    assert_eq!(ops.iter().map(|l| (l.target, l.store)).collect::<Vec<_>>(), vec![(0, Store::Plain), (1, Store::Plain)]);
    assert!(kept.is_empty());
    // (SINT: the op too (winref d3d11logic), signed)
    let (ops, _) = targets(&blend(false, [(1, 8), (0, 0), (0, 0)]), &[f::P_RGBA8SI, f::P_R32SI], &[false; 2], false);
    assert_eq!(ops.iter().map(|l| (l.store, l.sign, l.bits)).collect::<Vec<_>>(), vec![(Store::Direct(Op::Or), Sign::Signed, [8, 8, 8, 8]), (Store::Direct(Op::Or), Sign::Signed, [32, 0, 0, 0])]);
}

#[test]
fn eight_rgba16_targets_buffer_one_source() {
    let mut b = vec![0, 0];
    for _ in 0..8 {
        b.extend_from_slice(&[0, 2, 1, 1, 2, 1, 1, 0xf]);
    }
    b.extend_from_slice(&[1, 10]);
    let (out, _) = targets(&b, &[f::P_RGBA16UI; 8], &[true; 8], false);
    assert_eq!(out.len(), 8);
    for l in &out {
        assert_eq!(l.store, Store::Stash { op: Op::Xor, mark: Mark::Slot(0) });
        assert_eq!(l.buffered(), l.target == 0);
    }
}

proptest::proptest! {
    /// Every target's store follows WARP's rules: none for a format without integer channels,
    /// plain without the op, kept for NOOP, a stash only for a multisampled target with an op
    /// that reads it under a pixel-rate shader.
    #[test]
    fn stores_follow_warp(page in 0u32..64, op in 0u32..16, on: bool, ms: bool, rate: bool) {
        let b = blend(false, [(u32::from(on), op), (0, 0), (0, 0)]);
        let (ops, kept) = targets(&b, &[page], &[ms], rate);
        let op = Op::of(op);
        match (int_bits(page), op.filter(|_| on)) {
            (None, _) => proptest::prop_assert!(ops.is_empty() && kept.is_empty()),
            (Some(_), None) => proptest::prop_assert_eq!(ops.iter().map(|l| l.store).collect::<Vec<_>>(), vec![Store::Plain]),
            (Some(_), Some(Op::Noop)) => proptest::prop_assert!(ops.is_empty() && kept == vec![0]),
            (Some((bits, sign)), Some(o)) => {
                let store = if ms && !rate && o.reads_dst() { Store::Stash { op: o, mark: Mark::Slot(1) } } else { Store::Direct(o) };
                proptest::prop_assert_eq!(ops, vec![Logic { target: 0, store, bits, sign, multisampled: ms }]);
            }
        }
    }
}

#[test]
fn copies_are_by_slot() {
    // (two targets of one size and format: two scratch keys, not one; the first word of the key)
    for (a, b) in [(COPY_OF, STASH_OF), (STASH_OF, MARK), (COPY_OF, MARK)] {
        assert_eq!(a & 0xff, 0);
        assert_ne!(a, b);
    }
    assert_ne!(COPY_OF | 1, COPY_OF);
    assert_eq!(unbound(961, true, Sign::Signed), [961, 5, 0, 6, 0, 1, 0, 1]);
    assert_eq!(unbound(960, false, Sign::Unsigned), [960, 4, 0, 1, 0, 1, 0, 1]);
}

#[test]
fn primitives_of_each_topology() {
    let p = |first, count, odd| Prim { first, count, odd };
    assert_eq!(primitives(0, 3, &[]), vec![p(0, 1, false), p(1, 1, false), p(2, 1, false)]);
    assert_eq!(primitives(1, 5, &[]), vec![p(0, 2, false), p(2, 2, false)]);
    assert_eq!(primitives(2, 3, &[]), vec![p(0, 2, false), p(1, 2, false)]);
    assert_eq!(primitives(3, 7, &[]), vec![p(0, 3, false), p(3, 3, false)]);
    assert_eq!(primitives(4, 5, &[]), vec![p(0, 3, false), p(1, 3, true), p(2, 3, false)]);
    // (a restart at 2: triangles 3 4 5 and 4 5 6 after it, the first even again)
    let cut = [false, false, true, false, false, false, false];
    assert_eq!(primitives(4, 7, &cut), vec![p(3, 3, false), p(4, 3, true)]);
    let cut = [false, false, false, true, false, false, false];
    assert_eq!(primitives(4, 7, &cut), vec![p(0, 3, false), p(4, 3, false)]);
    assert!(primitives(4, 2, &[]).is_empty());
}

const INTS: [u32; 9] = [f::P_RGBA8UI, f::P_RG8UI, f::P_R8UI, f::P_RGBA16UI, f::P_RG16UI, f::P_RGB10A2UI, f::P_RGBA8SI, f::P_RGBA16UI, f::P_RGBA16SI];

proptest::proptest! {
    /// Eight targets: no location free, the mark carried by the first stash that can carry it,
    /// every stash pointing at that one, its stash format one the page multisamples; with no
    /// stash that can carry it, at the first stash's location, that one's source buffered.
    #[test]
    fn eight_targets_carry_the_mark(pages in proptest::collection::vec(0usize..9, 8), ops in proptest::collection::vec(0u32..16, 8)) {
        let mut b = vec![0, 1];
        for _ in 0..8 {
            b.extend_from_slice(&[0, 2, 1, 1, 2, 1, 1, 0xf]);
        }
        for o in &ops {
            b.extend_from_slice(&[1, *o]);
        }
        let fmts: Vec<u32> = pages.iter().map(|&p| INTS[p]).collect();
        let (out, _) = targets(&b, &fmts, &[true; 8], false);
        let stashes: Vec<&Logic> = out.iter().filter(|l| matches!(l.store, Store::Stash { .. })).collect();
        let wanted: Vec<&Logic> = out.iter().filter(|l| Op::of(ops[usize::try_from(l.target).unwrap()]).is_some_and(Op::reads_dst)).collect();
        // (every op that reads its target a stash, the shader once a pixel)
        proptest::prop_assert_eq!(stashes.len(), wanted.len());
        let carrier = wanted.iter().find(|l| dxwgsl::logic::carry(l.bits).is_some());
        for l in &stashes {
            match (carrier, l.store) {
                (Some(c), Store::Stash { mark: Mark::Carried { by, .. }, .. }) => {
                    proptest::prop_assert_eq!(by, c.target);
                    let page = uint_page(l.stash_bits());
                    proptest::prop_assert!(page.is_some_and(crate::render::d3d::page_multisamples), "{:?}", l);
                }
                // (none can carry it: the first stash's location the mark's, its source buffered)
                (None, Store::Stash { mark: Mark::Slot(at), .. }) => {
                    proptest::prop_assert_eq!(at, stashes[0].target);
                    proptest::prop_assert_eq!(l.buffered(), l.target == at);
                    proptest::prop_assert!(uint_page(l.stash_bits()).is_some_and(crate::render::d3d::page_multisamples), "{:?}", l);
                }
                (_, store) => return Err(proptest::test_runner::TestCaseError::fail(format!("{store:?} with carrier {carrier:?}"))),
            }
        }
    }
}
