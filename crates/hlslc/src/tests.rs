//! Shaders compiled here go through dxwgsl's DXBC to WGSL translation and naga's validation.

use kome::locks::Lock as _;
fn compile(src: &str, entry: &str, target: &str) -> Vec<u8> {
    let mut inc = |_: &str, _: bool| None;
    match super::compile(src, &[], &mut inc, entry, target, 0) {
        Ok(b) => b,
        Err(e) => panic!("{entry}: {e}"),
    }
}

fn wgsl(code: &[u8], pixel: bool, ps: Option<&[u8]>) -> String {
    let w = dxwgsl::translate(code, pixel, ps).unwrap_or_else(|e| panic!("translate: {e}"));
    let m = match naga::front::wgsl::parse_str(&w.code) {
        Ok(m) => m,
        Err(e) => panic!("{}\n{}", e.emit_to_string(&w.code), w.code),
    };
    if let Err(e) = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m) {
        panic!("invalid: {e:?}\n{}", w.code);
    }
    w.code
}

/// vertex + pixel shader pair through the translator
fn pair(src: &str, vs: &str, ps: &str, target: &str) -> (String, String) {
    let v = compile(src, vs, &format!("vs_{target}"));
    let p = compile(src, ps, &format!("ps_{target}"));
    let pw = wgsl(&p, true, None);
    let vw = wgsl(&v, false, Some(&p));
    (vw, pw)
}

const D3D11TEST: &str = "cbuffer C : register(b0) { float4 rot; float4 tint; };\n\
struct V { float4 pos : SV_Position; float4 col : COLOR; float2 uv : TEXCOORD; };\n\
Texture2D t : register(t0); SamplerState s : register(s0);\n\
V vs_tri(float2 p : POSITION, float4 c : COLOR) {\n\
  V o; o.pos = float4(p.x * rot.x - p.y * rot.y, p.x * rot.y + p.y * rot.x, 0.5, 1); o.col = c; o.uv = 0; return o; }\n\
float4 ps_tri(V i) : SV_Target { return i.col * tint; }\n\
V vs_quad(float2 p : POSITION, float2 uv : TEXCOORD) {\n\
  V o; o.pos = float4(p, 0.5, 1); o.col = 1; o.uv = uv; return o; }\n\
float4 ps_quad(V i) : SV_Target { return t.Sample(s, i.uv); }\n";

#[test]
fn d3d11test() {
    for t in ["4_0", "5_0", "4_0_level_9_1"] {
        let (v, p) = pair(D3D11TEST, "vs_tri", "ps_tri", t);
        assert!(v.contains("fn main") && p.contains("fn main"));
        pair(D3D11TEST, "vs_quad", "ps_quad", t);
    }
    let v = compile(D3D11TEST, "vs_tri", "vs_4_0");
    let sig = dxwgsl::signature(&v, false);
    assert_eq!(sig.iter().map(|e| (e.name.as_str(), e.reg)).collect::<Vec<_>>(), [("POSITION", 0), ("COLOR", 1)]);
    let osig = dxwgsl::signature(&v, true);
    assert_eq!(osig[0].name, "SV_Position");
    assert_eq!(osig[0].sysval, 1);
}

const GAME: &str = r#"
#define BLUR_TAPS 5
#pragma pack_matrix(row_major)
cbuffer Scene : register(b0) {
    float4x4 world_view_proj;
    float4 color_mul;
    float4 color_add;
    float2 texel;
    float time;
    int mode;
    bool use_mask;
};
cbuffer Extra { float4 offsets[BLUR_TAPS]; float weights[BLUR_TAPS]; };
Texture2D<float4> tex : register(t0);
Texture2D mask_tex : register(t1);
SamplerState samp : register(s0);
static const float3 LUMA = float3(0.299, 0.587, 0.114);
static const float gauss[3] = { 0.25, 0.5, 0.25 };

struct VS_IN { float3 pos : POSITION; float2 uv : TEXCOORD0; float4 col : COLOR0; };
struct PS_IN { float4 pos : SV_POSITION; float2 uv : TEXCOORD0; float4 col : COLOR0; };

PS_IN vs_main(VS_IN i) {
    PS_IN o;
    o.pos = mul(float4(i.pos, 1.0), world_view_proj);
    o.uv = i.uv;
    o.col = i.col;
    return o;
}

float3 grey(float3 c) { return dot(c, LUMA).xxx; }

float4 blur(float2 uv) {
    float4 acc = 0;
    [unroll] for (int k = 0; k < BLUR_TAPS; k++) {
        acc += tex.Sample(samp, uv + offsets[k].xy * texel) * weights[k];
    }
    for (int j = 0; j < 3; ++j) acc += tex.SampleLevel(samp, uv, 0) * gauss[j] * 0.01;
    return acc;
}

float4 ps_main(PS_IN i) : SV_TARGET {
    float4 c = tex.Sample(samp, i.uv) * i.col;
    if (mode == 1) {
        c.rgb = grey(c.rgb);
    } else if (mode == 2) {
        c = blur(i.uv);
    } else {
        c.rgb = lerp(c.rgb, 1 - c.rgb, saturate(sin(time) * 0.5 + 0.5));
    }
    if (use_mask) {
        float m = mask_tex.Sample(samp, i.uv).a;
        clip(m - 0.5);
        c.a *= m;
    }
    switch (mode) {
    case 3: c.rg = c.gr; break;
    case 4:
    case 5: c = pow(abs(c), 2.2); break;
    default: break;
    }
    float r = length(i.uv - 0.5);
    c.rgb *= smoothstep(0.8, 0.2, r) + atan2(i.uv.y, i.uv.x) * 0.0;
    return c * color_mul + color_add;
}
"#;

#[test]
fn game_like() {
    pair(GAME, "vs_main", "ps_main", "4_0");
    pair(GAME, "vs_main", "ps_main", "5_0");
}

/// D3D9-style code compiled for shader model 4 (backward compatibility).
const LEGACY: &str = r#"
float4x4 mvp;
sampler2D s0 : register(s0);
sampler2D s1;
float alpha;
struct VO { float4 pos : POSITION; float2 uv : TEXCOORD0; };
VO vs(float4 pos : POSITION, float2 uv : TEXCOORD0) { VO o; o.pos = mul(mvp, pos); o.uv = uv; return o; }
float4 ps(float2 uv : TEXCOORD0) : COLOR {
    float4 a = tex2D(s0, uv);
    float4 b = tex2Dlod(s1, float4(uv, 0, 0));
    return lerp(a, b, alpha);
}
"#;

#[test]
fn legacy_syntax() {
    let v = compile(LEGACY, "vs", "vs_4_0");
    let p = compile(LEGACY, "ps", "ps_4_0");
    wgsl(&p, true, None);
    // (the vertex shader's TEXCOORD0 meets the pixel shader's)
    wgsl(&v, false, Some(&p));
}

const MISC: &str = r#"
struct Light { float3 dir; float4 color; };
cbuffer L : register(b1) { Light lights[4]; int count; float4x4 bones[8]; };
Texture2DArray arr; TextureCube cube; SamplerState ss;
Texture2D<float> depth; SamplerComparisonState cmp;
Buffer<float4> buf;
void vs(in float4 p : POSITION, in uint4 idx : BLENDINDICES, in float4 w : BLENDWEIGHT, uint vid : SV_VertexID,
        out float4 o : SV_Position, out float3 n : NORMAL, out nointerpolation uint id : ID) {
    float4x4 m = bones[idx.x] * w.x + bones[idx.y] * w.y;
    o = mul(p, m);
    n = normalize(mul((float3x3)m, p.xyz));
    id = vid % 3u + (uint)(vid / 7);
    int s = (int)vid; s = s / -3 + s % 5;
    id += (uint)s;
}
float4 calc(float3 n, out float spec) {
    float4 c = 0;
    spec = 0;
    for (int i = 0; i < count; i++) {
        if (i >= 4) break;
        float d = saturate(dot(n, -lights[i].dir));
        if (d <= 0) continue;
        c += lights[i].color * d;
        spec = max(spec, pow(d, 16.0));
    }
    return c;
}
float early(float x) {
    if (x < 0) return -1;
    while (x > 1) { x *= 0.5; if (x < 0.75) return x; }
    return x * 2;
}
float4 ps(float4 pos : SV_Position, float3 n : NORMAL, nointerpolation uint id : ID, bool front : SV_IsFrontFace) : SV_Target {
    float spec;
    float4 c = calc(n, spec);
    c += arr.Sample(ss, float3(pos.xy, id)) + cube.SampleLevel(ss, n, 2);
    c.x += depth.SampleCmpLevelZero(cmp, pos.xy, 0.5);
    c += buf.Load(id) + arr.Load(int4(pos.xy, 0, 0));
    uint w, h, l; arr.GetDimensions(0, w, h, l, l);
    c.y += early(c.y) + w * h;
    c.z += asin(c.x) + acos(c.y) + atan(c.z) + fmod(c.w, 2) + exp(spec) + log10(spec + 1);
    float3x3 rot = { 1, 0, 0, 0, 1, 0, 0, 0, 1 };
    c.xyz = mul(rot, c.xyz) + determinant(rot) + (front ? 1 : 0);
    float arr2[3] = { c.x, c.y, c.z };
    arr2[id % 3] = 5;
    c.w = arr2[(id + 1) % 3] + any(c > 2) + all(c < 0);
    return c;
}
"#;

#[test]
fn misc() {
    pair(MISC, "vs", "ps", "5_0");
}

#[test]
fn errors() {
    let mut inc = |_: &str, _: bool| None;
    let e = super::compile("float4 main() : SV_Target { return undefined_x; }", &[], &mut inc, "main", "ps_4_0", 0).unwrap_err();
    assert!(e.contains("undefined_x"), "{e}");
    let e = super::compile("float4 main() : SV_Target { return 1; }", &[], &mut inc, "nope", "ps_4_0", 0).unwrap_err();
    assert!(e.contains("nope"), "{e}");
}

#[test]
fn checksum() {
    // (a blob's checksum covers everything after it: changing a byte changes it)
    let a = compile(D3D11TEST, "ps_quad", "ps_4_0");
    let mut b = a.clone();
    let n = b.len();
    b[n - 5] ^= 1;
    assert_ne!(super::sm4::checksum(&a[20..]), super::sm4::checksum(&b[20..]));
    assert_eq!(&a[4..20], &super::sm4::checksum(&a[20..]));
}

#[test]
fn print() {
    let Ok(which) = std::env::var("HLSL_PRINT") else {
        return;
    };
    let (src, vs, ps) = match which.as_str() {
        "game" => (GAME, "vs_main", "ps_main"),
        "misc" => (MISC, "vs", "ps"),
        "legacy" => (LEGACY, "vs", "ps"),
        "sm3" | "sm3ps" | "skin" | "ps3" => {
            let (e, t) = match which.as_str() {
                "sm3" => ("vs", "vs_2_0"),
                "sm3ps" => ("ps", "ps_2_0"),
                "skin" => ("vs_skin", "vs_3_0"),
                _ => ("ps3", "ps_3_0"),
            };
            eprintln!("{}", sm3_text(SM3, e, t));
            let link = if t.starts_with("ps") { 0xffff } else { 0 };
            eprintln!("{}", d3d9(SM3, e, t, link).0);
            return;
        }
        _ => (D3D11TEST, "vs_tri", "ps_quad"),
    };
    let (v, p) = pair(src, vs, ps, "5_0");
    eprintln!("{v}\n-----\n{p}");
}

// ---- shader model 2/3 ----

/// D3D9 bytecode through dxwgsl's D3D9 translation; (WGSL, assembly)
fn d3d9(src: &str, entry: &str, target: &str, link: u32) -> (String, dxwgsl::d3d9::Shader) {
    let code = compile(src, entry, target);
    let words: Vec<u32> = code.chunks(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
    let s = dxwgsl::d3d9::Shader::parse(&words).unwrap_or_else(|| panic!("{entry}: dxwgsl cannot parse"));
    let g = dxwgsl::d3d9::Gen::new(&s);
    let code = if s.pixel {
        g.pixel(&dxwgsl::d3d9::Options { link, vs_inputs: Vec::new(), alpha_test: None, fog: false, addr: Vec::new(), targets: 1, flat: false })
    } else {
        let ins = s.inputs.iter().enumerate().map(|(k, i)| (i.0, k as u32, 0)).collect();
        g.vertex(&dxwgsl::d3d9::Options { link: 0, vs_inputs: ins, alpha_test: None, fog: false, addr: Vec::new(), targets: 1, flat: false })
    };
    let m = match naga::front::wgsl::parse_str(&code) {
        Ok(m) => m,
        Err(e) => panic!("{entry}: {}\n{}", e.emit_to_string(&code), code),
    };
    if let Err(e) = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m) {
        panic!("{entry}: invalid: {e:?}\n{code}");
    }
    (code, s)
}

fn sm3_text(src: &str, entry: &str, target: &str) -> String {
    let mut inc = |_: &str, _: bool| None;
    super::assembly(src, &[], &mut inc, entry, target, 0).unwrap_or_else(|e| panic!("{entry}: {e}"))
}

const SM3: &str = r#"
float4x4 WorldViewProj;
float4 Diffuse : register(c8);
float4x3 Bones[20];
sampler2D Tex : register(s0);
sampler2D Mask;
samplerCUBE Env;
float Alpha;
bool UseMask;
float4 Weights[4];
int Count;
struct VO { float4 pos : POSITION; float2 uv : TEXCOORD0; float4 col : COLOR0; float3 n : TEXCOORD1; };
VO vs(float4 p : POSITION, float2 uv : TEXCOORD0, float4 c : COLOR0, float3 n : NORMAL) {
    VO o;
    o.pos = mul(p, WorldViewProj);
    o.uv = uv;
    o.col = saturate(c * Diffuse);
    o.n = normalize(n);
    return o;
}
VO vs_skin(float4 p : POSITION, float2 uv : TEXCOORD0, float4 idx : BLENDINDICES, float4 w : BLENDWEIGHT) {
    VO o;
    float3 pos = 0;
    for (int k = 0; k < 2; k++) pos += mul(p, Bones[(int)idx[k]]) * w[k];
    o.pos = mul(float4(pos, 1), WorldViewProj);
    o.uv = uv;
    o.col = Count > 1 ? Diffuse : 1;
    o.n = 0;
    return o;
}
float4 ps(VO i) : COLOR0 {
    float4 c = tex2D(Tex, i.uv) * i.col;
    if (UseMask) c.a *= tex2D(Mask, i.uv).r;
    float4 acc = 0;
    for (int k = 0; k < Count; k++) acc += Weights[k] * tex2D(Tex, i.uv + k * 0.01);
    clip(c.a - 0.01);
    c.rgb = lerp(c.rgb, dot(c.rgb, float3(0.299, 0.587, 0.114)), Alpha);
    c += texCUBE(Env, i.n) * 0.1;
    return c + acc * (Count > 2 ? 1 : 0) + frac(i.uv.x) + fmod(i.uv.y, 0.5) + step(0.5, i.uv.x);
}
float early(float x) {
    if (x < 0) return -1;
    while (x > 1) { x *= 0.5; if (x < 0.75) return x; }
    return x * 2;
}
float4 ps3(VO i, float2 vpos : VPOS, float face : VFACE) : COLOR0 {
    if (i.uv.x > 2) return 0;
    int a = (int)(i.uv.x * 7);
    int b = a / 3 + a % 3;
    float s, co;
    sincos(i.uv.y, s, co);
    float4 c = float4(floor(i.uv.x), ceil(i.uv.y), round(vpos.x), sqrt(abs(s)));
    c.x += b + (face > 0 ? 1 : 0) + early(co) + pow(abs(c.y), 2.2) + exp(c.z) + atan2(s, co);
    c.y += tex2Dbias(Tex, float4(i.uv, 0, 1)).x + tex2Dproj(Tex, float4(i.uv, 0, 2)).y + tex2Dgrad(Tex, i.uv, ddx(i.uv), ddy(i.uv)).z;
    c.z += any(c.xy > 1) ? 1 : (all(c.xy == 0) ? 2 : -1);
    c.w = sign(c.x) + min(a, 2) + max(c.x, c.y) + (float)(a != 2 && b >= 1) + (float)!(a < 1 || b == 4);
    return c;
}
"#;

#[test]
fn sm3() {
    let (_, v) = d3d9(SM3, "vs", "vs_2_0", 0);
    let (_, p) = d3d9(SM3, "ps", "ps_2_0", v.writes);
    assert!(p.samplers.len() == 3);
    let (_, v3) = d3d9(SM3, "vs", "vs_3_0", 0);
    d3d9(SM3, "ps", "ps_3_0", v3.writes);
    let (_, sk) = d3d9(SM3, "vs_skin", "vs_3_0", 0);
    assert!(sk.uses_rel);
    d3d9(SM3, "ps3", "ps_3_0", v3.writes);
    // constants: Diffuse stays in c8; the rest packed around it
    let t = sm3_text(SM3, "vs", "vs_2_0");
    assert!(t.contains("c8"), "{t}");
}

#[test]
fn sm3_ctab() {
    let code = compile(SM3, "ps", "ps_2_0");
    let w = |i: usize| u32::from_le_bytes(code[4 * i..4 * i + 4].try_into().unwrap());
    assert_eq!(w(0), 0xffff_0200);
    assert_eq!(w(1) & 0xffff, 0xfffe);
    assert_eq!(&code[8..12], b"CTAB");
    let t = &code[12..];
    let r32 = |o: usize| u32::from_le_bytes(t[o..o + 4].try_into().unwrap());
    let r16 = |o: usize| u16::from_le_bytes(t[o..o + 2].try_into().unwrap());
    let name = |o: usize| String::from_utf8_lossy(t[o..].split(|&b| b == 0).next().unwrap()).into_owned();
    let n = r32(12) as usize;
    let info = r32(16) as usize;
    let mut got = Vec::new();
    for k in 0..n {
        let e = info + 20 * k;
        got.push((name(r32(e) as usize), r16(e + 4), r16(e + 8)));
    }
    got.sort();
    let want: Vec<(String, u16, u16)> = vec![
        ("Alpha".into(), 2, 1),
        ("Count".into(), 2, 1),
        ("Env".into(), 3, 1),
        ("Mask".into(), 3, 1),
        ("Tex".into(), 3, 1),
        ("UseMask".into(), 2, 1),
        ("Weights".into(), 2, 4),
    ];
    assert_eq!(got, want);
    assert_eq!(name(r32(24) as usize), "ps_2_0");
}

/// Inputs the shaderconf fuzzer found: an error, not a hang or a panic.
#[test]
fn malformed_sources_are_errors() {
    for src in ["float4 A : register(c0); float4 B : register(c/*); float4 C : register(c2);\nfloat4 ps() : COLOR { return A; }\n"] {
        for t in ["ps_3_0", "ps_4_0"] {
            let mut inc = |_: &str, _: bool| None;
            let _ = super::compile(src, &[], &mut inc, "ps", t, 0);
        }
    }
    // (shaderconf --fuzz: a register list that is not one, a cbuffer at a c register, and
    // COLOR4 in shader model 3, which has four render targets: errors, not bad bytecode)
    let mut inc = |_: &str, _: bool| None;
    for (src, t) in [
        ("cbuffer C : register(buniform mat2 m2; uniform mat3 m30) { float4 A; };\nfloat4 ps() : SV_Target { return A; }\n", "ps_4_0"),
        ("cbuffer C : register(c3) { float4 A; };\nfloat4 ps() : SV_Target { return A; }\n", "ps_4_0"),
        ("cbuffer C : register(b14) { float4 A; };\nfloat4 ps() : SV_Target { return A; }\n", "ps_4_0"),
        ("float4 A : register(c0);\nfloat4 ps() : COLOR4 { return A; }\n", "ps_3_0"),
        // (a constant buffer member written through a run-time index)
        ("cbuffer C : register(b0) { float4 A; float4 B; };\nfloat4 ps() : SV_Target { B[(int)A.x] = 1; return A; }\n", "ps_4_0"),
        ("cbuffer C : register(b0) { float4 A; float4 B; };\nfloat4 ps() : SV_Target { float4 t = 0; t.yz = A.rr[(int)A.x] = 2; return t; }\n", "ps_4_0"),
        ("cbuffer C : register(b0) { float4 A; float4 B[2]; };\nfloat4 ps() : SV_Target { B[(int)A.x].y = 1; return A; }\n", "ps_4_0"),
        ("float4 A : register(c0); float4 B : register(c1);\nfloat4 ps() : COLOR { B[(int)A.x] = 1; return A; }\n", "ps_3_0"),
        // (an array without a size or an initializer, indexed: it has no elements)
        ("float4 A : register(c0);\nfloat4 ps() : COLOR { float4 a[]; a[(int)A.x] = A; return a[1]; }\n", "ps_3_0"),
        ("float4 A : register(c0);\nfloat4 ps() : SV_Target { float4 a[]; a[(int)A.x] = A; return a[1]; }\n", "ps_4_0"),
        ("float4 A : register(c0);\nfloat4 ps() : COLOR { float a[2][]; return A; }\n", "ps_3_0"),
        // (globals too: a static or a uniform array without a size)
        ("float4 A : register(c0); static float4 a[];\nfloat4 ps() : COLOR { return A + a[(int)A.x]; }\n", "ps_3_0"),
        ("float4 A : register(c0); static float4 a[];\nfloat4 ps() : SV_Target { return A + a[(int)A.x]; }\n", "ps_4_0"),
        ("float4 A : register(c0); float4 a[];\nfloat4 ps() : COLOR { return A + a[(int)A.x]; }\n", "ps_3_0"),
        ("float4 A : register(c0); float4 a[];\nfloat4 ps() : SV_Target { return A + a[(int)A.x]; }\n", "ps_4_0"),
    ] {
        assert!(super::compile(src, &[], &mut inc, "ps", t, 0).is_err(), "{src}");
    }
    for (src, t) in [
        ("cbuffer C : register(b13) { float4 A; };\nfloat4 ps() : SV_Target { return A; }\n", "ps_4_0"),
        ("cbuffer C : register(ps_5_0, b1) { float4 A; };\nfloat4 ps() : SV_Target { return A; }\n", "ps_5_0"),
        ("cbuffer C : register(b0, space0) { float4 A; };\nfloat4 ps() : SV_Target { return A; }\n", "ps_5_0"),
        ("float4 A : register(c0);\nfloat4 ps() : COLOR3 { return A; }\n", "ps_3_0"),
        // (the size from the initializer list)
        ("float4 A : register(c0);\nfloat4 ps() : COLOR { float a[] = { 1, 2, 3 }; return A * a[(int)A.x]; }\n", "ps_3_0"),
        ("float4 A : register(c0);\nfloat4 ps() : SV_Target { float a[] = { 1, 2, 3 }; return A * a[(int)A.x]; }\n", "ps_4_0"),
    ] {
        if let Err(e) = super::compile(src, &[], &mut inc, "ps", t, 0) { panic!("{src}: {e}") }
    }
    // (an array sized by its initializer list is the array of that size)
    for t in ["ps_3_0", "ps_4_0"] {
        for (a, b, pick) in [("float a[]", "float a[6]", ""), ("float2 a[]", "float2 a[3]", ".y")] {
            let out = if t == "ps_3_0" { "COLOR" } else { "SV_Target" };
            let src = |d: &str| format!("float4 A : register(c0);\nfloat4 ps() : {out} {{ {d} = {{ 1, 2, 3, 4, 5, 6 }}; return A * a[(int)A.x]{pick}; }}\n");
            let x = super::compile(&src(a), &[], &mut inc, "ps", t, 0).unwrap();
            let y = super::compile(&src(b), &[], &mut inc, "ps", t, 0).unwrap();
            assert_eq!(x, y, "{t} {a}");
        }
    }
    for asm in ["ps_3_0\nexp , c0.x\nmov oCswitch\n", "ps_3_0\nlog r0.x,.y\nmov odcl_te0.5rd0 r0\n", "ps_3_0\nmov r0, ]c[\n", "ps_3_0\nmov r0, [1]\n"] {
        let _ = super::d3d9asm::assemble(asm);
    }
}

/// ANGLE's D3D11 back end (Godot's GL compatibility renderer on Windows): arrays of textures and
/// samplers indexed by a constant that arrives through a function argument
const ANGLE_TEX_ARRAY: &str = "struct PS_INPUT { float4 dx_Position : SV_Position; float2 v0 : TEXCOORD0; };\n\
static const uint _source = 0; static const uint _second = 1;\n\
uniform Texture2D<float4> textures2D[2] : register(t0);\n\
uniform SamplerState samplers2D[2] : register(s0);\n\
float4 gl_texture2D(uint samplerIndex, float2 t) { return textures2D[samplerIndex].Sample(samplers2D[samplerIndex], float2(t.x, t.y)); }\n\
float4 main(PS_INPUT input) : SV_Target { return gl_texture2D(_source, input.v0) + gl_texture2D(_second, input.v0.yx); }\n";

#[test]
fn texture_arrays() {
    let p = compile(ANGLE_TEX_ARRAY, "main", "ps_5_0");
    let w = wgsl(&p, true, None);
    // both elements are bound: t0/s0 and t1/s1
    assert!(w.matches("textureSample").count() >= 2, "{w}");
    assert!(w.contains("t1") && w.contains("s1"), "{w}");
    let mut inc = |_: &str, _: bool| None;
    let bad = ANGLE_TEX_ARRAY.replace("gl_texture2D(_second", "gl_texture2D(5");
    let e = super::compile(&bad, &[], &mut inc, "main", "ps_5_0", 0).unwrap_err();
    assert!(e.contains("out of range"), "{e}");
}

/// ANGLE's vertex shader for GL_POINTS (its geometry shader expands the points): translated in
/// point sprite mode, it is valid WGSL that offsets by the corner and the PSIZE output
#[test]
fn point_sprite_vertex_shader() {
    let src = "struct VS_OUTPUT { float4 dx_Position : SV_Position; float gl_PointSize : PSIZE; float2 v0 : TEXCOORD0; };\n\
VS_OUTPUT main(float2 p : TEXCOORD0, float s : TEXCOORD1) { VS_OUTPUT o; o.dx_Position = float4(p, 0, 1); o.gl_PointSize = s; o.v0 = p; return o; }\n\
float4 ps(VS_OUTPUT i) : SV_Target { return float4(i.v0, 0, 1); }\n";
    let v = compile(src, "main", "vs_5_0");
    let p = compile(src, "ps", "ps_5_0");
    let w = dxwgsl::translate_point_sprite(&v, Some(&p), 1024.0).unwrap();
    let m = naga::front::wgsl::parse_str(&w.code).unwrap_or_else(|e| panic!("{}\n{}", e.emit_to_string(&w.code), w.code));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap();
    assert!(w.code.contains("sp_view") && w.code.contains("clamp(bitcast<f32>(o"), "{}", w.code);
    assert!(w.bindings.iter().any(|b| b.0 == 15), "{:?}", w.bindings);
}

/// A point mip filter (dxwgsl::addr::POINT_MIP): SampleLevel with that sampler reads mip
/// floor(lod + 0.5), the other sampler and Sample as they are; the rule against Windows' answers
/// (winref d3d11minlod's point row: lods 0 0.25 1 1.75 2.5 3 5 on 4 mips of 50 100 150 200).
#[test]
fn point_mip_levels_round_half_up() {
    const SRC: &str = "Texture2D a : register(t0); SamplerState s0 : register(s0); SamplerState s1 : register(s1);\n\
float4 ps(float4 p : SV_Position, float2 uv : TEXCOORD, float l : LOD) : SV_Target {\n\
  return a.SampleLevel(s0, uv, l) + a.SampleLevel(s1, uv, l) + a.Sample(s0, uv); }\n";
    let code = compile(SRC, "ps", "ps_5_0");
    let addr = [dxwgsl::addr::Addr { sampler: 0, flags: dxwgsl::addr::POINT_MIP, border: [0; 4] }];
    let w = dxwgsl::translate_addr(&code, true, None, &addr).unwrap();
    let m = naga::front::wgsl::parse_str(&w.code).unwrap_or_else(|e| panic!("{}\n{}", e.emit_to_string(&w.code), w.code));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{e:?}\n{}", w.code));
    let c = &w.code;
    let level = |s: u32| c.lines().filter(|l| l.contains(&format!("textureSampleLevel(t0, s{s}"))).map(|l| l.contains("+ 0.5f)")).collect::<Vec<_>>();
    assert_eq!((level(0), level(1)), (vec![true], vec![false]), "{c}");
    // (no addressing helpers for a sampler with only POINT_MIP)
    assert!(!c.contains("am0(") && !c.contains("ab0_0("), "{c}");
    assert_eq!(dxwgsl::translate_addr(&code, true, None, &[]).unwrap().code.matches("+ 0.5f)").count(), 0);
    // (the rule against the golden)
    let golden = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/winref/golden/d3d11minlod-32/stdout.txt")).unwrap_or_default();
    let row = golden.lines().find(|l| l.trim_start().starts_with("point level")).unwrap_or_default();
    let seen: Vec<f32> = row.split(',').next().unwrap_or_default().split_whitespace().skip(2).filter_map(|v| v.parse().ok()).collect();
    let model: Vec<f32> = [0.0f32, 0.25, 1.0, 1.75, 2.5, 3.0, 5.0].iter().map(|&l| 50.0 * ((l + 0.5).floor().clamp(0.0, 3.0) + 1.0)).collect();
    assert_eq!(seen, model, "{row}");
}

/// Samplers with BORDER / MIRROR_ONCE (dxwgsl::addr): 2D textures and arrays sample at am{s}
/// and mix in the border; offsets, comparisons and other samplers sample as they are.
#[test]
fn border_addressing_in_d3d11_shaders() {
    const SRC: &str = "Texture2D a : register(t0); Texture2DArray b : register(t1); Texture2D<float> z : register(t2);\n\
SamplerState s0 : register(s0); SamplerState s1 : register(s1); SamplerComparisonState c : register(s2);\n\
float4 ps(float4 p : SV_Position, float2 uv : TEXCOORD) : SV_Target {\n\
  return a.Sample(s0, uv) + b.Sample(s0, float3(uv, 1)) + a.Sample(s1, uv) + a.Sample(s0, uv, int2(1, 0))\n\
    + z.SampleCmp(c, uv, 0.5) + a.SampleLevel(s0, uv * 2, 1); }\n";
    let code = compile(SRC, "ps", "ps_5_0");
    for flags in [1, 2, 3, 4, 8, 17, 19, 21, 31] {
        let addr = [dxwgsl::addr::Addr { sampler: 0, flags, border: [0.25f32.to_bits(), 0, 1.0f32.to_bits(), 0.5f32.to_bits()] },
            dxwgsl::addr::Addr { sampler: 2, flags, border: [0; 4] }];
        let w = dxwgsl::translate_addr(&code, true, None, &addr).unwrap();
        let m = naga::front::wgsl::parse_str(&w.code).unwrap_or_else(|e| panic!("{}\n{}", e.emit_to_string(&w.code), w.code));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&m)
            .unwrap_or_else(|e| panic!("{e:?}\n{}", w.code));
        let c = &w.code;
        // t0 and t1 with s0 (three samples without offset), not s1, not the offset one, not the comparison
        assert_eq!(c.matches("am0(").count() - 1, 3, "{c}");
        assert_eq!(c.matches("ab0_0(").count() - 1, 2, "{c}");
        assert_eq!(c.matches("ab1_0(").count() - 1, 1, "{c}");
        assert!(!c.contains("am1(") && !c.contains("am2(") && !c.contains("_2("), "{c}");
        assert!(c.contains("vec4f(0.25, 0.0, 1.0, 0.5)") == (flags & 3 != 0), "{c}");
    }
    // no samplers with them: as translate
    assert_eq!(dxwgsl::translate_addr(&code, true, None, &[]).unwrap().code, dxwgsl::translate(&code, true, None).unwrap().code);
}

// ---- compute shaders ----

fn cs(src: &str, defs: &[(&str, &str)], target: &str, fmts: &[(u32, &str)]) -> String {
    let defs: Vec<(String, String)> = defs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let mut inc = |name: &str, _: bool| match name {
        "NIS_Scaler.h" => Some(NIS_SCALER.to_string()),
        _ => None,
    };
    let code = super::compile(src, &defs, &mut inc, "main", target, 0).unwrap_or_else(|e| panic!("{target} {defs:?}: {e}\n{src}"));
    let w = dxwgsl::translate_compute(&code, fmts).unwrap_or_else(|e| panic!("translate {defs:?}: {e}\n{src}"));
    let m = match naga::front::wgsl::parse_str(&w.code) {
        Ok(m) => m,
        Err(e) => panic!("{}\n{}", e.emit_to_string(&w.code), w.code),
    };
    if let Err(e) = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty()).validate(&m) {
        panic!("invalid: {e:?}\n{}", w.code);
    }
    w.code
}

fn cs_err(src: &str, target: &str) -> String {
    let mut inc = |_: &str, _: bool| None;
    match super::compile(src, &[], &mut inc, "main", target, 0) {
        Err(e) => e,
        Ok(code) => match dxwgsl::translate_compute(&code, &[]) {
            Err(e) => e,
            Ok(w) => panic!("{target}: compiled and translated\n{src}\n{}", w.code),
        },
    }
}

const NIS_MAIN: &str = include_str!("../../../tests/compute/nis/NIS_Main.hlsl");
const NIS_SCALER: &str = include_str!("../../../tests/compute/nis/NIS_Scaler.h");

/// What compute shaders use: the four thread ID semantics, typed UAVs of every dimension and
/// channel type (stores, loads, compound assignments, ++), groupshared arrays, structs and
/// matrices with every barrier, numthreads at the limits, Gather of each channel, SampleLevel
/// and Load, cs_4_0's limits.
#[test]
fn compute_shaders() {
    let ids = "RWTexture2D<uint4> u : register(u0);\n\
        [numthreads(4, 2, 1)] void main(uint3 t : SV_DispatchThreadID, uint3 g : SV_GroupID, uint3 l : SV_GroupThreadID, uint i : SV_GroupIndex) {\n\
        u[t.xy] = uint4(g.x * 100 + l.x, g.y * 100 + l.y, i, t.z); }";
    let w = cs(ids, &[], "cs_5_0", &[]);
    assert!(w.contains("@compute @workgroup_size(4, 2, 1)"), "{w}");
    assert!(w.contains("texture_storage_2d<rgba32uint, write>"), "{w}");
    for (src, fmts, want) in [
        // (1D textures are 2D ones of height 1 on the page, see render::d3d create_texture2)
        ("RWTexture1D<float4> u : register(u2); [numthreads(64,1,1)] void main(uint3 t : SV_DispatchThreadID) { u[t.x] = t.x / 64.0; }", vec![], "texture_storage_2d<rgba32float"),
        ("RWTexture2DArray<int4> u : register(u1); [numthreads(8,8,1)] void main(uint3 t : SV_DispatchThreadID) { u[t] = int4(t, 1) - 4; }", vec![], "texture_storage_2d_array<rgba32sint"),
        ("RWTexture3D<float> u : register(u0); [numthreads(4,4,4)] void main(uint3 t : SV_DispatchThreadID) { u[t] = dot(t, 1); }", vec![(0, "r32float")], "texture_storage_3d<r32float"),
        ("RWTexture2D<unorm float4> u : register(u0); [numthreads(1,1,1)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = u[t.xy].yzwx * 0.5; }", vec![(0, "rgba8unorm")], "ur0"),
        ("RWTexture2D<float4> u : register(u0); [numthreads(8,8,1)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] += float4(1, 2, 3, 4); u[t.xy] *= 2; }", vec![(0, "rgba16float")], "rgba16float"),
        ("RWTexture2D<uint> u : register(u0); [numthreads(8,8,1)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy]++; ++u[t.yx]; }", vec![(0, "r32uint")], "r32uint"),
        ("RWTexture2D<snorm float4> u : register(u3); [numthreads(32,32,1)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = -1; }", vec![(3, "rgba8snorm")], "workgroup_size(32, 32, 1)"),
        ("RWTexture2D<float4> u : register(u0); [numthreads(1024,1,1)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = 0; }", vec![], "workgroup_size(1024, 1, 1)"),
        ("RWTexture2D<float4> u : register(u0); [numthreads(1,1,64)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = t.z; }", vec![], "workgroup_size(1, 1, 64)"),
    ] {
        let w = cs(src, &[], "cs_5_0", &fmts);
        assert!(w.contains(want), "{src}: no {want}\n{w}");
    }
    // groupshared
    let shared = "struct P { float3 c; uint n; };\n\
        groupshared float4 a[64]; groupshared P ps[16]; groupshared float2x2 m; groupshared uint total;\n\
        Texture2D<float4> t : register(t0); SamplerState s : register(s0); cbuffer C : register(b0) { float4 k; uint pick; };\n\
        RWTexture2D<float4> u : register(u0);\n\
        [numthreads(8, 8, 1)] void main(uint3 id : SV_DispatchThreadID, uint i : SV_GroupIndex) {\n\
          if (i == 0) { total = 0; m = float2x2(1, 2, 3, 4); }\n\
          a[i] = t.SampleLevel(s, id.xy / 64.0, 0) + t.Load(int3(id.xy, 0)) * k;\n\
          ps[i & 15].c = a[i].xyz; ps[i & 15].n = i;\n\
          GroupMemoryBarrierWithGroupSync();\n\
          float4 sum = 0; [unroll] for (int j = 0; j < 4; j++) sum += a[(i + j * 16) & 63];\n\
          sum.xy = mul(m, sum.xy); sum.z += ps[pick & 15].c.x;\n\
          GroupMemoryBarrier(); DeviceMemoryBarrier(); DeviceMemoryBarrierWithGroupSync(); AllMemoryBarrier(); AllMemoryBarrierWithGroupSync();\n\
          u[id.xy] = sum + float4(t.GatherGreen(s, id.xy / 64.0).x, t.GatherBlue(s, id.xy / 64.0).y, t.GatherAlpha(s, id.xy / 64.0).z, t.GatherRed(s, id.xy / 64.0).w); }";
    let w = cs(shared, &[], "cs_5_0", &[(0, "rgba8unorm")]);
    assert!(w.contains("var<workgroup>") && w.contains("workgroupBarrier()"), "{w}");
    assert!(w.contains("textureGather(1,") && w.contains("textureGather(2,") && w.contains("textureGather(3,"), "{w}");
    // shader model 4 compute: 768 threads, z of 1
    cs("RWTexture2D<float4> u : register(u0); [numthreads(768,1,1)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = 1; }", &[], "cs_4_0", &[]);
}

#[test]
fn compute_errors() {
    for (src, target, want) in [
        ("RWTexture2D<float4> u : register(u0); void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = 1; }", "cs_5_0", "numthreads"),
        ("RWTexture2D<float4> u : register(u0); [numthreads(0,1,1)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = 1; }", "cs_5_0", "numthreads"),
        ("RWTexture2D<float4> u : register(u0); [numthreads(33,32,1)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = 1; }", "cs_5_0", "numthreads"),
        ("RWTexture2D<float4> u : register(u0); [numthreads(1,1,65)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = 1; }", "cs_5_0", "numthreads"),
        ("RWTexture2D<float4> u : register(u0); [numthreads(1024,1,1)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = 1; }", "cs_4_0", "numthreads"),
        ("RWTexture2D<float4> u : register(u0); [numthreads(1,1,2)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = 1; }", "cs_4_0", "numthreads"),
        ("RWTexture2D<float4> u : register(u0); groupshared float4 g[2049]; [numthreads(1,1,1)] void main(uint3 t : SV_DispatchThreadID) { g[t.x] = 1; u[t.xy] = g[0]; }", "cs_5_0", "groupshared"),
        ("RWTexture2D<float4> u : register(u0); groupshared float g = 1; [numthreads(1,1,1)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = g; }", "cs_5_0", "groupshared"),
        ("RWTexture2D<float4> u : register(t0); [numthreads(1,1,1)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = 1; }", "cs_5_0", "register"),
        ("Texture2D t : register(t0); SamplerState s : register(s0); RWTexture2D<float4> u : register(u0); [numthreads(1,1,1)] void main(uint3 i : SV_DispatchThreadID) { u[i.xy] = t.Sample(s, i.xy); }", "cs_5_0", ""),
        ("RWTexture2D<float4> u : register(u0); [numthreads(1,1,1)] void main(uint3 i : SV_DispatchThreadID) { float x = i.x; u[i.xy] = ddx(x); }", "cs_5_0", ""),
        // (shaderconf --fuzz: a Load of too few coordinates, an array as an operand)
        ("Texture2D t : register(t0); RWTexture2D<float4> u : register(u0); [numthreads(1,1,1)] void main(uint3 i : SV_DispatchThreadID) { u[i.xy] = t.Load((i.x, i.y)); }", "cs_5_0", "coordinates"),
        ("RWTexture2D<uint> u : register(u0); groupshared uint a[4]; [numthreads(4,1,1)] void main(uint i : SV_GroupIndex) { a[i] = i; u[uint2(i, 0)] = a[i] + a; }", "cs_5_0", "types"),
        ("RWTexture2D<uint> u : register(u0); [numthreads(4,1,1)] void main(uint i : SV_GroupIndex) { uint a[2] = { 1, 2 }; u[uint2(i, 0)] = 1 + a; }", "cs_5_0", "types"),
        // (shaderconf --fuzz: a swizzle of a groupshared array recursed until the stack ran out)
        ("RWTexture2D<uint> u : register(u0); groupshared uint a[4]; [numthreads(4,1,1)] void main(uint i : SV_GroupIndex) { a[i] = i; u[uint2(i, 0)] = a.wzyx[i]; }", "cs_5_0", "groupshared array"),
        ("RWTexture2D<uint> u : register(u0); groupshared uint a[4]; [numthreads(4,1,1)] void main(uint i : SV_GroupIndex) { a.x = i; u[uint2(i, 0)] = a[i]; }", "cs_5_0", "groupshared array"),
        // (shaderconf --fuzz seed 447 #265534: NIS with `groupshared float shPixelsY[][38]`; fxc: X3074
        // implicit array missing initial value. hlslc gave dxwgsl a 0-byte declaration)
        ("RWTexture2D<uint> u : register(u0); groupshared uint a[][4]; [numthreads(4,1,1)] void main(uint i : SV_GroupIndex) { a[0][i] = i; u[uint2(i, 0)] = a[0][i]; }", "cs_5_0", "without a size"),
        ("RWTexture2D<uint> u : register(u0); groupshared uint a[]; [numthreads(4,1,1)] void main(uint i : SV_GroupIndex) { u[uint2(i, 0)] = i; }", "cs_5_0", "without a size"),
        ("RWTexture2D<uint> u : register(u0); groupshared uint a[4][]; [numthreads(4,1,1)] void main(uint i : SV_GroupIndex) { u[uint2(i, 0)] = i; }", "cs_5_0", "without a size"),
    ] {
        let e = cs_err(src, target);
        assert!(e.to_lowercase().contains(want), "{src}: {e}");
    }
    // a UAV's storage format of another channel type than the shader's: an error, not WGSL
    // that WebGPU refuses; a pixel shader is no compute shader
    let mut inc = |_: &str, _: bool| None;
    let code = super::compile("RWTexture2D<uint4> u : register(u0); [numthreads(1,1,1)] void main(uint3 t : SV_DispatchThreadID) { u[t.xy] = 1; }", &[], &mut inc, "main", "cs_5_0", 0).unwrap();
    assert!(dxwgsl::translate_compute(&code, &[(0, "rgba8unorm")]).is_err());
    assert!(dxwgsl::translate_compute(&code, &[(0, "rgba8uint")]).is_ok());
    assert_eq!(dxwgsl::uav_slots(&code).unwrap().len(), 1);
    let ps = super::compile("float4 main() : SV_Target { return 1; }", &[], &mut inc, "main", "ps_5_0", 0).unwrap();
    assert!(dxwgsl::translate_compute(&ps, &[]).is_err());
    // compute-only things in a pixel shader
    for src in [
        "groupshared float g[4]; float4 main() : SV_Target { return g[0]; }",
        "float4 main() : SV_Target { GroupMemoryBarrierWithGroupSync(); return 1; }",
    ] {
        assert!(super::compile(src, &[], &mut inc, "main", "ps_5_0", 0).is_err(), "{src}");
    }
}

/// Every variant of the NVIDIA Image Scaling SDK's shader a game can compile (tests/compute/nis):
/// scaler and sharpen-only, SDR / HDR linear / HDR PQ, half precision, viewport support, NV12
/// input, thread groups of 128 and 256, the block sizes of the SDK's own config.
#[test]
fn nis_variants() {
    let mut n = 0;
    for scaler in [1, 0] {
        for hdr in ["0", "1", "2"] {
            for half in ["0", "1"] {
                for vp in ["0", "1"] {
                    for nv12 in ["0", "1"] {
                        for tg in ["128", "256"] {
                            let (bw, bh) = if scaler == 1 { ("32", "24") } else { ("32", "32") };
                            let s = scaler.to_string();
                            let defs = [("NIS_SCALER", s.as_str()), ("NIS_HDR_MODE", hdr), ("NIS_BLOCK_WIDTH", bw), ("NIS_BLOCK_HEIGHT", bh), ("NIS_THREAD_GROUP_SIZE", tg), ("NIS_USE_HALF_PRECISION", half), ("NIS_VIEWPORT_SUPPORT", vp), ("NIS_NV12_SUPPORT", nv12)];
                            let w = cs(NIS_MAIN, &defs, "cs_5_0", &[(0, "rgba8unorm")]);
                            assert!(w.contains(&format!("@workgroup_size({tg}, 1, 1)")), "{defs:?}");
                            n += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(n, 96);
}

/// Multisampled textures (Texture2DMS / Texture2DMSArray of floats and integers): Load with a
/// sample index (ld2dms), GetDimensions (resinfo and sampleinfo), GetSamplePosition
/// (samplepos), to texture_multisampled_2d in WGSL that naga accepts.
#[test]
fn multisampled_textures() {
    const SRC: &str = "Texture2DMS<float4> ms : register(t0); Texture2DMS<uint4, 4> msu : register(t1); Texture2DMSArray<float4> msa : register(t2);
float4 ps(float4 pos : SV_Position, uint si : SV_SampleIndex) : SV_Target {
    uint w, h, n, aw, ah, al, an;
    ms.GetDimensions(w, h, n);
    msa.GetDimensions(aw, ah, al, an);
    float4 c = 0;
    for (uint k = 0; k < n; k++) c += ms.Load(int2(pos.xy), k);
    c += ms.Load(int2(pos.xy), si, int2(1, -1)) + float4(msu.Load(int2(0, 0), 3)) + msa.Load(int3(pos.xy, 0), 1);
    float2 p = ms.GetSamplePosition(si);
    return c / n + float4(p, w + h, aw + ah + al + an);
}";
    for t in ["ps_4_1", "ps_5_0"] {
        let code = compile(SRC, "ps", t);
        let w = wgsl(&code, true, None);
        assert!(w.contains("texture_multisampled_2d<f32>") && w.contains("texture_multisampled_2d<u32>"), "{w}");
        assert!(w.contains("textureNumSamples(t0)") && w.contains("d3d_samplepos("), "{w}");
        assert!(w.matches("textureLoad(t").count() >= 4, "{w}");
        let b = dxwgsl::translate(&code, true, None).unwrap().bindings;
        assert!(b.iter().filter(|x| matches!(x.1, dxwgsl::Binding::Texture(6, _))).count() == 3, "{b:?}");
        assert!(b.contains(&(17, dxwgsl::Binding::Texture(6, 1))), "{b:?}");
    }
}

/// A pixel shader of 8 integer targets with logic ops, every way dxwgsl writes a target
/// (plain, direct, a stash, a stash carrying the mark, a stash whose location is the mark's
/// and whose source goes to LOGIC_SRC), to WGSL that naga accepts.
#[test]
fn logic_stores() {
    use dxwgsl::logic::{Logic, Mark, Op, Sign, Store};
    const SRC: &str = "cbuffer C : register(b0) { uint4 s; };
struct O { uint4 a : SV_Target0; uint4 b : SV_Target1; int4 c : SV_Target2; uint4 d : SV_Target3; uint4 e : SV_Target4; uint4 f : SV_Target5; uint4 g : SV_Target6; uint4 h : SV_Target7; };
O ps(float4 p : SV_Position) { O o; o.a = s; o.b = s ^ 1; o.c = int4(s); o.d = s; o.e = s; o.f = s; o.g = s; o.h = s; return o; }";
    let code = compile(SRC, "ps", "ps_5_0");
    let t = |target, store, bits, sign| Logic { target, store, bits, sign, multisampled: true };
    for mark in [Mark::Slot(0), Mark::Carried { by: 1, channel: 2, bit: 0 }] {
        let st = |op| Store::Stash { op, mark };
        let logic = [
            t(0, st(Op::Xor), [16; 4], Sign::Unsigned),
            t(1, st(Op::And), [8, 8, 0, 0], Sign::Unsigned),
            t(2, st(Op::Or), [16; 4], Sign::Signed),
            t(3, Store::Plain, [8; 4], Sign::Unsigned),
            t(4, Store::Direct(Op::Copy), [8; 4], Sign::Unsigned),
            t(5, st(Op::Nand), [16; 4], Sign::Unsigned),
            t(6, st(Op::Xor), [16; 4], Sign::Unsigned),
            t(7, st(Op::Xor), [16; 4], Sign::Unsigned),
        ];
        let w = dxwgsl::translate_logic(&code, &[], &logic).unwrap_or_else(|e| panic!("{e}"));
        let m = naga::front::wgsl::parse_str(&w.code).unwrap_or_else(|e| panic!("{}\n{}", e.emit_to_string(&w.code), w.code));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{e:?}\n{}", w.code));
        let buffered = matches!(mark, Mark::Slot(0));
        assert_eq!(w.code.contains("lg_src"), buffered, "{}", w.code);
        assert_eq!(w.bindings.contains(&(dxwgsl::logic::LOGIC_SRC, dxwgsl::Binding::StorageBuffer(true))), buffered);
        // (8 locations: no mark of its own past them; never the sample index)
        assert!(!w.code.contains("@location(8)") && !w.code.contains("sample_index"), "{}", w.code);
    }
}

/// SV_Coverage as a pixel shader input (shader model 5): vCoverage declared, one component,
/// read as WGSL's sample_mask (winref d3d11logic: 0xf of a covered pixel of a 4x target, 0xc
/// on a triangle's edge); shader model 4 has none.
#[test]
fn coverage_input() {
    const SRC: &str = "uint4 ps(float4 pos : SV_Position, uint c : SV_Coverage) : SV_Target { return uint4(c, c * 2, 0, 1); }";
    let code = compile(SRC, "ps", "ps_5_0");
    let w = wgsl(&code, true, None);
    assert!(w.contains("@builtin(sample_mask) in_cov: u32") && w.contains("vcov = vec4<u32>(in_cov);"), "{w}");
    assert!(!dxwgsl::sample_rate(&code), "coverage alone keeps the shader per pixel");
    let mut inc = |_: &str, _: bool| None;
    if let Ok(c4) = super::compile(SRC, &[], &mut inc, "ps", "ps_4_0", 0) {
        assert!(dxwgsl::translate(&c4, true, None).is_ok_and(|w| !w.code.contains("in_cov")));
    }
}

/// `code` with the instruction `inst` put before its last one (ret), its SHDR/SHEX chunk and the
/// container's chunk offsets after it moved on (the checksum is not read).
fn splice(code: &[u8], inst: &[u32]) -> Vec<u8> {
    let rd = |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    let n = rd(code, 28) as usize;
    let offs: Vec<usize> = (0..n).map(|i| rd(code, 32 + 4 * i) as usize).collect();
    let k = offs.iter().position(|&o| &code[o..o + 4] == b"SHEX" || &code[o..o + 4] == b"SHDR").unwrap();
    let (o, size) = (offs[k], rd(code, offs[k] + 4) as usize);
    let at = o + 8 + size - 4;
    assert_eq!(rd(code, at), 0x0100_003e, "ends with ret");
    let mut out = code[..at].to_vec();
    out.extend(inst.iter().flat_map(|w| w.to_le_bytes()));
    out.extend_from_slice(&code[at..]);
    let grow = 4 * inst.len() as u32;
    let put = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    let total = rd(&out, 24) + grow;
    put(&mut out, 24, total);
    put(&mut out, o + 4, size as u32 + grow);
    let len = rd(&out, o + 12) + inst.len() as u32;
    put(&mut out, o + 12, len);
    for (i, &off) in offs.iter().enumerate() {
        if off > o {
            put(&mut out, 32 + 4 * i, off as u32 + grow);
        }
    }
    out
}

/// Every opcode with 0 to 6 operands of each register type a shader declares (temps, immediates,
/// constant buffers, textures, samplers, UAVs, groupshared memory), in a pixel and a compute
/// shader: the translators give an error or valid WGSL, never a panic (fuzzing found an
/// ld_structured of 3 operands read past them; this is every such count, not a sample).
#[test]
fn every_opcode_with_any_operand_count() {
    let ps = compile(
        "cbuffer C : register(b0) { float4 k; }; Texture2D t : register(t0); SamplerState s : register(s0);\n\
         float4 main(float4 p : SV_Position) : SV_Target { return t.Sample(s, k.xy) + k; }",
        "main",
        "ps_5_0",
    );
    let mut inc = |_: &str, _: bool| None;
    let cs = super::compile(
        "cbuffer C : register(b0) { float4 k; }; Texture2D t : register(t0); SamplerState s : register(s0);\n\
         RWTexture2D<float4> u : register(u1); groupshared uint g[64];\n\
         [numthreads(64, 1, 1)] void main(uint i : SV_GroupIndex) { g[i] = i; GroupMemoryBarrierWithGroupSync();\n\
           u[uint2(i, 0)] = t.SampleLevel(s, k.xy, 0) + g[63 - i] + k; }",
        &[],
        &mut inc,
        "main",
        "cs_5_0",
        0,
    )
    .unwrap();
    // (operand tokens: r0.xyzw / r0 masked, an immediate vector, cb0[0], t0, s0, u1, g0)
    let kinds: [&[u32]; 8] = [
        &[0x0010_0e46, 0],
        &[0x0010_00f2, 0],
        &[0x0000_4002, 0x3f80_0000, 1, 2, 0xffff_ffff],
        &[0x0020_8e46, 0, 0],
        &[0x0010_7e46, 0],
        &[0x0010_6000, 0],
        &[0x0011_ee46, 1],
        &[0x0011_f006, 0],
    ];
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut bad = Vec::new();
    let (mut tried, mut took) = (0, 0);
    for op in (0..0x100u32).filter(|&op| op != 0x35) {
        for n in 0..=6usize {
            for p in 0..kinds.len() * 2 {
                // (all operands of one kind, or a masked r0 destination and the rest of one kind)
                let mut body = Vec::new();
                for j in 0..n {
                    body.extend_from_slice(if p >= kinds.len() && j == 0 { kinds[1] } else { kinds[p % kinds.len()] });
                }
                let mut inst = vec![op | ((body.len() as u32 + 1) << 24)];
                inst.extend(body);
                for (code, compute) in [(&ps, false), (&cs, true)] {
                    let b = splice(code, &inst);
                    tried += 1;
                    let r = std::panic::catch_unwind(|| if compute { dxwgsl::translate_compute(&b, &[]) } else { dxwgsl::translate(&b, true, None) });
                    match r {
                        Err(_) => bad.push(format!("op {op:#x} {n} operands kind {p} compute {compute}: panic")),
                        Ok(Err(_)) => {}
                        Ok(Ok(w)) => {
                            took += 1;
                            if let Err(e) = naga::front::wgsl::parse_str(&w.code).map_err(|e| e.emit_to_string(&w.code)).and_then(|m| {
                                naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).map(|_| ()).map_err(|e| format!("{e:?}"))
                            }) {
                                bad.push(format!("op {op:#x} {n} operands kind {p} compute {compute}: invalid WGSL {}", e.lines().next().unwrap_or("")));
                            }
                        }
                    }
                }
            }
        }
    }
    std::panic::set_hook(hook);
    assert!(bad.is_empty(), "{} of {tried} ({took} translated):\n{}", bad.len(), bad.join("\n"));
    assert!(took > 1000, "{took} of {tried} translated: the splicing is wrong");
}

/// Register pressure in the SM3 lowering must fail at compilation, rather
/// than returning bytecode that the D3D9 translator cannot consume.
#[test]
fn sm3_temporary_budget() {
    let mut inc = |_: &str, _: bool| None;
    for target in ["ps_2_0", "ps_3_0"] {
        let within = "float4 ps(float4 v:TEXCOORD0):COLOR {float4 a[25]; a[(int)v.x]=v; return a[(int)v.x];}";
        let code = super::compile(within, &[], &mut inc, "ps", target, 0).unwrap();
        let words: Vec<u32> = code.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
        assert!(dxwgsl::d3d9::Shader::parse(&words).is_some());
        for source in [
            // Ordinary array register beyond r31.
            "float4 ps(float4 v:TEXCOORD0):COLOR {float4 a[40]; a[39]=v; a[(int)v.x]=v; return v;}",
            // Conversion/comparison scratch registers also count.
            "float4 ps(float4 v:TEXCOORD0):COLOR {float4 a[28]; a[(int)v.x]=v; return v;}",
        ] {
            let err = super::compile(source, &[], &mut inc, "ps", target, 0).unwrap_err();
            assert!(err.contains("exceeds r31"), "{target}: {err}");
        }
    }
}

/// Review A-6: shaders fxc rejects (X3013 no overload, X3052 dimensions, a void or mismatched
/// condition) panicked in the lowering, and a panic is the whole VM under panic=abort: each
/// must be an error. Also run on every target, with each source cut at every byte (a
/// truncated shader file), none of which may panic.
#[test]
fn shaders_fxc_rejects_are_errors_not_panics() {
    let srcs = [
        "sampler2D s;\nfloat4 ps(float2 uv : TEXCOORD0) : COLOR { return tex2D(s); }\n",
        "sampler2D s;\nfloat4 ps(float2 uv : TEXCOORD0) : COLOR { return tex2D(s, uv, uv); }\n",
        "sampler2D s;\nfloat4 ps(float2 uv : TEXCOORD0) : COLOR { return tex2Dgrad(s, uv); }\n",
        "sampler2D s;\nfloat4 ps(float2 uv : TEXCOORD0) : COLOR { return tex2Dlod(s, uv); }\n",
        "sampler2D s;\nfloat4 ps(float3 uv : TEXCOORD0) : COLOR { return tex2Dlod(s, uv); }\n",
        "sampler2D s;\nfloat4 ps(float3 uv : TEXCOORD0) : COLOR { return tex2Dbias(s, uv); }\n",
        "sampler2D s;\nfloat4 ps(float4 uv : TEXCOORD0) : COLOR { return tex2Dproj(s, uv.xyw); }\n",
        "sampler3D s;\nfloat4 ps(float2 uv : TEXCOORD0) : COLOR { return tex3D(s, uv); }\n",
        "sampler2D s;\nfloat4 ps(float2 uv : TEXCOORD0) : COLOR { return tex2D(s, uv.x); }\n",
        "sampler2D s;\nfloat4 ps() : COLOR { return tex2D(); }\n",
        "float4 ps() : COLOR { vector<float, 9> v = 0; return v.x; }\n",
        "float4 ps() : COLOR { vector<float, 0> v = 0; return 1; }\n",
        "float4 ps() : COLOR { matrix<float, 1, 9> m = 0; return m._m00; }\n",
        "float4 ps() : COLOR { matrix<float, 200, 200> m = 0; return 1; }\n",
        "float4 ps() : COLOR { vector<float, 4294967297> v = 0; return 1; }\n",
    ];
    for src in srcs {
        for t in ["ps_2_0", "ps_3_0", "ps_4_0", "ps_5_0"] {
            let mut inc = |_: &str, _: bool| None;
            assert!(super::compile(src, &[], &mut inc, "ps", t, 0).is_err(), "{t}: {src}");
        }
    }
    // sizes 1 to 4 still compile
    for src in [
        "float4 ps() : COLOR { vector<float, 1> a = 1; vector<float, 4> b = a.xxxx; return b; }\n",
        "float4 ps() : COLOR { matrix<float, 4, 4> m = 1; matrix<float, 1, 4> n = 2; return m[0] + n[0]; }\n",
        "sampler2D s;\nfloat4 ps(float4 uv : TEXCOORD0) : COLOR { return tex2Dlod(s, uv) + tex2Dbias(s, uv) + tex2Dproj(s, uv) + tex2D(s, uv.xy, uv.xy, uv.zw) + tex2Dgrad(s, uv.xy, uv.xy, uv.zw); }\n",
    ] {
        let mut inc = |_: &str, _: bool| None;
        assert!(super::compile(src, &[], &mut inc, "ps", "ps_3_0", 0).is_ok(), "{src}");
    }
    for src in srcs {
        for cut in 0..src.len() {
            let mut inc = |_: &str, _: bool| None;
            let _ = super::compile(&src[..cut], &[], &mut inc, "ps", "ps_3_0", 0);
        }
    }
}

/// The lowering cases of review A-6 (lower.rs): a matrix chosen by a vector condition and a
/// condition of type void. fxc refuses both; they must not panic.
#[test]
fn mismatched_conditions_are_errors_not_panics() {
    for src in [
"void f() {}\nfloat4 ps() : COLOR { if (f() ? 1 : 2) return 1; return 0; }\n",
        "void f() {}\nfloat4 ps() : COLOR { return f() ? 1 : 2; }\n",
        "void f() {}\nfloat4 ps() : COLOR { if (f()) return 1; return 0; }\n",
    ] {
        for t in ["ps_3_0", "ps_4_0"] {
            let mut inc = |_: &str, _: bool| None;
            let r = super::compile(src, &[], &mut inc, "ps", t, 0);
            assert!(r.is_err(), "{t}: {src}");
        }
    }
    // (the lowering compiles this one, its components as the condition's; what matters is that
    // the matrix has its four components after it)
    let src = "float2x2 a; float2x2 b; bool2 c;\nfloat4 ps() : COLOR { float2x2 m = c ? a : b; return m._m11 + m[1].y + mul(m, float2(1, 2)).x; }\n";
    for t in ["ps_3_0", "ps_4_0"] {
        let mut inc = |_: &str, _: bool| None;
        let _ = super::compile(src, &[], &mut inc, "ps", t, 0);
    }
}

/// review b00: what a guest's D3DCompile string can make of the compiler's stack and memory.
/// Each on a thread with the guest threads' 1 MiB stack (release; debug frames are bigger):
/// nesting up to the parser's limit compiles, past it is an error, never a stack overflow.
fn on_guest_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    // (wasm32-wasip1 has no threads: there on the main stack, which is 1 MiB as well)
    if cfg!(target_os = "wasi") {
        return f();
    }
    std::thread::Builder::new().stack_size(guest_stack()).spawn(f).unwrap().join().unwrap()
}

/// The stack the deep-nesting tests run on: the guest threads' 1 MiB in a release build. A
/// sanitizer build (tools/ci/sanitize.sh: --cfg masu_sanitize) has frames several times as
/// big (ASan's redzones around every local, TSan's bookkeeping; #242: the lowering's
/// stmt/stmts recursion ran out at 1 MiB), and a debug build's are bigger still; there the
/// stack is scaled, not the limit (the release build is what ships).
fn guest_stack() -> usize {
    if cfg!(debug_assertions) {
        64 << 20
    } else if cfg!(masu_sanitize) {
        SANITIZE_STACK
    } else {
        1 << 20
    }
}
const SANITIZE_STACK: usize = 8 << 20;

fn try_compile(src: String, target: &'static str) -> Result<Vec<u8>, String> {
    on_guest_stack(move || super::compile(&src, &[], &mut |_, _| None, "ps", target, 0))
}

#[test]
fn deep_nesting_is_an_error_not_a_stack_overflow() {
    let n = super::parse::MAX_NEST as usize;
    for t in ["ps_2_0", "ps_3_0", "ps_4_0", "ps_5_0"] {
        let ret = if t.starts_with("ps_2") || t.starts_with("ps_3") { "COLOR" } else { "SV_Target" };
        let wrap = |body: String| format!("float4 A : register(c0);\nfloat4 ps() : {ret} {{ {body} }}\n");
        // (parentheses, unary minus, a sum of terms, nested blocks, member/index chains; at
        // a quarter of the limit they compile)
        let ok = [
            wrap(format!("return {}A{};", "(".repeat(n / 4), ")".repeat(n / 4))),
            wrap(format!("return {}A;", "- ".repeat(n / 4))),
            wrap(format!("return A{};", " + A".repeat(n / 4))),
            wrap(format!("float4 r = A; {} r += A; {} return r;", "{".repeat(n / 4), "}".repeat(n / 4))),
            wrap(format!("return A{};", ".xyzw".repeat(n / 4))),
        ];
        for src in ok {
            let r = try_compile(src.clone(), t);
            assert!(r.is_ok(), "{t}: {:?}\n{}", r.err(), &src[..200.min(src.len())]);
        }
        // (every depth up to past the limit: compiles or an error, the stack holds)
        for k in (1..n + 16).step_by(13) {
            for src in [
                wrap(format!("return {}A{};", "(".repeat(k), ")".repeat(k))),
                wrap(format!("return {}A;", "- ".repeat(k))),
                wrap(format!("return A{};", " * A".repeat(k))),
                wrap(format!("float4 r = A; {} r += A; {} return r;", "{".repeat(k), "}".repeat(k))),
                wrap(format!("float4 r = A; {} r = A; return r;", "if (A.x) ".repeat(k))),
                wrap(format!("return A{};", ".x".repeat(k))),
            ] {
                let _ = try_compile(src, t);
            }
        }
        for k in [n + 1, 100_000] {
            let bad = [
                wrap(format!("return {}A{};", "(".repeat(k), ")".repeat(k))),
                wrap(format!("return {}A;", "- ".repeat(k))),
                wrap(format!("return {}A;", "!".repeat(k))),
                wrap(format!("return A{};", " + A".repeat(k))),
                wrap(format!("float4 r = A; {} r += A; {} return r;", "{".repeat(k), "}".repeat(k))),
                wrap(format!("return A{};", ".x".repeat(k))),
                wrap(format!("return A{};", "[0]".repeat(k))),
                wrap(format!("float4 r = A; {} r = A; return r;", "if (A.x) ".repeat(k))),
            ];
            for src in bad {
                assert!(try_compile(src, t).is_err(), "{t} {k}");
            }
        }
    }
}

/// review b00: macros that grow as a power (40 levels of `#define An An-1 An-1`) are an error,
/// not 2^40 tokens; #if nesting is bounded, and i64::MIN / -1 is no panic
#[test]
fn preprocessor_bombs_are_errors() {
    let mut src = String::from("#define A0 x\n");
    for i in 1..=40 {
        src += &format!("#define A{i} A{} A{}\n", i - 1, i - 1);
    }
    src += "float4 ps() : COLOR { return A40; }\n";
    let e = try_compile(src, "ps_3_0").unwrap_err();
    assert!(e.contains("expansion"), "{e}");
    for k in [300usize, 100_000] {
        let src = format!("#if {}1{}\n#endif\nfloat4 ps() : COLOR {{ return 0; }}\n", "(".repeat(k), ")".repeat(k));
        assert!(try_compile(src, "ps_3_0").is_err());
        let src = format!("#if {}1\n#endif\nfloat4 ps() : COLOR {{ return 0; }}\n", "!".repeat(k));
        assert!(try_compile(src, "ps_3_0").is_err());
    }
    for e in ["0x8000000000000000 / -1", "0x8000000000000000 % -1", "-0x8000000000000000", "-(0x8000000000000000 / -1)"] {
        let src = format!("#if {e}\nfloat4 ps() : COLOR {{ return 1; }}\n#else\nfloat4 ps() : COLOR {{ return 0; }}\n#endif\n");
        assert!(try_compile(src, "ps_3_0").is_ok(), "{e}");
    }
}

/// review b00: calls doubling at each level (2^30 inlined instructions) are an error in a short
/// time; 64 nested calls (the inlining limit) fit the guest's stack; types of 2^48 components
/// and a static initializer referring to itself are errors, not an abort or a hang
#[test]
fn blowups_are_errors() {
    let mut src = String::from("float4 A : register(c0);\nfloat4 f0(float4 x) { return x * A + A.yzwx; }\n");
    for i in 1..=30 {
        src += &format!("float4 f{i}(float4 x) {{ return f{}(x) + f{}(x.wzyx); }}\n", i - 1, i - 1);
    }
    src += "float4 ps() : SV_Target { return f30(A); }\n";
    let t = std::time::Instant::now();
    let e = try_compile(src, "ps_4_0").unwrap_err();
    assert!(e.contains("instructions"), "{e}");
    assert!(t.elapsed().as_secs() < 60, "{:?}", t.elapsed());
    for (n, ok) in [(60, true), (70, false)] {
        let mut src = String::from("float4 A : register(c0);\nfloat4 f0(float4 x) { return x * A; }\n");
        for i in 1..=n {
            src += &format!("float4 f{i}(float4 x) {{ return f{}(x + A); }}\n", i - 1);
        }
        src += &format!("float4 ps() : SV_Target {{ return f{n}(A); }}\n");
        // (shader model 3 runs out of its 32 temps first)
        for t in ["ps_4_0", "ps_5_0"] {
            let src = src.clone();
            let r = try_compile(src, t);
            assert_eq!(r.is_ok(), ok, "{n} {t} {r:?}");
        }
    }
    for src in [
        "typedef float4 V[65536];\nV a[65536];\nfloat4 ps() : SV_Target { return a[0][0]; }\n",
        "typedef float4 V[65536];\nstruct S { V a; V b; };\nstatic S s[65536];\nfloat4 ps() : SV_Target { return s[0].a[0]; }\n",
        "typedef float4x4 M[4096];\ntypedef M N[4096];\nfloat4 ps() : SV_Target { N n; return n[0][0][0]; }\n",
    ] {
        let e = try_compile(src.into(), "ps_4_0").unwrap_err();
        assert!(e.contains("components"), "{e}");
    }
    for src in [
        "static float a = a;\ntechnique T { pass P { AlphaRef = a; } }\n",
        "static float a = b;\nstatic float b = a;\ntechnique T { pass P { AlphaRef = a + 1; } }\n",
    ] {
        let src = src.to_string();
        let r = on_guest_stack(move || super::compile_effect(&src, &[], &mut |_, _| None, 0).map(|_| ()));
        let _ = r;
    }
    // (an array parameter past the 32 registers from any first index)
    for src in [
        "float4 ps(float4 a[65536] : TEXCOORD0) : SV_Target { return a[0]; }\n",
        "float4 ps(float4 a[4] : TEXCOORD30) : SV_Target { return a[0]; }\n",
    ] {
        let t = std::time::Instant::now();
        assert!(try_compile(src.into(), "ps_4_0").is_err(), "{src}");
        assert!(t.elapsed().as_secs() < 5);
    }
    assert!(try_compile("float4 ps(float4 a[4] : TEXCOORD28) : SV_Target { return a[3]; }\n".into(), "ps_4_0").is_ok());
    // (packoffset past a constant buffer's 4096 registers; huge array sizes from constant
    // arithmetic that overflows)
    for src in [
        "cbuffer C { float4 A : packoffset(c4096); };\nfloat4 ps() : SV_Target { return A; }\n",
        "cbuffer C { float4 A : packoffset(c4294967295); };\nfloat4 ps() : SV_Target { return A; }\n",
        "float4 a[0x7fffffffffffffff * 4];\nfloat4 ps() : SV_Target { return a[0]; }\n",
        "float4 a[1 << 64];\nfloat4 ps() : SV_Target { return a[0]; }\n",
        "float4 a[-0x8000000000000000];\nfloat4 ps() : SV_Target { return a[0]; }\n",
    ] {
        assert!(try_compile(src.into(), "ps_4_0").is_err(), "{src}");
    }
}

/// review b00: every intrinsic and texture method with every count (0 to 4) of arguments of
/// mismatched kinds (void, scalars, vectors of each size, a matrix, bools, ints, objects): an
/// error or a shader, never a panic (an abort of the VM under panic=abort). Exhaustive up to two
/// arguments, then a sample of the three- and four-argument ones.
#[test]
fn intrinsics_with_any_arguments_are_no_panic() {
    let args = ["v()", "1", "A.x", "A.xy", "A.xyz", "A", "(int)A.x", "(int3)A.xyz", "(bool2)A.xy", "(float2x2)A", "M", "(uint4)A", "T", "S"];
    let mut fns: Vec<&str> = super::intrinsics::NO_OUT.to_vec();
    fns.extend(["sincos", "modf", "frexp", "InterlockedAdd", "InterlockedMax", "InterlockedExchange", "InterlockedCompareExchange", "asdouble", "msad4", "noise", "D3DCOLORtoUBYTE4", "dst", "lit", "fma", "rcp", "mul", "transpose", "determinant", "GroupMemoryBarrierWithGroupSync", "abort", "errorf", "printf", "EvaluateAttributeAtSample", "GetRenderTargetSampleCount", "CheckAccessFullyMapped", "WaveActiveSum", "f16tof32"]);
    let methods = ["Sample", "SampleLevel", "SampleBias", "SampleGrad", "SampleCmp", "SampleCmpLevelZero", "Gather", "GatherRed", "GatherCmp", "Load", "GetSamplePosition", "GetDimensions", "CalculateLevelOfDetail", "mips"];
    let objs = ["T", "TA", "TC", "T3", "TM", "TMA", "B", "RW"];
    let head = "float4 A : register(c0); float2x2 M; Texture2D T; Texture2DArray TA; TextureCube TC; Texture3D T3; Texture2DMS<float4> TM; Texture2DMSArray<float4> TMA; Buffer<float4> B; RWTexture2D<float4> RW; SamplerState S; SamplerComparisonState SC;\nvoid v() {}\n";
    let mut calls = Vec::new();
    let combos = |calls: &mut Vec<String>, name: &str, n: usize| {
        let mut idx = vec![0usize; n];
        loop {
            calls.push(format!("{name}({})", idx.iter().map(|&i| args[i]).collect::<Vec<_>>().join(", ")));
            let mut k = 0;
            while k < n {
                idx[k] += 1;
                if idx[k] < args.len() { break }
                idx[k] = 0;
                k += 1;
            }
            if k == n { break }
        }
    };
    for f in &fns {
        for n in 0..=2 {
            combos(&mut calls, f, n);
        }
    }
    for o in objs {
        for m in methods {
            for n in 0..=2 {
                combos(&mut calls, &format!("{o}.{m}"), n);
            }
        }
    }
    // (three and four arguments: a fixed pseudo-random sample)
    let mut seed = 0x9e3779b97f4a7c15u64;
    let mut rnd = |k: usize| { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; (seed % k as u64) as usize };
    for _ in 0..6000 {
        let name = if rnd(3) == 0 { format!("{}.{}", objs[rnd(objs.len())], methods[rnd(methods.len())]) } else { fns[rnd(fns.len())].to_string() };
        let n = 3 + rnd(2);
        let a: Vec<&str> = (0..n).map(|_| args[rnd(args.len())]).collect();
        calls.push(format!("{name}({})", a.join(", ")));
    }
    let t0 = std::time::Instant::now();
    let total = calls.len();
    // (a batch of calls per shader: each call its own statement, so one bad call fails that
    // shader; then each call of a failing batch alone)
    let loc = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let l2 = loc.clone();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |i| *l2.locked() = i.location().map_or_default(|l| format!("{}:{}", l.file(), l.line()))));
    let mut panics: std::collections::BTreeMap<String, String> = Default::default();
    let mut one = |body: &str, t: &str| {
        let src = format!("{head}float4 ps(float4 p : SV_Position) : SV_Target {{ float4 r = 0; {body} return r; }}\n");
        match std::panic::catch_unwind(|| super::compile(&src, &[], &mut |_, _| None, "ps", t, 0).is_ok()) {
            Ok(ok) => ok,
            Err(_) => {
                panics.entry(loc.locked().clone()).or_insert(format!("{t}: {body}"));
                false
            }
        }
    };
    let mut ok = 0;
    for t in ["ps_5_0", "ps_3_0"] {
        for c in &calls {
            if one(&format!("r += (float4){c};"), t) {
                ok += 1;
            }
            let _ = one(&format!("{c};"), t);
        }
    }
    std::panic::set_hook(prev);
    assert!(panics.is_empty(), "{panics:#?}");
    eprintln!("{total} calls x 2 targets: {ok} compiled, {:?}", t0.elapsed());
}

/// review b00: the DXBC checksum's table is RFC 1321's (it was computed from 64 sines per block):
/// the table is the sines, and the checksum of known blocks is unchanged (`checksum` above
/// compares whole blobs against fxc's)
#[test]
fn md5_table_is_the_sines() {
    for (i, &k) in super::sm4::MD5_K.iter().enumerate() {
        assert_eq!(k, ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32, "{i}");
    }
}

/// review b00: a D3D9 assembly instruction with more than 15 operand tokens is an error, not a
/// length that spills into the co-issue and predicate bits; a shader's constant table past a
/// comment's 32767 dwords is an error
#[test]
fn d3d9_token_fields_do_not_overflow() {
    let ok = super::d3d9asm::assemble("ps_3_0\ndef c0, 1, 2, 3, 4\nmov oC0, c0\n").unwrap();
    assert_eq!(ok[1] >> 24 & 0xf, 5);
    let many: String = (0..16).map(|i| format!(", r{i}")).collect();
    assert!(super::d3d9asm::assemble(&format!("ps_3_0\nadd r0{many}\n")).is_err());
    // (many uniform structs, each member its own type record)
    let mut src = String::new();
    for i in 0..3000 {
        src += &format!("struct S{i} {{ float4 a{i}; float4 b{i}; float c{i}; }};\nS{i} u{i} : register(c{});\n", (i * 3) % 220);
    }
    src += "float4 ps() : COLOR { return 0";
    for i in 0..3000 {
        src += &format!(" + u{i}.a{i}");
    }
    src += "; }\n";
    let _ = try_compile(src, "ps_3_0");
}

fn probe_hlsl(c: &str) -> String {
    super::probe_source(c)
}

fn valid(w: &str) {
    let m = match naga::front::wgsl::parse_str(w) {
        Ok(m) => m,
        Err(e) => panic!("{}\n{w}", e.emit_to_string(w)),
    };
    if let Err(e) = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m) {
        panic!("invalid: {e:?}\n{w}");
    }
}

/// Every float division (not ps_div's) and every float product in the functions `only` names
/// (all when empty) of `w` that a compiler could fuse into an add (WGSL leaves contraction to the implementation; WARP rounds the
/// product and the sum each): a product an add or subtract takes, or one stored in a local,
/// passed to a function or returned, without the opaque rounding sx_r / sx_r3 / ps_r / ps_r4
/// between (or ps_div, which takes its operands' bits). The
/// function and the expression of each.
fn fusable(w: &str, only: &[&str]) -> Vec<String> {
    use naga::{BinaryOperator as B, Expression as E, ScalarKind, Statement as S, TypeInner as T};
    let m = match naga::front::wgsl::parse_str(w) {
        Ok(m) => m,
        Err(e) => panic!("{}\n{w}", e.emit_to_string(w)),
    };
    let info = match naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m) {
        Ok(i) => i,
        Err(e) => panic!("invalid: {e:?}\n{w}"),
    };
    let rounders = ["sx_r", "sx_r3", "ps_r", "ps_r4", "ps_div"];
    let mut out = Vec::new();
    let mut check = |name: &str, f: &naga::Function, fi: &naga::valid::FunctionInfo| {
        if !only.is_empty() && !only.contains(&name) {
            return;
        }
        let float = |h: naga::Handle<naga::Expression>| match fi[h].ty.inner_with(&m.types) {
            T::Scalar(s) | T::Vector { scalar: s, .. } => s.kind == ScalarKind::Float,
            _ => false,
        };
        let product = |h: naga::Handle<naga::Expression>| matches!(f.expressions[h], E::Binary { op: B::Multiply, .. }) && float(h);
        let local = |h: naga::Handle<naga::Expression>| {
            let mut p = h;
            loop {
                match f.expressions[p] {
                    E::LocalVariable(_) => return true,
                    E::Access { base, .. } | E::AccessIndex { base, .. } => p = base,
                    _ => return false,
                }
            }
        };
        // (lerp2 and its lp_ helpers are the processor's double in integers: WGSL's f32 arithmetic
        // may flush subnormals and its division is off by up to 2.5 ULP)
        let integer_only = name == "lerp2" || name.starts_with("lp_");
        for (h, e) in f.expressions.iter() {
            if integer_only && matches!(e, E::Binary { .. } | E::Unary { .. } | E::Math { .. }) && float(h) {
                out.push(format!("{name}: {e:?} is f32 arithmetic (lerp2 is integers)"));
            }
            // (a float division: WGSL's may be off by 2.5 ULP, ps_div is IEEE's)
            if let E::Binary { op: B::Divide, .. } = *e
                && float(h)
            {
                out.push(format!("{name}: {e:?} divides (ps_div)"));
            }
            if let E::Binary { op: B::Add | B::Subtract, left, right } = *e
                && float(h)
                && (product(left) || product(right))
            {
                out.push(format!("{name}: {e:?} adds a product"));
            }
        }
        fn walk(b: &naga::Block, v: &mut dyn FnMut(&naga::Statement)) {
            for st in b.iter() {
                v(st);
                match st {
                    S::Block(b) => walk(b, v),
                    S::If { accept, reject, .. } => {
                        walk(accept, v);
                        walk(reject, v);
                    }
                    S::Loop { body, continuing, .. } => {
                        walk(body, v);
                        walk(continuing, v);
                    }
                    S::Switch { cases, .. } => cases.iter().for_each(|c| walk(&c.body, v)),
                    _ => {}
                }
            }
        }
        walk(&f.body, &mut |st| match st {
            S::Store { pointer, value } if local(*pointer) && product(*value) => out.push(format!("{name}: a product stored in a local")),
            S::Call { function, arguments, .. } if !rounders.contains(&m.functions[*function].name.as_deref().unwrap_or("")) && arguments.iter().any(|&a| product(a)) => {
                out.push(format!("{name}: a product passed to {:?}", m.functions[*function].name));
            }
            S::Return { value: Some(v) } if product(*v) => out.push(format!("{name}: a product returned")),
            _ => {}
        });
    };
    for (h, f) in m.functions.iter() {
        check(f.name.as_deref().unwrap_or(""), f, &info[h]);
    }
    for (i, ep) in m.entry_points.iter().enumerate() {
        check(&ep.name, &ep.function, info.get_entry_point(i));
    }
    // (the rounders' 0 set by every entry point: one left at its initial 0 lets a compiler
    // see through it)
    for (h, g) in m.global_variables.iter() {
        let Some(name) = g.name.as_deref().filter(|n| ["sx_k", "ps_k"].contains(n)) else { continue };
        for ep in &m.entry_points {
            let f = &ep.function;
            let mut set = false;
            let mut look = |b: &naga::Block| {
                fn walk(b: &naga::Block, f: &naga::Function, h: naga::Handle<naga::GlobalVariable>, set: &mut bool) {
                    for st in b.iter() {
                        match st {
                            naga::Statement::Store { pointer, .. } if matches!(f.expressions[*pointer], naga::Expression::GlobalVariable(g) if g == h) => *set = true,
                            naga::Statement::Block(b) => walk(b, f, h, set),
                            _ => {}
                        }
                    }
                }
                walk(b, f, h, &mut set);
            };
            look(&f.body);
            if !set {
                out.push(format!("{}: {name} not set at the entry", ep.name));
            }
        }
    }
    out
}

/// fusable finds each shape a compiler could fuse, and none once the products go through the
/// rounder (its self-test: each mistake put into a shader that is otherwise right).
#[test]
fn fusable_finds_each_fusable_product() {
    let head = "var<private> ps_k = 0u;\nfn ps_r(v: f32) -> f32 { return bitcast<f32>(bitcast<u32>(v) | ps_k); }\nfn g(v: f32) -> f32 { return v; }\n";
    let good = "fn lp_or(a: f32, b: f32) -> f32 { return bitcast<f32>(bitcast<u32>(a) | bitcast<u32>(b)); }\nfn f(a: f32, b: f32, c: f32) -> f32 {\n    var x = ps_r(a * b);\n    x = g(ps_r(a * c)) + x;\n    return ps_r(a * b) + c;\n}\n@compute @workgroup_size(1) fn main() { ps_k = 0u; _ = f(1.0f, 2.0f, 3.0f); }\n";
    assert_eq!(fusable(&format!("{head}{good}"), &[]), Vec::<String>::new());
    let bad = [
        ("return ps_r(a * b) + c;", "return a * b + c;", "adds a product"),
        ("return ps_r(a * b) + c;", "return c - a * b;", "adds a product"),
        ("var x = ps_r(a * b);", "var x = a * b;", "stored in a local"),
        ("g(ps_r(a * c))", "g(a * c)", "passed to"),
        ("return ps_r(a * b) + c;", "return a * b;", "returned"),
        ("ps_k = 0u; ", "", "not set at the entry"),
        ("return ps_r(a * b) + c;", "return a / b;", "divides"),
        ("return bitcast<f32>(bitcast<u32>(a) | bitcast<u32>(b));", "return a + b;", "is f32 arithmetic"),
        ("return bitcast<f32>(bitcast<u32>(a) | bitcast<u32>(b));", "return -a;", "is f32 arithmetic"),
    ];
    for (from, to, what) in bad {
        let w = format!("{head}{}", good.replace(from, to));
        let f = fusable(&w, &[]);
        assert!(f.iter().any(|m| m.contains(what)), "{to}: {f:?}");
    }
    // (a function `only` leaves out is not looked at)
    assert!(fusable(&format!("{head}{}", good.replace("return ps_r(a * b) + c;", "return a * b + c;")), &["g"]).is_empty());
}

#[test]
fn geometry_shaders_of_the_d3d11gs_probe() {
    use dxwgsl::expand::{self, Store};
    let src = probe_hlsl(include_str!("../../../tests/winref/probes/d3d11gs.c"));
    let vs = compile(&src, "vs_id", "vs_4_0");
    let ps = compile(&src, "ps_flat", "ps_4_0");
    let vs_osgn = dxwgsl::signature(&vs, true);
    let vs_regs = expand::store_regs(&vs_osgn, 0);
    valid(&expand::vertex_pass(&vs, &[]).unwrap().code);
    let pos_vs = compile(&src, "vs_so", "vs_4_0");
    let attrs = [expand::Attr { reg: 0, fmt: 2, offset: 0, slot: 0, buf: 0, step: 0 }];
    valid(&expand::vertex_pass(&pos_vs, &attrs).unwrap().code);
    for (entry, target, verts, streams) in [
        ("gs_point", "gs_4_0", 1, vec![1]),
        ("gs_line", "gs_4_0", 2, vec![1]),
        ("gs_tri", "gs_4_0", 3, vec![1]),
        ("gs_lineadj", "gs_4_0", 4, vec![1]),
        ("gs_triadj", "gs_4_0", 6, vec![1]),
        ("gs_inst", "gs_5_0", 3, vec![1]),
        ("gs_strip", "gs_4_0", 1, vec![5]),
        ("gs_lstrip", "gs_4_0", 1, vec![3]),
        ("gs_few", "gs_4_0", 1, vec![5]),
        ("gs_quad", "gs_4_0", 1, vec![5]),
        ("gs_two", "gs_5_0", 1, vec![1, 1]),
    ] {
        let gs = compile(&src, entry, target);
        let d = expand::gs_decl(&gs).unwrap_or_else(|e| panic!("{entry}: {e}"));
        assert_eq!((d.verts, &d.streams), (verts, &streams), "{entry}");
        assert_eq!(d.instances, if entry == "gs_inst" { 3 } else { 1 }, "{entry}");
        let w = expand::gs_pass(&gs, &vs_osgn, vs_regs).unwrap_or_else(|e| panic!("{entry}: {e}"));
        valid(&w.code);
        let osgn = dxwgsl::signature(&gs, true);
        for s in 0..u32::try_from(streams.len()).unwrap() {
            let store = Store::Geometry { decl: d.clone(), stream: s, regs: expand::gs_regs(&gs) };
            let decls = [expand::SoEntry { stream: s, name: Some(if entry == "gs_quad" { "K" } else { "A" }.into()), index: 0, start: 0, count: 1, slot: s }];
            let mut strides = [0; 4];
            strides[usize::try_from(s).unwrap()] = 4;
            valid(&expand::so_pass(&store, &osgn, &decls, &strides).unwrap_or_else(|e| panic!("{entry}: {e}")).code);
            if entry == "gs_quad" {
                valid(&expand::store_vertex(&store, &osgn, Some(&ps)).unwrap().code);
                let w = expand::clip_pass(&store, &osgn).unwrap().code;
                valid(&w);
                let f = fusable(&w, &[]);
                assert!(f.is_empty(), "clip pass: products a compiler could fuse\n{}", f.join("\n"));
            }
        }
    }
    // stream output of the vertex shader: gaps, start components, two slots
    let so_osgn = dxwgsl::signature(&pos_vs, true);
    let store = Store::Vertex { regs: expand::store_regs(&so_osgn, 0), corners: 3 };
    let decls = [
        expand::SoEntry { stream: 0, name: Some("COL".into()), index: 0, start: 1, count: 2, slot: 0 },
        expand::SoEntry { stream: 0, name: None, index: 0, start: 0, count: 2, slot: 0 },
        expand::SoEntry { stream: 0, name: Some("TEX".into()), index: 0, start: 0, count: 1, slot: 0 },
        expand::SoEntry { stream: 0, name: Some("SV_Position".into()), index: 0, start: 0, count: 2, slot: 1 },
    ];
    valid(&expand::so_pass(&store, &so_osgn, &decls, &[24, 16, 0, 0]).unwrap().code);
    let w = expand::clip_pass(&store, &so_osgn).unwrap().code;
    valid(&w);
    let f = fusable(&w, &[]);
    assert!(f.is_empty(), "clip pass: products a compiler could fuse\n{}", f.join("\n"));
    valid(&expand::store_vertex(&store, &so_osgn, None).unwrap().code);
}

#[test]
fn geometry_shader_errors() {
    let mut inc = |_: &str, _: bool| None;
    let mut err = |src: &str, t: &str| super::compile(src, &[], &mut inc, "main", t, 0).unwrap_err();
    let v = "struct V { float4 p : SV_Position; };\n";
    assert!(err(&format!("{v}void main(triangle V v[3], inout PointStream<V> s) {{ s.Append(v[0]); }}"), "gs_4_0").contains("maxvertexcount"));
    assert!(err(&format!("{v}[maxvertexcount(2)] void main(triangle V v[2], inout PointStream<V> s) {{ s.Append(v[0]); }}"), "gs_4_0").contains("3 vertices"));
    assert!(err(&format!("{v}[maxvertexcount(2000)] void main(point V v[1], inout PointStream<V> s) {{ s.Append(v[0]); }}"), "gs_4_0").contains("out of range"));
    assert!(err(&format!("{v}[maxvertexcount(300)] void main(point V v[1], inout TriangleStream<V> s) {{ s.Append(v[0]); }}"), "gs_4_0").contains("1024"));
    assert!(err(&format!("{v}[instance(2)][maxvertexcount(2)] void main(point V v[1], inout PointStream<V> s) {{ s.Append(v[0]); }}"), "gs_4_0").contains("gs_5_0"));
    assert!(err(&format!("{v}[maxvertexcount(2)] void main(point V v[1], inout PointStream<V> s, inout PointStream<V> t) {{ s.Append(v[0]); }}"), "gs_4_0").contains("gs_5_0"));
    assert!(err(&format!("{v}[maxvertexcount(2)] void main(point V v[1], inout PointStream<V> s, inout LineStream<V> t) {{ s.Append(v[0]); }}"), "gs_5_0").contains("PointStreams"));
    assert!(err(&format!("{v}[maxvertexcount(2)] V main(point V v[1], inout PointStream<V> s) {{ return v[0]; }}"), "gs_4_0").contains("void"));
}

/// Structured and raw buffers (StructuredBuffer, RWStructuredBuffer, Append/Consume,
/// ByteAddressBuffer, RWByteAddressBuffer): reads, partial writes, counters, GetDimensions,
/// every Interlocked* on raw buffers, structured elements and groupshared memory; the WGSL
/// dxwgsl makes of them validates (naga), and each declares what it binds.
#[test]
fn structured_and_raw_buffers() {
    let cases: &[(&str, &[&str])] = &[
        (
            "struct P { float3 p; uint n; float2x2 m; };\n\
             StructuredBuffer<P> sb : register(t0); RWStructuredBuffer<P> rw : register(u0);\n\
             [numthreads(8,1,1)] void main(uint3 t : SV_DispatchThreadID) {\n\
               P a = sb[t.x]; rw[t.x].p.y = a.p.x + a.m._12; rw[t.x + 1].n += a.n; rw[t.x].m[1] = a.m[0];\n\
               uint n, s; sb.GetDimensions(n, s); rw[0].n = n * 100 + s; }",
            &["var<storage, read> tb0: array<u32>", "var<storage, read_write> ub0: array<u32>", "struct BV { t0: vec4<u32>, u0: vec4<u32> }"],
        ),
        (
            "AppendStructuredBuffer<uint2> a : register(u1); ConsumeStructuredBuffer<uint2> c : register(u2);\n\
             RWStructuredBuffer<uint> o : register(u3);\n\
             [numthreads(4,1,1)] void main(uint3 t : SV_DispatchThreadID) { uint2 v = c.Consume(); a.Append(v + t.x); o[o.IncrementCounter()] = o.DecrementCounter(); }",
            &["atomicAdd(&uc1, 1u)", "atomicSub(&uc2, 1u)", "uc3: atomic<u32>"],
        ),
        (
            "ByteAddressBuffer b : register(t3); RWByteAddressBuffer w : register(u0);\n\
             [numthreads(1,1,1)] void main(uint3 t : SV_DispatchThreadID) {\n\
               uint4 v = b.Load4(t.x * 16); w.Store3(t.x * 12, v.xyz + b.Load(0) + b.Load2(4).y + b.Load3(8).z);\n\
               uint d; b.GetDimensions(d); w.Store(0, d); uint o;\n\
               w.InterlockedAdd(4, 1); w.InterlockedAdd(8, 2, o); w.InterlockedMin(12, (int)o); w.InterlockedMax(16, 5u);\n\
               w.InterlockedAnd(20, 6); w.InterlockedOr(24, 7); w.InterlockedXor(28, 8); w.InterlockedExchange(32, 9, o);\n\
               w.InterlockedCompareExchange(36, 1, 2, o); w.InterlockedCompareStore(40, o, 3); }",
            &["ub0: array<atomic<u32>>", "atomicCompareExchangeWeak", "atomicExchange", "bitcast<i32>"],
        ),
        (
            "groupshared uint g[64]; groupshared int m; RWStructuredBuffer<int> o : register(u0);\n\
             [numthreads(64,1,1)] void main(uint i : SV_GroupIndex) { uint was; InterlockedAdd(g[i & 7], i, was); InterlockedMax(m, (int)was - 10);\n\
               InterlockedCompareExchange(g[0], 0, 1, was); InterlockedAdd(o[i].x, m); GroupMemoryBarrierWithGroupSync(); o[i] = g[i] + was; }",
            &["var<workgroup> g0: array<atomic<u32>, 64>", "atomicLoad(&g0"],
        ),
    ];
    for (src, wants) in cases {
        let w = cs(src, &[], "cs_5_0", &[]);
        for want in *wants {
            assert!(w.contains(want), "{src}\nno {want}\n{w}");
        }
    }
    // (pixel and vertex shaders read buffers too)
    let ps = "struct L { float4 c; float k; }; StructuredBuffer<L> ls : register(t2); ByteAddressBuffer raw : register(t4);\n\
              float4 ps(float4 p : SV_Position) : SV_Target { return ls[(uint)p.x].c * ls[1].k + asfloat(raw.Load((uint)p.y * 4)); }";
    let code = super::compile(ps, &[], &mut |_, _| None, "ps", "ps_5_0", 0).unwrap();
    let w = dxwgsl::translate(&code, true, None).unwrap();
    valid(&w.code);
    assert!(w.bindings.iter().any(|b| b.0 == 502) && w.bindings.iter().any(|b| b.0 == 504) && w.bindings.iter().any(|b| b.0 == dxwgsl::BUFFER_RANGES), "{:?}", w.bindings);
    // (and what is no buffer access is an error, not a shader)
    for (src, want) in [
        ("StructuredBuffer<uint> s : register(t0); RWStructuredBuffer<uint> o : register(u0); [numthreads(1,1,1)] void main() { s[0] = 1; o[0] = s[0]; }", "read-only"),
        ("ByteAddressBuffer s : register(t0); RWStructuredBuffer<uint> o : register(u0); [numthreads(1,1,1)] void main() { o[0] = s[0]; }", "buffer"),
        ("AppendStructuredBuffer<uint> a : register(u0); [numthreads(1,1,1)] void main() { a[0] = 1; }", "buffer"),
        ("StructuredBuffer<uint> s : register(t0); RWStructuredBuffer<uint> o : register(u0); [numthreads(1,1,1)] void main() { InterlockedAdd(s[0], 1); o[0] = 0; }", "read-only"),
        ("StructuredBuffer<uint> s : register(u0); [numthreads(1,1,1)] void main() { }", ""),
    ] {
        let e = cs_err(src, "cs_5_0");
        assert!(e.contains(want), "{src}: {e}");
    }
}

/// The program (SHEX) of DXBC `code`.
fn program_of(code: &[u8]) -> dxwgsl::tpf::Program {
    let c = dxwgsl::dxbc::Container::parse(code).unwrap();
    dxwgsl::tpf::decode(c.chunk(b"SHEX").or_else(|| c.chunk(b"SHDR")).unwrap()).unwrap()
}

#[test]
fn hull_and_domain_shaders_of_the_d3d11tess_probe() {
    use dxwgsl::tpf::Opcode;
    let src = probe_hlsl(include_str!("../../../tests/winref/probes/d3d11tess.c"));
    // (domain, partitioning, output topology as the declarations number them; control points)
    for (entry, domain, part, out, cps) in [
        ("hs_tri_int", 2, 1, 3, 3),
        ("hs_tri_pow2", 2, 2, 3, 3),
        ("hs_tri_odd", 2, 3, 3, 3),
        ("hs_tri_even", 2, 4, 3, 3),
        ("hs_tri_ccw", 2, 1, 4, 3),
        ("hs_tri_point", 2, 3, 1, 3),
        ("hs_quad_int", 3, 1, 3, 4),
        ("hs_quad_pow2", 3, 2, 4, 4),
        ("hs_quad_odd", 3, 3, 3, 4),
        ("hs_quad_even", 3, 4, 3, 4),
        ("hs_quad_point", 3, 1, 1, 4),
        ("hs_line_int", 1, 1, 2, 2),
        ("hs_line_odd", 1, 3, 2, 2),
        ("hs_line_even", 1, 4, 2, 2),
        ("hs_line_point", 1, 1, 1, 2),
        ("hs_tri_max", 2, 1, 3, 3),
        ("hs_tri_one", 2, 1, 3, 1),
    ] {
        let code = compile(&src, entry, "hs_5_0");
        let p = program_of(&code);
        assert_eq!(p.kind, dxwgsl::tpf::HULL, "{entry}");
        let decl = |op: Opcode| p.insts.iter().find(|i| i.op == op).map(|i| i.ctl & 0x3f).unwrap_or_else(|| panic!("{entry}: no {op:?}"));
        assert_eq!(decl(Opcode::DclTessDomain), domain, "{entry}");
        assert_eq!(decl(Opcode::DclTessPartitioning), part, "{entry}");
        assert_eq!(decl(Opcode::DclTessOutputPrimitive), out, "{entry}");
        assert_eq!(decl(Opcode::DclOutputControlPointCount), cps, "{entry}");
        let phases: Vec<Opcode> = p.insts.iter().map(|i| i.op).filter(|o| matches!(o, Opcode::HsDecls | Opcode::HsControlPointPhase | Opcode::HsForkPhase | Opcode::HsJoinPhase)).collect();
        assert_eq!(phases, [Opcode::HsDecls, Opcode::HsControlPointPhase, Opcode::HsForkPhase], "{entry}");
        let max = p.insts.iter().find(|i| i.op == Opcode::DclHsMaxTessFactor).map(|i| f32::from_bits(i.extra[0]));
        assert_eq!(max, if entry == "hs_tri_max" { Some(5.5) } else { None }, "{entry}");
        // ([maxtessfactor(5.5)]: each tess factor leaves the fork phase as min(f, 5.5), as
        // d3dcompiler's do: winref d3d11tess reads 5.5 back for NaN and 64)
        let mins = p.insts.iter().filter(|i| i.op == Opcode::Min).count();
        assert_eq!(mins, if entry == "hs_tri_max" { 4 } else { 0 }, "{entry}");
        // (the patch constants: the domain's tess factors, then PID)
        let pc = dxwgsl::patch_constants(&code);
        let want: &[u32] = match domain {
            1 => &[16, 15],
            2 => &[13, 13, 13, 14],
            _ => &[11, 11, 11, 11, 12, 12],
        };
        let got: Vec<u32> = pc.iter().filter(|e| e.name != "PID").map(|e| e.sysval).collect();
        assert_eq!(got, want, "{entry}: {pc:?}");
        // (PID packed into r0.y beside SV_TessFactor[0], as d3dcompiler's PCSG has it: winref
        // d3d11tess)
        assert!(pc.get(1).is_some_and(|e| e.name == "PID" && e.reg == 0 && e.mask == 2), "{entry}: {pc:?}");
        assert_eq!(dxwgsl::signature(&code, true).iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["ID", "C"], "{entry}");
    }
    for (entry, domain, cps) in [("ds_tri", 2, 3), ("ds_tri_one", 2, 1), ("ds_quad", 3, 4), ("ds_line", 1, 2)] {
        let code = compile(&src, entry, "ds_5_0");
        let p = program_of(&code);
        assert_eq!(p.kind, dxwgsl::tpf::DOMAIN, "{entry}");
        let decl = |op: Opcode| p.insts.iter().find(|i| i.op == op).map(|i| i.ctl & 0x3f).unwrap_or_else(|| panic!("{entry}: no {op:?}"));
        assert_eq!(decl(Opcode::DclTessDomain), domain, "{entry}");
        assert_eq!(decl(Opcode::DclInputControlPointCount), cps, "{entry}");
        assert!(!dxwgsl::patch_constants(&code).is_empty(), "{entry}");
        assert_eq!(dxwgsl::signature(&code, true).iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["L", "B"], "{entry}");
    }
    let vs = compile(&src, "vs", "vs_5_0");
    assert_eq!(program_of(&vs).kind, dxwgsl::tpf::ProgramType(1));
}

#[test]
fn hull_shader_errors() {
    let base = "struct CP { float4 p : P; };\nstruct PC { float e[3] : SV_TessFactor; float i : SV_InsideTessFactor; };\nPC pc(InputPatch<CP, 3> ip) { PC o; o.e[0] = 1; o.e[1] = 1; o.e[2] = 1; o.i = 1; return o; }\n";
    let hs = |attrs: &str, body: &str| format!("{base}{attrs}\nCP hs(InputPatch<CP, 3> ip, uint c : SV_OutputControlPointID) {{ {body} }}\n");
    let ok = "[domain(\"tri\")] [partitioning(\"integer\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(3)] [patchconstantfunc(\"pc\")]";
    let mut inc = |_: &str, _: bool| None;
    assert!(super::compile(&hs(ok, "return ip[c];"), &[], &mut inc, "hs", "hs_5_0", 0).is_ok());
    for bad in [
        // (each attribute is needed)
        "[partitioning(\"integer\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(3)] [patchconstantfunc(\"pc\")]",
        "[domain(\"tri\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(3)] [patchconstantfunc(\"pc\")]",
        "[domain(\"tri\")] [partitioning(\"integer\")] [outputcontrolpoints(3)] [patchconstantfunc(\"pc\")]",
        "[domain(\"tri\")] [partitioning(\"integer\")] [outputtopology(\"triangle_cw\")] [patchconstantfunc(\"pc\")]",
        "[domain(\"tri\")] [partitioning(\"integer\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(3)]",
        // (values out of range, topologies of another domain, an unknown function)
        "[domain(\"hex\")] [partitioning(\"integer\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(3)] [patchconstantfunc(\"pc\")]",
        "[domain(\"tri\")] [partitioning(\"odd\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(3)] [patchconstantfunc(\"pc\")]",
        "[domain(\"tri\")] [partitioning(\"integer\")] [outputtopology(\"line\")] [outputcontrolpoints(3)] [patchconstantfunc(\"pc\")]",
        "[domain(\"tri\")] [partitioning(\"integer\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(33)] [patchconstantfunc(\"pc\")]",
        "[domain(\"tri\")] [partitioning(\"integer\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(3)] [patchconstantfunc(\"nope\")]",
        "[domain(\"tri\")] [partitioning(\"integer\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(3)] [patchconstantfunc(\"pc\")] [maxtessfactor(65)]",
        "[domain(\"tri\")] [partitioning(\"integer\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(3)] [patchconstantfunc(\"pc\")] [maxtessfactor(0.5)]",
        // (the patch constant function's factors are a quad's)
        "[domain(\"quad\")] [partitioning(\"integer\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(3)] [patchconstantfunc(\"pc\")]",
    ] {
        assert!(super::compile(&hs(bad, "return ip[c];"), &[], &mut inc, "hs", "hs_5_0", 0).is_err(), "{bad}");
    }
    // (hull and domain shaders are shader model 5's)
    assert!(super::compile(&hs(ok, "return ip[c];"), &[], &mut inc, "hs", "hs_4_0", 0).is_err());
    // (SV_TessFactor outside the patch constants)
    assert!(super::compile("float4 vs(float f : SV_TessFactor) : SV_Position { return f; }", &[], &mut inc, "vs", "vs_5_0", 0).is_err());
}

#[test]
fn tessellation_passes_of_the_d3d11tess_probe() {
    use dxwgsl::expand;
    let src = probe_hlsl(include_str!("../../../tests/winref/probes/d3d11tess.c"));
    let vs = compile(&src, "vs", "vs_5_0");
    let vs_osgn = dxwgsl::signature(&vs, true);
    let vregs = expand::store_regs(&vs_osgn, 0);
    for (hs, ds) in [
        ("hs_tri_int", "ds_tri"),
        ("hs_tri_odd", "ds_tri"),
        ("hs_quad_even", "ds_quad"),
        ("hs_quad_point", "ds_quad"),
        ("hs_line_int", "ds_line"),
        ("hs_line_point", "ds_line"),
        ("hs_tri_max", "ds_tri"),
        ("hs_tri_one", "ds_tri_one"),
    ] {
        let h = compile(&src, hs, "hs_5_0");
        let d = compile(&src, ds, "ds_5_0");
        let t = expand::tess_decl(&h).unwrap_or_else(|e| panic!("{hs}: {e}"));
        assert!(t.hull && t.cp_phase && t.forks == [1] && t.joins.is_empty(), "{hs}: {t:?}");
        assert_eq!(t.factors.len(), match t.domain { 1 => 2, 2 => 4, _ => 6 }, "{hs}: {t:?}");
        let dt = expand::tess_decl(&d).unwrap_or_else(|e| panic!("{ds}: {e}"));
        assert!(!dt.hull && dt.domain == t.domain && dt.in_cps == t.out_cps, "{ds}: {dt:?}");
        valid(&expand::hs_pass(&h, &vs_osgn, vregs).unwrap_or_else(|e| panic!("{hs}: {e}")).code);
        let out = expand::HsOut::of(&h).unwrap();
        valid(&expand::ds_pass(&d, &out).unwrap_or_else(|e| panic!("{ds}: {e}")).code);
        // (what the domain shader writes is a store as the vertex shader's)
        let dosgn = dxwgsl::signature(&d, true);
        let store = expand::Store::Vertex { regs: expand::store_regs(&dosgn, 0), corners: 3 };
        let decls = [expand::SoEntry { stream: 0, name: Some("L".into()), index: 0, start: 0, count: 4, slot: 0 }, expand::SoEntry { stream: 0, name: Some("B".into()), index: 0, start: 0, count: 4, slot: 0 }];
        valid(&expand::so_pass(&store, &dosgn, &decls, &[32, 0, 0, 0]).unwrap().code);
    }
    // (a hull shader is not a domain shader's, and the other way)
    let h = compile(&src, "hs_tri_int", "hs_5_0");
    let d = compile(&src, "ds_tri", "ds_5_0");
    assert!(expand::hs_pass(&d, &vs_osgn, vregs).is_err());
    assert!(expand::ds_pass(&h, &expand::HsOut::of(&h).unwrap()).is_err());
    assert!(expand::HsOut::of(&d).is_err());
    assert!(expand::tess_decl(&vs).is_err());
}

/// An unattached depth output must not invalidate a fragment pipeline or its UAV writes.
#[test]
fn fragment_depth_requires_an_attachment() {
    let src = r"
        RWByteAddressBuffer u : register(u1);
        float4 plain(float4 p : SV_Position) : SV_Target {
            u.Store(0, asuint(p.z));
            return p;
        }
        float4 explicit_depth(float4 p : SV_Position, out float depth : SV_Depth) : SV_Target {
            depth = 0.25;
            return plain(p);
        }
    ";
    for (entry, setup) in [("plain", Some(1)), ("explicit_depth", None), ("explicit_depth", Some(4))] {
        let code = compile(src, entry, "ps_5_0");
        for has_depth in [false, true] {
            let w = dxwgsl::translate_views(&code, true, None, None, &[], &[], setup, &[], has_depth).unwrap().code;
            valid(&w);
            assert_eq!(w.contains("@builtin(frag_depth)"), has_depth, "{entry}");
            assert!(w.contains("@binding("), "{entry}: UAV binding lost");
        }
    }
}

/// The setup of winref d3d11interp's draws (dxwgsl::expand::setup): the setup pass of the
/// vertex shader's store (triangles, wireframe, lines, points) and of the geometry shader's, the store's vertex shader and every pixel
/// shader of the probe drawn from it, at 1 and 4 samples, are valid WGSL.
#[test]
fn setup_of_the_d3d11interp_probe() {
    use dxwgsl::expand::{self, setup, Store};
    let src = probe_hlsl(include_str!("../../../tests/winref/probes/d3d11interp.c"));
    let vs = compile(&src, "vs", "vs_5_0");
    let vsc = compile(&src, "vs_clip", "vs_5_0");
    let gs = compile(&src, "gs", "gs_5_0");
    let pss = ["ps_lin", "ps_nop", "ps_cen", "ps_nopcen", "ps_smp", "ps_nopsmp", "ps_flat", "ps_pos", "ps_possmp", "ps_poscen", "ps_clip"];
    for name in pss {
        let ps = compile(&src, name, "ps_5_0");
        let layout = setup::Layout::of(&ps).unwrap_or_else(|e| panic!("{name}: {e}"));
        for samples in [1, 4] {
            let w = dxwgsl::translate_setup(&ps, &[], &[], samples).unwrap_or_else(|e| panic!("{name}: {e}")).code;
            valid(&w);
            // (the setup's own evaluation: the guest shader's arithmetic is its own)
            let f = fusable(&w, &["sx_at", "sx_line_covers"]);
            assert!(f.is_empty(), "{name}: products a compiler could fuse\n{}", f.join("\n"));
        }
        valid(&setup::setup_vertex(layout.stride()).code);
        for v in [&vs, &vsc] {
            let osgn = dxwgsl::signature(v, true);
            for prim in [setup::Prim::Triangle, setup::Prim::Wire, setup::Prim::Line, setup::Prim::Point] {
                let store = Store::Vertex { regs: expand::store_regs(&osgn, 0), corners: prim.corners() };
                let w = setup::setup_pass(&store, &osgn, Some(&ps), prim).unwrap_or_else(|e| panic!("{name} {prim:?}: {e}")).code;
                valid(&w);
                let f = fusable(&w, &[]);
                assert!(f.is_empty(), "{name} {prim:?}: products a compiler could fuse\n{}", f.join("\n"));
            }
            // (a store of other corners than the primitive's is an error, not a pass)
            let store = Store::Vertex { regs: expand::store_regs(&osgn, 0), corners: 2 };
            assert!(setup::setup_pass(&store, &osgn, Some(&ps), setup::Prim::Triangle).is_err());
        }
        let d = expand::gs_decl(&gs).unwrap();
        let store = Store::Geometry { decl: d, stream: 0, regs: expand::gs_regs(&gs) };
        for prim in [setup::Prim::Triangle, setup::Prim::Wire] {
            valid(&setup::setup_pass(&store, &dxwgsl::signature(&gs, true), Some(&ps), prim).unwrap_or_else(|e| panic!("{name} {prim:?}: {e}")).code);
        }
    }
}
