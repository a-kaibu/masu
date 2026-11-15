//! GDI+ pinned as a whole before gdiplus*.rs are rewritten with types: seeded random programs
//! through the registered flat API (bitmaps of every format sake keeps and some it does not,
//! pixels in and out, LockBits/UnlockBits in every format and both directions, clones; solid,
//! hatch, texture, linear and path gradient brushes with their wrap modes, transforms, colors and
//! gamma; pens; fills and lines through a Graphics with rendering origins and page units;
//! matrices; paths). Every call's answer (objects by the program's names),
//! what it wrote back and a hash of every bitmap's pixels (read through LockBits as 32bppARGB)
//! go into gdiplus_api_golden.txt. MASU_UPDATE_GOLDEN=1 rewrites it.
use crate::mem::{self, slice, w32, w8};
use crate::test_guest::{call, enter, guest_thread};

/// The program's own buffers, bumped out of one region at a fixed address (reserved once), so
/// that the pointers in the log are the same whatever ran before
const ARENA: u32 = 0x0e00_0000;
const ARENA_SIZE: u32 = 1 << 20;
thread_local!(static NEXT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) });

fn heap_alloc(n: u32) -> u32 {
    NEXT.with(|c| {
        let p = ARENA + c.get();
        c.set(c.get() + (n + 15) & !15);
        assert!(c.get() <= ARENA_SIZE, "arena full");
        mem::zero(p, n);
        p
    })
}
fn heap_free(_: u32) {}

fn f(v: f32) -> u32 {
    v.to_bits()
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
    fn fl(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * (self.next() % 1001) as f32 / 1000.
    }
    fn argb(&mut self) -> u32 {
        match self.below(4) {
            0 => self.pick(&[0xff00_0000, 0xffff_ffff, 0x80ff_8040, 0, 0x7f00_00ff, 0xff12_3456]),
            _ => self.next(),
        }
    }
}

fn fnv(h: &mut u64, b: &[u8]) {
    for &x in b {
        *h = (*h ^ x as u64).wrapping_mul(0x100_0000_01b3);
    }
}

const FMTS: [u32; 10] = [0x0002_1808, 0x0002_2009, 0x0026_200a, 0x000e_200b, 0x0002_1005, 0x0002_1006, 0x0003_0101, 0x0003_0402, 0x0003_0803, 0x0010_3004];
const W: u32 = 13;
const H: u32 = 9;

struct World {
    rng: Rng,
    out: Vec<String>,
    names: Vec<(u32, String)>,
    bmps: Vec<u32>,
    gfx: Vec<u32>,
    brushes: Vec<u32>,
    pens: Vec<u32>,
    mats: Vec<u32>,
    paths: Vec<u32>,
    buf: u32,
    o: u32,
}

fn h(name: &str) -> crate::api::Handler {
    crate::api::find_impl("gdiplus", name).unwrap_or_else(|| panic!("gdiplus!{name}"))
}

impl World {
    fn show(&self, v: u32) -> String {
        match self.names.iter().rev().find(|(h, _)| *h == v) {
            Some((_, n)) => n.clone(),
            None => format!("{v:#x}"),
        }
    }
    fn log(&mut self, what: &str, args: &[u32], r: u32, extra: &[u8]) {
        let mut hh = 0xcbf2_9ce4_8422_2325u64;
        // every bitmap's pixels, as 32bppARGB through LockBits (read only)
        let data = heap_alloc(24);
        for &b in &self.bmps {
            if call(h("GdipBitmapLockBits"), &[b, 0, 1, 0x0026_200a, data]) == 0 {
                let (w, hgt, stride, scan0) = (mem::r32(data), mem::r32(data + 4), mem::r32(data + 8), mem::r32(data + 16));
                for y in 0..hgt {
                    fnv(&mut hh, slice(scan0 + y * stride, 4 * w));
                }
                call(h("GdipBitmapUnlockBits"), &[b, data]);
            }
        }
        heap_free(data);
        fnv(&mut hh, extra);
        let a: Vec<String> = args.iter().map(|&a| self.show(a)).collect();
        let line = format!("{what}({}) = {:#x} #{:016x}", a.join(","), r, hh);
        self.out.push(line);
    }
    /// a call that makes an object into `self.o`: named in the log, kept in `list`
    fn make(&mut self, what: &str, args: &[u32], kind: &str) -> Option<u32> {
        w32(self.o, 0);
        let r = call(h(what), args);
        let v = mem::r32(self.o);
        let made = (r == 0 && v != 0).then(|| {
            let k = self.names.len();
            self.names.push((v, format!("{kind}{k}")));
            v
        });
        self.log(what, args, r, &[]);
        made
    }
    fn go(&mut self, what: &str, args: &[u32]) -> u32 {
        let r = call(h(what), args);
        self.log(what, args, r, &[]);
        r
    }
    /// a getter's answer and the `n` bytes it wrote at `self.buf`
    fn get(&mut self, what: &str, args: &[u32], n: u32) {
        mem::zero(self.buf, n);
        let r = call(h(what), args);
        let e = slice(self.buf, n).to_vec();
        self.log(what, args, r, &e);
    }
    fn any(&mut self, v: &[u32]) -> u32 {
        if v.is_empty() { 0 } else { self.rng.pick(v) }
    }

    fn setup(&mut self) {
        for &fmt in &FMTS {
            let scan = heap_alloc(4 * W * H);
            let stride = (W * 4 + 3) & !3;
            for i in 0..4 * W * H {
                w8(scan + i, self.rng.next() as u8);
            }
            let src = if self.rng.below(2) == 0 { scan } else { 0 };
            if let Some(b) = self.make("GdipCreateBitmapFromScan0", &[W, H, stride, fmt, src, self.o], "bmp") {
                self.bmps.push(b);
            }
        }
        for &b in &self.bmps.clone() {
            if let Some(g) = self.make("GdipGetImageGraphicsContext", &[b, self.o], "gfx") {
                self.gfx.push(g);
            }
        }
        for _ in 0..3 {
            let c = self.rng.argb();
            if let Some(b) = self.make("GdipCreateSolidFill", &[c, self.o], "brush") {
                self.brushes.push(b);
            }
        }
        for s in [0, 5, 17, 52, 53] {
            let (a, b) = (self.rng.argb(), self.rng.argb());
            if let Some(x) = self.make("GdipCreateHatchBrush", &[s, a, b, self.o], "hatch") {
                self.brushes.push(x);
            }
        }
        for wrap in 0..5 {
            let img = self.any(&self.bmps.clone());
            if let Some(x) = self.make("GdipCreateTexture", &[img, wrap, self.o], "tex") {
                self.brushes.push(x);
            }
        }
        let pts = heap_alloc(64);
        for wrap in 0..5 {
            for i in 0..4 {
                w32(pts + 4 * i, self.rng.range(-3, 15));
            }
            let (a, b) = (self.rng.argb(), self.rng.argb());
            if let Some(x) = self.make("GdipCreateLineBrushI", &[pts, pts + 8, a, b, wrap, self.o], "line") {
                self.brushes.push(x);
            }
        }
        for wrap in 0..4 {
            let n = 3 + self.rng.below(3);
            for i in 0..2 * n {
                w32(pts + 4 * i, self.rng.range(-2, 14));
            }
            if let Some(x) = self.make("GdipCreatePathGradientI", &[pts, n, wrap, self.o], "pgrad") {
                self.brushes.push(x);
            }
        }
        heap_free(pts);
        for _ in 0..3 {
            let (c, w) = (self.rng.argb(), self.rng.fl(0., 4.));
            if let Some(p) = self.make("GdipCreatePen1", &[c, f(w), 0, self.o], "pen") {
                self.pens.push(p);
            }
        }
        for _ in 0..3 {
            let e: Vec<u32> = (0..6).map(|_| f(self.rng.fl(-2., 2.))).collect();
            if let Some(m) = self.make("GdipCreateMatrix2", &[e[0], e[1], e[2], e[3], e[4], e[5], self.o], "mat") {
                self.mats.push(m);
            }
        }
        for fm in 0..2 {
            if let Some(p) = self.make("GdipCreatePath", &[fm, self.o], "path") {
                self.paths.push(p);
            }
        }
    }

    fn step(&mut self) {
        let o = self.o;
        let buf = self.buf;
        match self.rng.below(30) {
            0 | 1 => {
                let (b, x, y, c) = (self.any(&self.bmps.clone()), self.rng.range(-1, W as i32), self.rng.range(-1, H as i32), self.rng.argb());
                self.go("GdipBitmapSetPixel", &[b, x, y, c]);
            }
            2 => {
                let (b, x, y) = (self.any(&self.bmps.clone()), self.rng.range(-1, W as i32), self.rng.range(-1, H as i32));
                self.get("GdipBitmapGetPixel", &[b, x, y, buf], 4);
            }
            3 | 4 => {
                // LockBits in some format, writes through it, UnlockBits
                let b = self.any(&self.bmps.clone());
                let fmt = if self.rng.below(5) == 0 { self.rng.next() } else { self.rng.pick(&FMTS) };
                let rect = if self.rng.below(3) == 0 { 0 } else {
                    let r = buf + 512;
                    let (x, y) = (self.rng.range(-1, W as i32 - 1), self.rng.range(-1, H as i32 - 1));
                    for (i, v) in [x, y, self.rng.range(0, W as i32), self.rng.range(0, H as i32)].into_iter().enumerate() {
                        w32(r + 4 * i as u32, v);
                    }
                    r
                };
                let flags = self.rng.pick(&[1, 2, 3, 1 | 4, 0]);
                let data = buf + 1024;
                mem::zero(data, 24);
                let r = call(h("GdipBitmapLockBits"), &[b, rect, flags, fmt, data]);
                let mut e = slice(data, 16).to_vec();
                if r == 0 {
                    let (w, hh, stride, scan0) = (mem::r32(data), mem::r32(data + 4), mem::r32(data + 8), mem::r32(data + 16));
                    for y in 0..hh {
                        e.extend_from_slice(slice(scan0 + y * stride, stride.min(4 * w)));
                    }
                    if flags & 2 != 0 {
                        for y in 0..hh {
                            for i in 0..stride.min(4 * w) {
                                if self.rng.below(3) == 0 {
                                    w8(scan0 + y * stride + i, self.rng.next() as u8);
                                }
                            }
                        }
                    }
                }
                self.log("GdipBitmapLockBits", &[b, rect, flags, fmt], r, &e);
                if r == 0 || self.rng.below(4) == 0 {
                    self.go("GdipBitmapUnlockBits", &[b, data]);
                }
            }
            5 => {
                let b = self.any(&self.bmps.clone());
                self.get("GdipGetImagePixelFormat", &[b, buf], 4);
                self.get("GdipGetImageFlags", &[b, buf], 4);
            }
            6..=10 => {
                let (g, b) = (self.any(&self.gfx.clone()), self.any(&self.brushes.clone()));
                if self.rng.below(2) == 0 {
                    let (x, y, w, hh) = (self.rng.range(-3, 12), self.rng.range(-3, 8), self.rng.range(-2, 15), self.rng.range(-2, 11));
                    self.go("GdipFillRectangleI", &[g, b, x, y, w, hh]);
                } else {
                    let v: Vec<u32> = [(-3., 12.), (-3., 8.), (-1., 15.), (-1., 11.)].iter().map(|&(a, z)| f(self.rng.fl(a, z))).collect();
                    self.go("GdipFillRectangle", &[g, b, v[0], v[1], v[2], v[3]]);
                }
            }
            11 | 12 => {
                let (g, p) = (self.any(&self.gfx.clone()), self.any(&self.pens.clone()));
                let v: Vec<u32> = (0..4).map(|_| self.rng.range(-3, 15)).collect();
                self.go("GdipDrawLineI", &[g, p, v[0], v[1], v[2], v[3]]);
            }
            13 => {
                let g = self.any(&self.gfx.clone());
                match self.rng.below(3) {
                    0 => {
                        let (x, y) = (self.rng.range(-5, 5), self.rng.range(-5, 5));
                        self.go("GdipSetRenderingOrigin", &[g, x, y]);
                    }
                    1 => {
                        let u = self.rng.range(0, 7);
                        self.go("GdipSetPageUnit", &[g, u]);
                    }
                    _ => {
                        let s = f(self.rng.pick(&[1., 0.5, 2., 0., -1.]));
                        self.go("GdipSetPageScale", &[g, s]);
                    }
                }
            }
            14 | 15 => {
                let b = self.any(&self.brushes.clone());
                let m = self.any(&self.mats.clone());
                let mode = self.rng.range(0, 5);
                let c = self.rng.argb();
                let (fl, ord) = (f(self.rng.fl(-90., 90.)), self.rng.range(0, 1));
                let (bit, s1, s2) = (self.rng.below(2), f(self.rng.fl(-2., 2.)), f(self.rng.fl(-2., 2.)));
                match self.rng.below(12) {
                    0 => { self.go("GdipSetTextureWrapMode", &[b, mode]); }
                    1 => { self.go("GdipSetLineWrapMode", &[b, mode]); }
                    2 => { self.go("GdipSetPathGradientWrapMode", &[b, mode]); }
                    3 => { self.go("GdipSetTextureTransform", &[b, m]); }
                    4 => { self.go("GdipRotateTextureTransform", &[b, fl, ord]); }
                    5 => { self.go("GdipSetLineGammaCorrection", &[b, bit]); }
                    6 => { self.go("GdipSetPathGradientCenterColor", &[b, c]); }
                    7 => { self.go("GdipSetSolidFillColor", &[b, c]); }
                    8 => { self.go("GdipSetLineTransform", &[b, m]); }
                    9 => { self.go("GdipScaleLineTransform", &[b, s1, s2, ord]); }
                    10 => {
                        let (a, z) = (self.rng.argb(), self.rng.argb());
                        self.go("GdipSetLineColors", &[b, a, z]);
                    }
                    _ => { self.go("GdipSetPathGradientGammaCorrection", &[b, bit]); }
                }
            }
            16 | 17 => {
                let b = self.any(&self.brushes.clone());
                let what = self.rng.pick(&["GdipGetBrushType", "GdipGetSolidFillColor", "GdipGetHatchStyle", "GdipGetHatchForegroundColor", "GdipGetHatchBackgroundColor", "GdipGetTextureWrapMode", "GdipGetLineWrapMode", "GdipGetLineGammaCorrection", "GdipGetPathGradientCenterColor", "GdipGetPathGradientWrapMode", "GdipGetPathGradientPointCount", "GdipGetPathGradientSurroundColorCount"]);
                self.get(what, &[b, buf], 4);
                match self.rng.below(4) {
                    0 => self.get("GdipGetLineColors", &[b, buf], 8),
                    1 => self.get("GdipGetLineRect", &[b, buf], 16),
                    2 => self.get("GdipGetPathGradientRect", &[b, buf], 16),
                    _ => self.get("GdipGetPathGradientCenterPoint", &[b, buf], 8),
                }
            }
            18 => {
                let b = self.any(&self.brushes.clone());
                if let Some(m) = self.make("GdipCreateMatrix", &[o], "mat") {
                    let what = self.rng.pick(&["GdipGetTextureTransform", "GdipGetLineTransform", "GdipGetPathGradientTransform"]);
                    self.go(what, &[b, m]);
                    self.get("GdipGetMatrixElements", &[m, buf], 24);
                    self.go("GdipDeleteMatrix", &[m]);
                }
            }
            19 | 20 => {
                let (m, m2) = (self.any(&self.mats.clone()), self.any(&self.mats.clone()));
                let ord = self.rng.range(0, 2);
                let v: Vec<u32> = (0..2).map(|_| f(self.rng.fl(-3., 3.))).collect();
                let ang = f(self.rng.fl(-180., 180.));
                match self.rng.below(7) {
                    0 => { self.go("GdipMultiplyMatrix", &[m, m2, ord]); }
                    1 => { self.go("GdipTranslateMatrix", &[m, v[0], v[1], ord]); }
                    2 => { self.go("GdipScaleMatrix", &[m, v[0], v[1], ord]); }
                    3 => { self.go("GdipRotateMatrix", &[m, ang, ord]); }
                    4 => { self.go("GdipShearMatrix", &[m, v[0], v[1], ord]); }
                    5 => { self.go("GdipInvertMatrix", &[m]); }
                    _ => {
                        for i in 0..6 {
                            w32(buf + 4 * i, f(self.rng.fl(-10., 10.)));
                        }
                        self.go("GdipTransformMatrixPoints", &[m, buf, 3]);
                        let e = slice(buf, 24).to_vec();
                        self.log("points", &[], 0, &e);
                    }
                }
                self.get("GdipGetMatrixElements", &[m, buf], 24);
                self.get("GdipIsMatrixInvertible", &[m, buf], 4);
                self.get("GdipIsMatrixIdentity", &[m, buf], 4);
            }
            21 | 22 => {
                let p = self.any(&self.paths.clone());
                let v: Vec<u32> = (0..4).map(|_| f(self.rng.fl(-4., 14.))).collect();
                match self.rng.below(4) {
                    0 => { self.go("GdipAddPathRectangle", &[p, v[0], v[1], v[2], v[3]]); }
                    1 => { self.go("GdipAddPathEllipse", &[p, v[0], v[1], v[2], v[3]]); }
                    2 => {
                        let n = self.rng.range(0, 5);
                        for i in 0..2 * n {
                            w32(buf + 4 * i, f(self.rng.fl(-4., 14.)));
                        }
                        self.go("GdipAddPathPolygon", &[p, buf, n]);
                    }
                    _ => {
                        let fm = self.rng.range(0, 2);
                        self.go("GdipSetPathFillMode", &[p, fm]);
                    }
                }
                mem::zero(buf, 4);
                call(h("GdipGetPointCount"), &[p, buf]);
                let n = mem::r32(buf).min(256);
                self.get("GdipGetPathPoints", &[p, buf, n], 8 * n);
                self.get("GdipGetPathTypes", &[p, buf, n], n);
                self.get("GdipGetPathFillMode", &[p, buf], 4);
            }
            23 => {
                let p = self.any(&self.paths.clone());
                if self.rng.below(3) == 0 {
                    if let Some(b) = self.make("GdipCreatePathGradientFromPath", &[p, o], "pgrad") {
                        self.brushes.push(b);
                    }
                } else {
                    self.go("GdipResetPath", &[p]);
                }
            }
            24 => {
                let b = self.any(&self.brushes.clone());
                if let Some(x) = self.make("GdipCloneBrush", &[b, o], "brush") {
                    self.brushes.push(x);
                }
            }
            25 => {
                let b = self.any(&self.bmps.clone());
                if let Some(x) = self.make("GdipCloneImage", &[b, o], "bmp") {
                    self.bmps.push(x);
                }
            }
            26 => {
                let b = self.any(&self.brushes.clone());
                let n = self.rng.range(0, 6);
                for i in 0..n {
                    w32(buf + 64 + 4 * i, self.rng.argb());
                }
                w32(buf, n);
                self.get("GdipSetPathGradientSurroundColorsWithCount", &[b, buf + 64, buf], 4);
                w32(buf, 8);
                self.get("GdipGetPathGradientSurroundColorsWithCount", &[b, buf + 64, buf], 4);
                let e = slice(buf + 64, 32).to_vec();
                self.log("colors", &[], 0, &e);
            }
            27 => {
                let b = self.any(&self.brushes.clone());
                let (x, y) = (f(self.rng.fl(0., 1.)), f(self.rng.fl(0., 1.)));
                self.go("GdipSetPathGradientFocusScales", &[b, x, y]);
                self.get("GdipGetPathGradientFocusScales", &[b, buf, buf + 4], 8);
            }
            28 => {
                let b = self.any(&self.brushes.clone());
                w32(buf, f(self.rng.fl(-2., 14.)));
                w32(buf + 4, f(self.rng.fl(-2., 10.)));
                self.go("GdipSetPathGradientCenterPoint", &[b, buf]);
            }
            _ => {
                let w = self.rng.pick(&[0u32, 1, 4, 40]);
                let fmt = self.rng.pick(&FMTS);
                let hh = self.rng.pick(&[0u32, 3]);
                if let Some(b) = self.make("GdipCreateBitmapFromScan0", &[w, hh, 0, fmt, 0, o], "bmp") {
                    self.go("GdipDisposeImage", &[b]);
                }
            }
        }
    }
}

fn program(seed: u64, steps: usize) -> Vec<String> {
    NEXT.with(|c| c.set(0));
    let buf = heap_alloc(8192);
    let o = heap_alloc(8);
    let mut w = World { rng: Rng(seed), out: vec![format!("seed {seed:#x}")], names: Vec::new(), bmps: Vec::new(), gfx: Vec::new(), brushes: Vec::new(), pens: Vec::new(), mats: Vec::new(), paths: Vec::new(), buf, o };
    w.setup();
    for _ in 0..steps {
        w.step();
    }
    let out = std::mem::take(&mut w.out);
    heap_free(buf);
    heap_free(o);
    out
}

#[test]
fn gdiplus_programs_as_before() {
    let _g = enter();
    guest_thread();
    crate::gdiplus::init();
    crate::gdiplus_text::init();
    crate::gdiplus_brush::init();
    crate::gdiplus_font::init();
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let at = mem::reserve(ARENA, ARENA_SIZE, 4, 0x3000, false);
        assert_eq!(at, Some(ARENA), "the test's arena is taken");
        mem::commit_range(ARENA, ARENA_SIZE, 4);
    });
    let mut out = Vec::new();
    for seed in [0x9e37_79b9_7f4a_7c15u64, 0xdead_beef_cafe_f00d, 0x0123_4567_89ab_cdef] {
        out.extend(program(seed, 1000));
    }
    let out = out.join("\n") + "\n";
    let golden = concat!(env!("CARGO_MANIFEST_DIR"), "/src/gdiplus_api_golden.txt");
    kome::golden::check(golden, &out);
}
