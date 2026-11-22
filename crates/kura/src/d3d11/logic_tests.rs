//! Which targets get logic ops, and a draw's primitives as draws of their own.
use super::*;

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
fn targets_of_uint_formats_only() {
    let fmts = [f::P_RGBA8UI, f::P_R32UI, f::P_RGBA8];
    let (ops, kept) = targets(&blend(true, [(1, 10), (1, 4), (1, 10)]), &fmts, &[false, true, false]);
    assert_eq!(ops, vec![Logic { target: 0, op: Op::Xor, bits: [8, 8, 8, 8], multisampled: false }]);
    assert_eq!(kept, vec![1]);
    // (without IndependentBlendEnable every target has target 0's)
    let (ops, _) = targets(&blend(false, [(1, 6), (0, 0), (0, 0)]), &fmts, &[false, true, false]);
    assert_eq!(ops.iter().map(|l| (l.target, l.op, l.multisampled)).collect::<Vec<_>>(), vec![(0, Op::And, false), (1, Op::And, true)]);
    // (LogicOpEnable off: none)
    assert!(targets(&blend(true, [(0, 10), (0, 10), (0, 10)]), &fmts, &[false; 3]).0.is_empty());
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
