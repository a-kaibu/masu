//! The fixed-function pipeline as WGSL: a vertex shader from the vertex declaration and
//! transforms, a pixel shader from the texture stage states.
//!
//! Uniforms (same block as programmable shaders, see d3d9::uniforms):
//! vertex: x[0] position fixup, x[2] (vp x, vp y, 2/vp w, 2/vp h) for pretransformed vertices,
//! c[0..4] world*view*projection (D3D's row-major memory), c[4..12] texture matrices of
//! stages 0-1; with lighting (lighting_regs): c[12..16] material diffuse, ambient,
//! specular, emissive, c[16] the ambient render state, c[17].x the specular power, c[18..22]
//! world*view, c[22..25] the normals' matrix, and from c[25] 7 registers per enabled light.
//! pixel: x[0] alpha bits and alpha reference, c[0] texture factor, c[1..9] stage constants.

use crate::d3d9::{uniforms, USAGE_COLOR, USAGE_NORMAL, USAGE_POSITION, USAGE_POSITIONT, USAGE_TEXCOORD};
use std::fmt::Write;

/// the first light's registers, and how many each light has (see `lighting_regs`)
pub const LIGHT_BASE: u32 = 25;
pub const LIGHT_REGS: u32 = 7;
/// D3DCAPS9.MaxActiveLights (kura's caps)
pub const MAX_LIGHTS: u32 = 8;
pub const VS_CONSTS: u32 = LIGHT_BASE + LIGHT_REGS * MAX_LIGHTS;
pub const PS_CONSTS: u32 = 9;

/// What the generated vertex shader depends on.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct VsKey {
    /// (usage, index, location, kind) of each input
    pub inputs: Vec<(u32, u32, u32, u32)>,
    pub rhw: bool,
    /// per stage: texcoord index (low bits of TEXCOORDINDEX), and the texture transform
    /// (TEXTURETRANSFORMFLAGS count, 0 off) for stages 0-1
    pub tci: [(u8, u8); 8],
    /// SHADEMODE flat: the colors come from each triangle's first vertex
    pub flat: bool,
    /// LIGHTING on, vertices not pretransformed: Direct3D's lighting equation (else None)
    pub light: Option<LightKey>,
}

/// What the lighting part of the vertex shader depends on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct LightKey {
    /// enabled lights (the first `count` light register blocks)
    pub count: u8,
    /// SPECULARENABLE: the specular term is computed
    pub specular: bool,
    /// LOCALVIEWER: the half vector from the vertex to the camera (else to (0, 0, -1))
    pub local_viewer: bool,
    /// NORMALIZENORMALS
    pub normalize: bool,
    /// where diffuse, specular, ambient and emissive come from: 0 the material, 1 the vertex's
    /// diffuse color, 2 its specular color (`sources` resolves COLORVERTEX and missing colors)
    pub sources: [u8; 4],
}

/// The material color sources of the lighting equation (D3DRS_DIFFUSEMATERIALSOURCE,
/// SPECULAR-, AMBIENT-, EMISSIVEMATERIALSOURCE): D3DMCS_COLOR1 / COLOR2 take the vertex's
/// color only when COLORVERTEX is on and the vertex has that color; otherwise the material's.
pub fn sources(rs: [u32; 4], color_vertex: bool, has: [bool; 2]) -> [u8; 4] {
    rs.map(|r| match r {
        1 if color_vertex && has[0] => 1,
        2 if color_vertex && has[1] => 2,
        _ => 0,
    })
}

/// A D3DLIGHT9.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Light {
    /// 1 point, 2 spot, 3 directional
    pub ty: u32,
    pub diffuse: [f32; 4],
    pub specular: [f32; 4],
    pub ambient: [f32; 4],
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub range: f32,
    pub falloff: f32,
    pub attenuation: [f32; 3],
    pub theta: f32,
    pub phi: f32,
}

impl Light {
    /// from the 104 bytes of a D3DLIGHT9
    pub fn from_bytes(b: &[u8]) -> Light {
        let f = |i: usize| b.get(i * 4..i * 4 + 4).map_or(0.0, |c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        let v4 = |i: usize| [f(i), f(i + 1), f(i + 2), f(i + 3)];
        let v3 = |i: usize| [f(i), f(i + 1), f(i + 2)];
        Light {
            ty: f(0).to_bits(),
            diffuse: v4(1),
            specular: v4(5),
            ambient: v4(9),
            position: v3(13),
            direction: v3(16),
            range: f(19),
            falloff: f(20),
            attenuation: v3(21),
            theta: f(24),
            phi: f(25),
        }
    }
}

/// D3D's row-vector product v * M (M row-major, 16 floats)
fn row_mul(v: [f32; 4], m: &[f32; 16]) -> [f32; 4] {
    let mut r = [0f32; 4];
    for (c, x) in r.iter_mut().enumerate() {
        *x = (0..4).map(|k| v[k] * m[k * 4 + c]).sum();
    }
    r
}

fn mat_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut r = [0f32; 16];
    for i in 0..4 {
        for j in 0..4 {
            r[i * 4 + j] = (0..4).map(|k| a[i * 4 + k] * b[k * 4 + j]).sum();
        }
    }
    r
}

/// The lighting registers (c[12..], see the module's comment) from the material (D3DMATERIAL9),
/// the AMBIENT render state, the WORLD and VIEW transforms and the enabled lights (at most
/// MAX_LIGHTS are used). Lights go to camera space here, as Direct3D lights there.
pub fn lighting_regs(regs: &mut [[f32; 4]], material: &[f32; 17], ambient: u32, world: &[f32; 16], view: &[f32; 16], lights: &[Light]) {
    for i in 0..4 {
        regs[12 + i] = [material[4 * i], material[4 * i + 1], material[4 * i + 2], material[4 * i + 3]];
    }
    let ch = |s: u32| f32::from(crate::lo8(ambient >> s)) / 255.0;
    regs[16] = [ch(16), ch(8), ch(0), ch(24)];
    regs[17] = [material[16], 0.0, 0.0, 0.0];
    let wv = mat_mul(world, view);
    for r in 0..4 {
        regs[18 + r] = [wv[4 * r], wv[4 * r + 1], wv[4 * r + 2], wv[4 * r + 3]];
    }
    // normals: n * inverse(transpose(WV3)) as WGSL mat3 columns = the columns of inverse(WV3)
    let m = |i: usize, j: usize| f64::from(wv[i * 4 + j]);
    let det = m(0, 0) * (m(1, 1) * m(2, 2) - m(1, 2) * m(2, 1)) - m(0, 1) * (m(1, 0) * m(2, 2) - m(1, 2) * m(2, 0))
        + m(0, 2) * (m(1, 0) * m(2, 1) - m(1, 1) * m(2, 0));
    for c in 0..3 {
        regs[22 + c] = if det.abs() > 1e-30 {
            // column c of the inverse = cofactors of row c of WV3 / det (adjugate^T)
            let (c1, c2) = ((c + 1) % 3, (c + 2) % 3);
            let mut v = [0f32; 4];
            for r in 0..3 {
                let (r1, r2) = ((r + 1) % 3, (r + 2) % 3);
                v[r] = ((m(c1, r1) * m(c2, r2) - m(c1, r2) * m(c2, r1)) / det) as f32;
            }
            v
        } else {
            [wv[c], wv[4 + c], wv[8 + c], 0.0]
        };
    }
    for (i, l) in lights.iter().take(crate::us(MAX_LIGHTS)).enumerate() {
        let b = crate::us(LIGHT_BASE) + crate::us(LIGHT_REGS) * i;
        let p = row_mul([l.position[0], l.position[1], l.position[2], 1.0], view);
        let d = row_mul([l.direction[0], l.direction[1], l.direction[2], 0.0], view);
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let d = if len > 0.0 { [d[0] / len, d[1] / len, d[2] / len] } else { [0.0; 3] };
        regs[b] = l.diffuse;
        regs[b + 1] = l.specular;
        regs[b + 2] = l.ambient;
        regs[b + 3] = [p[0], p[1], p[2], l.range];
        regs[b + 4] = [d[0], d[1], d[2], l.falloff];
        regs[b + 5] = [l.attenuation[0], l.attenuation[1], l.attenuation[2], l.ty as f32];
        regs[b + 6] = [(l.theta / 2.0).cos(), (l.phi / 2.0).cos(), 0.0, 0.0];
    }
}

/// `fn ff_light(p, n, cd, ca, cs, ce) -> array<vec4f, 2>`: the lit diffuse and specular colors
/// of a vertex at p with normal n (camera space) and material colors cd, ca, cs, ce, by the
/// lighting equation of the Direct3D 9 docs ("Mathematics of Lighting"), as Wine's
/// fixed-function pipeline (half vector to (0, 0, -1) without LOCALVIEWER, no specular on a
/// face turned from the light).
pub fn lighting_fn(k: &LightKey) -> String {
    let mut h = String::from(
        "fn ff_light(p: vec3f, n: vec3f, cd: vec4f, ca: vec4f, cs: vec4f, ce: vec4f) -> array<vec4f, 2> {
  var amb = vec3f(0.0);
  var dif = vec3f(0.0);
  var spe = vec3f(0.0);
",
    );
    let _ = writeln!(h, "  for (var i = 0u; i < {}u; i++) {{", k.count.min(crate::lo8(MAX_LIGHTS)));
    let _ = writeln!(h, "    let b = {}u + i * {}u;", LIGHT_BASE, LIGHT_REGS);
    h.push_str(
        "    let ty = u.c[b + 5u].w;
    var l = -u.c[b + 4u].xyz;
    var att = 1.0;
    if (ty != 3.0) {
      let d = u.c[b + 3u].xyz - p;
      let dist = length(d);
      if (dist > u.c[b + 3u].w) { continue; }
      l = select(vec3f(0.0), d / dist, dist > 0.0);
      let a = u.c[b + 5u].xyz;
      let den = a.x + a.y * dist + a.z * dist * dist;
      att = select(0.0, 1.0 / den, den > 0.0);
      if (ty == 2.0) {
        let rho = dot(-u.c[b + 4u].xyz, l);
        let ct = u.c[b + 6u].x;
        let cp = u.c[b + 6u].y;
        if (rho <= cp) { att = 0.0; }
        else if (rho <= ct) { att *= pw((rho - cp) / (ct - cp), u.c[b + 4u].w); }
      }
    }
    amb += att * u.c[b + 2u].xyz;
    let nl = dot(n, l);
    dif += att * max(nl, 0.0) * u.c[b].xyz;
",
    );
    if k.specular {
        let v = if k.local_viewer { "-normalize(p)" } else { "vec3f(0.0, 0.0, -1.0)" };
        let _ = writeln!(h, "    let hv = l + {v};");
        h.push_str(
            "    let hl = dot(hv, hv);
    let nh = select(0.0, dot(n, hv * inverseSqrt(hl)), hl > 0.0);
    if (nl > 0.0 && nh > 0.0) { spe += att * pw(nh, u.c[17].x) * u.c[b + 1u].xyz; }
",
        );
    }
    h.push_str(
        "  }
  let d = ce.xyz + ca.xyz * (u.c[16].xyz + amb) + cd.xyz * dif;
  return array<vec4f, 2>(clamp(vec4f(d, cd.w), vec4f(0.0), vec4f(1.0)), clamp(vec4f(cs.xyz * spe, 0.0), vec4f(0.0), vec4f(1.0)));
}
fn pw(x: f32, y: f32) -> f32 { if (y == 0.0) { return 1.0; } return pow(max(x, 0.0), y); }
",
    );
    h
}

pub fn vertex(k: &VsKey) -> String {
    let mut h = uniforms(VS_CONSTS, 0);
    h.push_str("struct In {\n");
    for &(_, _, loc, kind) in &k.inputs {
        let ty = ["vec4f", "vec4u", "vec4i", "vec4f"].get(crate::us(kind)).copied().unwrap_or("vec4f");
        let _ = writeln!(h, "  @location({}) i{}: {},", loc, loc, ty);
    }
    if k.inputs.is_empty() {
        h.push_str("  @builtin(vertex_index) vid: u32,\n");
    }
    let f = if k.flat { "@interpolate(flat) " } else { "" };
    let _ = write!(h, "}}\nstruct Out {{\n  @builtin(position) pos: vec4f,\n  @location(0) {f}l0: vec4f,\n  @location(1) {f}l1: vec4f,\n");
    for t in 0..8 {
        let _ = writeln!(h, "  @location({}) l{}: vec4f,", 2 + t, 2 + t);
    }
    h.push_str("}\n@vertex fn main(i: In) -> Out {\n  var o: Out;\n");
    let get = |usage: u32, index: u32| -> Option<String> {
        k.inputs.iter().find(|x| x.0 == usage && x.1 == index).map(|&(_, _, loc, kind)| match kind {
            3 => format!("i.i{}.zyxw", loc),
            0 => format!("i.i{}", loc),
            _ => format!("vec4f(i.i{})", loc),
        })
    };
    let pos = get(USAGE_POSITIONT, 0).or_else(|| get(USAGE_POSITION, 0)).unwrap_or("vec4f(0.0, 0.0, 0.0, 1.0)".into());
    if k.rhw {
        let _ = writeln!(h, "  let p = {};", pos);
        h.push_str(
            "  let vp = vec4f(bitcast<f32>(u.x[2].x), bitcast<f32>(u.x[2].y), bitcast<f32>(u.x[2].z), bitcast<f32>(u.x[2].w));
  var w = 1.0;
  if (p.w != 0.0) { w = 1.0 / p.w; }
  let ndc = vec2f((p.x - vp.x) * vp.z - 1.0, 1.0 - (p.y - vp.y) * vp.w);
  o.pos = vec4f(ndc * w, p.z * w, w);
",
        );
    } else {
        let _ = writeln!(h, "  let p = vec4f(({}).xyz, 1.0);", pos);
        h.push_str("  o.pos = mat4x4f(u.c[0], u.c[1], u.c[2], u.c[3]) * p;\n");
    }
    h.push_str("  o.pos = fixpos(o.pos);\n");
    let diffuse = get(USAGE_COLOR, 0);
    let specular = get(USAGE_COLOR, 1);
    match k.light.filter(|_| !k.rhw) {
        Some(lk) => {
            let src = |s: u8, mat: u32| match s {
                1 => diffuse.clone().unwrap_or_else(|| format!("u.c[{mat}]")),
                2 => specular.clone().unwrap_or_else(|| format!("u.c[{mat}]")),
                _ => format!("u.c[{mat}]"),
            };
            let n = get(USAGE_NORMAL, 0).map_or("vec3f(0.0)".to_string(), |n| format!("({n}).xyz"));
            h.push_str("  let vp = mat4x4f(u.c[18], u.c[19], u.c[20], u.c[21]) * p;\n");
            let _ = writeln!(h, "  var vn = mat3x3f(u.c[22].xyz, u.c[23].xyz, u.c[24].xyz) * {n};");
            if lk.normalize {
                h.push_str("  let nl = dot(vn, vn);\n  if (nl > 0.0) { vn = vn * inverseSqrt(nl); }\n");
            }
            let _ = writeln!(
                h,
                "  let lit = ff_light(vp.xyz, vn, {}, {}, {}, {});\n  o.l0 = lit[0];\n  o.l1 = lit[1];",
                src(lk.sources[0], 12),
                src(lk.sources[2], 13),
                src(lk.sources[1], 14),
                src(lk.sources[3], 15)
            );
        }
        None => {
            let _ = writeln!(h, "  o.l0 = clamp({}, vec4f(0.0), vec4f(1.0));", diffuse.clone().unwrap_or("vec4f(1.0)".into()));
            let _ = writeln!(h, "  o.l1 = {};", specular.clone().unwrap_or("vec4f(0.0)".into()));
        }
    }
    for t in 0..8u8 {
        let (src, count) = k.tci[usize::from(t)];
        let tc = get(USAGE_TEXCOORD, u32::from(src)).unwrap_or("vec4f(0.0, 0.0, 0.0, 1.0)".into());
        if count > 0 && t < 2 {
            // texture transform: (u, v, 1, 0) style input through the stage's matrix
            let _ = writeln!(
                h,
                "  {{ let c = {tc}; let v = vec4f(c.xy, select(c.z, 1.0, {last}), select(c.w, 1.0, {last3})); \
                 o.l{l} = mat4x4f(u.c[{m}], u.c[{m1}], u.c[{m2}], u.c[{m3}]) * v; }}",
                tc = tc,
                last = if count <= 2 { "true" } else { "false" },
                last3 = if count <= 3 { "true" } else { "false" },
                l = 2 + t,
                m = 4 + t * 4,
                m1 = 5 + t * 4,
                m2 = 6 + t * 4,
                m3 = 7 + t * 4
            );
        } else {
            let _ = writeln!(h, "  o.l{} = {};", 2 + t, tc);
        }
    }
    h.push_str("  return o;\n}\n");
    h.push_str(FIXPOS);
    if let Some(lk) = k.light.filter(|_| !k.rhw) {
        h.push_str(&lighting_fn(&lk));
    }
    h
}

const FIXPOS: &str = "
fn fixpos(p: vec4f) -> vec4f {
  return vec4f(p.x + bitcast<f32>(u.x[0].x) * p.w, p.y - bitcast<f32>(u.x[0].y) * p.w, p.z, p.w);
}
";

/// What the generated pixel shader depends on.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct PsKey {
    /// per stage until the first disabled one: colorop, arg1, arg2, arg0, alphaop, a1, a2, a0,
    /// resultarg, projected (TTFF_PROJECTED)
    pub stages: Vec<[u32; 10]>,
    pub specular: bool,
    pub alpha_test: Option<u32>,
    /// texture types per stage (2: 2D, 1: another kind, sampled as white, 0: none bound)
    pub tex: [u8; 8],
    /// SHADEMODE flat (must match the vertex shader's)
    pub flat: bool,
    /// stages with BORDER / MIRRORONCE addressing (crate::addr)
    pub addr: Vec<crate::addr::Addr>,
    /// the locations a programmable vertex shader writes (None: the fixed-function one, all):
    /// inputs it leaves out read as a shader's would (diffuse 1, the rest 0), since WebGPU
    /// rejects a fragment input with no vertex output (F&C's C4 trial: vs_1_1, no pixel shader)
    pub vs_writes: Option<u32>,
}

const TOP_DISABLE: u32 = 1;

fn arg(a: u32, stage: usize, alpha: bool) -> String {
    let base = match a & 0xf {
        0 => "diffuse".to_string(),
        1 => "current".to_string(),
        2 => format!("t{}", stage),
        3 => "u.c[0]".to_string(),
        4 => "specular".to_string(),
        5 => "temp".to_string(),
        6 => format!("u.c[{}]", 1 + stage),
        _ => "current".to_string(),
    };
    let mut v = base;
    if a & 0x20 != 0 && !alpha {
        v = format!("vec4f({}.w)", v); // ALPHAREPLICATE
    }
    if a & 0x10 != 0 {
        v = format!("(1.0 - {})", v); // COMPLEMENT
    }
    v
}

fn op(o: u32, a1: &str, a2: &str, a0: &str, stage: usize) -> String {
    match o {
        2 => a1.into(),
        3 => a2.into(),
        4 => format!("({} * {})", a1, a2),
        5 => format!("({} * {} * 2.0)", a1, a2),
        6 => format!("({} * {} * 4.0)", a1, a2),
        7 => format!("({} + {})", a1, a2),
        8 => format!("({} + {} - 0.5)", a1, a2),
        9 => format!("(({} + {} - 0.5) * 2.0)", a1, a2),
        10 => format!("({} - {})", a1, a2),
        11 => format!("({0} + {1} - {0} * {1})", a1, a2),
        12 => format!("mix({}, {}, diffuse.w)", a2, a1),
        13 => format!("mix({}, {}, t{}.w)", a2, a1, stage),
        14 => format!("mix({}, {}, u.c[0].w)", a2, a1),
        15 => format!("({} + {} * (1.0 - t{}.w))", a1, a2, stage),
        16 => format!("mix({}, {}, current.w)", a2, a1),
        18 => format!("vec4f({0}.xyz + {0}.w * {1}.xyz, {0}.w)", a1, a2),
        19 => format!("vec4f({0}.xyz * {1}.xyz + {0}.w, {0}.w)", a1, a2),
        20 => format!("vec4f((1.0 - {0}.w) * {1}.xyz + {0}.xyz, {0}.w)", a1, a2),
        21 => format!("vec4f((1.0 - {0}.xyz) * {1}.xyz + {0}.w, {0}.w)", a1, a2),
        24 => format!("vec4f(clamp(dot({0}.xyz - 0.5, {1}.xyz - 0.5) * 4.0, 0.0, 1.0))", a1, a2),
        25 => format!("({} + {} * {})", a0, a1, a2),
        26 => format!("mix({}, {}, {})", a2, a1, a0),
        _ => a1.into(),
    }
}

/// An op that reads the stage's texture (arg1 unless SELECTARG2, arg2 unless SELECTARG1, arg0 of
/// MULTIPLYADD / LERP): with no texture bound the stage is SELECTARG1 CURRENT instead.
pub fn no_texture(o: u32, a1: u32, a2: u32, a0: u32) -> bool {
    o != TOP_DISABLE && ((a1 & 0xf == 2 && o != 3) || (a2 & 0xf == 2 && o != 2) || (a0 & 0xf == 2 && (o == 25 || o == 26)))
}

pub fn pixel(k: &PsKey) -> String {
    let mut h = uniforms(PS_CONSTS, 1);
    for t in 0..k.stages.len() {
        if k.tex.get(t).copied().unwrap_or(0) == 2 {
            let _ = writeln!(h, "@group(1) @binding({}) var tex{}: texture_2d<f32>;", t * 2, t);
            let _ = writeln!(h, "@group(1) @binding({}) var smp{}: sampler;", t * 2 + 1, t);
        }
    }
    let addr: Vec<_> = k.addr.iter().filter(|a| k.tex.get(crate::us(a.sampler)) == Some(&2) && crate::us(a.sampler) < k.stages.len()).copied().collect();
    h.push_str(&crate::addr::helpers(&addr));
    let f = if k.flat { "@interpolate(flat) " } else { "" };
    let linked = k.vs_writes.unwrap_or(0x3ff);
    h.push_str("struct In {\n  @builtin(position) fragpos: vec4f,\n");
    for l in 0..10 {
        if linked & (1 << l) != 0 {
            let _ = writeln!(h, "  @location({l}) {}l{l}: vec4f,", if l < 2 { f } else { "" });
        }
    }
    h.push_str("}\n@fragment fn main(i: In) -> @location(0) vec4f {\n");
    for l in 0..10 {
        let v = if linked & (1 << l) != 0 { format!("i.l{l}") } else if l == 0 { "vec4f(1.0)".into() } else { "vec4f(0.0)".into() };
        let _ = writeln!(h, "  let l{l} = {v};");
    }
    h.push_str("  let diffuse = l0;\n  let specular = l1;\n  var current = diffuse;\n  var temp = vec4f(0.0);\n");
    // all samples first (uniform control flow)
    for (t, s) in k.stages.iter().enumerate() {
        // (BLENDTEXTUREALPHA / BLENDTEXTUREALPHAPM read the stage's texture as an alpha op too)
        let uses = [s[1], s[2], s[3], s[5], s[6], s[7]].iter().any(|a| a & 0xf == 2) || [s[0], s[4]].iter().any(|&o| o == 13 || o == 15);
        if !uses {
            continue;
        }
        if k.tex.get(t).copied().unwrap_or(0) == 2 {
            let coord = if s[9] != 0 {
                format!("(l{0}.xy / l{0}.w)", 2 + t)
            } else {
                format!("l{}.xy", 2 + t)
            };
            let smp = format!("textureSample(tex{0}, smp{0}, {1})", t, crate::addr::coord(&addr, crate::u32_of(t), &coord));
            let _ = writeln!(h, "  var t{} = {};", t, smp);
            let _ = writeln!(h, "  if ((u.x[0].z & {}u) != 0u) {{ t{}.w = 1.0; }}", 1u32 << t, t);
            // (bits 16-31: red is the luminance, rgb = r)
            let _ = writeln!(h, "  if ((u.x[0].z & {}u) != 0u) {{ t{t} = vec4f(t{t}.xxx, t{t}.w); }}", 1u32 << (t + 16));
            if addr.iter().any(|a| a.sampler == crate::u32_of(t)) {
                let _ = writeln!(h, "  t{t} = {};", crate::addr::wrap(&addr, crate::u32_of(t), &format!("t{t}"), &coord));
            }
        } else {
            // no texture: white (as stage 0's default MODULATE then gives the diffuse color)
            let _ = writeln!(h, "  let t{} = vec4f(1.0);", t);
        }
    }
    for (t, s) in k.stages.iter().enumerate() {
        if s[0] == TOP_DISABLE {
            break;
        }
        // (an op that reads the stage's texture where none is bound passes current through, as
        // Windows: SELECTARG1 CURRENT. 正しい性奴○の使い方's stage 1 MODULATE TEXTURE DIFFUSE
        // with no texture gave the diffuse, a white screen.)
        let none = k.tex.get(t).copied().unwrap_or(0) == 0;
        let (cop, c1, c2, c0) = if none && no_texture(s[0], s[1], s[2], s[3]) { (2, 1, 1, 1) } else { (s[0], s[1], s[2], s[3]) };
        let (aop, a1, a2, a0) = if none && no_texture(s[4], s[5], s[6], s[7]) { (2, 1, 1, 1) } else { (s[4], s[5], s[6], s[7]) };
        let c = op(cop, &arg(c1, t, false), &arg(c2, t, false), &arg(c0, t, false), t);
        let a = if aop == TOP_DISABLE {
            "current.w".to_string()
        } else {
            format!("({}).w", op(aop, &arg(a1, t, true), &arg(a2, t, true), &arg(a0, t, true), t))
        };
        let dst = if s[8] & 0xf == 5 { "temp" } else { "current" };
        let _ = writeln!(h, "  {{ let c = clamp({}, vec4f(-1.0), vec4f(1.0)); let a = clamp({}, -1.0, 1.0);", c, a);
        let _ = writeln!(h, "    {} = vec4f(c.xyz, a); }}", dst);
    }
    if k.specular {
        h.push_str("  current = vec4f(current.xyz + specular.xyz, current.w);\n");
    }
    h.push_str("  let out = clamp(current, vec4f(0.0), vec4f(1.0));\n");
    if let Some(func) = k.alpha_test {
        let _ = writeln!(h, "  if (!alpha_ok(out.w, {}u)) {{ discard; }}", func);
    }
    // (a render target of fewer bits: what it keeps, x[3])
    h.push_str("  return steps(out);\n}\n");
    h.push_str(
        "
// (a render target of fewer bits, x[3]: R, G, B, A each rounded to its 2^bits - 1 steps, 2^(bits-1) - 1 signed; 0: as it is)
fn steps(c: vec4f) -> vec4f {
  let m = vec4f(u.x[3] & vec4u(0x7fffffffu));
  // (bit 31: SNORM, -1 to 1)
  let lo = select(vec4f(0.0), vec4f(-1.0), (u.x[3] & vec4u(0x80000000u)) != vec4u(0u));
  return select(c, round(clamp(c, lo, vec4f(1.0)) * m) / max(m, vec4f(1.0)), m > vec4f(0.0));
}
fn alpha_ok(a: f32, func: u32) -> bool {
  let r = bitcast<f32>(u.x[0].w);
  // (the alpha as it is, against ref / 255: Windows lets 127.5 / 255 pass LESS 128)
  let q = a;
  switch (func) {
    case 1u: { return false; }
    case 2u: { return q < r; }
    case 3u: { return q == r; }
    case 4u: { return q <= r; }
    case 5u: { return q > r; }
    case 6u: { return q != r; }
    case 7u: { return q >= r; }
    default: { return true; }
  }
}
",
    );
    h
}

#[cfg(test)]
mod tests;
