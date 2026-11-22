//! The pixel shader's inputs as WARP (the Direct3D 11 device of the Windows runner) interpolates
//! them, to the last bit: WebGPU's rasterizer interpolates by its own arithmetic (SwiftShader's
//! 3e9c28f4 where WARP gives 3e9c28f6, winref d3d11logic "position, value, coverage"), so a
//! draw whose pixel shader reads interpolated inputs takes a pass of its own ([`setup_pass`])
//! after the vertex (geometry, domain) shader's: each list triangle of the store through WARP's
//! clipper and triangle setup (clean-room notes kikizake-re/d3d11/clip.md §2–§8), the planes of
//! its fan triangles into `setup`; the render pipeline draws those fan triangles
//! ([`setup_vertex`]) and the pixel shader evaluates the planes at its pixel itself (the
//! translation's [`Eval`] mode, `wgsl::translate_setup`).
//!
//! The rules (all f32 with rounding to nearest, no fused multiply-add):
//! - the triangle turned so the corner with the smallest clip-space (x, y) comes first (§2);
//! - clip codes of z, w - z (depth clip on), w + x, w - x, w - y, w + y and each
//!   SV_ClipDistance, no guard band; a triangle outside one plane, or with every corner outside
//!   one SV_CullDistance, dropped; one with codes set clipped (Sutherland-Hodgman, a point on a
//!   plane inside) against the planes the codes name, in that order, always from the inside
//!   point: positions, perspective attributes and the later planes' distances by
//!   `f32((f64(f32(b - a)) * dIn) / den + a)` (`den = f32(dIn - dOut)`), noperspective ones by
//!   the screen-space `s` of the positions' w, constant ones the provoking (first API) corner's;
//! - the polygon drawn as a fan from its first point; a new point's x and y clamped to ±w (w
//!   at least FLT_MIN), z to [0, w] with depth clip; every point's screen position
//!   round-to-even((rhw · x · sx + ox) · 256) in 1/256 pixels, its depth (rhw · z) · sz + oz;
//! - each fan triangle culled by the sign of its doubled area (0: dropped); its plane of a
//!   value a (a · rhw for perspective inputs, rhw itself, the depth): with x0 = X0 / 256 and the
//!   corner deltas dx10 = (X1 - X0) / 256, dx02 = (X0 - X2) / 256, dy01 = (Y0 - Y1) / 256,
//!   dy20 = (Y2 - Y0) / 256, inv = 1 / (f32(det) · 2^-16) (det the exact doubled area),
//!   gx = ((d10 · dy20) + (d20 · dy01)) · inv, gy = ((d10 · dx02) + (d20 · dx10)) · inv,
//!   c = (a0 - gx · x0) - gy · y0;
//! - at the pixel centre (cx, cy): (c + gy · cy) + gx · cx, a perspective input that times
//!   1 / (the rhw plane's value there).
//!
//! The store ([`Slot`]): per list primitive [`slots`] fan triangles of [`Layout::stride`]
//! vec4<u32> each: the three corners' clip-space positions (as drawn), the rhw plane and the
//! depth plane (gx, gy, c, 0), then per input register of the layout three vec4s: gx, gy and
//! c of its components (constant ones: c the bits, gx = gy = 0). A fan triangle not drawn has
//! three equal corners.

use super::super::{bits, parse, W, K, XYZW};
use super::ps::{self, Cull};
use super::{xd_decl, Store, ITEM};
use crate::dxbc::SigElement;
use crate::tpf;
use crate::{Binding, Wgsl};
use kome::num::to_f32;

/// Where a mode evaluates its input in the pixel.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum At {
    Center,
    Centroid,
    Sample,
}

/// How an input is interpolated (D3D10_SB_INTERPOLATION_MODE).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Mode {
    /// 0: not declared with a mode: zeros
    Undefined,
    /// 1, and every integer input: the provoking corner's bits
    Constant,
    /// 2, 3, 6: perspective-correct
    Perspective(At),
    /// 4, 5, 7: linear in screen space
    NoPerspective(At),
}

impl Mode {
    /// Of a dcl_input_ps mode, for an input of floats or not.
    pub fn of(interp: u32, float: bool) -> Option<Mode> {
        if !float {
            return Some(Mode::Constant);
        }
        Some(match interp {
            0 => Mode::Undefined,
            1 => Mode::Constant,
            2 => Mode::Perspective(At::Center),
            3 => Mode::Perspective(At::Centroid),
            4 => Mode::NoPerspective(At::Center),
            5 => Mode::NoPerspective(At::Centroid),
            6 => Mode::Perspective(At::Sample),
            7 => Mode::NoPerspective(At::Sample),
            _ => return None,
        })
    }

    /// Its number in the setup pass's WGSL (`setup_c`'s `mode`).
    fn word(self) -> u32 {
        match self {
            Mode::Undefined => 0,
            Mode::Constant => 1,
            Mode::Perspective(_) => 2,
            Mode::NoPerspective(_) => 4,
        }
    }

    /// Whether the input changes over the triangle.
    pub fn varies(self) -> bool {
        match self {
            Mode::Undefined | Mode::Constant => false,
            Mode::Perspective(_) | Mode::NoPerspective(_) => true,
        }
    }
}

/// A pixel shader input the setup gives planes for: its register, components, mode.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Input {
    pub reg: u32,
    pub mask: u32,
    pub mode: Mode,
    /// SV_ClipDistance / SV_CullDistance (2, 3) or 0
    pub sysval: u32,
}

/// The inputs of a pixel shader that come from the setup: its varyings and the clip and cull
/// distances it reads, in register order. Their place in a slot is their order here.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Layout {
    pub inputs: Vec<Input>,
    /// the components of SV_Position read (its x and y are the pixel's, z and w come from the
    /// depth and rhw planes)
    pub position: u32,
}

/// The components of input register `reg` the program's instructions read (every component a
/// source swizzle names).
fn read(p: &tpf::Program, reg: u32) -> u32 {
    let mut m = 0u32;
    for i in p.insts.iter().filter(|i| tpf::arity(i.op).is_some()) {
        for o in i.ops.iter().filter(|o| o.ty == tpf::INPUT && o.reg() == reg) {
            m |= o.swizzle().iter().fold(0, |a, &c| a | 1u32 << c);
        }
    }
    m
}

/// vec4s before the inputs' planes in a slot: the three corners drawn, the rhw plane (w: an
/// aliased line's hiCol, 0 for a triangle), the depth plane (w: the line's hiRow), the line's
/// nudged start and extent (X0, Y0, DX, DY).
pub const HEADER: u32 = 6;

impl Layout {
    /// A pixel shader's layout.
    pub fn of(ps: &[u8]) -> Result<Layout, String> {
        let (c, p) = parse(ps)?;
        if p.kind != tpf::PIXEL {
            return Err(format!("program type {} for a pixel shader", p.kind));
        }
        let mut w = W::new(true, c.signature(false), c.signature(true));
        super::super::declarations(&mut w, &p)?;
        let mut l = Layout::default();
        for (&reg, d) in &w.inputs {
            match d.sysval {
                0 => {
                    let k = w.isgn.iter().find(|e| e.reg == reg).map_or(K::F, |e| K::of_sig(e.component_type));
                    let mode = Mode::of(d.interp, k == K::F).ok_or(format!("v{reg}: interpolation mode {}", d.interp))?;
                    l.inputs.push(Input { reg, mask: d.mask, mode, sysval: d.sysval });
                }
                // (a clip or cull distance read by the pixel shader is 0 on WARP: winref
                // d3d11interp "clip distance", every pixel's SV_ClipDistance0 00000000)
                2 | 3 => l.inputs.push(Input { reg, mask: d.mask, mode: Mode::Undefined, sysval: d.sysval }),
                1 => l.position = d.mask & read(&p, reg),
                _ => {}
            }
        }
        Ok(l)
    }

    /// Whether a draw with this pixel shader needs the setup: an input that varies over the
    /// triangle, or SV_Position's z or w.
    pub fn needed(&self) -> bool {
        self.position & 0b1100 != 0 || self.inputs.iter().any(|i| i.mode.varies())
    }

    /// vec4<u32>s of a slot.
    pub fn stride(&self) -> u32 {
        HEADER.saturating_add(kome::num::u32_of(self.inputs.len()).saturating_mul(3))
    }
}

/// What the setup makes of a list primitive: a solid triangle's fan, a triangle's edges as
/// aliased lines (WIREFRAME), an aliased line, a point.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Prim {
    Triangle,
    Wire,
    Line,
    Point,
}

impl Prim {
    /// The primitive of a store's `corners` (wireframe fill or solid), None for none.
    pub fn of(corners: u32, wire: bool) -> Option<Prim> {
        match (corners, wire) {
            (3, false) => Some(Prim::Triangle),
            (3, true) => Some(Prim::Wire),
            (2, _) => Some(Prim::Line),
            (1, _) => Some(Prim::Point),
            _ => None,
        }
    }

    pub fn corners(self) -> u32 {
        match self {
            Prim::Triangle | Prim::Wire => 3,
            Prim::Line => 2,
            Prim::Point => 1,
        }
    }

    /// The slots (each a triangle drawn) one primitive takes with `user` clip distances: a
    /// fan of a polygon of at most 3 + planes points; a line's one triangle over its region,
    /// each of the 3 edges' for wireframe; a point's square of 2.
    pub fn slots(self, user: u32) -> u32 {
        match self {
            Prim::Triangle => user.saturating_add(7),
            Prim::Wire => 3,
            Prim::Line => 1,
            Prim::Point => 2,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The rule in Rust (the reference the tests hold the WGSL and Windows' answers to)

/// A clip-space corner of a triangle: its position, the store's components the layout reads
/// (in layout order, 4 a register, the bits), its SV_ClipDistance and SV_CullDistance values.
#[derive(Clone, Debug, Default)]
pub struct Corner {
    pub pos: [f32; 4],
    pub attrs: Vec<u32>,
    pub clip: Vec<f32>,
    pub cull: Vec<f32>,
}

/// What the rasterizer is set to (the setup's part of it).
#[derive(Clone, Copy, Debug)]
pub struct Raster {
    /// TopLeftX, TopLeftY, Width, Height, MinDepth, MaxDepth
    pub viewport: [f32; 6],
    pub cull: Cull,
    pub front_ccw: bool,
    pub depth_clip: bool,
}

/// A plane: the value at (x, y) is (c + gy · y) + gx · x.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Plane {
    pub gx: f32,
    pub gy: f32,
    pub c: f32,
}

impl Plane {
    pub fn at(self, x: f32, y: f32) -> f32 {
        (self.c + self.gy * y) + self.gx * x
    }
}

/// A fan triangle the setup made: its corners' clip-space positions, the rhw and depth planes,
/// and per layout component (4 a register) its plane (constant ones: c holds the bits).
#[derive(Clone, Debug)]
pub struct Fan {
    pub pos: [[f32; 4]; 3],
    pub rhw: Plane,
    pub z: Plane,
    pub attrs: Vec<Plane>,
    /// the corners' snapped screen positions (1/256 pixels)
    pub scr: [(i32, i32); 3],
}

/// How a point of the clipper's table came to be: from the inside point `inside` toward the
/// outside one `outside` across a plane (dIn, den).
#[derive(Clone, Copy, Debug)]
struct Made {
    inside: usize,
    outside: usize,
    d_in: f32,
    d_out: f32,
    den: f32,
}

#[derive(Clone, Debug)]
struct Point {
    p: [f32; 4],
    /// the six planes' distances, then the clip distances'
    d: Vec<f32>,
    made: Option<Made>,
}

/// WARP's `(f32(b - a) * dIn) / den + a`, in double, rounded once: the clipper's one
/// interpolation (ps.rs).
use ps::lerp;

/// The noperspective step's s from the positions' w (clip.md §5b).
fn screen_s(w_in: f32, w_out: f32, d_in: f32, d_out: f32) -> f32 {
    let num = d_in * w_out;
    let den = (w_out - w_in) * d_in + (d_in - d_out) * w_in;
    num / den
}

/// The table of the clipper's points (the corners turned first, then each new point in the
/// order it was made) and the final polygon's points in it; None: dropped.
fn clip(c: &[Corner; 3], depth_clip: bool) -> Option<(Vec<Point>, Vec<usize>, [usize; 3])> {
    let user = c.iter().map(|v| v.clip.len()).max().unwrap_or(0);
    // (cull distances: dropped when every corner is outside one)
    let culls = c.iter().map(|v| v.cull.len()).max().unwrap_or(0);
    for k in 0..culls {
        if c.iter().all(|v| v.cull.get(k).is_some_and(|d| 0.0 > *d)) {
            return None;
        }
    }
    // the corner with the smallest (x, y) first (strict compares: the earlier of equals)
    let gt = |u: &[f32; 4], v: &[f32; 4]| u[0] > v[0] || (u[0].partial_cmp(&v[0]) == Some(std::cmp::Ordering::Equal) && u[1] > v[1]);
    let s0 = usize::from(gt(&c[0].pos, &c[1].pos));
    let sel = if gt(&c.get(s0).map_or(c[0].pos, |v| v.pos), &c[2].pos) { 2 } else { s0 };
    let rot = match sel {
        1 => [1, 2, 0],
        2 => [2, 0, 1],
        _ => [0, 1, 2],
    };
    let mut table: Vec<Point> = rot
        .iter()
        .map(|&i| {
            let v = &c[i];
            let [x, y, z, w] = v.pos;
            let (n, f) = if depth_clip { (z, w - z) } else { (0.0, 0.0) };
            let mut d = vec![n, f, w + x, w - x, w - y, w + y];
            d.extend((0..user).map(|k| v.clip.get(k).copied().unwrap_or(0.0)));
            Point { p: v.pos, d, made: None }
        })
        .collect();
    let planes = 6usize.saturating_add(user);
    let code = |d: &[f32]| d.iter().enumerate().fold(0u64, |m, (k, v)| if v.is_sign_negative() && !v.is_nan() { m | 1 << k } else { m });
    if depth_clip && table.iter().any(|v| v.d.iter().take(6).any(|x| x.is_nan())) {
        return None;
    }
    let codes: Vec<u64> = table.iter().map(|v| code(&v.d)).collect();
    let or = codes.iter().fold(0, |m, c| m | c);
    let and = codes.iter().fold(u64::MAX, |m, c| m & c);
    if and != 0 {
        return None;
    }
    let mut poly: Vec<usize> = vec![0, 1, 2];
    for k in 0..planes {
        if or >> k & 1 == 0 {
            continue;
        }
        let n = poly.len();
        let mut out = Vec::with_capacity(n.saturating_add(1));
        for i in 0..n {
            let (Some(&a), Some(&b)) = (poly.get(i), poly.get(i.saturating_add(1).checked_rem(n).unwrap_or(0))) else { continue };
            let da = table.get(a).and_then(|p| p.d.get(k)).copied().unwrap_or(0.0);
            let db = table.get(b).and_then(|p| p.d.get(k)).copied().unwrap_or(0.0);
            let (a_out, b_out) = (0.0 > da, 0.0 > db);
            let cross = |inside: usize, outside: usize, d_in: f32, d_out: f32, table: &mut Vec<Point>| -> usize {
                let den = d_in - d_out;
                let (Some(i), Some(o)) = (table.get(inside), table.get(outside)) else { return inside };
                let p = std::array::from_fn(|q| lerp(i.p[q], o.p[q], d_in, den));
                let d = (0..planes).map(|q| {
                    let (x, y) = (i.d.get(q).copied().unwrap_or(0.0), o.d.get(q).copied().unwrap_or(0.0));
                    if q > k && or >> q & 1 != 0 { lerp(x, y, d_in, den) } else { x }
                });
                let pt = Point { p, d: d.collect(), made: Some(Made { inside, outside, d_in, d_out, den }) };
                table.push(pt);
                table.len().saturating_sub(1)
            };
            if !a_out {
                out.push(a);
                if b_out {
                    out.push(cross(a, b, da, db, &mut table));
                }
            } else if !b_out {
                out.push(cross(b, a, db, da, &mut table));
            }
        }
        poly = out;
        if poly.is_empty() {
            return None;
        }
    }
    Some((table, poly, rot))
}

/// round to nearest even, i32::MIN past the range (cvtss2si)
fn cvt(v: f32) -> i32 {
    let two31 = f32::from_bits(0x4f00_0000);
    if v.is_nan() || v >= two31 || v < -two31 { i32::MIN } else { kome::num::trunc_i32(v.round_ties_even()) }
}

/// A point's screen position (1/256 pixels), depth and rhw (clip.md §7).
fn screen(pt: &Point, r: &Raster) -> ((i32, i32), f32, f32) {
    let [x, y, z, w] = pt.p;
    let w = if f32::MIN_POSITIVE > w { f32::MIN_POSITIVE } else { w };
    let new = pt.made.is_some();
    let (x, y) = if new { (x.max(-w).min(w), y.max(-w).min(w)) } else { (x, y) };
    let z = if new && r.depth_clip { z.max(0.0).min(w) } else { z };
    let rhw = 1.0 / w;
    let [vx, vy, vw, vh, n, f] = r.viewport;
    let (sx, ox, sy, oy, sz, oz) = (vw * 0.5, vw * 0.5 + vx, -vh * 0.5, vh * 0.5 + vy, f - n, n);
    ((cvt((rhw * x * sx + ox) * 256.0), cvt((rhw * y * sy + oy) * 256.0)), rhw * z * sz + oz, rhw)
}

/// A clip-space position as the fan triangle's corner is drawn (a new point's clamped).
fn drawn(pt: &Point, depth_clip: bool) -> [f32; 4] {
    let [x, y, z, w] = pt.p;
    if pt.made.is_none() {
        return pt.p;
    }
    let wc = if f32::MIN_POSITIVE > w { f32::MIN_POSITIVE } else { w };
    [x.max(-wc).min(wc), y.max(-wc).min(wc), if depth_clip { z.max(0.0).min(wc) } else { z }, w]
}

/// The f32 of an exact integer, rounded to nearest even.
fn f32_of(v: i128) -> f32 {
    // (through f64: exact below 2^53, and the doubled areas of 1/256-pixel coordinates of at
    // most 2^31 stay below 2^63, whose f64 rounding could round twice: done by the bits)
    let neg = v < 0;
    let a = v.unsigned_abs();
    if a < 1 << 24 {
        let f = to_f32(f64::from(u32::try_from(a).unwrap_or(0)));
        return if neg { -f } else { f };
    }
    let top = 127u32.saturating_sub(a.leading_zeros());
    let shift = top.saturating_sub(23);
    let m = a >> shift;
    let rest = a & ((1u128 << shift).wrapping_sub(1));
    let half = 1u128 << shift.saturating_sub(1);
    let up = rest > half || (rest == half && m & 1 == 1);
    let m = m.saturating_add(u128::from(up));
    let (m, e) = if m >> 24 != 0 { (m >> 1, top.saturating_add(1)) } else { (m, top) };
    let bits = (e.saturating_add(127)) << 23 | u32::try_from(m & 0x7f_ffff).unwrap_or(0);
    let f = f32::from_bits(bits);
    if neg { -f } else { f }
}

/// A fan triangle's geometry for its planes: x0, y0, dx10, dx02, dy01, dy20, inv; None for no
/// area.
fn geometry(s: [(i32, i32); 3]) -> Option<[f32; 7]> {
    let [(x0, y0), (x1, y1), (x2, y2)] = s.map(|(x, y)| (i128::from(x), i128::from(y)));
    let det = (y2 - y0) * (x1 - x0) - (y0 - y1) * (x0 - x2);
    if det == 0 {
        return None;
    }
    let e = f32::from_bits(0x3b80_0000);
    let px = |v: i128| f32_of(v) * e;
    let inv = 1.0 / (f32_of(det) * f32::from_bits(0x3780_0000));
    Some([px(x0), px(y0), px(x1 - x0), px(x0 - x2), px(y0 - y1), px(y2 - y0), inv])
}

/// The plane through a0, a1, a2 at a fan triangle of geometry g.
fn plane(g: &[f32; 7], a: [f32; 3]) -> Plane {
    let [x0, y0, dx10, dx02, dy01, dy20, inv] = *g;
    let (d10, d20) = (a[1] - a[0], a[2] - a[0]);
    let gx = (d10 * dy20 + d20 * dy01) * inv;
    let gy = (d10 * dx02 + d20 * dx10) * inv;
    Plane { gx, gy, c: (a[0] - gx * x0) - gy * y0 }
}

/// Whether a fan triangle of doubled area `det` (its sign) is drawn.
fn facing(s: [(i32, i32); 3], r: &Raster) -> bool {
    let [(x0, y0), (x1, y1), (x2, y2)] = s.map(|(x, y)| (i128::from(x), i128::from(y)));
    let det = (y2 - y0) * (x1 - x0) - (y0 - y1) * (x0 - x2);
    let front = if r.front_ccw { det < 0 } else { det > 0 };
    match r.cull {
        Cull::None => true,
        Cull::Front => !front,
        Cull::Back => front,
    }
}

/// Each table point's value of one component: the corners' (turned), then the new points' by
/// their mode's rule.
fn values(table: &[Point], rot: [usize; 3], corners: [u32; 3], mode: Mode) -> Vec<f32> {
    let mut v: Vec<f32> = Vec::with_capacity(table.len());
    for (i, pt) in table.iter().enumerate() {
        let x = match (pt.made, mode) {
            (_, Mode::Undefined) => 0.0,
            (_, Mode::Constant) => f32::from_bits(corners[0]),
            (None, _) => f32::from_bits(rot.get(i).and_then(|&k| corners.get(k)).copied().unwrap_or(0)),
            (Some(m), Mode::Perspective(_)) => lerp(v.get(m.inside).copied().unwrap_or(0.0), v.get(m.outside).copied().unwrap_or(0.0), m.d_in, m.den),
            (Some(m), Mode::NoPerspective(_)) => {
                let (a, b) = (v.get(m.inside).copied().unwrap_or(0.0), v.get(m.outside).copied().unwrap_or(0.0));
                let w = |k: usize| table.get(k).map_or(0.0, |p| p.p[3]);
                let s = screen_s(w(m.inside), w(m.outside), m.d_in, m.d_out);
                (b - a) * s + a
            }
        };
        v.push(x);
    }
    v
}

/// The fan triangles WARP sets up for a triangle (API order corners), the layout's modes by
/// component (4 a register, `modes[i / 4]`).
pub fn triangle(c: &[Corner; 3], modes: &[Mode], r: &Raster) -> Vec<Fan> {
    let Some((table, poly, rot)) = clip(c, r.depth_clip) else { return Vec::new() };
    let scr: Vec<((i32, i32), f32, f32)> = table.iter().map(|p| screen(p, r)).collect();
    let comps = c.iter().map(|v| v.attrs.len()).max().unwrap_or(0);
    let vals: Vec<Vec<f32>> = (0..comps)
        .map(|k| {
            let mode = modes.get(k / 4).copied().unwrap_or(Mode::Undefined);
            values(&table, rot, std::array::from_fn(|i| c[i].attrs.get(k).copied().unwrap_or(0)), mode)
        })
        .collect();
    let mut out = Vec::new();
    for i in 1..poly.len().saturating_sub(1) {
        let idx = [poly.first().copied().unwrap_or(0), poly.get(i).copied().unwrap_or(0), poly.get(i.saturating_add(1)).copied().unwrap_or(0)];
        let s = idx.map(|k| scr.get(k).map_or((0, 0), |v| v.0));
        let Some(g) = geometry(s) else { continue };
        if !facing(s, r) {
            continue;
        }
        let rhw = idx.map(|k| scr.get(k).map_or(0.0, |v| v.2));
        let z = idx.map(|k| scr.get(k).map_or(0.0, |v| v.1));
        let attrs = vals
            .iter()
            .enumerate()
            .map(|(k, v)| {
                let at = idx.map(|j| v.get(j).copied().unwrap_or(0.0));
                match modes.get(k / 4).copied().unwrap_or(Mode::Undefined) {
                    Mode::Undefined => Plane::default(),
                    Mode::Constant => Plane { gx: 0.0, gy: 0.0, c: f32::from_bits(c[0].attrs.get(k).copied().unwrap_or(0)) },
                    Mode::Perspective(_) => plane(&g, [at[0] * rhw[0], at[1] * rhw[1], at[2] * rhw[2]]),
                    Mode::NoPerspective(_) => plane(&g, at),
                }
            })
            .collect();
        out.push(Fan { pos: idx.map(|k| table.get(k).map_or([0.0; 4], |p| drawn(p, r.depth_clip))), rhw: plane(&g, rhw), z: plane(&g, z), attrs, scr: s });
    }
    out
}

/// A component's value at (x, y) of a fan triangle (bits; constant ones as set up).
pub fn eval(f: &Fan, k: usize, mode: Mode, x: f32, y: f32) -> u32 {
    let p = f.attrs.get(k).copied().unwrap_or_default();
    match mode {
        Mode::Undefined => 0,
        Mode::Constant => p.c.to_bits(),
        Mode::NoPerspective(_) => p.at(x, y).to_bits(),
        Mode::Perspective(_) => (p.at(x, y) * (1.0 / f.rhw.at(x, y))).to_bits(),
    }
}

/// An aliased line as WARP sets it up (lines.md §1): the nudged start and extent its coverage
/// test reads, the bounds of that test, the planes along the major axis.
#[derive(Clone, Debug)]
pub struct Line {
    pub x0: f32,
    pub y0: f32,
    pub dx: f32,
    pub dy: f32,
    pub hi_col: f32,
    pub hi_row: f32,
    pub rhw: Plane,
    pub z: Plane,
    pub attrs: Vec<Plane>,
    /// the snapped ends (pixels)
    pub ends: [(f32, f32); 2],
}

/// The viewport's clip space position (w 1) of a pixel position at window depth z (the depth
/// range's 0..1, clamped).
fn clip_at(x: f32, y: f32, z: f32, r: &Raster) -> [f32; 4] {
    let [vx, vy, vw, vh, n, f] = r.viewport;
    let (hx, ox, hy, oy) = (vw * 0.5, vw * 0.5 + vx, -vh * 0.5, vh * 0.5 + vy);
    let dz = if f == n { 0.0 } else { ((z - n) / (f - n)).clamp(0.0, 1.0) };
    [(x - ox) / hx, (y - oy) / hy, dz, 1.0]
}

/// The triangle a line's slot draws: over its pixels and one more on every side (the pixel
/// shader's coverage test decides), its depth the line's plane at its corners.
pub fn line_region(l: &Line, r: &Raster) -> [[f32; 4]; 3] {
    let [(x0, y0), (x1, y1)] = l.ends;
    let (bx0, by0) = (x0.min(x1).floor() - 1.0, y0.min(y1).floor() - 1.0);
    let (bx1, by1) = (x0.max(x1).ceil() + 1.0, y0.max(y1).ceil() + 1.0);
    [(bx0, by0), (bx0 + 2.0 * (bx1 - bx0), by0), (bx0, by0 + 2.0 * (by1 - by0))].map(|(x, y)| clip_at(x, y, line_at(l.z, x, y), r))
}

/// ceil(v) - 0.5 as WARP computes it (round to nearest, up when below)
fn owner(v: f32) -> f32 {
    let n = v.round_ties_even();
    let n = if v > n { n + 1.0 } else { n };
    n - 0.5
}

fn diamond(x: f32, y: f32, cx: f32, cy: f32, xmajor: bool) -> bool {
    let d = (x - cx).abs() + (y - cy).abs();
    let inclusive = y > cy || (y == cy && !xmajor && x > cx);
    if inclusive { 0.5 >= d } else { 0.5 > d }
}

/// The clipper's table for a line (its two corners, then each new point) and the two ends in
/// it after every plane in clip.md's order (an end outside moved to the crossing from the
/// inside one), None when it is outside.
fn clip_line(c: &[Corner; 2], depth_clip: bool) -> Option<(Vec<Point>, [usize; 2])> {
    let user = c.iter().map(|v| v.clip.len()).max().unwrap_or(0);
    let culls = c.iter().map(|v| v.cull.len()).max().unwrap_or(0);
    for k in 0..culls {
        if c.iter().all(|v| v.cull.get(k).is_some_and(|d| 0.0 > *d)) {
            return None;
        }
    }
    let mut table: Vec<Point> = c
        .iter()
        .map(|v| {
            let [x, y, z, w] = v.pos;
            let (n, f) = if depth_clip { (z, w - z) } else { (0.0, 0.0) };
            let mut d = vec![n, f, w + x, w - x, w - y, w + y];
            d.extend((0..user).map(|k| v.clip.get(k).copied().unwrap_or(0.0)));
            Point { p: v.pos, d, made: None }
        })
        .collect();
    if depth_clip && table.iter().any(|v| v.d.iter().take(6).any(|x| x.is_nan())) {
        return None;
    }
    let planes = 6usize.saturating_add(user);
    let mut ends = [0usize, 1];
    for k in 0..planes {
        let d = ends.map(|e| table.get(e).and_then(|p| p.d.get(k)).copied().unwrap_or(0.0));
        let out = d.map(|v| 0.0 > v);
        if out[0] && out[1] {
            return None;
        }
        if !out[0] && !out[1] {
            continue;
        }
        let (i, o) = if out[1] { (0, 1) } else { (1, 0) };
        let (inside, outside) = (ends[i], ends[o]);
        let (d_in, d_out) = (d[i], d[o]);
        let den = d_in - d_out;
        let (Some(a), Some(b)) = (table.get(inside), table.get(outside)) else { return None };
        let p = std::array::from_fn(|q| lerp(a.p[q], b.p[q], d_in, den));
        let dd = (0..planes).map(|q| {
            let (x, y) = (a.d.get(q).copied().unwrap_or(0.0), b.d.get(q).copied().unwrap_or(0.0));
            if q > k { lerp(x, y, d_in, den) } else { x }
        });
        let pt = Point { p, d: dd.collect(), made: Some(Made { inside, outside, d_in, d_out, den }) };
        table.push(pt);
        ends[o] = table.len().saturating_sub(1);
    }
    Some((table, ends))
}

/// An aliased line WARP sets up for two corners (API order: the first is the start and the
/// provoking vertex), None for a line it drops.
pub fn line(c: &[Corner; 2], modes: &[Mode], r: &Raster) -> Option<Line> {
    let (table, ends) = clip_line(c, r.depth_clip)?;
    let scr: Vec<((i32, i32), f32, f32)> = table.iter().map(|p| screen(p, r)).collect();
    let e = f32::from_bits(0x3b80_0000);
    let at = |k: usize| scr.get(k).map_or((0.0, 0.0), |v| (f32_of(i128::from(v.0.0)) * e, f32_of(i128::from(v.0.1)) * e));
    let ((x0, y0), (x1, y1)) = (at(ends[0]), at(ends[1]));
    if x0 == x1 && y0 == y1 {
        return None;
    }
    let xmajor = (x0 - x1).abs() >= (y0 - y1).abs();
    let (dx, dy) = (x1 - x0, y1 - y0);
    let (cx0, cy0, cx1, cy1) = (owner(x0), owner(y0), owner(x1), owner(y1));
    let in_s = diamond(x0, y0, cx0, cy0, xmajor);
    let in_e = diamond(x1, y1, cx1, cy1, xmajor);
    let (hi_col, hi_row) = (f32::from_bits(0x3f00_0001), if xmajor { 0.5 } else { f32::from_bits(0x3f00_0001) });
    let (mut sx, mut sy, mut sdx, mut sdy) = (x0, y0, dx, dy);
    if in_s && in_e && cx0 == cx1 && cy0 == cy1 {
        (sx, sy, sdx, sdy) = (-0.5, -0.5, 0.0, 0.0);
    } else {
        if in_s {
            let (u, v) = if xmajor {
                let u = (x0 - cx0) + if dx > 0.0 { e } else { -e };
                (u, (dy / dx) * u)
            } else {
                let v = (y0 - cy0) + if dy > 0.0 { e } else { -e };
                ((dx / dy) * v, v)
            };
            (sx, sy, sdx, sdy) = (x0 - u, y0 - v, dx + u, dy + v);
        }
        if in_e {
            let (u, v) = if xmajor {
                let u = ((sx + sdx) - cx1) + if sdx > 0.0 { e } else { -e };
                (u, (sdy / sdx) * u)
            } else {
                let v = ((sy + sdy) - cy1) + if sdy > 0.0 { e } else { -e };
                ((sdx / sdy) * v, v)
            };
            (sdx, sdy) = (sdx - u, sdy - v);
        }
    }
    // the planes along the major axis, from the snapped ends
    let (d, base) = if xmajor { (x1 - x0, x0) } else { (y1 - y0, y0) };
    let inv = 1.0 / d;
    let axis = |v0: f32, v1: f32| -> Plane {
        let k = (v1 - v0) * inv;
        let c0 = v0 - k * base;
        if xmajor { Plane { gx: k, gy: 0.0, c: c0 } } else { Plane { gx: 0.0, gy: k, c: c0 } }
    };
    let rhw = ends.map(|k| scr.get(k).map_or(0.0, |v| v.2));
    let z = ends.map(|k| scr.get(k).map_or(0.0, |v| v.1));
    let comps = c.iter().map(|v| v.attrs.len()).max().unwrap_or(0);
    let attrs = (0..comps)
        .map(|k| {
            let mode = modes.get(k / 4).copied().unwrap_or(Mode::Undefined);
            let corners = [c[0].attrs.get(k).copied().unwrap_or(0), c[1].attrs.get(k).copied().unwrap_or(0), 0];
            let v = line_values(&table, corners, mode);
            let at = ends.map(|j| v.get(j).copied().unwrap_or(0.0));
            match mode {
                Mode::Undefined => Plane::default(),
                Mode::Constant => Plane { gx: 0.0, gy: 0.0, c: f32::from_bits(corners[0]) },
                Mode::Perspective(_) => axis(at[0] * rhw[0], at[1] * rhw[1]),
                Mode::NoPerspective(_) => axis(at[0], at[1]),
            }
        })
        .collect();
    Some(Line { x0: sx, y0: sy, dx: sdx, dy: sdy, hi_col, hi_row, rhw: axis(rhw[0], rhw[1]), z: axis(z[0], z[1]), attrs, ends: [(x0, y0), (x1, y1)] })
}

/// Each table point's value of one component on a line (the corners as given, new points by
/// their mode's rule).
fn line_values(table: &[Point], corners: [u32; 3], mode: Mode) -> Vec<f32> {
    let mut v: Vec<f32> = Vec::with_capacity(table.len());
    for (i, pt) in table.iter().enumerate() {
        let x = match (pt.made, mode) {
            (_, Mode::Undefined) => 0.0,
            (_, Mode::Constant) => f32::from_bits(corners[0]),
            // (the ends' copies at the table's end: the point each copies)
            (None, _) => f32::from_bits(corners.get(i).copied().unwrap_or(0)),
            (Some(m), Mode::Perspective(_)) => lerp(v.get(m.inside).copied().unwrap_or(0.0), v.get(m.outside).copied().unwrap_or(0.0), m.d_in, m.den),
            (Some(m), Mode::NoPerspective(_)) => {
                let (a, b) = (v.get(m.inside).copied().unwrap_or(0.0), v.get(m.outside).copied().unwrap_or(0.0));
                let w = |k: usize| table.get(k).map_or(0.0, |p| p.p[3]);
                let s = screen_s(w(m.inside), w(m.outside), m.d_in, m.d_out);
                (b - a) * s + a
            }
        };
        v.push(x);
    }
    v
}

/// Whether an aliased line lights the pixel whose centre is (px, py) (lines.md §1.2).
pub fn line_covers(l: &Line, px: f32, py: f32) -> bool {
    let (inv_dx, inv_dy) = (-1.0 / l.dx, -1.0 / l.dy);
    let (a, b) = (l.x0 - px, l.y0 - py);
    let t = a * inv_dx;
    let col = (0.0..=1.0).contains(&t) && {
        let e = t * l.dy + b;
        e > -0.5 && l.hi_col > e
    };
    let t2 = b * inv_dy;
    let row = (0.0..=1.0).contains(&t2) && {
        let e = t2 * l.dx + a;
        e > -0.5 && l.hi_row > e
    };
    col || row
}

/// A point as WARP sets it up: the pixel it lights and its values (constant planes: a
/// perspective input's value times rhw, the pixel shader's divide by rhw after).
#[derive(Clone, Debug)]
pub struct Dot {
    /// the pixel: the one whose centre is in [X - 0.5, X + 0.5) of the snapped position
    pub pixel: (i32, i32),
    pub rhw: f32,
    pub z: f32,
    pub attrs: Vec<f32>,
}

/// A point WARP draws for a corner, None when a plane has it outside.
pub fn point(c: &Corner, modes: &[Mode], r: &Raster) -> Option<Dot> {
    let [x, y, z, w] = c.pos;
    let (n, f) = if r.depth_clip { (z, w - z) } else { (0.0, 0.0) };
    let d = [n, f, w + x, w - x, w - y, w + y];
    if d.iter().any(|v| 0.0 > *v || v.is_nan()) || c.clip.iter().chain(&c.cull).any(|v| 0.0 > *v) {
        return None;
    }
    let pt = Point { p: c.pos, d: d.to_vec(), made: None };
    let ((sx, sy), z, rhw) = screen(&pt, r);
    // (the pixel i with i + 0.5 in [X - 0.5, X + 0.5): i = ceil(X - 1) in 1/256 units)
    let px = |v: i32| -> i32 { (v.saturating_sub(256)).div_euclid(256).saturating_add(i32::from(v.rem_euclid(256) != 0)) };
    let attrs = c
        .attrs
        .iter()
        .enumerate()
        .map(|(k, &a)| match modes.get(k / 4).copied().unwrap_or(Mode::Undefined) {
            Mode::Undefined => 0.0,
            Mode::Constant | Mode::NoPerspective(_) => f32::from_bits(a),
            Mode::Perspective(_) => f32::from_bits(a) * rhw,
        })
        .collect();
    Some(Dot { pixel: (px(sx), px(sy)), rhw, z, attrs })
}

/// The two triangles a point's slots draw: a square inside its pixel holding the centre and
/// every standard sample position, at its depth.
pub fn dot_squares(d: &Dot, r: &Raster) -> [[[f32; 4]; 3]; 2] {
    let (px, py) = (f32_of(i128::from(d.pixel.0)), f32_of(i128::from(d.pixel.1)));
    let (lo, hi) = ((px + 0.0625, py + 0.0625), (px + 0.9375, py + 0.9375));
    let q = [lo, (hi.0, lo.1), (lo.0, hi.1), hi].map(|(x, y)| clip_at(x, y, d.z, r));
    [[q[0], q[1], q[2]], [q[1], q[3], q[2]]]
}

/// A point's component at its pixel (bits).
pub fn dot_eval(d: &Dot, k: usize, mode: Mode) -> u32 {
    let v = d.attrs.get(k).copied().unwrap_or(0.0);
    match mode {
        Mode::Undefined => 0,
        Mode::Constant | Mode::NoPerspective(_) => v.to_bits(),
        Mode::Perspective(_) => (v * (1.0 / d.rhw)).to_bits(),
    }
}

/// A line plane's value at (x, y): A * x + (B * y + C) (lines.md §1.4).
pub fn line_at(p: Plane, x: f32, y: f32) -> f32 {
    p.gx * x + (p.gy * y + p.c)
}

// ---------------------------------------------------------------------------------------------
// The setup pass (WGSL)

/// The setup store's binding in the setup pass (read-write) and in the pixel shader (group 1,
/// read).
pub const B_SETUP: u32 = 460;

/// Exact integer to f32 conversions (WGSL leaves which neighbour an inexact one takes to the
/// implementation), WARP's setup of a fan triangle and of one component's planes.
/// A product passed through sx_r is rounded on its own: sx_k is a 0 no compiler can see (the
/// entry point sets it from a runtime word whose bit it keeps is never set), so the product
/// is never fused with the add that takes it (WGSL leaves contraction to the implementation;
/// WARP rounds the product and the sum each).
const SX_ROUND: &str = "var<private> sx_k = 0u;
fn sx_r(v: f32) -> f32 { return bitcast<f32>(bitcast<u32>(v) | sx_k); }
fn sx_r3(v: vec3<f32>) -> vec3<f32> { return bitcast<vec3<f32>>(bitcast<vec3<u32>>(v) | vec3<u32>(sx_k)); }
";

const SETUP_WGSL: &str = r#"
fn sx_u64_f32(hi: u32, lo: u32) -> f32 {
    if (hi == 0u && lo < 0x1000000u) { return f32(lo); }
    var top: u32;
    if (hi != 0u) { top = 63u - countLeadingZeros(hi); } else { top = 31u - countLeadingZeros(lo); }
    let sh = top - 23u;
    // (the 24 bits from the top, the rest's half and sticky)
    var m: u32; var rest_hi: u32; var rest_lo: u32; var half_hi = 0u; var half_lo = 0u;
    if (sh >= 32u) {
        m = hi >> (sh - 32u);
        rest_hi = hi & ((1u << (sh - 32u)) - 1u);
        rest_lo = lo;
        if (sh == 32u) { half_lo = 0x80000000u; } else { half_hi = 1u << (sh - 33u); }
    } else {
        m = (lo >> sh) | select(0u, hi << (32u - sh), sh != 0u);
        rest_hi = 0u;
        rest_lo = lo & ((1u << sh) - 1u);
        half_lo = 1u << (sh - 1u);
    }
    let above = rest_hi > half_hi || (rest_hi == half_hi && rest_lo > half_lo);
    let tie = rest_hi == half_hi && rest_lo == half_lo;
    if (above || (tie && (m & 1u) == 1u)) { m += 1u; }
    var e = top;
    if ((m >> 24u) != 0u) { m = m >> 1u; e += 1u; }
    return bitcast<f32>(((e + 127u) << 23u) | (m & 0x7fffffu));
}
// a 64-bit two's complement (lo, hi) as f32
fn sx_i64_f32(v: vec2<u32>) -> f32 {
    if ((v.y >> 31u) != 0u) {
        let n = d_neg(v);
        return -sx_u64_f32(n.y, n.x);
    }
    return sx_u64_f32(v.y, v.x);
}
fn sx_i32_f32(v: i32) -> f32 {
    if (v < 0) { return -sx_u64_f32(0u, u32(-v)); }
    return sx_u64_f32(0u, u32(v));
}
// a * b - c * d exactly (64 bits)
fn sx_det(a: i32, b: i32, c: i32, d: i32) -> vec2<u32> {
    return d_add(m64(a, b), d_neg(m64(c, d)));
}
// the geometry of the fan triangle s0 s1 s2 (x0 y0 dx10 dx02, dy01 dy20 inv valid)
var<private> sx_ga: vec4<f32>;
var<private> sx_gb: vec4<f32>;
fn sx_geometry(s0: vec2<i32>, s1: vec2<i32>, s2: vec2<i32>) -> bool {
    let det = sx_det(s2.y - s0.y, s1.x - s0.x, s0.y - s1.y, s0.x - s2.x);
    if (det.x == 0u && det.y == 0u) { return false; }
    let e = 0.00390625f;
    let inv = ps_div(1.0f, sx_i64_f32(det) * 0.0000152587890625f);
    sx_ga = vec4<f32>(sx_i32_f32(s0.x) * e, sx_i32_f32(s0.y) * e, sx_i32_f32(s1.x - s0.x) * e, sx_i32_f32(s0.x - s2.x) * e);
    sx_gb = vec4<f32>(sx_i32_f32(s0.y - s1.y) * e, sx_i32_f32(s2.y - s0.y) * e, inv, 0.0f);
    let bits = xw(XD_RASTER);
    let neg = (det.y >> 31u) != 0u;
    let front = select(!neg, neg, (bits & 4u) != 0u);
    let cull = bits & 3u;
    return !((cull == 2u && front) || (cull == 3u && !front));
}
fn sx_plane(a0: f32, a1: f32, a2: f32) -> vec3<f32> {
    let d10 = a1 - a0;
    let d20 = a2 - a0;
    let gx = sx_r((sx_r(d10 * sx_gb.y) + sx_r(d20 * sx_gb.x)) * sx_gb.z);
    let gy = sx_r((sx_r(d10 * sx_ga.w) + sx_r(d20 * sx_ga.z)) * sx_gb.z);
    return vec3<f32>(gx, gy, (a0 - sx_r(gx * sx_ga.x)) - sx_r(gy * sx_ga.y));
}
fn sx_screen_s(w_in: f32, w_out: f32, d_in: f32, d_out: f32) -> f32 {
    let num = d_in * w_out;
    let den = sx_r((w_out - w_in) * d_in) + sx_r((d_in - d_out) * w_in);
    return ps_div(num, den);
}
"#;

/// The setup pass: each list triangle of `store` (positions and the components the pixel
/// shader `ps` reads by semantic (none: no pixel shader), `osgn` the outputs of the shader that
/// wrote it) through WARP's clipper and setup, its fan triangles' slots into `setup` at `item *
/// slots`: list primitive `item` + xd FIRST (a draw set up in pieces).
pub fn setup_pass(store: &Store, osgn: &[SigElement], ps: Option<&[u8]>, prim: Prim) -> Result<Wgsl, String> {
    let layout = match ps {
        Some(p) => Layout::of(p)?,
        None => Layout::default(),
    };
    let stream = match store {
        Store::Vertex { .. } => 0,
        Store::Geometry { stream, .. } => *stream,
    };
    let osgn: Vec<SigElement> = osgn.iter().filter(|e| e.stream == stream).cloned().collect();
    let isgn = match ps {
        Some(p) => parse(p)?.0.signature(false),
        None => Vec::new(),
    };
    if store.corners() != prim.corners() {
        return Err(format!("a setup of {prim:?} from {}-corner primitives", store.corners()));
    }
    let regs = store.regs();
    let reg_of = |corner: &str, r: u32, c: u8| format!("store[{corner} * {regs}u + {r}u].{}", XYZW.get(usize::from(c)).copied().unwrap_or('x'));
    // the store's clip and cull distances, in order (SV_ClipDistance0 x..w, then 1)
    let dist = |sv: u32| -> Vec<(u32, u8)> {
        let mut e: Vec<&SigElement> = osgn.iter().filter(|e| e.sysval == sv).collect();
        e.sort_by_key(|e| e.index);
        e.iter().flat_map(|e| bits(e.mask).into_iter().map(|c| (e.reg, c))).collect()
    };
    let clips = dist(2);
    let culls = dist(3);
    let user = kome::num::u32_of(clips.len());
    let planes = 6u32.saturating_add(user);
    let nslots = prim.slots(user);
    let stride = layout.stride();
    // (points: the 3 corners and at most 2 a plane; the polygon at most 3 + planes)
    let maxt = planes.saturating_mul(2).saturating_add(3);
    let maxp = planes.saturating_add(3);
    // each input component of the layout: (its slot vec4, component, mode word, store source)
    let mut comps: Vec<Comp> = Vec::new();
    for (i, inp) in (0u32..).zip(&layout.inputs) {
        let at = HEADER.saturating_add(i.saturating_mul(3));
        for pcomp in bits(inp.mask) {
            let mut src = None;
            for e in isgn.iter().filter(|e| e.reg == inp.reg && e.mask & (1 << pcomp) != 0) {
                let Some(f) = osgn.iter().find(|f| f.sysval == e.sysval && f.name.eq_ignore_ascii_case(&e.name) && f.index == e.index) else { continue };
                // (the element's components in order: the pixel shader's to the store's)
                if let Some((_, vc)) = bits(e.mask).into_iter().zip(bits(f.mask)).find(|(p, _)| *p == pcomp) {
                    src = Some((f.reg, vc));
                }
            }
            comps.push((at, pcomp, inp.mode.word(), src));
        }
    }
    let val = |corner: &str, src: Option<(u32, u8)>| match src {
        Some((r, vc)) => reg_of(corner, r, vc),
        None => "0u".to_string(),
    };
    let pos = osgn.iter().find(|e| e.sysval == 1).map(|e| e.reg);
    let pos_of = |corner: &str| match pos {
        Some(r) => format!("bitcast<vec4<f32>>(store[{corner} * {regs}u + {r}u])"),
        None => "vec4<f32>(0.0f)".to_string(),
    };
    let dc = "(xw(XD_RASTER) & 8u) != 0u";
    let mut m = String::new();
    let mut bindings = Vec::new();
    xd_decl(&mut m, &mut bindings);
    store.decls(&mut m, &mut bindings);
    wln!(m, "@group(0) @binding({B_SETUP}) var<storage, read_write> setup: array<vec4<u32>>;");
    bindings.push((B_SETUP, Binding::StorageBuffer(true)));
    m.push_str(ps::WGSL);
    m.push_str(ps::DIV);
    m.push_str(SX_ROUND);
    m.push_str(SETUP_WGSL);
    wln!(m, "const SX_PLANES = {planes}u;\nconst SX_SLOTS = {nslots}u;\nconst SX_STRIDE = {stride}u;");
    wln!(m, "var<private> tp: array<vec4<f32>, {maxt}>;\nvar<private> td: array<array<f32, {planes}>, {maxt}>;");
    wln!(m, "var<private> tin: array<u32, {maxt}>;\nvar<private> tout: array<u32, {maxt}>;\nvar<private> tdin: array<f32, {maxt}>;\nvar<private> tdout: array<f32, {maxt}>;\nvar<private> tden: array<f32, {maxt}>;\nvar<private> tn = 0u;");
    wln!(m, "var<private> rot: array<u32, 3>;\nvar<private> fan: array<vec3<u32>, {nslots}>;\nvar<private> fga: array<vec4<f32>, {nslots}>;\nvar<private> fgb: array<vec4<f32>, {nslots}>;\nvar<private> frhw: array<vec3<f32>, {nslots}>;\nvar<private> nf = 0u;\nvar<private> base = 0u;");
    wln!(m, "var<private> av: array<f32, {maxt}>;");
    // one component: its value at every table point, then its plane on every fan triangle
    wln!(
        m,
        "fn setup_c(at: u32, c: u32, mode: u32, v0: u32, v1: u32, v2: u32) {{
    let v = array<u32, 3>(v0, v1, v2);
    for (var i = 0u; i < tn; i += 1u) {{
        if (mode == 0u) {{ av[i] = 0.0f; }}
        else if (mode == 1u) {{ av[i] = bitcast<f32>(v0); }}
        else if (i < 3u) {{ av[i] = bitcast<f32>(v[rot[i]]); }}
        else {{
            let a = av[tin[i]]; let b = av[tout[i]];
            if (mode == 2u) {{ av[i] = lerp2(a, b, tdin[i], tden[i]); }}
            else {{
                let s = sx_screen_s(tp[tin[i]].w, tp[tout[i]].w, tdin[i], tdout[i]);
                av[i] = sx_r((b - a) * s) + a;
            }}
        }}
    }}
    for (var f = 0u; f < nf; f += 1u) {{
        let o = (base + f) * SX_STRIDE + at;
        if (mode <= 1u) {{
            setup[o][c] = 0u; setup[o + 1u][c] = 0u;
            setup[o + 2u][c] = select(0u, v0, mode == 1u);
            continue;
        }}
        sx_ga = fga[f]; sx_gb = fgb[f];
        let k = fan[f];
        var a = vec3<f32>(av[k.x], av[k.y], av[k.z]);
        if (mode == 2u) {{ a = sx_r3(a * frhw[f]); }}
        let p = sx_plane(a.x, a.y, a.z);
        setup[o][c] = bitcast<u32>(p.x); setup[o + 1u][c] = bitcast<u32>(p.y); setup[o + 2u][c] = bitcast<u32>(p.z);
    }}
}}"
    );
    if matches!(prim, Prim::Line | Prim::Wire) {
        line_fns(&mut m, &clips, &culls, &comps, &val, &pos_of, regs);
    }
    if prim == Prim::Point {
        point_fn(&mut m, &clips, &culls, &comps, &val, &pos_of, regs);
    }
    wln!(m, "@compute @workgroup_size(64)\nfn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {{\n    {ITEM}");
    m.push_str(ps::ENTRY);
    wln!(m, "    sx_k = ps_k;\n    let t = item + xw(XD_FIRST);\n    base = item * SX_SLOTS;");
    // (an empty slot: three equal corners)
    m.push_str("    for (var f = 0u; f < SX_SLOTS; f += 1u) {\n        let o = (base + f) * SX_STRIDE;\n        setup[o] = vec4<u32>(0u); setup[o + 1u] = vec4<u32>(0u); setup[o + 2u] = vec4<u32>(0u);\n    }\n");
    let corners = prim.corners();
    for c in 0..corners {
        wln!(m, "    let e{c} = list[({}) * {corners}u + {c}u];", store.slot("t"));
        wln!(m, "    if (e{c} == 0xffffffffu) {{ return; }}");
    }
    match prim {
        Prim::Point => {
            m.push_str("    point_slots(e0);\n}\n");
            return Ok(Wgsl { code: m, bindings });
        }
        Prim::Line => {
            m.push_str("    line_slot(0u, e0, e1);\n}\n");
            return Ok(Wgsl { code: m, bindings });
        }
        Prim::Triangle | Prim::Wire => {}
    }
    for c in 0..3 {
        wln!(m, "    let p{c} = {};", pos_of(&format!("e{c}")));
    }
    // cull distances
    for &(r, cc) in &culls {
        let x = XYZW.get(usize::from(cc)).copied().unwrap_or('x');
        wln!(m, "    if (0.0f > bitcast<f32>(store[e0 * {regs}u + {r}u].{x}) && 0.0f > bitcast<f32>(store[e1 * {regs}u + {r}u].{x}) && 0.0f > bitcast<f32>(store[e2 * {regs}u + {r}u].{x})) {{ return; }}");
    }
    // the turn
    m.push_str(
        "    var s0 = 0u;\n    if (p0.x > p1.x || (p0.x == p1.x && p0.y > p1.y)) { s0 = 1u; }\n    let ps = array<vec4<f32>, 3>(p0, p1, p2);\n    var sel = s0;\n    if (ps[s0].x > p2.x || (ps[s0].x == p2.x && ps[s0].y > p2.y)) { sel = 2u; }\n    rot = array<u32, 3>(sel, (sel + 1u) % 3u, (sel + 2u) % 3u);\n",
    );
    m.push_str("    let es = array<u32, 3>(e0, e1, e2);\n");
    wln!(m, "    let dclip = {dc};");
    m.push_str("    var or = 0u; var and = 0xffffffffu; var nan = false;\n    for (var i = 0u; i < 3u; i += 1u) {\n        let p = ps[rot[i]];\n        tp[i] = p;\n        let e = es[rot[i]];\n        var code = 0u;\n        for (var k = 0u; k < 6u; k += 1u) {\n            let d = ps_dist(p, k, dclip);\n            td[i][k] = d;\n            if (dclip && d != d) { nan = true; }\n            if (ps_neg(d)) { code |= 1u << k; }\n        }\n");
    for (k, &(r, cc)) in clips.iter().enumerate() {
        let x = XYZW.get(usize::from(cc)).copied().unwrap_or('x');
        wln!(m, "        {{ let d = bitcast<f32>(store[e * {regs}u + {r}u].{x}); td[i][{}u] = d; if (ps_neg(d)) {{ code |= 1u << {}u; }} }}", k.saturating_add(6), k.saturating_add(6));
    }
    m.push_str("        or |= code; and &= code;\n    }\n    if (and != 0u || nan) { return; }\n    tn = 3u;\n");
    // the clipper
    wln!(m, "    var poly: array<u32, {maxp}>;\n    var next: array<u32, {maxp}>;\n    var n = 3u;\n    poly[0] = 0u; poly[1] = 1u; poly[2] = 2u;");
    m.push_str(
        "    for (var k = 0u; k < SX_PLANES; k += 1u) {
        if (((or >> k) & 1u) == 0u) { continue; }
        var o = 0u;
        for (var i = 0u; i < n; i += 1u) {
            let a = poly[i];
            let b = poly[(i + 1u) % n];
            let da = td[a][k];
            let db = td[b][k];
            let a_out = 0.0f > da;
            let b_out = 0.0f > db;
            var inside = a; var outside = b; var din = da; var dout = db; var cross = false;
            if (!a_out) {
                next[o] = a; o += 1u;
                cross = b_out;
            } else if (!b_out) {
                inside = b; outside = a; din = db; dout = da; cross = true;
            }
            if (cross) {
                let den = din - dout;
                let j = tn;
                tn += 1u;
                tp[j] = lerp4(tp[inside], tp[outside], din, den);
                for (var q = 0u; q < SX_PLANES; q += 1u) {
                    if (q > k && ((or >> q) & 1u) != 0u) { td[j][q] = lerp2(td[inside][q], td[outside][q], din, den); }
                    else { td[j][q] = td[inside][q]; }
                }
                tin[j] = inside; tout[j] = outside; tdin[j] = din; tdout[j] = dout; tden[j] = den;
                next[o] = j; o += 1u;
            }
        }
        n = o;
        if (n == 0u) { return; }
        for (var i = 0u; i < n; i += 1u) { poly[i] = next[i]; }
    }
",
    );
    // the fan, its corners' screen positions, depths and rhws, the header of each slot
    const FAN: &str = "    let vx = bitcast<f32>(xw(XD_VIEWPORT)); let vy = bitcast<f32>(xw(XD_VIEWPORT + 1u));
    let vw = bitcast<f32>(xw(XD_VIEWPORT + 2u)); let vh = bitcast<f32>(xw(XD_VIEWPORT + 3u));
    let zn = bitcast<f32>(xw(XD_DEPTH)); let zf = bitcast<f32>(xw(XD_DEPTH + 1u));
    nf = 0u;
    for (var i = 1u; i + 1u < n; i += 1u) {
        let k = vec3<u32>(poly[0], poly[i], poly[i + 1u]);
        var s: array<vec2<i32>, 3>;
        var z: array<f32, 3>;
        var r: array<f32, 3>;
        var dp: array<vec4<f32>, 3>;
        for (var c = 0u; c < 3u; c += 1u) {
            let j = k[c];
            let made = j >= 3u;
            let p = tp[j];
            s[c] = ps_snap(p, made);
            let w = select(p.w, 1.17549435e-38f, 1.17549435e-38f > p.w);
            var zc = p.z;
            var q = p;
            if (made) {
                if (dclip) { zc = min(max(zc, 0.0f), w); }
                q = vec4<f32>(min(max(p.x, -w), w), min(max(p.y, -w), w), zc, p.w);
            }
            let rhw = ps_div(1.0f, w);
            r[c] = rhw;
            z[c] = sx_r(rhw * zc * (zf - zn)) + zn;
            dp[c] = q;
        }
        if (!sx_geometry(s[0], s[1], s[2])) { continue; }
        let o = (base + nf) * SX_STRIDE;
        fan[nf] = k;
        fga[nf] = sx_ga;
        fgb[nf] = sx_gb;
        frhw[nf] = vec3<f32>(r[0], r[1], r[2]);
        setup[o] = bitcast<vec4<u32>>(dp[0]); setup[o + 1u] = bitcast<vec4<u32>>(dp[1]); setup[o + 2u] = bitcast<vec4<u32>>(dp[2]);
        let pr = sx_plane(r[0], r[1], r[2]);
        let pz = sx_plane(z[0], z[1], z[2]);
        setup[o + 3u] = vec4<u32>(bitcast<u32>(pr.x), bitcast<u32>(pr.y), bitcast<u32>(pr.z), 0u);
        setup[o + 4u] = vec4<u32>(bitcast<u32>(pz.x), bitcast<u32>(pz.y), bitcast<u32>(pz.z), 0u);
        nf += 1u;
    }
";
    const FAN_WIRE: &str = "    let vx = bitcast<f32>(xw(XD_VIEWPORT)); let vy = bitcast<f32>(xw(XD_VIEWPORT + 1u));
    let vw = bitcast<f32>(xw(XD_VIEWPORT + 2u)); let vh = bitcast<f32>(xw(XD_VIEWPORT + 3u));
    let zn = bitcast<f32>(xw(XD_DEPTH)); let zf = bitcast<f32>(xw(XD_DEPTH + 1u));
    nf = 0u;
    for (var i = 1u; i + 1u < n; i += 1u) {
        let k = vec3<u32>(poly[0], poly[i], poly[i + 1u]);
        var s: array<vec2<i32>, 3>;
        var z: array<f32, 3>;
        var r: array<f32, 3>;
        var dp: array<vec4<f32>, 3>;
        for (var c = 0u; c < 3u; c += 1u) {
            let j = k[c];
            let made = j >= 3u;
            let p = tp[j];
            s[c] = ps_snap(p, made);
            let w = select(p.w, 1.17549435e-38f, 1.17549435e-38f > p.w);
            var zc = p.z;
            var q = p;
            if (made) {
                if (dclip) { zc = min(max(zc, 0.0f), w); }
                q = vec4<f32>(min(max(p.x, -w), w), min(max(p.y, -w), w), zc, p.w);
            }
            let rhw = ps_div(1.0f, w);
            r[c] = rhw;
            z[c] = sx_r(rhw * zc * (zf - zn)) + zn;
            dp[c] = q;
        }
        if (!sx_geometry(s[0], s[1], s[2])) { continue; }
        // (WIREFRAME: drawn when a fan triangle is, culling as solid)
        nf += 1u;
        break;
    }
";
    m.push_str(if prim == Prim::Wire { FAN_WIRE } else { FAN });
    if prim == Prim::Wire {
        // (each edge an aliased line of its own, in the API's corner order)
        m.push_str("    if (nf == 0u) { return; }\n    line_slot(0u, e0, e1);\n    line_slot(1u, e1, e2);\n    line_slot(2u, e2, e0);\n}\n");
        return Ok(Wgsl { code: m, bindings });
    }
    // each input component of the layout, from the store by semantic
    for &(at, pcomp, mode, src) in &comps {
        wln!(m, "    setup_c({at}u, {pcomp}u, {mode}u, {}, {}, {});", val("e0", src), val("e1", src), val("e2", src));
    }
    m.push_str("}\n");
    Ok(Wgsl { code: m, bindings })
}

/// An input component of the layout in the setup pass: its slot vec4, component, mode word
/// and the store's register and component it comes from (None: 0).
type Comp = (u32, u8, u32, Option<(u32, u8)>);

/// A store component's value expression for a corner index variable.
type Val<'a> = &'a dyn Fn(&str, Option<(u32, u8)>) -> String;

/// The WGSL that clips one point's distances (clip and cull) for corner variable `e` into
/// `td[i]`, `fail` run when it is NaN with depth clip.
fn dist_rows(m: &mut String, clips: &[(u32, u8)], regs: u32, e: &str, i: &str) {
    wln!(m, "        for (var k = 0u; k < 6u; k += 1u) {{ td[{i}][k] = ps_dist(tp[{i}], k, dclip); }}");
    for (k, &(r, cc)) in clips.iter().enumerate() {
        let x = XYZW.get(usize::from(cc)).copied().unwrap_or('x');
        wln!(m, "        td[{i}][{}u] = bitcast<f32>(store[{e} * {regs}u + {r}u].{x});", k.saturating_add(6));
    }
}

/// The WGSL of an aliased line's setup (lines.md §1): `line_slot(f, ea, eb)` clips the line
/// from corner ea to eb (clip.md's planes, the inside point's interpolation), snaps its ends,
/// nudges the start and end out of their diamonds, and writes slot f: a triangle over the
/// line's region, the coverage test's numbers, the planes along the major axis.
fn line_fns(m: &mut String, clips: &[(u32, u8)], culls: &[(u32, u8)], comps: &[Comp], val: Val, pos_of: &dyn Fn(&str) -> String, regs: u32) {
    m.push_str(
        "var<private> lx_major = false;
var<private> l_inv = 0.0f;
var<private> l_base = 0.0f;
var<private> lrhw = vec2<f32>(0.0f);
var<private> lend = vec2<u32>(0u, 1u);
fn line_axis(v0: f32, v1: f32) -> vec3<f32> {
    let k = (v1 - v0) * l_inv;
    let c = v0 - sx_r(k * l_base);
    if (lx_major) { return vec3<f32>(k, 0.0f, c); }
    return vec3<f32>(0.0f, k, c);
}
fn setup_lc(o: u32, c: u32, mode: u32, v0: u32, v1: u32) {
    let v = array<u32, 2>(v0, v1);
    for (var i = 0u; i < tn; i += 1u) {
        if (mode == 0u) { av[i] = 0.0f; }
        else if (mode == 1u) { av[i] = bitcast<f32>(v0); }
        else if (i < 2u) { av[i] = bitcast<f32>(v[i]); }
        else {
            let a = av[tin[i]]; let b = av[tout[i]];
            if (mode == 2u) { av[i] = lerp2(a, b, tdin[i], tden[i]); }
            else {
                let s = sx_screen_s(tp[tin[i]].w, tp[tout[i]].w, tdin[i], tdout[i]);
                av[i] = sx_r((b - a) * s) + a;
            }
        }
    }
    if (mode <= 1u) {
        setup[o][c] = 0u; setup[o + 1u][c] = 0u;
        setup[o + 2u][c] = select(0u, v0, mode == 1u);
        return;
    }
    var a = vec2<f32>(av[lend.x], av[lend.y]);
    if (mode == 2u) { a = vec2<f32>(sx_r(a.x * lrhw.x), sx_r(a.y * lrhw.y)); }
    let p = line_axis(a.x, a.y);
    setup[o][c] = bitcast<u32>(p.x); setup[o + 1u][c] = bitcast<u32>(p.y); setup[o + 2u][c] = bitcast<u32>(p.z);
}
// ceil(v) - 0.5 as WARP takes it: round to nearest, up when below
fn l_owner(v: f32) -> f32 {
    var n = round(v);
    if (v > n) { n = n + 1.0f; }
    return n - 0.5f;
}
fn l_diamond(x: f32, y: f32, cx: f32, cy: f32, xm: bool) -> bool {
    let d = abs(x - cx) + abs(y - cy);
    if (y > cy || (y == cy && !xm && x > cx)) { return 0.5f >= d; }
    return 0.5f > d;
}
",
    );
    wln!(m, "fn line_slot(f: u32, ea: u32, eb: u32) {{\n    let o = (base + f) * SX_STRIDE;");
    for &(r, cc) in culls {
        let x = XYZW.get(usize::from(cc)).copied().unwrap_or('x');
        wln!(m, "    if (0.0f > bitcast<f32>(store[ea * {regs}u + {r}u].{x}) && 0.0f > bitcast<f32>(store[eb * {regs}u + {r}u].{x})) {{ return; }}");
    }
    wln!(m, "    let dclip = (xw(XD_RASTER) & 8u) != 0u;\n    tp[0] = {};\n    tp[1] = {};", pos_of("ea"), pos_of("eb"));
    m.push_str("    {\n");
    dist_rows(m, clips, regs, "ea", "0u");
    dist_rows(m, clips, regs, "eb", "1u");
    m.push_str("    }\n");
    m.push_str(
        "    if (dclip) { for (var k = 0u; k < 6u; k += 1u) { if (td[0][k] != td[0][k] || td[1][k] != td[1][k]) { return; } } }
    tn = 2u;
    lend = vec2<u32>(0u, 1u);
    for (var k = 0u; k < SX_PLANES; k += 1u) {
        let da = td[lend.x][k];
        let db = td[lend.y][k];
        let ao = 0.0f > da;
        let bo = 0.0f > db;
        if (ao && bo) { return; }
        if (!ao && !bo) { continue; }
        var inside = lend.x; var outside = lend.y; var din = da; var dout = db;
        if (ao) { inside = lend.y; outside = lend.x; din = db; dout = da; }
        let den = din - dout;
        let j = tn;
        tn += 1u;
        tp[j] = lerp4(tp[inside], tp[outside], din, den);
        for (var q = 0u; q < SX_PLANES; q += 1u) {
            if (q > k) { td[j][q] = lerp2(td[inside][q], td[outside][q], din, den); }
            else { td[j][q] = td[inside][q]; }
        }
        tin[j] = inside; tout[j] = outside; tdin[j] = din; tdout[j] = dout; tden[j] = den;
        if (ao) { lend.x = j; } else { lend.y = j; }
    }
    let zn = bitcast<f32>(xw(XD_DEPTH)); let zf = bitcast<f32>(xw(XD_DEPTH + 1u));
    var sc: array<vec2<i32>, 2>;
    var zz: array<f32, 2>;
    var rr: array<f32, 2>;
    for (var c = 0u; c < 2u; c += 1u) {
        let j = lend[c];
        let made = j >= 2u;
        let p = tp[j];
        sc[c] = ps_snap(p, made);
        let w = select(p.w, 1.17549435e-38f, 1.17549435e-38f > p.w);
        var zc = p.z;
        if (made && dclip) { zc = min(max(zc, 0.0f), w); }
        let rhw = ps_div(1.0f, w);
        rr[c] = rhw;
        zz[c] = sx_r(rhw * zc * (zf - zn)) + zn;
    }
    let e = 0.00390625f;
    let x0 = sx_r(sx_i32_f32(sc[0].x) * e); let y0 = sx_r(sx_i32_f32(sc[0].y) * e);
    let x1 = sx_r(sx_i32_f32(sc[1].x) * e); let y1 = sx_r(sx_i32_f32(sc[1].y) * e);
    if (x0 == x1 && y0 == y1) { return; }
    let xm = abs(x0 - x1) >= abs(y0 - y1);
    let dx = x1 - x0; let dy = y1 - y0;
    let cx0 = l_owner(x0); let cy0 = l_owner(y0); let cx1 = l_owner(x1); let cy1 = l_owner(y1);
    let in_s = l_diamond(x0, y0, cx0, cy0, xm);
    let in_e = l_diamond(x1, y1, cx1, cy1, xm);
    let hc = bitcast<f32>(0x3f000001u);
    var hr = 0.5f;
    if (!xm) { hr = hc; }
    var sx0 = x0; var sy0 = y0; var sdx = dx; var sdy = dy;
    if (in_s && in_e && cx0 == cx1 && cy0 == cy1) {
        sx0 = -0.5f; sy0 = -0.5f; sdx = 0.0f; sdy = 0.0f;
    } else {
        if (in_s) {
            var u = 0.0f; var v = 0.0f;
            if (xm) { u = (x0 - cx0) + select(-e, e, dx > 0.0f); v = sx_r(ps_div(dy, dx) * u); }
            else { v = (y0 - cy0) + select(-e, e, dy > 0.0f); u = sx_r(ps_div(dx, dy) * v); }
            sx0 = x0 - u; sy0 = y0 - v; sdx = dx + u; sdy = dy + v;
        }
        if (in_e) {
            var u = 0.0f; var v = 0.0f;
            if (xm) { u = ((sx0 + sdx) - cx1) + select(-e, e, sdx > 0.0f); v = sx_r(ps_div(sdy, sdx) * u); }
            else { v = ((sy0 + sdy) - cy1) + select(-e, e, sdy > 0.0f); u = sx_r(ps_div(sdx, sdy) * v); }
            sdx = sdx - u; sdy = sdy - v;
        }
    }
    lx_major = xm;
    if (xm) { l_inv = ps_div(1.0f, x1 - x0); l_base = x0; } else { l_inv = ps_div(1.0f, y1 - y0); l_base = y0; }
    lrhw = vec2<f32>(rr[0], rr[1]);
    let pr = line_axis(rr[0], rr[1]);
    let pz = line_axis(zz[0], zz[1]);
    // the triangle drawn: over the line's pixels and one more on every side, its depth the
    // plane's at its corners (in the viewport's clip space, w 1)
    let vx = bitcast<f32>(xw(XD_VIEWPORT)); let vy = bitcast<f32>(xw(XD_VIEWPORT + 1u));
    let vw = bitcast<f32>(xw(XD_VIEWPORT + 2u)); let vh = bitcast<f32>(xw(XD_VIEWPORT + 3u));
    let hx = vw * 0.5f; let ox = sx_r(vw * 0.5f) + vx; let hy = -vh * 0.5f; let oy = sx_r(vh * 0.5f) + vy;
    let bx0 = floor(min(x0, x1)) - 1.0f; let by0 = floor(min(y0, y1)) - 1.0f;
    let bx1 = ceil(max(x0, x1)) + 1.0f; let by1 = ceil(max(y0, y1)) + 1.0f;
    let cs = array<vec2<f32>, 3>(vec2<f32>(bx0, by0), vec2<f32>(bx0 + sx_r(2.0f * (bx1 - bx0)), by0), vec2<f32>(bx0, by0 + sx_r(2.0f * (by1 - by0))));
    for (var c = 0u; c < 3u; c += 1u) {
        let q = cs[c];
        var dz = 0.0f;
        if (zf != zn) { dz = clamp(ps_div((sx_r(pz.x * q.x) + (sx_r(pz.y * q.y) + pz.z)) - zn, zf - zn), 0.0f, 1.0f); }
        setup[o + c] = bitcast<vec4<u32>>(vec4<f32>(ps_div(q.x - ox, hx), ps_div(q.y - oy, hy), dz, 1.0f));
    }
    setup[o + 3u] = vec4<u32>(bitcast<u32>(pr.x), bitcast<u32>(pr.y), bitcast<u32>(pr.z), bitcast<u32>(hc));
    setup[o + 4u] = vec4<u32>(bitcast<u32>(pz.x), bitcast<u32>(pz.y), bitcast<u32>(pz.z), bitcast<u32>(hr));
    setup[o + 5u] = bitcast<vec4<u32>>(vec4<f32>(sx0, sy0, sdx, sdy));
",
    );
    for &(at, pcomp, mode, src) in comps {
        wln!(m, "    setup_lc(o + {at}u, {pcomp}u, {mode}u, {}, {});", val("ea", src), val("eb", src));
    }
    m.push_str("}\n");
}

/// The WGSL of a point's setup: `point_slots(e)` drops a point outside a plane, else writes its
/// two slots: the square of the pixel whose centre is in [X - 0.5, X + 0.5) of its snapped
/// position, every plane constant (a perspective input's value times rhw).
fn point_fn(m: &mut String, clips: &[(u32, u8)], culls: &[(u32, u8)], comps: &[Comp], val: Val, pos_of: &dyn Fn(&str) -> String, regs: u32) {
    wln!(m, "fn point_slots(e0: u32) {{\n    let p = {};\n    let dclip = (xw(XD_RASTER) & 8u) != 0u;", pos_of("e0"));
    m.push_str("    for (var k = 0u; k < 6u; k += 1u) { let d = ps_dist(p, k, dclip); if (0.0f > d || d != d) { return; } }\n");
    for &(r, cc) in clips.iter().chain(culls) {
        let x = XYZW.get(usize::from(cc)).copied().unwrap_or('x');
        wln!(m, "    if (0.0f > bitcast<f32>(store[e0 * {regs}u + {r}u].{x})) {{ return; }}");
    }
    m.push_str(
        "    let s = ps_snap(p, false);
    let w = select(p.w, 1.17549435e-38f, 1.17549435e-38f > p.w);
    let rhw = ps_div(1.0f, w);
    let zn = bitcast<f32>(xw(XD_DEPTH)); let zf = bitcast<f32>(xw(XD_DEPTH + 1u));
    let z = sx_r(rhw * p.z * (zf - zn)) + zn;
    let px = f32((s.x - 1) >> 8u); let py = f32((s.y - 1) >> 8u);
    let vx = bitcast<f32>(xw(XD_VIEWPORT)); let vy = bitcast<f32>(xw(XD_VIEWPORT + 1u));
    let vw = bitcast<f32>(xw(XD_VIEWPORT + 2u)); let vh = bitcast<f32>(xw(XD_VIEWPORT + 3u));
    let hx = vw * 0.5f; let ox = sx_r(vw * 0.5f) + vx; let hy = -vh * 0.5f; let oy = sx_r(vh * 0.5f) + vy;
    var dz = 0.0f;
    if (zf != zn) { dz = clamp(ps_div(z - zn, zf - zn), 0.0f, 1.0f); }
    // (a square inside the pixel holding its centre and every standard sample position)
    let lo = vec2<f32>(px + 0.0625f, py + 0.0625f);
    let hi = vec2<f32>(px + 0.9375f, py + 0.9375f);
    let q = array<vec2<f32>, 4>(lo, vec2<f32>(hi.x, lo.y), vec2<f32>(lo.x, hi.y), hi);
    let tri = array<vec3<u32>, 2>(vec3<u32>(0u, 1u, 2u), vec3<u32>(1u, 3u, 2u));
    for (var f = 0u; f < 2u; f += 1u) {
        let o = (base + f) * SX_STRIDE;
        for (var c = 0u; c < 3u; c += 1u) {
            let v = q[tri[f][c]];
            setup[o + c] = bitcast<vec4<u32>>(vec4<f32>(ps_div(v.x - ox, hx), ps_div(v.y - oy, hy), dz, 1.0f));
        }
        setup[o + 3u] = vec4<u32>(0u, 0u, bitcast<u32>(rhw), 0u);
        setup[o + 4u] = vec4<u32>(0u, 0u, bitcast<u32>(z), 0u);
        setup[o + 5u] = vec4<u32>(0u);
",
    );
    for &(at, pcomp, mode, src) in comps {
        let v = val("e0", src);
        let c = match mode {
            0 => "0u".to_string(),
            2 => format!("bitcast<u32>(sx_r(bitcast<f32>({v}) * rhw))"),
            _ => v,
        };
        wln!(m, "        setup[o + {at}u][{pcomp}u] = 0u; setup[o + {}u][{pcomp}u] = 0u; setup[o + {}u][{pcomp}u] = {c};", at.saturating_add(1), at.saturating_add(2));
    }
    m.push_str("    }\n}\n");
}

/// The render pipeline's vertex shader of a setup store: vertex i is corner i % 3 of slot i / 3,
/// the slot to the pixel shader at location 0 (flat).
pub fn setup_vertex(stride: u32) -> Wgsl {
    let mut m = String::new();
    wln!(m, "@group(0) @binding({B_SETUP}) var<storage, read> setup: array<vec4<u32>>;");
    m.push_str("struct Out {\n    @builtin(position) pos: vec4<f32>,\n    @location(0) @interpolate(flat) slot: u32,\n}\n\n");
    wln!(m, "@vertex\nfn main(@builtin(vertex_index) vi: u32) -> Out {{\n    var out: Out;\n    out.slot = vi / 3u;\n    out.pos = bitcast<vec4<f32>>(setup[out.slot * {stride}u + vi % 3u]);\n    return out;\n}}");
    Wgsl { code: m, bindings: vec![(B_SETUP, Binding::StorageBuffer(false))] }
}

// ---------------------------------------------------------------------------------------------
// The pixel shader's side

/// The standard sample positions of a target of `samples` samples (Direct3D's, WebGPU's too),
/// in 1/16 pixel from the centre.
pub fn sample_offsets(samples: u32) -> &'static [(i32, i32)] {
    match samples {
        4 => &[(-2, -6), (6, -2), (-6, 2), (2, 6)],
        _ => &[(0, 0)],
    }
}

/// An aliased line's coverage of the pixel centred at p (lines.md §1.2: f32 each step, a
/// reciprocal of 0 leaving its test out, as its infinite t fails it).
const SX_LINE_COVERS: &str = "fn sx_line_covers(o: u32, p: vec2<f32>) -> bool {
    let l = bitcast<vec4<f32>>(setup[o + 5u]);
    let hc = bitcast<f32>(setup[o + 3u].w);
    let hr = bitcast<f32>(setup[o + 4u].w);
    let a = l.x - p.x;
    let b = l.y - p.y;
    var hit = false;
    if (l.z != 0.0f) {
        let t = a * ps_div(-1.0f, l.z);
        if (t >= 0.0f && t <= 1.0f) { let e = sx_r(t * l.w) + b; if (e > -0.5f && e < hc) { hit = true; } }
    }
    if (l.w != 0.0f) {
        let t2 = b * ps_div(-1.0f, l.w);
        if (t2 >= 0.0f && t2 <= 1.0f) { let e2 = sx_r(t2 * l.z) + a; if (e2 > -0.5f && e2 < hr) { hit = true; } }
    }
    return hit;
}
";

/// The pixel shader's entry point's part for a setup store (`wgsl::translate_setup`): the slot
/// (location 0) and position (and sample index, coverage) parameters, the setup's binding, and
/// the input registers the layout names, and SV_Position, evaluated at their places.
pub(crate) fn inputs(ps: &[u8], samples: u32, params: &mut Vec<String>, pre: &mut Vec<String>, m: &mut String, bindings: &mut Vec<(u32, Binding)>) -> Result<(), String> {
    let layout = Layout::of(ps)?;
    let stride = layout.stride();
    params.push("@builtin(position) in_pos: vec4<f32>".into());
    params.push("@location(0) @interpolate(flat) in_slot: u32".into());
    wln!(m, "@group(1) @binding({B_SETUP}) var<storage, read> setup: array<vec4<u32>>;");
    bindings.push((B_SETUP, Binding::StorageBuffer(false)));
    let (c, p) = parse(ps)?;
    let mut w = W::new(true, c.signature(false), c.signature(true));
    super::super::declarations(&mut w, &p)?;
    let pos_mode = w.inputs.values().find(|d| d.sysval == 1).map(|d| d.interp);
    let ats: Vec<At> = layout
        .inputs
        .iter()
        .filter_map(|i| match i.mode {
            Mode::Perspective(a) | Mode::NoPerspective(a) => Some(a),
            Mode::Undefined | Mode::Constant => None,
        })
        .chain(pos_mode.and_then(|i| Mode::of(i, true)).and_then(|m| match m {
            Mode::Perspective(a) | Mode::NoPerspective(a) => Some(a),
            Mode::Undefined | Mode::Constant => None,
        }))
        .collect();
    let offs = sample_offsets(samples);
    if ats.contains(&At::Sample) {
        params.push("@builtin(sample_index) in_si: u32".into());
        let list: Vec<String> = offs.iter().map(|&(x, y)| format!("vec2<f32>({}f, {}f)", f64::from(x) / 16.0, f64::from(y) / 16.0)).collect();
        wln!(m, "const SX_OFFS = array<vec2<f32>, {}>({});", offs.len(), list.join(", "));
    }
    if ats.contains(&At::Centroid) {
        params.push("@builtin(sample_mask) in_cov: u32".into());
        let list: Vec<String> = offs.iter().map(|&(x, y)| format!("vec2<f32>({}f, {}f)", f64::from(x) / 16.0, f64::from(y) / 16.0)).collect();
        wln!(m, "const SX_COFFS = array<vec2<f32>, {}>({});", offs.len(), list.join(", "));
    }
    m.push_str(SX_ROUND);
    m.push_str(ps::DIV);
    m.push_str("fn sx_at(o: u32, c: u32, p: vec2<f32>) -> f32 {\n    return (bitcast<f32>(setup[o + 2u][c]) + sx_r(bitcast<f32>(setup[o + 1u][c]) * p.y)) + sx_r(bitcast<f32>(setup[o][c]) * p.x);\n}\n");
    pre.push("sx_k = in_slot & 0x80000000u;".into());
    pre.push(format!("let sx_o = in_slot * {stride}u;"));
    pre.push("let sx_c = floor(in_pos.xy) + vec2<f32>(0.5f);".into());
    // (an aliased line's slot: the pixel lit by lines.md §1.2's test at its centre, the
    // triangle drawn only bounds it)
    m.push_str(SX_LINE_COVERS);
    pre.push("if (setup[sx_o + 3u].w != 0u && !sx_line_covers(sx_o, sx_c)) { discard; }".into());
    let point = |a: At| -> String {
        match a {
            At::Center => "sx_c".into(),
            At::Sample => "(sx_c + SX_OFFS[in_si])".into(),
            At::Centroid => "sx_cen".into(),
        }
    };
    if ats.contains(&At::Centroid) {
        // (the centre when every sample is covered, else the first covered sample's position)
        let all = (1u32 << offs.len()).wrapping_sub(1);
        pre.push(format!("var sx_cen = sx_c;\n    if ((in_cov & {all}u) != {all}u) {{ let k = firstTrailingBit(in_cov & {all}u); sx_cen = sx_c + SX_COFFS[min(k, {}u)]; }}", offs.len().saturating_sub(1)));
    }
    let rhw_at = |pt: &str| format!("ps_div(1.0f, sx_at(sx_o + 3u, 0u, {pt}))");
    for (i, inp) in (0u32..).zip(&layout.inputs) {
        let o = HEADER.saturating_add(i.saturating_mul(3));
        for c in bits(inp.mask) {
            let x = XYZW.get(usize::from(c)).copied().unwrap_or('x');
            let v = match inp.mode {
                Mode::Undefined => "0u".to_string(),
                Mode::Constant => format!("setup[sx_o + {}u].{x}", o.saturating_add(2)),
                Mode::NoPerspective(a) => format!("bitcast<u32>(sx_at(sx_o + {o}u, {c}u, {}))", point(a)),
                Mode::Perspective(a) => format!("bitcast<u32>(sx_at(sx_o + {o}u, {c}u, {p}) * {})", rhw_at(&point(a)), p = point(a)),
            };
            pre.push(format!("v{}.{x} = {v};", inp.reg));
        }
    }
    if let Some(mode) = pos_mode {
        let a = match Mode::of(mode, true) {
            Some(Mode::Perspective(a) | Mode::NoPerspective(a)) => a,
            Some(Mode::Undefined | Mode::Constant) | None => At::Center,
        };
        let pt = point(a);
        for (&r, d) in &w.inputs {
            if d.sysval != 1 {
                continue;
            }
            for c in bits(d.mask) {
                let x = XYZW.get(usize::from(c)).copied().unwrap_or('x');
                let v = match c {
                    0 => format!("bitcast<u32>({pt}.x)"),
                    1 => format!("bitcast<u32>({pt}.y)"),
                    2 => format!("bitcast<u32>(sx_at(sx_o + 4u, 0u, {pt}))"),
                    _ => format!("bitcast<u32>({})", rhw_at(&pt)),
                };
                pre.push(format!("v{r}.{x} = {v};"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "setup_golden_tests.rs"]
mod golden_tests;
