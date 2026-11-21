//! Direct3D 11's fixed function tessellator: the domain points and primitives of a patch from
//! its tess factors. A port of Microsoft's "D3D11 Fixed Function Tessellator Reference"
//! (CHWTessellator, May 7 2012, MIT license; the copy Mesa keeps in
//! src/gallium/auxiliary/tessellator), which is what hardware and WARP are held to: the same
//! fixed point (15.16) arithmetic, the same point order and the same triangles, so a domain
//! shader sees the points Windows gives it in the order it gives them. (The reference's
//! CHLSLTessellator, the HLSL compiler's preconditioning of the factors, is the hull shader's
//! own code and is not here.)
//!
//! Copyright (c) Microsoft Corporation (the reference this follows)
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy of this software
//! and associated documentation files (the "Software"), to deal in the Software without
//! restriction, including without limitation the rights to use, copy, modify, merge, publish,
//! distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
//! Software is furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in all copies or
//! substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
//! BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
//! NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//! DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use kome::num::{to_f32, trunc_i32, us};

/// dcl_tessellator_partitioning (D3D11_SB_TESSELLATOR_PARTITIONING: 1 integer, 2 pow2,
/// 3 fractional_odd, 4 fractional_even)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Partitioning {
    Integer,
    Pow2,
    FractionalOdd,
    FractionalEven,
}

impl Partitioning {
    pub fn of(v: u32) -> Option<Partitioning> {
        match v {
            1 => Some(Partitioning::Integer),
            2 => Some(Partitioning::Pow2),
            3 => Some(Partitioning::FractionalOdd),
            4 => Some(Partitioning::FractionalEven),
            _ => None,
        }
    }
}

/// dcl_tessellator_output_primitive (D3D11_SB_TESSELLATOR_OUTPUT_PRIMITIVE: 1 point, 2 line,
/// 3 triangle_cw, 4 triangle_ccw)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Output {
    Point,
    Line,
    TriangleCw,
    TriangleCcw,
}

impl Output {
    pub fn of(v: u32) -> Option<Output> {
        match v {
            1 => Some(Output::Point),
            2 => Some(Output::Line),
            3 => Some(Output::TriangleCw),
            4 => Some(Output::TriangleCcw),
            _ => None,
        }
    }

    /// Vertices a primitive of it has.
    pub fn corners(self) -> u32 {
        match self {
            Output::Point => 1,
            Output::Line => 2,
            Output::TriangleCw | Output::TriangleCcw => 3,
        }
    }
}

/// dcl_tessellator_domain (D3D11_SB_TESSELLATOR_DOMAIN: 1 isoline, 2 tri, 3 quad)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Domain {
    Isoline,
    Tri,
    Quad,
}

impl Domain {
    pub fn of(v: u32) -> Option<Domain> {
        match v {
            1 => Some(Domain::Isoline),
            2 => Some(Domain::Tri),
            3 => Some(Domain::Quad),
            _ => None,
        }
    }

    /// The tess factors it reads: edges then insides (isoline: density, detail).
    pub fn factors(self) -> usize {
        match self {
            Domain::Isoline => 2,
            Domain::Tri => 4,
            Domain::Quad => 6,
        }
    }
}

/// A patch's tess factors, as the hull shader's patch constant function wrote them (the
/// system values SV_TessFactor and SV_InsideTessFactor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Factors {
    /// SV_TessFactor[0] (line density, the V direction), [1] (line detail, U)
    Isoline { density: f32, detail: f32 },
    /// SV_TessFactor[0..3] (the U == 0, V == 0, W == 0 edges), SV_InsideTessFactor
    Tri { edges: [f32; 3], inside: f32 },
    /// SV_TessFactor[0..4] (U == 0, V == 0, U == 1, V == 1), SV_InsideTessFactor[0..2] (U, V)
    Quad { edges: [f32; 4], inside: [f32; 2] },
}

/// What the tessellator made of a patch: domain points (u, v; a tri's w is 1 - u - v), and
/// primitives as point indices (Output::corners of them each). Nothing for a culled patch.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tessellation {
    pub points: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// Tessellates one patch.
pub fn tessellate(p: Partitioning, out: Output, f: Factors) -> Tessellation {
    let mut t = Hw::new(p, out);
    match f {
        Factors::Isoline { density, detail } => t.isoline(density, detail),
        Factors::Tri { edges, inside } => t.tri(edges, inside),
        Factors::Quad { edges, inside } => t.quad(edges, inside),
    }
    t.finish()
}

// ---------------------------------------------------------------------------------------------
// D3D compliant float math (the reference's tess_isNaN, tess_flush, tess_fmin, tess_fmax)

fn is_nan(a: f32) -> bool {
    let i = a.to_bits();
    (i & 0x7f80_0000) == 0x7f80_0000 && (i & 0x007f_ffff) != 0
}

/// Denormals to signed zero.
fn flush(a: f32) -> f32 {
    let i = a.to_bits();
    if (i & 0x7fff_ffff) < 0x0080_0000 { f32::from_bits(i & 0x8000_0000) } else { a }
}

/// +0 or -0 (by its bits: the reference's == 0 on floats)
fn is_zero(a: f32) -> bool {
    a.to_bits() & 0x7fff_ffff == 0
}

fn negative_sign(a: f32) -> bool {
    a.to_bits() & 0x8000_0000 != 0
}

/// IEEE 754R min (a NaN `b` gives `a`; -0 below +0)
fn fmin(a: f32, b: f32) -> f32 {
    let (fa, fb) = (flush(a), flush(b));
    if is_nan(fb) {
        a
    } else if is_zero(fa) && is_zero(fb) {
        if negative_sign(fa) { a } else { b }
    } else if fa < fb {
        a
    } else {
        b
    }
}

/// IEEE 754R max (a NaN `b` gives `a`)
fn fmax(a: f32, b: f32) -> f32 {
    let (fa, fb) = (flush(a), flush(b));
    if is_nan(fb) {
        a
    } else if is_zero(fa) && is_zero(fb) {
        if negative_sign(fb) { a } else { b }
    } else if fa >= fb {
        a
    } else {
        b
    }
}

/// A factor that culls: not above 0 (NaN too)
fn culls(f: f32) -> bool {
    !matches!(f.partial_cmp(&0.0), Some(std::cmp::Ordering::Greater))
}

/// A factor of exactly 1 (after clamping: no NaN, no -1)
fn is_one(f: f32) -> bool {
    f.to_bits() == 1.0f32.to_bits()
}

// ---------------------------------------------------------------------------------------------
// the reference's C ints: point counts and indices

/// A count or an index of the tessellator's (the reference's `int`): a patch has at most a few
/// thousand points, so nothing here comes near 2^31; + - * << wrap as two's complement, the
/// way the reference's ints behave on every compiler it was built with. (clippy.toml allows
/// its arithmetic: it cannot panic.)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub(crate) struct N(pub(crate) i32);

impl std::ops::Add for N {
    type Output = N;
    fn add(self, o: N) -> N {
        N(self.0.wrapping_add(o.0))
    }
}

impl std::ops::Add<i32> for N {
    type Output = N;
    fn add(self, o: i32) -> N {
        N(self.0.wrapping_add(o))
    }
}

impl std::ops::Sub for N {
    type Output = N;
    fn sub(self, o: N) -> N {
        N(self.0.wrapping_sub(o.0))
    }
}

impl std::ops::Sub<i32> for N {
    type Output = N;
    fn sub(self, o: i32) -> N {
        N(self.0.wrapping_sub(o))
    }
}

impl std::ops::Mul for N {
    type Output = N;
    fn mul(self, o: N) -> N {
        N(self.0.wrapping_mul(o.0))
    }
}

impl std::ops::Mul<i32> for N {
    type Output = N;
    fn mul(self, o: i32) -> N {
        N(self.0.wrapping_mul(o))
    }
}

impl std::ops::Shl<u32> for N {
    type Output = N;
    fn shl(self, o: u32) -> N {
        N(self.0.wrapping_shl(o))
    }
}

impl std::ops::Shr<u32> for N {
    type Output = N;
    fn shr(self, o: u32) -> N {
        N(self.0.wrapping_shr(o))
    }
}

impl std::ops::AddAssign<i32> for N {
    fn add_assign(&mut self, o: i32) {
        *self = *self + o;
    }
}

impl std::ops::AddAssign for N {
    fn add_assign(&mut self, o: N) {
        *self = *self + o;
    }
}

impl std::ops::SubAssign<i32> for N {
    fn sub_assign(&mut self, o: i32) {
        *self = *self - o;
    }
}

impl std::ops::SubAssign for N {
    fn sub_assign(&mut self, o: N) {
        *self = *self - o;
    }
}

impl N {
    /// C's `/ 2` (towards zero)
    fn half(self) -> N {
        N(self.0.wrapping_div(2))
    }

    /// self, self + 1, ... below `end` (C's `for (i = self; i < end; i++)`)
    pub(crate) fn upto(self, end: N) -> impl Iterator<Item = N> {
        (self.0..end.0).map(N)
    }

    /// ... up to `end` itself
    fn through(self, end: N) -> impl Iterator<Item = N> {
        (self.0..=end.0).map(N)
    }

    /// `end`, `end - 1`, ... down to self (C's `for (i = end; i >= self; i--)`)
    fn down_from(self, end: N) -> impl Iterator<Item = N> {
        (self.0..=end.0).rev().map(N)
    }

    fn of(v: u32) -> N {
        N(i32::try_from(v).unwrap_or(i32::MAX))
    }
}

/// a[i], `or` past either end
fn at<T: Copy>(a: &[T], i: N, or: T) -> T {
    usize::try_from(i.0).ok().and_then(|i| a.get(i)).copied().unwrap_or(or)
}

/// Of a pair: the second when `second`.
fn pick<T: Copy>(pair: [T; 2], second: bool) -> T {
    let [a, b] = pair;
    if second { b } else { a }
}

// ---------------------------------------------------------------------------------------------
// fixed point: 15.16 unsigned (the reference's FXP, an unsigned int: its arithmetic wraps)

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Fxp(u32);

const FRACTION_BITS: u32 = 16;
const FRACTION_MASK: u32 = 0x0000_ffff;
const INTEGER_MASK: u32 = 0x7fff_0000;
const ONE: u32 = 1 << FRACTION_BITS;
const ONE_THIRD: u32 = 0x0000_5555;
const TWO_THIRDS: u32 = 0x0000_aaaa;
const ONE_HALF: u32 = 0x0000_8000;

/// 2^-16, the smallest positive fixed point fraction
const EPSILON: f32 = 0.000_015_258_789;
const MIN_ODD: f32 = 1.0;
const MAX_ODD: f32 = 63.0;
const MIN_EVEN: f32 = 2.0;
const MAX_EVEN: f32 = 64.0;
const MIN_ISOLINE_DENSITY: f32 = 1.0;
const MAX_ISOLINE_DENSITY: f32 = 64.0;
const MIN_ODD_PLUS_HALF_EPSILON: f32 = MIN_ODD + EPSILON / 2.0;

/// 1/n in fixed point (entry 0 unused)
const RECIPROCAL: [u32; 65] = [
    0xffff_ffff, 0x10000, 0x8000, 0x5555, 0x4000, 0x3333, 0x2aab, 0x2492, 0x2000, 0x1c72, 0x199a, 0x1746, 0x1555, 0x13b1, 0x1249, 0x1111, 0x1000, 0xf0f, 0xe39, 0xd79, 0xccd, 0xc31, 0xba3, 0xb21, 0xaab, 0xa3d, 0x9d9, 0x97b, 0x925, 0x8d4, 0x889,
    0x842, 0x800, 0x7c2, 0x788, 0x750, 0x71c, 0x6eb, 0x6bd, 0x690, 0x666, 0x63e, 0x618, 0x5f4, 0x5d1, 0x5b0, 0x591, 0x572, 0x555, 0x539, 0x51f, 0x505, 0x4ec, 0x4d5, 0x4be, 0x4a8, 0x492, 0x47e, 0x46a, 0x457, 0x444, 0x432, 0x421, 0x410, 0x400,
];

/// The reference's floatToIDotF<15, 16, unsigned>: NaN and negatives 0, 2^15 and more the
/// largest value, anything else rounded to the nearest 2^-16 (ties to even).
fn to_fixed(input: f32) -> Fxp {
    let bits = input.to_bits();
    let exponent = (bits >> 23) & 0xff;
    let negative = bits & 0x8000_0000 != 0;
    // (the largest value as a float's bits: (127 + 15) << 23)
    let max_pos = 142u32 << 23;
    if exponent == 255 && (bits & 0x007f_ffff) != 0 {
        Fxp(0)
    } else if !negative && bits >= max_pos {
        Fxp(u32::MAX)
    } else if negative || exponent < 110 {
        // (unsigned: the most negative value is 0; below 2^-17, 0)
        Fxp(0)
    } else {
        let m = (bits & 0x007f_ffff) | 0x0080_0000;
        // (the bits below 2^-16: 23 - 16 - (exponent - 127), 0..24 here or negative)
        if exponent <= 134 {
            let e = 134u32.wrapping_sub(exponent);
            let lsb = 1u32.wrapping_shl(e);
            let mask = lsb.wrapping_sub(1);
            let half = lsb >> 1;
            let r = if (m & lsb) != 0 || (m & mask) > half { m.wrapping_add(half) } else { m };
            Fxp(r.wrapping_shr(e))
        } else {
            Fxp(m.wrapping_shl(exponent.wrapping_sub(134)))
        }
    }
}

fn to_float(x: Fxp) -> f32 {
    to_f32(x.0 >> FRACTION_BITS) + to_f32(x.0 & FRACTION_MASK) / to_f32(ONE)
}

/// (int)input & 1 == 0
fn is_even(input: f32) -> bool {
    trunc_i32(input) & 1 == 0
}

fn ceil(x: Fxp) -> Fxp {
    if x.0 & FRACTION_MASK != 0 { Fxp((x.0 & INTEGER_MASK).wrapping_add(ONE)) } else { x }
}

fn floor(x: Fxp) -> Fxp {
    Fxp(x.0 & INTEGER_MASK)
}

/// The integer part of a 15.16 value (as the reference's `>> FXP_FRACTION_BITS` into an int)
fn whole(x: u32) -> N {
    N::of(x >> FRACTION_BITS)
}

/// `val` without its highest set bit among the 8 below the byte boundary it is in (the
/// reference's RemoveMSB)
fn remove_msb(val: N) -> N {
    let val = val.0;
    let mut check: i32 = if val <= 0xffff {
        if val <= 0xff { 0x80 } else { 0x8000 }
    } else if val <= 0x00ff_ffff {
        0x0080_0000
    } else {
        i32::MIN
    };
    for _ in 0..8 {
        if val & check != 0 {
            return N(val & !check);
        }
        check >>= 1;
    }
    N(0)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Parity {
    Even,
    Odd,
}

impl Parity {
    /// a factor's own parity, for integer partitioning
    fn of(f: f32) -> Parity {
        if is_even(f) { Parity::Even } else { Parity::Odd }
    }
}

/// TESS_FACTOR_CONTEXT
#[derive(Clone, Copy, Debug)]
struct Ctx {
    inv_floor: u32,
    inv_ceil: u32,
    half_fraction: u32,
    half_points: N,
    split_floor: N,
}

#[derive(Clone, Copy)]
enum Diagonals {
    InsideToOutside,
    InsideToOutsideExceptMiddle,
    Mirrored,
}

/// INDEX_PATCH_CONTEXT
#[derive(Clone, Copy, Default)]
struct Patch1 {
    inside_delta: N,
    inside_bad: N,
    inside_replacement: N,
    outside_base: N,
    outside_delta: N,
    outside_bad: N,
    outside_replacement: N,
}

/// INDEX_PATCH_CONTEXT2
#[derive(Clone, Copy, Default)]
struct Patch2 {
    invert_base: N,
    invert_end: N,
    corner_bad: N,
    corner_replacement: N,
}

/// Which index patching is on (the reference's two flags: never both)
#[derive(Clone, Copy, PartialEq, Eq)]
enum Patching {
    Off,
    One,
    Two,
}

/// CHWTessellator
struct Hw {
    original_parity: Parity,
    parity: Parity,
    partitioning: Partitioning,
    output: Output,
    points: Vec<[Fxp; 2]>,
    indices: Vec<N>,
    num_points: N,
    num_indices: N,
    patching: Patching,
    p1: Patch1,
    p2: Patch2,
}

/// One edge of a patch, or one inside direction: its parity, placement and point count (the
/// reference's per-edge arrays, together)
#[derive(Clone, Copy)]
struct Side {
    parity: Parity,
    ctx: Ctx,
    points: N,
}

struct Quad {
    /// U == 0, V == 0, U == 1, V == 1
    outside: [Side; 4],
    /// U, V
    inside: [Side; 2],
    inside_base: N,
}

struct Tri {
    outside: [Side; 3],
    inside: Side,
    inside_base: N,
}

/// What processing the factors decided.
enum Plan<T> {
    Culled,
    Minimum,
    Full(T),
}

impl Hw {
    fn new(partitioning: Partitioning, output: Output) -> Hw {
        let parity = match partitioning {
            Partitioning::FractionalOdd => Parity::Odd,
            Partitioning::Integer | Partitioning::Pow2 | Partitioning::FractionalEven => Parity::Even,
        };
        Hw { original_parity: parity, parity, partitioning, output, points: Vec::new(), indices: Vec::new(), num_points: N(0), num_indices: N(0), patching: Patching::Off, p1: Patch1::default(), p2: Patch2::default() }
    }

    fn finish(self) -> Tessellation {
        let np = us(self.num_points.0.max(0).unsigned_abs());
        let ni = us(self.num_indices.0.max(0).unsigned_abs());
        Tessellation {
            points: self.points.iter().take(np).map(|p| p.map(to_float)).collect(),
            indices: self.indices.iter().take(ni).map(|i| i.0.max(0).unsigned_abs()).collect(),
        }
    }

    fn odd(&self) -> bool {
        self.parity == Parity::Odd
    }

    fn hw_integer(&self) -> bool {
        matches!(self.partitioning, Partitioning::Integer | Partitioning::Pow2)
    }

    fn bounds(&self) -> (f32, f32) {
        match self.partitioning {
            Partitioning::Integer | Partitioning::Pow2 => (MIN_ODD, MAX_EVEN),
            Partitioning::FractionalEven => (MIN_EVEN, MAX_EVEN),
            Partitioning::FractionalOdd => (MIN_ODD, MAX_ODD),
        }
    }

    fn define_point(&mut self, u: Fxp, v: Fxp, at: N) {
        let Ok(i) = usize::try_from(at.0) else { return };
        if self.points.len() <= i {
            self.points.resize(i.saturating_add(1), [Fxp(0); 2]);
        }
        if let Some(p) = self.points.get_mut(i) {
            *p = [u, v];
        }
    }

    fn define_index(&mut self, index: N, at: N) {
        let index = self.patch_index(index);
        let Ok(i) = usize::try_from(at.0) else { return };
        if self.indices.len() <= i {
            self.indices.resize(i.saturating_add(1), N(0));
        }
        if let Some(p) = self.indices.get_mut(i) {
            *p = index;
        }
    }

    /// A clockwise triangle, stored clockwise or counterclockwise as the output asks.
    fn triangle(&mut self, i0: N, i1: N, i2: N, at: N) {
        self.define_index(i0, at);
        match self.output {
            Output::TriangleCw => {
                self.define_index(i1, at + 1);
                self.define_index(i2, at + 2);
            }
            Output::Point | Output::Line | Output::TriangleCcw => {
                self.define_index(i2, at + 1);
                self.define_index(i1, at + 2);
            }
        }
    }

    fn dump_points(&mut self) {
        for p in N(0).upto(self.num_points) {
            let at = self.num_indices;
            self.define_index(p, at);
            self.num_indices += 1;
        }
    }

    fn dump_lines(&mut self) {
        for p in N(1).upto(self.num_points) {
            let at = self.num_indices;
            self.define_index(p - 1, at);
            self.define_index(p, at + 1);
            self.num_indices += 2;
        }
    }

    fn points_for(&self, tf: Fxp) -> N {
        let half = tf.0.wrapping_add(1) / 2;
        if self.odd() { whole(ceil(Fxp(ONE_HALF.wrapping_add(half))).0.wrapping_mul(2)) } else { whole(ceil(Fxp(half)).0.wrapping_mul(2)) + 1 }
    }

    fn context(&self, tf: Fxp) -> Ctx {
        let mut half = tf.0.wrapping_add(1) / 2;
        if self.odd() || half == ONE_HALF {
            half = half.wrapping_add(ONE_HALF);
        }
        let fl = floor(Fxp(half));
        let ce = ceil(Fxp(half));
        let half_points = whole(ce.0);
        let fl_int = whole(fl.0);
        let split_floor = if ce == fl {
            half_points + 1
        } else if self.odd() {
            if fl.0 == ONE { N(0) } else { (remove_msb(fl_int - 1) << 1) + 1 }
        } else {
            (remove_msb(fl_int) << 1) + 1
        };
        let mut nfloor = whole(fl.0.wrapping_mul(2));
        let mut nceil = whole(ce.0.wrapping_mul(2));
        if self.odd() {
            nfloor -= 1;
            nceil -= 1;
        }
        Ctx { inv_floor: at(&RECIPROCAL, nfloor, u32::MAX), inv_ceil: at(&RECIPROCAL, nceil, u32::MAX), half_fraction: half.wrapping_sub(fl.0), half_points, split_floor }
    }

    fn place(&self, c: &Ctx, point: N) -> Fxp {
        let mut point = point;
        let flip = if point >= c.half_points {
            point = (c.half_points << 1) - point;
            if self.odd() {
                point -= 1;
            }
            true
        } else {
            false
        };
        if point == c.half_points {
            return Fxp(ONE_HALF);
        }
        let on_ceil = point.0.cast_unsigned();
        let on_floor = if point > c.split_floor { on_ceil.wrapping_sub(1) } else { on_ceil };
        let lf = on_floor.wrapping_mul(c.inv_floor);
        let lc = on_ceil.wrapping_mul(c.inv_ceil);
        let loc = lf.wrapping_mul(ONE.wrapping_sub(c.half_fraction)).wrapping_add(lc.wrapping_mul(c.half_fraction));
        let loc = loc.wrapping_add(ONE_HALF) >> FRACTION_BITS;
        Fxp(if flip { ONE.wrapping_sub(loc) } else { loc })
    }

    fn patch_index(&self, index: N) -> N {
        match self.patching {
            Patching::One => {
                let p = &self.p1;
                if index >= p.outside_base {
                    if index == p.outside_bad { p.outside_replacement } else { index + p.outside_delta }
                } else if index == p.inside_bad {
                    p.inside_replacement
                } else {
                    index + p.inside_delta
                }
            }
            Patching::Two => {
                let p = &self.p2;
                if index >= p.invert_base {
                    if index == p.corner_bad { p.corner_replacement } else { p.invert_end - index }
                } else if index == p.corner_bad {
                    p.corner_replacement
                } else {
                    index
                }
            }
            Patching::Off => index,
        }
    }

    /// A side of factor `tf`: its placement and point count with `parity`.
    fn side(&mut self, tf: Fxp, parity: Parity) -> Side {
        self.parity = parity;
        Side { parity, ctx: self.context(tf), points: self.points_for(tf) }
    }

    fn stitch_regular(&mut self, trapezoid: bool, diagonals: Diagonals, base: N, inside_points: N, inside_base: N, outside_base: N) {
        let mut b = base;
        let mut ip = inside_base;
        let mut op = outside_base;
        if trapezoid {
            self.triangle(op, op + 1, ip, b);
            b += 3;
            op += 1;
        }
        match diagonals {
            Diagonals::InsideToOutside => {
                for _ in N(0).upto(inside_points - 1) {
                    self.triangle(ip, op, op + 1, b);
                    self.triangle(ip, op + 1, ip + 1, b + 3);
                    b += 6;
                    ip += 1;
                    op += 1;
                }
            }
            Diagonals::InsideToOutsideExceptMiddle => {
                let mut p = N(0);
                while p < inside_points.half() - 1 {
                    self.triangle(op, op + 1, ip, b);
                    self.triangle(ip, op + 1, ip + 1, b + 3);
                    b += 6;
                    ip += 1;
                    op += 1;
                    p += 1;
                }
                self.triangle(op, ip + 1, ip, b);
                self.triangle(op, op + 1, ip + 1, b + 3);
                b += 6;
                ip += 1;
                op += 1;
                p += 2;
                while p < inside_points {
                    self.triangle(op, op + 1, ip, b);
                    self.triangle(ip, op + 1, ip + 1, b + 3);
                    b += 6;
                    ip += 1;
                    op += 1;
                    p += 1;
                }
            }
            Diagonals::Mirrored => {
                let mut p = N(0);
                while p < inside_points.half() {
                    self.triangle(op, ip + 1, ip, b);
                    self.triangle(op, op + 1, ip + 1, b + 3);
                    b += 6;
                    ip += 1;
                    op += 1;
                    p += 1;
                }
                while p < inside_points - 1 {
                    self.triangle(ip, op, op + 1, b);
                    self.triangle(ip, op + 1, ip + 1, b + 3);
                    b += 6;
                    ip += 1;
                    op += 1;
                    p += 1;
                }
            }
        }
        if trapezoid {
            self.triangle(op, op + 1, ip, b);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stitch_transition(&mut self, base: N, inside_base: N, inside_half: N, inside_parity: Parity, outside_base: N, outside_half: N, outside_parity: Parity) {
        // where vertex i ends up on the half edge at the most tessellation (ruler function
        // split order), and the loop bounds for each half tess factor
        const FINAL: [i32; 33] = [0, 32, 16, 8, 17, 4, 18, 9, 19, 2, 20, 10, 21, 5, 22, 11, 23, 1, 24, 12, 25, 6, 26, 13, 27, 3, 28, 14, 29, 7, 30, 15, 31];
        const START: [i32; 33] = [1, 1, 17, 9, 9, 5, 5, 5, 5, 3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2];
        const END: [i32; 33] = [0, 0, 17, 17, 25, 25, 25, 25, 29, 29, 29, 29, 29, 29, 29, 29, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 32];
        let ih = if inside_parity == Parity::Odd { inside_half - 1 } else { inside_half };
        let oh = if outside_parity == Parity::Odd { outside_half - 1 } else { outside_half };
        let mut b = base;
        let mut op = outside_base;
        let mut ip = inside_base;
        let i_start = N(at(&START, ih, 1).min(at(&START, oh, 1)));
        let i_end = N(at(&END, ih, 0).max(at(&END, oh, 0)));
        let fin = |i: N| N(at(&FINAL, i, i32::MAX));
        if fin(N(0)) < oh {
            self.triangle(op, op + 1, ip, b);
            b += 3;
            op += 1;
        }
        for i in i_start.through(i_end) {
            if fin(i) < ih {
                self.triangle(ip, op, ip + 1, b);
                b += 3;
                ip += 1;
            }
            if fin(i) < oh {
                self.triangle(op, op + 1, ip, b);
                b += 3;
                op += 1;
            }
        }
        if inside_parity != outside_parity || inside_parity == Parity::Odd {
            if inside_parity == outside_parity {
                // a quad in the middle
                self.triangle(ip, op, ip + 1, b);
                self.triangle(ip + 1, op, op + 1, b + 3);
                b += 6;
                ip += 1;
                op += 1;
            } else if inside_parity == Parity::Even {
                // a triangle pointing inside
                self.triangle(ip, op, op + 1, b);
                b += 3;
                op += 1;
            } else {
                // a triangle pointing outside
                self.triangle(ip, op, ip + 1, b);
                b += 3;
                ip += 1;
            }
        }
        for i in i_start.down_from(i_end) {
            if fin(i) < oh {
                self.triangle(op, op + 1, ip, b);
                b += 3;
                op += 1;
            }
            if fin(i) < ih {
                self.triangle(ip, op, ip + 1, b);
                b += 3;
                ip += 1;
            }
        }
        if fin(N(0)) < oh {
            self.triangle(op, op + 1, ip, b);
        }
    }

    // ---- quad ----

    fn quad(&mut self, edges: [f32; 4], inside: [f32; 2]) {
        match self.quad_process(edges, inside) {
            Plan::Culled => {
                self.num_points = N(0);
                self.num_indices = N(0);
            }
            Plan::Minimum => {
                self.define_point(Fxp(0), Fxp(0), N(0));
                self.define_point(Fxp(ONE), Fxp(0), N(1));
                self.define_point(Fxp(ONE), Fxp(ONE), N(2));
                self.define_point(Fxp(0), Fxp(ONE), N(3));
                self.num_points = N(4);
                match self.output {
                    Output::TriangleCw | Output::TriangleCcw => {
                        self.triangle(N(0), N(1), N(3), N(0));
                        self.triangle(N(1), N(2), N(3), N(3));
                        self.num_indices = N(6);
                    }
                    Output::Point => self.dump_points(),
                    Output::Line => self.dump_lines(),
                }
            }
            Plan::Full(q) => {
                self.quad_points(&q);
                match self.output {
                    Output::Point => self.dump_points(),
                    Output::Line => self.dump_lines(),
                    Output::TriangleCw | Output::TriangleCcw => self.quad_connectivity(&q),
                }
            }
        }
    }

    fn quad_process(&mut self, edges: [f32; 4], inside: [f32; 2]) -> Plan<Quad> {
        if edges.iter().any(|&e| culls(e)) {
            return Plan::Culled;
        }
        let (mut lower, upper) = self.bounds();
        let mut edges = edges.map(|e| fmin(upper, fmax(lower, e)));
        if self.hw_integer() {
            edges = edges.map(f32::ceil);
        }
        if self.partitioning == Partitioning::FractionalOdd && (edges.iter().any(|&e| e > MIN_ODD_PLUS_HALF_EPSILON) || inside.iter().any(|&i| i > MIN_ODD_PLUS_HALF_EPSILON)) {
            // (a picture frame)
            lower = MIN_ODD + EPSILON;
        }
        let mut inside = inside.map(|i| fmin(upper, fmax(lower, i)));
        if self.hw_integer() {
            inside = inside.map(f32::ceil);
        }
        self.num_points = N(0);
        self.num_indices = N(0);
        let integer = self.hw_integer();
        let original = self.original_parity;
        let outside_parity = edges.map(|e| if integer { Parity::of(e) } else { original });
        // (an inside factor of 1 is even to the hardware)
        let inside_parity = inside.map(|i| if !integer { original } else if is_one(i) { Parity::Even } else { Parity::of(i) });
        let outside_tf = edges.map(to_fixed);
        let inside_tf = inside.map(to_fixed);
        if (self.hw_integer() || self.odd()) && inside_tf.iter().chain(&outside_tf).all(|f| f.0 == ONE) {
            return Plan::Minimum;
        }
        let [o0, o1, o2, o3] = outside_tf;
        let [p0, p1, p2, p3] = outside_parity;
        let outside = [self.side(o0, p0), self.side(o1, p1), self.side(o2, p2), self.side(o3, p3)];
        let [i0, i1] = inside_tf;
        let [q0, q1] = inside_parity;
        let inside = [self.side(i0, q0), self.side(i1, q1)].map(|s| {
            let min = if s.parity == Parity::Odd { 4 } else { 3 };
            Side { points: s.points.max(N(min)), ..s }
        });
        for s in &outside {
            self.num_points += s.points;
        }
        self.num_points -= 4;
        let inside_base = self.num_points;
        let [u, v] = inside;
        self.num_points += (u.points - 2) * (v.points - 2);
        Plan::Full(Quad { outside, inside, inside_base })
    }

    fn quad_points(&mut self, q: &Quad) {
        let mut off = N(0);
        // the outside ring, clockwise from the top left
        for (edge, s) in q.outside.iter().enumerate() {
            let end = s.points - 1;
            for p in N(0).upto(end) {
                let qq = if edge == 1 || edge == 2 { p } else { end - p };
                self.parity = s.parity;
                let param = self.place(&s.ctx, qq);
                if edge & 1 == 1 {
                    self.define_point(param, Fxp(if edge == 3 { ONE } else { 0 }), off);
                } else {
                    self.define_point(Fxp(if edge == 2 { ONE } else { 0 }), param, off);
                }
                off += 1;
            }
        }
        // the inside rings, clockwise from (0, 1) spiralling in
        let [u, v] = q.inside;
        let rings = u.points.min(v.points) >> 1;
        for ring in N(1).upto(rings) {
            let start = ring;
            let end = [u.points - 1 - start, v.points - 1 - start];
            for edge in 0..4usize {
                // (the perpendicular direction: U on odd edges; the other one along)
                let odd = edge & 1 == 1;
                let (perp_side, along_side) = (pick(q.inside, odd), pick(q.inside, !odd));
                let perp_point = if edge < 2 { start } else { pick(end, odd) };
                self.parity = perp_side.parity;
                let perp = self.place(&perp_side.ctx, perp_point);
                self.parity = along_side.parity;
                let e1 = pick(end, !odd);
                for p in start.upto(e1) {
                    let qq = if edge == 1 || edge == 2 { p } else { e1 - (p - start) };
                    let param = self.place(&along_side.ctx, qq);
                    if odd {
                        self.define_point(param, perp, off);
                    } else {
                        self.define_point(perp, param, off);
                    }
                    off += 1;
                }
            }
        }
        // even tessellation's degenerate inner "ring": a row of points
        if u.points > v.points && v.parity == Parity::Even {
            let start = rings;
            let end = u.points - 1 - start;
            self.parity = u.parity;
            for p in start.through(end) {
                let param = self.place(&u.ctx, p);
                self.define_point(param, Fxp(ONE_HALF), off);
                off += 1;
            }
        } else if v.points >= u.points && u.parity == Parity::Even {
            let start = rings;
            let end = v.points - 1 - start;
            self.parity = v.parity;
            for p in start.down_from(end) {
                let param = self.place(&v.ctx, p);
                self.define_point(Fxp(ONE_HALF), param, off);
                off += 1;
            }
        }
    }

    fn quad_connectivity(&mut self, q: &Quad) {
        let [u, v] = q.inside;
        let rows = [(u.points + 1) >> 1, (v.points + 1) >> 1];
        let [rows_u, rows_v] = rows;
        let rings = rows_u.min(rows_v);
        let degenerate = [if v.parity == Parity::Even { rows_v - 1 } else { N(-1) }, if u.parity == Parity::Even { rows_u - 1 } else { N(-1) }];
        let mut outside = q.outside;
        let mut inside_base = q.inside_base;
        let mut outside_base = N(0);
        for ring in N(1).upto(rings) {
            let inside_edge = [u.points - ring * 2, v.points - ring * 2];
            let edge0_inside = inside_base;
            let edge0_outside = outside_base;
            for (edge, o) in outside.iter_mut().enumerate() {
                // (the inside direction this edge runs along: V on even edges)
                let along = edge & 1 == 0;
                let in_edge = pick(inside_edge, along);
                let in_side = pick(q.inside, along);
                let degen = ring == pick(degenerate, along);
                let triangles = in_edge + o.points - 2;
                let (ib, ob);
                if edge == 3 {
                    if degen {
                        let base = inside_base + 1;
                        self.p2 = Patch2 { invert_base: base, corner_bad: outside_base + o.points - 1, corner_replacement: edge0_outside, invert_end: (base << 1) - 1 };
                        ib = base;
                        ob = outside_base;
                        self.patching = Patching::Two;
                    } else {
                        let ibad = in_edge - 1;
                        let obase = ibad + 1;
                        self.p1 = Patch1 { inside_delta: inside_base, inside_bad: ibad, inside_replacement: edge0_inside, outside_base: obase, outside_delta: outside_base - obase, outside_bad: obase + o.points - 1, outside_replacement: edge0_outside };
                        ib = N(0);
                        ob = obase;
                        self.patching = Patching::One;
                    }
                } else if edge == 2 && degen {
                    self.p2 = Patch2 { invert_base: inside_base, corner_bad: N(-1), corner_replacement: N(-1), invert_end: inside_base << 1 };
                    ib = inside_base;
                    ob = outside_base;
                    self.patching = Patching::Two;
                } else {
                    ib = inside_base;
                    ob = outside_base;
                }
                let base = self.num_indices;
                if ring == N(1) {
                    self.stitch_transition(base, ib, in_side.ctx.half_points, in_side.parity, ob, o.ctx.half_points, o.parity);
                } else {
                    self.stitch_regular(true, Diagonals::Mirrored, base, in_edge, ib, ob);
                }
                self.patching = Patching::Off;
                self.num_indices += triangles * 3;
                outside_base += o.points - 1;
                if edge == 2 && degen {
                    inside_base -= in_edge - 1;
                } else {
                    inside_base += in_edge - 1;
                }
                o.points = in_edge;
            }
            if ring == N(1) {
                for (edge, o) in outside.iter_mut().enumerate() {
                    let s = pick(q.inside, edge & 1 == 1);
                    o.ctx = s.ctx;
                    o.parity = s.parity;
                }
            }
        }
        // the center: a row of quads if odd
        if u.points > v.points && v.parity == Parity::Odd {
            self.patching = Patching::Two;
            let quads = (((u.points >> 1) - (v.points >> 1)) << 1) + if u.parity == Parity::Even { 2 } else { 1 };
            let base = outside_base + quads + 2;
            self.p2 = Patch2 { invert_base: base, corner_bad: base, corner_replacement: outside_base, invert_end: base + base + quads };
            let at = self.num_indices;
            self.stitch_regular(false, Diagonals::InsideToOutside, at, quads + 1, base, outside_base + 1);
            self.patching = Patching::Off;
            self.num_indices += quads * 6;
        } else if v.points >= u.points && u.parity == Parity::Odd {
            self.patching = Patching::Two;
            let quads = (((v.points >> 1) - (u.points >> 1)) << 1) + if v.parity == Parity::Even { 2 } else { 1 };
            let base = outside_base + quads + 1;
            // (the corner case's replacement is left as it was: the reference does not set it)
            self.p2 = Patch2 { invert_base: base, corner_bad: N(-1), corner_replacement: self.p2.corner_replacement, invert_end: base + base + quads };
            let diag = if v.parity == Parity::Even { Diagonals::InsideToOutside } else { Diagonals::InsideToOutsideExceptMiddle };
            let at = self.num_indices;
            self.stitch_regular(false, diag, at, quads + 1, base, outside_base);
            self.patching = Patching::Off;
            self.num_indices += quads * 6;
        }
    }

    // ---- tri ----

    fn tri(&mut self, edges: [f32; 3], inside: f32) {
        match self.tri_process(edges, inside) {
            Plan::Culled => {
                self.num_points = N(0);
                self.num_indices = N(0);
            }
            Plan::Minimum => {
                self.define_point(Fxp(0), Fxp(ONE), N(0));
                self.define_point(Fxp(0), Fxp(0), N(1));
                self.define_point(Fxp(ONE), Fxp(0), N(2));
                self.num_points = N(3);
                match self.output {
                    Output::TriangleCw | Output::TriangleCcw => {
                        let at = self.num_indices;
                        self.triangle(N(0), N(1), N(2), at);
                        self.num_indices = N(3);
                    }
                    Output::Point => self.dump_points(),
                    Output::Line => self.dump_lines(),
                }
            }
            Plan::Full(t) => {
                self.tri_points(&t);
                match self.output {
                    Output::Point => self.dump_points(),
                    Output::Line => self.dump_lines(),
                    Output::TriangleCw | Output::TriangleCcw => self.tri_connectivity(&t),
                }
            }
        }
    }

    fn tri_process(&mut self, edges: [f32; 3], inside: f32) -> Plan<Tri> {
        if edges.iter().any(|&e| culls(e)) {
            return Plan::Culled;
        }
        let (mut lower, upper) = self.bounds();
        let mut edges = edges.map(|e| fmin(upper, fmax(lower, e)));
        if self.hw_integer() {
            edges = edges.map(f32::ceil);
        }
        // (only the edges: a tri patch has one inside factor)
        if self.partitioning == Partitioning::FractionalOdd && edges.iter().any(|&e| e > MIN_ODD_PLUS_HALF_EPSILON) {
            lower = MIN_ODD + EPSILON;
        }
        let mut inside = fmin(upper, fmax(lower, inside));
        if self.hw_integer() {
            inside = inside.ceil();
        }
        self.num_points = N(0);
        self.num_indices = N(0);
        let integer = self.hw_integer();
        let original = self.original_parity;
        let outside_parity = edges.map(|e| if integer { Parity::of(e) } else { original });
        let inside_parity = if !integer { original } else if is_one(inside) { Parity::Even } else { Parity::of(inside) };
        let outside_tf = edges.map(to_fixed);
        let inside_tf = to_fixed(inside);
        if (self.hw_integer() || self.odd()) && inside_tf.0 == ONE && outside_tf.iter().all(|f| f.0 == ONE) {
            return Plan::Minimum;
        }
        let [o0, o1, o2] = outside_tf;
        let [p0, p1, p2] = outside_parity;
        let outside = [self.side(o0, p0), self.side(o1, p1), self.side(o2, p2)];
        for s in &outside {
            self.num_points += s.points;
        }
        self.num_points -= 3;
        let inside = self.side(inside_tf, inside_parity);
        let min = if self.odd() { 4 } else { 3 };
        let inside = Side { points: inside.points.max(N(min)), ..inside };
        let inside_base = self.num_points;
        let rings = (inside.points >> 1) - 1;
        self.num_points += if self.odd() { (rings * (rings + 1) - rings) * 3 } else { rings * (rings + 1) * 3 + 1 };
        Plan::Full(Tri { outside, inside, inside_base })
    }

    fn tri_points(&mut self, t: &Tri) {
        let mut off = N(0);
        // the outside ring, clockwise from V (VW, the U == 0 edge)
        for (edge, s) in t.outside.iter().enumerate() {
            let end = s.points - 1;
            for p in N(0).upto(end) {
                let qq = if edge & 1 == 1 { p } else { end - p };
                self.parity = s.parity;
                let param = self.place(&s.ctx, qq);
                if edge == 0 {
                    self.define_point(Fxp(0), param, off);
                } else {
                    self.define_point(param, Fxp(if edge == 2 { ONE.wrapping_sub(param.0) } else { 0 }), off);
                }
                off += 1;
            }
        }
        // the inside rings, clockwise spiralling in
        self.parity = t.inside.parity;
        let rings = t.inside.points >> 1;
        for ring in N(1).upto(rings) {
            let start = ring;
            let end = t.inside.points - 1 - start;
            for edge in 0..3usize {
                let perp = self.place(&t.inside.ctx, start).0.wrapping_mul(TWO_THIRDS);
                let perp = perp.wrapping_add(ONE_HALF) >> FRACTION_BITS;
                for p in start.upto(end) {
                    let qq = if edge & 1 == 1 { p } else { end - (p - start) };
                    let param = self.place(&t.inside.ctx, qq).0;
                    let along = param.wrapping_sub(perp.wrapping_add(1) / 2);
                    match edge {
                        0 => self.define_point(Fxp(perp), Fxp(along), off),
                        1 => self.define_point(Fxp(along), Fxp(perp), off),
                        _ => self.define_point(Fxp(along), Fxp(ONE.wrapping_sub(along).wrapping_sub(perp)), off),
                    }
                    off += 1;
                }
            }
        }
        if !self.odd() {
            // the center
            self.define_point(Fxp(ONE_THIRD), Fxp(ONE_THIRD), off);
        }
    }

    fn tri_connectivity(&mut self, t: &Tri) {
        let rings = (t.inside.points + 1) >> 1;
        let mut outside = t.outside;
        let mut inside_base = t.inside_base;
        let mut outside_base = N(0);
        for ring in N(1).upto(rings) {
            let inside_edge = t.inside.points - ring * 2;
            let edge0_inside = inside_base;
            let edge0_outside = outside_base;
            for (edge, o) in outside.iter_mut().enumerate() {
                let triangles = inside_edge + o.points - 2;
                let (ib, ob) = if edge == 2 {
                    let ibad = inside_edge - 1;
                    let obase = ibad + 1;
                    self.p1 = Patch1 { inside_delta: inside_base, inside_bad: ibad, inside_replacement: edge0_inside, outside_base: obase, outside_delta: outside_base - obase, outside_bad: obase + o.points - 1, outside_replacement: edge0_outside };
                    self.patching = Patching::One;
                    (N(0), obase)
                } else {
                    (inside_base, outside_base)
                };
                let base = self.num_indices;
                if ring == N(1) {
                    self.stitch_transition(base, ib, t.inside.ctx.half_points, t.inside.parity, ob, o.ctx.half_points, o.parity);
                } else {
                    self.stitch_regular(true, Diagonals::Mirrored, base, inside_edge, ib, ob);
                }
                if edge == 2 {
                    self.patching = Patching::Off;
                }
                self.num_indices += triangles * 3;
                outside_base += o.points - 1;
                inside_base += inside_edge - 1;
                o.points = inside_edge;
            }
            if ring == N(1) {
                for o in &mut outside {
                    o.ctx = t.inside.ctx;
                    o.parity = t.inside.parity;
                }
            }
        }
        if self.odd() {
            // the center: one triangle
            let at = self.num_indices;
            self.triangle(outside_base, outside_base + 1, outside_base + 2, at);
            self.num_indices += 3;
        }
    }

    // ---- isoline ----

    fn isoline(&mut self, density: f32, detail: f32) {
        if culls(density) || culls(detail) {
            self.num_points = N(0);
            self.num_indices = N(0);
            return;
        }
        let (lower, upper) = self.bounds();
        let density = fmin(MAX_ISOLINE_DENSITY, fmax(MIN_ISOLINE_DENSITY, density));
        let mut detail = fmin(upper, fmax(lower, detail));
        self.num_points = N(0);
        self.num_indices = N(0);
        let detail_parity = if self.hw_integer() {
            detail = detail.ceil();
            Parity::of(detail)
        } else {
            self.original_parity
        };
        let fdetail = to_fixed(detail);
        let detail_side = self.side(fdetail, detail_parity);
        // (the density is integer partitioned)
        let density = density.ceil();
        let density_side = self.side(to_fixed(density), Parity::of(density));
        let per_line = detail_side.points;
        // (no line at V == 1)
        let lines = density_side.points - 1;
        self.num_points = per_line * lines;
        let mut off = N(0);
        for line in N(0).upto(lines) {
            for point in N(0).upto(per_line) {
                self.parity = density_side.parity;
                let v = self.place(&density_side.ctx, line);
                self.parity = detail_side.parity;
                let u = self.place(&detail_side.ctx, point);
                self.define_point(u, v, off);
                off += 1;
            }
        }
        let mut off = N(0);
        let mut at = N(0);
        match self.output {
            Output::Point => {
                for _ in N(0).upto(lines * per_line) {
                    self.define_index(off, at);
                    off += 1;
                    at += 1;
                }
                self.num_indices = self.num_points;
            }
            Output::Line | Output::TriangleCw | Output::TriangleCcw => {
                for _ in N(0).upto(lines) {
                    for point in N(0).upto(per_line) {
                        if point > N(0) {
                            self.define_index(off - 1, at);
                            self.define_index(off, at + 1);
                            at += 2;
                        }
                        off += 1;
                    }
                }
                self.num_indices = lines * (per_line - 1) * 2;
            }
        }
    }
}

#[cfg(test)]
#[path = "tess_tests.rs"]
mod tests;
