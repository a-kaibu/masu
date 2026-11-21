//! PSInvocations as Windows counts them (WARP, the Direct3D 11 device of the Windows runner): a
//! pixel shader runs on whole 2x2 quads of pixels, so each rasterized triangle adds 4 for every
//! quad of the target that holds a pixel it covers, and a quad two triangles share counts for
//! both (winref d3d11gs "pipeline statistics of triangles in clip space": PS 544 for a triangle
//! of 32 by 32 pixels, 1072 for one the right edge cuts into two).
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
//! Which pixels a triangle covers: the pixel centres inside it, a centre on an edge by the
//! top-left rule, except that a centre on an edge of the triangle as the draw gave it (not an
//! edge the clipper or the fan made) counts as covered. That last rule is what Windows'
//! numbers show: "across the near plane" (and far, "one w negative") counts a quad whose only
//! centres lie on the original right edge (496, not 492); a fan's own edge does not ("w zero"
//! 1184, "strip of 6 (2 outside)" 1612). It leaves one of the probe's answers apart:
//! tests/winref/known.txt "d3d11gs" (across two edges: 3764, Windows 3772).
//!
//! Points: a point inside the clip volume covers the pixel whose centre is in the 1-pixel square
//! around it (left and top sides in): one quad. Lines: the pixels WARP's aliased-line test lights
//! (kikizake-re/d3d11/lines.md, the diamond-exit rule), counted by quads as triangles are; the
//! probe's lines answer more (tests/winref/known.txt "d3d11gs" line list and line strip).
//!
//! [`invocations`] is the rule in Rust (tests); [`WGSL`] the same for the clip pass.

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
/// planes, whether the edge from it to the next point is part of an edge of the draw's triangle.
#[derive(Clone, Copy, Debug)]
struct Pt {
    p: [f32; 4],
    d: [f32; 6],
    original: bool,
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
    Pt { p, d, original: false, new: true }
}

/// a == b as the machine compares floats (WARP's tie tests are exact)
fn same(a: f32, b: f32) -> bool {
    a.partial_cmp(&b) == Some(std::cmp::Ordering::Equal)
}

/// The centre of pixel row or column `i` in 1/256 units.
fn centre(i: i64) -> i64 {
    i.saturating_mul(256).saturating_add(128)
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
    let mut poly: Vec<Pt> = rot.iter().map(|p| Pt { p: *p, d: distances(p, depth_clip), original: true, new: false }).collect();
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
                let mut m = interpolate(b, a, k, or);
                m.original = a.original;
                out.push(m);
            }
        }
        poly = out;
        if poly.is_empty() {
            return poly;
        }
    }
    poly
}

/// Is pixel centre (cx, cy) (1/256 units) inside the triangle (doubled area > 0), `orig[e]`:
/// edge e (from corner e to e + 1) one of the draw's own?
fn covers(v: &[(i64, i64); 3], orig: &[bool; 3], cx: i64, cy: i64) -> bool {
    (0..3).all(|e| {
        let (Some(&(ax, ay)), Some(&(bx, by))) = (v.get(e), v.get(e.saturating_add(1).checked_rem(3).unwrap_or(0))) else { return false };
        let (dx, dy) = (bx.saturating_sub(ax), by.saturating_sub(ay));
        let f = dx.saturating_mul(cy.saturating_sub(ay)).saturating_sub(dy.saturating_mul(cx.saturating_sub(ax)));
        let top_left = (dy == 0 && dx > 0) || dy < 0;
        f > 0 || (f == 0 && (top_left || orig.get(e).copied().unwrap_or(false)))
    })
}

/// The quads of `rect` with a pixel a fan triangle covers, times 4 (0 when culled).
fn triangle(pts: [(i32, i32); 3], orig: [bool; 3], r: &Raster) -> u64 {
    let [(x0, y0), (x1, y1), (x2, y2)] = pts.map(|(x, y)| (i64::from(x), i64::from(y)));
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
    // (the same corners in the order that makes the area positive; edge e stays with its ends)
    let (v, o) = if det > 0 { ([(x0, y0), (x1, y1), (x2, y2)], orig) } else { ([(x0, y0), (x2, y2), (x1, y1)], [orig[2], orig[1], orig[0]]) };
    let [l, t, rr, b] = r.rect.map(i64::from);
    let lo_x = v.iter().map(|p| p.0).min().unwrap_or(0);
    let hi_x = v.iter().map(|p| p.0).max().unwrap_or(0);
    let lo_y = v.iter().map(|p| p.1).min().unwrap_or(0);
    let hi_y = v.iter().map(|p| p.1).max().unwrap_or(0);
    // (pixels whose centres can be inside: centre i * 256 + 128 within the corners')
    let px = |lo: i64, hi: i64, a: i64, z: i64| (lo.saturating_sub(128).div_euclid(256).max(a), hi.saturating_sub(128).div_euclid(256).saturating_add(1).min(z));
    let (i0, i1) = px(lo_x, hi_x, l, rr);
    let (j0, j1) = px(lo_y, hi_y, t, b);
    let mut quads = 0u64;
    let mut qy = j0.div_euclid(2);
    while qy.saturating_mul(2) < j1 {
        let mut qx = i0.div_euclid(2);
        while qx.saturating_mul(2) < i1 {
            let hit = (0..4).any(|k| {
                let (i, j) = (qx.saturating_mul(2).saturating_add(k & 1), qy.saturating_mul(2).saturating_add(k >> 1));
                i >= i0 && i < i1 && j >= j0 && j < j1 && covers(&v, &o, centre(i), centre(j))
            });
            quads = quads.saturating_add(u64::from(hit));
            qx = qx.saturating_add(1);
        }
        qy = qy.saturating_add(1);
    }
    quads.saturating_mul(4)
}

/// WARP's aliased line test (lines.md 1.1 and 1.2): the pixels lit between snapped ends.
fn line_pixels(a: (i32, i32), b: (i32, i32), r: &Raster) -> Vec<(i64, i64)> {
    let e = 1.0f32 / 256.0;
    let (x0, y0, x1, y1) = (to_f32(a.0) * e, to_f32(a.1) * e, to_f32(b.0) * e, to_f32(b.1) * e);
    if a == b {
        return Vec::new();
    }
    let xmajor = (x0 - x1).abs() >= (y0 - y1).abs();
    let (dx, dy) = (x1 - x0, y1 - y0);
    let ceil = |v: f32| {
        let n = v.round_ties_even();
        (if n < v { n + 1.0 } else { n }) - 0.5
    };
    let (cx0, cy0, cx1, cy1) = (ceil(x0), ceil(y0), ceil(x1), ceil(y1));
    let diamond = |x: f32, y: f32, cx: f32, cy: f32| {
        let d = (x - cx).abs() + (y - cy).abs();
        if y > cy || (same(y, cy) && !xmajor && x > cx) { d <= 0.5 } else { d < 0.5 }
    };
    let (in_s, in_e) = (diamond(x0, y0, cx0, cy0), diamond(x1, y1, cx1, cy1));
    if in_s && in_e && same(cx0, cx1) && same(cy0, cy1) {
        return Vec::new();
    }
    let (mut sx, mut sy, mut ddx, mut ddy) = (x0, y0, dx, dy);
    if in_s {
        let (u, v) = if xmajor {
            let u = (x0 - cx0) + if dx > 0.0 { e } else { -e };
            (u, (dy / dx) * u)
        } else {
            let v = (y0 - cy0) + if dy > 0.0 { e } else { -e };
            ((dx / dy) * v, v)
        };
        sx = x0 - u;
        sy = y0 - v;
        ddx = dx + u;
        ddy = dy + v;
    }
    if in_e {
        let (u, v) = if xmajor {
            let u = ((sx + ddx) - cx1) + if ddx > 0.0 { e } else { -e };
            (u, (ddy / ddx) * u)
        } else {
            let v = ((sy + ddy) - cy1) + if ddy > 0.0 { e } else { -e };
            ((ddx / ddy) * v, v)
        };
        ddx -= u;
        ddy -= v;
    }
    let hi_col = f32::from_bits(0x3f00_0001);
    let hi_row = if xmajor { 0.5 } else { hi_col };
    let (inv_dx, inv_dy) = (-1.0 / ddx, -1.0 / ddy);
    let lit = |i: i64, j: i64| {
        let (px, py) = (to_f32(i) + 0.5, to_f32(j) + 0.5);
        let (a, b) = (sx - px, sy - py);
        let t = a * inv_dx;
        let col = (0.0..=1.0).contains(&t) && {
            let v = t * ddy + b;
            v > -0.5 && v < hi_col
        };
        let t2 = b * inv_dy;
        let row = (0.0..=1.0).contains(&t2) && {
            let v = t2 * ddx + a;
            v > -0.5 && v < hi_row
        };
        col || row
    };
    let [l, t, rr, bb] = r.rect.map(i64::from);
    let fl = |v: f32| i64::from(trunc_i32(v.floor()));
    let (i0, i1) = (fl(x0.min(x1)).saturating_sub(2).max(l), fl(x0.max(x1)).saturating_add(3).min(rr));
    let (j0, j1) = (fl(y0.min(y1)).saturating_sub(2).max(t), fl(y0.max(y1)).saturating_add(3).min(bb));
    let mut out = Vec::new();
    for j in j0..j1 {
        for i in i0..i1 {
            if lit(i, j) {
                out.push((i, j));
            }
        }
    }
    out
}

/// The pixel shader invocations Windows counts for one primitive of 1 (point), 2 (line) or 3
/// (triangle) clip-space corners.
pub fn invocations(p: &[[f32; 4]], r: &Raster) -> u64 {
    let quads_of = |px: &[(i64, i64)]| {
        let mut q: Vec<(i64, i64)> = px.iter().map(|&(i, j)| (i.div_euclid(2), j.div_euclid(2))).collect();
        q.sort_unstable();
        q.dedup();
        kome::num::u64_of(q.len()).saturating_mul(4)
    };
    match *p {
        [a, b, c] => {
            let poly = clip(&[a, b, c], r.depth_clip);
            let Some(first) = poly.first() else { return 0 };
            let s0 = snap(first, &r.viewport);
            let n = poly.len();
            let mut sum = 0u64;
            for i in 1..n.saturating_sub(1) {
                let (Some(pa), Some(pb)) = (poly.get(i), poly.get(i.saturating_add(1))) else { continue };
                // (edges: first to i (the polygon's edge 0 when i is 1), i to i + 1, i + 1 to
                // first (the polygon's last edge when i + 1 is the last point))
                let e0 = i == 1 && first.original;
                let e2 = i.saturating_add(2) == n && pb.original;
                sum = sum.saturating_add(triangle([s0, snap(pa, &r.viewport), snap(pb, &r.viewport)], [e0, pa.original, e2], r));
            }
            sum
        }
        [a, b] => {
            let mut ends = [Pt { p: a, d: distances(&a, r.depth_clip), original: true, new: false }, Pt { p: b, d: distances(&b, r.depth_clip), original: true, new: false }];
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
            quads_of(&line_pixels(snap(&s, &r.viewport), snap(&e, &r.viewport), r))
        }
        [a] => {
            if distances(&a, r.depth_clip).iter().any(|d| 0.0 > *d || d.is_nan()) {
                return 0;
            }
            let (x, y) = snap(&Pt { p: a, d: [0.0; 6], original: true, new: false }, &r.viewport);
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
/// ps_quads(p: array<vec4<f32>, 3>, n: u32) -> u32`, the quads (invocations / 4). The 64-bit
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
fn ps_covers(v: array<vec2<i32>, 3>, o: vec3<bool>, cx: i32, cy: i32) -> bool {
    var vv = v;
    var oo = o;
    for (var e = 0u; e < 3u; e += 1u) {
        let a = vv[e];
        let b = vv[(e + 1u) % 3u];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let f = s64(dx, cy - a.y, dy, cx - a.x);
        let tl = (dy == 0 && dx > 0) || dy < 0;
        if (!(f > 0 || (f == 0 && (tl || oo[e])))) { return false; }
    }
    return true;
}
// the quads of a fan triangle (culled: 0)
fn ps_tri(v0: vec2<i32>, v1: vec2<i32>, v2: vec2<i32>, o: vec3<bool>) -> u32 {
    let det = s64(v2.y - v0.y, v1.x - v0.x, v0.y - v1.y, v0.x - v2.x);
    if (det == 0) { return 0u; }
    let bits = xw(XD_RASTER);
    let ccw = (bits & 4u) != 0u;
    let front = select(det > 0, det < 0, ccw);
    let cull = bits & 3u;
    if ((cull == 2u && front) || (cull == 3u && !front)) { return 0u; }
    var v: array<vec2<i32>, 3>;
    var oo = o;
    if (det > 0) { v = array<vec2<i32>, 3>(v0, v1, v2); } else { v = array<vec2<i32>, 3>(v0, v2, v1); oo = vec3<bool>(o.z, o.y, o.x); }
    let lo = min(min(v0, v1), v2);
    let hi = max(max(v0, v1), v2);
    let i0 = max((lo.x - 128) >> 8u, ps_rect(0u));
    let i1 = min(((hi.x - 128) >> 8u) + 1, ps_rect(2u));
    let j0 = max((lo.y - 128) >> 8u, ps_rect(1u));
    let j1 = min(((hi.y - 128) >> 8u) + 1, ps_rect(3u));
    var quads = 0u;
    for (var qy = j0 >> 1u; qy * 2 < j1; qy += 1) {
        // (the triangle's x extent over the quad's two rows of centres, a pixel wider)
        let ya = f32(qy * 512 + 128);
        let yb = ya + 256.0f;
        var xl = 3.0e38f;
        var xh = -3.0e38f;
        for (var e = 0u; e < 3u; e += 1u) {
            let a = vec2<f32>(v[e]);
            let b = vec2<f32>(v[(e + 1u) % 3u]);
            if (a.y >= ya && a.y <= yb) { xl = min(xl, a.x); xh = max(xh, a.x); }
            for (var r = 0u; r < 2u; r += 1u) {
                let yl = select(ya, yb, r == 1u);
                if (a.y != b.y && (yl - a.y) * (yl - b.y) <= 0.0f) {
                    let x = a.x + (yl - a.y) * (b.x - a.x) / (b.y - a.y);
                    xl = min(xl, x); xh = max(xh, x);
                }
            }
        }
        if (xl > xh) { continue; }
        let ia = max(i32(floor((xl - 128.0f) / 256.0f)) - 1, i0);
        let ib = min(i32(floor((xh - 128.0f) / 256.0f)) + 2, i1);
        for (var qx = ia >> 1u; qx * 2 < ib; qx += 1) {
            var hit = false;
            for (var k = 0; k < 4 && !hit; k += 1) {
                let i = qx * 2 + (k & 1);
                let j = qy * 2 + (k >> 1u);
                hit = i >= i0 && i < i1 && j >= j0 && j < j1 && ps_covers(v, oo, i * 256 + 128, j * 256 + 128);
            }
            quads += select(0u, 1u, hit);
        }
    }
    return quads;
}
fn ps_ceil(v: f32) -> f32 {
    let n = round(v);
    return select(n, n + 1.0f, n < v) - 0.5f;
}
fn ps_diamond(x: f32, y: f32, cx: f32, cy: f32, xmajor: bool) -> bool {
    let d = abs(x - cx) + abs(y - cy);
    if (y > cy || (y == cy && !xmajor && x > cx)) { return d <= 0.5f; }
    return d < 0.5f;
}
// WARP's aliased line: the quads with a lit pixel
fn ps_line(a: vec2<i32>, b: vec2<i32>) -> u32 {
    if (all(a == b)) { return 0u; }
    let e = 1.0f / 256.0f;
    let x0 = f32(a.x) * e; let y0 = f32(a.y) * e; let x1 = f32(b.x) * e; let y1 = f32(b.y) * e;
    let xmajor = abs(x0 - x1) >= abs(y0 - y1);
    let dx = x1 - x0; let dy = y1 - y0;
    let cx0 = ps_ceil(x0); let cy0 = ps_ceil(y0); let cx1 = ps_ceil(x1); let cy1 = ps_ceil(y1);
    let ins = ps_diamond(x0, y0, cx0, cy0, xmajor);
    let ine = ps_diamond(x1, y1, cx1, cy1, xmajor);
    if (ins && ine && cx0 == cx1 && cy0 == cy1) { return 0u; }
    var sx = x0; var sy = y0; var ddx = dx; var ddy = dy;
    if (ins) {
        var u: f32; var v: f32;
        if (xmajor) { u = (x0 - cx0) + select(-e, e, dx > 0.0f); v = (dy / dx) * u; }
        else { v = (y0 - cy0) + select(-e, e, dy > 0.0f); u = (dx / dy) * v; }
        sx = x0 - u; sy = y0 - v; ddx = dx + u; ddy = dy + v;
    }
    if (ine) {
        var u: f32; var v: f32;
        if (xmajor) { u = ((sx + ddx) - cx1) + select(-e, e, ddx > 0.0f); v = (ddy / ddx) * u; }
        else { v = ((sy + ddy) - cy1) + select(-e, e, ddy > 0.0f); u = (ddx / ddy) * v; }
        ddx = ddx - u; ddy = ddy - v;
    }
    let hi_col = bitcast<f32>(0x3f000001u);
    let hi_row = select(hi_col, 0.5f, xmajor);
    let idx = -1.0f / ddx; let idy = -1.0f / ddy;
    let i0 = max(i32(floor(min(x0, x1))) - 2, ps_rect(0u));
    let i1 = min(i32(floor(max(x0, x1))) + 3, ps_rect(2u));
    let j0 = max(i32(floor(min(y0, y1))) - 2, ps_rect(1u));
    let j1 = min(i32(floor(max(y0, y1))) + 3, ps_rect(3u));
    var quads = 0u;
    for (var qy = j0 >> 1u; qy * 2 < j1; qy += 1) {
        // (the line's x over the quad's rows, two pixels wider; every x when it is level)
        var ia = i0; var ib = i1;
        if (dy != 0.0f) {
            let xa = x0 + (f32(qy * 2) - 1.0f - y0) * dx / dy;
            let xb = x0 + (f32(qy * 2) + 3.0f - y0) * dx / dy;
            ia = max(i32(floor(min(xa, xb))) - 2, i0);
            ib = min(i32(floor(max(xa, xb))) + 3, i1);
        }
        for (var qx = ia >> 1u; qx * 2 < ib; qx += 1) {
            var hit = false;
            for (var k = 0; k < 4 && !hit; k += 1) {
                let i = qx * 2 + (k & 1);
                let j = qy * 2 + (k >> 1u);
                if (i < i0 || i >= i1 || j < j0 || j >= j1) { continue; }
                let px = f32(i) + 0.5f; let py = f32(j) + 0.5f;
                let aa = sx - px; let bb = sy - py;
                let t = aa * idx;
                if (t >= 0.0f && t <= 1.0f) { let w = t * ddy + bb; if (w > -0.5f && w < hi_col) { hit = true; } }
                let t2 = bb * idy;
                if (t2 >= 0.0f && t2 <= 1.0f) { let w = t2 * ddx + aa; if (w > -0.5f && w < hi_row) { hit = true; } }
            }
            quads += select(0u, 1u, hit);
        }
    }
    return quads;
}
fn ps_quads(p: array<vec4<f32>, 3>, n: u32) -> u32 {
    let dc = (xw(XD_RASTER) & 8u) != 0u;
    if ((xw(XD_RASTER) & 16u) == 0u) { return 0u; }
    if (n == 1u) {
        for (var k = 0u; k < 6u; k += 1u) { let d = ps_dist(p[0], k, dc); if (0.0f > d || d != d) { return 0u; } }
        let s = ps_snap(p[0], false);
        let i = ((s.x + 255) >> 8u) - 1;
        let j = ((s.y + 255) >> 8u) - 1;
        return select(0u, 1u, i >= ps_rect(0u) && i < ps_rect(2u) && j >= ps_rect(1u) && j < ps_rect(3u));
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
        return ps_line(ps_snap(s, sn), ps_snap(e, en));
    }
    // a triangle: rotated to the smallest (x, y), clipped against its codes' planes
    var r = 0u;
    if (p[0].x > p[1].x || (p[0].x == p[1].x && p[0].y > p[1].y)) { r = 1u; }
    if (p[r].x > p[2].x || (p[r].x == p[2].x && p[r].y > p[2].y)) { r = 2u; }
    var pp: array<vec4<f32>, 12>;
    var pd: array<array<f32, 6>, 12>;
    var po: array<bool, 12>;
    var pn: array<bool, 12>;
    var qp: array<vec4<f32>, 12>;
    var qd: array<array<f32, 6>, 12>;
    var qo: array<bool, 12>;
    var qn: array<bool, 12>;
    var or_ = 0u;
    var and_ = 0x3fu;
    var nan = false;
    for (var c = 0u; c < 3u; c += 1u) {
        pp[c] = p[(c + r) % 3u];
        po[c] = true;
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
                qp[o] = pp[i]; qd[o] = pd[i]; qo[o] = po[i]; qn[o] = pn[i]; o += 1u;
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
                // (entering: the edge on to the inside point is the original's; leaving: on the plane)
                qo[o] = ao && po[i];
                qn[o] = true;
                o += 1u;
            }
        }
        m = o;
        if (m == 0u) { return 0u; }
        for (var c = 0u; c < m; c += 1u) { pp[c] = qp[c]; pd[c] = qd[c]; po[c] = qo[c]; pn[c] = qn[c]; }
    }
    let s0 = ps_snap(pp[0], pn[0]);
    var quads = 0u;
    for (var i = 1u; i + 1u < m; i += 1u) {
        let e0 = i == 1u && po[0];
        let e2 = i + 2u == m && po[i + 1u];
        quads += ps_tri(s0, ps_snap(pp[i], pn[i]), ps_snap(pp[i + 1u], pn[i + 1u]), vec3<bool>(e0, po[i], e2));
    }
    return quads;
}
"#;
