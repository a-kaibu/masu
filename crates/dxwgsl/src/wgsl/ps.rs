//! PSInvocations as Windows counts them (WARP, the Direct3D 11 device of the Windows runner):
//! not the pixels drawn but the 2x2 quads WARP's rasterizers hand the pixel shader (clean-room
//! notes kikizake-re/d3d11/pipeline-stats.md), a quad two triangles share counted for both
//! (winref d3d11gs "pipeline statistics of triangles in clip space": PS 544 for a triangle of
//! 32 by 32 pixels, 1072 for one the right edge cuts into two, 3772 for one across two edges).
//!
//! The geometry follows WARP's clipper and setup (clean-room notes kikizake-re/d3d11/clip.md):
//! - the triangle is rotated so the corner with the smallest clip-space (x, y) comes first;
//! - the clip codes are the sign bits of z, w - z (with depth clip), w + x, w - x, w - y, w + y
//!   (no guard band); a triangle outside one plane is dropped; with a code set it is clipped
//!   (Sutherland-Hodgman, a point on a plane inside) against only the planes the corners' codes
//!   name, in that order, interpolating from the inside corner in double precision, the later
//!   planes' distances interpolated as well;
//! - the polygon is drawn as a fan from its first point; a new point's x and y are clamped to
//!   ±w (w at least FLT_MIN) before the divide; every point is snapped to 1/256 of a pixel,
//!   rounding to the nearest (even);
//! - each fan triangle is culled by the sign of its doubled area in those units (0: dropped).
//!
//! A triangle's quads are those of its edge table: for each pixel row, the span from the left
//! edges' x to the right edges' (each the first pixel whose centre is at or right of the edge),
//! the x of an edge at the row's centre by a fixed-point DDA (slope floor((dx << 42) / dy), the
//! start plus 2^49 - 1, x = the accumulator >> 50: x0 << 42 the edge's top), clamped to the
//! drawable columns; rows outside the triangle hold (0x7fff, 0). In each quad row, the quads from
//! min(the two rows' starts) >> 1 to (max(their ends) + 1) >> 1 (out), 4 each.
//!
//! Points: a point inside the clip volume covers the pixel whose centre is in the 1-pixel square
//! around it (left and top sides in): one quad.
//!
//! Lines are not counted by the pixels they light: WARP's line rasterizer (PixelJITRasterizeLine
//! and SetupLine, clean-room notes kikizake-re/d3d11/lines.md 1.3 and pipeline-stats.md) runs
//! the pixel shader, which tests each pixel's line coverage itself, on every quad of a region
//! around the line, and adds 2 for each of those quads (winref d3d11gs: a 32-pixel vertical
//! line 114 = 19 quad rows of 3 quads, two of them 228; the line strip 438):
//! - quad rows from (min(floor(y0 - 0.5), floor(y1 - 0.5)) - 2) / 2 (dividing toward zero) to
//!   (max(..) + 3) >> 1 (out), within the drawable rows (top >> 1 to (bottom + 1) >> 1);
//! - columns from (floor(min x - 0.5) & !1) - 2 to ((ceil(max x - 0.5) + 1) & !1) + 2 (pixels),
//!   within the drawable ones;
//! - in quad row q, unless the line is level (dy = 0: every column), the pixels from floor(xa)
//!   - 2 to ceil(xb) + 2, xa and xb the smaller and larger of slope * (2q - 2) + icpt and slope *
//!   (2q + 2) + icpt (slope = dx / dy, icpt = x0 - slope * y0, f32, each row 2 * slope on), held
//!   to the columns; the quads from the first's (>> 1) to the last's ((p + 1) >> 1, out).
//!
//! [`invocations`] is the rule in Rust (tests); [`WGSL`] the same for the clip pass, in pairs of
//! invocations (a quad of a triangle or a point 2, a visited quad of a line 1).

use kome::num::{to_f32, trunc_i32};

/// D3D11_CULL_MODE.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cull {
    None,
    Front,
    Back,
}

impl Cull {
    /// Of D3D11_CULL_MODE's value (1 none, 2 front, 3 back).
    pub fn of(v: u32) -> Option<Cull> {
        match v {
            1 => Some(Cull::None),
            2 => Some(Cull::Front),
            3 => Some(Cull::Back),
            _ => None,
        }
    }

    /// Its value in the clip pass's uniform (xd RASTER bits 0..2).
    pub fn word(self) -> u32 {
        match self {
            Cull::None => 1,
            Cull::Front => 2,
            Cull::Back => 3,
        }
    }
}

/// What the rasterizer is set to for a draw.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Raster {
    /// the viewport: TopLeftX, TopLeftY, Width, Height
    pub viewport: [f32; 4],
    pub cull: Cull,
    pub front_ccw: bool,
    pub depth_clip: bool,
    /// the pixels that can be drawn (the viewport's, the render target's and the scissor
    /// rectangle's): left, top, right, bottom (right and bottom out)
    pub rect: [i32; 4],
}

/// The pixels that can be drawn (Raster::rect): those whose centres are inside the viewport
/// (TopLeftX, TopLeftY, Width, Height ..), inside the target of `target` pixels (none: any) and,
/// given one, the scissor rectangle (left, top, right, bottom).
pub fn rect(viewport: &[f32; 6], target: Option<(u32, u32)>, scissor: Option<[i32; 4]>) -> [i32; 4] {
    let [x, y, w, h, ..] = *viewport;
    // (the first pixel whose centre i + 0.5 is at or past v)
    let from = |v: f32| trunc_i32((v - 0.5).ceil());
    let mut r = [from(x), from(y), from(x + w), from(y + h)];
    if let Some((tw, th)) = target {
        let (tw, th) = (i32::try_from(tw).unwrap_or(i32::MAX), i32::try_from(th).unwrap_or(i32::MAX));
        r = [r[0].max(0), r[1].max(0), r[2].min(tw), r[3].min(th)];
    }
    if let Some([l, t, rr, b]) = scissor {
        r = [r[0].max(l), r[1].max(t), r[2].min(rr), r[3].min(b)];
    }
    r
}

/// A point of a polygon being clipped: its clip-space position, its distances inside the six
/// planes.
#[derive(Clone, Copy, Debug)]
struct Pt {
    p: [f32; 4],
    d: [f32; 6],
    /// made by the clipper (its screen position is clamped)
    new: bool,
}

/// The six planes' distances of a clip-space position (near and far 0 without depth clip).
fn distances(p: &[f32; 4], depth_clip: bool) -> [f32; 6] {
    let [x, y, z, w] = *p;
    let (n, f) = if depth_clip { (z, w - z) } else { (0.0, 0.0) };
    [n, f, w + x, w - x, w - y, w + y]
}

/// WARP's interpolation from the inside value `a` toward the outside `b` (dIn, dOut the
/// distances): `(f32(b - a) * dIn) / f32(dIn - dOut) + a` in double, rounded once.
fn lerp(a: f32, b: f32, d_in: f32, den: f32) -> f32 {
    to_f32(f64::from(b - a) * f64::from(d_in) / f64::from(den) + f64::from(a))
}

fn interpolate(i: &Pt, o: &Pt, k: usize, planes: u32) -> Pt {
    let d_in = i.d.get(k).copied().unwrap_or(0.0);
    let d_out = o.d.get(k).copied().unwrap_or(0.0);
    let den = d_in - d_out;
    let p = std::array::from_fn(|c| lerp(i.p.get(c).copied().unwrap_or(0.0), o.p.get(c).copied().unwrap_or(0.0), d_in, den));
    let d = std::array::from_fn(|q| {
        let (a, b) = (i.d.get(q).copied().unwrap_or(0.0), o.d.get(q).copied().unwrap_or(0.0));
        if q > k && planes >> q & 1 != 0 { lerp(a, b, d_in, den) } else { a }
    });
    Pt { p, d, new: true }
}

/// a == b as the machine compares floats (WARP's tie tests are exact)
fn same(a: f32, b: f32) -> bool {
    a.partial_cmp(&b) == Some(std::cmp::Ordering::Equal)
}

/// cvtss2si: round to the nearest even; NaN or out of range: i32::MIN.
fn cvt(v: f32) -> i32 {
    let two31 = f32::from_bits(0x4f00_0000);
    if v.is_nan() || v >= two31 || v < -two31 { i32::MIN } else { trunc_i32(v.round_ties_even()) }
}

/// A point's screen position in 1/256 pixel units.
fn snap(pt: &Pt, vp: &[f32; 4]) -> (i32, i32) {
    let [x, y, _, w] = pt.p;
    let w = if f32::MIN_POSITIVE > w { f32::MIN_POSITIVE } else { w };
    let (x, y) = if pt.new { (x.max(-w).min(w), y.max(-w).min(w)) } else { (x, y) };
    let rhw = 1.0 / w;
    let [vx, vy, vw, vh] = *vp;
    let (sx, ox, sy, oy) = (vw * 0.5, vw * 0.5 + vx, -vh * 0.5, vh * 0.5 + vy);
    (cvt((rhw * x * sx + ox) * 256.0), cvt((rhw * y * sy + oy) * 256.0))
}

/// The clipper's polygon of a triangle (empty: dropped).
fn clip(tri: &[[f32; 4]; 3], depth_clip: bool) -> Vec<Pt> {
    let [a, b, c] = *tri;
    let lt = |u: &[f32; 4], v: &[f32; 4]| u[0] > v[0] || (same(u[0], v[0]) && u[1] > v[1]);
    let s0 = if lt(&a, &b) { 1 } else { 0 };
    let first = [a, b, c].get(s0).copied().unwrap_or(a);
    let sel = if lt(&first, &c) { 2 } else { s0 };
    let rot = match sel {
        1 => [b, c, a],
        2 => [c, a, b],
        _ => [a, b, c],
    };
    let mut poly: Vec<Pt> = rot.iter().map(|p| Pt { p: *p, d: distances(p, depth_clip), new: false }).collect();
    let code = |d: &[f32; 6]| d.iter().enumerate().fold(0u32, |m, (k, v)| if v.is_sign_negative() && !v.is_nan() { m | 1 << k } else { m });
    let nan = depth_clip && poly.iter().any(|v| v.d.iter().any(|x| x.is_nan()));
    let codes: Vec<u32> = poly.iter().map(|v| code(&v.d)).collect();
    let or = codes.iter().fold(0, |m, c| m | c);
    let and = codes.iter().fold(0x3f, |m, c| m & c);
    if and != 0 || nan {
        return Vec::new();
    }
    for k in 0..6 {
        if or >> k & 1 == 0 {
            continue;
        }
        let n = poly.len();
        let mut out = Vec::with_capacity(n.saturating_add(1));
        for i in 0..n {
            let (Some(a), Some(b)) = (poly.get(i), poly.get(i.saturating_add(1).checked_rem(n).unwrap_or(0))) else { continue };
            let (da, db) = (a.d.get(k).copied().unwrap_or(0.0), b.d.get(k).copied().unwrap_or(0.0));
            let (a_out, b_out) = (0.0 > da, 0.0 > db);
            if !a_out {
                out.push(*a);
                if b_out {
                    // (the edge from the crossing on lies on the plane)
                    out.push(interpolate(a, b, k, or));
                }
            } else if !b_out {
                out.push(interpolate(b, a, k, or));
            }
        }
        poly = out;
        if poly.is_empty() {
            return poly;
        }
    }
    poly
}

/// The quads WARP's triangle rasterizer hands the pixel shader for a fan triangle, times 4 (0
/// when culled).
fn triangle(pts: [(i32, i32); 3], r: &Raster) -> u64 {
    let v = pts.map(|(x, y)| (i64::from(x), i64::from(y)));
    let [(x0, y0), (x1, y1), (x2, y2)] = v;
    // (coordinates of at most 2^31 each: the products fit in i128)
    let det = i128::from(y2.saturating_sub(y0)).saturating_mul(i128::from(x1.saturating_sub(x0))).saturating_sub(i128::from(y0.saturating_sub(y1)).saturating_mul(i128::from(x0.saturating_sub(x2))));
    if det == 0 {
        return 0;
    }
    let front = if r.front_ccw { det < 0 } else { det > 0 };
    let drawn = match r.cull {
        Cull::None => true,
        Cull::Front => !front,
        Cull::Back => front,
    };
    if !drawn {
        return 0;
    }
    spans(&v, r)
}

/// The x of the edge from `t` (its top) to `b` at row centre `yc` (1/256 units): the first pixel
/// whose centre is at or right of it, by WARP's DDA (the module's comment).
fn edge_x(t: (i64, i64), b: (i64, i64), yc: i64) -> i64 {
    let dy = u64::try_from(b.1.saturating_sub(t.1)).unwrap_or(0).max(1);
    // (the unsigned 64-bit x86 arithmetic: dx zero-extended from 32 bits, shifts wrap)
    let du = |d: i64| u64::from(lo32u(d)) << 42;
    let slope = if t.0 <= b.0 { du(b.0.wrapping_sub(t.0)).checked_div(dy).unwrap_or(0).cast_signed() } else { du(t.0.wrapping_sub(b.0)).wrapping_add(dy).checked_div(dy).unwrap_or(0).cast_signed().wrapping_neg() };
    yc.wrapping_sub(t.1).wrapping_mul(slope).wrapping_add(0x1_ffff_ffff_ffff).wrapping_add(t.0.wrapping_shl(42)) >> 50
}

/// The low 32 bits of `v` (x86's 32-bit registers).
fn lo32u(v: i64) -> u32 {
    u32::try_from(v & 0xffff_ffff).unwrap_or(0)
}

/// A logical shift right by 1 of a 32-bit value (the rasterizers' shr).
fn shr1(v: i64) -> i64 {
    i64::from(lo32u(v) >> 1)
}

/// WARP's triangle edge table counted by quad rows (the module's comment), times 4.
fn spans(v: &[(i64, i64); 3], r: &Raster) -> u64 {
    // (the top corner: the smallest y, then x)
    let mut ti = 0usize;
    for (i, c) in v.iter().enumerate() {
        let tc = v.get(ti).copied().unwrap_or(*c);
        if c.1 < tc.1 || (c.1 == tc.1 && c.0 < tc.0) {
            ti = i;
        }
    }
    let at = |k: usize| v.get(k.checked_rem(3).unwrap_or(0)).copied().unwrap_or((0, 0));
    let (t, p, q) = (at(ti), at(ti.saturating_add(1)), at(ti.saturating_add(2)));
    let cross = i128::from(p.0.saturating_sub(t.0)).saturating_mul(i128::from(q.1.saturating_sub(t.1))).saturating_sub(i128::from(p.1.saturating_sub(t.1)).saturating_mul(i128::from(q.0.saturating_sub(t.0))));
    let (l, rt) = if cross < 0 { (p, q) } else { (q, p) };
    let [cl, ct, cr, cb] = r.rect.map(i64::from);
    let first = (t.1.saturating_add(127) >> 8).max(ct);
    let end = (l.1.max(rt.1).saturating_add(127) >> 8).min(cb);
    if first >= end {
        return 0;
    }
    // (a pixel row's span; a row of the quad rows outside the triangle (0x7fff, 0))
    let row = |j: i64| -> (i64, i64) {
        if j < first || j >= end {
            return (0x7fff, 0);
        }
        let yc = j.saturating_mul(256).saturating_add(128);
        // (an edge ends at the row whose centre is at or below its lower corner)
        let lx = if l.1 <= rt.1 && yc >= l.1 { edge_x(l, rt, yc) } else { edge_x(t, l, yc) };
        let rx = if rt.1 < l.1 && yc >= rt.1 { edge_x(rt, l, yc) } else { edge_x(t, rt, yc) };
        (lx.max(cl).min(cr), rx.max(cl).min(cr))
    };
    let mut n = 0i64;
    let mut j = first & !1;
    while j < end {
        let (a, b) = (row(j), row(j.saturating_add(1)));
        n = n.wrapping_add(shr1(a.1.max(b.1).wrapping_add(1)).wrapping_sub(shr1(a.0.min(b.0))));
        j = j.saturating_add(2);
    }
    u64::from(lo32u(n.wrapping_mul(4)))
}

/// The invocations WARP counts for a line between snapped ends (the module's comment): 2 for
/// each quad of the region its rasterizer visits.
fn line_visits(a: (i32, i32), b: (i32, i32), r: &Raster) -> u64 {
    if a == b {
        return 0;
    }
    let e = 1.0f32 / 256.0;
    let (x0, y0, x1, y1) = (to_f32(a.0) * e, to_f32(a.1) * e, to_f32(b.0) * e, to_f32(b.1) * e);
    let fl = |v: f32| trunc_i32(v.floor());
    // (a logical shift right by 1, as the rasterizer's shr)
    let half = |v: i32| (v.cast_unsigned() >> 1).cast_signed();
    let (ya, yb) = (fl(y0 - 0.5), fl(y1 - 0.5));
    let row0 = ya.min(yb).saturating_sub(2) / 2;
    let row1 = half(ya.max(yb).wrapping_add(3));
    let (lo, hi) = if x1 > x0 { (x0 - 0.5, x1 - 0.5) } else { (x1 - 0.5, x0 - 0.5) };
    let col0 = (fl(lo) & !1).saturating_sub(2);
    let col1 = (trunc_i32(hi.ceil()).saturating_add(1) & !1).saturating_add(2);
    let dy = y1 - y0;
    let slope = if dy == 0.0 { 0.0 } else { (x1 - x0) / dy };
    let icpt = if dy == 0.0 { x0 } else { x0 - slope * y0 };
    let [l, t, rr, bb] = r.rect;
    let (c0, c1) = (col0.max(l), col1.min(rr));
    let q0 = row0.max(half(t));
    let q1 = row1.min(half(bb.wrapping_add(1)));
    if q1 <= q0 {
        return 0;
    }
    let (mut xa, mut xb) = if slope == 0.0 {
        (to_f32(c0), to_f32(c1))
    } else {
        let (u, v) = (to_f32(q0.saturating_mul(2).saturating_sub(2)) * slope + icpt, to_f32(q0.saturating_mul(2).saturating_add(2)) * slope + icpt);
        (u.min(v), u.max(v))
    };
    let step = slope * 2.0;
    // (cvtss2si: to the nearest, ties to even; then one down or up to floor or ceil)
    let near = |v: f32| trunc_i32(v.round_ties_even());
    let mut n = 0u64;
    for _ in q0..q1 {
        let fa = near(xa);
        let from = (if xa < to_f32(fa) { fa.saturating_sub(1) } else { fa }).saturating_sub(2);
        let fb = near(xb);
        let to = (if to_f32(fb) < xb { fb.saturating_add(1) } else { fb }).saturating_add(2);
        let first = from.max(c0).min(c1);
        let last = to.max(c0).min(c1);
        let (qa, qb) = (half(first), half(first.max(last).wrapping_add(1)));
        n = n.saturating_add(u64::from(qb.saturating_sub(qa).cast_unsigned()).saturating_mul(2));
        xa += step;
        xb += step;
    }
    n
}

/// The pixel shader invocations Windows counts for one primitive of 1 (point), 2 (line) or 3
/// (triangle) clip-space corners.
pub fn invocations(p: &[[f32; 4]], r: &Raster) -> u64 {
    match *p {
        [a, b, c] => {
            let poly = clip(&[a, b, c], r.depth_clip);
            let Some(first) = poly.first() else { return 0 };
            let s0 = snap(first, &r.viewport);
            let n = poly.len();
            let mut sum = 0u64;
            for i in 1..n.saturating_sub(1) {
                let (Some(pa), Some(pb)) = (poly.get(i), poly.get(i.saturating_add(1))) else { continue };
                sum = sum.saturating_add(triangle([s0, snap(pa, &r.viewport), snap(pb, &r.viewport)], r));
            }
            sum
        }
        [a, b] => {
            let mut ends = [Pt { p: a, d: distances(&a, r.depth_clip), new: false }, Pt { p: b, d: distances(&b, r.depth_clip), new: false }];
            for k in 0..6 {
                let [s, e] = ends;
                let (ds, de) = (s.d.get(k).copied().unwrap_or(0.0), e.d.get(k).copied().unwrap_or(0.0));
                match (0.0 > ds, 0.0 > de) {
                    (true, true) => return 0,
                    (false, false) => {}
                    (true, false) => ends = [interpolate(&e, &s, k, 0), e],
                    (false, true) => ends = [s, interpolate(&s, &e, k, 0)],
                }
                // (a line's later distances: of the new end's position)
                for v in &mut ends {
                    v.d = distances(&v.p, r.depth_clip);
                }
            }
            let [s, e] = ends;
            line_visits(snap(&s, &r.viewport), snap(&e, &r.viewport), r)
        }
        [a] => {
            if distances(&a, r.depth_clip).iter().any(|d| 0.0 > *d || d.is_nan()) {
                return 0;
            }
            let (x, y) = snap(&Pt { p: a, d: [0.0; 6], new: false }, &r.viewport);
            // (the pixel whose centre is in [x - 0.5, x + 0.5) by [y - 0.5, y + 0.5))
            let cell = |v: i32| i64::from(v).saturating_add(255).div_euclid(256).saturating_sub(1);
            let (i, j) = (cell(x), cell(y));
            let [l, t, rr, b] = r.rect.map(i64::from);
            if i >= l && i < rr && j >= t && j < b { 4 } else { 0 }
        }
        _ => 0,
    }
}

/// [`invocations`] in WGSL, over the uniform's raster words (xd VIEWPORT, RASTER, RECT): `fn
/// ps_pairs(p: array<vec4<f32>, 3>, n: u32) -> u32`, invocations / 2. The 64-bit
/// products of the 1/256-unit coordinates are pairs of u32 (m64); the double-precision
/// interpolation is f32 pairs (Dekker's products and Knuth's sums: 48 bits, one rounding to f32).
pub const WGSL: &str = r#"
fn m64(a: i32, b: i32) -> vec2<u32> {
    let ua = u32(abs(a));
    let ub = u32(abs(b));
    let a0 = ua & 0xffffu; let a1 = ua >> 16u; let b0 = ub & 0xffffu; let b1 = ub >> 16u;
    let p00 = a0 * b0; let p01 = a0 * b1; let p10 = a1 * b0; let p11 = a1 * b1;
    let mid = (p00 >> 16u) + (p01 & 0xffffu) + (p10 & 0xffffu);
    let lo = (p00 & 0xffffu) | (mid << 16u);
    let hi = p11 + (p01 >> 16u) + (p10 >> 16u) + (mid >> 16u);
    if ((a < 0) != (b < 0)) {
        let nl = ~lo + 1u;
        return vec2<u32>(nl, ~hi + select(0u, 1u, nl == 0u));
    }
    return vec2<u32>(lo, hi);
}
// the sign of a * b - c * d
fn s64(a: i32, b: i32, c: i32, d: i32) -> i32 {
    let x = m64(a, b);
    let y = m64(c, d);
    let lo = x.x - y.x;
    let hi = x.y - y.y - select(0u, 1u, x.x < y.x);
    if (bitcast<i32>(hi) < 0) { return -1; }
    if (hi == 0u && lo == 0u) { return 0; }
    return 1;
}
fn split2(a: f32) -> vec2<f32> {
    let c = 4097.0f * a;
    let hi = c - (c - a);
    return vec2<f32>(hi, a - hi);
}
// a * b as p + e exactly
fn prod2(a: f32, b: f32) -> vec2<f32> {
    let p = a * b;
    let x = split2(a);
    let y = split2(b);
    return vec2<f32>(p, ((x.x * y.x - p) + x.x * y.y + x.y * y.x) + x.y * y.y);
}
// WARP's (f32(b - a) * dIn) / den + a, in double, rounded once
fn lerp2(a: f32, b: f32, din: f32, den: f32) -> f32 {
    let pr = prod2(b - a, din);
    let q = pr.x / den;
    let qd = prod2(q, den);
    let ql = (((pr.x - qd.x) - qd.y) + pr.y) / den;
    let s = q + a;
    let bb = s - q;
    let e = (q - (s - bb)) + (a - bb);
    return s + (e + ql);
}
fn ps_dist(p: vec4<f32>, k: u32, dc: bool) -> f32 {
    switch k {
        case 0u: { return select(0.0f, p.z, dc); }
        case 1u: { return select(0.0f, p.w - p.z, dc); }
        case 2u: { return p.w + p.x; }
        case 3u: { return p.w - p.x; }
        case 4u: { return p.w - p.y; }
        default: { return p.w + p.y; }
    }
}
fn ps_neg(v: f32) -> bool { return (bitcast<u32>(v) >> 31u) != 0u && v == v; }
fn cvt_ss(v: f32) -> i32 {
    if (!(v < 2147483648.0f && v >= -2147483648.0f)) { return i32(-2147483647) - 1; }
    return i32(round(v));
}
fn ps_snap(p: vec4<f32>, new_pt: bool) -> vec2<i32> {
    let w = select(p.w, 1.17549435e-38f, 1.17549435e-38f > p.w);
    var x = p.x;
    var y = p.y;
    if (new_pt) { x = min(max(x, -w), w); y = min(max(y, -w), w); }
    let rhw = 1.0f / w;
    let vx = bitcast<f32>(xw(XD_VIEWPORT)); let vy = bitcast<f32>(xw(XD_VIEWPORT + 1u));
    let vw = bitcast<f32>(xw(XD_VIEWPORT + 2u)); let vh = bitcast<f32>(xw(XD_VIEWPORT + 3u));
    let sx = vw * 0.5f; let ox = vw * 0.5f + vx; let sy = -vh * 0.5f; let oy = vh * 0.5f + vy;
    return vec2<i32>(cvt_ss((rhw * x * sx + ox) * 256.0f), cvt_ss((rhw * y * sy + oy) * 256.0f));
}
fn ps_rect(k: u32) -> i32 { return bitcast<i32>(xw(XD_RECT + k)); }
// 64-bit two's complement integers as (low, high) u32 pairs: WARP's triangle DDA
fn d_add(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let lo = a.x + b.x;
    return vec2<u32>(lo, a.y + b.y + select(0u, 1u, lo < a.x));
}
fn d_neg(a: vec2<u32>) -> vec2<u32> { return d_add(~a, vec2<u32>(1u, 0u)); }
// (the full product of two u32)
fn d_mul32(a: u32, b: u32) -> vec2<u32> {
    let al = a & 0xffffu; let ah = a >> 16u; let bl = b & 0xffffu; let bh = b >> 16u;
    let ll = al * bl; let lh = al * bh; let hl = ah * bl; let hh = ah * bh;
    let mid = (ll >> 16u) + (lh & 0xffffu) + (hl & 0xffffu);
    return vec2<u32>((ll & 0xffffu) | (mid << 16u), hh + (lh >> 16u) + (hl >> 16u) + (mid >> 16u));
}
// (a times k, wrapping)
fn d_mul(a: vec2<u32>, k: u32) -> vec2<u32> {
    let p = d_mul32(a.x, k);
    return vec2<u32>(p.x, p.y + a.y * k);
}
// (n / d, unsigned, d below 2^31)
fn d_div(n: vec2<u32>, d: u32) -> vec2<u32> {
    var q = vec2<u32>(0u, 0u);
    var r = 0u;
    for (var i = 63; i >= 0; i -= 1) {
        let bit = select((n.x >> u32(i)) & 1u, (n.y >> u32(i - 32)) & 1u, i >= 32);
        r = (r << 1u) | bit;
        if (r >= d) {
            r -= d;
            if (i >= 32) { q.y |= 1u << u32(i - 32); } else { q.x |= 1u << u32(i); }
        }
    }
    return q;
}
// the first pixel whose centre is at or right of the edge t (top) to b at row centre yc
fn ps_edge_x(t: vec2<i32>, b: vec2<i32>, yc: i32) -> i32 {
    let dy = u32(max(b.y - t.y, 1));
    var slope: vec2<u32>;
    if (t.x <= b.x) { slope = d_div(vec2<u32>(0u, bitcast<u32>(b.x - t.x) << 10u), dy); }
    else { slope = d_neg(d_div(d_add(vec2<u32>(0u, bitcast<u32>(t.x - b.x) << 10u), vec2<u32>(dy, 0u)), dy)); }
    let k = yc - t.y;
    var acc = d_mul(slope, u32(abs(k)));
    if (k < 0) { acc = d_neg(acc); }
    acc = d_add(d_add(acc, vec2<u32>(0xffffffffu, 0x1ffffu)), vec2<u32>(0u, bitcast<u32>(t.x) << 10u));
    return bitcast<i32>(acc.y) >> 18u;
}
fn ps_shr1(v: i32) -> i32 { return bitcast<i32>(bitcast<u32>(v) >> 1u); }
// the quads of a fan triangle (culled: 0): WARP's edge table by quad rows (spans)
fn ps_tri(v0: vec2<i32>, v1: vec2<i32>, v2: vec2<i32>) -> u32 {
    let det = s64(v2.y - v0.y, v1.x - v0.x, v0.y - v1.y, v0.x - v2.x);
    if (det == 0) { return 0u; }
    let bits = xw(XD_RASTER);
    let ccw = (bits & 4u) != 0u;
    let front = select(det > 0, det < 0, ccw);
    let cull = bits & 3u;
    if ((cull == 2u && front) || (cull == 3u && !front)) { return 0u; }
    let v = array<vec2<i32>, 3>(v0, v1, v2);
    var ti = 0u;
    for (var i = 1u; i < 3u; i += 1u) {
        if (v[i].y < v[ti].y || (v[i].y == v[ti].y && v[i].x < v[ti].x)) { ti = i; }
    }
    let t = v[ti];
    let p = v[(ti + 1u) % 3u];
    let q = v[(ti + 2u) % 3u];
    let cross = s64(p.x - t.x, q.y - t.y, p.y - t.y, q.x - t.x);
    var l = q; var rt = p;
    if (cross < 0) { l = p; rt = q; }
    let cl = ps_rect(0u); let ct = ps_rect(1u); let cr = ps_rect(2u); let cb = ps_rect(3u);
    let first = max((t.y + 127) >> 8u, ct);
    let end = min((max(l.y, rt.y) + 127) >> 8u, cb);
    if (first >= end) { return 0u; }
    var n = 0i;
    for (var j = first & ~1; j < end; j += 2) {
        var lo = 0x7fff; var hi = 0;
        for (var k = 0; k < 2; k += 1) {
            let jj = j + k;
            if (jj < first || jj >= end) { lo = min(lo, 0x7fff); hi = max(hi, 0); continue; }
            let yc = jj * 256 + 128;
            var lx: i32; var rx: i32;
            if (l.y <= rt.y && yc >= l.y) { lx = ps_edge_x(l, rt, yc); } else { lx = ps_edge_x(t, l, yc); }
            if (rt.y < l.y && yc >= rt.y) { rx = ps_edge_x(rt, l, yc); } else { rx = ps_edge_x(t, rt, yc); }
            lo = min(lo, min(max(lx, cl), cr));
            hi = max(hi, min(max(rx, cl), cr));
        }
        n += ps_shr1(hi + 1) - ps_shr1(lo);
    }
    return bitcast<u32>(n);
}
// WARP's line rasterizer: the quads of the region it visits (line_visits)
fn ps_half(v: i32) -> i32 { return bitcast<i32>(bitcast<u32>(v) >> 1u); }
fn ps_near(v: f32) -> i32 { return i32(round(v)); }
fn ps_line_visits(a: vec2<i32>, b: vec2<i32>) -> u32 {
    if (all(a == b)) { return 0u; }
    let e = 1.0f / 256.0f;
    let x0 = f32(a.x) * e; let y0 = f32(a.y) * e; let x1 = f32(b.x) * e; let y1 = f32(b.y) * e;
    let ya = i32(floor(y0 - 0.5f)); let yb = i32(floor(y1 - 0.5f));
    let row0 = (min(ya, yb) - 2) / 2;
    let row1 = ps_half(max(ya, yb) + 3);
    var lo = x1 - 0.5f; var hi = x0 - 0.5f;
    if (x1 > x0) { lo = x0 - 0.5f; hi = x1 - 0.5f; }
    let col0 = (i32(floor(lo)) & ~1) - 2;
    let col1 = ((i32(ceil(hi)) + 1) & ~1) + 2;
    let dy = y1 - y0;
    var slope = 0.0f; var icpt = x0;
    if (dy != 0.0f) { slope = (x1 - x0) / dy; icpt = x0 - slope * y0; }
    let c0 = max(col0, ps_rect(0u)); let c1 = min(col1, ps_rect(2u));
    let q0 = max(row0, ps_half(ps_rect(1u)));
    let q1 = min(row1, ps_half(ps_rect(3u) + 1));
    if (q1 <= q0) { return 0u; }
    var xa = f32(c0); var xb = f32(c1);
    if (slope != 0.0f) {
        let u = f32(q0 * 2 - 2) * slope + icpt;
        let v = f32(q0 * 2 + 2) * slope + icpt;
        xa = min(u, v); xb = max(u, v);
    }
    let step = slope * 2.0f;
    var n = 0u;
    for (var q = q0; q < q1; q += 1) {
        let fa = ps_near(xa);
        let lo_px = select(fa, fa - 1, xa < f32(fa)) - 2;
        let fb = ps_near(xb);
        let hi_px = select(fb, fb + 1, f32(fb) < xb) + 2;
        let first = min(max(lo_px, c0), c1);
        let last = min(max(hi_px, c0), c1);
        n += bitcast<u32>(ps_half(max(first, last) + 1) - ps_half(first));
        xa = xa + step; xb = xb + step;
    }
    return n;
}
fn ps_pairs(p: array<vec4<f32>, 3>, n: u32) -> u32 {
    let dc = (xw(XD_RASTER) & 8u) != 0u;
    if ((xw(XD_RASTER) & 16u) == 0u) { return 0u; }
    if (n == 1u) {
        for (var k = 0u; k < 6u; k += 1u) { let d = ps_dist(p[0], k, dc); if (0.0f > d || d != d) { return 0u; } }
        let s = ps_snap(p[0], false);
        let i = ((s.x + 255) >> 8u) - 1;
        let j = ((s.y + 255) >> 8u) - 1;
        return select(0u, 2u, i >= ps_rect(0u) && i < ps_rect(2u) && j >= ps_rect(1u) && j < ps_rect(3u));
    }
    if (n == 2u) {
        var s = p[0];
        var e = p[1];
        var sn = false;
        var en = false;
        for (var k = 0u; k < 6u; k += 1u) {
            let ds = ps_dist(s, k, dc);
            let de = ps_dist(e, k, dc);
            let so = 0.0f > ds;
            let eo = 0.0f > de;
            if (so && eo) { return 0u; }
            if (so) { s = vec4<f32>(lerp2(e.x, s.x, de, de - ds), lerp2(e.y, s.y, de, de - ds), lerp2(e.z, s.z, de, de - ds), lerp2(e.w, s.w, de, de - ds)); sn = true; }
            if (eo) { e = vec4<f32>(lerp2(s.x, e.x, ds, ds - de), lerp2(s.y, e.y, ds, ds - de), lerp2(s.z, e.z, ds, ds - de), lerp2(s.w, e.w, ds, ds - de)); en = true; }
        }
        return ps_line_visits(ps_snap(s, sn), ps_snap(e, en));
    }
    // a triangle: rotated to the smallest (x, y), clipped against its codes' planes
    var r = 0u;
    if (p[0].x > p[1].x || (p[0].x == p[1].x && p[0].y > p[1].y)) { r = 1u; }
    if (p[r].x > p[2].x || (p[r].x == p[2].x && p[r].y > p[2].y)) { r = 2u; }
    var pp: array<vec4<f32>, 12>;
    var pd: array<array<f32, 6>, 12>;
    var pn: array<bool, 12>;
    var qp: array<vec4<f32>, 12>;
    var qd: array<array<f32, 6>, 12>;
    var qn: array<bool, 12>;
    var or_ = 0u;
    var and_ = 0x3fu;
    var nan = false;
    for (var c = 0u; c < 3u; c += 1u) {
        pp[c] = p[(c + r) % 3u];
        pn[c] = false;
        var code = 0u;
        for (var k = 0u; k < 6u; k += 1u) {
            let d = ps_dist(pp[c], k, dc);
            pd[c][k] = d;
            if (ps_neg(d)) { code |= 1u << k; }
            if (dc && d != d) { nan = true; }
        }
        or_ |= code;
        and_ &= code;
    }
    if (and_ != 0u || nan) { return 0u; }
    var m = 3u;
    for (var k = 0u; k < 6u; k += 1u) {
        if (((or_ >> k) & 1u) == 0u) { continue; }
        var o = 0u;
        for (var i = 0u; i < m; i += 1u) {
            let j = (i + 1u) % m;
            let da = pd[i][k];
            let db = pd[j][k];
            let ao = 0.0f > da;
            let bo = 0.0f > db;
            // (in, out: the inside point and the outside one of a crossing)
            var a_in = i;
            var b_out = j;
            var cross = false;
            if (!ao) {
                qp[o] = pp[i]; qd[o] = pd[i]; qn[o] = pn[i]; o += 1u;
                cross = bo;
            } else if (!bo) {
                a_in = j; b_out = i; cross = true;
            }
            if (cross && o < 12u) {
                let din = pd[a_in][k];
                let den = din - pd[b_out][k];
                let a = pp[a_in];
                let b = pp[b_out];
                qp[o] = vec4<f32>(lerp2(a.x, b.x, din, den), lerp2(a.y, b.y, din, den), lerp2(a.z, b.z, din, den), lerp2(a.w, b.w, din, den));
                for (var q = 0u; q < 6u; q += 1u) {
                    qd[o][q] = select(pd[a_in][q], lerp2(pd[a_in][q], pd[b_out][q], din, den), q > k && ((or_ >> q) & 1u) != 0u);
                }
                qn[o] = true;
                o += 1u;
            }
        }
        m = o;
        if (m == 0u) { return 0u; }
        for (var c = 0u; c < m; c += 1u) { pp[c] = qp[c]; pd[c] = qd[c]; pn[c] = qn[c]; }
    }
    let s0 = ps_snap(pp[0], pn[0]);
    var quads = 0u;
    for (var i = 1u; i + 1u < m; i += 1u) {
        quads += ps_tri(s0, ps_snap(pp[i], pn[i]), ps_snap(pp[i + 1u], pn[i + 1u]));
    }
    return quads * 2u;
}
"#;
