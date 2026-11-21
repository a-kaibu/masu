//! DDS headers against the code as it was before the cube faces became a type: random headers,
//! every face and level, compared with a copy of the old parser.
use super::*;
use proptest::prelude::*;

#[derive(Debug)]
struct Old {
    w: u32,
    h: u32,
    depth: u32,
    levels: u32,
    fmt: u32,
    faces: u32,
}
type Dds = Old;
fn old_parse(data: &[u8]) -> Option<Dds> {
    if data.len() < HEADER || &data[..4] != b"DDS " {
        return None;
    }
    let g = |o: usize| data[o..o + 4].try_into().map_or(0, u32::from_le_bytes);
    let (h, w) = (g(12), g(16));
    let caps2 = g(112);
    let fmt = pixel_format(&data[76..108])?;
    let depth = if caps2 & VOLUME != 0 { g(24).max(1) } else { 1 };
    // (a mip count of 0: one level; never more than the chain down to 1x1)
    let max = w.max(h).max(depth).bit_width();
    let levels = g(28).clamp(1, max.max(1));
    let faces = if caps2 & CUBEMAP != 0 {
        FACES.iter().enumerate().filter(|&(_, &b)| caps2 & b != 0).fold(0, |m, (k, _)| m | 1 << k)
    } else {
        0
    };
    if w == 0 || h == 0 || w > 16384 || h > 16384 || depth > 2048 || caps2 & CUBEMAP != 0 && (faces == 0 || w != h) {
        return None;
    }
    let d = Dds { w, h, depth, levels, fmt, faces };
    let end = HEADER as u64 + d.face_size() * u64::from(d.face_count());
    (end <= data.len() as u64).then_some(d)
}

impl Old {
    /// Bytes of one row of blocks and the rows of blocks of a level (one slice).
    fn level_rows(&self, level: u32) -> (u32, u32) {
        let (bpb, bs) = f::layout(self.fmt).unwrap_or((4, 1));
        let (lw, lh) = ((self.w >> level).max(1), (self.h >> level).max(1));
        (lw.div_ceil(bs) * bpb, lh.div_ceil(bs))
    }
    fn level_size(&self, level: u32) -> u64 {
        let (row, rows) = self.level_rows(level);
        u64::from(row) * u64::from(rows) * u64::from((self.depth >> level).max(1))
    }
    fn face_size(&self) -> u64 {
        (0..self.levels).map(|l| self.level_size(l)).sum()
    }
    /// The faces in the file (1 for a 2D or volume texture).
    fn face_count(&self) -> u32 {
        if self.faces == 0 { 1 } else { self.faces.count_ones() }
    }
    /// Where level `level` of face `face` (0..6; 0 for a 2D texture) starts in the file and its
    /// length; None for a face the file doesn't have.
    fn level_at(&self, face: u32, level: u32) -> Option<(usize, usize)> {
        if level >= self.levels {
            return None;
        }
        let k = if self.faces == 0 {
            (face == 0).then_some(0)?
        } else {
            if face >= 6 || self.faces & 1 << face == 0 {
                return None;
            }
            (self.faces & ((1 << face) - 1)).count_ones()
        };
        let at = HEADER as u64 + u64::from(k) * self.face_size() + (0..level).map(|l| self.level_size(l)).sum::<u64>();
        Some((at as usize, self.level_size(level) as usize))
    }
}


proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]
    #[test]
    fn headers_parse_as_before(w in prop_oneof![1u32..20, any::<u32>()], h in prop_oneof![1u32..20, Just(0u32), Just(u32::MAX)], depth in 0u32..6, mips in prop_oneof![0u32..8, any::<u32>()],
        caps2 in prop_oneof![any::<u32>(), (0u32..64).prop_map(|f| CUBEMAP | f << 10), Just(VOLUME)], pf in 0usize..3, extra in prop_oneof![0usize..256, Just(1usize << 20)]) {
        // (u32::MAX: square, so that cube maps parse)
        let h = if h == u32::MAX { w } else { h };
        let mut v = vec![0u8; HEADER + extra];
        v[..4].copy_from_slice(b"DDS ");
        let pfs: [[u32; 8]; 3] = [[32, 0x41, 0, 32, 0xff_0000, 0xff00, 0xff, 0xff00_0000], [32, 4, kura::d3d9::format::DXT1, 0, 0, 0, 0, 0], [32, 4, kura::d3d9::format::DXT5, 0, 0, 0, 0, 0]];
        let mut put = |o: usize, x: u32| v[o..o + 4].copy_from_slice(&x.to_le_bytes());
        put(12, h);
        put(16, w);
        put(24, depth);
        put(28, mips);
        for (k, x) in pfs[pf].iter().enumerate() {
            put(76 + 4 * k, *x);
        }
        put(112, caps2);
        let (new, old) = (parse(&v), old_parse(&v));
        prop_assert_eq!(new.is_some(), old.is_some());
        let (Some(n), Some(o)) = (new, old) else { return Ok(()) };
        prop_assert_eq!((n.w, n.h, n.depth, n.levels, n.fmt), (o.w, o.h, o.depth, o.levels, o.fmt));
        prop_assert_eq!(n.faces.map_or(0, |f| u32::from(f.bits())), o.faces);
        prop_assert_eq!(n.face_count(), o.face_count());
        for face in (0..8).chain([31, 32, u32::MAX]) {
            for l in 0..o.levels.min(12) + 2 {
                prop_assert_eq!(n.level_at(face, l), o.level_at(face, l), "{} {}", face, l);
            }
        }
    }
}
