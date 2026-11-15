//! DirectDraw pinned as a whole before its code is rewritten with types: seeded random programs
//! on system-memory surfaces of every format sake makes (8 bpp with a palette, 16 bpp 5-6-5 and
//! 5-5-5, 24, 32, the display's): Blt (color fills, stretches, mirrors, raster ops, source and
//! destination keys and their overrides, whole surfaces and rectangles in and out of bounds),
//! BltFast with keys, SetColorKey/GetColorKey, Lock and writes through it, GetCaps,
//! GetSurfaceDesc/GetPixelFormat, palettes. Every call's answer and, after it, a hash of every
//! surface's bytes and of what the call filled go into ddraw/api_golden.txt (addresses left
//! out). MASU_UPDATE_GOLDEN=1 rewrites the file.
use super::*;
use crate::mem::{bytes, w16, w32, w8};
use crate::test_guest::{call, enter, guest_thread};
use crate::api::Handler;

thread_local!(static LIVE: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) });

fn heap_alloc(n: u32, zero: bool) -> u32 {
    let p = mem::heap_alloc(n, zero);
    LIVE.with(|l| l.borrow_mut().push(p));
    p
}
fn heap_free(p: u32) {
    LIVE.with(|l| l.borrow_mut().retain(|&q| q != p));
    mem::heap_free(p);
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
    fn range(&mut self, a: i32, b: i32) -> u32 {
        (a + self.below((b - a + 1) as u32) as i32) as u32
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len() as u32) as usize]
    }
}

fn fnv(h: &mut u64, b: &[u8]) {
    for &x in b {
        *h = (*h ^ x as u64).wrapping_mul(0x100_0000_01b3);
    }
}

struct World {
    rng: Rng,
    out: Vec<String>,
    names: Vec<(u32, String)>,
    /// the surfaces' interfaces, the size each was asked for
    s: Vec<(u32, i32, i32)>,
    pals: Vec<u32>,
    buf: u32,
}

impl World {
    fn name(&mut self, h: u32, n: &str) -> u32 {
        if h != 0 {
            let k = self.names.len();
            self.names.push((h, format!("{n}{k}")));
        }
        h
    }
    fn show(&self, v: u32) -> String {
        match self.names.iter().rev().find(|(h, _)| *h == v) {
            Some((_, n)) => n.clone(),
            None if LIVE.with(|l| l.borrow().contains(&v)) => "ptr".into(),
            None => format!("{v:#x}"),
        }
    }
    fn log(&mut self, what: &str, args: &[u32], r: u32, extra: &[u8]) {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for &(i, _, _) in &self.s {
            let id = with(|st| st.blocks.get(&i).map(|b| b.0));
            if let Some(x) = id.and_then(surf) {
                fnv(&mut h, &bytes(x.bits, x.pitch * x.h as u32));
                for v in [x.ck_src, x.ck_dst].into_iter().flatten() {
                    fnv(&mut h, &v.0.to_le_bytes());
                    fnv(&mut h, &v.1.to_le_bytes());
                }
            }
        }
        fnv(&mut h, extra);
        let a: Vec<String> = args.iter().map(|&a| self.show(a)).collect();
        let line = format!("{what}({}) = {} #{:016x}", a.join(","), self.show(r), h);
        self.out.push(line);
    }
    fn go(&mut self, what: &str, f: Handler, args: &[u32]) -> u32 {
        let r = call(f, args);
        self.log(what, args, r, &[]);
        r
    }
    fn pick(&mut self) -> (u32, i32, i32) {
        let i = self.rng.below(self.s.len() as u32) as usize;
        self.s[i]
    }
    /// a surface of `d`'s depth (another depth goes through GDI by guest calls, which a test
    /// thread cannot make)
    fn pick_like(&mut self, d: u32) -> (u32, i32, i32) {
        let bpp = |i: u32| with(|st| st.blocks.get(&i).map(|b| b.0)).and_then(surf).map(|x| x.bpp);
        let like: Vec<(u32, i32, i32)> = self.s.iter().copied().filter(|x| bpp(x.0) == bpp(d)).collect();
        self.rng.pick(&like)
    }
    /// a rectangle near (and sometimes past) a w x h surface, or none
    fn rect(&mut self, w: i32, h: i32) -> u32 {
        if self.rng.below(4) == 0 {
            return 0;
        }
        let p = heap_alloc(16, true);
        let (l, t, r, b) = if self.rng.below(6) == 0 {
            (self.rng.range(-3, w), self.rng.range(-3, h), self.rng.range(-3, w + 3), self.rng.range(-3, h + 3))
        } else {
            // inside: most calls get to the pixels
            let (l, t) = (self.rng.range(0, w - 1), self.rng.range(0, h - 1));
            (l, t, self.rng.range(l as i32 + 1, w), self.rng.range(t as i32 + 1, h))
        };
        for (i, v) in [l, t, r, b].into_iter().enumerate() {
            w32(p + 4 * i as u32, v);
        }
        p
    }
    fn key(&mut self) -> u32 {
        // few distinct values: keys that catch pixels
        let v = self.rng.next() & self.rng.pick(&[0x83838383, 0xff, 0x0101, 0, 0xffffffff]);
        v
    }

    fn setup(&mut self) {
        let pdd = heap_alloc(8, true);
        call(DirectDrawCreateEx, &[0, pdd, 0, 0]);
        let dd = mem::r32(pdd);
        self.name(dd, "dd");
        // (bpp, flags, r, g, b, a) of DDPIXELFORMAT; None: the display's format
        let fmts: [Option<(u32, u32, u32, u32, u32, u32)>; 7] = [
            Some((8, 0x60, 0, 0, 0, 0)),
            Some((16, 0x40, 0xf800, 0x07e0, 0x001f, 0)),
            Some((16, 0x40, 0x7c00, 0x03e0, 0x001f, 0)),
            Some((24, 0x40, 0xff0000, 0xff00, 0xff, 0)),
            Some((32, 0x40, 0xff0000, 0xff00, 0xff, 0)),
            Some((32, 0x41, 0xff0000, 0xff00, 0xff, 0xff00_0000)),
            None,
        ];
        let desc = heap_alloc(124, true);
        let out = heap_alloc(8, true);
        for (k, f) in fmts.into_iter().enumerate() {
            for (w, h) in [(21, 13), (9, 7)] {
                if k >= 5 && w == 9 {
                    continue;
                }
                mem::zero(desc, 124);
                w32(desc, 124);
                w32(desc + 4, 1 | 2 | 4 | if f.is_some() { 0x1000 } else { 0 });
                w32(desc + 8, h as u32);
                w32(desc + 12, w as u32);
                w32(desc + 104, 0x40 | 0x800);
                if let Some((bpp, fl, r, g, b, a)) = f {
                    w32(desc + 72, 32);
                    w32(desc + 76, fl);
                    w32(desc + 84, bpp);
                    w32(desc + 88, r);
                    w32(desc + 92, g);
                    w32(desc + 96, b);
                    w32(desc + 100, a);
                }
                let r = call(D_CreateSurface, &[dd, desc, out, 0]);
                let s = mem::r32(out);
                self.name(s, &format!("s{}_", f.map_or(0, |f| f.0)));
                self.log("CreateSurface", &[dd, f.map_or(0, |f| f.0), w as u32], r, &[]);
                if r == DD_OK {
                    // random pixels through Lock
                    let lr = call(S_Lock, &[s, 0, desc, 1, 0]);
                    assert_eq!(lr, DD_OK);
                    let (pitch, bits) = (mem::r32(desc + 16), mem::r32(desc + 36));
                    for i in 0..pitch * h as u32 {
                        w8(bits + i, (self.rng.next() & 0x87) as u8);
                    }
                    call(S_Unlock, &[s, 0]);
                    self.s.push((s, w, h));
                }
            }
        }
        // palettes for the 8 bpp surfaces
        let ent = heap_alloc(1024, true);
        for i in 0..256 {
            w32(ent + 4 * i, self.rng.next() & 0xffffff);
        }
        for _ in 0..2 {
            let r = call(D_CreatePalette, &[dd, 4 | 0x40, ent, out, 0]);
            let p = mem::r32(out);
            self.name(p, "pal");
            self.log("CreatePalette", &[dd], r, &[]);
            if r == DD_OK {
                self.pals.push(p);
            }
        }
        heap_free(ent);
        heap_free(desc);
        heap_free(out);
        heap_free(pdd);
        self.buf = heap_alloc(4096, true);
    }

    fn step(&mut self) {
        let (d, dw, dh) = self.pick();
        match self.rng.below(16) {
            0..=4 => {
                let (s, sw, sh) = if self.rng.below(5) == 0 { (d, dw, dh) } else { self.pick_like(d) };
                let dr = self.rect(dw, dh);
                let sr = self.rect(sw, sh);
                let mut flags = 0x0100_0000;
                for (bit, p) in [(0x400u32, 5u32), (0x800, 4), (0x8000, 3), (0x2000, 8), (0x10000, 5), (0x4000, 10), (0x20000, 8), (0x200_0000, 20)] {
                    if self.rng.below(p) == 0 {
                        flags |= bit;
                    }
                }
                let fx = heap_alloc(100, true);
                w32(fx, if self.rng.below(10) == 0 { 0 } else { 100 });
                w32(fx + 4, self.rng.pick(&[0, 2, 4, 6, 8, 0x10]));
                let any = self.rng.next();
                w32(fx + 8, self.rng.pick(&[0xcc0020, 0xf00021, 0x000042, 0xff0062, 0x660046, any]));
                w32(fx + 80, self.rng.next());
                let (a, b, c, e) = (self.key(), self.key(), self.key(), self.key());
                w32(fx + 84, a);
                w32(fx + 88, if self.rng.below(2) == 0 { a } else { b });
                w32(fx + 92, c);
                w32(fx + 96, if self.rng.below(2) == 0 { c } else { e });
                let src = if flags & 0x400 != 0 && self.rng.below(2) == 0 { 0 } else { s };
                self.go("Blt", S_Blt, &[d, dr, src, sr, flags, fx]);
                for p in [dr, sr, fx] {
                    if p != 0 {
                        heap_free(p);
                    }
                }
            }
            5 | 6 => {
                let (s, sw, sh) = self.pick_like(d);
                let sr = self.rect(sw, sh);
                let (rw, rh) = if sr == 0 { (sw, sh) } else { (mem::r32(sr + 8) as i32 - mem::r32(sr) as i32, mem::r32(sr + 12) as i32 - mem::r32(sr + 4) as i32) };
                let (x, y) = if self.rng.below(5) == 0 || rw > dw || rh > dh { (self.rng.range(-3, dw), self.rng.range(-3, dh)) } else { (self.rng.range(0, dw - rw), self.rng.range(0, dh - rh)) };
                let trans = self.rng.pick(&[0, 1, 2, 3, 0x10, 0x11, 0x12]);
                self.go("BltFast", S_BltFast, &[d, x, y, s, sr, trans]);
                if sr != 0 {
                    heap_free(sr);
                }
            }
            7 | 8 => {
                let ck = heap_alloc(8, true);
                let (a, b) = (self.key(), self.key());
                w32(ck, a);
                w32(ck + 4, if self.rng.below(2) == 0 { a } else { b });
                let flags = self.rng.pick(&[8, 2, 1 | 8, 4, 0x10, 0]);
                let p = if self.rng.below(6) == 0 { 0 } else { ck };
                self.go("SetColorKey", S_SetColorKey, &[d, flags, p]);
                heap_free(ck);
            }
            9 => {
                let flags = self.rng.pick(&[8, 2, 4, 0x10]);
                mem::zero(self.buf, 8);
                let r = call(S_GetColorKey, &[d, flags, self.buf]);
                self.log("GetColorKey", &[d, flags], r, &bytes(self.buf, 8));
            }
            10 | 11 => {
                let r = self.rect(dw, dh);
                mem::zero(self.buf, 124);
                w32(self.buf, 124);
                let v = call(S_Lock, &[d, r, self.buf, 1, 0]);
                let mut e = bytes(self.buf, 124).to_vec();
                e[36..40].fill(0);
                self.log("Lock", &[d, r], v, &e);
                if v == DD_OK {
                    let (pitch, bits) = (mem::r32(self.buf + 16), mem::r32(self.buf + 36));
                    let (n, rows) = (self.rng.range(1, 8), self.rng.range(1, 3));
                    for y in 0..rows {
                        for i in 0..n {
                            w8(bits + y * pitch + i, self.rng.next() as u8);
                        }
                    }
                    self.go("Unlock", S_Unlock, &[d, r]);
                }
                if r != 0 {
                    heap_free(r);
                }
            }
            12 => {
                mem::zero(self.buf, 108);
                let v = call(S_GetCaps, &[d, self.buf]);
                self.log("GetCaps", &[d], v, &bytes(self.buf, 16));
            }
            13 => {
                mem::zero(self.buf, 124);
                w32(self.buf, self.rng.pick(&[124, 108, 0]));
                let v = call(S_GetSurfaceDesc, &[d, self.buf]);
                let mut e = bytes(self.buf, 124).to_vec();
                e[36..40].fill(0);
                self.log("GetSurfaceDesc", &[d], v, &e);
                mem::zero(self.buf, 32);
                w32(self.buf, 32);
                let v = call(S_GetPixelFormat, &[d, self.buf]);
                self.log("GetPixelFormat", &[d], v, &bytes(self.buf, 32));
            }
            14 => {
                if !self.pals.is_empty() {
                    let p = if self.rng.below(5) == 0 { 0 } else { self.rng.pick(&self.pals.clone()) };
                    self.go("SetPalette", S_SetPalette, &[d, p]);
                }
            }
            _ => {
                if !self.pals.is_empty() {
                    let p = self.rng.pick(&self.pals.clone());
                    let (s, n) = (self.rng.range(0, 255), self.rng.range(0, 40));
                    for i in 0..n {
                        w32(self.buf + 4 * i, self.rng.next() & 0xffffff);
                    }
                    self.go("SetEntries", P_SetEntries, &[p, 0, s, n, self.buf]);
                    mem::zero(self.buf, 1024);
                    let v = call(P_GetEntries, &[p, 0, 0, 256, self.buf]);
                    self.log("GetEntries", &[p], v, &bytes(self.buf, 1024));
                }
            }
        }
        let _ = w16;
    }
}

fn program(seed: u64, steps: usize) -> Vec<String> {
    let mut w = World { rng: Rng(seed), out: vec![format!("seed {seed:#x}")], names: Vec::new(), s: Vec::new(), pals: Vec::new(), buf: 0 };
    w.setup();
    for _ in 0..steps {
        w.step();
    }
    w.out
}

#[test]
fn ddraw_programs_as_before() {
    let _g = enter();
    guest_thread();
    crate::gdi::init();
    with(|s| *s = fresh());
    let mut out = Vec::new();
    for seed in [0x9e37_79b9_7f4a_7c15u64, 0xdead_beef_cafe_f00d, 0x0123_4567_89ab_cdef] {
        out.extend(program(seed, 1200));
    }
    let out = out.join("\n") + "\n";
    let golden = concat!(env!("CARGO_MANIFEST_DIR"), "/src/ddraw/api_golden.txt");
    kome::golden::check(golden, &out);
}
