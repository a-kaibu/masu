//! 8-bit SNORM render targets (R8_SNORM, R8G8_SNORM, R8G8B8A8_SNORM). WebGPU draws to no SNORM
//! format, so the page draws to an SINT twin of the target (the same bytes: crates/kura d3d11
//! logic.rs copies the target's bits into it before the draw and back after) and the pixel
//! shader writes each channel's byte itself, blending it over the twin's copy (LOGIC_DST, as a
//! logic op reads it) when the blend state asks: the output merger's conversion and blend,
//! as WARP does them (winref d3d11snorm), in [`store`], [`load`] and [`blend`], and the same
//! arithmetic in WGSL ([`Snorm::output_wgsl`]).

/// D3D11_BLEND (D3D11_BLEND_SRC1_* among them).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Factor {
    Zero,
    One,
    SrcColor,
    InvSrcColor,
    SrcAlpha,
    InvSrcAlpha,
    DestAlpha,
    InvDestAlpha,
    DestColor,
    InvDestColor,
    SrcAlphaSat,
    BlendFactor,
    InvBlendFactor,
    Src1Color,
    InvSrc1Color,
    Src1Alpha,
    InvSrc1Alpha,
}

impl Factor {
    /// Of its D3D11_BLEND value (1..=19 but 12 and 13, which d3d11.h has not).
    pub fn of(v: u32) -> Option<Factor> {
        Some(match v {
            1 => Factor::Zero,
            2 => Factor::One,
            3 => Factor::SrcColor,
            4 => Factor::InvSrcColor,
            5 => Factor::SrcAlpha,
            6 => Factor::InvSrcAlpha,
            7 => Factor::DestAlpha,
            8 => Factor::InvDestAlpha,
            9 => Factor::DestColor,
            10 => Factor::InvDestColor,
            11 => Factor::SrcAlphaSat,
            14 => Factor::BlendFactor,
            15 => Factor::InvBlendFactor,
            16 => Factor::Src1Color,
            17 => Factor::InvSrc1Color,
            18 => Factor::Src1Alpha,
            19 => Factor::InvSrc1Alpha,
            _ => return None,
        })
    }

    /// Whether the factor reads the destination.
    pub fn reads_dst(self) -> bool {
        match self {
            Factor::DestAlpha | Factor::InvDestAlpha | Factor::DestColor | Factor::InvDestColor | Factor::SrcAlphaSat => true,
            Factor::Zero
            | Factor::One
            | Factor::SrcColor
            | Factor::InvSrcColor
            | Factor::SrcAlpha
            | Factor::InvSrcAlpha
            | Factor::BlendFactor
            | Factor::InvBlendFactor
            | Factor::Src1Color
            | Factor::InvSrc1Color
            | Factor::Src1Alpha
            | Factor::InvSrc1Alpha => false,
        }
    }

    /// Whether the factor reads the second source (dual-source blending).
    pub fn reads_src1(self) -> bool {
        match self {
            Factor::Src1Color | Factor::InvSrc1Color | Factor::Src1Alpha | Factor::InvSrc1Alpha => true,
            Factor::Zero
            | Factor::One
            | Factor::SrcColor
            | Factor::InvSrcColor
            | Factor::SrcAlpha
            | Factor::InvSrcAlpha
            | Factor::DestAlpha
            | Factor::InvDestAlpha
            | Factor::DestColor
            | Factor::InvDestColor
            | Factor::SrcAlphaSat
            | Factor::BlendFactor
            | Factor::InvBlendFactor => false,
        }
    }

    /// Whether the factor reads the blend state's constant (OMSetBlendState's BlendFactor).
    pub fn reads_constant(self) -> bool {
        matches!(self, Factor::BlendFactor | Factor::InvBlendFactor)
    }

    /// The factor of channel `c` (0..4; 3 alpha) from the source `s`, second source `s1`,
    /// destination `d` and constant `k` (each as [`blend`] has them).
    pub fn value(self, c: usize, s: [f32; 4], s1: [f32; 4], d: [f32; 4], k: [f32; 4]) -> f32 {
        let at = |v: [f32; 4], i: usize| v.get(i).copied().unwrap_or(0.0);
        match self {
            Factor::Zero => 0.0,
            Factor::One => 1.0,
            Factor::SrcColor => at(s, c),
            Factor::InvSrcColor => 1.0 - at(s, c),
            Factor::SrcAlpha => at(s, 3),
            Factor::InvSrcAlpha => 1.0 - at(s, 3),
            Factor::DestAlpha => at(d, 3),
            Factor::InvDestAlpha => 1.0 - at(d, 3),
            Factor::DestColor => at(d, c),
            Factor::InvDestColor => 1.0 - at(d, c),
            Factor::SrcAlphaSat if c == 3 => 1.0,
            Factor::SrcAlphaSat => at(s, 3).min(1.0 - at(d, 3)),
            Factor::BlendFactor => at(k, c),
            Factor::InvBlendFactor => 1.0 - at(k, c),
            Factor::Src1Color => at(s1, c),
            Factor::InvSrc1Color => 1.0 - at(s1, c),
            Factor::Src1Alpha => at(s1, 3),
            Factor::InvSrc1Alpha => 1.0 - at(s1, 3),
        }
    }

    /// The factor in WGSL, of channel `c` ("x".."w"; "w" alpha) over vec4<f32> `s`, `s1`, `d`,
    /// `k` ([`Factor::value`]'s arithmetic).
    pub fn wgsl(self, c: &str) -> String {
        let alpha = c == "w";
        match self {
            Factor::Zero => "0.0f".into(),
            Factor::One => "1.0f".into(),
            Factor::SrcColor => format!("sn_s.{c}"),
            Factor::InvSrcColor => format!("(1.0f - sn_s.{c})"),
            Factor::SrcAlpha => "sn_s.w".into(),
            Factor::InvSrcAlpha => "(1.0f - sn_s.w)".into(),
            Factor::DestAlpha => "sn_d.w".into(),
            Factor::InvDestAlpha => "(1.0f - sn_d.w)".into(),
            Factor::DestColor => format!("sn_d.{c}"),
            Factor::InvDestColor => format!("(1.0f - sn_d.{c})"),
            Factor::SrcAlphaSat if alpha => "1.0f".into(),
            Factor::SrcAlphaSat => "min(sn_s.w, 1.0f - sn_d.w)".into(),
            Factor::BlendFactor => format!("sn_k.{c}"),
            Factor::InvBlendFactor => format!("(1.0f - sn_k.{c})"),
            Factor::Src1Color => format!("sn_s1.{c}"),
            Factor::InvSrc1Color => format!("(1.0f - sn_s1.{c})"),
            Factor::Src1Alpha => "sn_s1.w".into(),
            Factor::InvSrc1Alpha => "(1.0f - sn_s1.w)".into(),
        }
    }
}

/// D3D11_BLEND_OP.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum BlendOp {
    Add,
    Subtract,
    RevSubtract,
    Min,
    Max,
}

impl BlendOp {
    /// Of its D3D11_BLEND_OP value (1..=5).
    pub fn of(v: u32) -> Option<BlendOp> {
        Some(match v {
            1 => BlendOp::Add,
            2 => BlendOp::Subtract,
            3 => BlendOp::RevSubtract,
            4 => BlendOp::Min,
            5 => BlendOp::Max,
            _ => return None,
        })
    }

    /// The op over the source's and destination's terms (`s` the source times its factor, `d`
    /// the destination times its factor) and, for min and max, the source and destination
    /// themselves (which take no factors).
    pub fn apply(self, s_term: f32, d_term: f32, s: f32, d: f32) -> f32 {
        match self {
            BlendOp::Add => s_term + d_term,
            BlendOp::Subtract => s_term - d_term,
            BlendOp::RevSubtract => d_term - s_term,
            BlendOp::Min => s.min(d),
            BlendOp::Max => s.max(d),
        }
    }
}

/// One equation of the blend: color's or alpha's.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct Equation {
    pub src: Factor,
    pub dst: Factor,
    pub op: BlendOp,
}

impl Equation {
    /// Whether CreateBlendState takes it: MIN and MAX ignore their factors, but WARP refuses
    /// either of them with a dual-source factor (E_INVALIDARG; winref d3d11snorm).
    pub fn takes(&self) -> bool {
        match self.op {
            BlendOp::Min | BlendOp::Max => !self.src.reads_src1() && !self.dst.reads_src1(),
            BlendOp::Add | BlendOp::Subtract | BlendOp::RevSubtract => true,
        }
    }
    fn reads_dst(&self) -> bool {
        match self.op {
            BlendOp::Min | BlendOp::Max => true,
            BlendOp::Add | BlendOp::Subtract | BlendOp::RevSubtract => self.dst != Factor::Zero || self.src.reads_dst(),
        }
    }
    fn reads(&self, f: fn(Factor) -> bool) -> bool {
        match self.op {
            BlendOp::Min | BlendOp::Max => false,
            BlendOp::Add | BlendOp::Subtract | BlendOp::RevSubtract => f(self.src) || f(self.dst),
        }
    }
}

/// A target's blend (the blend state's BlendEnable on): the color and alpha equations.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct Blend {
    pub color: Equation,
    pub alpha: Equation,
}

impl Blend {
    /// Of a D3D11_RENDER_TARGET_BLEND_DESC's SrcBlend, DestBlend, BlendOp, SrcBlendAlpha,
    /// DestBlendAlpha, BlendOpAlpha (None: a value d3d11.h has not; the runtime refuses such a
    /// state).
    pub fn of(w: [u32; 6]) -> Option<Blend> {
        let [s, d, o, sa, da, oa] = w;
        Some(Blend {
            color: Equation { src: Factor::of(s)?, dst: Factor::of(d)?, op: BlendOp::of(o)? },
            alpha: Equation { src: Factor::of(sa)?, dst: Factor::of(da)?, op: BlendOp::of(oa)? },
        })
    }
    /// Whether CreateBlendState takes both equations ([`Equation::takes`]).
    pub fn takes(&self) -> bool {
        self.color.takes() && self.alpha.takes()
    }
    /// Whether the result depends on the target's texel (the draw then reads its copy).
    pub fn reads_dst(&self) -> bool {
        self.color.reads_dst() || self.alpha.reads_dst()
    }
    /// Whether it reads the second source (SV_Target1).
    pub fn reads_src1(&self) -> bool {
        self.color.reads(Factor::reads_src1) || self.alpha.reads(Factor::reads_src1)
    }
    /// Whether it reads the blend state's constant.
    pub fn reads_constant(&self) -> bool {
        self.color.reads(Factor::reads_constant) || self.alpha.reads(Factor::reads_constant)
    }
}

/// The channel's byte of a shader's output `v`, as the output merger converts it: clamped to
/// -1..1 (NaN -1), times 127, rounded to the nearest integer (halves away from zero).
pub fn store(v: f32) -> i8 {
    // (the product in f32, its halves away from zero: winref d3d11snorm's sweeps; within
    // -127..=127 after the clamp)
    kome::num::trunc_i8((clamp_src(v) * 127.0).round())
}

/// The value of an SNORM byte: b / 127, -128 as -1 (as -127).
pub fn load(b: i8) -> f32 {
    (f32::from(b) / 127.0).max(-1.0)
}

/// A value as the output merger takes it: clamped to the format's range, NaN as -1 (WARP
/// writes NaN as -127: winref d3d11snorm).
pub fn clamp_src(v: f32) -> f32 {
    if v.is_nan() { -1.0 } else { v.clamp(-1.0, 1.0) }
}

/// What a channel holds after blend `b` of sources `s`, `s1` (the shader's SV_Target0 and 1)
/// and constant `k` over destination bytes `d`.
pub fn blend(b: &Blend, s: [f32; 4], s1: [f32; 4], d: [i8; 4], k: [f32; 4]) -> [i8; 4] {
    let (s, s1, k) = (s.map(clamp_src), s1.map(clamp_src), k.map(clamp_src));
    let d = d.map(load);
    std::array::from_fn(|c| {
        let e = if c == 3 { b.alpha } else { b.color };
        let (sv, dv) = (s.get(c).copied().unwrap_or(0.0), d.get(c).copied().unwrap_or(0.0));
        let v = e.op.apply(sv * e.src.value(c, s, s1, d, k), dv * e.dst.value(c, s, s1, d, k), sv, dv);
        store(v)
    })
}

/// The binding of the blend state's constant (a uniform vec4<f32>) in the pixel shader's group:
/// past every logic op binding (LOGIC_DST's 8 targets, LOGIC_STASH, LOGIC_MARK, LOGIC_SRC), so
/// one draw can hold an SNORM blend and a stashed logic op together.
pub const SNORM_CONSTANT: u32 = 995;
const _: () = {
    use crate::logic::{LOGIC_DST, LOGIC_MARK, LOGIC_SRC, LOGIC_STASH};
    assert!(SNORM_CONSTANT >= LOGIC_DST + 8 || SNORM_CONSTANT < LOGIC_DST);
    assert!(SNORM_CONSTANT >= LOGIC_STASH + 8 || SNORM_CONSTANT < LOGIC_STASH);
    assert!(SNORM_CONSTANT >= LOGIC_MARK + 8 || SNORM_CONSTANT < LOGIC_MARK);
    assert!(SNORM_CONSTANT >= LOGIC_SRC + 1 || SNORM_CONSTANT < LOGIC_SRC);
    // (WebGPU's default maxBindingsPerBindGroup: 1000)
    assert!(SNORM_CONSTANT < 1000);
};

/// The WGSL of the conversions and blend (module scope). A load is a table of [`load`]'s
/// values (a WGSL division is within 2.5 ULP, not exact: the blend would differ from WARP's in
/// the last bit, and the byte with it). Each product goes through `sn_r`: its bits or'd with
/// a zero the compiler cannot see (bit 30 of a value within -1..1: its exponent's top bit), so
/// it is rounded as WARP rounds it and no driver fuses it with the add after it (an FMA blends
/// 1.9 * -5/127 to -10, WARP to -9).
pub fn helpers() -> String {
    let table: Vec<String> = (i8::MIN..=i8::MAX).map(|b| format!("0x{:08x}u", load(b).to_bits())).collect();
    format!(
        "const SN_LOAD = array<u32, 256>({});\n\
fn sn_store(v: vec4<f32>) -> vec4<i32> {{\n\
\x20 let c = sn_clamp(v);\n\
\x20 let x = sn_r(c * 127.0f, bitcast<vec4<u32>>(c) & vec4<u32>(0x40000000u));\n\
\x20 let t = trunc(x);\n\
\x20 return vec4<i32>(t + select(vec4<f32>(0.0f), sign(x), abs(x - t) >= vec4<f32>(0.5f)));\n\
}}\n\
fn sn_load(b: vec4<i32>) -> vec4<f32> {{\n\
\x20 let i = (vec4<u32>(b + vec4<i32>(128))) & vec4<u32>(255u);\n\
\x20 return bitcast<vec4<f32>>(vec4<u32>(SN_LOAD[i.x], SN_LOAD[i.y], SN_LOAD[i.z], SN_LOAD[i.w]));\n\
}}\n\
fn sn_r(p: vec4<f32>, z: vec4<u32>) -> vec4<f32> {{\n\
\x20 return bitcast<vec4<f32>>(bitcast<vec4<u32>>(p) | z);\n\
}}\n\
fn sn_r1(p: f32, z: u32) -> f32 {{\n\
\x20 return bitcast<f32>(bitcast<u32>(p) | z);\n\
}}\n\
fn sn_clamp(v: vec4<f32>) -> vec4<f32> {{\n\
\x20 let nan = (bitcast<vec4<u32>>(v) & vec4<u32>(0x7fffffffu)) > vec4<u32>(0x7f800000u);\n\
\x20 return select(clamp(v, vec4<f32>(-1.0f), vec4<f32>(1.0f)), vec4<f32>(-1.0f), nan);\n\
}}\n",
        table.join(", ")
    )
}

/// The WGSL of equation `e`'s result in channel `c` over sn_s, sn_s1, sn_d, sn_k.
fn equation_wgsl(e: &Equation, c: &str) -> String {
    let (s, d) = (format!("sn_s.{c}"), format!("sn_d.{c}"));
    let (st, dt) = (format!("sn_r1({s} * {}, sn_z)", e.src.wgsl(c)), format!("sn_r1({d} * {}, sn_z)", e.dst.wgsl(c)));
    match e.op {
        BlendOp::Add => format!("{st} + {dt}"),
        BlendOp::Subtract => format!("{st} - {dt}"),
        BlendOp::RevSubtract => format!("{dt} - {st}"),
        BlendOp::Min => format!("min({s}, {d})"),
        BlendOp::Max => format!("max({s}, {d})"),
    }
}

/// The WGSL statements of a blended target's bytes as `out` (vec4<i32>), from `src` and `src1`
/// (vec4<f32> expressions: SV_Target0 and 1) over the twin's bytes `dst` (vec4<i32>; the
/// target's copy) with the constant `sn_k`; `r` names the statements' own variables.
pub fn blend_wgsl(b: &Blend, src: &str, src1: &str, dst: &str, out: &str) -> Vec<String> {
    let [x, y, z] = ["x", "y", "z"].map(|c| equation_wgsl(&b.color, c));
    vec![
        format!("let sn_s = sn_clamp({src});"),
        // (a zero the compiler cannot see: sn_r)
        "let sn_z = bitcast<u32>(sn_s.x) & 0x40000000u;".to_owned(),
        format!("let sn_s1 = sn_clamp({src1});"),
        format!("let sn_d = sn_load({dst});"),
        format!("{out} = sn_store(vec4<f32>({x}, {y}, {z}, {}));", equation_wgsl(&b.alpha, "w")),
    ]
}

#[cfg(test)]
#[path = "snorm_tests.rs"]
mod tests;
