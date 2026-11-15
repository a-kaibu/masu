//! An audio packet's spectrum to time: libvorbis's mapping0_inverse (floor 1, residues 0/1/2,
//! inverse coupling, IMDCT), step for step.

use kome::num::*;
use crate::bits::{i32_len, ilog, Bits};
use crate::floor1_table::INVERSE_DB;
use crate::imdct::Imdct;
use crate::setup::{Floor1, Residue, ResidueKind, Setup};

/// Reusable buffers and the transforms of one stream.
pub struct State {
    imdct: [Imdct; 2],
    floor: Vec<Vec<i32>>,
    /// per channel: whether its floor gave a curve (libvorbis's floormemo)
    curve: Vec<bool>,
    nonzero: Vec<bool>,
    /// floor lines drawn in fixed point (KiriKiri's wuvorbis.dll): y rounded down, not towards
    /// the line's start
    pub lines_round_down: bool,
}

impl State {
    pub fn new(setup: &Setup) -> State {
        let [s, l] = setup.ident.blocksize;
        State { imdct: [Imdct::new(s), Imdct::new(l)], floor: vec![Vec::new(); setup.ident.channels], curve: vec![false; setup.ident.channels], nonzero: vec![false; setup.ident.channels], lines_round_down: false }
    }
}

/// libvorbis's floor1_inverse1: the posts' values, or None for an unused floor (or the packet's
/// end).
fn floor1_read(f: &Floor1, setup: &Setup, b: &mut Bits, out: &mut Vec<i32>) -> Option<()> {
    if !b.flag()? {
        return None;
    }
    let q = f.multiplier.range();
    // (q - 1 < 256: a post's value is at most 8 bits, an i32)
    let bits = ilog(q.unsigned_abs() - 1);
    let post = |v: u32| i32::try_from(v).ok();
    out.clear();
    out.push(post(b.read(bits)?)?);
    out.push(post(b.read(bits)?)?);
    for &c in &f.partition_class {
        let cl = &f.classes[usize::from(c)];
        let csub = (1u32 << cl.sub) - 1;
        let mut cval = match cl.master {
            Some(m) if cl.sub > 0 => setup.books[m].decode(b)?,
            Some(_) | None => 0,
        };
        for _ in 0..cl.dim {
            let book = cl.subs[us(cval & csub)];
            cval >>= cl.sub;
            out.push(match book {
                Some(k) => post(setup.books[k].decode(b)?)?,
                None => 0,
            });
        }
    }
    // amplitude synthesis
    for i in 2..f.x.len() {
        let (lo, hi) = (f.lo[i - 2], f.hi[i - 2]);
        let predicted = render_point(f.x[lo], f.x[hi], out[lo], out[hi], f.x[i]);
        let hiroom = q - predicted;
        let loroom = predicted;
        let room = hiroom.min(loroom) << 1;
        let mut val = out[i];
        if val != 0 {
            if val >= room {
                val = if hiroom > loroom { val - loroom } else { -1 - (val - hiroom) };
            } else if val & 1 != 0 {
                val = -((val + 1) >> 1);
            } else {
                val >>= 1;
            }
            out[i] = (val + predicted) & 0x7fff;
            out[lo] &= 0x7fff;
            out[hi] &= 0x7fff;
        } else {
            out[i] = predicted | 0x8000;
        }
    }
    Some(())
}

fn render_point(x0: i32, x1: i32, y0: i32, y1: i32, x: i32) -> i32 {
    let (y0, y1) = (y0 & 0x7fff, y1 & 0x7fff);
    let dy = y1 - y0;
    let adx = x1 - x0;
    let ady = dy.abs();
    let err = ady * (x - x0);
    let off = err / adx;
    if dy < 0 {
        y0 - off
    } else {
        y0 + off
    }
}

fn render_line(n: usize, x0: i32, x1: i32, y0: i32, y1: i32, d: &mut [f32], down: bool) {
    if down {
        // in 10.22 fixed point: the slope truncated, y rounded down
        let slope = ((y1 - y0) << 22) / (x1 - x0);
        let mut acc = y0 << 22;
        // (posts' x are >= 0)
        for x in us(x0)..us(i32_len(n).min(x1)) {
            d[x] *= INVERSE_DB[us((acc >> 22).clamp(0, 255))];
            acc = acc.wrapping_add(slope);
        }
        return;
    }
    let dy = y1 - y0;
    let adx = x1 - x0;
    let mut ady = dy.abs();
    let base = dy / adx;
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    // (posts' x are >= 0)
    let mut x = us(x0);
    let mut y = y0;
    let mut err = 0;
    ady -= (base * adx).abs();
    let n = us(i32_len(n).min(x1));
    if x < n {
        d[x] *= INVERSE_DB[us(y)];
    }
    loop {
        x += 1;
        if x >= n {
            break;
        }
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }
        // (a y under 0 is 255: 2^31 or more, clamped)
        d[x] *= INVERSE_DB[us(y).min(255)];
    }
}

/// libvorbis's floor1_inverse2: the curve multiplied into the spectrum (n / 2 values).
fn floor1_apply(f: &Floor1, fit: &[i32], out: &mut [f32], down: bool) {
    let n = out.len();
    let clamp = |v: i32| v.clamp(0, 255);
    let (mut lx, mut hx) = (0, 0);
    let mut ly = clamp(fit[0] * f.multiplier.get());
    for &cur in &f.sorted[1..] {
        let hy = fit[cur] & 0x7fff;
        if hy == fit[cur] {
            hx = f.x[cur];
            let hy = clamp(hy * f.multiplier.get());
            render_line(n, lx, hx, ly, hy, out, down);
            lx = hx;
            ly = hy;
        }
    }
    for v in out.iter_mut().skip(us(hx.max(0))) {
        *v *= INVERSE_DB[us(ly)];
    }
}

/// libvorbis's _01inverse (residues 0 and 1) and res2_inverse; stops quietly at the packet's
/// end, keeping what was decoded.
fn residue(r: &Residue, setup: &Setup, b: &mut Bits, vecs: &mut [&mut [f32]], half: usize) {
    let classbook = &setup.books[r.classbook];
    let per_word = classbook.dim;
    let ch = vecs.len();
    let (max, n_ch) = match r.kind {
        ResidueKind::Two => (half * ch, 1),
        ResidueKind::Zero | ResidueKind::One => (half, ch),
    };
    let end = r.end.min(max);
    if end <= r.begin {
        return;
    }
    let n = end - r.begin;
    let partvals = n / r.grouping;
    let partwords = partvals.div_ceil(per_word);
    let partvals_max = u32::try_from(per_word).ok().and_then(|p| r.classifications.checked_pow(p)).unwrap_or(usize::MAX);
    // the classifications of each partition, per channel (one "channel" for residue 2)
    let mut classes = vec![0u8; n_ch * partwords * per_word];
    'stages: for s in 0..r.stages {
        let mut i = 0;
        let mut l = 0;
        while i < partvals {
            if s == 0 {
                for j in 0..n_ch {
                    let Some(temp) = classbook.decode(b) else { break 'stages };
                    let mut v = us(temp);
                    if v >= partvals_max {
                        break 'stages;
                    }
                    for k in (0..per_word).rev() {
                        // (classifications <= 64)
                        classes[(j * partwords + l) * per_word + k] = (v % r.classifications).to_le_bytes()[0];
                        v /= r.classifications;
                    }
                }
            }
            let mut k = 0;
            while k < per_word && i < partvals {
                for j in 0..n_ch {
                    let c = usize::from(classes[(j * partwords + l) * per_word + k]);
                    let Some(book) = r.books[c][s] else { continue };
                    let book = &setup.books[book];
                    let offset = r.begin + i * r.grouping;
                    let ok = match r.kind {
                        ResidueKind::Zero => decodevs_add(book, b, &mut vecs[j][offset..], r.grouping),
                        ResidueKind::One => decodev_add(book, b, &mut vecs[j][offset..], r.grouping),
                        ResidueKind::Two => decodevv_add(book, b, vecs, offset, r.grouping),
                    };
                    if !ok {
                        break 'stages;
                    }
                }
                k += 1;
                i += 1;
            }
            l += 1;
        }
    }
}

fn decodevs_add(book: &crate::codebook::Codebook, b: &mut Bits, a: &mut [f32], n: usize) -> bool {
    let step = n / book.dim;
    let mut entries = Vec::with_capacity(step);
    for _ in 0..step {
        let Some(e) = book.decode(b) else { return false };
        entries.push(e);
    }
    for i in 0..book.dim {
        let o = i * step;
        for (j, &e) in entries.iter().enumerate() {
            if o + j >= n {
                break;
            }
            a[o + j] += book.vector(e)[i];
        }
    }
    true
}

fn decodev_add(book: &crate::codebook::Codebook, b: &mut Bits, a: &mut [f32], n: usize) -> bool {
    let mut i = 0;
    while i < n {
        let Some(e) = book.decode(b) else { return false };
        for &v in book.vector(e) {
            if i >= n {
                break;
            }
            a[i] += v;
            i += 1;
        }
    }
    true
}

fn decodevv_add(book: &crate::codebook::Codebook, b: &mut Bits, a: &mut [&mut [f32]], offset: usize, n: usize) -> bool {
    if book.used == 0 {
        return true;
    }
    let ch = a.len();
    let m = (offset + n) / ch;
    let mut i = offset / ch;
    let mut chptr = 0;
    while i < m {
        let Some(e) = book.decode(b) else { return false };
        for &v in book.vector(e) {
            if i >= m {
                break;
            }
            a[chptr][i] += v;
            chptr += 1;
            if chptr == ch {
                chptr = 0;
                i += 1;
            }
        }
    }
    true
}

/// mapping0_inverse: decodes the rest of an audio packet (`b` past its mode and window flags)
/// for a block of mode `mode` into `pcm` (one vector per channel, the block's size each): the
/// block's samples before windowing.
pub fn mapping_inverse(setup: &Setup, st: &mut State, b: &mut Bits, mode: usize, pcm: &mut [&mut [f32]]) {
    let m = &setup.mappings[setup.modes[mode].mapping];
    let w = setup.modes[mode].block();
    let n = setup.ident.size(w);
    let half = n / 2;
    let ch = setup.ident.channels;
    for i in 0..ch {
        let f = &setup.floors[m.submaps[m.mux[i]].0];
        st.curve[i] = floor1_read(f, setup, b, &mut st.floor[i]).is_some();
        st.nonzero[i] = st.curve[i];
        pcm[i][..half].fill(0.0);
    }
    for &(mag, ang) in &m.coupling {
        if st.nonzero[mag] || st.nonzero[ang] {
            st.nonzero[mag] = true;
            st.nonzero[ang] = true;
        }
    }
    for (s, &(_, r)) in m.submaps.iter().enumerate() {
        let r = &setup.residues[r];
        let mut bundle: Vec<&mut [f32]> = Vec::new();
        let mut any = false;
        let mut zero = Vec::new();
        for (j, v) in pcm.iter_mut().enumerate() {
            if m.mux[j] == s {
                any |= st.nonzero[j];
                zero.push(!st.nonzero[j]);
                bundle.push(&mut v[..half]);
            }
        }
        match r.kind {
            ResidueKind::Two => {
                if any {
                    residue(r, setup, b, &mut bundle, half);
                }
            }
            ResidueKind::Zero | ResidueKind::One => {
                // (residues 0 and 1 decode the channels in use only)
                let mut used: Vec<&mut [f32]> = bundle.into_iter().zip(zero).filter(|(_, z)| !z).map(|(v, _)| v).collect();
                if !used.is_empty() {
                    residue(r, setup, b, &mut used, half);
                }
            }
        }
    }
    for &(mag, ang) in m.coupling.iter().rev() {
        for j in 0..half {
            let (mv, av) = (pcm[mag][j], pcm[ang][j]);
            let (nm, na) = if mv > 0.0 {
                if av > 0.0 {
                    (mv, mv - av)
                } else {
                    (mv + av, mv)
                }
            } else if av > 0.0 {
                (mv, mv + av)
            } else {
                (mv - av, mv)
            };
            pcm[mag][j] = nm;
            pcm[ang][j] = na;
        }
    }
    for i in 0..ch {
        let f = &setup.floors[m.submaps[m.mux[i]].0];
        if st.curve[i] {
            floor1_apply(f, &st.floor[i], &mut pcm[i][..half], st.lines_round_down);
        } else {
            pcm[i][..half].fill(0.0);
        }
    }
    for v in pcm.iter_mut().take(ch) {
        st.imdct[w.index()].backward(&mut v[..n]);
    }
}

/// Reads an audio packet's header: (mode, previous and next window flags) with `b` past them.
pub fn packet_header(setup: &Setup, b: &mut Bits) -> Option<(usize, bool, bool)> {
    if b.read(1)? != 0 {
        return None;
    }
    let mode = us(b.read(ilog(u32::try_from(setup.modes.len() - 1).ok()?))?);
    if mode >= setup.modes.len() {
        return None;
    }
    if setup.modes[mode].blockflag {
        let p = b.flag()?;
        let n = b.flag()?;
        Some((mode, p, n))
    } else {
        Some((mode, false, false))
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// A predicted post lies between its neighbours (so it indexes the dB table when they do).
    #[kani::proof]
    fn render_point_between() {
        let (x0, x1, x, y0, y1): (i32, i32, i32, i32, i32) = (kani::any(), kani::any(), kani::any(), kani::any(), kani::any());
        kani::assume(0 <= x0 && x0 < x && x < x1 && x1 <= 32768);
        kani::assume((0..=255).contains(&y0) && (0..=255).contains(&y1));
        let p = render_point(x0, x1, y0, y1, x);
        assert!(p >= y0.min(y1) && p <= y0.max(y1));
    }

    /// The curve between two posts in 0..=255 stays in the dB table, and in the vector.
    #[kani::proof]
    #[kani::unwind(10)]
    fn render_line_in_table() {
        let (x0, len, y0, y1, n): (i32, i32, i32, i32, usize) = (kani::any(), kani::any(), kani::any(), kani::any(), kani::any());
        kani::assume((0..4).contains(&x0) && (1..=8).contains(&len) && n <= 12);
        kani::assume((0..=255).contains(&y0) && (0..=255).contains(&y1));
        let mut d = [1f32; 12];
        render_line(n, x0, x0 + len, y0, y1, &mut d[..n.max((x0 + len) as usize).min(12)], kani::any());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The y each x of a line got (d starts at 1).
    fn ys(x0: i32, x1: i32, y0: i32, y1: i32, down: bool) -> Vec<usize> {
        let mut d = vec![1f32; x1 as usize];
        render_line(x1 as usize, x0, x1, y0, y1, &mut d, down);
        d[x0 as usize..].iter().map(|v| INVERSE_DB.iter().position(|t| t == v).unwrap()).collect()
    }

    /// wuvorbis.dll's fixed-point lines start where libvorbis's do and stay within one step
    /// below them (libvorbis rounds towards the start, they round down); rising lines that
    /// meet whole steps exactly agree.
    #[test]
    fn fixed_point_lines() {
        for dx in 1..70 {
            for y0 in (0..256).step_by(17) {
                for y1 in (0..256).step_by(13) {
                    let a = ys(3, 3 + dx, y0, y1, false);
                    let b = ys(3, 3 + dx, y0, y1, true);
                    assert_eq!(a[0], b[0]);
                    for (k, (&a, &b)) in a.iter().zip(&b).enumerate() {
                        assert!(b <= a && a - b <= 1, "dx {dx} y {y0}->{y1} x+{k}: {a} {b}");
                        let exact = (y1 - y0) * k as i32 % dx == 0;
                        if y1 >= y0 && exact && ((y1 - y0) << 22) % dx == 0 {
                            assert_eq!(a, b);
                        }
                    }
                }
            }
        }
    }
}
