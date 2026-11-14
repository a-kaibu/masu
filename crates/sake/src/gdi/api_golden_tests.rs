//! gdi32 as a whole, pinned before its code is rewritten with types: seeded random programs of
//! GDI calls (bitmaps of every depth and both row orders, every raster op, stretches, masks,
//! transparency, alpha, gradients, shapes, pens and brushes, regions and clips, mapping modes,
//! saved states, DIB reads and writes, palettes) on DCs drawing into each other. Every call's
//! answer (handles as the pool's names, so the order other tests ran in does not matter) and,
//! after it, a hash of every bitmap's bytes and every buffer it filled go into
//! gdi/api_golden.txt. MASU_UPDATE_GOLDEN=1 rewrites the file.
use super::*;
use crate::mem::{slice, w16, w32, w8};
use crate::test_guest::{call, enter, guest_thread};
use crate::api::Handler;

thread_local!(static LIVE: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) });

/// guest memory the program passes (shown in the log as "ptr": where the heap puts it depends
/// on the tests that ran before)
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
    fn color(&mut self) -> u32 {
        match self.below(4) {
            0 => self.pick(&[0, 0xffffff, 0xff, 0xff00, 0xff0000, 0x808080, 0xc0c0c0]),
            1 => self.next() & 0xffffff,
            2 => 0x0100_0000 | self.below(16),
            _ => self.next(),
        }
    }
}

fn fnv(h: &mut u64, b: &[u8]) {
    for &x in b {
        *h = (*h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3);
    }
}

const W: i32 = 23;
const H: i32 = 13;

/// a BITMAPINFO of `bpp` (16: 5-6-5 through BI_BITFIELDS when `r565`), `h` < 0 top-down,
/// `ncol` color table entries from `rng`
fn bmi(rng: &mut Rng, w: i32, h: i32, bpp: u16, r565: bool, ncol: u32) -> u32 {
    let p = heap_alloc(40 + 4 * 256 + 12, true);
    w32(p, 40);
    w32(p + 4, w as u32);
    w32(p + 8, h as u32);
    w16(p + 12, 1);
    w16(p + 14, bpp);
    w32(p + 16, if bpp == 16 && r565 { 3 } else { 0 });
    w32(p + 32, ncol);
    if bpp == 16 && r565 {
        w32(p + 40, 0xf800);
        w32(p + 44, 0x07e0);
        w32(p + 48, 0x001f);
    } else {
        let n = if bpp <= 8 { if ncol != 0 { ncol } else { 1 << bpp } } else { 0 };
        for i in 0..n {
            w32(p + 40 + 4 * i, if bpp == 1 && i < 2 && rng.below(2) == 0 { [0, 0xffffff][i as usize] } else { rng.next() & 0xffffff });
        }
    }
    p
}

struct Target {
    dc: u32,
    bmp: u32,
}

struct World {
    rng: Rng,
    out: Vec<String>,
    /// every handle made, by name in the log
    names: Vec<(u32, String)>,
    t: Vec<Target>,
    brushes: Vec<u32>,
    pens: Vec<u32>,
    rgns: Vec<u32>,
    pals: Vec<u32>,
    mask: u32,
    buf: u32,
}

const BUF: u32 = 64 * 1024;

impl World {
    fn name(&mut self, h: u32, n: &str) -> u32 {
        if h != 0 {
            let k = self.names.len();
            self.names.push((h, format!("{n}{k}")));
        }
        h
    }
    fn show(&self, v: u32) -> String {
        match self.names.iter().find(|(h, _)| *h == v) {
            Some((_, n)) => n.clone(),
            None if LIVE.with(|l| l.borrow().contains(&v)) => "ptr".into(),
            None if (STOCK..=DEFAULT_BITMAP).contains(&v) => format!("stock{}", (v - STOCK) / 4),
            None => format!("{v:#x}"),
        }
    }
    fn bitmap_bytes(&self, h: &mut u64) {
        for t in &self.t {
            let b = with(|g| match g.m.get(&t.bmp) {
                Some(G::Bitmap(b)) => Some((b.bits, b.stride * b.h as u32, b.pal.clone())),
                _ => None,
            });
            if let Some((bits, n, pal)) = b {
                fnv(h, slice(bits, n));
                for p in pal {
                    fnv(h, &p.to_le_bytes());
                }
            }
        }
    }
    fn log(&mut self, what: &str, args: &[u32], r: u32, extra: &[u8]) {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        self.bitmap_bytes(&mut h);
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
    fn dc(&mut self) -> u32 {
        let i = self.rng.below(self.t.len() as u32) as usize;
        self.t[i].dc
    }
    fn xy(&mut self) -> (u32, u32) {
        (self.rng.range(-8, W + 8), self.rng.range(-6, H + 6))
    }
    fn wh(&mut self) -> (u32, u32) {
        (self.rng.range(-12, W + 4), self.rng.range(-8, H + 4))
    }
    fn rop3(&mut self) -> u32 {
        let op = if self.rng.below(3) == 0 { self.rng.pick(&[0xcc, 0x66, 0x88, 0xee, 0x33, 0x44, 0x99, 0xbb, 0xf0, 0x5a, 0xa5, 0x00, 0xff, 0xe2, 0xb8, 0x55]) } else { self.rng.below(256) };
        op << 16 | if self.rng.below(2) == 0 { 0 } else { self.rng.below(0x10000) }
    }
    fn points(&mut self, n: u32) -> u32 {
        let p = heap_alloc(8 * n.max(1), true);
        for i in 0..n {
            let (x, y) = self.xy();
            w32(p + 8 * i, x);
            w32(p + 8 * i + 4, y);
        }
        p
    }
    fn rect(&mut self) -> u32 {
        let p = heap_alloc(16, true);
        let (l, t) = self.xy();
        let (r, b) = self.xy();
        for (i, v) in [l, t, r, b].into_iter().enumerate() {
            w32(p + 4 * i as u32, v);
        }
        p
    }

    fn setup(&mut self) {
        let screen = call(CreateCompatibleDC, &[0]);
        // DIB sections of every depth, both row orders, 16 bpp both ways
        for (bpp, r565) in [(1u16, false), (4, false), (8, false), (16, false), (16, true), (24, false), (32, false), (32, false)] {
            let up = self.rng.below(2) == 0;
            let ncol = if bpp <= 8 && self.rng.below(2) == 0 { 1 + self.rng.below(1 << bpp) } else { 0 };
            let info = bmi(&mut self.rng, W, if up { H } else { -H }, bpp, r565, ncol);
            let pbits = heap_alloc(4, true);
            let bmp = call(CreateDIBSection, &[screen, info, 0, pbits, 0, 0]);
            assert_ne!(bmp, 0, "{bpp}");
            let bits = mem::r32(pbits);
            for i in 0..(((W * i32::from(bpp) + 31) / 32 * 4 * H) as u32) {
                w8(bits + i, self.rng.next() as u8);
            }
            heap_free(pbits);
            heap_free(info);
            let dc = self.name(call(CreateCompatibleDC, &[screen]), "dc");
            self.name(bmp, &format!("dib{bpp}{}{}_", if r565 { "r" } else { "" }, if up { "u" } else { "d" }));
            self.log("select", &[dc, bmp], call(SelectObject, &[dc, bmp]), &[]);
            self.t.push(Target { dc, bmp });
        }
        // DDBs: monochrome, compatible and CreateBitmap's color depths
        for (w, h, planes, bpp) in [(W, H, 1u32, 1u32), (W, H, 1, 8), (W, H, 1, 32), (W, H, 1, 16)] {
            let bmp = call(CreateBitmap, &[w as u32, h as u32, planes, bpp, 0]);
            self.name(bmp, &format!("ddb{bpp}_"));
            let dc = self.name(call(CreateCompatibleDC, &[screen]), "dc");
            self.log("select", &[dc, bmp], call(SelectObject, &[dc, bmp]), &[]);
            self.t.push(Target { dc, bmp });
        }
        let cb = call(CreateCompatibleBitmap, &[screen, W as u32, H as u32]);
        self.name(cb, "compat");
        let dc = self.name(call(CreateCompatibleDC, &[screen]), "dc");
        self.log("select", &[dc, cb], call(SelectObject, &[dc, cb]), &[]);
        self.t.push(Target { dc, bmp: cb });
        // a mask for MaskBlt
        let m = call(CreateBitmap, &[W as u32, H as u32, 1, 1, 0]);
        self.mask = self.name(m, "mask");
        let mdc = call(CreateCompatibleDC, &[screen]);
        call(SelectObject, &[mdc, m]);
        for _ in 0..40 {
            let (x, y) = self.xy();
            call(SetPixel, &[mdc, x, y, 0xffffff]);
        }
        call(DeleteDC, &[mdc]);
        // brushes, pens, regions, palettes
        for _ in 0..4 {
            let c = self.rng.color();
            let b = call(CreateSolidBrush, &[c]);
            let h = self.name(b, "brush");
            self.brushes.push(h);
        }
        for s in 0..6 {
            let c = self.rng.color();
            let b = call(CreateHatchBrush, &[s, c]);
            let h = self.name(b, "hatch");
            self.brushes.push(h);
        }
        for k in [9usize, 11] {
            let b = call(CreatePatternBrush, &[self.t[k].bmp]);
            let h = self.name(b, "pattern");
            self.brushes.push(h);
        }
        for s in 0..7 {
            let (w, c) = (self.rng.below(4), self.rng.color());
            let p = call(CreatePen, &[s, w, c]);
            let h = self.name(p, "pen");
            self.pens.push(h);
        }
        let r = call(CreateRectRgn, &[2, 1, 15, 9]);
        let h = self.name(r, "rgn");
            self.rgns.push(h);
        let r = call(CreateEllipticRgn, &[0, 0, 19, 12]);
        let h = self.name(r, "rgn");
            self.rgns.push(h);
        let r = call(CreateRoundRectRgn, &[3, 2, 22, 13, 6, 5]);
        let h = self.name(r, "rgn");
            self.rgns.push(h);
        let pts = self.points(5);
        let r = call(CreatePolygonRgn, &[pts, 5, 1 + self.rng.below(2)]);
        let h = self.name(r, "rgn");
            self.rgns.push(h);
        heap_free(pts);
        let lp = heap_alloc(4 + 4 * 256, true);
        w16(lp, 0x300);
        w16(lp + 2, 256);
        for i in 0..256 {
            w32(lp + 4 + 4 * i, self.rng.next() & 0xffffff);
        }
        let p = call(CreatePalette, &[lp]);
        let h = self.name(p, "pal");
            self.pals.push(h);
        heap_free(lp);
        let p = call(CreateHalftonePalette, &[screen]);
        let h = self.name(p, "pal");
            self.pals.push(h);
        self.buf = heap_alloc(BUF, true);
    }

    fn step(&mut self) {
        let dc = self.dc();
        let k = self.rng.below(48);
        match k {
            0 => {
                let (x, y) = self.xy();
                let c = self.rng.color();
                self.go("SetPixel", SetPixel, &[dc, x, y, c]);
            }
            1 => {
                let (x, y) = self.xy();
                self.go("GetPixel", GetPixel, &[dc, x, y]);
            }
            2 => {
                let ((x, y), (w, h), op) = (self.xy(), self.wh(), self.rop3());
                self.go("PatBlt", PatBlt, &[dc, x, y, w, h, op]);
            }
            3 | 4 | 5 => {
                let ((x, y), (w, h), s, (sx, sy), op) = (self.xy(), self.wh(), self.dc(), self.xy(), self.rop3());
                self.go("BitBlt", BitBlt, &[dc, x, y, w, h, s, sx, sy, op]);
            }
            6 | 7 => {
                let ((x, y), (w, h), s, (sx, sy), (sw, sh), op) = (self.xy(), self.wh(), self.dc(), self.xy(), self.wh(), self.rop3());
                self.go("StretchBlt", StretchBlt, &[dc, x, y, w, h, s, sx, sy, sw, sh, op]);
            }
            8 => {
                let ((x, y), (w, h), s, (sx, sy), (mx, my)) = (self.xy(), self.wh(), self.dc(), self.xy(), self.xy());
                let rop = (self.rop3() & 0xff_0000) << 8 | self.rop3();
                let m = if self.rng.below(4) == 0 { 0 } else { self.mask };
                self.go("MaskBlt", MaskBlt, &[dc, x, y, w, h, s, sx, sy, m, mx, my, rop]);
            }
            9 => {
                let ((x, y), (w, h), s, (sx, sy), (sw, sh), c) = (self.xy(), self.wh(), self.dc(), self.xy(), self.wh(), self.rng.color());
                self.go("TransparentBlt", TransparentBlt, &[dc, x, y, w, h, s, sx, sy, sw, sh, c]);
            }
            10 | 11 => {
                let ((x, y), (w, h), s, (sx, sy)) = (self.xy(), self.wh(), self.dc(), self.xy());
                let (sw, sh) = if self.rng.below(2) == 0 { (w, h) } else { self.wh() };
                let bf = self.rng.below(2) | self.rng.below(2) << 8 | { let any = self.rng.below(256); self.rng.pick(&[0, 1, 0x7f, 0x80, 0xfe, 0xff, any]) } << 16 | self.rng.pick(&[0, 1, 1, 2]) << 24;
                self.go("AlphaBlend", AlphaBlend, &[dc, x, y, w, h, s, sx, sy, sw, sh, bf]);
            }
            12 => {
                let n = 1 + self.rng.below(4);
                let v = heap_alloc(16 * n, true);
                for i in 0..n {
                    let (x, y) = self.xy();
                    w32(v + 16 * i, x);
                    w32(v + 16 * i + 4, y);
                    for c in 0..4 {
                        w16(v + 16 * i + 8 + 2 * c, self.rng.next() as u16);
                    }
                }
                let mode = self.rng.pick(&[0, 1, 2, 3]);
                let nm = 1 + self.rng.below(2);
                let m = heap_alloc(12 * nm, true);
                for i in 0..3 * nm {
                    w32(m + 4 * i, self.rng.below(n + 1));
                }
                self.go("GradientFill", GradientFill, &[dc, v, n, m, nm, mode]);
                heap_free(v);
                heap_free(m);
            }
            13 | 14 => {
                let f: [(&str, Handler); 3] = [("Rectangle", Rectangle), ("Ellipse", Ellipse), ("RoundRect", RoundRect)];
                let (n, f) = self.rng.pick(&f);
                let ((l, t), (r, b), (cw, ch)) = (self.xy(), self.xy(), (self.rng.range(-2, 12), self.rng.range(-2, 12)));
                self.go(n, f, &[dc, l, t, r, b, cw, ch]);
            }
            15 => {
                let f: [(&str, Handler); 3] = [("Pie", Pie), ("Chord", Chord), ("Arc", Arc)];
                let (n, f) = self.rng.pick(&f);
                let ((l, t), (r, b), (x1, y1), (x2, y2)) = (self.xy(), self.xy(), self.xy(), self.xy());
                self.go(n, f, &[dc, l, t, r, b, x1, y1, x2, y2]);
            }
            16 | 17 => {
                let f: [(&str, Handler); 3] = [("Polygon", Polygon), ("Polyline", Polyline), ("PolyBezier", PolyBezier)];
                let (name, f) = self.rng.pick(&f);
                let n = if name == "PolyBezier" { 1 + 3 * self.rng.below(3) } else { self.rng.below(7) };
                let p = self.points(n);
                self.go(name, f, &[dc, p, n]);
                heap_free(p);
            }
            18 => {
                let (x, y) = self.xy();
                call(MoveToEx, &[dc, x, y, 0]);
                let (x, y) = self.xy();
                self.go("LineTo", LineTo, &[dc, x, y]);
            }
            19 | 20 => {
                let o = match self.rng.below(3) {
                    0 => self.rng.pick(&self.brushes.clone()),
                    1 => self.rng.pick(&self.pens.clone()),
                    _ => call(GetStockObject, &[self.rng.below(20)]),
                };
                self.go("SelectObject", SelectObject, &[dc, o]);
            }
            21 => {
                let (r, b) = (self.rng.pick(&self.rgns.clone()), self.rng.pick(&self.brushes.clone()));
                match self.rng.below(4) {
                    0 => self.go("FillRgn", FillRgn, &[dc, r, b]),
                    1 => {
                        let (w, h) = (self.rng.range(-1, 4), self.rng.range(-1, 4));
                        self.go("FrameRgn", FrameRgn, &[dc, r, b, w, h])
                    }
                    2 => self.go("InvertRgn", InvertRgn, &[dc, r]),
                    _ => self.go("PaintRgn", PaintRgn, &[dc, r]),
                };
            }
            22 => {
                let rg = self.rgns.clone();
                let (d, a, b) = (self.rng.pick(&rg), self.rng.pick(&rg), self.rng.pick(&rg));
                let mode = self.rng.range(0, 6);
                self.go("CombineRgn", CombineRgn, &[d, a, b, mode]);
            }
            23 => {
                let r = self.rng.pick(&self.rgns.clone());
                match self.rng.below(5) {
                    0 => {
                        let (x, y) = (self.rng.range(-3, 3), self.rng.range(-3, 3));
                        self.go("OffsetRgn", OffsetRgn, &[r, x, y]);
                    }
                    1 => {
                        let p = heap_alloc(16, true);
                        let v = call(GetRgnBox, &[r, p]);
                        self.log("GetRgnBox", &[r], v, slice(p, 16));
                        heap_free(p);
                    }
                    2 => {
                        let (x, y) = self.xy();
                        self.go("PtInRegion", PtInRegion, &[r, x, y]);
                    }
                    3 => {
                        let p = self.rect();
                        self.go("RectInRegion", RectInRegion, &[r, p]);
                        heap_free(p);
                    }
                    _ => {
                        let n = call(GetRegionData, &[r, 0, 0]);
                        let v = call(GetRegionData, &[r, BUF, self.buf]);
                        self.log("GetRegionData", &[r, n], v, slice(self.buf, n.min(BUF)));
                    }
                }
            }
            24 => match self.rng.below(6) {
                0 => {
                    let r = if self.rng.below(3) == 0 { 0 } else { self.rng.pick(&self.rgns.clone()) };
                    self.go("SelectClipRgn", SelectClipRgn, &[dc, r]);
                }
                1 => {
                    let (r, m) = (self.rng.pick(&self.rgns.clone()), self.rng.range(0, 6));
                    self.go("ExtSelectClipRgn", ExtSelectClipRgn, &[dc, r, m]);
                }
                2 => {
                    let ((l, t), (r, b)) = (self.xy(), self.xy());
                    self.go("IntersectClipRect", IntersectClipRect, &[dc, l, t, r, b]);
                }
                3 => {
                    let ((l, t), (r, b)) = (self.xy(), self.xy());
                    self.go("ExcludeClipRect", ExcludeClipRect, &[dc, l, t, r, b]);
                }
                4 => {
                    let (x, y) = (self.rng.range(-4, 4), self.rng.range(-4, 4));
                    self.go("OffsetClipRgn", OffsetClipRgn, &[dc, x, y]);
                }
                _ => {
                    let p = heap_alloc(16, true);
                    let v = call(GetClipBox, &[dc, p]);
                    self.log("GetClipBox", &[dc], v, slice(p, 16));
                    heap_free(p);
                }
            },
            25 => {
                let v = self.rng.range(0, 17);
                self.go("SetROP2", SetROP2, &[dc, v]);
            }
            26 => {
                let v = self.rng.range(0, 3);
                self.go("SetBkMode", SetBkMode, &[dc, v]);
            }
            27 => {
                let c = self.rng.color();
                self.go("SetBkColor", SetBkColor, &[dc, c]);
            }
            28 => {
                let c = self.rng.color();
                self.go("SetTextColor", SetTextColor, &[dc, c]);
            }
            29 => {
                let v = self.rng.range(0, 5);
                self.go("SetStretchBltMode", SetStretchBltMode, &[dc, v]);
            }
            30 => {
                let v = self.rng.range(0, 3);
                self.go("SetPolyFillMode", SetPolyFillMode, &[dc, v]);
            }
            31 => {
                let (x, y) = (self.rng.range(-9, 9), self.rng.range(-9, 9));
                self.go("SetBrushOrgEx", SetBrushOrgEx, &[dc, x, y, 0]);
            }
            32 => {
                let (x, y) = (self.rng.range(-5, 5), self.rng.range(-5, 5));
                if self.rng.below(2) == 0 {
                    self.go("SetViewportOrgEx", SetViewportOrgEx, &[dc, x, y, 0]);
                } else {
                    self.go("SetWindowOrgEx", SetWindowOrgEx, &[dc, x, y, 0]);
                }
            }
            33 => {
                let m = self.rng.range(1, 8);
                self.go("SetMapMode", SetMapMode, &[dc, m]);
                let (x, y) = (self.rng.range(-30, 30), self.rng.range(-30, 30));
                if self.rng.below(2) == 0 {
                    self.go("SetViewportExtEx", SetViewportExtEx, &[dc, x, y, 0]);
                } else {
                    self.go("SetWindowExtEx", SetWindowExtEx, &[dc, x, y, 0]);
                }
            }
            34 => {
                if self.rng.below(2) == 0 {
                    self.go("SaveDC", SaveDC, &[dc]);
                } else {
                    let n = self.rng.pick(&[u32::MAX, 1, 2, (-2i32) as u32]);
                    self.go("RestoreDC", RestoreDC, &[dc, n]);
                }
            }
            35 | 36 => {
                // GetDIBits of any target into any format
                let t = self.rng.below(self.t.len() as u32) as usize;
                let bmp = self.t[t].bmp;
                let (bpp, r565) = self.rng.pick(&[(1u16, false), (4, false), (8, false), (16, false), (16, true), (24, false), (32, false)]);
                let h = if self.rng.below(2) == 0 { H } else { -H };
                let info = bmi(&mut self.rng, W, h, bpp, r565, 0);
                let (start, lines) = (self.rng.range(-1, H), self.rng.range(0, H + 2));
                let usage = self.rng.below(2);
                mem::zero(self.buf, BUF);
                let v = call(GetDIBits, &[dc, bmp, start, lines, self.buf, info, usage]);
                let mut e = slice(self.buf, (W as u32 * 4) * H as u32).to_vec();
                e.extend_from_slice(slice(info, 40 + 4 * 256));
                self.log("GetDIBits", &[dc, bmp, start, lines, u32::from(bpp), usage], v, &e);
                heap_free(info);
            }
            37 | 38 => {
                // SetDIBits / StretchDIBits / SetDIBitsToDevice from random bits of any format
                let (bpp, r565) = self.rng.pick(&[(1u16, false), (4, false), (8, false), (16, false), (16, true), (24, false), (32, false)]);
                let h = if self.rng.below(2) == 0 { H } else { -H };
                let ncol = if bpp <= 8 && self.rng.below(2) == 0 { 1 + self.rng.below(1 << bpp) } else { 0 };
                let info = bmi(&mut self.rng, W, h, bpp, r565, ncol);
                for i in 0..(W as u32 * 4) * H as u32 {
                    w8(self.buf + i, self.rng.next() as u8);
                }
                let usage = if self.rng.below(5) == 0 { 1 } else { 0 };
                match self.rng.below(3) {
                    0 => {
                        let t = self.rng.below(self.t.len() as u32) as usize;
                        let bmp = self.t[t].bmp;
                        let (start, lines) = (self.rng.range(-1, H), self.rng.range(0, H + 2));
                        self.go("SetDIBits", SetDIBits, &[dc, bmp, start, lines, self.buf, info, usage]);
                    }
                    1 => {
                        let ((x, y), (w, hh), (sx, sy), (sw, sh), op) = (self.xy(), self.wh(), self.xy(), self.wh(), self.rop3());
                        self.go("StretchDIBits", StretchDIBits, &[dc, x, y, w, hh, sx, sy, sw, sh, self.buf, info, usage, op]);
                    }
                    _ => {
                        let ((x, y), (w, hh), (sx, sy), (start, lines)) = (self.xy(), self.wh(), self.xy(), (self.rng.range(-1, H), self.rng.range(0, H + 2)));
                        self.go("SetDIBitsToDevice", SetDIBitsToDevice, &[dc, x, y, w, hh, sx, sy, start, lines, self.buf, info, usage]);
                    }
                }
                heap_free(info);
            }
            39 => {
                let c = self.rng.color();
                self.go("GetNearestColor", GetNearestColor, &[dc, c]);
            }
            40 => {
                let (s, n) = (self.rng.range(0, 20), self.rng.range(0, 260));
                if self.rng.below(2) == 0 {
                    for i in 0..256 {
                        w32(self.buf + 4 * i, self.rng.next());
                    }
                    self.go("SetDIBColorTable", SetDIBColorTable, &[dc, s, n, self.buf]);
                } else {
                    mem::zero(self.buf, 1024);
                    let v = call(GetDIBColorTable, &[dc, s, n, self.buf]);
                    self.log("GetDIBColorTable", &[dc, s, n], v, slice(self.buf, 1024));
                }
            }
            41 => {
                let o = if self.rng.below(2) == 0 { self.rng.pick(&self.brushes.clone()) } else { self.rng.pick(&self.pens.clone()) };
                mem::zero(self.buf, 64);
                let v = call(GetObjectW, &[o, 64, self.buf]);
                // (a pattern brush's LOGBRUSH names its bitmap: a handle, not pinned)
                let mut e = slice(self.buf, 64).to_vec();
                e[8..12].fill(0);
                self.log("GetObjectW", &[o], v, &e);
            }
            42 => {
                let b = self.rng.pick(&self.t.iter().map(|t| t.bmp).collect::<Vec<_>>());
                mem::zero(self.buf, 128);
                let v = call(GetObjectW, &[b, self.rng.pick(&[24, 84, 104, 128]), self.buf]);
                let mut e = slice(self.buf, 128).to_vec();
                e[20..24].fill(0);
                self.log("GetObjectW", &[b], v, &e);
            }
            43 => {
                let p = self.rng.pick(&self.pals.clone());
                self.go("SelectPalette", SelectPalette, &[dc, p, 0]);
                self.go("RealizePalette", RealizePalette, &[dc]);
            }
            44 => {
                let p = self.rng.pick(&self.pals.clone());
                let c = self.rng.color();
                self.go("GetNearestPaletteIndex", GetNearestPaletteIndex, &[p, c]);
            }
            45 => {
                let n = 1 + self.rng.below(3);
                let c = heap_alloc(4 * n, true);
                let mut all = 0;
                for i in 0..n {
                    let k = self.rng.range(1, 5);
                    w32(c + 4 * i, k);
                    all += k;
                }
                let p = self.points(all);
                self.go("PolyPolygon", PolyPolygon, &[dc, p, c, n]);
                heap_free(p);
                heap_free(c);
            }
            46 => {
                let f: [(&str, Handler); 4] = [("GetROP2", GetROP2), ("GetBkMode", GetBkMode), ("GetStretchBltMode", GetStretchBltMode), ("GetPolyFillMode", GetPolyFillMode)];
                let (n, f) = self.rng.pick(&f);
                self.go(n, f, &[dc]);
                let p = heap_alloc(8, true);
                let v = call(GetViewportOrgEx, &[dc, p]);
                self.log("GetViewportOrgEx", &[dc], v, slice(p, 8));
                let v = call(GetWindowExtEx, &[dc, p]);
                self.log("GetWindowExtEx", &[dc], v, slice(p, 8));
                heap_free(p);
            }
            _ => {
                let p = self.points(3);
                let v = call(LPtoDP, &[dc, p, 3]);
                self.log("LPtoDP", &[dc], v, slice(p, 24));
                heap_free(p);
            }
        }
    }
}

fn program(seed: u64, steps: usize) -> Vec<String> {
    let mut w = World { rng: Rng(seed), out: vec![format!("seed {seed:#x}")], names: Vec::new(), t: Vec::new(), brushes: Vec::new(), pens: Vec::new(), rgns: Vec::new(), pals: Vec::new(), mask: 0, buf: 0 };
    w.setup();
    for _ in 0..steps {
        w.step();
    }
    w.out
}

#[test]
fn gdi_programs_as_before() {
    let _g = enter();
    guest_thread();
    init();
    let mut out = Vec::new();
    for seed in [0x9e37_79b9_7f4a_7c15u64, 0xdead_beef_cafe_f00d, 0x0123_4567_89ab_cdef, 0x5555_aaaa_3333_cccc] {
        out.extend(program(seed, 1200));
    }
    let out = out.join("\n") + "\n";
    let golden = concat!(env!("CARGO_MANIFEST_DIR"), "/src/gdi/api_golden.txt");
    if std::env::var_os("MASU_UPDATE_GOLDEN").is_some() {
        std::fs::write(golden, &out).unwrap();
        return;
    }
    let want = std::fs::read_to_string(golden).unwrap_or_default();
    if want != out {
        let (w, o): (Vec<&str>, Vec<&str>) = (want.lines().collect(), out.lines().collect());
        let i = w.iter().zip(&o).position(|(a, b)| a != b).unwrap_or(w.len().min(o.len()));
        panic!("gdi: api_golden.txt differs at line {} (MASU_UPDATE_GOLDEN=1 rewrites it):\n  want {:?}\n  got  {:?}", i + 1, w.get(i), o.get(i));
    }
}
