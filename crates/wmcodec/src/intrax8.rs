//! IntraX8 ("J-frame") pictures of WMV 2 and WMV 3: FFmpeg's intrax8.c and intrax8dsp.c, ported
//! bit for bit. Each 8x8 block is predicted from its decoded neighbours in one of 12
//! directions and gets a WMV 2 IDCT residual; chroma blocks go with every second block of every
//! second row.

use crate::bits::Bits;
use crate::idct;
use crate::intrax8_tables as t;
use crate::msmpeg4_tables as m4;
use crate::num::{ix, lo16, lo8, us};
use crate::vlc::Vlc;
use std::sync::OnceLock;

struct Tables {
    /// [quant < 13][intra / inter][select]
    ac: Vec<Vlc>,
    /// [quant < 13][select]
    dc: Vec<Vlc>,
    /// [0][0..2]: quant >= 13, [1][0..4]: quant < 13
    orient_high: [Vlc; 2],
    orient_low: [Vlc; 4],
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        // (symbol, length) pairs, codes assigned in order (ff_vlc_init_tables_from_lengths)
        let mk = |bits: u32, pairs: &[u8]| {
            let syms: Vec<i32> = pairs.chunks(2).map(|p| i32::from(p[0])).collect();
            let lens: Vec<u8> = pairs.chunks(2).map(|p| p[1]).collect();
            Vlc::from_lengths(bits, &lens, Some(&syms), 0)
        };
        Tables {
            ac: (0..32).map(|k| mk(9, &t::X8_AC_QUANT_TABLE[k * 154..k * 154 + 154])).collect(),
            dc: (0..16).map(|k| mk(9, &t::X8_DC_QUANT_TABLE[k * 68..k * 68 + 68])).collect(),
            orient_high: std::array::from_fn(|k| mk(7, &t::X8_ORIENT_HIGHQUANT_TABLE[k * 24..k * 24 + 24])),
            orient_low: std::array::from_fn(|k| mk(7, &t::X8_ORIENT_LOWQUANT_TABLE[k * 24..k * 24 + 24])),
        }
    })
}

/// (extra bits, run mask, run offset, level offset) of AC codes 46..73 (ac_decode_table)
const AC_DECODE: [(u32, u32, i32, i32); 27] = [
    (3, 0xff, 16, 0), (3, 0xff, 24, 0), (2, 0xff, 4, 1), (3, 0xff, 8, 1),
    (5, 0xff, 32, 0), (4, 0xff, 16, 1),
    (2, 0, 0, 4), (2, 0, 0, 8), (2, 0, 0, 12), (3, 0, 0, 16), (3, 0, 0, 24),
    (2, 0, 1, 3), (3, 0, 1, 7),
    (2, 0xff, 16, 0), (2, 0xff, 20, 0), (2, 0xff, 24, 0), (2, 0xff, 28, 0), (4, 0xff, 32, 0), (4, 0xff, 48, 0),
    (2, 0xff, 4, 1), (3, 0xff, 8, 1), (4, 0xff, 16, 1),
    (2, 0, 0, 4), (3, 0, 0, 8), (4, 0, 0, 16),
    (2, 0, 1, 3), (3, 0, 1, 7),
];

const CRAZY_MIX_RUNLEVEL: [u8; 32] = [
    0x22, 0x32, 0x33, 0x53, 0x23, 0x42, 0x43, 0x63, 0x24, 0x52, 0x34, 0x73, 0x25, 0x62, 0x44, 0x83,
    0x26, 0x72, 0x35, 0x54, 0x27, 0x82, 0x45, 0x64, 0x28, 0x92, 0x36, 0x74, 0x29, 0xa2, 0x46, 0x84,
];

const DC_INDEX_OFFSET: [i32; 17] = [0, 1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193];

const QUANT_TABLE: [i32; 64] = [
    256, 256, 256, 256, 256, 256, 259, 262, 265, 269, 272, 275, 278, 282, 285, 288,
    292, 295, 299, 303, 306, 310, 314, 317, 321, 325, 329, 333, 337, 341, 345, 349,
    353, 358, 362, 366, 371, 375, 379, 384, 389, 393, 398, 403, 408, 413, 417, 422,
    428, 433, 438, 443, 448, 454, 459, 465, 470, 476, 482, 488, 493, 499, 505, 511,
];

/// The 12 prediction directions of a block (spatial_compensation_0..11): 0 the smooth blend of
/// both edges, 4 from above, 8 from the left, the rest the diagonals and blends between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Orient {
    O0,
    O1,
    O2,
    O3,
    O4,
    O5,
    O6,
    O7,
    O8,
    O9,
    O10,
    O11,
}

impl Orient {
    const ALL: [Orient; 12] = [O0, O1, O2, O3, O4, O5, O6, O7, O8, O9, O10, O11];

    /// Direction `n` (0..12).
    fn of(n: u32) -> Option<Orient> {
        Orient::ALL.get(us(n)).copied()
    }

    fn n(self) -> u32 {
        match self {
            O0 => 0,
            O1 => 1,
            O2 => 2,
            O3 => 3,
            O4 => 4,
            O5 => 5,
            O6 => 6,
            O7 => 7,
            O8 => 8,
            O9 => 9,
            O10 => 10,
            O11 => 11,
        }
    }
}

use Orient::*;

/// [predicted orientation][coded orientation]
const PREDICTION_TABLE: [[Orient; 12]; 3] = [
    [O0, O8, O4, O10, O11, O2, O6, O9, O1, O3, O5, O7],
    [O4, O0, O8, O11, O10, O3, O5, O2, O6, O9, O1, O7],
    [O8, O0, O4, O10, O11, O1, O7, O2, O6, O9, O3, O5],
];

// the scratchpad's areas: 1 and 2 the two columns left (bottom up), 3 the corner, 4 + 5 the row
// above and the one right of it, 6 the row above that
const AREA1: usize = 0;
const AREA2: usize = 8;
const AREA3: usize = 16;
const AREA4: usize = 17;
const AREA5: usize = 25;
const AREA6: usize = 33;

/// The IntraX8 state of a stream: the run / orientation predictions of the last two block rows.
pub struct IntraX8 {
    mb_w: usize,
    mb_h: usize,
    prediction: Vec<u8>,
    // per picture
    quant: i32,
    dquant: i32,
    qsum: i32,
    use_quant_matrix: bool,
    quant_dc_chroma: i32,
    divide_quant_dc_luma: i32,
    divide_quant_dc_chroma: i32,
    loopfilter: bool,
    ac_sel: [Option<usize>; 4],
    dc_sel: [Option<usize>; 3],
    orient_sel: Option<usize>,
    // per block
    edges: u32,
    flat_dc: bool,
    predicted_dc: i32,
    raw_orient: i32,
    chroma_orient: Orient,
    orient: Orient,
    est_run: i32,
    mb_x: usize,
    mb_y: usize,
    scratch: [u8; 42],
    block: [i16; 64],
}

/// A plane of the picture: pixels, stride.
pub struct Plane<'a> {
    pub px: &'a mut [u8],
    pub stride: usize,
}

impl IntraX8 {
    /// For pictures of `mb_w` x `mb_h` macroblocks.
    pub fn new(mb_w: usize, mb_h: usize) -> IntraX8 {
        tables();
        IntraX8 {
            mb_w, mb_h, prediction: vec![0; mb_w * 2 * 2],
            quant: 1, dquant: 2, qsum: 0, use_quant_matrix: false, quant_dc_chroma: 1, divide_quant_dc_luma: 1,
            divide_quant_dc_chroma: 1, loopfilter: false, ac_sel: [None; 4], dc_sel: [None; 3], orient_sel: None,
            edges: 0, flat_dc: false, predicted_dc: 0, raw_orient: 0, chroma_orient: O0, orient: O0, est_run: 0,
            mb_x: 0, mb_y: 0, scratch: [0; 42], block: [0; 64],
        }
    }

    /// ff_intrax8_decode_picture: `dquant` the doubled quantizer (odd for VC-1's half step),
    /// `quant_offset` added to each level. A damaged picture stops where it went wrong (what is
    /// decoded stays). Returns the 8x8 block row it stopped at (mb_h * 2 when whole).
    pub fn decode(&mut self, r: &mut Bits, planes: [Plane; 3], dquant: i32, quant_offset: i32, loopfilter: bool) -> usize {
        let [mut y, mut u, mut v] = planes;
        self.dquant = dquant;
        self.quant = dquant >> 1;
        self.qsum = quant_offset;
        self.loopfilter = loopfilter;
        self.use_quant_matrix = r.bit();
        let q = self.quant.max(1);
        self.divide_quant_dc_luma = ((1 << 16) + (q >> 1)) / q;
        if self.quant < 5 {
            self.quant_dc_chroma = self.quant;
            self.divide_quant_dc_chroma = self.divide_quant_dc_luma;
        } else {
            self.quant_dc_chroma = self.quant + ((self.quant + 3) >> 3);
            self.divide_quant_dc_chroma = ((1 << 16) + (self.quant_dc_chroma >> 1)) / self.quant_dc_chroma;
        }
        self.ac_sel = [None; 4];
        self.dc_sel = [None; 3];
        self.orient_sel = None;
        for mb_y in 0..self.mb_h * 2 {
            self.mb_y = mb_y;
            let mut d0 = mb_y * 8 * y.stride;
            let mut dc = (mb_y & !1) * 4 * u.stride;
            if r.remaining() == 0 {
                return mb_y;
            }
            for mb_x in 0..self.mb_w * 2 {
                self.mb_x = mb_x;
                self.get_prediction();
                if self.setup_spatial_predictor(r, &y, d0, false).is_none() || self.decode_block(r, &mut y, d0, 0).is_none() {
                    return mb_y;
                }
                if mb_x & mb_y & 1 != 0 {
                    self.get_prediction_chroma();
                    self.setup_spatial_predictor(r, &u, dc, true);
                    if self.decode_block(r, &mut u, dc, 1).is_none() {
                        return mb_y;
                    }
                    self.setup_spatial_predictor(r, &v, dc, true);
                    if self.decode_block(r, &mut v, dc, 2).is_none() {
                        return mb_y;
                    }
                    dc += 8;
                }
                d0 += 8;
            }
        }
        self.mb_h * 2
    }

    fn select_ac(&mut self, r: &mut Bits, mode: usize) {
        if self.ac_sel[mode].is_none() {
            let k = r.uz::<3>();
            self.ac_sel[mode] = Some((usize::from(self.quant < 13) * 2 + (mode >> 1)) * 8 + k);
        }
    }

    /// x8_get_ac_rlf: (run, level, final)
    fn ac_rlf(&mut self, r: &mut Bits, mode: usize) -> (i32, i32, bool) {
        let i = tables().ac[self.ac_sel[mode].unwrap_or(0)].read(r);
        if i < 46 {
            if i < 0 {
                return (64, 64, true);
            }
            let fin = i > 22;
            let i = i - 23 * i32::from(fin);
            let l = (0xE50000 >> (i & 0x1E)) & 3;
            let t = 0x01030F >> (l << 3);
            (i & t, l, fin)
        } else if i < 73 {
            let (eb, mask, ro, lo) = AC_DECODE[ix(i - 46)];
            let e = r.get(eb).cast_signed();
            let mask = mask.cast_signed();
            (ro + (e & mask), lo + (e & !mask), i - 46 > 58 - 46)
        } else if i < 75 {
            let e = r.uz::<5>();
            (i32::from(CRAZY_MIX_RUNLEVEL[e] >> 4), i32::from(CRAZY_MIX_RUNLEVEL[e] & 15), i & 1 == 0)
        } else {
            let level = r.get(7 - 3 * (i.cast_unsigned() & 1)).cast_signed();
            let run = r.i::<6>();
            (run, level, r.bit())
        }
    }

    /// x8_get_dc_rlf: (level, final), None on a bad code
    fn dc_rlf(&mut self, r: &mut Bits, mode: usize) -> Option<(i32, bool)> {
        if self.dc_sel[mode].is_none() {
            let k = r.uz::<3>();
            self.dc_sel[mode] = Some(usize::from(self.quant < 13) * 8 + k);
        }
        let mut i = tables().dc[self.dc_sel[mode].unwrap_or(0)].read(r);
        let fin = i > 16;
        i -= 17 * i32::from(fin);
        if i <= 0 {
            return if i == 0 { Some((0, fin)) } else { None };
        }
        let mut c = (i + 1) >> 1;
        c -= i32::from(c > 1);
        let e = r.get(c.cast_unsigned()).cast_signed();
        let i = DC_INDEX_OFFSET[ix(i)] + (e >> 1);
        let e = -(e & 1);
        Some(((i ^ e) - e, fin))
    }

    fn orient_vlc(&mut self, r: &mut Bits) -> i32 {
        let low = self.quant < 13;
        if self.orient_sel.is_none() {
            self.orient_sel = Some(us(r.get(1 + u32::from(low))));
        }
        let k = self.orient_sel.unwrap_or(0);
        if low { tables().orient_low[k].read(r) } else { tables().orient_high[k].read(r) }
    }

    /// x8_setup_spatial_predictor: None on a bad orientation code
    fn setup_spatial_predictor(&mut self, r: &mut Bits, p: &Plane, at: usize, chroma: bool) -> Option<()> {
        let (range, mut sum) = setup_spatial_compensation(p.px, at, p.stride, &mut self.scratch, self.edges);
        let quant = if chroma {
            self.orient = self.chroma_orient;
            self.quant_dc_chroma
        } else {
            self.quant
        };
        self.flat_dc = false;
        if range < quant || range < 3 {
            self.orient = O0;
            // (a +-1 IDCT difference breaks decoding here)
            if range < 3 {
                self.flat_dc = true;
                sum += 9;
                self.predicted_dc = sum * 6899 >> 17;
            }
        }
        if chroma {
            return Some(());
        }
        if range < 2 * self.quant {
            if self.edges & 3 == 0 {
                if self.orient == O1 {
                    self.orient = O11;
                }
                if self.orient == O2 {
                    self.orient = O10;
                }
            } else {
                self.orient = O0;
            }
            self.raw_orient = 0;
        } else {
            self.raw_orient = self.orient_vlc(r);
            if !(0..12).contains(&self.raw_orient) {
                return None;
            }
            self.orient = PREDICTION_TABLE[us(self.orient.n())][ix(self.raw_orient)];
        }
        Some(())
    }

    fn update_predictions(&mut self, orient: Orient, est_run: i32) {
        self.prediction[self.mb_x * 2 + (self.mb_y & 1)] = lo8(((est_run << 2) + i32::from(orient == O4) + 2 * i32::from(orient == O8)).cast_unsigned());
    }

    fn get_prediction_chroma(&mut self) {
        let (x, y) = (self.mb_x, self.mb_y);
        self.edges = u32::from(x >> 1 == 0) | 2 * u32::from(y >> 1 == 0) | 4 * u32::from(x >= 2 * self.mb_w - 1);
        self.raw_orient = 0;
        if self.edges & 3 != 0 {
            self.chroma_orient = if (0xCC >> self.edges) & 1 != 0 { O8 } else { O4 };
            return;
        }
        // (update_predictions stores 0..2 in the low bits)
        self.chroma_orient = Orient::of(u32::from(self.prediction[2 * x - 2] & 3) << 2).unwrap_or(O0);
    }

    fn get_prediction(&mut self) {
        let (x, y) = (self.mb_x, self.mb_y);
        self.edges = u32::from(x == 0) | 2 * u32::from(y == 0) | 4 * u32::from(x >= 2 * self.mb_w - 1);
        let p = &self.prediction;
        match self.edges & 3 {
            1 => {
                self.est_run = i32::from(p[usize::from(y & 1 == 0)] >> 2);
                self.orient = O1;
                return;
            }
            2 => {
                self.est_run = i32::from(p[2 * x - 2] >> 2);
                self.orient = O2;
                return;
            }
            3 => {
                self.est_run = 16;
                self.orient = O0;
                return;
            }
            _ => {}
        }
        let b = i32::from(p[2 * x + usize::from(y & 1 == 0)]);
        let a = i32::from(p[2 * x - 2 + (y & 1)]);
        let c = i32::from(p[2 * x - 2 + usize::from(y & 1 == 0)]);
        self.est_run = b.min(a);
        // (not the edges: x & y, as FFmpeg has it)
        if x & y != 0 {
            self.est_run = c.min(self.est_run);
        }
        self.est_run >>= 2;
        let (a, b, c) = (a & 3, b & 3, c & 3);
        let i = (0xFFEAF4C4u32 >> (2 * b + 8 * a)) & 3;
        let i = if i != 3 { i } else { (0xFFEAD8u32 >> (2 * c + 8 * i32::from(self.quant > 12))) & 3 };
        // (0..3: two bits)
        self.orient = Orient::of(i).unwrap_or(O0);
    }

    /// x8_ac_compensation
    fn ac_compensation(&mut self, direction: i32, dc_level: i32) {
        let b = &mut self.block;
        let tt = |x: i32| (x * dc_level + 0x8000) >> 16;
        let mut sub = |x: usize, y: usize, t: i32| b[x + y * 8] = lo16(i32::from(b[x + y * 8]) - t);
        match direction {
            0 => {
                let t = tt(3811);
                sub(1, 0, t);
                sub(0, 1, t);
                let t = tt(487);
                sub(2, 0, t);
                sub(0, 2, t);
                let t = tt(506);
                sub(3, 0, t);
                sub(0, 3, t);
                let t = tt(135);
                sub(4, 0, t);
                sub(0, 4, t);
                sub(2, 1, -t);
                sub(1, 2, -t);
                sub(3, 1, -t);
                sub(1, 3, -t);
                let t = tt(173);
                sub(5, 0, t);
                sub(0, 5, t);
                let t = tt(61);
                sub(6, 0, t);
                sub(0, 6, t);
                sub(5, 1, -t);
                sub(1, 5, -t);
                let t = tt(42);
                sub(7, 0, t);
                sub(0, 7, t);
                sub(4, 1, -t);
                sub(1, 4, -t);
                sub(4, 4, -t);
                let t = tt(1084);
                sub(1, 1, -t);
            }
            1 => {
                sub(0, 1, tt(6269));
                sub(0, 3, tt(708));
                sub(0, 5, tt(172));
                sub(0, 7, tt(73));
            }
            2 => {
                sub(1, 0, tt(6269));
                sub(3, 0, tt(708));
                sub(5, 0, tt(172));
                sub(7, 0, tt(73));
            }
            _ => {}
        }
    }

    /// x8_decode_intra_mb: one 8x8 block at `at` of plane `p` (`chroma` 0 luma, 1 / 2)
    fn decode_block(&mut self, r: &mut Bits, p: &mut Plane, at: usize, chroma: usize) -> Option<()> {
        self.block = [0; 64];
        let dc_mode = if chroma != 0 { 2 } else { usize::from(self.est_run != 0) };
        let (mut dc_level, fin) = self.dc_rlf(r, dc_mode)?;
        let mut n = 0;
        let mut zeros_only = false;
        let stride = p.stride;
        'placed: {
            if !fin {
                let mut use_quant_matrix = self.use_quant_matrix;
                let (mut ac_mode, est_run) = if chroma != 0 {
                    (1, 64)
                } else {
                    if self.raw_orient < 3 {
                        use_quant_matrix = false;
                    }
                    if self.raw_orient > 4 {
                        (0, 64)
                    } else if self.est_run > 1 {
                        (2, self.est_run)
                    } else {
                        (3, 64)
                    }
                };
                self.select_ac(r, ac_mode);
                let scan = [0, 2, 3][us((0x928548u32 >> (2 * self.orient.n())) & 3)];
                let mut pos = 0;
                loop {
                    n += 1;
                    if n >= est_run {
                        ac_mode = 3;
                        self.select_ac(r, 3);
                    }
                    let (run, level, f) = self.ac_rlf(r, ac_mode);
                    pos += run + 1;
                    if pos > 63 {
                        return None;
                    }
                    let mut level = (level + 1) * self.dquant + self.qsum;
                    let sign = -r.i::<1>();
                    level = (level ^ sign) - sign;
                    if use_quant_matrix {
                        level = (level * QUANT_TABLE[ix(pos)]) >> 8;
                    }
                    self.block[usize::from(m4::WMV1_SCANTABLE[scan * 64 + ix(pos)])] = lo16(level);
                    if f {
                        break;
                    }
                }
            } else {
                if self.flat_dc && (dc_level + 1).cast_unsigned() < 3 {
                    let (divide, dc_quant) = if chroma == 0 { (self.divide_quant_dc_luma, self.quant) } else { (self.divide_quant_dc_chroma, self.quant_dc_chroma) };
                    // (meant as dc_level += predicted_dc / quant)
                    dc_level += (self.predicted_dc * divide + (1 << 12)) >> 13;
                    solid(p.px, at, stride, lo8(((dc_level * dc_quant + 4) >> 3).clamp(0, 255).cast_unsigned()));
                    break 'placed;
                }
                zeros_only = dc_level == 0;
            }
            self.block[0] = lo16(dc_level * if chroma == 0 { self.quant } else { self.quant_dc_chroma });
            if (dc_level + 1).cast_unsigned() >= 3 && self.edges & 3 != 3 {
                let direction = (0x6A017Ci32 >> (self.orient.n() * 2)) & 3;
                if direction != 3 {
                    let dc = i32::from(self.block[0]);
                    self.ac_compensation(direction, dc);
                }
            }
            if self.flat_dc {
                solid(p.px, at, stride, lo8(self.predicted_dc.cast_unsigned()));
            } else {
                spatial_compensation(self.orient, &self.scratch, p.px, at, stride);
            }
            if !zeros_only {
                idct::wmv2_add(p.px, at, stride, &mut self.block);
            }
        }
        if chroma == 0 {
            self.update_predictions(self.orient, n);
        }
        if self.loopfilter {
            if !(self.edges & 2 != 0 || (zeros_only && matches!(self.orient, O0 | O4))) {
                loop_filter(p.px, at, stride.cast_signed(), 1, self.quant);
            }
            if !(self.edges & 1 != 0 || (zeros_only && matches!(self.orient, O0 | O8))) {
                loop_filter(p.px, at, 1, stride.cast_signed(), self.quant);
            }
        }
        Some(())
    }
}

fn solid(px: &mut [u8], at: usize, stride: usize, v: u8) {
    for y in 0..8 {
        px[at + y * stride..at + y * stride + 8].fill(v);
    }
}

/// x8_setup_spatial_compensation: the edge pixels of the block at `at` into `dst`; (range, sum)
fn setup_spatial_compensation(px: &[u8], at: usize, stride: usize, dst: &mut [u8; 42], edges: u32) -> (i32, i32) {
    if edges & 3 == 3 {
        dst[..41].fill(0x80);
        return (0, 0x80 * (8 + 1 + 8 + 2));
    }
    let (mut min, mut max, mut sum) = (256, -1, 0i32);
    if edges & 1 == 0 {
        // the left column (area 2) and the one left of it (area 1)
        let mut p = at - 1;
        for i in (0..8).rev() {
            dst[AREA1 + i] = px[p - 1];
            let b = px[p];
            let c = i32::from(b);
            sum += c;
            min = min.min(c);
            max = max.max(c);
            dst[AREA2 + i] = b;
            p += stride;
        }
    }
    if edges & 2 == 0 {
        let p = at - stride;
        let mut c = 0;
        for i in 0..8 {
            c = px[p + i];
            sum += i32::from(c);
            min = min.min(i32::from(c));
            max = max.max(i32::from(c));
        }
        if edges & 4 != 0 {
            dst[AREA5..AREA5 + 8].fill(c);
            dst[AREA4..AREA4 + 8].copy_from_slice(&px[p..p + 8]);
        } else {
            dst[AREA4..AREA4 + 16].copy_from_slice(&px[p..p + 16]);
        }
        dst[AREA6..AREA6 + 8].copy_from_slice(&px[p - stride..p - stride + 8]);
    }
    if edges & 3 != 0 {
        let avg = lo8(((sum + 4) >> 3).cast_unsigned());
        if edges & 1 != 0 {
            dst[AREA1..AREA1 + 17].fill(avg);
        } else {
            dst[AREA3..AREA3 + 25].fill(avg);
        }
        sum += i32::from(avg) * 9;
    } else {
        let c = px[at - 1 - stride];
        dst[AREA3] = c;
        sum += i32::from(c);
    }
    sum += i32::from(dst[AREA5]) + i32::from(dst[AREA5 + 1]);
    (max - min, sum)
}

const ZERO_PREDICTION_WEIGHTS: [u32; 128] = [
    640, 640, 669, 480, 708, 354, 748, 257, 792, 198, 760, 143, 808, 101, 772, 72,
    480, 669, 537, 537, 598, 416, 661, 316, 719, 250, 707, 185, 768, 134, 745, 97,
    354, 708, 416, 598, 488, 488, 564, 388, 634, 317, 642, 241, 716, 179, 706, 132,
    257, 748, 316, 661, 388, 564, 469, 469, 543, 395, 571, 311, 655, 238, 660, 180,
    198, 792, 250, 719, 317, 634, 395, 543, 469, 469, 507, 380, 597, 299, 616, 231,
    161, 855, 206, 788, 266, 710, 340, 623, 411, 548, 455, 455, 548, 366, 576, 288,
    122, 972, 159, 914, 211, 842, 276, 758, 341, 682, 389, 584, 483, 483, 520, 390,
    110, 1172, 144, 1107, 193, 1028, 254, 932, 317, 846, 366, 731, 458, 611, 499, 499,
];

/// spatial_compensation_0..11: the prediction of direction `k` from the edge pixels `s`
fn spatial_compensation(k: Orient, s: &[u8; 42], px: &mut [u8], at: usize, stride: usize) {
    let mut put = |f: &dyn Fn(usize, usize) -> u8| {
        for y in 0..8 {
            for x in 0..8 {
                px[at + y * stride + x] = f(x, y);
            }
        }
    };
    let g = |i: usize| usize::from(s[i]);
    let g4 = |i: usize| u16::from(lo8(g(i))) << 4;
    match k {
        O0 => {
            let mut left = [[0u16; 8]; 2];
            let mut top = [[0u16; 8]; 2];
            for i in 0..8 {
                let a = g4(AREA2 + 7 - i);
                for j in 0..8 {
                    let p = i.abs_diff(j);
                    left[p & 1][j] = left[p & 1][j].wrapping_add(a >> (p >> 1));
                }
            }
            for i in 0..12 {
                let a = g4(AREA4 + i);
                let from = match i {
                    0..8 => 0,
                    8..10 => 5,
                    _ => 7,
                };
                for j in from..8 {
                    let p = i.abs_diff(j);
                    top[p & 1][j] = top[p & 1][j].wrapping_add(a >> (p >> 1));
                }
            }
            for i in 0..8 {
                top[0][i] = top[0][i].wrapping_add(kome::num::lo16((u32::from(top[1][i]) * 181 + 128) >> 8));
                left[0][i] = left[0][i].wrapping_add(kome::num::lo16((u32::from(left[1][i]) * 181 + 128) >> 8));
            }
            put(&|x, y| lo8((u32::from(top[0][x]) * ZERO_PREDICTION_WEIGHTS[y * 16 + x * 2] + u32::from(left[0][y]) * ZERO_PREDICTION_WEIGHTS[y * 16 + x * 2 + 1] + 0x8000) >> 16));
        }
        O1 => put(&|x, y| s[AREA4 + (2 * y + x + 2).min(15)]),
        O2 => put(&|x, y| s[AREA4 + 1 + y + x]),
        O3 => put(&|x, y| s[AREA4 + ((y + 1) >> 1) + x]),
        O4 => put(&|x, _| lo8((g(AREA4 + x) + g(AREA6 + x) + 1) >> 1)),
        O5 => put(&|x, y| if 2 * x < y { s[AREA2 + 9 + 2 * x - y] } else { s[AREA4 + x - ((y + 1) >> 1)] }),
        O6 => put(&|x, y| s[AREA3 + x - y]),
        O7 => put(&|x, y| {
            if let Some(d) = x.checked_sub(2 * y).filter(|&d| d > 0) {
                lo8((g(AREA3 - 1 + d) + g(AREA3 + d) + 1) >> 1)
            } else {
                s[AREA2 + 8 - y + (x >> 1)]
            }
        }),
        O8 => put(&|_, y| lo8((g(AREA1 + 7 - y) + g(AREA2 + 7 - y) + 1) >> 1)),
        O9 => put(&|x, y| s[AREA2 + 6 - (x + y).min(6)]),
        O10 => put(&|x, y| lo8((g(AREA2 + 7 - y) * (8 - x) + g(AREA4 + x) * x + 4) >> 3)),
        O11 => put(&|x, y| lo8((g(AREA2 + 7 - y) * y + g(AREA4 + x) * (8 - y) + 4) >> 3)),
    }
}

/// x8_loop_filter: across the edge before `at` (`a` the step across it, `b` along it)
fn loop_filter(px: &mut [u8], at: usize, a: isize, b: isize, quant: i32) {
    let ql = (quant + 10) >> 3;
    for i in 0..8isize {
        let o = at.cast_signed() + i * b;
        let p = |k: isize| i32::from(px[(o + k * a).cast_unsigned()]);
        let (p0, p1, p2, p3, p4, p5, p6, p7, p8, p9) = (p(-5), p(-4), p(-3), p(-2), p(-1), p(0), p(1), p(2), p(3), p(4));
        let le = |x: i32, y: i32| i32::from((x - y).abs() <= ql);
        let mut t = le(p1, p2) + le(p2, p3) + le(p3, p4) + le(p4, p5);
        if t > 0 {
            t += le(p5, p6) + le(p6, p7) + le(p7, p8) + le(p8, p9) + le(p0, p1);
            if t >= 6 {
                let (mut min, mut max) = (p1, p1);
                for v in [p3, p5, p8] {
                    min = min.min(v);
                    max = max.max(v);
                }
                if max - min < 2 * quant {
                    for v in [p2, p4, p6, p7] {
                        min = min.min(v);
                        max = max.max(v);
                    }
                    if max - min < 2 * quant {
                        let mut set = |k: isize, v: i32| px[(o + k * a).cast_unsigned()] = lo8(v.cast_unsigned());
                        set(-2, (4 * p2 + 3 * p3 + p7 + 4) >> 3);
                        set(-1, (3 * p2 + 3 * p4 + 2 * p7 + 4) >> 3);
                        set(0, (2 * p2 + 3 * p5 + 3 * p7 + 4) >> 3);
                        set(1, (p2 + 3 * p6 + 4 * p7 + 4) >> 3);
                        continue;
                    }
                }
            }
        }
        let x0 = (2 * p3 - 5 * p4 + 5 * p5 - 2 * p6 + 4) >> 3;
        if x0.abs() < quant {
            let x1 = (2 * p1 - 5 * p2 + 5 * p3 - 2 * p4 + 4) >> 3;
            let x2 = (2 * p5 - 5 * p6 + 5 * p7 - 2 * p8 + 4) >> 3;
            let mut x = x0.abs() - x1.abs().min(x2.abs());
            let mut m = p4 - p5;
            if x > 0 && (m ^ x0) < 0 {
                let sign = m >> 31;
                m = (m ^ sign) - sign;
                m >>= 1;
                x = (5 * x) >> 3;
                if x > m {
                    x = m;
                }
                x = (x ^ sign) - sign;
                let (i4, i5) = ((o - a).cast_unsigned(), o.cast_unsigned());
                px[i4] = lo8((i32::from(px[i4]) - x).cast_unsigned());
                px[i5] = lo8((i32::from(px[i5]) + x).cast_unsigned());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Any bytes as an IntraX8 picture of any size and quantizer: never a panic, and it stops
        /// inside the picture.
        #[test]
        fn any_bytes(mb_w in 1usize..8, mb_h in 1usize..8, dquant in 2i32..64, off in 0i32..32, lf in any::<bool>(), bytes in prop::collection::vec(any::<u8>(), 0..800)) {
            let mut x8 = IntraX8::new(mb_w, mb_h);
            let (ls, uvls) = (mb_w * 16, mb_w * 8);
            let (mut y, mut u, mut v) = (vec![0u8; ls * mb_h * 16], vec![0u8; uvls * mb_h * 8], vec![0u8; uvls * mb_h * 8]);
            let mut r = Bits::new(&bytes);
            let rows = x8.decode(&mut r, [Plane { px: &mut y, stride: ls }, Plane { px: &mut u, stride: uvls }, Plane { px: &mut v, stride: uvls }], dquant, off, lf);
            prop_assert!(rows <= mb_h * 2);
        }

        /// The flat prediction of a block with all its edge pixels `v` is `v`, in every direction
        /// that reads only edge pixels.
        #[test]
        fn flat_edges_predict_flat(v in any::<u8>(), k in 1u32..12) {
            let s = [v; 42];
            let mut px = [0u8; 64];
            spatial_compensation(Orient::of(k).unwrap(), &s, &mut px, 0, 8);
            prop_assert!(px.iter().all(|&p| p == v), "direction {k}");
        }
    }

    /// The VLC tables hold every symbol of their code sets once (complete prefix codes).
    #[test]
    fn tables_are_complete() {
        for (pairs, n) in [(&t::X8_AC_QUANT_TABLE[..], 77), (&t::X8_DC_QUANT_TABLE[..], 34), (&t::X8_ORIENT_HIGHQUANT_TABLE[..], 12), (&t::X8_ORIENT_LOWQUANT_TABLE[..], 12)] {
            for set in pairs.chunks(n * 2) {
                let kraft: f64 = set.chunks(2).map(|p| 0.5f64.powi(p[1] as i32)).sum();
                assert!((kraft - 1.0).abs() < 1e-9, "kraft {kraft}");
                let mut syms: Vec<u8> = set.chunks(2).map(|p| p[0]).collect();
                syms.sort();
                syms.dedup();
                assert_eq!(syms.len(), n);
            }
        }
    }
}
