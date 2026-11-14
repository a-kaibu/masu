//! View's pixel access against the code as it was before the bit depth became a type (`Old`,
//! its methods copied unchanged): every depth a DIB header can carry, both 16 bpp layouts,
//! palettes of 0..=300 entries, random rows and random reads and writes, compared value by value
//! and byte by byte.
use super::*;
use proptest::prelude::*;

struct Old {
    top: *mut u8,
    stride: isize,
    bpp: u16,
    r565: bool,
    pal: Vec<u32>,
}

impl Old {
    #[inline]
    fn row(&self, y: i32) -> *mut u8 {
        unsafe { self.top.offset(y as isize * self.stride) }
    }
    /// The pixel as 0xRRGGBB (32 bpp: all four bytes).
    #[inline]
    fn get(&self, x: i32, y: i32) -> u32 {
        let r = self.row(y);
        unsafe {
            match self.bpp {
                32 => (r as *const u32).add(x as usize).read_unaligned(),
                24 => {
                    let p = r.add(3 * x as usize);
                    (*p.add(2) as u32) << 16 | (*p.add(1) as u32) << 8 | *p as u32
                }
                16 => {
                    let v = (r as *const u16).add(x as usize).read_unaligned() as u32;
                    if self.r565 {
                        let (r5, g6, b5) = (v >> 11 & 31, v >> 5 & 63, v & 31);
                        (r5 << 3 | r5 >> 2) << 16 | (g6 << 2 | g6 >> 4) << 8 | (b5 << 3 | b5 >> 2)
                    } else {
                        let (r5, g5, b5) = (v >> 10 & 31, v >> 5 & 31, v & 31);
                        (r5 << 3 | r5 >> 2) << 16 | (g5 << 3 | g5 >> 2) << 8 | (b5 << 3 | b5 >> 2)
                    }
                }
                8 => self.pal_at(*r.add(x as usize) as usize),
                4 => {
                    let b = *r.add(x as usize / 2);
                    self.pal_at(if x & 1 == 0 { b >> 4 } else { b & 15 } as usize)
                }
                1 => {
                    let b = *r.add(x as usize / 8);
                    self.pal_at((b >> (7 - (x & 7)) & 1) as usize)
                }
                _ => 0,
            }
        }
    }
    /// The pixel as stored (32/24 bpp: the color, 16: the 16-bit value, palettes: the index).
    /// Raster ops work on these, as in Windows (an indexed DIB's ops combine indices).
    #[inline]
    fn raw(&self, x: i32, y: i32) -> u32 {
        let r = self.row(y);
        unsafe {
            match self.bpp {
                32 => (r as *const u32).add(x as usize).read_unaligned(),
                24 => {
                    let p = r.add(3 * x as usize);
                    (*p.add(2) as u32) << 16 | (*p.add(1) as u32) << 8 | *p as u32
                }
                16 => (r as *const u16).add(x as usize).read_unaligned() as u32,
                8 => *r.add(x as usize) as u32,
                4 => (*r.add(x as usize / 2) >> if x & 1 == 0 { 4 } else { 0 } & 15) as u32,
                1 => (*r.add(x as usize / 8) >> (7 - (x & 7)) & 1) as u32,
                _ => 0,
            }
        }
    }
    #[inline]
    fn set_raw(&self, x: i32, y: i32, v: u32) {
        let r = self.row(y);
        unsafe {
            match self.bpp {
                32 => (r as *mut u32).add(x as usize).write_unaligned(v),
                24 => {
                    let p = r.add(3 * x as usize);
                    *p = v as u8;
                    *p.add(1) = (v >> 8) as u8;
                    *p.add(2) = (v >> 16) as u8;
                }
                16 => (r as *mut u16).add(x as usize).write_unaligned(v as u16),
                8 => *r.add(x as usize) = v as u8,
                4 => {
                    let p = r.add(x as usize / 2);
                    let i = (v & 15) as u8;
                    *p = if x & 1 == 0 { (*p & 0x0f) | i << 4 } else { (*p & 0xf0) | i };
                }
                1 => {
                    let p = r.add(x as usize / 8);
                    let m = 0x80u8 >> (x & 7);
                    *p = if v & 1 != 0 { *p | m } else { *p & !m };
                }
                _ => {}
            }
        }
    }
    /// A solid color (0xRRGGBB) as a stored pixel: palettes take the nearest entry.
    fn pixel(&self, c: u32) -> u32 {
        match self.bpp {
            32 | 24 => c & 0xffffff,
            16 if self.r565 => (c >> 19 & 31) << 11 | (c >> 10 & 63) << 5 | (c >> 3 & 31),
            16 => (c >> 19 & 31) << 10 | (c >> 11 & 31) << 5 | (c >> 3 & 31),
            _ => self.index_of(c),
        }
    }
    /// The COLORREF (0xBBGGRR) a stored pixel shows
    fn color_of(&self, px: u32) -> u32 {
        let c = match self.bpp {
            32 | 24 => px & 0xffffff,
            16 => {
                let x5 = |v: u32| v << 3 | v >> 2;
                let (r, g, b) = if self.r565 {
                    let g6 = px >> 5 & 63;
                    (px >> 11 & 31, g6 << 2 | g6 >> 4, px & 31)
                } else {
                    (px >> 10 & 31, x5(px >> 5 & 31), px & 31)
                };
                x5(r) << 16 | g << 8 | x5(b)
            }
            _ => self.pal_at(px as usize),
        };
        (c & 0xff) << 16 | (c & 0xff00) | (c >> 16 & 0xff)
    }
    /// The bits a stored pixel has
    fn raw_mask(&self) -> u32 {
        if self.bpp >= 32 {
            u32::MAX
        } else {
            (1u32 << self.bpp) - 1
        }
    }
    fn pal_at(&self, i: usize) -> u32 {
        self.pal.get(i).copied().unwrap_or(if i == 0 { 0 } else { 0xffffff })
    }
    fn index_of(&self, c: u32) -> u32 {
        let c = c & 0xffffff;
        if self.bpp == 1 && self.pal.len() == 2 && self.pal[0] == 0 && self.pal[1] == 0xffffff {
            return (c != 0) as u32;
        }
        let mut best = (u32::MAX, 0);
        for (i, &p) in self.pal.iter().enumerate() {
            let d = |s: u32| ((c >> s & 255) as i32 - (p >> s & 255) as i32).pow(2) as u32;
            let e = d(16) + d(8) + d(0);
            if e < best.0 {
                best = (e, i as u32);
                if e == 0 {
                    break;
                }
            }
        }
        best.1
    }
    #[inline]
    fn put(&self, x: i32, y: i32, c: u32) {
        let r = self.row(y);
        unsafe {
            match self.bpp {
                32 => (r as *mut u32).add(x as usize).write_unaligned(c),
                24 => {
                    let p = r.add(3 * x as usize);
                    *p = c as u8;
                    *p.add(1) = (c >> 8) as u8;
                    *p.add(2) = (c >> 16) as u8;
                }
                16 => {
                    let v = if self.r565 {
                        (c >> 19 & 31) << 11 | (c >> 10 & 63) << 5 | (c >> 3 & 31)
                    } else {
                        (c >> 19 & 31) << 10 | (c >> 11 & 31) << 5 | (c >> 3 & 31)
                    };
                    (r as *mut u16).add(x as usize).write_unaligned(v as u16)
                }
                8 => *r.add(x as usize) = self.index_of(c) as u8,
                4 => {
                    let i = self.index_of(c) as u8;
                    let p = r.add(x as usize / 2);
                    *p = if x & 1 == 0 { (*p & 0x0f) | i << 4 } else { (*p & 0xf0) | i };
                }
                1 => {
                    let i = self.index_of(c) as u8;
                    let p = r.add(x as usize / 8);
                    let m = 0x80u8 >> (x & 7);
                    *p = if i != 0 { *p | m } else { *p & !m };
                }
                _ => {}
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Get,
    Raw,
    SetRaw(u32),
    Put(u32),
    Pixel(u32),
    ColorOf(u32),
    Mask,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::Get),
        Just(Op::Raw),
        any::<u32>().prop_map(Op::SetRaw),
        any::<u32>().prop_map(Op::Put),
        any::<u32>().prop_map(Op::Pixel),
        prop_oneof![any::<u32>(), 0u32..300].prop_map(Op::ColorOf),
        Just(Op::Mask),
    ]
}

fn run_old(o: &Old, x: i32, y: i32, op: Op) -> u32 {
    match op {
        Op::Get => o.get(x, y),
        Op::Raw => o.raw(x, y),
        Op::SetRaw(v) => {
            o.set_raw(x, y, v);
            0
        }
        Op::Put(v) => {
            o.put(x, y, v);
            0
        }
        Op::Pixel(c) => o.pixel(c),
        Op::ColorOf(p) => o.color_of(p),
        Op::Mask => o.raw_mask(),
    }
}

fn run_new(v: &View, x: i32, y: i32, op: Op) -> u32 {
    match op {
        Op::Get => v.get(x, y),
        Op::Raw => v.raw(x, y),
        Op::SetRaw(c) => {
            v.set_raw(x, y, c);
            0
        }
        Op::Put(c) => {
            v.put(x, y, c);
            0
        }
        Op::Pixel(c) => v.pixel(c),
        Op::ColorOf(p) => v.color_of(p),
        Op::Mask => v.raw_mask(),
    }
}

/// a view over `buf` as View::of makes it (the new code's way of naming the depth)
fn new_view(buf: &mut [u8], w: i32, h: i32, stride: isize, up: bool, bpp: u16, r565: bool, pal: &[u32]) -> View {
    let top = if up { unsafe { buf.as_mut_ptr().offset((h as isize - 1) * stride) } } else { buf.as_mut_ptr() };
    View { top, stride: if up { -stride } else { stride }, w, h, bpp: Depth::of(bpp), r565, pal: pal.to_vec(), mono: false }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]
    #[test]
    fn pixels_as_before(
        bpp in prop_oneof![prop::sample::select(vec![0u16, 1, 2, 4, 8, 15, 16, 24, 32, 48, 64]), any::<u16>()],
        r565: bool, up: bool, w in 1i32..20, h in 1i32..4, npal in 0usize..=300, seed: u64,
        ops in prop::collection::vec((0i32..20, 0i32..4, op()), 1..60),
    ) {
        let mut rng = seed | 1;
        let mut next = || { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; rng };
        // rows wide enough for 64 bpp: depths GDI leaves alone must not be read past either
        let stride = (w as isize * 8 + 3) & !3;
        let init: Vec<u8> = (0..stride as usize * h as usize).map(|_| next() as u8).collect();
        let pal: Vec<u32> = (0..npal).map(|_| next() as u32 & if next() & 3 == 0 { 0xffff_ffff } else { 0xffffff }).collect();
        let pal = if npal == 2 && seed & 4 != 0 { vec![0, 0xffffff] } else { pal };
        let mut a = init.clone();
        let mut b = init;
        let top = if up { unsafe { a.as_mut_ptr().offset((h as isize - 1) * stride) } } else { a.as_mut_ptr() };
        let old = Old { top, stride: if up { -stride } else { stride }, bpp, r565, pal: pal.clone() };
        let new = new_view(&mut b, w, h, stride, up, bpp, r565, &pal);
        for (i, &(x, y, op)) in ops.iter().enumerate() {
            let (x, y) = (x % w, y % h);
            let (o, n) = (run_old(&old, x, y, op), run_new(&new, x, y, op));
            prop_assert_eq!(o, n, "op {} {:?} at ({}, {})", i, op, x, y);
        }
        drop(new);
        prop_assert!(a == b, "rows differ");
    }
}

/// Every u16 names the depth it was: the named ones for 1/4/8/16/24/32, `Other` keeping the rest.
#[test]
fn depth_round_trips_every_u16() {
    for b in 0..=u16::MAX {
        let d = Depth::of(b);
        assert_eq!(d.bits(), b);
        assert_eq!(matches!(d, Depth::Other(_)), ![1, 4, 8, 16, 24, 32].contains(&b), "{b}");
        assert_eq!(d.indexed(), b <= 8);
    }
}
