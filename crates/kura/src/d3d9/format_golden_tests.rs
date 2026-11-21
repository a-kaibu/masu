//! Every answer of d3d9::format pinned, before its tables change shape (D3DFORMATs as an enum the
//! compiler checks every table against instead of u32 constants and `_` arms): for every format
//! number 0..=255, the DXTn FOURCCs and a few out-of-range ones, each table's answer and a hash
//! of what texel, pack, page_texel, target_color/target_clear, convert (and decode_dxt) make of
//! the same random inputs. format_golden.txt holds them; MASU_UPDATE_GOLDEN=1 rewrites it.
use super::format::*;

fn fnv(h: &mut u64, bytes: &[u8]) {
    *h = kome::golden::Fnv(*h).bytes(bytes).get();
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
    fn float(&mut self) -> f32 {
        let x = self.next();
        match x % 8 {
            0 => f32::NAN,
            1 => f32::INFINITY,
            2 => -2.5,
            3 => 1.5,
            _ => f32::from_bits(0x3f80_0000 | (x >> 41) as u32) - 1.25,
        }
    }
}

fn line(fmt: u32) -> String {
    let mut rng = Rng(0x5eed_0000_0000_0001 ^ u64::from(fmt).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let mut s = format!(
        "{fmt:#x} sup={} depth={} sten={} rt={} layout={:?} pitch1={:?} pitch75={:?} pitch13={:?} page={} bpp={} chan={:?} steps={:?} x={} noalpha={} same={}",
        u8::from(supported(fmt)),
        u8::from(is_depth(fmt)),
        u8::from(has_stencil(fmt)),
        u8::from(renderable(fmt)),
        layout(fmt),
        pitch_rows(fmt, 1, 1),
        pitch_rows(fmt, 7, 5),
        pitch_rows(fmt, 13, 0),
        page_format(fmt),
        page_bpp(fmt),
        target_channels(fmt),
        target_steps(fmt),
        u8::from(target_x(fmt)),
        u8::from(no_alpha(fmt)),
        u8::from(same_on_page(fmt)),
    );
    // texel of random bytes, page_texel of random page bytes
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for _ in 0..64 {
        let p: [u8; 16] = std::array::from_fn(|_| rng.byte());
        for v in texel(fmt, &p) {
            fnv(&mut h, &v.to_bits().to_le_bytes());
        }
        for v in page_texel(fmt, &p) {
            fnv(&mut h, &v.to_bits().to_le_bytes());
        }
    }
    s += &format!(" texel={h:016x}");
    // pack of random colors (what it writes and whether it knew the format)
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for _ in 0..64 {
        let c = [rng.float(), rng.float(), rng.float(), rng.float()];
        let mut out = [0xa5u8; 20];
        let ok = pack(fmt, c, &mut out);
        fnv(&mut h, &[u8::from(ok)]);
        fnv(&mut h, &out);
    }
    s += &format!(" pack={h:016x}");
    // clears
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for c in [0u32, 0xffff_ffff, 0x8040_c020, 0x7f80_0181, 0x0012_3456] {
        fnv(&mut h, &target_color(fmt, c).to_le_bytes());
        for v in target_clear(fmt, c) {
            fnv(&mut h, &v.to_bits().to_le_bytes());
        }
    }
    s += &format!(" clear={h:016x}");
    // convert of a random 9 x 7 region (block formats: their blocks), rows padded
    let (w, hgt) = (9u32, 7u32);
    let (pitch, rows) = pitch_rows(fmt, w, hgt);
    let src: Vec<u8> = (0..pitch as usize * rows as usize + 16).map(|_| rng.byte()).collect();
    let mut out = vec![0x11u8; 3];
    unsafe { convert(fmt, src.as_ptr(), pitch as usize, w, hgt, &mut out) };
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    fnv(&mut h, &out);
    s += &format!(" convert={}:{h:016x}", out.len());
    if matches!(fmt, DXT1 | DXT2 | DXT3 | DXT4 | DXT5) {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for _ in 0..64 {
            let b: [u8; 16] = std::array::from_fn(|_| rng.byte());
            for p in decode_dxt(fmt, &b) {
                fnv(&mut h, &p);
            }
        }
        s += &format!(" dxt={h:016x}");
    }
    s
}

#[test]
fn every_format_answers_as_pinned() {
    let mut fmts: Vec<u32> = (0..=255).collect();
    fmts.extend([DXT1, DXT2, DXT3, DXT4, DXT5, u32::from_le_bytes(*b"DXT6"), u32::from_le_bytes(*b"ATI2"), 0x1_0000, u32::MAX]);
    let mut got = String::new();
    for f in fmts {
        got += &line(f);
        got.push('\n');
    }
    // the P8 palette lookup
    let pal: [u8; 1024] = std::array::from_fn(|i| (i * 37 % 251) as u8);
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for i in 0..=255u8 {
        fnv(&mut h, &p8_texel(&pal, i));
    }
    let idx: Vec<u8> = (0..=255u8).collect();
    let mut out = Vec::new();
    unsafe { convert_p8(idx.as_ptr(), 32, 16, 8, &pal, &mut out) };
    fnv(&mut h, &out);
    got += &format!("p8 {h:016x}\n");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/d3d9/format_golden.txt");
    kome::golden::check(&path, &got);
}
