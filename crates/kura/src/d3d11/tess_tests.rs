//! The tessellator (tess.rs) against what Direct3D 11's tessellation stage promises (the
//! functional spec, 11.7 "Tessellator"): a culled patch for any edge factor that is not above 0
//! (NaN too); points inside the domain, none twice, each one a primitive's; triangles that
//! cover the domain once (their signed areas add up to its area, every one wound the same
//! way); the points on an edge decided by that edge's factor alone (patches that share an edge
//! meet without cracks); integer and pow2 partitioning the same to the hardware, a factor and
//! its ceiling the same with them; fractional partitioning at an integer of its parity the same
//! as integer partitioning. Then fixed point conversion against f64, and a golden of the points
//! and triangles of chosen patches (the refactor oracle; Windows' answers: winref d3d11tess).

use super::*;
use proptest::prelude::*;

const PARTS: [Partitioning; 4] = [Partitioning::Integer, Partitioning::Pow2, Partitioning::FractionalOdd, Partitioning::FractionalEven];

fn factor() -> impl Strategy<Value = f32> {
    prop_oneof![
        3 => 0.0f32..70.0,
        3 => (1u32..=64).prop_map(kome::num::to_f32),
        1 => prop::sample::select(vec![0.0f32, -0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e-30, 0.5, 1.0, 1.000_007_7, 63.5, 64.0, 65.0, 1e9]),
    ]
}

fn part() -> impl Strategy<Value = Partitioning> {
    prop::sample::select(PARTS.to_vec())
}

fn tri_out() -> impl Strategy<Value = Output> {
    prop::sample::select(vec![Output::Point, Output::TriangleCw, Output::TriangleCcw])
}

/// Twice the signed area of a triangle of domain points.
fn area2(t: &Tessellation, k: usize) -> f64 {
    let p = |i: usize| t.indices.get(i).and_then(|&j| t.points.get(us(j))).copied().unwrap_or([0.0; 2]).map(f64::from);
    let (a, b, c) = (p(k), p(k + 1), p(k + 2));
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Each triangle wound the same way and their areas adding up to the domain's (a tri's 1/2,
/// a quad's 1): the domain covered once, no gaps, no overlaps.
/// Whether triangle `k` (its first index) has two corners at most 2 / 65536 apart: the
/// reference's 15.16 points next to an edge's middle. PlacePointIn1D special-cases the middle
/// "since 16 bit fixed math below can't reproduce 0.5 exactly", and the points beside it can land
/// past it (factor 30.005 fractional_even: 0.5000305, 0.5, 0.4999695 in that order), so the two
/// triangles there are wound the other way in the reference too, slivers of at most that width.
fn sliver(t: &Tessellation, k: usize) -> bool {
    let p = |i: usize| t.indices.get(i).and_then(|&j| t.points.get(us(j))).copied().unwrap_or([0.0; 2]).map(f64::from);
    let (a, b, c) = (p(k), p(k + 1), p(k + 2));
    let near = |x: [f64; 2], y: [f64; 2]| (x[0] - y[0]).abs().max((x[1] - y[1]).abs()) <= 2.0 / 65536.0;
    near(a, b) || near(b, c) || near(c, a)
}

/// The reference's own sliver (sliver's comment): the edge of factor 30.005 has its points beside
/// the middle on the wrong sides of it, and the two triangles to them are wound the other way,
/// while the patch is still covered once.
#[test]
fn middle_points_past_the_middle() {
    let t = tessellate(Partitioning::FractionalEven, Output::TriangleCw, Factors::Quad { edges: [20.992214, 14.804766, 30.005175, 60.035915], inside: [0.0, 0.0] });
    let v: Vec<f32> = [53usize, 54, 55].iter().map(|&i| t.points[i][1]).collect();
    assert_eq!(v, [0.5 + 2.0 / 65536.0, 0.5, 0.5 - 2.0 / 65536.0]);
    let reversed: Vec<usize> = (0..t.indices.len()).step_by(3).filter(|&k| area2(&t, k) < 0.0).collect();
    assert!(reversed.iter().all(|&k| sliver(&t, k)), "{reversed:?}");
    assert_eq!(reversed.len(), 2);
    covers(&t, 1.0, Output::TriangleCw).unwrap();
}

fn covers(t: &Tessellation, domain_area: f64, out: Output) -> Result<(), String> {
    let mut sum = 0.0;
    for k in (0..t.indices.len()).step_by(3) {
        let a = area2(t, k) / 2.0;
        // (Direct3D's u, v axes: a clockwise triangle has a positive area in them)
        let a = if out == Output::TriangleCw { a } else { -a };
        if a < -1e-9 && !sliver(t, k) {
            return Err(format!("triangle {} wound the other way ({a})", k / 3));
        }
        sum += a;
    }
    if (sum - domain_area).abs() > 1e-6 {
        return Err(format!("triangles cover {sum}, the domain {domain_area}"));
    }
    Ok(())
}

/// Points inside the domain, each in a primitive; with integer partitioning none at the same
/// place (fractional partitioning's shortest segments can round to nothing in 15.16: two points
/// at one place, as the reference makes them, a factor of 1 + 2^-16 the smallest case).
fn basic(t: &Tessellation, tri: bool, p: Partitioning) -> Result<(), String> {
    for (i, p) in t.points.iter().enumerate() {
        if !(0.0..=1.0).contains(&p[0]) || !(0.0..=1.0).contains(&p[1]) || (tri && p[0] + p[1] > 1.0 + 1e-6) {
            return Err(format!("point {i} {p:?} outside the domain"));
        }
    }
    let integer = matches!(p, Partitioning::Integer | Partitioning::Pow2);
    let mut seen: Vec<[u32; 2]> = t.points.iter().map(|p| [p[0].to_bits(), p[1].to_bits()]).collect();
    seen.sort_unstable();
    let n = seen.len();
    seen.dedup();
    if integer && seen.len() != n {
        return Err(format!("{} points twice", n - seen.len()));
    }
    let mut used = vec![false; t.points.len()];
    for &i in &t.indices {
        let Some(u) = used.get_mut(us(i)) else { return Err(format!("index {i} past {} points", t.points.len())) };
        *u = true;
    }
    if let Some(i) = used.iter().position(|u| !u) {
        return Err(format!("point {i} in no primitive"));
    }
    Ok(())
}

/// The points the tessellator puts on an edge of factor `f` alone (its 1D placement: the edge
/// clamped and rounded as an edge, its parity, 15.16 placement), in order from 0.
fn edge_1d(p: Partitioning, f: f32) -> Vec<f32> {
    let mut h = Hw::new(p, Output::TriangleCw);
    let (lower, upper) = h.bounds();
    let mut f = fmin(upper, fmax(lower, f));
    h.parity = if h.hw_integer() {
        f = f.ceil();
        if is_even(f) { Parity::Even } else { Parity::Odd }
    } else {
        h.original_parity
    };
    let fx = to_fixed(f);
    let c = h.context(fx);
    N(0).upto(h.points_for(fx)).map(|q| to_float(h.place(&c, q))).collect()
}

/// Edge `e`'s points as the patch has them: the outside ring's (its first points, edge by edge
/// clockwise, each edge's last point the next one's first), the coordinate along the edge
/// (`along`: 0 u, 1 v, for each edge), sorted.
fn ring_edge(t: &Tessellation, counts: &[usize], along: &[usize], e: usize) -> Vec<f32> {
    let ring: usize = counts.iter().map(|n| n - 1).sum();
    let start: usize = counts.iter().take(e).map(|n| n - 1).sum();
    let mut v: Vec<f32> = (0..counts[e]).map(|k| t.points.get((start + k) % ring).map_or(f32::NAN, |q| q[along[e]])).collect();
    v.sort_by(f32::total_cmp);
    v
}

const TRI_ALONG: [usize; 3] = [1, 0, 0];
const QUAD_ALONG: [usize; 4] = [1, 0, 1, 0];

/// Each outside edge of a patch holds its own factor's 1D points, whatever the other factors:
/// patches sharing an edge factor meet without cracks.
fn edges_are_their_own(p: Partitioning, t: &Tessellation, edges: &[f32]) -> Result<(), String> {
    let ones: Vec<Vec<f32>> = edges.iter().map(|&f| edge_1d(p, f)).collect();
    let counts: Vec<usize> = ones.iter().map(Vec::len).collect();
    let along: &[usize] = if edges.len() == 3 { &TRI_ALONG } else { &QUAD_ALONG };
    for (e, one) in ones.iter().enumerate() {
        let got = ring_edge(t, &counts, along, e);
        let mut want = one.clone();
        want.sort_by(f32::total_cmp);
        if got != want {
            return Err(format!("edge {e} (factor {}): {got:?}, its own points {want:?}", edges[e]));
        }
    }
    Ok(())
}

fn culled(edges: &[f32]) -> bool {
    edges.iter().any(|&e| !(e > 0.0))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn tri_patches(p in part(), out in tri_out(), e in prop::array::uniform3(factor()), i in factor()) {
        let t = tessellate(p, out, Factors::Tri { edges: e, inside: i });
        if culled(&e) {
            prop_assert_eq!(&t, &Tessellation::default());
            return Ok(());
        }
        prop_assert!(!t.points.is_empty());
        basic(&t, true, p).map_err(TestCaseError::fail)?;
        match out {
            Output::Point => prop_assert_eq!(t.indices.clone(), (0..kome::num::u32_of(t.points.len())).collect::<Vec<_>>()),
            Output::TriangleCw | Output::TriangleCcw => covers(&t, 0.5, out).map_err(TestCaseError::fail)?,
            Output::Line => {}
        }
        // the corners always
        for c in [[0.0f32, 1.0], [0.0, 0.0], [1.0, 0.0]] {
            prop_assert!(t.points.contains(&c), "corner {:?} missing", c);
        }
    }

    #[test]
    fn quad_patches(p in part(), out in tri_out(), e in prop::array::uniform4(factor()), i in prop::array::uniform2(factor())) {
        let t = tessellate(p, out, Factors::Quad { edges: e, inside: i });
        if culled(&e) {
            prop_assert_eq!(&t, &Tessellation::default());
            return Ok(());
        }
        basic(&t, false, p).map_err(TestCaseError::fail)?;
        match out {
            Output::Point => prop_assert_eq!(t.indices.clone(), (0..kome::num::u32_of(t.points.len())).collect::<Vec<_>>()),
            Output::TriangleCw | Output::TriangleCcw => covers(&t, 1.0, out).map_err(TestCaseError::fail)?,
            Output::Line => {}
        }
        for c in [[0.0f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            prop_assert!(t.points.contains(&c), "corner {:?} missing", c);
        }
    }

    /// Every edge of a tri or quad patch is its factor's 1D points (so patches that share an
    /// edge factor share its points), and those are symmetric: p and 1 - p.
    #[test]
    fn an_edge_is_its_own_factors(p in part(), e in prop::array::uniform4(factor()), i in prop::array::uniform2(factor()), quad in any::<bool>()) {
        let edges: &[f32] = if quad { &e } else { &e[..3] };
        prop_assume!(!culled(edges));
        let t = if quad {
            tessellate(p, Output::TriangleCw, Factors::Quad { edges: e, inside: i })
        } else {
            tessellate(p, Output::TriangleCcw, Factors::Tri { edges: [e[0], e[1], e[2]], inside: i[0] })
        };
        edges_are_their_own(p, &t, edges).map_err(TestCaseError::fail)?;
        for &f in edges {
            let one = edge_1d(p, f);
            let mut mirrored: Vec<f32> = one.iter().map(|x| 1.0 - x).collect();
            mirrored.reverse();
            prop_assert_eq!(&one, &mirrored);
        }
    }

    /// Integer and pow2 partitioning are one to the hardware, and a factor is its ceiling.
    #[test]
    fn integer_is_pow2_and_its_ceiling(out in tri_out(), e in prop::array::uniform4(factor()), i in prop::array::uniform2(factor())) {
        let q = |p, e: [f32; 4], i: [f32; 2]| tessellate(p, out, Factors::Quad { edges: e, inside: i });
        prop_assert_eq!(q(Partitioning::Integer, e, i), q(Partitioning::Pow2, e, i));
        let ce = e.map(|x| if x > 0.0 && x < 64.0 { x.ceil() } else { x });
        let ci = i.map(|x| if x > 0.0 && x < 64.0 { x.ceil() } else { x });
        prop_assert_eq!(q(Partitioning::Integer, e, i), q(Partitioning::Integer, ce, ci));
    }

    /// Fractional partitioning at an integer of its parity is integer partitioning (odd ones
    /// from 3: fractional odd keeps an inside factor of 1 odd and above 1 beside larger edges,
    /// where integer partitioning takes 1 as even).
    #[test]
    fn fractional_at_its_integers(out in tri_out(), k in 2u32..32, rest in prop::array::uniform3(2u32..32)) {
        let odd = |n: u32| kome::num::to_f32(2 * n - 1);
        let even = |n: u32| kome::num::to_f32(2 * n);
        for (p, f) in [(Partitioning::FractionalOdd, &odd as &dyn Fn(u32) -> f32), (Partitioning::FractionalEven, &even)] {
            let e = [f(k), f(rest[0]), f(rest[1])];
            let i = f(rest[2]);
            prop_assert_eq!(tessellate(p, out, Factors::Tri { edges: e, inside: i }), tessellate(Partitioning::Integer, out, Factors::Tri { edges: e, inside: i }), "{:?}", p);
        }
    }

    /// Isolines: ceil(density) lines (1..64, the last at V == 1 left out... the first at 0),
    /// each of the detail factor's points, joined in order.
    #[test]
    fn isolines(p in part(), density in factor(), detail in factor(), lines in any::<bool>()) {
        let out = if lines { Output::Line } else { Output::Point };
        let t = tessellate(p, out, Factors::Isoline { density, detail });
        if culled(&[density, detail]) {
            prop_assert_eq!(&t, &Tessellation::default());
            return Ok(());
        }
        basic(&t, false, p).map_err(TestCaseError::fail)?;
        let n = kome::num::trunc_u32(density.clamp(1.0, 64.0).ceil());
        let mut vs: Vec<u32> = t.points.iter().map(|q| q[1].to_bits()).collect();
        let per = vs.iter().take_while(|&&v| v == vs[0]).count();
        prop_assert_eq!(t.points.len(), per * us(n));
        vs.dedup();
        prop_assert_eq!(f32::from_bits(vs[0]), 0.0);
        for line in t.points.chunks(per) {
            prop_assert_eq!(line[0][0], 0.0);
            prop_assert_eq!(line[per - 1][0], 1.0);
            prop_assert!(line.windows(2).all(|w| w[0][0] <= w[1][0] && w[0][1] == w[1][1]));
        }
        if lines {
            prop_assert_eq!(t.indices.len(), us(n) * (per - 1) * 2);
        }
    }

    /// 15.16 fixed point: the nearest 2^-16 (ties to even), 0 for NaN and negatives, the most
    /// for 2^15 and up.
    #[test]
    fn fixed_point(x in prop_oneof![0.0f32..40000.0, any::<f32>()]) {
        let want = if x.is_nan() || x <= 0.0 {
            0
        } else if x >= 32768.0 {
            u32::MAX
        } else {
            let r = (f64::from(x) * 65536.0).round_ties_even();
            kome::num::trunc_u32(r)
        };
        prop_assert_eq!(to_fixed(x).0, want, "{}", x);
    }
}

/// Uniform integer factors give the counts Direct3D's concentric rings make: a quad (n + 1)²
/// points and 2n² triangles; a tri rings of 3k points (k = n, n - 2, ... above 1) and a center
/// (a triangle for odd n, a point for even), 6k - 6 triangles between a ring and the next, the
/// center 1 (odd) or 6 (even).
#[test]
fn uniform_counts() {
    for n in 1u32..=64 {
        let f = kome::num::to_f32(n);
        let q = tessellate(Partitioning::Integer, Output::TriangleCw, Factors::Quad { edges: [f; 4], inside: [f; 2] });
        assert_eq!(q.points.len(), us((n + 1) * (n + 1)), "quad {n} points");
        assert_eq!(q.indices.len(), us(6 * n * n), "quad {n} triangles");
        let t = tessellate(Partitioning::Integer, Output::TriangleCw, Factors::Tri { edges: [f; 3], inside: f });
        let (mut points, mut tris, mut k) = (0, 0, n);
        while k > 2 {
            points += 3 * k;
            tris += 6 * k - 6;
            k -= 2;
        }
        match k {
            1 => {
                points += 3;
                tris += 1;
            }
            _ => {
                points += 7;
                tris += 6;
            }
        }
        assert_eq!(t.points.len(), us(points), "tri {n} points");
        assert_eq!(t.indices.len(), us(3 * tris), "tri {n} triangles");
    }
}

/// The points and triangles of chosen patches (a golden: tess.rs's own answers, pinned so a
/// change shows; Windows' are winref d3d11tess's).
#[test]
fn golden_patches() {
    let mut s = String::new();
    let cases: &[(Partitioning, Output, Factors)] = &[
        (Partitioning::Integer, Output::TriangleCw, Factors::Tri { edges: [1.0; 3], inside: 1.0 }),
        (Partitioning::Integer, Output::TriangleCw, Factors::Tri { edges: [2.0, 3.0, 4.0], inside: 3.0 }),
        (Partitioning::Integer, Output::TriangleCcw, Factors::Tri { edges: [3.0; 3], inside: 2.0 }),
        (Partitioning::FractionalOdd, Output::TriangleCw, Factors::Tri { edges: [2.5, 1.0, 3.7], inside: 1.2 }),
        (Partitioning::FractionalEven, Output::TriangleCw, Factors::Tri { edges: [2.5, 3.0, 5.25], inside: 4.5 }),
        (Partitioning::Integer, Output::TriangleCw, Factors::Quad { edges: [1.0; 4], inside: [1.0; 2] }),
        (Partitioning::Integer, Output::TriangleCw, Factors::Quad { edges: [2.0, 3.0, 4.0, 1.0], inside: [3.0, 2.0] }),
        (Partitioning::FractionalOdd, Output::TriangleCcw, Factors::Quad { edges: [1.0, 1.0, 1.0, 1.0], inside: [1.5, 1.0] }),
        (Partitioning::FractionalEven, Output::TriangleCw, Factors::Quad { edges: [2.2, 3.3, 4.4, 5.5], inside: [6.6, 2.0] }),
        (Partitioning::Pow2, Output::Point, Factors::Quad { edges: [3.0; 4], inside: [5.0, 2.0] }),
        (Partitioning::Integer, Output::Line, Factors::Isoline { density: 2.5, detail: 3.0 }),
        (Partitioning::FractionalOdd, Output::Point, Factors::Isoline { density: 1.0, detail: 2.5 }),
    ];
    for (p, o, f) in cases {
        let t = tessellate(*p, *o, *f);
        s.push_str(&format!("{p:?} {o:?} {f:?}\n points"));
        for q in &t.points {
            s.push_str(&format!(" {},{}", q[0], q[1]));
        }
        s.push_str("\n indices");
        for i in &t.indices {
            s.push_str(&format!(" {i}"));
        }
        s.push('\n');
    }
    // and a hash of every integer quad and tri up to 16
    let mut h = kome::golden::Fnv::new();
    for p in PARTS {
        for a in 1u32..=16 {
            for b in 1u32..=16 {
                let (fa, fb) = (kome::num::to_f32(a) * 1.25, kome::num::to_f32(b) * 0.75);
                for f in [Factors::Tri { edges: [fa, fb, fa], inside: fb }, Factors::Quad { edges: [fa, fb, fb, fa], inside: [fb, fa] }] {
                    let t = tessellate(p, Output::TriangleCw, f);
                    h.words(t.points.iter().flat_map(|q| [q[0].to_bits(), q[1].to_bits()])).words(t.indices.iter().copied());
                }
            }
        }
    }
    s.push_str(&format!("sweep {:016x}\n", h.get()));
    kome::golden::check(concat!(env!("CARGO_MANIFEST_DIR"), "/src/d3d11/tess_golden.txt"), &s);
}

