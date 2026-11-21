#![allow(non_snake_case)]
//! ntdll's own C runtime: its character classes, case mapping, case-insensitive compares and
//! integer conversions. They are not msvcrt's (no locale; ASCII and Latin-1 tables, no base
//! checks in _itoa, wrapping atoi), so ntdll's exports get these and msvcrt keeps its own.
//!
//! Written from the clean-room notes (project folder sake-re/ntdll/crt-case.md and
//! crt-numbers.md); checked by Wine's ntdll:string tests.

use crate::api::{implement, Ctx};
use crate::mem;

/// C1 character class bits (the CRT's _UPPER .. _ALPHA)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Class(u16);

impl Class {
    pub const UPPER: Class = Class(0x1);
    pub const LOWER: Class = Class(0x2);
    pub const DIGIT: Class = Class(0x4);
    pub const SPACE: Class = Class(0x8);
    pub const PUNCT: Class = Class(0x10);
    pub const CONTROL: Class = Class(0x20);
    pub const BLANK: Class = Class(0x40);
    pub const HEX: Class = Class(0x80);
    pub const ALPHA: Class = Class(0x100);
    const NONE: Class = Class(0);
    pub const fn bits(self) -> u16 {
        self.0
    }
    const fn or(self, o: Class) -> Class {
        Class(self.0 | o.0)
    }
}

/// what an is*/isw* function asks: its mask
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Test {
    Upper,
    Lower,
    Digit,
    Space,
    Punct,
    Cntrl,
    Xdigit,
    Alpha,
    Alnum,
    Graph,
    Print,
}

impl Test {
    pub const fn mask(self) -> u16 {
        match self {
            Test::Upper => 0x1,
            Test::Lower => 0x2,
            Test::Digit => 0x4,
            Test::Space => 0x8,
            Test::Punct => 0x10,
            Test::Cntrl => 0x20,
            Test::Xdigit => 0x80,
            Test::Alpha => 0x103,
            Test::Alnum => 0x107,
            Test::Graph => 0x117,
            Test::Print => 0x157,
        }
    }
    const ALL: [(Test, &'static str); 11] = [
        (Test::Upper, "upper"),
        (Test::Lower, "lower"),
        (Test::Digit, "digit"),
        (Test::Space, "space"),
        (Test::Punct, "punct"),
        (Test::Cntrl, "cntrl"),
        (Test::Xdigit, "xdigit"),
        (Test::Alpha, "alpha"),
        (Test::Alnum, "alnum"),
        (Test::Graph, "graph"),
        (Test::Print, "print"),
    ];
}

/// _pwctype: the class of U+0000..U+00FF (crt-case.md)
pub const fn wide_class(c: u8) -> Class {
    let p = Class::PUNCT;
    match c {
        0x00..=0x08 | 0x0e..=0x1f | 0x7f..=0x9f => Class::CONTROL,
        0x09 => Class::SPACE.or(Class::CONTROL).or(Class::BLANK),
        0x0a..=0x0d => Class::SPACE.or(Class::CONTROL),
        0x20 | 0xa0 => Class::SPACE.or(Class::BLANK),
        0x21..=0x2f | 0x3a..=0x40 | 0x5b..=0x60 | 0x7b..=0x7e | 0xa1..=0xb1 | 0xb4..=0xb8 | 0xba..=0xbf | 0xd7 | 0xf7 => p,
        0xb2 | 0xb3 | 0xb9 => p.or(Class::DIGIT),
        0x30..=0x39 => Class::DIGIT.or(Class::HEX),
        0x41..=0x46 => Class::UPPER.or(Class::HEX).or(Class::ALPHA),
        0x47..=0x5a | 0xc0..=0xd6 | 0xd8..=0xde => Class::UPPER.or(Class::ALPHA),
        0x61..=0x66 => Class::LOWER.or(Class::HEX).or(Class::ALPHA),
        0x67..=0x7a | 0xdf..=0xf6 | 0xf8..=0xff => Class::LOWER.or(Class::ALPHA),
    }
}

/// _pctype: the "C" table of 0x00..0x7F (letters without _ALPHA, TAB without _BLANK), 0 above
pub const fn char_class(c: u8) -> Class {
    match c {
        0x80..=0xff => Class::NONE,
        0x09 => Class::SPACE.or(Class::CONTROL),
        _ => Class(wide_class(c).0 & !Class::ALPHA.0),
    }
}

/// iswctype(c, mask)
pub fn iswctype(c: u16, mask: u16) -> u16 {
    match u8::try_from(c) {
        Ok(b) => wide_class(b).bits() & mask,
        Err(_) => 0,
    }
}

/// is*(c, mask): EOF (-1) and the low byte of anything else
pub fn isctype(c: i32, mask: u16) -> u16 {
    if c == -1 { 0 } else { char_class(kome::num::lo8(c)).bits() & mask }
}

/// ntdll's towlower: +0x20 for the Latin-1 upper case letters only
pub fn towlower(c: u16) -> u16 {
    if iswctype(c, Class::UPPER.bits()) != 0 { c.wrapping_add(0x20) } else { c }
}

/// ntdll's tolower: the whole int + 0x20 where its low byte is an ASCII upper case letter
pub fn tolower(c: i32) -> i32 {
    if isctype(c, Class::UPPER.bits()) != 0 { c.wrapping_add(0x20) } else { c }
}

/// the ASCII-only fold of _wcsicmp / _wcsnicmp
const fn fold(u: u16) -> u16 {
    if u.wrapping_sub(0x41) <= 0x19 { u.wrapping_add(0x20) } else { u }
}

/// _wcsnicmp over `at(k)`; n None is _wcsicmp
pub fn wcsnicmp(a: &dyn Fn(usize) -> u16, b: &dyn Fn(usize) -> u16, n: Option<usize>) -> i32 {
    if n == Some(0) {
        return 0;
    }
    let mut k = 0;
    loop {
        let (x, y) = (fold(a(k)), fold(b(k)));
        k = k.saturating_add(1);
        if n.is_some_and(|n| k >= n) || x == 0 || x != y {
            return i32::from(x).wrapping_sub(i32::from(y));
        }
    }
}

/// a digit of _itoa's: 0-9, then 'a' onward in 8 (or 16) bits, past 'z' for bases over 36
fn digit_char(d: u64) -> u16 {
    if d <= 9 { kome::num::lo16(d.wrapping_add(0x30)) } else { kome::num::lo16(d.wrapping_add(0x57)) }
}

/// _itoa's text (as code units; the char versions keep the low byte): `negative` writes '-'
/// and the two's complement of `value`; `bits` 32 or 64. No base check (crt-numbers.md); None
/// for a base of 0 or 1, where Windows faults or never ends.
pub fn xtoa(value: u64, base: u32, negative: bool, bits: u32) -> Option<Vec<u16>> {
    if base < 2 {
        return None;
    }
    let mask = if bits == 32 { u64::from(u32::MAX) } else { u64::MAX };
    let mut v = if negative { value.wrapping_neg() & mask } else { value & mask };
    let mut d = Vec::new();
    loop {
        d.push(digit_char(v.checked_rem(u64::from(base)).unwrap_or(0)));
        v = v.checked_div(u64::from(base)).unwrap_or(0);
        if v == 0 {
            break;
        }
    }
    if negative {
        d.push(0x2d);
    }
    d.reverse();
    Some(d)
}

/// the digit value of a unit (_wchartodigit): the scripts' zeros
const ZEROS: [u16; 18] = [0x30, 0x660, 0x6f0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xc66, 0xce6, 0xd66, 0xe50, 0xed0, 0xf20, 0x1040, 0x17e0, 0x1810, 0xff10];

pub fn wchartodigit(c: u16) -> Option<u32> {
    ZEROS.iter().find_map(|&z| {
        let d = c.wrapping_sub(z);
        (d <= 9).then_some(u32::from(d))
    })
}

/// how strtol's core treats the value
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// strtol / wcstol: saturate to the signed range, ERANGE
    Signed,
    /// strtoul / wcstoul: all ones on overflow, ERANGE
    Unsigned,
    /// atoi / _wtoi: wrap, no ERANGE
    Wrapping,
}

/// strtol's answer: the value (in `bits`), how many characters were used (0: no number), and
/// whether it overflowed (ERANGE)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub value: u64,
    pub used: usize,
    pub range: bool,
}

/// What a character of the string is to strtol's core
pub trait Unit: Copy {
    fn space(self) -> bool;
    fn digit(self) -> Option<u32>;
    fn letter(self) -> Option<u32>;
    fn code(self) -> u32;
}

impl Unit for u16 {
    fn space(self) -> bool {
        iswctype(self, Class::SPACE.bits()) != 0
    }
    fn digit(self) -> Option<u32> {
        wchartodigit(self)
    }
    fn letter(self) -> Option<u32> {
        match self {
            0x41..=0x5a => Some(u32::from(self).wrapping_sub(0x37)),
            0x61..=0x7a => Some(u32::from(self).wrapping_sub(0x57)),
            _ => None,
        }
    }
    fn code(self) -> u32 {
        u32::from(self)
    }
}

impl Unit for u8 {
    fn space(self) -> bool {
        char_class(self).bits() & Class::SPACE.bits() != 0
    }
    fn digit(self) -> Option<u32> {
        (char_class(self).bits() & Class::DIGIT.bits() != 0).then(|| u32::from(self).wrapping_sub(0x30))
    }
    fn letter(self) -> Option<u32> {
        u16::from(self).letter()
    }
    fn code(self) -> u32 {
        u32::from(self)
    }
}

/// strtol's core over `at(k)` (0 at the end). None: a base other than 0 and 2..36 (the
/// invalid-parameter handler; 0 returned).
pub fn strtox<U: Unit>(at: &dyn Fn(usize) -> U, base: u32, kind: Kind, bits: u32) -> Option<Parsed> {
    if base == 1 || base > 36 {
        return None;
    }
    let mut i = 0;
    while at(i).space() {
        i = i.saturating_add(1);
    }
    let negative = at(i).code() == 0x2d;
    if negative || at(i).code() == 0x2b {
        i = i.saturating_add(1);
    }
    let next_x = |k: usize| matches!(at(k.saturating_add(1)).code(), 0x58 | 0x78);
    let zero = |k: usize| at(k).digit() == Some(0);
    let base = match base {
        0 if !zero(i) => 10,
        0 if next_x(i) => {
            i = i.saturating_add(2);
            16
        }
        0 => 8,
        16 if zero(i) && next_x(i) => {
            i = i.saturating_add(2);
            16
        }
        b => b,
    };
    let max: u64 = if bits == 32 { u64::from(u32::MAX) } else { u64::MAX };
    let (lim, rem) = (max.checked_div(u64::from(base)).unwrap_or(0), max.checked_rem(u64::from(base)).unwrap_or(0));
    let (mut v, mut any, mut over) = (0u64, false, false);
    loop {
        let c = at(i);
        let Some(d) = c.digit().or_else(|| c.letter()) else { break };
        if d >= base {
            break;
        }
        any = true;
        let d = u64::from(d);
        if kind == Kind::Wrapping {
            v = v.wrapping_mul(u64::from(base)).wrapping_add(d) & max;
        } else if v < lim || (v == lim && d <= rem) {
            v = v.wrapping_mul(u64::from(base)).wrapping_add(d);
        } else {
            over = true;
        }
        i = i.saturating_add(1);
    }
    if !any {
        return Some(Parsed { value: 0, used: 0, range: false });
    }
    let half = max.wrapping_shr(1).wrapping_add(1); // 2^31 / 2^63
    let range = match kind {
        Kind::Wrapping => false,
        Kind::Unsigned => over,
        Kind::Signed => over || (negative && v > half) || (!negative && v >= half),
    };
    let value = match (kind, range) {
        (Kind::Signed, true) => if negative { half } else { half.wrapping_sub(1) },
        // (all ones, then the '-' applied all the same: -"4294967296" is 1)
        (Kind::Unsigned, true) => if negative { 1 } else { max },
        _ => if negative { v.wrapping_neg() & max } else { v },
    };
    Some(Parsed { value, used: i, range })
}

// ---------------------------------------------------------------------------------------
// the handlers

fn r_u16(s: u32) -> impl Fn(usize) -> u16 {
    move |k| mem::r16(mem::ga(s).at(kome::num::u32_of(k), 2))
}
fn r_u8(s: u32) -> impl Fn(usize) -> u8 {
    move |k| mem::r8(mem::ga(s) + kome::num::u32_of(k))
}

fn towlower_h(c: &mut Ctx) -> u32 {
    u32::from(towlower(kome::num::lo16(c.a(0))))
}
fn towupper_h(c: &mut Ctx) -> u32 {
    u32::from(crate::nt::up(kome::num::lo16(c.a(0))))
}
fn tolower_h(c: &mut Ctx) -> u32 {
    tolower(c.a(0).cast_signed()).cast_unsigned()
}
/// RtlAnsiCharToUnicodeChar's rule: one ANSI character at `at` (two bytes when the first is a lead
/// byte of the code page; under UTF-8 the length its first byte gives), as one UTF-16 unit (the
/// first one; a space when it does not convert), and how many bytes it took
pub fn ansi_char(at: &dyn Fn(usize) -> u8) -> (u16, usize) {
    let b0 = at(0);
    let n = if crate::text::acp() == 65001 {
        match b0 {
            0..0xc0 => 1,
            0xc0..0xe0 => 2,
            0xe0..0xf0 => 3,
            0xf0..0xf8 => 4,
            0xf8..=0xff => 1,
        }
    } else if crate::text::is_lead(b0) {
        2
    } else {
        1
    };
    let b: Vec<u8> = (0..n).map(at).collect();
    (crate::text::a2w(&b).first().copied().unwrap_or(0x20), n)
}
/// RtlAnsiCharToUnicodeChar(&p): ansi_char at *p, *p moved past it
fn ansi_char_h(c: &mut Ctx) -> u32 {
    let pp = c.a(0);
    let s = mem::rp(pp);
    let (u, n) = ansi_char(&|i| mem::r8(mem::ga(s) + kome::num::u32_of(i)));
    mem::wp(pp, s.wrapping_add(kome::num::u32_of(n)));
    u32::from(u)
}
/// toupper's rule: the bytes of c (low first) as one ANSI character (ansi_char), upper-cased and
/// back; one byte -> it, two -> first << 8 | second, else c unchanged
pub fn toupper(v: u32) -> u32 {
    let b = v.to_le_bytes();
    let (u, _) = ansi_char(&|i| b.get(i).copied().unwrap_or(0));
    let a = crate::text::w2a(&[crate::nt::up(u)]);
    match a.as_slice() {
        [x] => u32::from(*x),
        [x, y] => u32::from(*x) << 8 | u32::from(*y),
        _ => v,
    }
}
fn toupper_h(c: &mut Ctx) -> u32 {
    toupper(c.a(0))
}
fn wcslwr_h(c: &mut Ctx) -> u32 {
    let s = c.a(0);
    if s == 0 {
        return 0;
    }
    case_in_place(s, 0x41, 0x20);
    s
}
fn wcsupr_h(c: &mut Ctx) -> u32 {
    let s = c.a(0);
    case_in_place(s, 0x61, 0x20u16.wrapping_neg());
    s
}
/// each unit from `from` to from + 25 moved by `by`, until a 0 unit
fn case_in_place(s: u32, from: u16, by: u16) {
    let at = r_u16(s);
    let mut k = 0usize;
    loop {
        let u = at(k);
        if u == 0 {
            return;
        }
        if u.wrapping_sub(from) <= 0x19 {
            mem::w16(mem::ga(s).at(kome::num::u32_of(k), 2), u.wrapping_add(by));
        }
        k = k.saturating_add(1);
    }
}
fn wcsicmp_h(c: &mut Ctx) -> u32 {
    wcsnicmp(&r_u16(c.a(0)), &r_u16(c.a(1)), None).cast_unsigned()
}
fn wcsnicmp_h(c: &mut Ctx) -> u32 {
    wcsnicmp(&r_u16(c.a(0)), &r_u16(c.a(1)), Some(kome::num::us(c.a(2)))).cast_unsigned()
}
fn iswascii_h(c: &mut Ctx) -> u32 {
    u32::from(kome::num::lo16(c.a(0)) < 0x80)
}
fn iswctype_h(c: &mut Ctx) -> u32 {
    u32::from(iswctype(kome::num::lo16(c.a(0)), kome::num::lo16(c.a(1))))
}

/// the argument of an _itoa: a value of `bits`, signed or not
#[derive(Clone, Copy)]
enum Arg {
    S32,
    U32,
    S64,
    U64,
}

/// _itoa(value, buf, base) .. _ui64tow: the value, buf, base; `wide` writes WCHARs
fn itoa_call(c: &mut Ctx, arg: Arg, wide: bool) -> u32 {
    let (value, rest) = match arg {
        Arg::S32 | Arg::U32 => (u64::from(c.a(0)), 1),
        // (x86: the value takes two slots)
        Arg::S64 | Arg::U64 => (c.q(0), if mem::x64() { 1 } else { 2 }),
    };
    let (buf, base) = (c.a(rest), c.a(rest.saturating_add(1)));
    let (negative, bits) = match arg {
        Arg::S32 => (base == 10 && kome::num::lo32(value).cast_signed() < 0, 32),
        Arg::U32 => (false, 32),
        Arg::S64 => (base == 10 && value.cast_signed() < 0, 64),
        Arg::U64 => (false, 64),
    };
    let Some(d) = xtoa(value, base, negative, bits) else { return buf };
    for (k, &u) in d.iter().chain(&[0]).enumerate() {
        let k = kome::num::u32_of(k);
        if wide { mem::w16(mem::ga(buf).at(k, 2), u) } else { mem::w8(mem::ga(buf) + k, kome::num::lo8(u)) }
    }
    buf
}

/// strtol and its family: (s, endptr, base) or atoi's (s); the answer in eax (edx for 64 bits)
fn strto_call<U: Unit>(c: &mut Ctx, read: fn(u32) -> Box<dyn Fn(usize) -> U>, unit: u32, atoi: Option<Kind>, bits: u32, kind: Kind) -> u32 {
    let s = c.a(0);
    let (end, base, kind) = match atoi {
        Some(k) => (0, 10, k),
        None => (c.a(1), c.a(2), kind),
    };
    if s == 0 {
        // (atoi of NULL is 0; strtol's invalid-parameter handler also answers 0)
        return ret64(c, 0, bits);
    }
    if end != 0 {
        mem::wp(end, s);
    }
    let Some(p) = strtox(&*read(s), base, kind, bits) else { return ret64(c, 0, bits) };
    if p.range {
        crate::crt::set_errno(34);
    }
    if end != 0 && p.used > 0 {
        mem::wp(end, s.wrapping_add(kome::num::u32_of(p.used).wrapping_mul(unit)));
    }
    ret64(c, p.value, bits)
}
fn ret64(c: &mut Ctx, v: u64, bits: u32) -> u32 {
    if bits == 64 { c.ret64(v) } else { kome::num::lo32(v) }
}
fn rw(s: u32) -> Box<dyn Fn(usize) -> u16> {
    Box::new(r_u16(s))
}
fn rb(s: u32) -> Box<dyn Fn(usize) -> u8> {
    Box::new(r_u8(s))
}

pub fn init() {
    let d = "ntdll";
    implement(d, "towlower", towlower_h);
    implement(d, "towupper", towupper_h);
    implement(d, "tolower", tolower_h);
    implement(d, "toupper", toupper_h);
    implement(d, "RtlAnsiCharToUnicodeChar", ansi_char_h);
    implement(d, "_wcslwr", wcslwr_h);
    implement(d, "_wcsupr", wcsupr_h);
    implement(d, "_wcsicmp", wcsicmp_h);
    implement(d, "_wcsnicmp", wcsnicmp_h);
    implement(d, "iswascii", iswascii_h);
    implement(d, "iswctype", iswctype_h);
    macro_rules! classes {
        ($($t:ident $n:literal),*) => {$(
            implement(d, concat!("is", $n), |c: &mut Ctx| u32::from(isctype(c.a(0).cast_signed(), Test::$t.mask())));
        )*};
    }
    macro_rules! wclasses {
        ($($t:ident $n:literal),*) => {$(
            implement(d, concat!("isw", $n), |c: &mut Ctx| u32::from(iswctype(kome::num::lo16(c.a(0)), Test::$t.mask())));
        )*};
    }
    classes!(Upper "upper", Lower "lower", Digit "digit", Space "space", Punct "punct", Cntrl "cntrl", Xdigit "xdigit", Alpha "alpha", Alnum "alnum", Graph "graph", Print "print");
    // (ntdll exports no iswupper, iswpunct, iswcntrl)
    wclasses!(Lower "lower", Digit "digit", Space "space", Xdigit "xdigit", Alpha "alpha", Alnum "alnum", Graph "graph", Print "print");
    const _: () = assert!(Test::ALL.len() == 11);
    implement(d, "_itoa", |c: &mut Ctx| itoa_call(c, Arg::S32, false));
    implement(d, "_ltoa", |c: &mut Ctx| itoa_call(c, Arg::S32, false));
    implement(d, "_ultoa", |c: &mut Ctx| itoa_call(c, Arg::U32, false));
    implement(d, "_i64toa", |c: &mut Ctx| itoa_call(c, Arg::S64, false));
    implement(d, "_ui64toa", |c: &mut Ctx| itoa_call(c, Arg::U64, false));
    implement(d, "_itow", |c: &mut Ctx| itoa_call(c, Arg::S32, true));
    implement(d, "_ltow", |c: &mut Ctx| itoa_call(c, Arg::S32, true));
    implement(d, "_ultow", |c: &mut Ctx| itoa_call(c, Arg::U32, true));
    implement(d, "_i64tow", |c: &mut Ctx| itoa_call(c, Arg::S64, true));
    implement(d, "_ui64tow", |c: &mut Ctx| itoa_call(c, Arg::U64, true));
    // (printf and scanf: msvcrt's engine, ntdll's edges)
    implement(d, "_snprintf", |c: &mut Ctx| crate::crt::nt_snprintf::<u8>(c, false));
    implement(d, "_vsnprintf", |c: &mut Ctx| crate::crt::nt_snprintf::<u8>(c, true));
    implement(d, "_snwprintf", |c: &mut Ctx| crate::crt::nt_snprintf::<u16>(c, false));
    implement(d, "_vsnwprintf", |c: &mut Ctx| crate::crt::nt_snprintf::<u16>(c, true));
    implement(d, "sscanf", |c: &mut Ctx| crate::crt::nt_sscanf::<u8>(c, false));
    implement(d, "sscanf_s", |c: &mut Ctx| crate::crt::nt_sscanf::<u8>(c, true));
    implement(d, "swscanf_s", |c: &mut Ctx| crate::crt::nt_sscanf::<u16>(c, true));
    implement(d, "atoi", |c: &mut Ctx| strto_call(c, rb, 1, Some(Kind::Wrapping), 32, Kind::Wrapping));
    implement(d, "atol", |c: &mut Ctx| strto_call(c, rb, 1, Some(Kind::Wrapping), 32, Kind::Wrapping));
    implement(d, "_wtoi", |c: &mut Ctx| strto_call(c, rw, 2, Some(Kind::Wrapping), 32, Kind::Wrapping));
    implement(d, "_wtol", |c: &mut Ctx| strto_call(c, rw, 2, Some(Kind::Wrapping), 32, Kind::Wrapping));
    implement(d, "_atoi64", |c: &mut Ctx| strto_call(c, rb, 1, Some(Kind::Signed), 64, Kind::Signed));
    implement(d, "_wtoi64", |c: &mut Ctx| strto_call(c, rw, 2, Some(Kind::Signed), 64, Kind::Signed));
    implement(d, "strtol", |c: &mut Ctx| strto_call(c, rb, 1, None, 32, Kind::Signed));
    implement(d, "strtoul", |c: &mut Ctx| strto_call(c, rb, 1, None, 32, Kind::Unsigned));
    implement(d, "wcstol", |c: &mut Ctx| strto_call(c, rw, 2, None, 32, Kind::Signed));
    implement(d, "wcstoul", |c: &mut Ctx| strto_call(c, rw, 2, None, 32, Kind::Unsigned));
    implement(d, "_wcstoi64", |c: &mut Ctx| strto_call(c, rw, 2, None, 64, Kind::Signed));
    implement(d, "_wcstoui64", |c: &mut Ctx| strto_call(c, rw, 2, None, 64, Kind::Unsigned));
}

#[cfg(test)]
#[path = "ntcrt_tests.rs"]
mod tests;
