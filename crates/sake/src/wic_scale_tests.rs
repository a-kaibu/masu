//! WIC's scaler against the code as it was before the interpolation mode became a type
//! (`old_taps`, `old_scale`: copied unchanged): random images of random sizes, both ways, every
//! mode and values past them, compared byte by byte.
use super::*;
use proptest::prelude::*;

fn old_taps(s: u32, d: u32, mode: u32) -> Vec<Vec<(u32, f32)>> {
    let ratio = s as f32 / d as f32;
    (0..d)
        .map(|i| {
            let mut t: Vec<(u32, f32)> = Vec::new();
            let mut add = |j: i64, w: f32| {
                let j = j.clamp(0, i64::from(s) - 1) as u32;
                match t.iter_mut().find(|e| e.0 == j) {
                    Some(e) => e.1 += w,
                    None => t.push((j, w)),
                }
            };
            if mode == 3 {
                // Windows' Fant integrates intervals starting at i*s/d. The first
                // partial source pixel is interpolated at that start, rather than
                // taking its constant color. Use integer positions to keep exact
                // pixel boundaries (including integral shrink ratios).
                let (s, d) = (u64::from(s), u64::from(d));
                let (lo, hi) = (u64::from(i) * s, (u64::from(i) + 1) * s);
                for j in lo / d..hi.div_ceil(d) {
                    let start = lo.max(j * d);
                    let width = hi.min((j + 1) * d) - start;
                    let frac = start - j * d;
                    let weight = width as f64 / (s as f64 * d as f64);
                    add(j as i64, (weight * (d - frac) as f64) as f32);
                    if frac != 0 {
                        add(j as i64 + 1, (weight * frac as f64) as f32);
                    }
                }
                return t;
            }
            if mode == 1 {
                // Windows' bilinear: 16.16 positions, weights in 256ths (each pass floors)
                let step = (u64::from(s) << 16) / u64::from(d);
                let p = (step / 2 + u64::from(i) * step).saturating_sub(0x8000);
                let x = ((p & 0xffff) >> 8) as f32;
                add((p >> 16) as i64, (256.0 - x) / 256.0);
                add((p >> 16) as i64 + 1, x / 256.0);
                return t;
            }
            let f = if mode == 2 {
                // Windows advances in 16.16, then indexes 64 cubic phases.
                // Keep negative edge positions: clamp the taps, not the center,
                // so the negative outer lobe still contributes at the border.
                let step = (u64::from(s) << 16) / u64::from(d);
                let pos = (step / 2 + u64::from(i) * step) as i64 - 0x8000;
                (pos >> 10) as f32 / 64.0
            } else {
                ((i as f32 + 0.5) * ratio - 0.5).max(0.0)
            };
            let (j0, x) = (f.floor() as i64, f - f.floor());
            if mode == 2 {
                // Exact 19-bit coefficients for the 64 phases. Evaluating the
                // polynomial in floats can make an opaque alpha round to 254.
                let cr = |d: f32| {
                    let n = (d.abs() * 64.0) as i32;
                    let weight = if n < 64 {
                        3 * n * n * n - 320 * n * n + 524288
                    } else if n < 128 {
                        -n * n * n + 320 * n * n - 32768 * n + 1048576
                    } else { 0 };
                    weight as f32 / 524288.0
                };
                for k in -1..=2 {
                    add(j0 + k, cr(x - k as f32));
                }
            } else {
                add(j0, 1.0 - x);
                add(j0 + 1, x);
            }
            t
        })
        .collect()
}

fn old_scale(img: &Img, w: u32, h: u32, mode: u32) -> Img {
    let mut px = vec![0u8; (w * h * 4) as usize];
    if img.px.is_empty() {
        return Img { w, h, px, palette: img.palette.clone(), dpi: img.dpi };
    }
    let (sw, sh) = (img.w, img.h);
    let mut pm = vec![0u8; img.px.len()];
    picture::from_bgra(&img.px, img.w, Fmt::Pbgra, (0, 0, img.w, img.h), img.w * 4, &mut pm, &[]);
    if mode == 0 {
        // Windows' pick: (x * sw + sw - 1) / w
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = (((x * sw + sw - 1) / w).min(sw - 1), ((y * sh + sh - 1) / h).min(sh - 1));
                let d = ((y * w + x) * 4) as usize;
                px[d..d + 4].copy_from_slice(&pm[((sy * sw + sx) * 4) as usize..][..4]);
            }
        }
    } else {
        // across, then down
        let (tx, ty) = (old_taps(sw, w, mode), old_taps(sh, h, mode));
        // Specialize rounding outside the pixel loops: nearest/linear must not
        // pay a per-channel branch for Fant's different arithmetic.
        match mode {
            1 => filter::<1>(&pm, sw, sh, w, h, &tx, &ty, &mut px),
            2 => filter_cubic(&pm, sw, sh, w, h, &tx, &ty, &mut px),
            3 => filter::<3>(&pm, sw, sh, w, h, &tx, &ty, &mut px),
            _ => filter::<0>(&pm, sw, sh, w, h, &tx, &ty, &mut px),
        }
    }
    let px = picture::to_bgra(Fmt::Pbgra, w, h, w * 4, &px, &[]);
    Img { w, h, px, palette: img.palette.clone(), dpi: img.dpi }
}


proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]
    #[test]
    fn scale_as_before(sw in 1u32..12, sh in 1u32..9, w in 1u32..20, h in 1u32..14, mode in prop_oneof![0u32..6, any::<u32>()], seed: u64, opaque: bool) {
        let mut x = seed | 1;
        let px: Vec<u8> = (0..sw * sh * 4).map(|i| { x ^= x << 13; x ^= x >> 7; x ^= x << 17; if opaque && i % 4 == 3 { 255 } else { x as u8 } }).collect();
        let img = Img::new(sw, sh, px);
        let want = old_scale(&img, w, h, mode);
        let got = scale(&img, w, h, mode);
        prop_assert!(want.px == got.px && (want.w, want.h) == (got.w, got.h));
    }
}
