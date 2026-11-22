//! A snapshot of everything the translators make (shaderconf --snapshot; the test below): hlslc's
//! bytecode, assembly and errors, dxwgsl's WGSL and bindings through each entry point and option
//! set, glwgsl's linked stages, for every case, the oracle's mutants and the fuzzer's inputs
//! (fixed seeds). A refactor of hlslc, dxwgsl or glwgsl must leave it as it was: snapshot.tsv
//! keeps one hash per 256 lines, `shaderconf --snapshot` prints the lines themselves (run it on
//! both commits and diff to see which input changed).
//!
//!   cargo run --release -p shaderconf -- --snapshot-update   (writes snapshot.tsv: only when a
//!                                                            change of output is intended)
//!
//! The lines are written from the translators' public results field by field (not their Debug
//! text), so retyping a field (a newtype, an enum) does not change them.

use crate::{Case, Tol};
use std::panic::{self, AssertUnwindSafe};

/// FNV-1a, 64 bits: stable across Rust versions and platforms (std's hasher is not)
pub fn hash(b: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in b {
        h ^= u64::from(x);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// `f()`, or None when it panicked (the panic hook `lines` sets prints nothing)
pub fn quiet<T>(f: impl FnOnce() -> T) -> Option<T> {
    panic::catch_unwind(AssertUnwindSafe(f)).ok()
}

/// An error as one line: its first line (up to 100 chars), then the whole text's hash
pub fn err(e: &str) -> String {
    let first: String = e.lines().next().unwrap_or("").chars().take(100).collect();
    format!("E:{first}#{}", hash(e.as_bytes()))
}

pub fn result(r: Option<Result<String, String>>) -> String {
    match r {
        None => "panic".into(),
        Some(Ok(s)) => s,
        Some(Err(e)) => err(&e),
    }
}

fn binding(b: &dxwgsl::Binding) -> String {
    match b {
        dxwgsl::Binding::Uniform(n) => format!("u{n}"),
        dxwgsl::Binding::Texture(d, t) => format!("t{d},{t}"),
        dxwgsl::Binding::LoadTexture(d, t) => format!("tl{d},{t}"),
        dxwgsl::Binding::Sampler(c) => format!("s{}", u8::from(*c)),
        dxwgsl::Binding::Storage(d, f) => format!("w{d},{f}"),
        dxwgsl::Binding::StorageBuffer(rw) => format!("b{}", u8::from(*rw)),
        dxwgsl::Binding::BufferView(rw) => format!("v{}", u8::from(*rw)),
        dxwgsl::Binding::Counter => "c".into(),
        dxwgsl::Binding::BufferRanges(v) => format!("r{}", v.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(",")),
        dxwgsl::Binding::LogicDst { multisampled, sign } => format!("g{}{}", u8::from(*multisampled), u8::from(*sign == dxwgsl::logic::Sign::Signed)),
        dxwgsl::Binding::BlendConstant => "k".into(),
    }
}

fn wgsl(r: Option<Result<dxwgsl::Wgsl, String>>) -> String {
    result(r.map(|r| {
        r.map(|w| {
            let b: Vec<String> = w.bindings.iter().map(|(n, b)| format!("{n}={}", binding(b))).collect();
            format!("{}[{}]", hash(w.code.as_bytes()), b.join(" "))
        })
    }))
}

fn sig(code: &[u8], output: bool) -> String {
    let s: Vec<String> = dxwgsl::signature(code, output).iter().map(|e| format!("{}{}:{},{},{},{:x},{}", e.name, e.index, e.sysval, e.component_type, e.reg, e.mask, e.stream)).collect();
    hash(s.join(";").as_bytes())
}

/// DXBC through each dxwgsl entry point
pub fn dxbc(code: &[u8]) -> String {
    let addr = [dxwgsl::addr::Addr::argb(0, dxwgsl::addr::flags(4, 5, true), 0x80ff_4020)];
    let uavs = [(0, "rgba8unorm"), (1, "r32float")];
    let slots = result(quiet(|| dxwgsl::uav_slots(code)).map(|r| r.map(|v| v.iter().map(|(a, b, c)| format!("{a},{b},{c}")).collect::<Vec<_>>().join(";"))));
    format!(
        " ps:{} vs:{} vsps:{} cs:{} csu:{} addr:{} sprite:{} uav:{slots} in:{} out:{} gs:{:?}",
        wgsl(quiet(|| dxwgsl::translate(code, true, None))),
        wgsl(quiet(|| dxwgsl::translate(code, false, None))),
        wgsl(quiet(|| dxwgsl::translate(code, false, Some(code)))),
        wgsl(quiet(|| dxwgsl::translate_compute(code, &[]))),
        wgsl(quiet(|| dxwgsl::translate_compute(code, &uavs))),
        wgsl(quiet(|| dxwgsl::translate_addr(code, true, None, &addr))),
        wgsl(quiet(|| dxwgsl::translate_point_sprite(code, None, 64.0))),
        quiet(|| sig(code, false)).unwrap_or_else(|| "panic".into()),
        quiet(|| sig(code, true)).unwrap_or_else(|| "panic".into()),
        dxwgsl::point_sprite_gs(code).map(f32::to_bits),
    )
}

/// D3D9 tokens: the parse, then the stage's WGSL under several option sets
pub fn d3d9(w: &[u32]) -> String {
    let Some(p) = quiet(|| dxwgsl::d3d9::Shader::parse(w)) else { return " panic".into() };
    let Some(s) = p else { return " none".into() };
    let mut o = format!(
        " {}{}.{} in:{} decl:{} smp:{} c{} rel{} w{:x} r{:x} u{}",
        if s.pixel { "ps" } else { "vs" },
        s.major,
        s.minor,
        s.inputs.iter().map(|&(a, b, c)| format!("{a},{b},{c}")).collect::<Vec<_>>().join(";"),
        s.decls.iter().map(|&(a, b, c)| format!("{a},{b},{c}")).collect::<Vec<_>>().join(";"),
        s.samplers.iter().map(|&(a, b)| format!("{a},{b}")).collect::<Vec<_>>().join(";"),
        s.max_const,
        u8::from(s.uses_rel),
        s.writes,
        s.reads,
        dxwgsl::d3d9::uniform_size(s.max_const)
    );
    use dxwgsl::d3d9::Options;
    let plain = Options { link: 0, vs_inputs: Vec::new(), alpha_test: None, fog: false, addr: Vec::new(), targets: 1, flat: false };
    let addr = vec![dxwgsl::addr::Addr::argb(0, dxwgsl::addr::flags(4, 5, false), 0x4000_ff80), dxwgsl::addr::Addr::argb(2, dxwgsl::addr::flags(3, 3, true), 0xffff_ffff)];
    let all = Options { link: 0xff, vs_inputs: vec![(0, 0, 3), (1, 1, 4), (2, 2, 2)], alpha_test: Some(0x0003_0005), fog: true, addr, targets: 4, flat: true };
    for opts in [&plain, &all] {
        let g = quiet(|| {
            let g = dxwgsl::d3d9::Gen::new(&s);
            if s.pixel { g.pixel(opts) } else { g.vertex(opts) }
        });
        o.push_str(&format!(" {}", g.map_or_else(|| "panic".into(), |g| hash(g.as_bytes()))));
    }
    o
}

/// GLSL as the fragment shader of a program, and glwgsl::check of it as either stage
pub fn glsl(ps: &str) -> String {
    let stage = |s: &glwgsl::Stage| {
        let block = s.block.as_ref().map_or_else(String::new, |b| {
            let m: Vec<String> = b.members.iter().map(|m| format!("{}@{}:{}x{}*{:?}/{}", m.name, m.offset, m.cols, m.rows, m.count, m.stride)).collect();
            format!("{}{{{}}}", b.size, m.join(","))
        });
        format!("{}|{block}|{}", hash(s.wgsl.as_bytes()), s.samplers.join(","))
    };
    let link = result(quiet(|| glwgsl::link(crate::cases::GL_VS, ps, &Default::default())).map(|r| {
        r.map(|p| {
            let mut sizes: Vec<String> = p.sizes.iter().map(|(k, v)| format!("{k}={v}")).collect();
            sizes.sort();
            let attribs: Vec<String> = p.attribs.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!("vs:{} ps:{} a:{} s:{}", stage(&p.vs), stage(&p.ps), attribs.join(","), sizes.join(","))
        })
    }));
    let check = |pixel| match quiet(|| glwgsl::check(ps, pixel)) {
        None => "panic".into(),
        Some(Ok(())) => "ok".into(),
        Some(Err((a, b))) => format!("{}|{}", err(&a), err(&b)),
    };
    format!(" {link} chkps:{} chkvs:{}", check(true), check(false))
}

fn tol(t: Tol) -> String {
    match t {
        Tol::Exact => "exact".into(),
        Tol::Ulp(n) => format!("ulp{n}"),
        Tol::Abs(e) => format!("abs{:x}", e.to_bits()),
        Tol::Rel(r, a) => format!("rel{:x},{:x}", r.to_bits(), a.to_bits()),
    }
}

fn case(c: &Case) -> String {
    let mut px = Vec::new();
    for (data, want) in &c.pixels {
        px.extend_from_slice(data);
        for w in want {
            match w {
                None => px.push(0xee),
                Some(v) => {
                    px.push(u8::try_from(v.len()).unwrap_or(0xff));
                    for f in v {
                        px.extend_from_slice(&f.to_bits().to_le_bytes());
                    }
                }
            }
        }
    }
    format!("case {}\t{} g{} b{} {} {} src:{} code:{} wgsl:{} px:{}", c.name, c.api, c.group, c.binding, tol(c.tol), c.rule, hash(c.src.as_bytes()), hash(&c.code), hash(c.wgsl.as_bytes()), hash(&px))
}

/// hlslc on a case's source at more targets and flags than the case uses
fn hlsl_more(c: &Case) -> String {
    let mut o = String::new();
    let mut inc = |_: &str, _: bool| None;
    let targets: &[&str] = if c.api == "dxbc" { &["ps_4_1", "ps_5_0", "ps_4_0_level_9_3"] } else { &["ps_2_0", "ps_2_b", "ps_3_0"] };
    for t in targets {
        for flags in [0, hlslc::PACK_MATRIX_ROW_MAJOR | hlslc::IEEE_STRICTNESS, hlslc::D3D10_BLOB] {
            o.push_str(&format!(" {t}/{flags:x}:{}", result(quiet(|| hlslc::compile(&c.src, &[], &mut inc, "ps", t, flags)).map(|r| r.map(|b| hash(&b))))));
        }
        if c.api != "dxbc" {
            o.push_str(&format!(" asm:{}", result(quiet(|| hlslc::assembly(&c.src, &[], &mut inc, "ps", t, 0)).map(|r| r.map(|a| hash(a.as_bytes()))))));
        }
    }
    let defs = [("X".to_string(), "1".to_string()), ("float".to_string(), "half".to_string())];
    o.push_str(&format!(" pp:{}", result(quiet(|| hlslc::preprocess(&c.src, &defs, &mut inc)).map(|r| r.map(|t| hash(format!("{t:?}").as_bytes()))))));
    o
}

/// HLSL sources that walk hlslc's numeric types: every matrix size cast to every other, its
/// transpose, mul by vectors and matrices, members and indexing; every vector width with
/// casts, swizzles, comparisons and selects (the cases above use few of them)
fn type_sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let dims: Vec<(u8, u8)> = (1..=4).flat_map(|r| (1..=4).map(move |c| (r, c))).collect();
    for &(r, c) in &dims {
        for &(r2, c2) in &dims {
            // (a cast only narrows; a wider one starts from a component)
            let from = if r2 <= r && c2 <= c { "a" } else { "a._m00" };
            let src = format!(
                "float4 main(float4 p : TEXCOORD0, uint i : TEXCOORD1) : SV_Target {{
    float{r}x{c} a = (float{r}x{c})p.x + p.y;
    float{r2}x{c2} b = (float{r2}x{c2}){from};
    float{c}x{r} t = transpose(a);
    float{r}x{c2} m = mul(a, (float{c}x{c2})p.z);
    float{c} v = mul((float{r})p, a);
    float{r} w = mul(a, (float{c})p);
    matrix<float, {r2}, {c2}> n = b * 2;
    return float4(b._m00 + n._11, dot(t[i % {c}], t[0]), m._11, dot(v, v) + dot(w, w)) + dot(a[i % {r}], a[0]);
}}"
            );
            out.push((format!("m{r}x{c}-{r2}x{c2}"), src));
        }
    }
    for n in 1..=4 {
        for k in 1..=4 {
            for swz in ["x", "xy", "yx", "xyz", "zyx", "wzyx", "xxxx"] {
                let x = if swz.len() >= n { format!("p.{swz}") } else { format!("p.{swz}.x") };
                let y = if k <= n { "x" } else { "x.x" };
                let src = format!(
                    "float4 main(float4 p : TEXCOORD0, uint i : TEXCOORD1) : SV_Target {{
    float{n} x = (float{n}){x};
    float{k} y = (float{k}){y};
    vector<float, {n}> z = x * 3;
    bool{n} c = x > z;
    float{n} s = c ? x : -z;
    int{k} q = (int{k})y + i;
    return (float4)dot(s, s) + (float)dot(q, q) + dot(y, y);
}}"
                );
                out.push((format!("v{n}-{k}-{swz}"), src));
            }
        }
    }
    // (each interpolation modifier, on float and int inputs, beside a position and a face)
    for m in ["", "linear", "centroid", "nointerpolation", "noperspective", "noperspective centroid", "sample", "linear centroid"] {
        for t in ["float4", "float2", "int2", "uint"] {
            let src = format!(
                "float4 main({m} {t} a : TEXCOORD0, {m} float b : TEXCOORD1, float4 p : SV_Position, bool f : SV_IsFrontFace) : SV_Target {{
    return (float4)a.x * b + p * (f ? 1 : 2);
}}"
            );
            out.push((format!("interp-{m}-{t}"), src));
        }
    }
    out
}

fn types() -> Vec<String> {
    par_map(&type_sources(), |_, (name, src)| {
        let mut o = format!("types {name}\t");
        for t in ["ps_4_0", "ps_5_0", "ps_3_0"] {
            for flags in [0, hlslc::PACK_MATRIX_ROW_MAJOR] {
                o.push_str(&format!(" {t}/{flags:x}:{}", result(quiet(|| hlslc::compile(src, &[], &mut |_: &str, _: bool| None, "main", t, flags)).map(|r| r.map(|b| hash(&b))))));
            }
        }
        o
    })
}

/// Effect parameters of every D3DX class and type (numbers, vectors, matrices both ways,
/// arrays, structs, strings, each texture and sampler kind, shader objects, states,
/// annotations): one effect each, then all together, through compile_effect
fn effect_params() -> Vec<&'static str> {
    vec![
        "bool b = true;",
        "int i = -3;",
        "float f = 1.5;",
        "half h = 2;",
        "float2 v2 = {1, 2};",
        "float3 v3 = float3(1, 2, 3);",
        "float4 v4 <string UIName = \"x\"; float UIMin = 0.5;> = {1, 2, 3, 4};",
        "int2 iv = {7, -8};",
        "bool3 bv = {true, false, true};",
        "float4x4 m;",
        "float3x2 m32 = {1, 2, 3, 4, 5, 6};",
        "row_major float2x3 rm = {1, 2, 3, 4, 5, 6};",
        "column_major float3x3 cm = {1, 2, 3, 4, 5, 6, 7, 8, 9};",
        "float a[3] = {1, 2, 3};",
        "float4 av[2] = {1, 2, 3, 4, 5, 6, 7, 8};",
        "struct S { float x; int2 y; float2x2 z; }; S st;",
        "struct T { float x; }; T sa[2];",
        "string str = \"hello\";",
        "texture tx;",
        "Texture1D<float4> t1;",
        "texture2D t2;",
        "Texture2D<float4> u2;",
        "Texture3D<float4> t3;",
        "TextureCube<float4> tc;",
        "sampler s0;",
        "texture tt; sampler1D s1 = sampler_state { Texture = <tt>; };",
        "sampler2D s2 = sampler_state { MinFilter = LINEAR; AddressU = WRAP; MaxAnisotropy = 4; };",
        "sampler3D s3;",
        "samplerCUBE sc;",
        "sampler2D sar[2];",
        "float4 vsf(float4 p : POSITION) : POSITION { return p; } VertexShader vs = compile vs_2_0 vsf();",
        "PixelShader pa = asm { ps_2_0 mov oC0, c0 };",
    ]
}

const EFFECT_BODY: &str = "
float4 ps(float4 c : COLOR0) : COLOR { return c * 2; }
float4 vs2(float4 p : POSITION) : POSITION { return p; }
technique t0 <string name = \"t\";> {
    pass p0 <int k = 3;> {
        ZEnable = true;
        AlphaBlendEnable = FALSE;
        SrcBlend = SRCALPHA;
        FillMode = WIREFRAME;
        VertexShader = compile vs_2_0 vs2();
        PixelShader = compile ps_2_0 ps();
    }
    pass p1 { PixelShader = compile ps_3_0 ps(); }
}
technique t1 { pass { PixelShader = asm { ps_2_0 mov oC0, v0 }; } }";

fn effect_src(params: &str) -> String {
    format!("{params}{EFFECT_BODY}")
}

fn effects() -> Vec<String> {
    let mut srcs: Vec<(String, String)> = effect_params().iter().enumerate().map(|(k, p)| (format!("{k}"), effect_src(p))).collect();
    srcs.push(("all".into(), effect_src(&effect_params().join("\n"))));
    par_map(&srcs, |_, (name, src)| {
        let mut o = format!("effect {name}\t");
        for flags in [0, hlslc::PACK_MATRIX_ROW_MAJOR] {
            o.push_str(&format!(" {flags:x}:{}", result(quiet(|| hlslc::compile_effect(src, &[], &mut |_: &str, _: bool| None, flags)).map(|r| r.map(|b| hash(&b))))));
        }
        o
    })
}

/// `f` of each item with its index, on all cores, in order (the translators share no state)
pub fn par_map<T: Sync>(items: &[T], f: impl Fn(usize, &T) -> String + Sync) -> Vec<String> {
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let per = items.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let parts: Vec<_> = items.chunks(per).enumerate().map(|(c, part)| {
            let f = &f;
            s.spawn(move || part.iter().enumerate().map(|(k, x)| f(c * per + k, x)).collect::<Vec<_>>())
        }).collect();
        parts.into_iter().flat_map(|p| p.join().unwrap_or_else(|_| vec!["panic".into()])).collect()
    })
}

/// Every line, in a fixed order.
pub fn lines(fuzz_n: usize, oracle_n: usize) -> Vec<String> {
    let hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let (mut out, mut errors) = (Vec::new(), Vec::new());
    crate::cases::all(&mut out, &mut errors);
    let mut l: Vec<String> = errors.iter().map(|e| format!("error {}", err(e))).collect();
    let per_case = par_map(&out, |_, c| {
        let mut o = case(c);
        match c.api {
            "dxbc" => o.push_str(&format!("\ndxbc {}\t{}{}", c.name, dxbc(&c.code).trim_start(), hlsl_more(c))),
            "d3d9" | "d3d9ff" if !c.code.is_empty() => {
                let w: Vec<u32> = c.code.chunks(4).map(|b| u32::from_le_bytes([b[0], b.get(1).copied().unwrap_or(0), b.get(2).copied().unwrap_or(0), b.get(3).copied().unwrap_or(0)])).collect();
                let more = if c.src.trim_start().starts_with("ps_") || c.src.is_empty() { String::new() } else { hlsl_more(c) };
                o.push_str(&format!("\nd3d9 {}\t{}{more}", c.name, d3d9(&w).trim_start()));
            }
            "glsl" => o.push_str(&format!("\nglsl {}\t{}", c.name, glsl(&c.src).trim_start())),
            _ => {}
        }
        o
    });
    l.extend(per_case.iter().flat_map(|o| o.split('\n').map(String::from)));
    l.push(format!("compute {}", result(quiet(crate::compute::cases).map(|r| r.map(|s| hash(s.as_bytes()))))));
    for m in crate::oracle::cases(&out, oracle_n, 7) {
        l.push(case(&m));
    }
    l.extend(crate::fuzz::traces(&out, fuzz_n, 7));
    l.extend(types());
    l.extend(effects());
    panic::set_hook(hook);
    l
}

pub const FUZZ_N: usize = 20000;
pub const ORACLE_N: usize = 3000;
const CHUNK: usize = 256;

/// One line per CHUNK lines: its index, the first line's name, the hash; then the count.
pub fn digest(l: &[String]) -> String {
    let mut s = String::from("# crates/shaderconf/src/snapshot.rs: what the translators make, 256 lines per hash\n");
    for (k, c) in l.chunks(CHUNK).enumerate() {
        let name = c[0].split('\t').next().unwrap_or("");
        s.push_str(&format!("{k}\t{name}\t{}\n", hash(c.join("\n").as_bytes())));
    }
    s.push_str(&format!("lines\t{}\n", l.len()));
    s
}

pub fn path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("snapshot.tsv")
}

#[cfg(test)]
mod tests {
    /// The translators make what they made when snapshot.tsv was written (a refactor must not
    /// change a byte; an intended change rewrites it with --snapshot-update).
    #[test]
    fn translators_unchanged() {
        let l = super::lines(super::FUZZ_N, super::ORACLE_N);
        let want = std::fs::read_to_string(super::path()).unwrap_or_default();
        let got = super::digest(&l);
        if got != want {
            let diff: Vec<String> = got.lines().zip(want.lines()).filter(|(a, b)| a != b).map(|(a, b)| format!("now  {a}\nwas  {b}")).take(20).collect();
            panic!("the translators' output changed ({} lines now; `shaderconf --snapshot` on both commits, then diff):\n{}", l.len(), diff.join("\n"));
        }
    }

    #[test]
    fn hash_is_fnv1a() {
        assert_eq!(super::hash(b""), "cbf29ce484222325");
        assert_eq!(super::hash(b"a"), "af63dc4c8601ec8c");
    }
}
