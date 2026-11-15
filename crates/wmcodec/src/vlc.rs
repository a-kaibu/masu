//! Variable-length code tables (FFmpeg's VLC: vlc_init / ff_vlc_init_from_lengths / get_vlc2):
//! lookups of `bits` bits, with sub-tables for longer codes.

use kome::num::*;
use crate::bits::Bits;


#[derive(Clone, Copy, Debug, Default)]
enum Entry {
    /// no code starts with these bits
    #[default]
    None,
    /// symbol `sym`, its code `len` bits long
    Sym { sym: i32, len: u8 },
    /// a longer code: look `nb` more bits up in the table at `base`
    Sub { base: u32, nb: u8 },
}

#[derive(Clone, Debug)]
pub struct Vlc {
    table: Vec<Entry>,
    bits: u32,
}

impl Vlc {
    /// Codes given as (code, length, symbol), the code in the low `length` bits.
    pub fn new(bits: u32, codes: &[(u32, u8, i32)]) -> Vlc {
        let mut v = Vlc { table: Vec::new(), bits };
        let cs: Vec<(u32, u8, i32)> = codes.iter().copied().filter(|c| c.1 > 0 && c.1 <= 32).collect();
        v.build(0, bits, &cs);
        v
    }
    /// Codes of the lengths `lens` (symbols `syms`, or their index + `offset`), assigned in
    /// list order (ff_vlc_init_from_lengths).
    pub fn from_lengths(bits: u32, lens: &[u8], syms: Option<&[i32]>, offset: i32) -> Vlc {
        let mut code: u64 = 0;
        let mut cs = Vec::new();
        for (i, &l) in (0i32..).zip(lens) {
            let s = syms.map_or(i, |s| s[us(i.cast_unsigned())]) + offset;
            if l > 0 {
                let shift = 32 - u32::from(l);
                cs.push((kome::num::lo32(code >> shift), l, s));
                code += 1u64 << shift;
            }
        }
        Vlc::new(bits, &cs)
    }
    /// Codes `codes[i]` of lengths `lens[i]` for symbols i (vlc_init's tables).
    pub fn from_codes(bits: u32, lens: &[u8], codes: &[u32]) -> Vlc {
        let cs: Vec<(u32, u8, i32)> = (0i32..).zip(lens.iter().zip(codes)).map(|(i, (&l, &c))| (c, l, i)).collect();
        Vlc::new(bits, &cs)
    }

    /// Fills a table of `nb` bits at `base` for codes that start with `prefix_len` bits already
    /// matched (`codes` hold what is left of them, in their low bits).
    fn build(&mut self, base: usize, nb: u32, codes: &[(u32, u8, i32)]) {
        let size = 1usize << nb;
        if self.table.len() < base + size {
            self.table.resize(base + size, Entry::default());
        }
        // short codes fill all the entries they prefix
        let mut long: std::collections::BTreeMap<u32, Vec<(u32, u8, i32)>> = Default::default();
        for &(code, len, sym) in codes {
            if u32::from(len) <= nb {
                let shift = nb - u32::from(len);
                let start = us(code << shift);
                for k in 0..(1usize << shift) {
                    let e = &mut self.table[base + start + k];
                    if let Entry::None = e {
                        *e = Entry::Sym { sym, len };
                    }
                }
            } else {
                let rest = len - kome::num::lo8(nb);
                let idx = code >> rest;
                long.entry(idx).or_default().push((code & ((1u32 << rest) - 1), rest, sym));
            }
        }
        for (idx, sub) in long {
            let max = sub.iter().map(|c| c.1).max().unwrap_or(1);
            let snb = max.min(kome::num::lo8(self.bits));
            let sbase = self.table.len();
            let Ok(b) = u32::try_from(sbase) else { return };
            self.table[base + us(idx)] = Entry::Sub { base: b, nb: snb };
            self.build(sbase, u32::from(snb), &sub);
        }
    }

    /// The symbol of the code at the reader (-1: no such code; its bits are taken anyway).
    #[inline]
    pub fn read(&self, r: &mut Bits) -> i32 {
        let mut base = 0usize;
        let mut nb = self.bits;
        loop {
            let idx = us(r.peek(nb));
            match self.table[base + idx] {
                Entry::Sym { sym, len } => {
                    r.skip(usize::from(len));
                    return sym;
                }
                Entry::None => {
                    r.skip(us(nb));
                    return -1;
                }
                Entry::Sub { base: b, nb: n } => {
                    r.skip(us(nb));
                    base = us(b);
                    nb = u32::from(n);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// A prefix-free code of random lengths (canonical, Kraft sum <= 1).
    fn prefix_code(lens: &[u8]) -> Vec<(u32, u8, i32)> {
        let mut order: Vec<usize> = (0..lens.len()).collect();
        order.sort_by_key(|&i| lens[i]);
        let mut code: u64 = 0;
        let mut out = Vec::new();
        for i in order {
            let l = lens[i] as u32;
            if code >= 1 << 32 {
                break;
            }
            out.push(((code >> (32 - l)) as u32, lens[i], i as i32));
            code += 1 << (32 - l);
        }
        out
    }

    proptest! {
        /// Every code reads back as its symbol, whatever bits follow it, with any lookup width.
        #[test]
        fn codes_read_back(lens in prop::collection::vec(1u8..=22, 1..300), bits in 3u32..12, tail in any::<u32>()) {
            let cs = prefix_code(&lens);
            let v = Vlc::new(bits, &cs);
            for &(code, len, sym) in &cs {
                let word = ((code as u64) << (64 - len as u32)) | ((tail as u64 | (tail as u64) << 32) >> len);
                let b = word.to_be_bytes();
                let mut r = Bits::new(&b);
                prop_assert_eq!(v.read(&mut r), sym);
                prop_assert_eq!(r.count(), len as usize);
            }
        }
    }
}
