//! MPEG-1 video (ISO/IEC 11172-2): pictures from an elementary stream, in display order, as
//! YCbCr 4:2:0 planes. A port of pl_mpeg's plm_video (MIT, Dominic Szablewski), with its
//! integer IDCT, so its output is pl_mpeg's to the bit; fed bytes as they come (`push`), a
//! picture decoded once the start code after it is in (or the stream ended).

use crate::bits::Bits;
use crate::tables::*;

const PICTURE: u8 = 0x00;
const SEQUENCE: u8 = 0xb3;
const EXTENSION: u8 = 0xb5;
const USER_DATA: u8 = 0xb2;
const GOP: u8 = 0xb8;
const SEQUENCE_END: u8 = 0xb7;

/// A picture's coding type (D pictures, type 4, are not decoded).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PictureType {
    I,
    P,
    B,
}

impl PictureType {
    /// The type of a picture header's three bits (None: forbidden, D, or reserved).
    fn of(three_bits: u32) -> Option<PictureType> {
        match three_bits {
            1 => Some(PictureType::I),
            2 => Some(PictureType::P),
            3 => Some(PictureType::B),
            _ => None,
        }
    }
}

/// A decoded picture: planes with whole macroblocks (`y_stride` x 16-aligned height; chroma
/// half of each), of which `width` x `height` is the picture.
#[derive(Clone, Default)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub y_stride: usize,
    pub c_stride: usize,
    pub y: Vec<u8>,
    pub cb: Vec<u8>,
    pub cr: Vec<u8>,
}

/// The sequence header's facts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Header {
    pub width: usize,
    pub height: usize,
    /// pictures a second (0: unknown)
    pub rate: f64,
}

#[derive(Default, Clone, Copy)]
struct Motion {
    full_px: bool,
    is_set: bool,
    r_size: u32,
    h: i32,
    v: i32,
}

pub struct Video {
    es: Vec<u8>,
    /// where the next start code is looked for in `es`
    at: usize,
    /// an incomplete picture's search for its end: (the picture's start code, where to look on)
    scan: (usize, usize),
    /// the picture being decoded, with a start code after it (kept: one allocation)
    pic: Vec<u8>,
    ended: bool,
    header: Option<Header>,
    mb_width: i32,
    mb_height: i32,
    mb_size: i32,
    luma_width: usize,
    chroma_width: usize,
    intra_quant: [u8; 64],
    non_intra_quant: [u8; 64],
    /// the three picture buffers; current / forward / backward are indices into it
    frames: Vec<Frame>,
    cur: usize,
    fwd: usize,
    bwd: usize,
    /// the picture's type (None: one not decoded)
    picture_type: Option<PictureType>,
    motion_fwd: Motion,
    motion_bwd: Motion,
    quantizer_scale: i32,
    slice_begin: bool,
    mb_address: i32,
    mb_row: i32,
    mb_col: i32,
    mb_type: u32,
    mb_intra: bool,
    dc_predictor: [i32; 3],
    block: [i32; 64],
    has_reference: bool,
    /// pictures handed out
    pub pictures: u64,
}

/// The largest picture a sequence header may give (4096x2304; 12-bit sizes reach 4095x4095).
pub const MAX_PIXELS: usize = 4096 * 2304;

/// The most bytes a picture may take (an intra 4096x2304 picture at a high rate is a few MB).
pub const MAX_PICTURE: usize = 16 << 20;

/// `n` zero bytes, or None when there is no memory for them.
fn zeroed(n: usize) -> Option<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).ok()?;
    v.resize(n, 0);
    Some(v)
}

// (tests: steps `find` took)
#[cfg(test)]
thread_local!(static STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) });

impl Default for Video {
    fn default() -> Self {
        Self::new()
    }
}

impl Video {
    pub fn new() -> Self {
        Video {
            es: Vec::new(),
            at: 0,
            scan: (usize::MAX, 0),
            pic: Vec::new(),
            ended: false,
            header: None,
            mb_width: 0,
            mb_height: 0,
            mb_size: 0,
            luma_width: 0,
            chroma_width: 0,
            intra_quant: INTRA_QUANT,
            non_intra_quant: [16; 64],
            frames: Vec::new(),
            cur: 0,
            fwd: 1,
            bwd: 2,
            picture_type: None,
            motion_fwd: Motion::default(),
            motion_bwd: Motion::default(),
            quantizer_scale: 0,
            slice_begin: false,
            mb_address: 0,
            mb_row: 0,
            mb_col: 0,
            mb_type: 0,
            mb_intra: false,
            dc_predictor: [128; 3],
            block: [0; 64],
            has_reference: false,
            pictures: 0,
        }
    }

    pub fn header(&self) -> Option<Header> {
        self.header
    }

    /// More of the stream.
    pub fn push(&mut self, d: &[u8]) {
        // (what was decoded already goes once it is a good part of the buffer)
        if self.at > 1 << 20 && self.at * 2 > self.es.len() {
            self.es.drain(..self.at);
            if self.scan.0 != usize::MAX {
                self.scan = (self.scan.0 - self.at, self.scan.1 - self.at);
            }
            self.at = 0;
        }
        self.es.extend_from_slice(d);
    }

    /// No more of the stream: the last picture decodes without a start code after it.
    pub fn end(&mut self) {
        self.ended = true;
    }

    /// Back to the start of a new stream (a seek): the next picture must be an intra one.
    pub fn reset(&mut self) {
        self.es.clear();
        self.at = 0;
        self.scan = (usize::MAX, 0);
        self.ended = false;
        self.has_reference = false;
    }

    /// The start code at or after `from` in the buffer: (its position, its value).
    fn find(&self, from: usize) -> Option<(usize, u8)> {
        let d = &self.es;
        let mut i = from;
        while i + 3 < d.len() {
            #[cfg(test)]
            STEPS.with(|c| c.set(c.get() + 1));
            if d[i + 2] > 1 {
                i += 3;
            } else if d[i] == 0 && d[i + 1] == 0 && d[i + 2] == 1 {
                return Some((i, d[i + 3]));
            } else {
                i += 1;
            }
        }
        None
    }

    /// The next picture in display order, if one is complete.
    pub fn decode(&mut self) -> Option<&Frame> {
        loop {
            let Some((p, code)) = self.find(self.at) else {
                // the last reference picture is still to show at the end
                if self.ended && self.has_reference && self.header.is_some() {
                    self.has_reference = false;
                    self.at = self.es.len();
                    self.pictures += 1;
                    return Some(&self.frames[self.bwd]);
                }
                // (no start code in what is here: looked at again only from its last 3 bytes, and
                // `push` drains the rest; a stream with no start codes kept all of it, rescanned)
                self.at = self.at.max(self.es.len().saturating_sub(3));
                return None;
            };
            match code {
                SEQUENCE => {
                    // (64 bits + two matrices at most)
                    if self.es.len() < p + 4 + 140 && !self.ended {
                        return None;
                    }
                    let end = (p + 4 + 140).min(self.es.len());
                    let d = self.es[p + 4..end].to_vec();
                    self.sequence_header(&d);
                    self.at = p + 4;
                }
                PICTURE if self.header.is_some() => {
                    // the whole picture: up to the next picture / sequence / GOP / end code
                    // (on from where the last call stopped: a picture comes in many packets)
                    let mut q = if self.scan.0 == p { self.scan.1 } else { p + 4 };
                    let next = loop {
                        match self.find(q) {
                            Some((n, c)) if matches!(c, PICTURE | SEQUENCE | GOP | SEQUENCE_END) => break Some(n),
                            Some((n, _)) => q = n + 3,
                            None => break None,
                        }
                    };
                    let end = match next {
                        Some(n) => n,
                        None if self.ended => self.es.len(),
                        // (a picture that never ends, the stream after it no MPEG-1 video: it is
                        // dropped, as the bytes after a start code no picture holds)
                        None if self.es.len() - p > MAX_PICTURE => {
                            self.scan = (usize::MAX, 0);
                            self.at = self.es.len().saturating_sub(3);
                            continue;
                        }
                        None => {
                            self.scan = (p, q.max(self.es.len().saturating_sub(3)));
                            return None;
                        }
                    };
                    self.scan = (usize::MAX, 0);
                    // (with a start code and room after it, as pl_mpeg reads the stream on)
                    let mut pic = std::mem::take(&mut self.pic);
                    pic.clear();
                    pic.extend_from_slice(&self.es[p + 4..end]);
                    pic.extend_from_slice(&[0, 0, 1, SEQUENCE_END, 0, 0, 0, 0]);
                    self.at = end;
                    self.picture(&pic);
                    self.pic = pic;
                    let shown = match self.picture_type {
                        Some(PictureType::B) => Some(self.cur),
                        Some(PictureType::I | PictureType::P) => {
                            if self.has_reference {
                                Some(self.fwd)
                            } else {
                                self.has_reference = true;
                                None
                            }
                        }
                        None => None,
                    };
                    if let Some(f) = shown {
                        self.pictures += 1;
                        return Some(&self.frames[f]);
                    }
                }
                _ => self.at = p + 4,
            }
        }
    }

    fn sequence_header(&mut self, d: &[u8]) {
        let mut b = Bits::new(d);
        let width = b.uz::<12>();
        let height = b.uz::<12>();
        // (a corrupt header's size: three frames of it are made at once)
        if width == 0 || height == 0 || width * height > MAX_PIXELS {
            return;
        }
        b.skip(4); // pixel aspect ratio
        let rate = PICTURE_RATE[b.uz::<4>()];
        b.skip(18 + 1 + 10 + 1);
        let mut intra = INTRA_QUANT;
        if b.read(1) != 0 {
            for &z in ZIG_ZAG.iter() {
                intra[usize::from(z)] = b.u8::<8>();
            }
        }
        let mut non_intra = [16u8; 64];
        if b.read(1) != 0 {
            for &z in ZIG_ZAG.iter() {
                non_intra[usize::from(z)] = b.u8::<8>();
            }
        }
        self.intra_quant = intra;
        self.non_intra_quant = non_intra;
        // (a repeated header of the same size keeps the pictures)
        if self.header.is_some_and(|h| h.width == width && h.height == height) {
            self.header = Some(Header { width, height, rate });
            return;
        }
        self.header = Some(Header { width, height, rate });
        // (12-bit sizes: 256 macroblocks a side at most)
        let (mbw, mbh) = (width.div_ceil(16), height.div_ceil(16));
        self.mb_width = i32::try_from(mbw).unwrap_or(i32::MAX);
        self.mb_height = i32::try_from(mbh).unwrap_or(i32::MAX);
        self.mb_size = self.mb_width * self.mb_height;
        self.luma_width = mbw * 16;
        self.chroma_width = mbw * 8;
        let luma = self.luma_width * mbh * 16;
        let chroma = self.chroma_width * mbh * 8;
        // (a row and a pixel more, as pl_mpeg's frames sit one after another in memory: a
        // half-pixel motion vector at the bottom edge reads past its plane)
        // (memory there is not for them: no pictures, as for no header, rather than an abort)
        self.frames = Vec::new();
        let made = (0..3).try_for_each(|_| {
            let f = Frame {
                width,
                height,
                y_stride: self.luma_width,
                c_stride: self.chroma_width,
                y: zeroed(luma + self.luma_width + 1)?,
                cb: zeroed(chroma + self.chroma_width + 1)?,
                cr: zeroed(chroma + self.chroma_width + 1)?,
            };
            self.frames.push(f);
            Some(())
        });
        if made.is_none() {
            self.frames = Vec::new();
            self.header = None;
            return;
        }
        self.cur = 0;
        self.fwd = 1;
        self.bwd = 2;
        self.has_reference = false;
    }

    fn picture(&mut self, d: &[u8]) {
        let mut b = Bits::new(d);
        b.skip(10); // temporal reference
        self.picture_type = PictureType::of(b.read(3));
        b.skip(16); // vbv delay
        let Some(ty) = self.picture_type else { return };
        if matches!(ty, PictureType::P | PictureType::B) {
            self.motion_fwd.full_px = b.read(1) != 0;
            let f_code = b.read(3);
            if f_code == 0 {
                self.picture_type = None;
                return;
            }
            self.motion_fwd.r_size = f_code - 1;
        }
        if ty == PictureType::B {
            self.motion_bwd.full_px = b.read(1) != 0;
            let f_code = b.read(3);
            if f_code == 0 {
                self.picture_type = None;
                return;
            }
            self.motion_bwd.r_size = f_code - 1;
        }
        let reference = matches!(ty, PictureType::I | PictureType::P);
        let temp = self.fwd;
        if reference {
            self.fwd = self.bwd;
        }
        let mut code = loop {
            match b.next_start_code() {
                Some(EXTENSION) | Some(USER_DATA) => continue,
                c => break c,
            }
        };
        while let Some(c @ 0x01..=0xaf) = code {
            self.slice(&mut b, i32::from(c));
            if self.mb_address >= self.mb_size - 1 {
                break;
            }
            code = b.next_start_code();
        }
        if reference {
            self.bwd = self.cur;
            self.cur = temp;
        }
    }

    fn slice(&mut self, b: &mut Bits, slice: i32) {
        self.slice_begin = true;
        self.mb_address = (slice - 1) * self.mb_width - 1;
        self.motion_bwd.h = 0;
        self.motion_fwd.h = 0;
        self.motion_bwd.v = 0;
        self.motion_fwd.v = 0;
        self.dc_predictor = [128; 3];
        self.quantizer_scale = b.i32::<5>();
        while b.read(1) != 0 {
            b.skip(8);
        }
        loop {
            self.macroblock(b);
            if !(self.mb_address < self.mb_size - 1 && b.peek_non_zero(23)) {
                break;
            }
        }
    }

    fn macroblock(&mut self, b: &mut Bits) {
        let mut increment = 0i32;
        let mut t = i32::from(b.vlc(MB_ADDRESS_INCREMENT));
        while t == 34 {
            t = i32::from(b.vlc(MB_ADDRESS_INCREMENT));
        }
        while t == 35 {
            increment += 33;
            t = i32::from(b.vlc(MB_ADDRESS_INCREMENT));
        }
        increment += t;
        if self.slice_begin {
            self.slice_begin = false;
            self.mb_address += increment;
        } else {
            if self.mb_address + increment >= self.mb_size {
                return;
            }
            if increment > 1 {
                self.dc_predictor = [128; 3];
                if self.picture_type == Some(PictureType::P) {
                    self.motion_fwd.h = 0;
                    self.motion_fwd.v = 0;
                }
            }
            while increment > 1 {
                self.mb_address += 1;
                self.mb_row = self.mb_address / self.mb_width;
                self.mb_col = self.mb_address % self.mb_width;
                self.predict_macroblock();
                increment -= 1;
            }
            self.mb_address += 1;
        }
        if self.mb_address < 0 {
            return;
        }
        self.mb_row = self.mb_address / self.mb_width;
        self.mb_col = self.mb_address % self.mb_width;
        if self.mb_col >= self.mb_width || self.mb_row >= self.mb_height {
            return;
        }
        let table = match self.picture_type {
            Some(PictureType::I) => MB_TYPE_I,
            Some(PictureType::P) => MB_TYPE_P,
            // (no picture of another type gets here: its slices are not read)
            Some(PictureType::B) | None => MB_TYPE_B,
        };
        self.mb_type = u32::from(b.vlc(table));
        self.mb_intra = self.mb_type & 1 != 0;
        self.motion_fwd.is_set = self.mb_type & 8 != 0;
        self.motion_bwd.is_set = self.mb_type & 4 != 0;
        if self.mb_type & 0x10 != 0 {
            self.quantizer_scale = b.i32::<5>();
        }
        if self.mb_intra {
            self.motion_bwd.h = 0;
            self.motion_fwd.h = 0;
            self.motion_bwd.v = 0;
            self.motion_fwd.v = 0;
        } else {
            self.dc_predictor = [128; 3];
            self.motion_vectors(b);
            self.predict_macroblock();
        }
        let cbp = if self.mb_type & 2 != 0 {
            u32::from(b.vlc(CODED_BLOCK_PATTERN))
        } else if self.mb_intra {
            0x3f
        } else {
            0
        };
        for block in 0..6 {
            if cbp & (0x20 >> block) != 0 {
                self.decode_block(b, block);
            }
        }
    }

    fn motion_vectors(&mut self, b: &mut Bits) {
        if self.motion_fwd.is_set {
            let r = self.motion_fwd.r_size;
            self.motion_fwd.h = motion_vector(b, r, self.motion_fwd.h);
            self.motion_fwd.v = motion_vector(b, r, self.motion_fwd.v);
        } else if self.picture_type == Some(PictureType::P) {
            self.motion_fwd.h = 0;
            self.motion_fwd.v = 0;
        }
        if self.motion_bwd.is_set {
            let r = self.motion_bwd.r_size;
            self.motion_bwd.h = motion_vector(b, r, self.motion_bwd.h);
            self.motion_bwd.v = motion_vector(b, r, self.motion_bwd.v);
        }
    }

    fn predict_macroblock(&mut self) {
        let (mut fh, mut fv) = (self.motion_fwd.h, self.motion_fwd.v);
        if self.motion_fwd.full_px {
            fh <<= 1;
            fv <<= 1;
        }
        if self.picture_type == Some(PictureType::B) {
            let (mut bh, mut bv) = (self.motion_bwd.h, self.motion_bwd.v);
            if self.motion_bwd.full_px {
                bh <<= 1;
                bv <<= 1;
            }
            if self.motion_fwd.is_set {
                self.copy_macroblock(self.fwd, fh, fv, false);
                if self.motion_bwd.is_set {
                    self.copy_macroblock(self.bwd, bh, bv, true);
                }
            } else {
                self.copy_macroblock(self.bwd, bh, bv, false);
            }
        } else {
            self.copy_macroblock(self.fwd, fh, fv, false);
        }
    }

    fn copy_macroblock(&mut self, s: usize, mh: i32, mv: i32, interp: bool) {
        if s == self.cur || self.frames.len() < 3 {
            return;
        }
        let (src, dst) = two(&mut self.frames, s, self.cur);
        let (row, col, mbw, mbh) = (self.mb_row, self.mb_col, self.mb_width, self.mb_height);
        process(&src.y, &mut dst.y, row, col, mbw, mbh, mh, mv, 16, interp);
        process(&src.cr, &mut dst.cr, row, col, mbw, mbh, mh / 2, mv / 2, 8, interp);
        process(&src.cb, &mut dst.cb, row, col, mbw, mbh, mh / 2, mv / 2, 8, interp);
    }

    fn decode_block(&mut self, b: &mut Bits, block: usize) {
        let mut n = 0usize;
        let quant;
        if self.mb_intra {
            let plane = if block > 3 { block - 3 } else { 0 };
            let predictor = self.dc_predictor[plane];
            let size = u32::from(b.vlc(if plane == 0 { DCT_SIZE_Y } else { DCT_SIZE_C }));
            self.block[0] = if size > 0 {
                let diff = b.read(size).cast_signed();
                if diff & (1 << (size - 1)) != 0 {
                    predictor.wrapping_add(diff)
                } else {
                    predictor.wrapping_add((-(1i32 << size)) | (diff + 1))
                }
            } else {
                predictor
            };
            self.dc_predictor[plane] = self.block[0];
            self.block[0] = self.block[0].wrapping_shl(3 + 5);
            quant = self.intra_quant;
            n = 1;
        } else {
            quant = self.non_intra_quant;
        }
        loop {
            let coeff = b.vlc(DCT_COEFF);
            if coeff == 0x0001 && n > 0 && b.read(1) == 0 {
                break; // end of block
            }
            let (run, mut level);
            if coeff == 0xffff {
                run = b.uz::<6>();
                level = b.i32::<8>();
                if level == 0 {
                    level = b.i32::<8>();
                } else if level == 128 {
                    level = b.i32::<8>() - 256;
                } else if level > 128 {
                    level -= 256;
                }
            } else {
                run = usize::from(coeff >> 8);
                level = i32::from(coeff & 0xff);
                if b.read(1) != 0 {
                    level = -level;
                }
            }
            n += run;
            if n >= 64 {
                return;
            }
            let z = usize::from(ZIG_ZAG[n]);
            n += 1;
            level <<= 1;
            if !self.mb_intra {
                level += if level < 0 { -1 } else { 1 };
            }
            level = (level * self.quantizer_scale * i32::from(quant[z])) >> 4;
            if level & 1 == 0 {
                level -= if level > 0 { 1 } else { -1 };
            }
            level = level.clamp(-2048, 2047);
            self.block[z] = level * i32::from(PREMULTIPLIER[z]);
        }
        // (the macroblock is in the picture: 0 <= row < mb_height, 0 <= col < mb_width)
        let (row, col) = (crate::bits::us(self.mb_row.cast_unsigned()), crate::bits::us(self.mb_col.cast_unsigned()));
        let lw = self.luma_width;
        let cw = self.chroma_width;
        let cur = self.cur;
        let f = &mut self.frames[cur];
        let (d, dw, mut di) = if block < 4 {
            let mut di = (row * lw + col) << 4;
            if block & 1 != 0 {
                di += 8;
            }
            if block & 2 != 0 {
                di += lw << 3;
            }
            (&mut f.y, lw, di)
        } else {
            (if block == 4 { &mut f.cb } else { &mut f.cr }, cw, ((row * lw) << 2) + (col << 3))
        };
        if di + 7 * dw + 8 > d.len() {
            self.block = [0; 64];
            return;
        }
        let s = &mut self.block;
        if n == 1 {
            let v = (s[0].wrapping_add(128)) >> 8;
            for _ in 0..8 {
                for x in 0..8 {
                    d[di + x] = if self.mb_intra { clamp(v) } else { clamp(i32::from(d[di + x]) + v) };
                }
                di += dw;
            }
            s[0] = 0;
        } else {
            idct_put(s, d, di, dw, self.mb_intra);
        }
    }
}

fn two(f: &mut [Frame], s: usize, d: usize) -> (&Frame, &mut Frame) {
    if s < d {
        let (a, b) = f.split_at_mut(d);
        (&a[s], &mut b[0])
    } else {
        let (a, b) = f.split_at_mut(s);
        (&b[0], &mut a[d])
    }
}

fn clamp(n: i32) -> u8 {
    // (0..=255: the low byte)
    n.clamp(0, 255).to_le_bytes()[0]
}

fn motion_vector(b: &mut Bits, r_size: u32, motion: i32) -> i32 {
    let fscale = 1i32 << r_size;
    // (the table's values are i16s)
    let m_code = i32::from(b.vlc(MOTION).cast_signed());
    let d = if m_code != 0 && fscale != 1 {
        let r = b.read(r_size).cast_signed();
        let d = ((m_code.abs() - 1) << r_size) + r + 1;
        if m_code < 0 {
            -d
        } else {
            d
        }
    } else {
        m_code
    };
    let mut m = motion + d;
    if m > (fscale << 4) - 1 {
        m -= fscale << 5;
    } else if m < -(fscale << 4) {
        m += fscale << 5;
    }
    m
}

#[allow(clippy::too_many_arguments)]
fn process(s: &[u8], d: &mut [u8], row: i32, col: i32, mb_width: i32, mb_height: i32, mh: i32, mv: i32, bs: i32, interp: bool) {
    let dw = mb_width * bs;
    let hp = mh >> 1;
    let vp = mv >> 1;
    let odd_h = mh & 1 == 1;
    let odd_v = mv & 1 == 1;
    let w = i64::from;
    let si = w((row * bs) + vp) * w(dw) + w(col * bs) + w(hp);
    let di = w((row * dw + col) * bs);
    let max = w(dw) * w(mb_height * bs - bs + 1) - w(bs);
    // (pl_mpeg compares unsigned: a negative address is past the maximum)
    if si > max || di > max {
        return;
    }
    let (Ok(si), Ok(di), Ok(dw), Ok(bs)) = (usize::try_from(si), usize::try_from(di), usize::try_from(dw), usize::try_from(bs)) else { return };
    if si + (bs - 1 + usize::from(odd_v)) * dw + bs + usize::from(odd_h) > s.len() || di + (bs - 1) * dw + bs > d.len() {
        return;
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    return unsafe { crate::simd::process_rows(s, d, si, di, dw, bs, odd_h, odd_v, interp) };
    #[allow(unreachable_code)]
    process_rows(s, d, si, di, dw, bs, odd_h, odd_v, interp)
}

/// process's loop (the scalar code crate::simd's is held to).
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn process_rows(s: &[u8], d: &mut [u8], mut si: usize, mut di: usize, dw: usize, bs: usize, odd_h: bool, odd_v: bool, interp: bool) {
    for _ in 0..bs {
        for x in 0..bs {
            let px = |i: usize| u32::from(s[i]);
            let p = match (odd_h, odd_v) {
                (false, false) => px(si + x),
                (false, true) => (px(si + x) + px(si + x + dw) + 1) >> 1,
                (true, false) => (px(si + x) + px(si + x + 1) + 1) >> 1,
                (true, true) => (px(si + x) + px(si + x + 1) + px(si + x + dw) + px(si + x + dw + 1) + 2) >> 2,
            };
            // (averages of bytes: 0..=255, the low byte)
            d[di + x] = if interp { ((u32::from(d[di + x]) + p + 1) >> 1).to_le_bytes()[0] } else { p.to_le_bytes()[0] };
        }
        si += dw;
        di += dw;
    }
}

/// pl_mpeg's integer IDCT (in place; rows of 8), with wrapping arithmetic so corrupt data
/// cannot overflow.
pub fn idct(b: &mut [i32; 64]) {
    #[inline]
    fn m(a: i32, k: i32) -> i32 {
        a.wrapping_mul(k).wrapping_add(128) >> 8
    }
    for i in 0..8 {
        let b1 = b[4 * 8 + i];
        let b3 = b[2 * 8 + i].wrapping_add(b[6 * 8 + i]);
        let b4 = b[5 * 8 + i].wrapping_sub(b[3 * 8 + i]);
        let tmp1 = b[8 + i].wrapping_add(b[7 * 8 + i]);
        let tmp2 = b[3 * 8 + i].wrapping_add(b[5 * 8 + i]);
        let b6 = b[8 + i].wrapping_sub(b[7 * 8 + i]);
        let b7 = tmp1.wrapping_add(tmp2);
        let m0 = b[i];
        let x4 = (b6.wrapping_mul(473).wrapping_sub(b4.wrapping_mul(196)).wrapping_add(128) >> 8).wrapping_sub(b7);
        let x0 = x4.wrapping_sub(m(tmp1.wrapping_sub(tmp2), 362));
        let x1 = m0.wrapping_sub(b1);
        let x2 = m(b[2 * 8 + i].wrapping_sub(b[6 * 8 + i]), 362).wrapping_sub(b3);
        let x3 = m0.wrapping_add(b1);
        let y3 = x1.wrapping_add(x2);
        let y4 = x3.wrapping_add(b3);
        let y5 = x1.wrapping_sub(x2);
        let y6 = x3.wrapping_sub(b3);
        let y7 = x0.wrapping_neg().wrapping_sub(b4.wrapping_mul(473).wrapping_add(b6.wrapping_mul(196)).wrapping_add(128) >> 8);
        b[i] = b7.wrapping_add(y4);
        b[8 + i] = x4.wrapping_add(y3);
        b[2 * 8 + i] = y5.wrapping_sub(x0);
        b[3 * 8 + i] = y6.wrapping_sub(y7);
        b[4 * 8 + i] = y6.wrapping_add(y7);
        b[5 * 8 + i] = x0.wrapping_add(y5);
        b[6 * 8 + i] = y3.wrapping_sub(x4);
        b[7 * 8 + i] = y4.wrapping_sub(b7);
    }
    for i in (0..64).step_by(8) {
        let b1 = b[4 + i];
        let b3 = b[2 + i].wrapping_add(b[6 + i]);
        let b4 = b[5 + i].wrapping_sub(b[3 + i]);
        let tmp1 = b[1 + i].wrapping_add(b[7 + i]);
        let tmp2 = b[3 + i].wrapping_add(b[5 + i]);
        let b6 = b[1 + i].wrapping_sub(b[7 + i]);
        let b7 = tmp1.wrapping_add(tmp2);
        let m0 = b[i];
        let x4 = (b6.wrapping_mul(473).wrapping_sub(b4.wrapping_mul(196)).wrapping_add(128) >> 8).wrapping_sub(b7);
        let x0 = x4.wrapping_sub(m(tmp1.wrapping_sub(tmp2), 362));
        let x1 = m0.wrapping_sub(b1);
        let x2 = m(b[2 + i].wrapping_sub(b[6 + i]), 362).wrapping_sub(b3);
        let x3 = m0.wrapping_add(b1);
        let y3 = x1.wrapping_add(x2);
        let y4 = x3.wrapping_add(b3);
        let y5 = x1.wrapping_sub(x2);
        let y6 = x3.wrapping_sub(b3);
        let y7 = x0.wrapping_neg().wrapping_sub(b4.wrapping_mul(473).wrapping_add(b6.wrapping_mul(196)).wrapping_add(128) >> 8);
        let r = |v: i32| v.wrapping_add(128) >> 8;
        b[i] = r(b7.wrapping_add(y4));
        b[1 + i] = r(x4.wrapping_add(y3));
        b[2 + i] = r(y5.wrapping_sub(x0));
        b[3 + i] = r(y6.wrapping_sub(y7));
        b[4 + i] = r(y6.wrapping_add(y7));
        b[5 + i] = r(x0.wrapping_add(y5));
        b[6 + i] = r(y3.wrapping_sub(x4));
        b[7 + i] = r(y4.wrapping_sub(b7));
    }
}

/// [`idct`] of `s`, then added to (intra: written to) `d` at `di`, rows `dw` apart, clamped
/// to bytes; `s` is left zero. `d` holds the 8 rows.
#[doc(hidden)]
pub fn idct_put(s: &mut [i32; 64], d: &mut [u8], di: usize, dw: usize, intra: bool) {
    assert!(di + 7 * dw + 8 <= d.len());
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    return unsafe { crate::simd::idct_put(s, d, di, dw, intra) };
    #[allow(unreachable_code)]
    idct_put_scalar(s, d, di, dw, intra)
}

/// idct_put's scalar code (crate::simd's is held to it).
#[doc(hidden)]
pub fn idct_put_scalar(s: &mut [i32; 64], d: &mut [u8], mut di: usize, dw: usize, intra: bool) {
    idct(s);
    for y in 0..8 {
        for x in 0..8 {
            let v = s[y * 8 + x];
            d[di + x] = if intra { clamp(v) } else { clamp(i32::from(d[di + x]) + v) };
        }
        di += dw;
    }
    *s = [0; 64];
}

/// A picture as 32-bit BGRX pixels (DirectShow's RGB32), BT.601 as pl_mpeg converts:
/// `stride` bytes a row, rows from the bottom when `bottom_up` (a DIB with a positive height).
pub fn to_bgrx(f: &Frame, out: &mut [u8], stride: usize, bottom_up: bool) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        let (w, h) = (f.width & !1, f.height & !1);
        assert!(f.y.len() >= h * f.y_stride && f.cb.len() >= h / 2 * f.c_stride && f.cr.len() >= h / 2 * f.c_stride && f.c_stride >= w / 2 && f.y_stride >= w);
        for row in 0..h / 2 {
            let mut col = 0;
            while col + 8 <= w / 2 && unsafe { crate::simd::bgrx16(f, out, stride, bottom_up, h, row, col) } {
                col += 8;
            }
            bgrx_pairs(f, out, stride, bottom_up, row, col);
        }
        return;
    }
    #[allow(unreachable_code)]
    to_bgrx_scalar(f, out, stride, bottom_up)
}

/// to_bgrx's scalar code (crate::simd's is held to it).
#[doc(hidden)]
pub fn to_bgrx_scalar(f: &Frame, out: &mut [u8], stride: usize, bottom_up: bool) {
    for row in 0..(f.height & !1) / 2 {
        bgrx_pairs(f, out, stride, bottom_up, row, 0);
    }
}

/// to_bgrx of rows 2 row, 2 row + 1 from chroma column `from` on.
fn bgrx_pairs(f: &Frame, out: &mut [u8], stride: usize, bottom_up: bool, row: usize, from: usize) {
    let (w, h) = (f.width & !1, f.height & !1);
    for col in from..w / 2 {
        let ci = row * f.c_stride + col;
        let cr = i32::from(f.cr[ci]) - 128;
        let cb = i32::from(f.cb[ci]) - 128;
        let r = (cr * 104597) >> 16;
        let g = (cb * 25674 + cr * 53278) >> 16;
        let b = (cb * 132201) >> 16;
        for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let (x, y) = (col * 2 + dx, row * 2 + dy);
            let yv = ((i32::from(f.y[y * f.y_stride + x]) - 16) * 76309) >> 16;
            let oy = if bottom_up { h - 1 - y } else { y };
            let o = oy * stride + x * 4;
            if o + 4 <= out.len() {
                out[o] = clamp(yv + b);
                out[o + 1] = clamp(yv - g);
                out[o + 2] = clamp(yv + r);
                out[o + 3] = 0xff;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// testsrc.mpg's video elementary stream.
    fn es() -> Vec<u8> {
        let d = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/testsrc.mpg")).unwrap();
        let mut m = crate::Demux::new();
        m.push(&d);
        m.end();
        let mut es = Vec::new();
        while let Some(p) = m.next() {
            if crate::ps::is_video(p.id) {
                es.extend(p.data);
            }
        }
        es
    }

    /// A picture coming a byte at a time is searched on from where the last search stopped:
    /// the bytes looked at grow with the stream, not with its square (each push rescanned the
    /// picture from its start).
    #[test]
    fn picture_searches_go_on_where_they_stopped() {
        let es = es();
        let mut v = Video::new();
        STEPS.with(|c| c.set(0));
        let mut n = 0;
        for b in &es {
            v.push(std::slice::from_ref(b));
            while v.decode().is_some() {
                n += 1;
            }
        }
        v.end();
        while v.decode().is_some() {
            n += 1;
        }
        assert_eq!(n, 30);
        let steps = STEPS.with(|c| c.get());
        assert!(steps < 8 * es.len() as u64, "{steps} steps for {} bytes", es.len());
    }

    /// A header for 4095x4095 pictures (three frames of 25 MB) is not taken.
    #[test]
    fn huge_sequence_headers_are_refused() {
        let mut v = Video::new();
        // 0xfff wide, 0xfff high, aspect 1, rate 3, then the rest of the header
        v.push(&[0, 0, 1, SEQUENCE, 0xff, 0xff, 0xff, 0x13, 0xff, 0xff, 0xe0, 0x18]);
        v.push(&[0; 140]);
        v.end();
        assert!(v.decode().is_none());
        assert!(v.header().is_none());
        assert!(v.frames.is_empty());
        // (one in bounds is)
        let mut v = Video::new();
        v.push(&[0, 0, 1, SEQUENCE, 0x14, 0x00, 0xf0, 0x13, 0xff, 0xff, 0xe0, 0x18]);
        v.push(&[0; 140]);
        v.end();
        let _ = v.decode();
        assert_eq!(v.header().map(|h| (h.width, h.height)), Some((320, 240)));
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

        /// Bytes with no start code in them, pushed and decoded for as long as they come, are
        /// kept only as far as their last 3 bytes plus what `push` has not drained yet: they
        /// grew the buffer without bound, each decode rescanning all of it.
        #[test]
        fn no_start_codes_keep_little(chunk in 64usize..200_000, fill in 1u8..=255, start in proptest::bool::ANY) {
            let mut v = Video::new();
            let es = es();
            if start {
                // (pictures first, then a stream fed as video that is not)
                v.push(&es);
                while v.decode().is_some() {}
            }
            let junk = vec![fill; chunk];
            STEPS.with(|c| c.set(0));
            let mut pushed = 0u64;
            while pushed < 40 << 20 {
                v.push(&junk);
                pushed += chunk as u64;
                while v.decode().is_some() {}
                prop_assert!(v.es.len() <= MAX_PICTURE + (2 << 20) + 2 * chunk + es.len(), "{} bytes kept", v.es.len());
                // (each byte looked at about once: no rescans of what is kept)
                let steps = STEPS.with(|c| c.get());
                prop_assert!(steps <= 2 * pushed + 4 * es.len() as u64, "{steps} steps for {pushed} bytes");
            }
        }

        /// The stream in any slices decodes to the pictures of the whole (the search left where
        /// it stopped and the drained buffer move together).
        #[test]
        fn any_slices_decode_the_same(cuts in prop::collection::vec(1usize..6000, 1..40)) {
            let es = es();
            let whole = {
                let mut v = Video::new();
                v.push(&es);
                v.end();
                let mut h = vec![];
                while let Some(f) = v.decode() {
                    h.push(crate::check::fnv(0xcbf29ce484222325, &f.y));
                }
                h
            };
            let mut v = Video::new();
            let mut got = vec![];
            let (mut at, mut i) = (0, 0);
            while at < es.len() {
                let n = cuts[i % cuts.len()].min(es.len() - at);
                v.push(&es[at..at + n]);
                at += n;
                i += 1;
                while let Some(f) = v.decode() {
                    got.push(crate::check::fnv(0xcbf29ce484222325, &f.y));
                }
            }
            v.end();
            while let Some(f) = v.decode() {
                got.push(crate::check::fnv(0xcbf29ce484222325, &f.y));
            }
            prop_assert_eq!(got, whole);
        }
    }
}
