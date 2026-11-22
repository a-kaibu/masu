//! The SNORM conversions and blend: against WARP's answers (winref d3d11snorm's golden output,
//! every row of it), their properties, and their WGSL (naga reads and validates it).
use proptest::prelude::*;

use super::*;

const FACTORS: [Factor; 17] = [
    Factor::Zero,
    Factor::One,
    Factor::SrcColor,
    Factor::InvSrcColor,
    Factor::SrcAlpha,
    Factor::InvSrcAlpha,
    Factor::DestAlpha,
    Factor::InvDestAlpha,
    Factor::DestColor,
    Factor::InvDestColor,
    Factor::SrcAlphaSat,
    Factor::BlendFactor,
    Factor::InvBlendFactor,
    Factor::Src1Color,
    Factor::InvSrc1Color,
    Factor::Src1Alpha,
    Factor::InvSrc1Alpha,
];
const OPS: [BlendOp; 5] = [BlendOp::Add, BlendOp::Subtract, BlendOp::RevSubtract, BlendOp::Min, BlendOp::Max];

/// D3D11_BLEND's values in d3d11.h order (12 and 13 none)
#[test]
fn factors_are_d3d11s() {
    let values = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 14, 15, 16, 17, 18, 19];
    for (f, v) in FACTORS.iter().zip(values) {
        assert_eq!(Factor::of(v), Some(*f));
        assert_eq!(f.word(), v);
    }
    for v in [0, 12, 13, 20, u32::MAX] {
        assert_eq!(Factor::of(v), None);
    }
    for (i, o) in OPS.iter().enumerate() {
        assert_eq!(BlendOp::of(u32::try_from(i).unwrap() + 1), Some(*o));
        assert_eq!(o.word(), u32::try_from(i).unwrap() + 1);
    }
    assert_eq!(BlendOp::of(0), None);
    assert_eq!(BlendOp::of(6), None);
}

/// what each factor reads, by its value: a factor that reads the destination changes with it,
/// one that does not stays (so the draw reads the target's copy exactly when it must)
#[test]
fn factors_read_what_they_say() {
    let (s, s1, k) = ([0.25, -0.5, 0.75, 0.5], [0.1, 0.2, -0.3, 0.4], [0.3, -0.6, 0.9, 0.2]);
    let (d0, d1) = ([0.0; 4], [0.5, -0.25, 1.0, -0.75]);
    for f in FACTORS {
        for c in 0..4 {
            let dst_changes = f.value(c, s, s1, d0, k) != f.value(c, s, s1, d1, k);
            let src1_changes = f.value(c, s, s1, d0, k) != f.value(c, s, [0.0; 4], d0, k);
            let k_changes = f.value(c, s, s1, d0, k) != f.value(c, s, s1, d0, [0.0; 4]);
            if dst_changes {
                assert!(f.reads_dst(), "{f:?} channel {c}");
            }
            if src1_changes {
                assert!(f.reads_src1(), "{f:?} channel {c}");
            }
            if k_changes {
                assert!(f.reads_constant(), "{f:?} channel {c}");
            }
        }
    }
}

proptest! {
    /// a store is the nearest of the 255 values (-127..=127) to the clamped value (of the f32
    /// product), halves away from zero; NaN -127; a load of it the value again
    #[test]
    fn store_is_nearest(bits in any::<u32>()) {
        let v = f32::from_bits(bits);
        let b = store(v);
        prop_assert!(b >= -127);
        if v.is_nan() {
            prop_assert_eq!(b, -127);
        } else {
            let x = f64::from(v.clamp(-1.0, 1.0) * 127.0);
            let e = (f64::from(b) - x).abs();
            prop_assert!(e < 0.5 || (e == 0.5 && f64::from(b).abs() > x.abs()), "{} -> {}", v, b);
        }
        prop_assert_eq!(store(load(b)), b);
    }

    /// the shader's hidden zero (snorm::helpers' sn_r): bit 30 of every clamped value is 0, so
    /// or-ing it into a product's bits leaves the product
    #[test]
    fn clamped_values_have_no_bit_30(bits in any::<u32>()) {
        prop_assert_eq!(clamp_src(f32::from_bits(bits)).to_bits() & 0x4000_0000, 0);
    }

    /// a blend that reads no destination gives the same bytes over any destination, one with
    /// ONE ZERO ADD everywhere the source's store
    #[test]
    fn blend_reads_dst_only_when_it_says(sc in 0usize..17, dc in 0usize..17, oc in 0usize..5, sa in 0usize..17, da in 0usize..17, oa in 0usize..5, s in any::<[f32; 4]>(), s1 in any::<[f32; 4]>(), k in any::<[f32; 4]>(), d0 in any::<[i8; 4]>(), d1 in any::<[i8; 4]>()) {
        let b = Blend { color: Equation { src: FACTORS[sc], dst: FACTORS[dc], op: OPS[oc] }, alpha: Equation { src: FACTORS[sa], dst: FACTORS[da], op: OPS[oa] } };
        if !b.reads_dst() {
            prop_assert_eq!(blend(&b, s, s1, d0, k), blend(&b, s, s1, d1, k));
        }
        if !b.reads_src1() {
            prop_assert_eq!(blend(&b, s, s1, d0, k), blend(&b, s, [0.0; 4], d0, k));
        }
        if !b.reads_constant() {
            prop_assert_eq!(blend(&b, s, s1, d0, k), blend(&b, s, s1, d0, [0.0; 4]));
        }
        let copy = Blend { color: Equation { src: Factor::One, dst: Factor::Zero, op: BlendOp::Add }, alpha: Equation { src: Factor::One, dst: Factor::Zero, op: BlendOp::Add } };
        prop_assert_eq!(blend(&copy, s, s1, d0, k), s.map(store));
    }

    /// every SNORM stash's resolve pass parses and validates
    #[test]
    fn resolve_wgsl_validates(sc in 0usize..17, dc in 0usize..17, oc in 0usize..5, sa in 0usize..17, da in 0usize..17, oa in 0usize..5, channel in 0u32..4, bit in 0u32..32, wide in any::<bool>()) {
        let b = Blend { color: Equation { src: FACTORS[sc], dst: FACTORS[dc], op: OPS[oc] }, alpha: Equation { src: FACTORS[sa], dst: FACTORS[da], op: OPS[oa] } };
        let src = resolve_wgsl(width(wide), &b, channel, bit);
        let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{b:?}: {}\n{src}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{b:?}: {e:?}\n{src}"));
    }

    /// a resolve of samples all one byte is that byte
    #[test]
    fn resolve_of_equal_samples(b in any::<i8>(), n in 1usize..9) {
        let b = b.max(-127);
        prop_assert_eq!(resolve(&vec![b; n]), b);
    }

    /// a resolve of samples all one 16-bit value is that value
    #[test]
    fn resolve16_of_equal_samples(q in -32767i32..=32767, n in 1usize..9) {
        prop_assert_eq!(resolve_in(Width::Bits16, &vec![q; n]), q);
    }

    /// a 16-bit store: within -32767..=32767, NaN -32767, the nearest value of the clamped
    /// product (halves away from zero), and a load stored back the same value
    #[test]
    fn store16_nearest(v in any::<f32>(), q in -32768i32..=32767) {
        let x = store_in(Width::Bits16, v);
        prop_assert!((-32767..=32767).contains(&x));
        if v.is_nan() {
            prop_assert_eq!(x, -32767);
        } else {
            let p = f64::from(v.clamp(-1.0, 1.0) * 32767.0);
            let e = (f64::from(x) - p).abs();
            prop_assert!(e < 0.5 || (e == 0.5 && f64::from(x).abs() > p.abs()), "{} -> {}", v, x);
        }
        prop_assert_eq!(store_in(Width::Bits16, load_in(Width::Bits16, q)), q.max(-32767));
    }

    /// a 16-bit blend that reads no destination gives the same values over any destination
    #[test]
    fn blend16_reads_dst_only_when_it_says(sc in 0usize..17, dc in 0usize..17, oc in 0usize..5, s in any::<[f32; 4]>(), s1 in any::<[f32; 4]>(), k in any::<[f32; 4]>(), d0 in any::<[i16; 4]>(), d1 in any::<[i16; 4]>()) {
        let e = Equation { src: FACTORS[sc], dst: FACTORS[dc], op: OPS[oc] };
        let b = Blend { color: e, alpha: e };
        let (d0, d1) = (d0.map(i32::from), d1.map(i32::from));
        if !b.reads_dst() {
            prop_assert_eq!(blend_in(Width::Bits16, &b, s, s1, d0, k), blend_in(Width::Bits16, &b, s, s1, d1, k));
        }
    }

    /// every blend's WGSL parses and validates
    #[test]
    fn blend_wgsl_validates(sc in 0usize..17, dc in 0usize..17, oc in 0usize..5, sa in 0usize..17, da in 0usize..17, oa in 0usize..5, wide in any::<bool>()) {
        let b = Blend { color: Equation { src: FACTORS[sc], dst: FACTORS[dc], op: OPS[oc] }, alpha: Equation { src: FACTORS[sa], dst: FACTORS[da], op: OPS[oa] } };
        let body = blend_wgsl(width(wide), &b, "s", "s1", "d", "o");
        let src = format!("{}fn f(s: vec4<f32>, s1: vec4<f32>, d: vec4<i32>, sn_k: vec4<f32>) -> vec4<i32> {{ var o: vec4<i32>; {} return o; }}\n", helpers(), body.join(" "));
        let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{b:?}: {}\n{src}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{b:?}: {e:?}\n{src}"));
    }
}

/// the multisampled resolve's pass parses and validates
#[test]
fn ms_resolve_wgsl_validates() {
    for w in [Width::Bits8, Width::Bits16] {
    let src = ms_resolve_wgsl(w);
    let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}\n{src}", e.emit_to_string(&src)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
    }
}

fn width(wide: bool) -> Width {
    if wide { Width::Bits16 } else { Width::Bits8 }
}

#[test]
fn unorm8_floats_round_back() {
    for b in 0u8..=255 {
        assert_eq!((f32::from(b) / 255.0 * 255.0).round(), f32::from(b), "{b}");
    }
}

/// a view's load of the page's texels validates, for each source, width and channel count
#[test]
fn view_load_wgsl_validates() {
    for source in [Source::Sint, Source::Unorm8] {
        for width in [Width::Bits8, Width::Bits16] {
            for channels in [1, 2, 4] {
                let v = View { slot: 0, channels, width, source };
                let src = format!("{}@group(0) @binding(0) var t: {};\nfn f(q: vec2<i32>) -> vec4<f32> {{ return {}; }}\n", helpers(), v.texture_type(), v.load_wgsl("textureLoad(t, q, 0)"));
                let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{v:?}: {}\n{src}", e.emit_to_string(&src)));
                naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{v:?}: {e:?}\n{src}"));
            }
        }
    }
}

/// One row of winref d3d11snorm's golden output.
fn golden(name: &str) -> Option<String> {
    let p = format!("{}/../../tests/winref/golden/{name}/stdout.txt", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(p).ok()
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s, 16).unwrap()
}

fn bytes(w: u32) -> [i8; 4] {
    w.to_le_bytes().map(u8::cast_signed)
}

fn word(b: [i8; 4]) -> u32 {
    u32::from_le_bytes(b.map(i8::cast_unsigned))
}

/// Every answer WARP gave in winref d3d11snorm (both builds): the sweeps' bytes, the clears,
/// and every blend of every factor pair and op over the three sets.
#[test]
fn warp_answers() {
    let mut checked = 0;
    for name in ["d3d11snorm-32", "d3d11snorm-64"] {
        let Some(out) = golden(name) else { continue };
        let mut set: Option<([f32; 4], [f32; 4], u32)> = None;
        let mut alpha = false;
        let k = [0.3f32, -0.5, 1.5, 0.75];
        for line in out.lines() {
            let t: Vec<&str> = line.split_whitespace().collect();
            match t.as_slice() {
                // a sweep: format, first bits, step, then the bytes
                [f @ ("r8" | "rg8" | "rgba8"), from, step, row] => {
                    let ch = match *f {
                        "r8" => 1,
                        "rg8" => 2,
                        _ => 4,
                    };
                    let (from, step) = (hex(from.trim_end_matches(':')), hex(step.trim_end_matches(':')));
                    for x in 0..256u32 {
                        let v = f32::from_bits(from.wrapping_add(x.wrapping_mul(step)));
                        for c in 0..ch {
                            let at = usize::try_from(x * ch + c).unwrap() * 2;
                            let got = u8::from_str_radix(&row[at..at + 2], 16).unwrap().cast_signed();
                            assert_eq!(store(v), got, "{name}: {f} {v:e} ({:08x})", v.to_bits());
                            checked += 1;
                        }
                    }
                }
                // the resolve of 4 samples a pixel each its own bytes (ps_samples): pixel p's
                // sample k of channel c is ((p * 4 + k) * K[c] + seed) % 255 - 127
                ["seed", seed, words @ ..] => {
                    let seed: u32 = seed.trim_end_matches(':').parse().unwrap();
                    let ks = [37u32, 53, 71, 97];
                    for (p, w) in (0u32..).zip(words) {
                        let want: [i8; 4] = std::array::from_fn(|c| {
                            let samples: Vec<i8> = (0..4u32).map(|k| i8::try_from(i32::try_from(((p * 4 + k) * ks[c] + seed) % 255).unwrap() - 127).unwrap()).collect();
                            resolve(&samples)
                        });
                        assert_eq!(word(want), hex(w), "{name}: seed {seed} pixel {p}");
                        checked += 1;
                    }
                }
                // the MIN / MAX table: CreateBlendState's answer for each factor pair (the other
                // equation one zero add)
                [op @ ("min" | "max"), side @ ("color" | "alpha"), src, row] if src.ends_with(':') => {
                    let op = OPS[OP_NAMES.iter().position(|o| o == op).unwrap()];
                    let alpha = *side == "alpha";
                    let names: &[&str] = if alpha { &ALPHA_NAMES } else { &COLOR_NAMES };
                    let at = |n: &str| FACTORS[COLOR_NAMES.iter().position(|c| *c == n).unwrap()];
                    let sf = at(src.trim_end_matches(':'));
                    assert_eq!(row.len(), names.len(), "{name}: {line}");
                    for (n, answer) in names.iter().zip(row.chars()) {
                        let e = Equation { src: sf, dst: at(n), op };
                        let plain = Equation { src: Factor::One, dst: Factor::Zero, op: BlendOp::Add };
                        let b = if alpha { Blend { color: plain, alpha: e } } else { Blend { color: e, alpha: plain } };
                        assert_eq!(b.takes(), answer == 'o', "{name}: {op:?} {side} {sf:?} {n}");
                        checked += 1;
                    }
                }
                // NaN and infinite sources (bits) blended over 3081c040 with the probe's nine
                // blends (D3D11_BLEND and D3D11_BLEND_OP values, alpha the color's alpha twin)
                [s0, s1, s2, s3, "/", t0, t1, t2, t3, words @ ..] if t3.ends_with(':') => {
                    let s = [s0, s1, s2, s3].map(|w| f32::from_bits(hex(w)));
                    let s1 = [t0, t1, t2, t3.trim_end_matches(':')].map(|w| f32::from_bits(hex(w)));
                    let blends = [[2, 1, 1], [2, 2, 1], [5, 6, 1], [9, 1, 1], [16, 1, 1], [1, 18, 1], [2, 2, 2], [2, 1, 4], [2, 1, 5]];
                    let alpha_of = |f: u32| match f {
                        3 => 5,
                        4 => 6,
                        9 => 7,
                        10 => 8,
                        16 => 18,
                        17 => 19,
                        f => f,
                    };
                    assert_eq!(words.len(), blends.len(), "{name}: {line}");
                    for ([a, b, o], w) in blends.into_iter().zip(words) {
                        let bl = Blend::of([a, b, o, alpha_of(a), alpha_of(b), o]).unwrap();
                        assert_eq!(word(blend(&bl, s, s1, bytes(0x3081_c040), k)), hex(w), "{name}: {s:?} {s1:?} {a} {b} {o}");
                        checked += 1;
                    }
                }
                ["blend", "set", _, "src", r @ ..] => {
                    let f = |i: usize| r[i].parse::<f32>().unwrap();
                    set = Some(([f(0), f(1), f(2), f(3)], [f(5), f(6), f(7), f(8)], hex(r[10])));
                }
                ["color", ..] => alpha = false,
                ["alpha", ..] => alpha = true,
                [op, src, words @ ..] if set.is_some() && OP_NAMES.contains(op) && src.ends_with(':') => {
                    let (s, s1, d) = set.unwrap();
                    let op = OPS[OP_NAMES.iter().position(|o| o == op).unwrap()];
                    let names: &[&str] = if alpha { &ALPHA_NAMES } else { &COLOR_NAMES };
                    let factors: Vec<Factor> = names.iter().map(|n| FACTORS[COLOR_NAMES.iter().position(|c| c == n).unwrap()]).collect();
                    let sf = factors[names.iter().position(|n| *n == src.trim_end_matches(':')).unwrap()];
                    // (min and max: one row, their factors ignored; an entry WARP refused:
                    // "CreateBlendState failed <hr> <what the target held>")
                    let mut words = words.iter();
                    for df in factors.iter().copied() {
                        let e = Equation { src: sf, dst: df, op };
                        let plain = Equation { src: Factor::One, dst: Factor::Zero, op: BlendOp::Add };
                        let b = if alpha { Blend { color: plain, alpha: e } } else { Blend { color: e, alpha: plain } };
                        let what = format!("{name}: {} {op:?} {sf:?} {df:?} over {d:08x}", if alpha { "alpha" } else { "color" });
                        let w = words.next().unwrap_or_else(|| panic!("{what}: no answer"));
                        if *w == "CreateBlendState" {
                            assert!(!b.takes(), "{what}: WARP refuses it");
                            words.nth(2);
                        } else {
                            assert!(b.takes(), "{what}: WARP takes it");
                            assert_eq!(word(blend(&b, s, s1, bytes(d), k)), hex(w), "{what}");
                        }
                        checked += 1;
                    }
                }
                _ => {}
            }
        }
    }
    eprintln!("{checked} answers checked");
}

const OP_NAMES: [&str; 5] = ["add", "subtract", "rev_subtract", "min", "max"];
const COLOR_NAMES: [&str; 17] = ["zero", "one", "src_color", "inv_src_color", "src_alpha", "inv_src_alpha", "dest_alpha", "inv_dest_alpha", "dest_color", "inv_dest_color", "src_alpha_sat", "blend_factor", "inv_blend_factor", "src1_color", "inv_src1_color", "src1_alpha", "inv_src1_alpha"];
const ALPHA_NAMES: [&str; 11] = ["zero", "one", "src_alpha", "inv_src_alpha", "dest_alpha", "inv_dest_alpha", "src_alpha_sat", "blend_factor", "inv_blend_factor", "src1_alpha", "inv_src1_alpha"];

/// A 64-bit word of four 16-bit channels (channel 0 the low bits) as values.
fn words16(w: u64) -> [i32; 4] {
    std::array::from_fn(|c| i32::from(u16::try_from((w >> (16 * c)) & 0xffff).unwrap().cast_signed()))
}

fn word16(q: [i32; 4]) -> u64 {
    q.iter().rev().fold(0u64, |a, &v| (a << 16) | u64::from(i16::try_from(v).unwrap().cast_unsigned()))
}

/// The four channel floats of a sample ("a.b.c.d", float bits).
fn sample(s: &str) -> [u32; 4] {
    let v: Vec<u32> = s.split('.').map(hex).collect();
    v.try_into().unwrap()
}

/// What the probe's ps_samples writes to sample `k` of pixel `p` (channel `c`): the integer
/// ((p * 4 + k) * K + seed) % 65535 - 32767 over 32767 (an f32 division).
fn ps_samples(p: u32, k: u32, c: usize, seed: u32) -> f32 {
    let ks = [9973u32, 7919, 6151, 4099];
    let b = i32::try_from(((p * 4 + k).wrapping_mul(ks[c]).wrapping_add(seed)) % 65535).unwrap() - 32767;
    kome::num::to_f32(b) / 32767.0
}

/// The UNORM byte of value `v` as the output merger converts it (clamped, times 255, ties to
/// even).
fn unorm8(v: f32) -> u8 {
    u8::try_from(kome::num::trunc_i32((v.clamp(0.0, 1.0) * 255.0).round_ties_even())).unwrap()
}

/// Every answer WARP gave in winref d3d11snorm2 (both builds): the 16-bit sweeps, Load of every
/// line of 16-bit words, every blend of every factor pair and op over the three sets, the NaN
/// and infinite sources, ClearView's rects (one sample and four, 8 and 16 bits), the 4x
/// R16G16B16A16_SNORM and R8G8B8A8_TYPELESS (SNORM views) half-covered blends, their resolves,
/// the resolves of samples each their own, and the typeless texture through UNORM views.
#[test]
fn warp_answers16() {
    let mut checked = 0;
    let k = [0.3f32, -0.5, 1.5, 0.75];
    let src0 = [0.25f32, -0.5, 0.75, 0.6];
    let dc = [64.0f32 / 127.0, -64.0 / 127.0, -1.0, 48.0 / 127.0];
    let cvc = [0.5f32, -0.5, 0.25, -1.0];
    // (left, top, right, bottom)
    let rects = [(1u32, 1u32, 3u32, 3u32), (3, 0, 4, 2)];
    let in_rects = |x: u32, y: u32| rects.iter().any(|&(l, t, r, b)| x >= l && x < r && y >= t && y < b);
    let half = Blend::of([5, 6, 1, 5, 6, 1]).unwrap();
    for name in ["d3d11snorm2-32", "d3d11snorm2-64"] {
        let Some(out) = golden(name) else { continue };
        let mut set: Option<([f32; 4], [f32; 4], u64)> = None;
        let mut alpha = false;
        // the multisampled section: its width, whether it is a ClearView, the rows' samples
        // (pixel-major, 4 a pixel), and the covered samples of the typeless half blend
        #[derive(Clone, Copy, PartialEq)]
        enum Ms {
            None,
            Half(Width),
            Clear(Width),
            Unorm,
        }
        let mut ms = Ms::None;
        let mut rows: Vec<[u32; 4]> = Vec::new();
        let mut covered: Vec<bool> = Vec::new();
        let mut unorm_rows = 0;
        let clear_q = |w: Width, v: [f32; 4]| v.map(|x| store_in(w, x));
        for line in out.lines() {
            let t: Vec<&str> = line.split_whitespace().collect();
            if !line.starts_with(' ') {
                ms = Ms::None;
                set = if line.starts_with("blend16 set") { set } else { None };
            }
            if line.starts_with("4x multisampled R16G16B16A16_SNORM") {
                ms = Ms::Half(Width::Bits16);
                rows.clear();
            } else if line.starts_with("4x multisampled R8G8B8A8_SNORM: ClearView") {
                ms = Ms::Clear(Width::Bits8);
                rows.clear();
            } else if line.starts_with("4x multisampled R8G8B8A8_TYPELESS") {
                ms = Ms::Half(Width::Bits8);
                rows.clear();
                covered.clear();
            } else if line.starts_with("  ClearView (after") {
                ms = match ms {
                    Ms::Half(w) | Ms::Clear(w) => Ms::Clear(w),
                    m => m,
                };
                rows.clear();
            } else if line.starts_with("  through R8G8B8A8_UNORM views") {
                ms = Ms::Unorm;
                rows.clear();
                unorm_rows = 0;
            }
            match t.as_slice() {
                [f @ ("r16" | "rg16" | "rgba16"), from, step, row] => {
                    let ch = match *f {
                        "r16" => 1,
                        "rg16" => 2,
                        _ => 4,
                    };
                    let (from, step) = (hex(from.trim_end_matches(':')), hex(step.trim_end_matches(':')));
                    assert_eq!(row.len(), 256 * ch * 4, "{name}: {f} {from:08x}");
                    for x in 0..256u32 {
                        let v = f32::from_bits(from.wrapping_add(x.wrapping_mul(step)));
                        for c in 0..ch {
                            let at = (usize::try_from(x).unwrap() * ch + c) * 4;
                            let w = u16::from_le_bytes([u8::from_str_radix(&row[at..at + 2], 16).unwrap(), u8::from_str_radix(&row[at + 2..at + 4], 16).unwrap()]);
                            assert_eq!(store_in(Width::Bits16, v), i32::from(w.cast_signed()), "{name}: {f} {v:e} ({:08x})", v.to_bits());
                            checked += 1;
                        }
                    }
                }
                ["load:", pairs @ ..] => {
                    for p in pairs {
                        let (w, v) = p.split_once('=').unwrap();
                        let q = i32::from(u16::from_str_radix(w, 16).unwrap().cast_signed());
                        assert_eq!(load_in(Width::Bits16, q).to_bits(), hex(v), "{name}: load {w}");
                        checked += 1;
                    }
                }
                ["blend16", "set", _, "src", r @ ..] => {
                    let f = |i: usize| r[i].parse::<f32>().unwrap();
                    set = Some(([f(0), f(1), f(2), f(3)], [f(5), f(6), f(7), f(8)], u64::from_str_radix(r[10], 16).unwrap()));
                }
                ["color", ..] => alpha = false,
                ["alpha", ..] => alpha = true,
                [op, src, words @ ..] if set.is_some() && OP_NAMES.contains(op) && src.ends_with(':') => {
                    let (s, s1, d) = set.unwrap();
                    let op = OPS[OP_NAMES.iter().position(|o| o == op).unwrap()];
                    let names: &[&str] = if alpha { &ALPHA_NAMES } else { &COLOR_NAMES };
                    let factors: Vec<Factor> = names.iter().map(|n| FACTORS[COLOR_NAMES.iter().position(|c| c == n).unwrap()]).collect();
                    let sf = factors[names.iter().position(|n| *n == src.trim_end_matches(':')).unwrap()];
                    let mut words = words.iter();
                    for df in factors.iter().copied() {
                        let e = Equation { src: sf, dst: df, op };
                        let plain = Equation { src: Factor::One, dst: Factor::Zero, op: BlendOp::Add };
                        let b = if alpha { Blend { color: plain, alpha: e } } else { Blend { color: e, alpha: plain } };
                        let what = format!("{name}: 16-bit {} {op:?} {sf:?} {df:?} over {d:016x}", if alpha { "alpha" } else { "color" });
                        let w = words.next().unwrap_or_else(|| panic!("{what}: no answer"));
                        if *w == "CreateBlendState" {
                            assert!(!b.takes(), "{what}: WARP refuses it");
                            words.nth(2);
                        } else {
                            assert!(b.takes(), "{what}: WARP takes it");
                            assert_eq!(word16(blend_in(Width::Bits16, &b, s, s1, words16(d), k)), u64::from_str_radix(w, 16).unwrap(), "{what}");
                        }
                        checked += 1;
                    }
                }
                // NaN and infinite sources over set 0's destination
                [s0, s1, s2, s3, "/", t0, t1, t2, t3, words @ ..] if t3.ends_with(':') => {
                    let s = [s0, s1, s2, s3].map(|w| f32::from_bits(hex(w)));
                    let s1 = [t0, t1, t2, t3.trim_end_matches(':')].map(|w| f32::from_bits(hex(w)));
                    let blends = [[2, 1, 1], [2, 2, 1], [5, 6, 1], [9, 1, 1], [16, 1, 1], [1, 18, 1], [2, 2, 2], [2, 1, 4], [2, 1, 5]];
                    let alpha_of = |f: u32| match f {
                        3 => 5,
                        4 => 6,
                        9 => 7,
                        10 => 8,
                        16 => 18,
                        17 => 19,
                        f => f,
                    };
                    assert_eq!(words.len(), blends.len(), "{name}: {line}");
                    for ([a, b, o], w) in blends.into_iter().zip(words) {
                        let bl = Blend::of([a, b, o, alpha_of(a), alpha_of(b), o]).unwrap();
                        let d = words16(0x6000_c001_8000_1234);
                        assert_eq!(word16(blend_in(Width::Bits16, &bl, s, s1, d, k)), u64::from_str_radix(w, 16).unwrap(), "{name}: 16-bit {s:?} {s1:?} {a} {b} {o}");
                        checked += 1;
                    }
                }
                // ClearView of a 4x4 R8G8B8A8_SNORM target (one sample): the rects' texels
                // the view's color, the rest ClearRenderTargetView's
                texels if texels.len() == 16 && texels.iter().all(|w| w.len() == 8) && ms == Ms::None => {
                    for (i, w) in (0u32..).zip(texels) {
                        let v = if in_rects(i % 4, i / 4) { cvc } else { [0.25, -0.75, 1.0, -0.125] };
                        assert_eq!(word(clear_q(Width::Bits8, v).map(|q| i8::try_from(q).unwrap())), hex(w), "{name}: ClearView texel {i}");
                        checked += 1;
                    }
                }
                [row, _, samples @ ..] if *row == "row" => {
                    assert_eq!(samples.len(), 16, "{name}: {line}");
                    rows.extend(samples.iter().map(|s| sample(s)));
                    let y = u32::try_from(rows.len() / 16 - 1).unwrap() % 4;
                    for (x, s) in (0u32..).zip(samples) {
                        let got = sample(s);
                        let px = x / 4;
                        let what = format!("{name}: {line:.40} pixel ({px},{y}) sample {}", x % 4);
                        match ms {
                            Ms::Clear(w) => {
                                let want = clear_q(w, if in_rects(px, y) { cvc } else { dc }).map(|q| load_in(w, q).to_bits());
                                assert_eq!(got, want, "{what}");
                            }
                            Ms::Half(w) => {
                                let c = clear_q(w, dc);
                                let cleared = c.map(|q| load_in(w, q).to_bits());
                                let blended = blend_in(w, &half, src0, [0.0; 4], c, k).map(|q| load_in(w, q).to_bits());
                                assert!(got == cleared || got == blended, "{what}: {got:08x?} neither {cleared:08x?} nor {blended:08x?}");
                                if w == Width::Bits8 {
                                    covered.push(got == blended);
                                }
                            }
                            Ms::Unorm => {
                                // each sample's bytes: the SNORM half blend's (or the clear's),
                                // one one add of 0.25 0.5 0.75 1 as UNORM; read as UNORM, then
                                // (the second four rows) as SNORM
                                let at = usize::try_from(y * 16 + x).unwrap();
                                let c = clear_q(Width::Bits8, dc);
                                let q = if covered[at] { blend_in(Width::Bits8, &half, src0, [0.0; 4], c, k) } else { c };
                                let add = [0.25f32, 0.5, 0.75, 1.0];
                                let u: [u8; 4] = std::array::from_fn(|ch| unorm8(f32::from(i8::try_from(q[ch]).unwrap().cast_unsigned()) / 255.0 + add[ch]));
                                let want = if unorm_rows < 4 { u.map(|b| (f32::from(b) / 255.0).to_bits()) } else { u.map(|b| load(b.cast_signed()).to_bits()) };
                                assert_eq!(got, want, "{what}");
                            }
                            Ms::None => panic!("{name}: a row outside a section: {line:.60}"),
                        }
                        checked += 1;
                    }
                    if ms == Ms::Unorm {
                        unorm_rows += 1;
                    }
                }
                ["resolved:", words @ ..] => {
                    let w = match ms {
                        Ms::Half(w) | Ms::Clear(w) => w,
                        _ => panic!("{name}: {line:.60}"),
                    };
                    assert_eq!(words.len(), 16, "{name}: {line:.60}");
                    for (p, got) in words.iter().enumerate() {
                        let y = p / 4;
                        let samples: Vec<[u32; 4]> = (0..4).map(|k| rows[y * 16 + (p % 4) * 4 + k]).collect();
                        let want: [i32; 4] = std::array::from_fn(|c| {
                            let q: Vec<i32> = samples.iter().map(|s| store_in(w, f32::from_bits(s[c]))).collect();
                            resolve_in(w, &q)
                        });
                        match w {
                            Width::Bits16 => assert_eq!(word16(want), u64::from_str_radix(got, 16).unwrap(), "{name}: resolved pixel {p}"),
                            Width::Bits8 => assert_eq!(word(want.map(|q| i8::try_from(q).unwrap())), hex(got), "{name}: resolved pixel {p}"),
                        }
                        checked += 1;
                    }
                }
                [tag @ ("seed16" | "seed"), seed, words @ ..] => {
                    let w = if *tag == "seed16" { Width::Bits16 } else { Width::Bits8 };
                    let seed: u32 = seed.trim_end_matches(':').parse().unwrap();
                    assert_eq!(words.len(), 16, "{name}: {line:.60}");
                    for (p, got) in (0u32..).zip(words) {
                        let want: [i32; 4] = std::array::from_fn(|c| {
                            let q: Vec<i32> = (0..4).map(|k| store_in(w, ps_samples(p, k, c, seed))).collect();
                            resolve_in(w, &q)
                        });
                        match w {
                            Width::Bits16 => assert_eq!(word16(want), u64::from_str_radix(got, 16).unwrap(), "{name}: {tag} {seed} pixel {p}"),
                            Width::Bits8 => assert_eq!(word(want.map(|q| i8::try_from(q).unwrap())), hex(got), "{name}: {tag} {seed} pixel {p}"),
                        }
                        checked += 1;
                    }
                }
                _ => {}
            }
        }
        assert!(covered.iter().any(|&c| c) && covered.iter().any(|&c| !c), "{name}: the half triangle covers some samples, not all");
    }
    eprintln!("{checked} answers checked");
}
