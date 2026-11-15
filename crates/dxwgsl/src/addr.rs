//! Direct3D 9's BORDER and MIRRORONCE texture addressing, which WebGPU's samplers do not have:
//! the sampler clamps (to the edge) and the shader does the rest.
//!
//! - MIRRORONCE: mirroring at 0 maps texel i to -1 - i, that is u to -u, so a clamping sampler
//!   at |u| reads what mirror-once reads.
//! - BORDER, point filtering: the texel floor(u * W) is the texture's when 0 <= u < 1, the
//!   border color otherwise.
//! - BORDER, linear filtering: in texel space x = u * W - 0.5 the two taps are floor(x) and
//!   floor(x) + 1 with weights 1 - f and f. A clamping sampler reads the edge texel for a tap
//!   past the edge, so the result is mix(border, clamped, w) with w the weight of the taps
//!   inside: saturate(x + 1) * saturate(W - x) per axis (1 for x in [0, W - 1], f below it,
//!   1 - f above). Exact for bilinear on level 0 (both axes multiply: the four taps' weights
//!   are products).

/// How one sampler addresses: `flags` bit 0 / 1 BORDER in u / v, bit 2 / 3 MIRRORONCE in u / v,
/// bit 4 linear (magnification) filtering; `border` the border color, RGBA (f32 bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Addr {
    pub sampler: u32,
    pub flags: u32,
    pub border: [u32; 4],
}

impl Addr {
    /// Direct3D 9's: the border a D3DCOLOR (ARGB)
    pub fn argb(sampler: u32, flags: u32, c: u32) -> Addr {
        let f = |sh: u32| (f32::from(kome::num::lo8(c >> sh)) / 255.0).to_bits();
        Addr { sampler, flags, border: [f(16), f(8), f(0), f(24)] }
    }
}

/// The flags of an address mode in u and v (D3DTEXTUREADDRESS and D3D11_TEXTURE_ADDRESS_MODE
/// alike: 1 wrap, 2 mirror, 3 clamp, 4 border, 5 mirror once) and whether magnification filters
/// linearly; 0 when the sampler does it all.
pub fn flags(u: u32, v: u32, linear: bool) -> u32 {
    let f = u32::from(u == 4) | u32::from(v == 4) << 1 | u32::from(u == 5) << 2 | u32::from(v == 5) << 3;
    if f == 0 { 0 } else { f | u32::from(linear) << 4 }
}

/// An address mode as the sampler does it: BORDER and MIRRORONCE clamp.
pub fn sampler_mode(m: u32) -> u32 {
    if m == 4 || m == 5 { 3 } else { m }
}

/// am{sampler}(uv): the coordinate to sample at.
pub fn coord_fn(x: &Addr) -> String {
    let m = |bit: u32, c: &str| if x.flags & bit != 0 { format!("abs(uv.{c})") } else { format!("uv.{c}") };
    format!("fn am{}(uv: vec2f) -> vec2f {{ return vec2f({}, {}); }}\n", x.sampler, m(4, "x"), m(8, "y"))
}

/// `name`(color, uv): a sample of `tex` with the border.
pub fn border_fn(x: &Addr, tex: &str, name: &str) -> String {
    if x.flags & 3 == 0 {
        return format!("fn {name}(c: vec4f, uv: vec2f) -> vec4f {{ return c; }}\n");
    }
    let one = |bit: u32, e: String| if x.flags & bit != 0 { e } else { "1.0".into() };
    let k = if x.flags & 16 != 0 {
        format!(
            "let d = vec2f(textureDimensions({tex}));\n  let t = uv * d - 0.5;\n  let w = saturate(t + 1.0) * saturate(d - t);\n  let k = {} * {};",
            one(1, "w.x".into()),
            one(2, "w.y".into())
        )
    } else {
        let i = |c: &str| format!("f32(uv.{c} >= 0.0 && uv.{c} < 1.0)");
        format!("let k = {} * {};", one(1, i("x")), one(2, i("y")))
    };
    let b = x.border.map(|v| {
        let f = f32::from_bits(v);
        crate::f32_text(if f.is_finite() { f } else { 0.0 })
    });
    format!("fn {name}(c: vec4f, uv: vec2f) -> vec4f {{\n  {k}\n  return mix(vec4f({}), c, k);\n}}\n", b.join(", "))
}

/// Direct3D 9's functions of the samplers in `a` (texture tex{s}): am{s}, ab{s}.
pub fn helpers(a: &[Addr]) -> String {
    a.iter().map(|x| coord_fn(x) + &border_fn(x, &format!("tex{}", x.sampler), &format!("ab{}", x.sampler))).collect()
}

/// `sample` (a sample of sampler `s` at am{s}(uv)) with the border, or as it is when `s` needs
/// nothing.
pub fn wrap(a: &[Addr], s: u32, sample: &str, uv: &str) -> String {
    if a.iter().any(|x| x.sampler == s) { format!("ab{s}({sample}, {uv})") } else { sample.to_string() }
}

/// The coordinate sampler `s` samples at.
pub fn coord(a: &[Addr], s: u32, uv: &str) -> String {
    if a.iter().any(|x| x.sampler == s) { format!("am{s}({uv})") } else { uv.to_string() }
}

#[cfg(test)]
mod tests;
