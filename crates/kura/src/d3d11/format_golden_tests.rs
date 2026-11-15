//! Every answer of d3d11::format pinned, before its conversion codes become typed (`conv`, a u32
//! holding either a D3DFORMAT or one of format.rs's own codes, as an enum the compiler checks
//! every use against): for every DXGI format 0..=200 and a few out-of-range ones, info, support,
//! page, pitch_rows, texture_bytes, page_texture_bytes, vertex_format, family, copy_compatible,
//! packs_back, and hashes of what from_page and to_page make of the same random texels.
//! format_golden.txt holds them; MASU_UPDATE_GOLDEN=1 rewrites it.
use super::format::*;

fn fnv(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h ^= u64::from(b);
        *h = h.wrapping_mul(0x100_0000_01b3);
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn byte(&mut self) -> u8 {
        self.next().to_le_bytes()[3]
    }
}

/// Conv as the u32 code Format::conv was (a D3DFORMAT or format.rs's own codes), so the golden
/// reads the same.
fn code(c: Conv) -> u32 {
    match c {
        Conv::Copy => 0,
        Conv::D3d9(v) => v.d3d9().raw(),
        U16N => 0x1601,
        S16N => 0x1602,
        BC4U => 0xbc41,
        BC4S => 0xbc42,
        BC5U => 0xbc51,
        BC5S => 0xbc52,
        BC6U => 0xbc61,
        BC6S => 0xbc62,
        BC7 => 0xbc70,
        RGB32F => 0x9601,
        RGB32I => 0x9602,
        RGBG => 0x6801,
        GRGB => 0x6901,
        XR_BIAS => 0x8901,
    }
}

const SEED: u64 = 0xcbf2_9ce4_8422_2325;

fn line(dxgi: u32, all: &[u32]) -> String {
    let mut rng = Rng(0x5eed_0000_0000_0011 ^ u64::from(dxgi).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let mut s = format!("{dxgi}");
    match info(dxgi) {
        Some(f) => s += &format!(" page={} bpp={} block={} conv={:#x} noalpha={}", f.page, f.bpp, f.block, code(f.conv), u8::from(f.no_alpha)),
        None => s += " none",
    }
    s += &format!(" support={:#x} page={:?}", support(dxgi), [0, 8, 0x20, 0x40, 0x48].map(|b| page(dxgi, b)));
    s += &format!(" pitch={:?}", [(1, 1), (7, 5), (16, 16), (13, 0)].map(|(w, h)| pitch_rows(dxgi, w, h)));
    s += &format!(" bytes={:?}", [(4, 4, 1, 1, 1, 1), (17, 9, 3, 2, 5, 1), (16384, 16384, 1, 2048, 15, 8), (u32::MAX, 3, 1, 1, 40, 1)].map(|(w, h, d, l, m, n)| (texture_bytes(dxgi, w, h, d, l, m, n), page_texture_bytes(dxgi, w, h, d, l, m, n))));
    s += &format!(" vertex={:?} family={} back={}", vertex_format(dxgi), family(dxgi), u8::from(packs_back(dxgi)));
    let mut h = SEED;
    for &b in all {
        fnv(&mut h, &[u8::from(copy_compatible(dxgi, b))]);
    }
    s += &format!(" copy={h:016x}");
    // from_page of random page texels
    let (w, hgt) = (9u32, 7u32);
    let texels: Vec<u8> = (0..w as usize * hgt as usize * 16).map(|_| rng.byte()).collect();
    let mut h = SEED;
    match from_page(dxgi, &texels, w, hgt) {
        Some(v) => fnv(&mut h, &v),
        None => fnv(&mut h, b"none"),
    }
    s += &format!(" from_page={h:016x}");
    // to_page of random guest texels, for each page format a texture of it is kept as (page():
    // bound for depth or not)
    if let Some(f) = info(dxgi) {
        let (pitch, rows) = pitch_rows(dxgi, w, hgt);
        let pitch = pitch + 4;
        let src: Vec<u8> = (0..pitch as usize * rows as usize + 64).map(|_| rng.byte()).collect();
        let mut h = SEED;
        for p in [page(dxgi, 0), page(dxgi, 0x40)].into_iter().flatten() {
            let (v, bpr) = unsafe { to_page(&f, p, src.as_ptr(), pitch, w, hgt) };
            fnv(&mut h, &bpr.to_le_bytes());
            fnv(&mut h, &v);
        }
        s += &format!(" to_page={h:016x}");
    }
    s
}

#[test]
fn every_format_answers_as_pinned() {
    let mut fmts: Vec<u32> = (0..=200).collect();
    fmts.extend([0x100, 0xffff, u32::MAX]);
    let mut got = String::new();
    for &f in &fmts {
        got += &line(f, &fmts);
        got.push('\n');
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/d3d11/format_golden.txt");
    kome::golden::check(&path, &got);
}

/// D3D11_BLEND (0..=20 and far past them) to the pipeline key's blend factor, as a color factor
/// and as an alpha factor (render::d3d BLEND2 codes).
#[test]
fn blend_factors_as_pinned() {
    let codes: Vec<u32> = (0..=20).chain([0x8001, u32::MAX]).collect();
    let got = [false, true].map(|alpha| codes.iter().map(|&b| super::blend_factor(b, alpha).code().to_string()).collect::<Vec<_>>().join(" "));
    assert_eq!(got, ["1 0 1 2 3 4 5 6 7 8 9 10 1 1 11 12 1 1 1 1 1 1 1", "1 0 1 4 5 4 5 6 7 6 7 10 1 1 11 12 1 1 1 1 1 1 1"]);
}
