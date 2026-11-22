//! Compute shader cases for tests/compute.mjs (`shaderconf --compute`): the NVIDIA Image Scaling
//! SDK's shader (tests/compute/nis) through hlslc and dxwgsl, next to Slang's translation of the
//! same source (tests/compute/slang: an independent HLSL compiler), run on the same inputs; what
//! the two store must agree within a byte. And shaders whose answer is known exactly.

use std::fmt::Write;
use std::path::Path;

/// NIS_Config.h NVScalerUpdateConfig / NVSharpenUpdateConfig (SDK 1.0.3), whole viewports: the
/// constant buffer's words.
fn config(sharp: f32, iw: u32, ih: u32, ow: u32, oh: u32, hdr: u32) -> Vec<u32> {
    let s = sharp.clamp(0.0, 1.0) - 0.5;
    let (max_scale, min_scale, limit_scale) = if s >= 0.0 { (1.25, 1.25, 1.25) } else { (1.75, 1.0, 1.0) };
    let (mut thres, mut min_c, mut max_c, mut sy, mut ey) = (64.0 / 1024.0, 2.0f32, 10.0f32, 0.45f32, 0.9f32);
    let mut smin = (0.4 + s * min_scale * 1.2f32).max(0.0);
    let mut smax = 1.6 + s * max_scale * 1.8;
    let mut lmin = (0.14 + s * limit_scale * 0.32f32).max(0.1);
    let mut lmax = 0.5 + s * limit_scale * 0.6;
    if hdr != 0 {
        (thres, min_c, max_c) = (32.0 / 1024.0, 1.5, 5.0);
        smin = (0.4 + s * min_scale * 1.1f32).max(0.0);
        smax = 2.2 + s * max_scale * 1.8;
        lmin = (0.10 + s * limit_scale * 0.28f32).max(0.06);
        lmax = 0.6 + s * limit_scale * 0.6;
        (sy, ey) = if hdr == 2 { (0.35, 0.55) } else { (0.3, 0.5) };
    }
    let f = [
        2.0 * 1127.0 / 1024.0,
        thres,
        min_c,
        1.0 / (max_c - min_c),
        1.0,
        1.0 / 255.0,
        sy,
        1.0 / (ey - sy),
        smin,
        smax - smin,
        lmin,
        lmax - lmin,
        kome::num::to_f32(iw) / kome::num::to_f32(ow),
        kome::num::to_f32(ih) / kome::num::to_f32(oh),
        1.0 / kome::num::to_f32(ow),
        1.0 / kome::num::to_f32(oh),
        1.0 / kome::num::to_f32(iw),
        1.0 / kome::num::to_f32(ih),
    ];
    let mut w: Vec<u32> = f.iter().map(|x| x.to_bits()).collect();
    w.extend_from_slice(&[0, 0, iw, ih, 0, 0, ow, oh, 0, 0]);
    w
}

/// noise RGBA bytes, alpha 255 (few exact ties between the scaler's gradients)
fn noise(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761);
    (0..w * h * 4)
        .map(|i| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345) & 0x7fff_ffff;
            if i % 4 == 3 { 255 } else { kome::num::lo8(x >> 16) }
        })
        .collect()
}

fn list<T: std::fmt::Display>(v: &[T]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
}

fn string(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", u32::from(c));
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The prefix sum of the groupshared case (a barrier in a loop counted in a register).
const SCAN: &str = "RWTexture2D<uint> u : register(u1); groupshared uint a[64], b[64];\n\
        [numthreads(64, 1, 1)] void main(uint3 t : SV_DispatchThreadID, uint i : SV_GroupIndex, uint3 g : SV_GroupID) {\n\
          a[i] = (i * 7 + g.y * 3) % 13; GroupMemoryBarrierWithGroupSync();\n\
          [loop] for (uint s = 1; s < 64; s <<= 1) {\n\
            b[i] = i >= s ? a[i] + a[i - s] : a[i]; GroupMemoryBarrierWithGroupSync();\n\
            a[i] = b[i]; GroupMemoryBarrierWithGroupSync(); }\n\
          u[t.xy] = a[63 - i] * 1000 + a[i]; }\n";

/// Compute shader sources for the fuzzer (entry main, cs_5_0, nothing to include): the scan, a
/// read-modify-write of a UAV, and the NIS scaler and sharpener with their defines and header
/// written in.
pub fn fuzz_sources() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/compute/nis");
    let main = std::fs::read_to_string(dir.join("NIS_Main.hlsl")).unwrap_or_default();
    let header = std::fs::read_to_string(dir.join("NIS_Scaler.h")).unwrap_or_default();
    let mut out = vec![
        SCAN.to_string(),
        "Texture2D<float4> t : register(t0); RWTexture2D<unorm float4> u : register(u0);\n\
         [numthreads(8, 8, 1)] void main(uint3 i : SV_DispatchThreadID, uint3 l : SV_GroupThreadID) {\n\
           u[i.xy] = u[i.xy] * 0.5 + t.Load(int3(i.xy, 0)) * float(l.x) / 8; }\n"
            .to_string(),
    ];
    for (sc, hdr, half, vp) in [(1, 0, 0, 0), (0, 0, 0, 1), (1, 1, 1, 0)] {
        let (bw, bh) = if sc == 1 { (32, 24) } else { (32, 32) };
        let defs = format!(
            "#define NIS_SCALER {sc}\n#define NIS_HDR_MODE {hdr}\n#define NIS_BLOCK_WIDTH {bw}\n#define NIS_BLOCK_HEIGHT {bh}\n\
             #define NIS_THREAD_GROUP_SIZE 128\n#define NIS_VIEWPORT_SUPPORT {vp}\n#define NIS_USE_HALF_PRECISION {half}\n"
        );
        out.push(defs + &main.replace("#include \"NIS_Scaler.h\"", &header));
    }
    out
}

/// The cases as compute.mjs reads them.
pub fn cases() -> Result<String, String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/compute");
    let read = |p: &str| std::fs::read(dir.join(p)).map_err(|e| format!("{p}: {e}"));
    let text = |p: &str| std::fs::read_to_string(dir.join(p)).map_err(|e| format!("{p}: {e}"));
    let main = text("nis/NIS_Main.hlsl")?;
    let scaler_h = text("nis/NIS_Scaler.h")?;
    let floats = |b: Vec<u8>| list(&b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).map(|f| format!("{f:e}")).collect::<Vec<_>>());
    let (coef_scale, coef_usm) = (floats(read("nis/coef_scale.f32")?), floats(read("nis/coef_usm.f32")?));
    let mut out = Vec::new();
    // (name, scaler, in w h, out w h, sharpness, hdr, thread group, half precision, viewport)
    let all: [(&str, u32, u32, u32, u32, u32, f32, u32, u32, u32, u32); 6] = [
        ("nis scaler 64x48->96x72 s0.5 tg128", 1, 64, 48, 96, 72, 0.5, 0, 128, 0, 0),
        ("nis scaler 50x30->133x77 s1.0 tg256", 1, 50, 30, 133, 77, 1.0, 0, 256, 0, 0),
        ("nis scaler 64x48->96x72 s0.0 hdr1 tg128", 1, 64, 48, 96, 72, 0.0, 1, 128, 0, 0),
        ("nis scaler 40x40->70x55 s0.3 hdr2 half tg128", 1, 40, 40, 70, 55, 0.3, 2, 128, 1, 0),
        ("nis sharpen 70x50 s0.5 tg256", 0, 70, 50, 70, 50, 0.5, 0, 256, 0, 0),
        ("nis sharpen 33x65 s1.0 tg128 vp", 0, 33, 65, 33, 65, 1.0, 0, 128, 0, 1),
    ];
    for (k, &(name, sc, iw, ih, ow, oh, sharp, hdr, tg, half, vp)) in all.iter().enumerate() {
        let (bw, bh) = if sc == 1 { (32, 24) } else { (32, 32) };
        let defs: Vec<(String, String)> = [
            ("NIS_SCALER", sc),
            ("NIS_HDR_MODE", hdr),
            ("NIS_BLOCK_WIDTH", bw),
            ("NIS_BLOCK_HEIGHT", bh),
            ("NIS_THREAD_GROUP_SIZE", tg),
            ("NIS_VIEWPORT_SUPPORT", vp),
            ("NIS_USE_HALF_PRECISION", half),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let mut inc = |n: &str, _: bool| (n == "NIS_Scaler.h").then(|| scaler_h.clone());
        let code = hlslc::compile(&main, &defs, &mut inc, "main", "cs_5_0", 0).map_err(|e| format!("{name}: hlslc: {e}"))?;
        let ours = dxwgsl::translate_compute(&code, &[(0, "rgba8unorm")]).map_err(|e| format!("{name}: dxwgsl: {e}"))?.code;
        let slang = text(&format!("slang/nis_s{sc}_h{hdr}_t{tg}_p{half}_v{vp}.wgsl"))?;
        let img = noise(iw, ih, 2 * kome::num::u32_of(k) + 1);
        let mut b = vec![format!(r#"{{"binding":0,"uniform":[{}]}}"#, list(&config(sharp, iw, ih, ow, oh, hdr)))];
        b.push(format!(r#"{{"binding":16,"texture":{{"format":"rgba8unorm","w":{iw},"h":{ih},"bytes":[{}]}}}}"#, list(&img)));
        if sc == 1 {
            b.push(format!(r#"{{"binding":17,"texture":{{"format":"rgba32float","w":2,"h":64,"f32":[{coef_scale}]}}}}"#));
            b.push(format!(r#"{{"binding":18,"texture":{{"format":"rgba32float","w":2,"h":64,"f32":[{coef_usm}]}}}}"#));
            // (masu's ld reads the coefficients from every level's views, LOAD_TEXTURES + 1 and
            // + 2, their one level in the ranges)
            let l = dxwgsl::LOAD_TEXTURES;
            b.push(format!(r#"{{"binding":{},"texture":{{"format":"rgba32float","w":2,"h":64,"f32":[{coef_scale}]}}}}"#, l + 1));
            b.push(format!(r#"{{"binding":{},"texture":{{"format":"rgba32float","w":2,"h":64,"f32":[{coef_usm}]}}}}"#, l + 2));
            b.push(format!(r#"{{"binding":{},"uniform":[0,1,0,0,0,1,0,0]}}"#, dxwgsl::BUFFER_RANGES));
        }
        b.push(r#"{"binding":160,"sampler":{"filter":"linear","address":"clamp-to-edge"}}"#.into());
        b.push(format!(r#"{{"binding":200,"storage":{{"format":"rgba8unorm","w":{ow},"h":{oh}}}}}"#));
        out.push(format!(
            r#"{{"name":{},"dispatch":[{},{},1],"out":200,"tol":1,"variants":[{{"name":"masu","wgsl":{}}},{{"name":"slang","wgsl":{}}}],"bindings":[{}]}}"#,
            string(name),
            ow.div_ceil(bw),
            oh.div_ceil(bh),
            string(&ours),
            string(&slang),
            b.join(",")
        ));
    }
    // a prefix sum in groupshared memory with a barrier in a loop counted in a register (WGSL's
    // uniformity rules: a barrier only in uniform control flow), against its answer
    let code = hlslc::compile(SCAN, &[], &mut |_, _| None, "main", "cs_5_0", 0).map_err(|e| format!("scan: {e}"))?;
    let ours = dxwgsl::translate_compute(&code, &[(1, "r32uint")]).map_err(|e| format!("scan: {e}"))?.code;
    let mut want = Vec::new();
    for gy in 0..4u32 {
        let a: Vec<u32> = (0..64).map(|i| (i * 7 + gy * 3) % 13).collect();
        let s: Vec<u32> = (0..64).map(|i| a[..=i].iter().sum()).collect();
        for i in 0..64 {
            want.extend_from_slice(&(s[63 - i] * 1000 + s[i]).to_le_bytes());
        }
    }
    out.push(format!(
        r#"{{"name":"groupshared scan, barriers in a loop","dispatch":[1,4,1],"out":201,"want":[{}],"variants":[{{"name":"masu","wgsl":{}}}],"bindings":[{{"binding":201,"storage":{{"format":"r32uint","w":64,"h":4}}}}]}}"#,
        list(&want),
        string(&ours)
    ));
    out.extend(buffer_cases()?);
    out.extend(ps_cases());
    out.extend(setup_cases()?);
    out.push(div_case());
    out.push(lerp_case());
    Ok(format!("[\n{}\n]\n", out.join(",\n")))
}

/// ps_div (WGSL) against IEEE 754's f32 division (Rust's `/`): random bit patterns, quotients
/// near every rounding boundary, subnormals in and out, overflow, zeros, infinities, NaN (every
/// NaN compared as the one quiet NaN: its payload is the implementation's).
fn div_case() -> String {
    use dxwgsl::expand::ps;
    const N: u32 = 1 << 16;
    let mut rng = Rng(0x1234_5679);
    let specials = [0u32, 0x8000_0000, 1, 0x8000_0001, 0x007f_ffff, 0x0080_0000, 0x7f7f_ffff, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0x3f80_0000, 0xbf80_0000, 0x3f7f_ffff, 0x0040_0000];
    let mut ins = Vec::new();
    for i in 0..N {
        let pick = |rng: &mut Rng| -> u32 {
            match rng.next() % 8 {
                0 => specials.get(kome::num::to_usize(rng.next() % 14)).copied().unwrap_or(0),
                // (small exponents: subnormal quotients and inputs)
                1 => rng.next() & 0x81ff_ffff,
                // (large ones: overflow)
                2 => rng.next() | 0x7e00_0000,
                // (short mantissas: exact quotients and ties' neighbours)
                3 => (rng.next() & 0xfff8_0000) | 0x3f00_0000,
                _ => rng.next(),
            }
        };
        let (a, b) = (pick(&mut rng), pick(&mut rng));
        // (every 16th: a = q * b for a random q, the quotient next to a representable one)
        let a = if i % 16 == 0 { (f32::from_bits(rng.next() & 0x3fff_ffff | 0x3f00_0000) * f32::from_bits(b)).to_bits() } else { a };
        ins.extend([a, b]);
    }
    let canon = |v: u32| if v & 0x7f80_0000 == 0x7f80_0000 && v & 0x007f_ffff != 0 { 0x7fc0_0000 } else { v };
    let want: Vec<u32> = ins.as_chunks::<2>().0.iter().map(|&[a, b]| canon((f32::from_bits(a) / f32::from_bits(b)).to_bits())).collect();
    let mut m = String::new();
    m.push_str(ps::DIV);
    m.push_str(&format!(
        "@group(0) @binding(1) var<storage, read_write> ins: array<u32>;\n@group(0) @binding(2) var<storage, read_write> outs: array<u32>;\n\
         @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) g: vec3<u32>) {{\n  let i = g.y * 65535u * 64u + g.x;\n  if (i >= {N}u) {{ return; }}\n\
           var v = bitcast<u32>(ps_div(bitcast<f32>(ins[i * 2u]), bitcast<f32>(ins[i * 2u + 1u])));\n\
           if ((v & 0x7f800000u) == 0x7f800000u && (v & 0x7fffffu) != 0u) {{ v = 0x7fc00000u; }}\n  outs[i] = v;\n}}\n"
    ));
    let b = [format!(r#"{{"binding":1,"buffer":{{"words":[{}]}}}}"#, list(&ins)), format!(r#"{{"binding":2,"buffer":{{"words":[{}]}}}}"#, list(&vec![0u32; kome::num::to_usize(N)]))];
    format!(
        r#"{{"name":"ps_div (WGSL) = IEEE 754 f32 division","dispatch":[{},1,1],"out":[2],"want":[{}],"variants":[{{"name":"masu","wgsl":{}}}],"bindings":[{}]}}"#,
        N.div_ceil(64),
        list(&words_bytes(&want)),
        string(&m),
        b.join(",")
    )
}

/// The clipper's lerp2 (WGSL: WARP's double, emulated in integers) against ps::lerp (Rust, f64)
/// over random, special and cancelling operands, NaNs' bits as the processor's.
fn lerp_case() -> String {
    use dxwgsl::expand::{ps, xd, B_XD, XD_VECS};
    const N: u32 = 1 << 16;
    let mut rng = Rng(0x2545_f491);
    let specials = [0u32, 0x8000_0000, 1, 0x8000_0001, 0x007f_ffff, 0x0080_0000, 0x7f7f_ffff, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0xffc1_2345, 0x3f80_0000, 0xbf80_0000, 0x3f7f_ffff, 0x0040_0000, 0x7fa0_0001];
    let mut ins = Vec::new();
    for i in 0..N {
        let pick = |rng: &mut Rng| -> u32 {
            match rng.next() % 8 {
                0 => specials.get(kome::num::to_usize(rng.next() % 16)).copied().unwrap_or(0),
                1 => rng.next() & 0x81ff_ffff,
                2 => rng.next() | 0x7e00_0000,
                3 => (rng.next() & 0xfff8_0000) | 0x3f00_0000,
                4 => (rng.next() & 0x80ff_ffff) | 0x3e00_0000,
                _ => rng.next(),
            }
        };
        let (a, b, d, n) = (pick(&mut rng), pick(&mut rng), pick(&mut rng), pick(&mut rng));
        // (every 8th: a near the quotient's negative, the sum cancelling)
        let a = if i % 8 == 1 {
            let (fa, fb, fd, fn_) = (f32::from_bits(a), f32::from_bits(b), f32::from_bits(d), f32::from_bits(n));
            let q = kome::num::to_f32(f64::from(fb - fa) * f64::from(fd) / f64::from(fn_));
            (-q).to_bits() ^ (rng.next() & 3)
        } else {
            a
        };
        ins.extend([a, b, d, n]);
    }
    let want: Vec<u32> = ins.as_chunks::<4>().0.iter().map(|&[a, b, d, n]| ps::lerp(f32::from_bits(a), f32::from_bits(b), f32::from_bits(d), f32::from_bits(n)).to_bits()).collect();
    let mut m = String::new();
    m.push_str(&format!("@group(0) @binding({B_XD}) var<uniform> xd: array<vec4<u32>, {XD_VECS}>;\n"));
    m.push_str("fn xw(i: u32) -> u32 { return xd[i / 4u][i % 4u]; }\n");
    for (name, w) in xd::NAMED {
        m.push_str(&format!("const XD_{name} = {w}u;\n"));
    }
    m.push_str(ps::WGSL);
    m.push_str(ps::DIV);
    m.push_str(&format!(
        "@group(0) @binding(1) var<storage, read_write> ins: array<vec4<u32>>;\n@group(0) @binding(2) var<storage, read_write> outs: array<u32>;\n\
         @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) g: vec3<u32>) {{\n{}  let i = g.x;\n  if (i >= {N}u) {{ return; }}\n\
           let v = bitcast<vec4<f32>>(ins[i]);\n  outs[i] = bitcast<u32>(lerp2(v.x, v.y, v.z, v.w));\n}}\n",
        ps::ENTRY
    ));
    let b = [
        format!(r#"{{"binding":{B_XD},"uniform":[{}]}}"#, list(&vec![0u32; kome::num::to_usize(XD_VECS).saturating_mul(4)])),
        format!(r#"{{"binding":1,"buffer":{{"words":[{}]}}}}"#, list(&ins)),
        format!(r#"{{"binding":2,"buffer":{{"words":[{}]}}}}"#, list(&vec![0u32; kome::num::to_usize(N)])),
    ];
    format!(
        r#"{{"name":"lerp2 (WGSL) = ps::lerp (Rust, f64)","dispatch":[{},1,1],"out":[2],"want":[{}],"variants":[{{"name":"masu","wgsl":{}}}],"bindings":[{}]}}"#,
        N.div_ceil(64),
        list(&words_bytes(&want)),
        string(&m),
        b.join(",")
    )
}

/// Words a buffer starts with where a shader leaves it alone.
fn pattern(n: usize) -> Vec<u32> {
    (0..kome::num::u32_of(n)).map(|k| k.wrapping_mul(0x0101_0107) ^ 0x5a5a_0000).collect()
}

fn words_bytes(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// A case of buffers only: (binding, its words) bound, the bindings read back after one
/// dispatch, what they must hold (Direct3D 11.3 spec: raw and structured views, the hidden
/// counter, the atomics; reads out of the view 0, writes there dropped).
/// A compute case whose outputs are storage buffers: `bufs` (binding, words), the views'
/// ranges `uniform`, `extra` other bindings as tests/compute.mjs takes them.
#[allow(clippy::too_many_arguments)]
fn buffer_case(name: &str, src: &str, dispatch: [u32; 3], bufs: &[(u32, Vec<u32>)], uniform: Vec<u32>, extra: &[String], outs: &[u32], want: Vec<u32>) -> Result<String, String> {
    let code = hlslc::compile(src, &[], &mut |_, _| None, "main", "cs_5_0", 0).map_err(|e| format!("{name}: hlslc: {e}"))?;
    let ours = dxwgsl::translate_compute(&code, &[]).map_err(|e| format!("{name}: dxwgsl: {e}"))?;
    let ranges: Vec<u32> = ours.bindings.iter().filter_map(|(_, b)| if let dxwgsl::Binding::BufferRanges(r) = b { Some(r.clone()) } else { None }).flatten().collect();
    // (the textures `ld` reads first, their levels; then the buffer views' ranges)
    let loaded = ours.bindings.iter().filter_map(|(n, b)| matches!(b, dxwgsl::Binding::LoadTexture(..)).then_some(*n));
    let want_ranges: Vec<u32> = loaded.chain(bufs.iter().map(|b| b.0).filter(|&b| (500..700).contains(&b))).collect();
    if ranges != want_ranges {
        return Err(format!("{name}: ranges of {ranges:?}, the case binds {want_ranges:?}"));
    }
    let mut b: Vec<String> = bufs.iter().map(|(n, w)| format!(r#"{{"binding":{n},"buffer":{{"words":[{}]}}}}"#, list(w))).collect();
    b.push(format!(r#"{{"binding":{},"uniform":[{}]}}"#, dxwgsl::BUFFER_RANGES, list(&uniform)));
    b.extend(extra.iter().cloned());
    Ok(format!(
        r#"{{"name":{},"dispatch":[{},{},{}],"out":[{}],"want":[{}],"variants":[{{"name":"masu","wgsl":{}}}],"bindings":[{}]}}"#,
        string(name),
        dispatch[0],
        dispatch[1],
        dispatch[2],
        list(outs),
        list(&words_bytes(&want)),
        string(&ours.code),
        b.join(",")
    ))
}

fn buffer_cases() -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    // structured: a view from element 1 (its first dword 8 in what is bound) of 7 elements; a
    // struct's members, a column_major matrix's elements, past the view 0; GetDimensions; a
    // partial write keeps the rest of the element
    let src = "struct P { float3 p; uint n; float2x2 m; };\n\
        StructuredBuffer<P> sb : register(t0); RWStructuredBuffer<uint4> o : register(u1);\n\
        [numthreads(8,1,1)] void main(uint3 t : SV_DispatchThreadID) {\n\
          P a = sb[t.x];\n\
          o[t.x] = uint4(asuint(a.p.y), a.n, asuint(a.m._12), asuint(a.m[1].x));\n\
          if (t.x == 0) { uint n, s; sb.GetDimensions(n, s); o[8].xy = uint2(n, s); } }";
    // (structure e of the view, word j: distinct, and floats where they are read as floats)
    let elem = |e: u32, j: u32| 0x3f80_0000u32.wrapping_add(e.wrapping_mul(1000)).wrapping_add(j);
    let mut input = pattern(8);
    for e in 1..8 {
        input.extend((0..8).map(|j| elem(e, j)));
    }
    let init = pattern(36);
    let mut want = Vec::new();
    for e in 1..8 {
        want.extend([elem(e, 1), elem(e, 3), elem(e, 6), elem(e, 5)]);
    }
    want.extend([0, 0, 0, 0, 7, 32]);
    want.extend_from_slice(init.get(34..).unwrap_or_default());
    out.push(buffer_case("structured: view range, members, matrices, out of range", src, [1, 1, 1], &[(500, input), (601, init)], vec![8, 7 * 32, 7, 0, 0, 144, 9, 0], &[], &[601], want)?);
    // the hidden counters: Append and IncrementCounter return the count before, Consume and
    // DecrementCounter the count after
    let src = "AppendStructuredBuffer<uint> a : register(u0); RWStructuredBuffer<uint> c : register(u1); ConsumeStructuredBuffer<uint> s : register(u2);\n\
        [numthreads(1,1,1)] void main() { a.Append(7); a.Append(8); uint i = c.IncrementCounter(); c[i] = 50; c[c.DecrementCounter() + 4] = 60;\n\
          uint v = s.Consume(); c[6] = v; c[7] = s.Consume(); }";
    let (ia, ic) = (pattern(4), pattern(8));
    let mut want: Vec<u32> = ia.iter().take(2).copied().chain([7, 8]).collect();
    want.extend(ic.iter().enumerate().map(|(k, &x)| match k {
        1 => 50,
        5 => 60,
        6 => 12,
        7 => 11,
        _ => x,
    }));
    want.extend([4, 1, 1]);
    out.push(buffer_case(
        "append, consume and the counters",
        src,
        [1, 1, 1],
        &[(600, ia), (601, ic), (602, vec![10, 11, 12, 13]), (700, vec![2]), (701, vec![1]), (702, vec![3])],
        vec![0, 16, 4, 0, 0, 32, 8, 0, 0, 16, 4, 0],
        &[],
        &[600, 601, 700, 701, 702],
        want,
    )?);
    // raw views and atomics (signed and unsigned min/max, compare-store, exchange, on groupshared
    // memory too); a raw view of 6 dwords from dword 2
    let src = "ByteAddressBuffer b : register(t0); RWByteAddressBuffer w : register(u0); groupshared uint g[4];\n\
        [numthreads(64,1,1)] void main(uint i : SV_GroupIndex) {\n\
          if (i < 4) g[i] = 0;\n\
          GroupMemoryBarrierWithGroupSync();\n\
          InterlockedAdd(g[i & 3], i);\n\
          w.InterlockedAdd(0, 1); w.InterlockedMax(4, (int)i - 32); w.InterlockedMin(8, (int)i - 32); w.InterlockedMin(12, i + 5);\n\
          w.InterlockedOr(16, 1u << (i & 31)); w.InterlockedXor(20, i); w.InterlockedCompareStore(24, 0, 77);\n\
          uint was; if (i == 5) { w.InterlockedExchange(28, 99, was); w.Store(32, was); }\n\
          GroupMemoryBarrierWithGroupSync();\n\
          if (i == 0) { w.Store4(36, uint4(g[0], g[1], g[2], g[3])); uint d; b.GetDimensions(d); w.Store2(52, uint2(d, b.Load3(8).z)); w.Store(60, b.Load(1000)); } }";
    let raw: Vec<u32> = (0x100..0x109).collect();
    let iw: Vec<u32> = [0, (-100i32).cast_unsigned(), 100, 1000, 0, 0, 0, 13].into_iter().chain(pattern(16).into_iter().skip(8)).collect();
    // (g[k]: the sum of the group indices i with i & 3 == k, 480 + 16k; raw dword 6 of the
    // buffer, 0x106, is the view's dword 4)
    let want = vec![64, 31, (-32i32).cast_unsigned(), 5, u32::MAX, 0, 77, 99, 13, 480, 496, 512, 528, 24, 0x106, 0];
    out.push(buffer_case("raw views and atomics", src, [1, 1, 1], &[(500, raw), (600, iw)], vec![2, 24, 24, 0, 0, 64, 64, 0], &[], &[600], want)?);
    // Load outside the texture: 0 in every component (an address past its size or below 0, a
    // level past its levels, an offset taking it outside); inside, with and without an offset
    let src = "Texture2D<float4> t : register(t0); RWStructuredBuffer<float4> o : register(u0);\n\
        [numthreads(1,1,1)] void main() { o[0] = t.Load(int3(1, 1, 0)); o[1] = t.Load(int3(2, 0, 0)); o[2] = t.Load(int3(-1, 0, 0));\n\
          o[3] = t.Load(int3(0, 0, 1)); o[4] = t.Load(int3(0, 1, 0), int2(1, 0)); o[5] = t.Load(int3(1, 0, 0), int2(1, 0)); o[6] = t.Load(int3(0, 5, 0)); }";
    // (texel i of the 2x2: 10i + 1 .. 10i + 4)
    let texel = |i: u8| (1..5u8).map(move |c| f32::from(i.wrapping_mul(10).wrapping_add(c)).to_bits());
    let tex: Vec<String> = (0..4u8).flat_map(texel).map(|b| f32::from_bits(b).to_string()).collect();
    // (ld reads tl0, binding LOAD_TEXTURES: every level's view; the SRV's levels 0 and 1 in the
    // ranges first)
    let tex = format!(r#"{{"binding":{},"texture":{{"format":"rgba32float","w":2,"h":2,"f32":[{}]}}}}"#, dxwgsl::LOAD_TEXTURES, tex.join(","));
    let want: Vec<u32> = texel(3).chain([0; 12]).chain(texel(3)).chain([0; 8]).collect();
    out.push(buffer_case("Load outside the texture", src, [1, 1, 1], &[(600, pattern(28))], vec![0, 1, 0, 0, 0, 112, 7, 0], &[tex], &[600], want)?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    /// (the cases build: every variant compiles and translates, the fixtures are there; 11
    /// buffer cases and the clip pass's 7 ps_cases)
    #[test]
    fn compute_cases_build() {
        let s = super::cases().unwrap();
        assert_eq!(s.matches("\"dispatch\"").count(), 18);
    }
}

/// A xorshift generator for the primitives of [`ps_cases`] (fixed seeds: the same cases each run).
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// A float in [lo, hi), on a grid of 1/64 a quarter of the time (ties on pixel centres and
    /// edges)
    fn f(&mut self, lo: f32, hi: f32) -> f32 {
        let u = kome::num::to_f32(self.next() >> 8) / 16_777_216.0;
        let v = lo + (hi - lo) * u;
        if self.next() % 4 == 0 { (v * 64.0).round() / 64.0 } else { v }
    }

    /// A clip-space position: mostly inside the volume, sometimes outside planes, now and then
    /// behind the eye or on w = 0
    fn pos(&mut self) -> [f32; 4] {
        let w = match self.next() % 16 {
            0 => -self.f(0.1, 2.0),
            1 => 0.0,
            _ => self.f(0.25, 3.0),
        };
        let span = if self.next() % 3 == 0 { 3.0 } else { 1.1 };
        let a = w.abs().max(0.25) * span;
        [self.f(-a, a), self.f(-a, a), self.f(-0.5 * w.abs(), 1.5 * w.abs()), w]
    }
}

/// PSInvocations' WGSL (dxwgsl expand::ps::WGSL, the clip pass's ps_pairs) against its Rust rule
/// (ps::invocations, the one checked against Windows' WARP counts: winref d3d11gs) on random
/// points, lines and triangles under several rasterizer states: every primitive's pairs the same.
/// The clip pass is the one counting on the page; the Rust rule alone is what the tests know.
fn ps_cases() -> Vec<String> {
    use dxwgsl::expand::{ps, xd, B_XD, XD_VECS};
    // (viewport x, y, w, h, min depth, max depth; target; scissor; cull, ccw, depth clip)
    let states: [([f32; 6], Option<(u32, u32)>, Option<[i32; 4]>, ps::Cull, bool, bool); 7] = [
        ([0.0, 0.0, 64.0, 64.0, 0.0, 1.0], Some((64, 64)), None, ps::Cull::None, false, true),
        ([0.0, 0.0, 640.0, 480.0, 0.0, 1.0], Some((640, 480)), None, ps::Cull::Back, false, true),
        ([10.5, 3.25, 100.0, 61.0, 0.0, 1.0], Some((128, 96)), Some([20, 8, 90, 50]), ps::Cull::Front, true, false),
        ([-30.0, -20.0, 200.0, 150.0, 0.0, 1.0], Some((96, 80)), None, ps::Cull::Back, true, true),
        ([0.0, 0.0, 4096.0, 4096.0, 0.0, 1.0], Some((4096, 4096)), None, ps::Cull::None, false, false),
        // (corners past 4096 pixels: WARP's BigEdgeIterator)
        ([-9000.0, -9000.0, 16384.0, 16384.0, 0.0, 1.0], Some((16384, 16384)), None, ps::Cull::None, false, true),
        // (a viewport wholly outside the target: no pixel can be drawn)
        ([-20000.0, -9000.0, 16384.0, 16384.0, 0.0, 1.0], Some((16384, 16384)), None, ps::Cull::Back, false, true),
    ];
    const N: u32 = 1536;
    let mut out = Vec::new();
    for (k, &(vp, target, scissor, cull, ccw, dc)) in states.iter().enumerate() {
        let r = ps::Raster { viewport: [vp[0], vp[1], vp[2], vp[3]], cull, front_ccw: ccw, depth_clip: dc, rect: ps::rect(&vp, target, scissor) };
        let mut xw = vec![0u32; kome::num::to_usize(XD_VECS).saturating_mul(4)];
        let mut put = |at: usize, v: u32| {
            if let Some(w) = xw.get_mut(at) {
                *w = v;
            }
        };
        for (i, v) in vp.iter().take(4).enumerate() {
            put(xd::VIEWPORT.wrapping_add(i), v.to_bits());
        }
        put(xd::RASTER, cull.word() | u32::from(ccw) << 2 | u32::from(dc) << 3 | 1 << 4);
        for (i, v) in r.rect.iter().enumerate() {
            put(xd::RECT.wrapping_add(i), v.cast_unsigned());
        }
        let mut rng = Rng(0x9e37_79b9 ^ kome::num::u32_of(k).wrapping_mul(0x85eb_ca6b) | 1);
        let (mut prims, mut want) = (Vec::new(), Vec::new());
        for i in 0..N {
            // (a third each points, lines, triangles; a point or line repeated in its unused words)
            let n = i.wrapping_rem(3).wrapping_add(1);
            let p = [rng.pos(), rng.pos(), rng.pos()];
            let used = p.get(..kome::num::to_usize(n)).unwrap_or(&p);
            // (WARP's counter is 32 bits: the page's pairs times 2 in u32)
            want.push(kome::num::lo32(ps::invocations(used, &r)));
            for v in p {
                prims.extend(v.map(f32::to_bits));
            }
            prims.extend([n, 0, 0, 0]);
        }
        let mut m = String::new();
        m.push_str(&format!("@group(0) @binding({B_XD}) var<uniform> xd: array<vec4<u32>, {XD_VECS}>;\n"));
        m.push_str("fn xw(i: u32) -> u32 { return xd[i / 4u][i % 4u]; }\n");
        for (name, w) in xd::NAMED {
            m.push_str(&format!("const XD_{name} = {w}u;\n"));
        }
        m.push_str(ps::WGSL);
        m.push_str(ps::DIV);
        m.push_str(&format!(
            "@group(0) @binding(1) var<storage, read_write> prims: array<vec4<u32>>;\n@group(0) @binding(2) var<storage, read_write> pairs: array<u32>;\n\
             @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) g: vec3<u32>) {{\n{}  let i = g.x;\n  if (i >= {N}u) {{ return; }}\n\
               let p = array<vec4<f32>, 3>(bitcast<vec4<f32>>(prims[i * 4u]), bitcast<vec4<f32>>(prims[i * 4u + 1u]), bitcast<vec4<f32>>(prims[i * 4u + 2u]));\n\
               pairs[i] = ps_pairs(p, prims[i * 4u + 3u].x) * 2u;\n}}\n",
            ps::ENTRY
        ));
        let b = [
            format!(r#"{{"binding":{B_XD},"uniform":[{}]}}"#, list(&xw)),
            format!(r#"{{"binding":1,"buffer":{{"words":[{}]}}}}"#, list(&prims)),
            format!(r#"{{"binding":2,"buffer":{{"words":[{}]}}}}"#, list(&vec![0u32; kome::num::to_usize(N)])),
        ];
        out.push(format!(
            r#"{{"name":{},"dispatch":[{},1,1],"out":[2],"want":[{}],"variants":[{{"name":"masu","wgsl":{}}}],"bindings":[{}]}}"#,
            string(&format!("PSInvocations: ps_pairs (WGSL) = ps::invocations (Rust), state {k}")),
            N.div_ceil(64),
            list(&words_bytes(&want)),
            string(&m),
            b.join(",")
        ));
    }
    out
}

/// The setup pass's WGSL (dxwgsl expand::setup::setup_pass) against its Rust rule
/// (setup::triangle, the one held to Windows' WARP: winref d3d11logic, d3d11interp) on random
/// triangles (inside, across planes, behind the eye, a clip distance below 0) under several
/// rasterizer states: every slot's words the same, bit for bit.
fn setup_cases() -> Result<Vec<String>, String> {
    use dxwgsl::expand::{ps, setup, xd, Store, B_PRIMS, B_VOUT, B_XD, XD_VECS};
    let src = "struct V { float4 p : SV_Position; float4 a : A; float4 b : B; uint2 k : K; float c : SV_ClipDistance0; };\n\
        V vs(uint id : SV_VertexID) { V o = (V)0; o.p = float4(id, 0, 0, 1); return o; }\n\
        float4 ps(float4 p : SV_Position, linear float4 a : A, noperspective float3 b : B, nointerpolation uint2 k : K, float c : SV_ClipDistance0) : SV_Target { return p + a + b.xyzz + k.xyxy + c; }";
    let vs = hlslc::compile(src, &[], &mut |_, _| None, "vs", "vs_5_0", 0).map_err(|e| format!("setup vs: {e}"))?;
    let psc = hlslc::compile(src, &[], &mut |_, _| None, "ps", "ps_5_0", 0).map_err(|e| format!("setup ps: {e}"))?;
    let osgn = dxwgsl::signature(&vs, true);
    let regs = dxwgsl::expand::store_regs(&osgn, 0);
    let layout = setup::Layout::of(&psc)?;
    let stride = layout.stride();
    let reg_of = |name: &str| osgn.iter().find(|e| e.name.eq_ignore_ascii_case(name)).map_or(0, |e| e.reg);
    let (rp, rc) = (reg_of("SV_Position"), reg_of("SV_ClipDistance"));
    let modes: Vec<setup::Mode> = layout.inputs.iter().map(|i| i.mode).collect();
    let states: [([f32; 6], ps::Cull, bool, bool); 4] = [
        ([0.0, 0.0, 64.0, 64.0, 0.0, 1.0], ps::Cull::None, false, true),
        ([0.0, 0.0, 640.0, 480.0, 0.25, 0.75], ps::Cull::Back, false, true),
        ([10.5, 3.25, 100.0, 61.0, 0.0, 1.0], ps::Cull::Front, true, false),
        ([-30.0, -20.0, 200.0, 150.0, 1.0, 0.0], ps::Cull::None, true, true),
    ];
    const N: u32 = 512;
    let mut out = Vec::new();
    for prim in [setup::Prim::Triangle, setup::Prim::Wire, setup::Prim::Line, setup::Prim::Point] {
    let corners_of = prim.corners();
    let store = Store::Vertex { regs, corners: corners_of };
    let wgsl = setup::setup_pass(&store, &osgn, Some(&psc), prim)?;
    let slots = prim.slots(1);
    for (k, &(vp, cull, ccw, dc)) in states.iter().enumerate() {
        let r = setup::Raster { viewport: vp, cull, front_ccw: ccw, depth_clip: dc };
        let mut xw = vec![0u32; kome::num::to_usize(XD_VECS).saturating_mul(4)];
        let mut put = |at: usize, v: u32| {
            if let Some(w) = xw.get_mut(at) {
                *w = v;
            }
        };
        for (i, v) in vp.iter().take(4).enumerate() {
            put(xd::VIEWPORT.wrapping_add(i), v.to_bits());
        }
        put(xd::DEPTH, vp[4].to_bits());
        put(xd::DEPTH.wrapping_add(1), vp[5].to_bits());
        put(xd::RASTER, cull.word() | u32::from(ccw) << 2 | u32::from(dc) << 3 | 1 << 4);
        put(xd::ITEMS, N);
        let mut rng = Rng(0x2545_f491 ^ kome::num::u32_of(k).wrapping_mul(0x9e37_79b9) | 1);
        let mut vout = vec![0u32; kome::num::to_usize(N.saturating_mul(corners_of).saturating_mul(regs).saturating_mul(4))];
        let mut want = vec![0u32; kome::num::to_usize(N.saturating_mul(slots).saturating_mul(stride).saturating_mul(4))];
        for t in 0..N {
            let mut corners: Vec<setup::Corner> = vec![setup::Corner::default(); kome::num::to_usize(corners_of)];
            for (c, corner) in corners.iter_mut().enumerate() {
                let e = t.saturating_mul(corners_of).saturating_add(kome::num::u32_of(c));
                let mut pos = rng.pos();
                // (a third of the triangles well inside: what most draws are)
                if t % 3 == 0 {
                    pos = [rng.f(-0.9, 0.9), rng.f(-0.9, 0.9), rng.f(0.0, 1.0), rng.f(0.5, 2.0)];
                    pos = [pos[0] * pos[3], pos[1] * pos[3], pos[2] * pos[3], pos[3]];
                }
                let dist = if t % 5 == 0 { rng.f(-1.0, 1.0) } else { 1.0 };
                let mut words = vec![0u32; kome::num::to_usize(regs.saturating_mul(4))];
                let mut set = |r: u32, comp: u32, v: u32| {
                    if let Some(w) = words.get_mut(kome::num::to_usize(r.saturating_mul(4).saturating_add(comp))) {
                        *w = v;
                    }
                };
                for (q, v) in (0u32..).zip(pos) {
                    set(rp, q, v.to_bits());
                }
                set(rc, 0, dist.to_bits());
                corner.pos = pos;
                corner.clip = vec![dist];
                for inp in &layout.inputs {
                    let f = osgn.iter().find(|f| f.sysval == if inp.sysval == 2 { 2 } else { 0 } && dxwgsl::signature(&psc, false).iter().any(|e| e.reg == inp.reg && e.name.eq_ignore_ascii_case(&f.name)));
                    for comp in 0..4u32 {
                        let v = if inp.mode == setup::Mode::Constant { rng.next() } else { rng.f(-100.0, 100.0).to_bits() };
                        let v = match f {
                            Some(_) if inp.sysval == 2 => if comp == 0 { dist.to_bits() } else { 0 },
                            Some(f) => {
                                set(f.reg, comp, v);
                                v
                            }
                            None => 0,
                        };
                        corner.attrs.push(v);
                    }
                }
                let at = kome::num::to_usize(e.saturating_mul(regs).saturating_mul(4));
                if let Some(d) = vout.get_mut(at..at.saturating_add(words.len())) {
                    d.copy_from_slice(&words);
                }
            }
            // the slots the model gives the primitive: (corners drawn, rhw and depth planes with
            // their w, the line's words, the inputs' planes)
            type Slot = ([[f32; 4]; 3], setup::Plane, u32, setup::Plane, u32, [f32; 4], Vec<setup::Plane>);
            let of_line = |l: &setup::Line| -> Slot { (setup::line_region(l, &r), l.rhw, l.hi_col.to_bits(), l.z, l.hi_row.to_bits(), [l.x0, l.y0, l.dx, l.dy], l.attrs.clone()) };
            let tri = |c: &[setup::Corner]| -> [setup::Corner; 3] { [c[0].clone(), c[1].clone(), c[2].clone()] };
            let mut made: Vec<(u32, Slot)> = Vec::new();
            match prim {
                setup::Prim::Triangle => {
                    for (f, fan) in (0u32..).zip(setup::triangle(&tri(&corners), &modes, &r)) {
                        made.push((f, (fan.pos, fan.rhw, 0, fan.z, 0, [0.0; 4], fan.attrs)));
                    }
                }
                setup::Prim::Wire => {
                    if !setup::triangle(&tri(&corners), &modes, &r).is_empty() {
                        for (f, (a, b)) in (0u32..).zip([(0, 1), (1, 2), (2, 0)]) {
                            if let Some(l) = setup::line(&[corners[a].clone(), corners[b].clone()], &modes, &r) {
                                made.push((f, of_line(&l)));
                            }
                        }
                    }
                }
                setup::Prim::Line => {
                    if let Some(l) = setup::line(&[corners[0].clone(), corners[1].clone()], &modes, &r) {
                        made.push((0, of_line(&l)));
                    }
                }
                setup::Prim::Point => {
                    if let Some(d) = setup::point(&corners[0], &modes, &r) {
                        let flat = |v: f32| setup::Plane { gx: 0.0, gy: 0.0, c: v };
                        let attrs: Vec<setup::Plane> = (0..d.attrs.len()).map(|k| match modes.get(k / 4) {
                            Some(setup::Mode::Constant) => setup::Plane { gx: 0.0, gy: 0.0, c: f32::from_bits(corners[0].attrs.get(k).copied().unwrap_or(0)) },
                            _ => flat(d.attrs.get(k).copied().unwrap_or(0.0)),
                        }).collect();
                        for (f, sq) in (0u32..).zip(setup::dot_squares(&d, &r)) {
                            made.push((f, (sq, flat(d.rhw), 0, flat(d.z), 0, [0.0; 4], attrs.clone())));
                        }
                    }
                }
            }
            for (f, (pos, rhw, rw, z, zw, lw, attrs)) in made {
                let base = kome::num::to_usize(t.saturating_mul(slots).saturating_add(f).saturating_mul(stride).saturating_mul(4));
                let mut put = |vec: u32, comp: u32, v: u32| {
                    if let Some(w) = want.get_mut(base.saturating_add(kome::num::to_usize(vec.saturating_mul(4).saturating_add(comp)))) {
                        *w = v;
                    }
                };
                for (c, p) in (0u32..).zip(pos) {
                    for (q, v) in (0u32..).zip(p) {
                        put(c, q, v.to_bits());
                    }
                }
                for (vec, pl, w) in [(3u32, rhw, rw), (4, z, zw)] {
                    put(vec, 0, pl.gx.to_bits());
                    put(vec, 1, pl.gy.to_bits());
                    put(vec, 2, pl.c.to_bits());
                    put(vec, 3, w);
                }
                for (q, v) in (0u32..).zip(lw) {
                    put(5, q, v.to_bits());
                }
                for (i, inp) in (0u32..).zip(&layout.inputs) {
                    for comp in 0..4u32 {
                        if inp.mask >> comp & 1 == 0 {
                            continue;
                        }
                        let pl = attrs.get(kome::num::to_usize(i.saturating_mul(4).saturating_add(comp))).copied().unwrap_or_default();
                        let o = setup::HEADER.saturating_add(i.saturating_mul(3));
                        put(o, comp, pl.gx.to_bits());
                        put(o.saturating_add(1), comp, pl.gy.to_bits());
                        put(o.saturating_add(2), comp, pl.c.to_bits());
                    }
                }
            }
        }
        let prims: Vec<u32> = (0..N.saturating_mul(corners_of)).collect();
        let b = [
            format!(r#"{{"binding":{B_XD},"uniform":[{}]}}"#, list(&xw)),
            format!(r#"{{"binding":{B_VOUT},"buffer":{{"words":[{}]}}}}"#, list(&vout)),
            format!(r#"{{"binding":{B_PRIMS},"buffer":{{"words":[{}]}}}}"#, list(&prims)),
            format!(r#"{{"binding":{},"buffer":{{"words":[{}]}}}}"#, setup::B_SETUP, list(&vec![0u32; want.len()])),
        ];
        out.push(format!(
            r#"{{"name":{},"dispatch":[{},1,1],"out":[{}],"want":[{}],"variants":[{{"name":"masu","wgsl":{}}}],"bindings":[{}]}}"#,
            string(&format!("setup pass (WGSL) = the setup model (Rust), {prim:?}, state {k}")),
            N.div_ceil(64),
            setup::B_SETUP,
            list(&words_bytes(&want)),
            string(&wgsl.code),
            b.join(",")
        ));
    }
    }
    Ok(out)
}
