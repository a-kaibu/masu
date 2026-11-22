//! The texel a clear writes: ClearUnorderedAccessViewUint / ClearUnorderedAccessViewFloat and
//! ID3D11DeviceContext1::ClearView, in the view's DXGI format as the guest would have written
//! it (the upload path then makes the page's texel of it, as for UpdateSubresource).
//!
//! The rules are the documented ones (D3D11 functional spec; ID3D11DeviceContext methods):
//! - Uint: each channel gets the lower bits of its array element, no conversion (a FLOAT
//!   channel gets the bits as they are).
//! - Float: FLOAT, UNORM and SNORM channels get the value converted (UNORM clamped to 0..1,
//!   SNORM to -1..1, times 2^n - 1 (2^(n-1) - 1), rounded to the nearest even, NaN 0; an sRGB
//!   format's color through the linear-to-sRGB curve first); on an integer format a Float
//!   clear does nothing (None).
//! - SNORM, as Windows does it (winref d3d11snorm): the product in f32, its halves away from
//!   zero (2.5 / 127 is 3, not 2).
//! - What Windows does beyond the documents (winref d3d11clear): 16-bit and the 11- and 10-bit
//!   floats are rounded toward zero (past the largest finite value: the largest, 70000 is
//!   0x7bff; an infinity stays one); a 16-bit NaN is the quiet 0x7e00 with its sign, an
//!   unsigned float's NaN every exponent and mantissa bit; ClearView's color on an integer
//!   format is each value truncated toward zero (clamped to the channel, NaN 0).

use kome::num::{lo32, trunc_i64, trunc_u32, us};

/// What a channel holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    F32,
    F16,
    /// an unsigned float of 6 (11-bit) or 5 (10-bit) mantissa bits, 5 exponent bits
    UF,
    Unorm,
    Snorm,
    Uint,
    Sint,
}

/// A channel: (bits, kind, which of the clear's four values: 0 R, 1 G, 2 B, 3 A).
type Ch = (u32, Kind, usize);

/// How the clear's values are given.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Values {
    Uint([u32; 4]),
    Float([f32; 4]),
    /// ClearView's color: a Float clear's, and integer channels get it truncated
    Color([f32; 4]),
}

/// A DXGI format's channels from the lowest bits up, and whether its color is sRGB-encoded.
/// None: no format a render target or unordered access view has.
pub fn channels(dxgi: u32) -> Option<(&'static [Ch], bool)> {
    use Kind::*;
    const RGBA32F: &[Ch] = &[(32, F32, 0), (32, F32, 1), (32, F32, 2), (32, F32, 3)];
    const RGBA32U: &[Ch] = &[(32, Uint, 0), (32, Uint, 1), (32, Uint, 2), (32, Uint, 3)];
    const RGBA32S: &[Ch] = &[(32, Sint, 0), (32, Sint, 1), (32, Sint, 2), (32, Sint, 3)];
    const RGB32F: &[Ch] = &[(32, F32, 0), (32, F32, 1), (32, F32, 2)];
    const RGB32U: &[Ch] = &[(32, Uint, 0), (32, Uint, 1), (32, Uint, 2)];
    const RGB32S: &[Ch] = &[(32, Sint, 0), (32, Sint, 1), (32, Sint, 2)];
    const RGBA16F: &[Ch] = &[(16, F16, 0), (16, F16, 1), (16, F16, 2), (16, F16, 3)];
    const RGBA16N: &[Ch] = &[(16, Unorm, 0), (16, Unorm, 1), (16, Unorm, 2), (16, Unorm, 3)];
    const RGBA16U: &[Ch] = &[(16, Uint, 0), (16, Uint, 1), (16, Uint, 2), (16, Uint, 3)];
    const RGBA16SN: &[Ch] = &[(16, Snorm, 0), (16, Snorm, 1), (16, Snorm, 2), (16, Snorm, 3)];
    const RGBA16S: &[Ch] = &[(16, Sint, 0), (16, Sint, 1), (16, Sint, 2), (16, Sint, 3)];
    const RG32F: &[Ch] = &[(32, F32, 0), (32, F32, 1)];
    const RG32U: &[Ch] = &[(32, Uint, 0), (32, Uint, 1)];
    const RG32S: &[Ch] = &[(32, Sint, 0), (32, Sint, 1)];
    const RGB10A2N: &[Ch] = &[(10, Unorm, 0), (10, Unorm, 1), (10, Unorm, 2), (2, Unorm, 3)];
    const RGB10A2U: &[Ch] = &[(10, Uint, 0), (10, Uint, 1), (10, Uint, 2), (2, Uint, 3)];
    const RG11B10F: &[Ch] = &[(11, UF, 0), (11, UF, 1), (10, UF, 2)];
    const RGBA8N: &[Ch] = &[(8, Unorm, 0), (8, Unorm, 1), (8, Unorm, 2), (8, Unorm, 3)];
    const RGBA8U: &[Ch] = &[(8, Uint, 0), (8, Uint, 1), (8, Uint, 2), (8, Uint, 3)];
    const RGBA8SN: &[Ch] = &[(8, Snorm, 0), (8, Snorm, 1), (8, Snorm, 2), (8, Snorm, 3)];
    const RGBA8S: &[Ch] = &[(8, Sint, 0), (8, Sint, 1), (8, Sint, 2), (8, Sint, 3)];
    const RG16F: &[Ch] = &[(16, F16, 0), (16, F16, 1)];
    const RG16N: &[Ch] = &[(16, Unorm, 0), (16, Unorm, 1)];
    const RG16U: &[Ch] = &[(16, Uint, 0), (16, Uint, 1)];
    const RG16SN: &[Ch] = &[(16, Snorm, 0), (16, Snorm, 1)];
    const RG16S: &[Ch] = &[(16, Sint, 0), (16, Sint, 1)];
    const R32F: &[Ch] = &[(32, F32, 0)];
    const R32U: &[Ch] = &[(32, Uint, 0)];
    const R32S: &[Ch] = &[(32, Sint, 0)];
    const RG8N: &[Ch] = &[(8, Unorm, 0), (8, Unorm, 1)];
    const RG8U: &[Ch] = &[(8, Uint, 0), (8, Uint, 1)];
    const RG8SN: &[Ch] = &[(8, Snorm, 0), (8, Snorm, 1)];
    const RG8S: &[Ch] = &[(8, Sint, 0), (8, Sint, 1)];
    const R16F: &[Ch] = &[(16, F16, 0)];
    const R16N: &[Ch] = &[(16, Unorm, 0)];
    const R16U: &[Ch] = &[(16, Uint, 0)];
    const R16SN: &[Ch] = &[(16, Snorm, 0)];
    const R16S: &[Ch] = &[(16, Sint, 0)];
    const R8N: &[Ch] = &[(8, Unorm, 0)];
    const R8U: &[Ch] = &[(8, Uint, 0)];
    const R8SN: &[Ch] = &[(8, Snorm, 0)];
    const R8S: &[Ch] = &[(8, Sint, 0)];
    const A8N: &[Ch] = &[(8, Unorm, 3)];
    const B5G6R5: &[Ch] = &[(5, Unorm, 2), (6, Unorm, 1), (5, Unorm, 0)];
    const B5G5R5A1: &[Ch] = &[(5, Unorm, 2), (5, Unorm, 1), (5, Unorm, 0), (1, Unorm, 3)];
    const BGRA8N: &[Ch] = &[(8, Unorm, 2), (8, Unorm, 1), (8, Unorm, 0), (8, Unorm, 3)];
    // (X: its bits stay 0)
    const BGRX8N: &[Ch] = &[(8, Unorm, 2), (8, Unorm, 1), (8, Unorm, 0), (8, Uint, 4)];
    const B4G4R4A4: &[Ch] = &[(4, Unorm, 2), (4, Unorm, 1), (4, Unorm, 0), (4, Unorm, 3)];
    Some(match dxgi {
        2 => (RGBA32F, false),
        3 => (RGBA32U, false),
        4 => (RGBA32S, false),
        6 => (RGB32F, false),
        7 => (RGB32U, false),
        8 => (RGB32S, false),
        10 => (RGBA16F, false),
        11 => (RGBA16N, false),
        12 => (RGBA16U, false),
        13 => (RGBA16SN, false),
        14 => (RGBA16S, false),
        16 => (RG32F, false),
        17 => (RG32U, false),
        18 => (RG32S, false),
        24 => (RGB10A2N, false),
        25 => (RGB10A2U, false),
        26 => (RG11B10F, false),
        28 => (RGBA8N, false),
        29 => (RGBA8N, true),
        30 => (RGBA8U, false),
        31 => (RGBA8SN, false),
        32 => (RGBA8S, false),
        34 => (RG16F, false),
        35 => (RG16N, false),
        36 => (RG16U, false),
        37 => (RG16SN, false),
        38 => (RG16S, false),
        41 => (R32F, false),
        42 => (R32U, false),
        43 => (R32S, false),
        49 => (RG8N, false),
        50 => (RG8U, false),
        51 => (RG8SN, false),
        52 => (RG8S, false),
        54 => (R16F, false),
        56 => (R16N, false),
        57 => (R16U, false),
        58 => (R16SN, false),
        59 => (R16S, false),
        61 => (R8N, false),
        62 => (R8U, false),
        63 => (R8SN, false),
        64 => (R8S, false),
        65 => (A8N, false),
        85 => (B5G6R5, false),
        86 => (B5G5R5A1, false),
        87 => (BGRA8N, false),
        88 => (BGRX8N, false),
        91 => (BGRA8N, true),
        93 => (BGRX8N, true),
        115 => (B4G4R4A4, false),
        _ => return None,
    })
}

/// linear to sRGB (the curve of the sRGB spec, IEC 61966-2-1)
fn to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// A finite float's magnitude `a` (not negative) as a float of 5 exponent bits (bias 15) and
/// `m` mantissa bits, rounded toward zero: past the largest finite value the largest.
fn rtz_bits(a: f32, m: u32) -> u32 {
    let max = (0x1e_u32 << m) | mask(m);
    let b = a.to_bits();
    // (the f32 exponent rebiased to 15)
    let e = i64::from((b >> 23) & 0xff).wrapping_sub(112);
    if e >= 31 {
        return max;
    }
    if e >= 1 {
        let man = (b & 0x7f_ffff) >> 23u32.saturating_sub(m);
        return (u32::try_from(e).unwrap_or(0) << m | man).min(max);
    }
    // (a denormal: a / 2^-14 in units of 2^-m)
    trunc_u32((f64::from(a) * 2f64.powi(i32::try_from(m).unwrap_or(0).saturating_add(14))).trunc()).min(mask(m))
}

/// a float as an unsigned float of `m` mantissa bits and 5 exponent bits (R11G11B10's):
/// negative values 0, rounded toward zero, past the largest finite value the largest, an
/// infinity one; NaN every exponent and mantissa bit
fn uf_bits(v: f32, m: u32) -> u32 {
    if v.is_nan() {
        return 0x1f << m | mask(m);
    }
    if v <= 0.0 {
        return 0;
    }
    if v.is_infinite() {
        return 0x1f << m;
    }
    rtz_bits(v, m)
}

/// a float as a 16-bit float: rounded toward zero, past the largest finite value the largest,
/// an infinity one, NaN the quiet 0x7e00; the sign kept
fn f16_rtz(v: f32) -> u32 {
    let sign = (v.to_bits() >> 16) & 0x8000;
    if v.is_nan() {
        return sign | 0x7e00;
    }
    if v.is_infinite() {
        return sign | 0x7c00;
    }
    sign | rtz_bits(v.abs(), 10)
}

/// a float to an n-bit integer channel, truncated toward zero and clamped to the channel's
/// range (NaN 0), as ClearView gives an integer format its color
fn int_trunc(v: f32, n: u32, signed: bool) -> u32 {
    let v = f64::from(if v.is_nan() { 0.0 } else { v }).trunc();
    if signed {
        let hi = f64::from(mask(n.saturating_sub(1)));
        lo32(trunc_i64(v.clamp(-hi - 1.0, hi))) & mask(n)
    } else {
        trunc_u32(v.clamp(0.0, f64::from(mask(n))))
    }
}

/// a float to an n-bit UNORM or SNORM value (documented: clamped, scaled, rounded to the
/// nearest even; NaN is 0; the product in f32 and SNORM's halves away from zero, winref
/// d3d11snorm)
fn norm(v: f32, n: u32, signed: bool) -> u32 {
    let v = if v.is_nan() { 0.0 } else { v };
    if signed {
        // (2^(n-1) - 1 up to 16 bits: exact in f32)
        let max = kome::num::to_f32(mask(n.saturating_sub(1)));
        let q = trunc_i64(f64::from((v.clamp(-1.0, 1.0) * max).round()));
        lo32(q) & mask(n)
    } else {
        // (the f32 product: 0.5 / 255's nearest f32 is 0.50000003 / 255, and WARP clears it to
        // 0, winref d3d11snorm; 2^n - 1 is exact in f32 up to 24 bits)
        let max = kome::num::to_f32(mask(n));
        trunc_u32(f64::from((v.clamp(0.0, 1.0) * max).round_ties_even()))
    }
}

const fn mask(n: u32) -> u32 {
    if n >= 32 { u32::MAX } else { (1u32 << n).wrapping_sub(1) }
}

/// The bytes of one texel of `dxgi` a clear with `v` writes (little-endian, the format's size);
/// None: no texel (a Float clear of an integer format does nothing, or no such format).
pub fn texel(dxgi: u32, v: Values) -> Option<Vec<u8>> {
    let (chs, srgb) = channels(dxgi)?;
    if let Values::Float(_) = v
        && chs.iter().any(|c| matches!(c.1, Kind::Uint | Kind::Sint) && c.2 < 4)
    {
        return None;
    }
    let mut bits: Vec<(u32, u32)> = Vec::new();
    for &(n, kind, i) in chs {
        let w = match v {
            // (an X channel: 0)
            _ if i >= 4 => 0,
            Values::Uint(u) => u.get(i).copied().unwrap_or(0) & mask(n),
            Values::Color(f) if matches!(kind, Kind::Uint | Kind::Sint) => int_trunc(f.get(i).copied().unwrap_or(0.0), n, kind == Kind::Sint),
            Values::Float(f) | Values::Color(f) => {
                let x = f.get(i).copied().unwrap_or(0.0);
                let x = if srgb && i < 3 && !x.is_nan() { to_srgb(x.clamp(0.0, 1.0)) } else { x };
                match kind {
                    Kind::F32 => x.to_bits(),
                    Kind::F16 => f16_rtz(x),
                    Kind::UF => uf_bits(x, n.saturating_sub(5)),
                    Kind::Unorm => norm(x, n, false),
                    Kind::Snorm => norm(x, n, true),
                    Kind::Uint | Kind::Sint => 0,
                }
            }
        };
        bits.push((n, w));
    }
    // (packed from the lowest bits up: at most 128 of them)
    let mut word = 0u128;
    let mut at = 0u32;
    for (n, w) in bits {
        word |= u128::from(w & mask(n)).checked_shl(at).unwrap_or(0);
        at = at.saturating_add(n);
    }
    Some(word.to_le_bytes().into_iter().take(us(at >> 3)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn as_documented() {
        // UNORM: clamped, scaled, ties to even (0.5 * 255 = 127.5 -> 128)
        assert_eq!(texel(28, Values::Float([0.5, 2.0, -1.0, f32::NAN])), Some(vec![128, 255, 0, 0]));
        // BGRA: B first; X 0
        assert_eq!(texel(87, Values::Float([1.0, 0.0, 0.25, 1.0])), Some(vec![64, 0, 255, 255]));
        assert_eq!(texel(88, Values::Float([1.0, 0.0, 0.25, 1.0])), Some(vec![64, 0, 255, 0]));
        // SNORM: -1 is -127; halves away from zero (winref d3d11snorm)
        assert_eq!(texel(31, Values::Float([-1.0, 1.0, 0.0, -2.0])), Some(vec![0x81, 0x7f, 0, 0x81]));
        assert_eq!(texel(31, Values::Float([2.5 / 127.0, -2.5 / 127.0, 63.5 / 127.0, f32::NAN])), Some(vec![3, 0xfd, 64, 0]));
        // a Float clear of an integer format does nothing; a Uint clear of a float one is bits
        assert_eq!(texel(42, Values::Float([1.0; 4])), None);
        assert_eq!(texel(41, Values::Uint([0x3f80_0000, 0, 0, 0])), Some(1f32.to_le_bytes().to_vec()));
        // Uint: the lower bits of each element
        assert_eq!(texel(30, Values::Uint([0x1ff, 2, 3, 4])), Some(vec![0xff, 2, 3, 4]));
        assert_eq!(texel(24, Values::Uint([1023, 0, 1023, 3])), Some(0xfff0_03ffu32.to_le_bytes().to_vec()));
        // R11G11B10: 1.0 is exponent 15, mantissa 0
        assert_eq!(texel(26, Values::Float([1.0, 1.0, 1.0, 0.0])), Some((15u32 << 6 | 15 << 17 | 15 << 27).to_le_bytes().to_vec()));
        // A8: the alpha
        assert_eq!(texel(65, Values::Float([1.0, 1.0, 1.0, 0.0])), Some(vec![0]));
        // sRGB: 0.5 linear is 0.7354 encoded: 188
        assert_eq!(texel(29, Values::Float([0.5, 0.0, 1.0, 0.5])), Some(vec![188, 0, 255, 128]));
        assert_eq!(texel(9, Values::Uint([0; 4])), None, "typeless: views have a type");
        // Windows beyond the documents (winref d3d11clear): 16-bit floats toward zero, past
        // the largest the largest, infinity kept, NaN the quiet one with its sign
        assert_eq!(f16_rtz(70000.0), 0x7bff);
        assert_eq!(f16_rtz(f32::INFINITY), 0x7c00);
        assert_eq!(f16_rtz(f32::NEG_INFINITY), 0xfc00);
        assert_eq!(f16_rtz(f32::NAN), 0x7e00);
        assert_eq!(f16_rtz(-f32::NAN), 0xfe00);
        assert_eq!(f16_rtz(1e-7), 0x0001);
        assert_eq!(f16_rtz(1.0009), 0x3c00, "the nearest would be 0x3c01");
        // unsigned floats: NaN every bit, negative 0, past the largest the largest
        assert_eq!(uf_bits(f32::NAN, 6), 0x7ff);
        assert_eq!(uf_bits(f32::NAN, 5), 0x3ff);
        assert_eq!(uf_bits(-1.0, 6), 0);
        assert_eq!(uf_bits(70000.0, 6), 0x7bf);
        assert_eq!(uf_bits(f32::INFINITY, 5), 0x3e0);
        // ClearView's color on an integer format: each value truncated
        assert_eq!(texel(30, Values::Color([0.25, 0.75, 1.0, 300.0])), Some(vec![0, 0, 1, 255]));
        assert_eq!(texel(32, Values::Color([-1.5, 2.9, f32::NAN, -300.0])), Some(vec![0xff, 2, 0, 0x80]));
    }

    /// winref d3d11snorm's clears of halves of a step: R16G16_SNORM's halves away from zero,
    /// R8G8B8A8_UNORM's to even (every row of both builds' golden output)
    #[test]
    fn warp_tie_clears() {
        let mut checked = 0;
        for name in ["d3d11snorm-32", "d3d11snorm-64"] {
            let p = format!("{}/../../tests/winref/golden/{name}/stdout.txt", env!("CARGO_MANIFEST_DIR"));
            let Ok(out) = std::fs::read_to_string(p) else { continue };
            for line in out.lines() {
                let t: Vec<&str> = line.split_whitespace().collect();
                let (dxgi, vals, w) = match t.as_slice() {
                    ["r16g16_snorm", a, b, w] => (37, vec![*a, b.trim_end_matches(':')], *w),
                    ["rgba8_unorm", a, b, c, d, w] => (28, vec![*a, b, c, d.trim_end_matches(':')], *w),
                    _ => continue,
                };
                let mut f = [0.0f32; 4];
                for (x, v) in f.iter_mut().zip(&vals) {
                    *x = f32::from_bits(u32::from_str_radix(v, 16).unwrap());
                }
                let want = u32::from_str_radix(w, 16).unwrap().to_le_bytes();
                assert_eq!(texel(dxgi, Values::Float(f)), Some(want.to_vec()), "{name}: {line}");
                checked += 1;
            }
        }
        assert!(checked == 0 || checked == 10, "{checked} clears");
    }

    proptest! {
        /// UNORM8 of every value: the nearest of the 256 steps (ties to even)
        #[test]
        fn unorm8_nearest(v in -0.5f32..1.5) {
            let got = texel(61, Values::Float([v, 0.0, 0.0, 0.0])).unwrap_or_default()[0];
            let c = v.clamp(0.0, 1.0) * 255.0;
            let err = (f32::from(got) - c).abs();
            prop_assert!(err <= 0.5, "{} -> {}", v, got);
        }

        /// the 11- and 10-bit floats: rounded toward zero (Windows, winref d3d11clear): the
        /// decoded value is at most v and less than a step below it; past the largest finite
        /// value, the largest
        #[test]
        fn small_floats_toward_zero(v in 0.0f32..70000.0, m in prop_oneof![Just(6u32), Just(5u32)]) {
            let b = uf_bits(v, m);
            let (e, man) = (b >> m, b & ((1 << m) - 1));
            let dec = if e == 0 { f64::from(man) / f64::from(1u32 << m) * 2f64.powi(-14) } else { (1.0 + f64::from(man) / f64::from(1u32 << m)) * 2f64.powi(i32::try_from(e).unwrap_or(0) - 15) };
            let step = if e == 0 { 2f64.powi(-14) / f64::from(1u32 << m) } else { 2f64.powi(i32::try_from(e).unwrap_or(0) - 15) / f64::from(1u32 << m) };
            prop_assert!(e < 31, "{} -> e {}", v, e);
            let top = b == (0x1e << m | ((1 << m) - 1));
            prop_assert!(dec <= f64::from(v) && (f64::from(v) < dec + step || top), "{} -> e {} m {} = {}", v, e, man, dec);
        }

        /// 16-bit floats: rounded toward zero too, with the sign
        #[test]
        fn f16_toward_zero(v in -70000.0f32..70000.0) {
            let b = f16_rtz(v);
            let a = b & 0x7fff;
            prop_assert_eq!(b >> 15, u32::from(v.is_sign_negative()), "sign of {}", v);
            let (e, man) = (a >> 10, a & 0x3ff);
            prop_assert!(e < 31, "{} -> {:#x}", v, b);
            let dec = if e == 0 { f64::from(man) * 2f64.powi(-24) } else { (1.0 + f64::from(man) / 1024.0) * 2f64.powi(i32::try_from(e).unwrap_or(0) - 15) };
            let step = if e == 0 { 2f64.powi(-24) } else { 2f64.powi(i32::try_from(e).unwrap_or(0) - 25) };
            let x = f64::from(v.abs());
            prop_assert!(dec <= x && (x < dec + step || a == 0x7bff), "{} -> {:#x}", v, b);
        }

        /// every format: the texel is its size, and a Uint clear's bits read back per channel
        #[test]
        fn uint_bits_read_back(fmt in 1u32..=132, u in any::<[u32; 4]>()) {
            if let Some((chs, _)) = channels(fmt) {
                let t = texel(fmt, Values::Uint(u)).unwrap_or_default();
                let total: u32 = chs.iter().map(|c| c.0).sum();
                prop_assert_eq!(u32::try_from(t.len() * 8).unwrap_or(0), total);
                let word = t.iter().rev().fold(0u128, |a, &b| a << 8 | u128::from(b));
                let mut at = 0;
                for &(n, _, i) in chs {
                    let got = lo32(word >> at) & mask(n);
                    let want = if i < 4 { u[i] & mask(n) } else { 0 };
                    prop_assert_eq!(got, want, "format {} channel {}", fmt, i);
                    at += n;
                }
            }
        }
    }
}
