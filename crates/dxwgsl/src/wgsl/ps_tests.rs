//! The triangle count's DDA against geometry: a span's ends are the first pixel whose centre is
//! at or right of the triangle's left boundary and of its right one, found here from the edges'
//! exact crossings of the row centre (rationals in i128) and the leftmost / rightmost of them, no
//! DDA, no chains, no winding. Any fixed-point shortcut, a chain taken at the wrong row or left
//! and right swapped shows up as a different count. Both of WARP's iterators: EdgeIterator (its
//! 2^-42 slope) for the triangles near the origin, BigEdgeIterator (2^-64) past ±2^20 or 2^21
//! rows (pipeline-stats.md 2).
use proptest::prelude::*;

use super::{big, edge_x, spans, Cull, Raster};

/// ceil(n / d), d > 0
fn div_ceil(n: i128, d: i128) -> i128 {
    let q = n.div_euclid(d);
    if n.rem_euclid(d) == 0 { q } else { q + 1 }
}

/// The first pixel whose centre (256 p + 128) is at or right of edge a-b's x at yc.
fn exact_x(a: (i64, i64), b: (i64, i64), yc: i64) -> i64 {
    let (t, bt) = if a.1 <= b.1 { (a, b) } else { (b, a) };
    let dy = i128::from(bt.1 - t.1);
    let num = i128::from(t.0 - 128) * dy + i128::from(yc - t.1) * i128::from(bt.0 - t.0);
    i64::try_from(div_ceil(num, 256 * dy)).unwrap_or(0)
}

/// The count by geometry: rows whose centres are in [top, bottom) of the triangle, the span of
/// the leftmost and rightmost crossings of the non-level edges (an edge holds yc in [its top,
/// its bottom]), clamped, by quad rows as WARP's edge table.
fn reference(v: &[(i64, i64); 3], r: &Raster) -> u64 {
    let [cl, ct, cr, cb] = r.rect.map(i64::from);
    let top = v.iter().map(|p| p.1).min().unwrap_or(0);
    let bot = v.iter().map(|p| p.1).max().unwrap_or(0);
    let first = ((top + 127) >> 8).max(ct);
    let end = ((bot + 127) >> 8).min(cb);
    if first >= end {
        return 0;
    }
    let row = |j: i64| -> (i64, i64) {
        if j < first || j >= end {
            return (0x7fff, 0);
        }
        let yc = j * 256 + 128;
        let xs: Vec<i64> = (0..3).filter_map(|k| {
            let (a, b) = (v[k], v[(k + 1) % 3]);
            (a.1 != b.1 && yc >= a.1.min(b.1) && yc <= a.1.max(b.1)).then(|| exact_x(a, b, yc))
        }).collect();
        let lo = xs.iter().copied().min().unwrap_or(0);
        let hi = xs.iter().copied().max().unwrap_or(0);
        (lo.max(cl).min(cr), hi.max(cl).min(cr))
    };
    let mut n = 0i64;
    let mut j = first & !1;
    while j < end {
        let (a, b) = (row(j), row(j + 1));
        n += ((a.1.max(b.1) + 1) >> 1) - (a.0.min(b.0) >> 1);
        j += 2;
    }
    u64::try_from(n * 4).unwrap_or(0)
}

fn raster(rect: [i32; 4]) -> Raster {
    Raster { viewport: [0.0, 0.0, 4096.0, 4096.0], cull: Cull::None, front_ccw: false, depth_clip: true, rect }
}

/// a corner in 1/256 units: anywhere in a 4000-pixel box, or near a pixel centre or edge
fn corner() -> impl Strategy<Value = (i64, i64)> {
    let c = || prop_oneof![-200_000i64..1_000_000, (-800i64..4000).prop_flat_map(|p| p * 256 + 120..p * 256 + 137)];
    (c(), c())
}

/// a corner anywhere a viewport of D3D11 puts one (±2^15 + 16384 pixels): BigEdgeIterator's too
fn far_corner() -> impl Strategy<Value = (i64, i64)> {
    let c = || prop_oneof![-12_000_000i64..12_000_000, (-46_000i64..46_000).prop_flat_map(|p| p * 256 + 120..p * 256 + 137)];
    (c(), c())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20_000))]

    /// spans (the DDA) = the geometric count, in the whole target and in a scissored part
    #[test]
    fn spans_are_the_geometric_spans(a in corner(), b in corner(), c in corner(), small in any::<bool>()) {
        let v = [a, b, c];
        let det = (b.0 - a.0) as i128 * (c.1 - a.1) as i128 - (b.1 - a.1) as i128 * (c.0 - a.0) as i128;
        prop_assume!(det != 0);
        let r = raster(if small { [100, 37, 2501, 1999] } else { [0, 0, 4096, 4096] });
        prop_assert_eq!(spans(&v, &r), reference(&v, &r));
    }

    /// spans = the geometric count for corners anywhere (mostly BigEdgeIterator's triangles)
    #[test]
    fn far_spans_are_the_geometric_spans(a in far_corner(), b in far_corner(), c in far_corner()) {
        let v = [a, b, c];
        let det = (b.0 - a.0) as i128 * (c.1 - a.1) as i128 - (b.1 - a.1) as i128 * (c.0 - a.0) as i128;
        prop_assume!(det != 0);
        let r = raster([-40_000, -40_000, 40_000, 40_000]);
        prop_assert_eq!(spans(&v, &r), reference(&v, &r));
    }

    /// one edge's DDA = the exact first centre at or right of it, every row of the edge
    #[test]
    fn edge_dda_is_exact(p in corner(), q in corner(), k in 0i64..4096) {
        prop_assume!(p.1 != q.1);
        let (t, b) = if p.1 < q.1 { (p, q) } else { (q, p) };
        let rows = (b.1 - t.1) / 256;
        let yc = ((t.1 + 127) >> 8) * 256 + 128 + 256 * (k % (rows + 1));
        prop_assume!(yc <= b.1 && yc >= t.1);
        prop_assert_eq!(edge_x(t, b, yc), exact_x(t, b, yc));
    }
}

/// winref d3d11gs's "across two edges" triangle through the reference as well as the rule
#[test]
fn reference_agrees_on_a_fixed_case() {
    let v = [(2048, 2048), (12288, 4096), (5000, 15000)];
    let r = raster([0, 0, 64, 64]);
    assert_eq!(spans(&v, &r), reference(&v, &r));
}

/// which iterator: corners' x within ±2^20 and under 2^21 from the top to the lowest corner
#[test]
fn big_iterator_bounds() {
    assert!(!big(&[(0, 0), (0xf_ffff, 10), (-0xf_ffff, 0x1f_ffff)], 0));
    assert!(big(&[(0, 0), (0x10_0000, 10), (5, 20)], 0));
    assert!(big(&[(0, 0), (-0x10_0000, 10), (5, 20)], 0));
    assert!(big(&[(0, 0), (5, 10), (5, 0x20_0000)], 0));
}

