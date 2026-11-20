#![allow(non_snake_case)]
//! ntdll's integer <-> string conversions: RtlCharToInteger, RtlUnicodeStringToInteger,
//! RtlIntegerToChar, RtlIntegerToUnicodeString.
//!
//! Written from the clean-room notes (project folder sake-re/ntdll/rtl-integer-strings.md) and
//! Microsoft Learn; checked against Windows by kikizake's ntstr suite (tests/winref, golden
//! ntstr-*) and Wine's ntdll:rtlstr / ntdll:string tests.

use crate::api::{implement, Ctx};
use crate::mem;

/// the NTSTATUS values these functions return
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Success,
    BufferOverflow,
    InvalidParameter,
}

impl Status {
    pub const fn code(self) -> u32 {
        match self {
            Status::Success => 0,
            Status::BufferOverflow => 0x8000_0005,
            Status::InvalidParameter => 0xc000_000d,
        }
    }
}

/// a base the functions accept (0 resolved by the caller)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Radix {
    Bin,
    Oct,
    Dec,
    Hex,
}

impl Radix {
    /// 2, 8, 10, 16; None for any other value (0 included: what it means differs by function)
    pub const fn of(base: u32) -> Option<Radix> {
        match base {
            2 => Some(Radix::Bin),
            8 => Some(Radix::Oct),
            10 => Some(Radix::Dec),
            16 => Some(Radix::Hex),
            _ => None,
        }
    }
    pub const fn value(self) -> u32 {
        match self {
            Radix::Bin => 2,
            Radix::Oct => 8,
            Radix::Dec => 10,
            Radix::Hex => 16,
        }
    }
    /// the base a prefix letter after a '0' names: lowercase x, o, b only
    const fn prefix(c: u32) -> Option<Radix> {
        match c {
            0x78 => Some(Radix::Hex),
            0x6f => Some(Radix::Oct),
            0x62 => Some(Radix::Bin),
            _ => None,
        }
    }
}

/// What base a call asked for: the prefix decides (0), or a valid base
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    ByPrefix,
    Fixed(Radix),
}

impl Base {
    /// None: a base no function takes (STATUS_INVALID_PARAMETER)
    pub const fn of(base: u32) -> Option<Base> {
        if base == 0 {
            return Some(Base::ByPrefix);
        }
        match Radix::of(base) {
            Some(r) => Some(Base::Fixed(r)),
            None => None,
        }
    }
}

const fn digit(c: u32) -> Option<u32> {
    match c {
        0x30..=0x39 => Some(c.wrapping_sub(0x30)),
        0x41..=0x46 => Some(c.wrapping_sub(0x37)),
        0x61..=0x66 => Some(c.wrapping_sub(0x57)),
        _ => None,
    }
}

/// the number from character `i` on (past the white space): sign, prefix, digits. `at(k)` is
/// character k, 0 at and past the end; a 0 character ends the number.
fn number(at: &dyn Fn(usize) -> u32, mut i: usize, base: Base) -> u32 {
    let sign = at(i);
    if sign == 0x2b || sign == 0x2d {
        i = i.saturating_add(1);
    }
    let radix = match base {
        Base::Fixed(r) => r,
        Base::ByPrefix if at(i) == 0x30 => match Radix::prefix(at(i.saturating_add(1))) {
            Some(r) => {
                i = i.saturating_add(2);
                r
            }
            // (the '0' adds nothing; the digits go on after it)
            None => {
                i = i.saturating_add(1);
                Radix::Dec
            }
        },
        Base::ByPrefix => Radix::Dec,
    };
    let mut v: u32 = 0;
    while let Some(d) = digit(at(i)) {
        if d >= radix.value() {
            break;
        }
        v = v.wrapping_mul(radix.value()).wrapping_add(d);
        i = i.saturating_add(1);
    }
    if sign == 0x2d { v.wrapping_neg() } else { v }
}

/// RtlCharToInteger over `at(k)` (byte k of the string): the status and the value (None: the
/// value is not written). White space is any byte <= 0x20 as a signed char, passed only while
/// the byte after it is not 0 (so a blank at the end, or alone, stays and makes 0).
pub fn char_to_integer(at: &dyn Fn(usize) -> u8, base: u32) -> (Status, Option<u32>) {
    let mut i = 0;
    while at(i).cast_signed() <= 0x20 {
        if at(i.saturating_add(1)) == 0 {
            break;
        }
        i = i.saturating_add(1);
    }
    let Some(base) = Base::of(base) else {
        return (Status::InvalidParameter, None);
    };
    if at(i).cast_signed() <= 0x20 {
        return (Status::Success, Some(0));
    }
    let chars = |k: usize| u32::from(at(k));
    (Status::Success, Some(number(&chars, i, base)))
}

/// RtlUnicodeStringToInteger of a UNICODE_STRING of `length` bytes; `unit(k)` is unit k (only
/// k < length / 2 is read): the status and the value, which is always written
pub fn unicode_to_integer(unit: &dyn Fn(usize) -> u16, length: u16, base: u32) -> (Status, u32) {
    if length == 0 || length & 1 == 1 {
        return (Status::InvalidParameter, 0);
    }
    let n = usize::from(length >> 1);
    let at = |k: usize| if k < n { u32::from(unit(k)) } else { 0 };
    let mut i = 0;
    while i < n && at(i) <= 0x20 {
        i = i.saturating_add(1);
    }
    let Some(base) = Base::of(base) else {
        return (Status::InvalidParameter, 0);
    };
    (Status::Success, number(&at, i, base))
}

/// the digits of `value` in `radix` (unsigned, uppercase, at least one)
pub fn digits(value: u32, radix: Radix) -> Vec<u8> {
    let mut v = value;
    let mut d = Vec::new();
    loop {
        d.push(b"0123456789ABCDEF".get(kome::num::us(v.checked_rem(radix.value()).unwrap_or(0))).copied().unwrap_or(b'0'));
        v = v.checked_div(radix.value()).unwrap_or(0);
        if v == 0 {
            break;
        }
    }
    d.reverse();
    d
}

/// What RtlIntegerToChar writes for a length: the bytes (None: nothing, STATUS_BUFFER_OVERFLOW)
pub fn integer_to_char(value: u32, base: u32, length: i32) -> Result<Vec<u8>, Status> {
    let radix = match Base::of(base) {
        Some(Base::ByPrefix) => Radix::Dec,
        Some(Base::Fixed(r)) => r,
        None => return Err(Status::InvalidParameter),
    };
    let d = digits(value, radix);
    let n = kome::num::u64_of(d.len());
    let l = u64::from(length.unsigned_abs());
    if n > l {
        return Err(Status::BufferOverflow);
    }
    if length >= 0 {
        // (a NUL only where there is room past the digits)
        let mut v = d;
        if n < l {
            v.push(0);
        }
        Ok(v)
    } else {
        // (exactly |length| bytes, the digits right-aligned after '0's, no NUL)
        let mut v = vec![b'0'; kome::num::us(l.saturating_sub(n))];
        v.extend_from_slice(&d);
        Ok(v)
    }
}

fn RtlCharToInteger(c: &mut Ctx) -> u32 {
    let (s, base, out) = (c.a(0), c.a(1), c.a(2));
    let at = |k: usize| mem::r8(mem::ga(s) + kome::num::u32_of(k));
    let (st, v) = char_to_integer(&at, base);
    if let Some(v) = v {
        mem::w32(out, v);
    }
    st.code()
}

fn RtlUnicodeStringToInteger(c: &mut Ctx) -> u32 {
    let (us, base, out) = (c.a(0), c.a(1), c.a(2));
    let length = mem::r16(us);
    let buf = mem::rp(mem::ga(us) + mem::ps());
    let unit = |k: usize| mem::r16(mem::ga(buf).at(kome::num::u32_of(k), 2));
    let (st, v) = unicode_to_integer(&unit, length, base);
    mem::w32(out, v);
    st.code()
}

fn RtlIntegerToChar(c: &mut Ctx) -> u32 {
    let (value, base, length, s) = (c.a(0), c.a(1), c.a(2).cast_signed(), c.a(3));
    match integer_to_char(value, base, length) {
        Ok(b) => {
            for (k, &x) in b.iter().enumerate() {
                mem::w8(mem::ga(s) + kome::num::u32_of(k), x);
            }
            Status::Success.code()
        }
        Err(st) => st.code(),
    }
}

/// RtlIntegerToUnicodeString(value, base, UNICODE_STRING *): RtlIntegerToChar into 33 bytes,
/// then RtlAnsiStringToUnicodeString(String, the digits, FALSE)
fn RtlIntegerToUnicodeString(c: &mut Ctx) -> u32 {
    let (value, base, us) = (c.a(0), c.a(1), c.a(2));
    let d = match integer_to_char(value, base, 33) {
        Ok(mut b) => {
            b.retain(|&x| x != 0);
            b
        }
        Err(st) => return st.code(),
    };
    let w: Vec<u16> = d.iter().map(|&b| u16::from(b)).collect();
    crate::k32::counted_out(us, &w, 2, false)
}

pub fn init() {
    implement("ntdll", "RtlCharToInteger", RtlCharToInteger);
    implement("ntdll", "RtlUnicodeStringToInteger", RtlUnicodeStringToInteger);
    implement("ntdll", "RtlIntegerToChar", RtlIntegerToChar);
    implement("ntdll", "RtlIntegerToUnicodeString", RtlIntegerToUnicodeString);
}

#[cfg(test)]
#[path = "ntstr_tests.rs"]
mod tests;
