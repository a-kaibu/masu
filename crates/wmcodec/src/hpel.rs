//! Motion compensation pixel operations (FFmpeg's hpeldsp put / put_no_rnd and wmv2dsp's
//! mspel filters). Sources are blocks with the pixels around them (see `window`).

use crate::num::{i32z, ix, lo8};

/// A `w` x `h` block of `plane` (stride `stride`, `pw` x `ph` pixels) at (x, y), pixels
/// outside repeated from the edge (emulated_edge_mc).
pub fn window(plane: &[u8], stride: usize, pw: i32, ph: i32, x: i32, y: i32, w: usize, h: usize, out: &mut [u8]) {
    // columns [i0, i1) are inside the plane; those left of it repeat its first pixel, those
    // right of it its last
    let wi = i32::try_from(w).unwrap_or(i32::MAX);
    let i0 = ix((-x).clamp(0, wi));
    let i1 = ix((pw - x).clamp(0, wi));
    for (sy, o) in (y..).zip(out[..w * h].chunks_exact_mut(w)) {
        let sy = ix(sy.clamp(0, ph - 1));
        let row = &plane[sy * stride..sy * stride + ix(pw)];
        o[..i0].fill(row[0]);
        if i1 > i0 {
            // (i0 is max(-x, 0) here, below w)
            let sx = ix(x.max(0));
            o[i0..i1].copy_from_slice(&row[sx..sx + i1 - i0]);
        }
        o[i1.max(i0)..].fill(row[ix(pw) - 1]);
    }
}

/// The `w` x `h` block at (x, y) as (pixels, stride): straight from `plane` when it is all
/// inside, else `window`ed into `buf` (what FFmpeg does with emulated_edge_mc).
pub fn source<'a>(plane: &'a [u8], stride: usize, pw: i32, ph: i32, x: i32, y: i32, w: usize, h: usize, buf: &'a mut [u8]) -> (&'a [u8], usize) {
    let inside = |p: i32, n: usize, max: i32| p >= 0 && p + i32z(n) <= max;
    if inside(x, w, pw) && inside(y, h, ph) {
        (&plane[ix(y) * stride + ix(x)..], stride)
    } else {
        window(plane, stride, pw, ph, x, y, w, h, buf);
        (buf, w)
    }
}

/// put_pixels / put_no_rnd_pixels, `w` x `h`, half-pel position `dxy` (bit 0: x, bit 1: y);
/// `src` holds (w + 1) x (h + 1) pixels with stride `ss`.
pub fn hpel(dst: &mut [u8], doff: usize, ds: usize, src: &[u8], ss: usize, w: usize, h: usize, dxy: usize, rnd: bool) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    if crate::simd::hpel(dst, doff, ds, src, ss, w, h, dxy, rnd) {
        return;
    }
    hpel_c(dst, doff, ds, src, ss, w, h, dxy, rnd)
}

/// hpel in scalar code
pub(crate) fn hpel_c(dst: &mut [u8], doff: usize, ds: usize, src: &[u8], ss: usize, w: usize, h: usize, dxy: usize, rnd: bool) {
    // (one loop per position, over whole rows: LLVM vectorizes them)
    let r = u16::from(rnd);
    for y in 0..h {
        let d = &mut dst[doff + y * ds..][..w];
        let (a, b) = (&src[y * ss..][..w + 1], &src[(y + 1) * ss..][..w + 1]);
        match dxy & 3 {
            0 => d.copy_from_slice(&a[..w]),
            1 => {
                for ((o, &p), &q) in d.iter_mut().zip(&a[..w]).zip(&a[1..]) {
                    *o = lo8(u32::from((u16::from(p) + u16::from(q) + r) >> 1));
                }
            }
            2 => {
                for ((o, &p), &q) in d.iter_mut().zip(&a[..w]).zip(&b[..w]) {
                    *o = lo8(u32::from((u16::from(p) + u16::from(q) + r) >> 1));
                }
            }
            _ => {
                for (x, o) in d.iter_mut().enumerate() {
                    *o = lo8(u32::from((u16::from(a[x]) + u16::from(a[x + 1]) + u16::from(b[x]) + u16::from(b[x + 1]) + 1 + r) >> 2));
                }
            }
        }
    }
}

#[inline]
fn tap(a: u8, b: u8, c: u8, d: u8) -> u8 {
    lo8(((9 * (i32::from(b) + i32::from(c)) - (i32::from(a) + i32::from(d)) + 8) >> 4).clamp(0, 255).cast_unsigned())
}

/// wmv2_mspel8_h_lowpass: `h` rows of 8 from `src` at `so` (it reads one left, two right).
fn h_low(dst: &mut [u8], ds: usize, src: &[u8], so: usize, ss: usize, h: usize) {
    for y in 0..h {
        let s = so + y * ss;
        for x in 0..8 {
            dst[y * ds + x] = tap(src[s + x - 1], src[s + x], src[s + x + 1], src[s + x + 2]);
        }
    }
}

/// wmv2_mspel8_v_lowpass: 8 x 8 (it reads one row up, two down).
fn v_low(dst: &mut [u8], doff: usize, ds: usize, src: &[u8], so: usize, ss: usize) {
    for x in 0..8usize {
        for y in 0..8usize {
            let s = |k: isize| src[(so.cast_signed() + x.cast_signed() + (y.cast_signed() + k) * ss.cast_signed()).cast_unsigned()];
            dst[doff + y * ds + x] = tap(s(-1), s(0), s(1), s(2));
        }
    }
}

fn avg(dst: &mut [u8], doff: usize, ds: usize, a: &[u8], ao: usize, as_: usize, b: &[u8], bs: usize) {
    for y in 0..8 {
        for x in 0..8 {
            dst[doff + y * ds + x] = lo8((u32::from(a[ao + y * as_ + x]) + u32::from(b[y * bs + x]) + 1) >> 1);
        }
    }
}

/// put_mspel_pixels_tab[dxy] for an 8x8 block: `src` at `so` with at least one pixel / row
/// before it and two after (stride `ss`).
pub fn mspel8(dst: &mut [u8], doff: usize, ds: usize, src: &[u8], so: usize, ss: usize, dxy: usize) {
    let mut half = [0u8; 64];
    let mut hh = [0u8; 88];
    let mut hv = [0u8; 64];
    match dxy {
        0 => {
            for y in 0..8 {
                dst[doff + y * ds..doff + y * ds + 8].copy_from_slice(&src[so + y * ss..so + y * ss + 8]);
            }
        }
        1 | 3 => {
            h_low(&mut half, 8, src, so, ss, 8);
            avg(dst, doff, ds, src, so + usize::from(dxy == 3), ss, &half, 8);
        }
        2 => h_low(&mut dst[doff..], ds, src, so, ss, 8),
        4 => v_low(dst, doff, ds, src, so, ss),
        5 | 7 => {
            h_low(&mut hh, 8, src, so - ss, ss, 11);
            v_low(&mut half, 0, 8, src, so + usize::from(dxy == 7), ss);
            v_low(&mut hv, 0, 8, &hh, 8, 8);
            avg(dst, doff, ds, &half, 0, 8, &hv, 8);
        }
        _ => {
            h_low(&mut hh, 8, src, so - ss, ss, 11);
            v_low(dst, doff, ds, &hh, 8, 8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// window, pixel by pixel
    fn window_ref(plane: &[u8], stride: usize, pw: i32, ph: i32, x: i32, y: i32, w: usize, h: usize, out: &mut [u8]) {
        for j in 0..h {
            let sy = (y + j as i32).clamp(0, ph - 1) as usize;
            for i in 0..w {
                out[j * w + i] = plane[sy * stride + (x + i as i32).clamp(0, pw - 1) as usize];
            }
        }
    }

    /// hpel, pixel by pixel
    fn hpel_ref(dst: &mut [u8], doff: usize, ds: usize, src: &[u8], ss: usize, w: usize, h: usize, dxy: usize, rnd: bool) {
        let r = rnd as u32;
        for y in 0..h {
            for x in 0..w {
                let p = |dx: usize, dy: usize| src[(y + dy) * ss + x + dx] as u32;
                dst[doff + y * ds + x] = match dxy & 3 {
                    0 => p(0, 0),
                    1 => (p(0, 0) + p(1, 0) + r) >> 1,
                    2 => (p(0, 0) + p(0, 1) + r) >> 1,
                    _ => (p(0, 0) + p(1, 0) + p(0, 1) + p(1, 1) + 1 + r) >> 2,
                } as u8;
            }
        }
    }

    proptest! {
        /// Blocks anywhere, inside the plane, across its edges or wholly outside it: the
        /// pixel-by-pixel copy.
        #[test]
        fn window_is_the_clamped_copy(pw in 1i32..40, ph in 1i32..40, pad in 0usize..8, x in -60i32..60, y in -60i32..60, w in 1usize..20, h in 1usize..20, seed in any::<u64>()) {
            let stride = pw as usize + pad;
            let mut s = seed;
            let plane: Vec<u8> = (0..stride * ph as usize).map(|_| { s = s.wrapping_mul(6364136223846793005).wrapping_add(1); (s >> 56) as u8 }).collect();
            let (mut a, mut b) = (vec![0u8; w * h], vec![1u8; w * h]);
            window(&plane, stride, pw, ph, x, y, w, h, &mut a);
            window_ref(&plane, stride, pw, ph, x, y, w, h, &mut b);
            prop_assert_eq!(a, b);
        }

        /// Every half-pel position and rounding: the pixel-by-pixel average.
        #[test]
        fn hpel_is_the_average(dxy in 0usize..4, rnd in any::<bool>(), w in prop::sample::select(vec![4usize, 8, 16]), src in prop::collection::vec(any::<u8>(), 17 * 17), doff in 0usize..5) {
            let n = w.max(8);
            let (mut a, mut b) = (vec![7u8; 24 * 24], vec![7u8; 24 * 24]);
            hpel(&mut a, doff, 24, &src, 17, w, n, dxy, rnd);
            hpel_ref(&mut b, doff, 24, &src, 17, w, n, dxy, rnd);
            prop_assert_eq!(a, b);
        }
    }
}
