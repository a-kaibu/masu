//! Surface rows widened to BGRA against the code as it was before the surface format became a
//! type: every format GetDC and read_bgra take, random rows, byte for byte.
use super::*;
use proptest::prelude::*;

fn old_widen(row: &[u8], fmt: u32) -> Vec<u8> {
    let x5 = |v: u16, s: u32| (u32::from(v >> s & 31) * 255 / 31) as u8;
    match fmt {
        21 => row.to_vec(),
        22 => row.chunks(4).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        23 => row
            .chunks(2)
            .flat_map(|p| {
                let v = u16::from_le_bytes([p[0], p[1]]);
                [x5(v, 0), (u32::from(v >> 5 & 63) * 255 / 63) as u8, x5(v, 11), 255]
            })
            .collect(),
        _ => row
            .chunks(2)
            .flat_map(|p| {
                let v = u16::from_le_bytes([p[0], p[1]]);
                let a = if fmt == 25 && v & 0x8000 == 0 { 0 } else { 255 };
                [x5(v, 0), x5(v, 5), x5(v, 10), a]
            })
            .collect(),
    }
}

/// GetDC's (bits per pixel, R5G6B5) and read_bgra's bytes per pixel as they were
fn old_dc(fmt: u32) -> Option<(u16, bool)> {
    match fmt {
        0x15 | 0x16 => Some((32, false)),
        0x17 => Some((16, true)),
        0x18 | 0x19 => Some((16, false)),
        _ => None,
    }
}
fn old_bpp(fmt: u32) -> Option<u32> {
    match fmt {
        21 | 22 => Some(4),
        23 | 24 | 25 => Some(2),
        _ => None,
    }
}

proptest! {
    #[test]
    fn rows_widen_as_before(fmt in 21u32..=25, px in 0usize..40, seed: u64) {
        let mut s = seed | 1;
        let bpp = old_bpp(fmt).unwrap() as usize;
        let row: Vec<u8> = (0..px * bpp).map(|_| { s ^= s << 13; s ^= s >> 7; s ^= s << 17; (s >> 24) as u8 }).collect();
        let f = SurfFmt::of(fmt).unwrap();
        prop_assert_eq!(widen_bgra(&row, f), old_widen(&row, fmt));
    }
}

#[test]
fn formats_as_before() {
    for fmt in (0..64).chain([u32::MAX, 0x8000_0015, 21 + 256]) {
        let f = SurfFmt::of(fmt);
        assert_eq!(f.map(SurfFmt::dc_bits), old_dc(fmt), "{fmt}");
        assert_eq!(f.map(SurfFmt::bytes), old_bpp(fmt), "{fmt}");
    }
}
