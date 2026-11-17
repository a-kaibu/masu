//! Lazily computed status flags. The JIT records what the last flag-setting operation was and
//! its operands; EFLAGS are worked out from them only when something reads them.

use kome::num::*;
use crate::flags::{self, *};

/// `op` = kind * 4 + size (0: 8 bits, 1: 16, 2: 32, 3: 64); 0: EFLAGS are exact. A raw u32
/// because the JIT's wasm stores it; Rust code goes through [`Op`].
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Lazy {
    pub op: u32,
    /// carry in (ADC/SBB), CF kept through INC/DEC, or the overflow bit of MUL/IMUL
    pub c: u32,
    pub a: u64,
    pub b: u64,
    pub r: u64,
}
// (the JIT's wasm stores the fields at these offsets)
const _: () = assert!(size_of::<Lazy>() == 32 && core::mem::offset_of!(Lazy, a) == 8);

/// The flag-setting operation a lazy record is of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Add,
    Sub,
    /// AND / OR / XOR / TEST: SZP from `r`, CF = OF = 0
    Logic,
    Inc,
    Dec,
    /// shifts: `a` the value, `b` the count (1..31)
    Shl,
    Shr,
    Sar,
    Adc,
    Sbb,
    /// MUL/IMUL: CF = OF = `c`, SZP from `r`
    Mul,
}

impl Kind {
    pub const ALL: [Kind; 11] = [Kind::Add, Kind::Sub, Kind::Logic, Kind::Inc, Kind::Dec, Kind::Shl, Kind::Shr, Kind::Sar, Kind::Adc, Kind::Sbb, Kind::Mul];

    /// Its number in [`Lazy::op`] (never 0: that is "exact").
    pub const fn code(self) -> u32 {
        match self {
            Kind::Add => 1,
            Kind::Sub => 2,
            Kind::Logic => 3,
            Kind::Inc => 4,
            Kind::Dec => 5,
            Kind::Shl => 6,
            Kind::Shr => 7,
            Kind::Sar => 8,
            Kind::Adc => 9,
            Kind::Sbb => 10,
            Kind::Mul => 11,
        }
    }

    pub const fn from_code(code: u32) -> Option<Kind> {
        // (a match, not a search of ALL: Lazy::status is on the JIT's flag path)
        Some(match code {
            1 => Kind::Add,
            2 => Kind::Sub,
            3 => Kind::Logic,
            4 => Kind::Inc,
            5 => Kind::Dec,
            6 => Kind::Shl,
            7 => Kind::Shr,
            8 => Kind::Sar,
            9 => Kind::Adc,
            10 => Kind::Sbb,
            11 => Kind::Mul,
            _ => return None,
        })
    }
}

/// The operand size of a lazy record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Size {
    B8,
    B16,
    B32,
    B64,
}

impl Size {
    pub const fn code(self) -> u32 {
        match self {
            Size::B8 => 0,
            Size::B16 => 1,
            Size::B32 => 2,
            Size::B64 => 3,
        }
    }

    pub const fn from_code(code: u32) -> Size {
        match code & 3 {
            0 => Size::B8,
            1 => Size::B16,
            2 => Size::B32,
            _ => Size::B64,
        }
    }

    pub const fn bits(self) -> u32 {
        8 << self.code()
    }

    /// The 32-bit JIT's sizes: 8 and 16, anything else 32.
    pub const fn of_bits32(bits: u32) -> Size {
        match bits {
            8 => Size::B8,
            16 => Size::B16,
            _ => Size::B32,
        }
    }
}

/// A lazy operation: what [`Lazy::op`] holds, typed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Op {
    pub kind: Kind,
    pub size: Size,
}

impl Op {
    pub const fn new(kind: Kind, size: Size) -> Op {
        Op { kind, size }
    }

    pub const fn raw(self) -> u32 {
        self.kind.code() * 4 + self.size.code()
    }

    /// None: 0 (EFLAGS exact) or a number no kind has.
    pub fn decode(raw: u32) -> Option<Op> {
        Some(Op { kind: Kind::from_code(raw >> 2)?, size: Size::from_code(raw) })
    }
}

// (the raw kind numbers, for the long-mode JIT's lazy records)
pub const ADD: u32 = Kind::Add.code();
pub const SUB: u32 = Kind::Sub.code();
pub const LOGIC: u32 = Kind::Logic.code();
pub const INC: u32 = Kind::Inc.code();
pub const DEC: u32 = Kind::Dec.code();
pub const SHL: u32 = Kind::Shl.code();
pub const SHR: u32 = Kind::Shr.code();
pub const SAR: u32 = Kind::Sar.code();
pub const ADC: u32 = Kind::Adc.code();
pub const SBB: u32 = Kind::Sbb.code();
pub const MUL: u32 = Kind::Mul.code();

pub fn size_code(bits: u32) -> u32 {
    Size::of_bits32(bits).code()
}

pub fn op(kind: u32, bits: u32) -> u32 {
    kind * 4 + size_code(bits)
}

impl Lazy {
    /// The status flags (CF PF AF ZF SF OF) this operation produced.
    pub fn status(&self) -> u32 {
        let Some(Op { kind, size }) = Op::decode(self.op) else { return 0 };
        let bits = size.bits();
        let (a64, b64, r64, c) = (self.a, self.b, self.r, self.c);
        // (shifts: up to 32 bits only, the long-mode JIT works out 64-bit ones itself)
        let (a, b) = (lo32(a64), lo32(b64));
        match kind {
            Kind::Add => flags::add64(a64, b64, 0, bits).1,
            Kind::Sub => flags::sub64(a64, b64, 0, bits).1,
            Kind::Adc => flags::add64(a64, b64, c, bits).1,
            Kind::Sbb => flags::sub64(a64, b64, c, bits).1,
            Kind::Logic => flags::szp64(r64, bits),
            Kind::Inc => (flags::add64(a64, 1, 0, bits).1 & !CF) | (c & CF),
            Kind::Dec => (flags::sub64(a64, 1, 0, bits).1 & !CF) | (c & CF),
            Kind::Shl => {
                let m = flags::mask(bits);
                let wide = u64::from(a & m) << b;
                let r = lo32(wide) & m;
                let cf = if b <= bits { lo32((wide >> bits) & 1) } else { 0 };
                let of = u32::from(r & flags::sign(bits) != 0) ^ cf;
                flags::szp(r, bits) | cf * CF | of * OF
            }
            Kind::Shr => {
                let a = a & flags::mask(bits);
                let r = if b >= 32 { 0 } else { a >> b };
                let cf = if b <= bits { (a >> (b - 1)) & 1 } else { 0 };
                let of = u32::from(a & flags::sign(bits) != 0);
                flags::szp(r, bits) | cf * CF | of * OF
            }
            Kind::Sar => {
                let sx = (lo32(a << (32 - bits)).cast_signed()) >> (32 - bits);
                let k = b.min(31);
                let r = lo32(sx >> k) & flags::mask(bits);
                let cf = if b > 31 { u32::from(sx < 0) } else { lo32((sx >> (k - 1).min(31)) & 1) };
                flags::szp(r, bits) | cf * CF
            }
            Kind::Mul => flags::szp64(r64, bits) | if c != 0 { CF | OF } else { 0 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every raw `op` decodes as the old arithmetic read it (kind = op >> 2 when it is one of
    /// the eleven, size = op & 3), and Op::raw is its inverse.
    #[test]
    fn op_codes_round_trip() {
        for raw in 0u32..0x400 {
            match Op::decode(raw) {
                Some(op) => {
                    assert!((1..=11).contains(&(raw >> 2)), "{raw}");
                    assert_eq!(op.raw(), raw);
                    assert_eq!(op.size.bits(), 8 << (raw & 3));
                }
                None => assert!(raw >> 2 == 0 || raw >> 2 > 11, "{raw}"),
            }
        }
        for k in Kind::ALL {
            assert_eq!(Kind::from_code(k.code()), Some(k));
            for sz in [Size::B8, Size::B16, Size::B32, Size::B64] {
                assert_eq!(Op::decode(Op::new(k, sz).raw()), Some(Op::new(k, sz)));
            }
        }
        assert_eq!([8, 16, 32, 64, 1].map(|b| Size::of_bits32(b).code()), [0, 1, 2, 2, 2]);
    }

    /// The 64-bit helpers give the 32-bit ones' flags at 8, 16 and 32 bits.
    #[test]
    fn widths_agree() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..200_000 {
            let (a, b, c) = (next(), next(), (next() & 1) as u32);
            for bits in [8, 16, 32] {
                let (a32, b32) = (a as u32, b as u32);
                assert_eq!(flags::add(a32, b32, c, bits).1, flags::add64(a, b, c, bits).1, "add {a:x} {b:x} {c} {bits}");
                assert_eq!(flags::sub(a32, b32, c, bits).1, flags::sub64(a, b, c, bits).1, "sub {a:x} {b:x} {c} {bits}");
                assert_eq!(flags::logic(a32, bits), flags::szp64(a, bits), "logic {a:x} {bits}");
            }
        }
    }
}

/// Kani: what the JIT records (the operation and its operands) gives the flags the
/// interpreter's eager helpers give, and shifts give the SDM's (Vol. 2 SAL/SAR/SHL/SHR: CF the
/// last bit shifted out, OF for 1-bit shifts only), for every operand, count and width.
#[cfg(kani)]
mod proofs {
    use super::*;

    fn any_bits() -> u32 {
        let k: u8 = kani::any_where(|&k| k < 3);
        8 << k
    }
    fn lazy(kind: u32, bits: u32, a: u64, b: u64, c: u32, r: u64) -> Lazy {
        Lazy { op: op(kind, bits), c, a, b, r }
    }

    #[kani::proof]
    fn arithmetic_is_the_eager_flags() {
        let bits = any_bits();
        let (a, b): (u32, u32) = (kani::any(), kani::any());
        let c: u32 = kani::any_where(|&c| c < 2);
        assert_eq!(lazy(ADD, bits, a as u64, b as u64, 0, 0).status(), flags::add(a, b, 0, bits).1);
        assert_eq!(lazy(SUB, bits, a as u64, b as u64, 0, 0).status(), flags::sub(a, b, 0, bits).1);
        assert_eq!(lazy(ADC, bits, a as u64, b as u64, c, 0).status(), flags::add(a, b, c, bits).1);
        assert_eq!(lazy(SBB, bits, a as u64, b as u64, c, 0).status(), flags::sub(a, b, c, bits).1);
        assert_eq!(lazy(LOGIC, bits, 0, 0, 0, a as u64).status(), flags::logic(a, bits));
    }

    /// INC / DEC: ADD / SUB of 1 that keep CF.
    #[kani::proof]
    fn inc_dec_keep_cf() {
        let bits = any_bits();
        let a: u32 = kani::any();
        let cf: u32 = kani::any_where(|&c| c == 0 || c == CF);
        assert_eq!(lazy(INC, bits, a as u64, 0, cf, 0).status(), (flags::add(a, 1, 0, bits).1 & !CF) | cf);
        assert_eq!(lazy(DEC, bits, a as u64, 0, cf, 0).status(), (flags::sub(a, 1, 0, bits).1 & !CF) | cf);
    }

    fn result_flags(r: u32, bits: u32) -> u32 {
        flags::szp(r, bits)
    }

    #[kani::proof]
    fn shifts_are_the_sdm_shifts() {
        let bits = any_bits();
        let a: u32 = kani::any::<u32>() & flags::mask(bits);
        let n: u32 = kani::any_where(|&n| (1..32).contains(&n));
        let top = |v: u32| (v >> (bits - 1)) & 1;
        // SHL
        let r = if n >= bits { 0 } else { (a << n) & flags::mask(bits) };
        let f = lazy(SHL, bits, a as u64, n as u64, 0, 0).status();
        assert_eq!(f & (ZF | SF | PF), result_flags(r, bits));
        if n <= bits {
            assert_eq!(f & CF != 0, (a >> (bits - n)) & 1 == 1);
        }
        if n == 1 {
            assert_eq!(f & OF != 0, top(r) != (a >> (bits - 1)) & 1);
        }
        // SHR
        let r = if n >= bits { 0 } else { a >> n };
        let f = lazy(SHR, bits, a as u64, n as u64, 0, 0).status();
        assert_eq!(f & (ZF | SF | PF), result_flags(r, bits));
        if n <= bits {
            assert_eq!(f & CF != 0, (a >> (n - 1)) & 1 == 1);
        }
        if n == 1 {
            assert_eq!(f & OF != 0, top(a) == 1);
        }
        // SAR: the sign bit fills
        let neg = top(a) == 1;
        let sx = if neg { a as i64 - (1i64 << bits) } else { a as i64 };
        let r = ((sx >> n.min(63)) as u32) & flags::mask(bits);
        let f = lazy(SAR, bits, a as u64, n as u64, 0, 0).status();
        assert_eq!(f & (ZF | SF | PF), result_flags(r, bits));
        let last_out = if n <= bits { (sx >> (n - 1)) & 1 == 1 } else { neg };
        assert_eq!(f & CF != 0, last_out);
        if n == 1 {
            assert_eq!(f & OF, 0);
        }
    }
}
