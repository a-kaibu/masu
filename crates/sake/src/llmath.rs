#![allow(non_snake_case)]
//! ntdll's 64-bit integer arithmetic: the x86 compiler helpers (_allmul, _alldiv, _allrem,
//! _aulldiv, _aullrem, _alldvrm, _aulldvrm, _allshl, _allshr, _aullshr) and
//! RtlExtendedMagicDivide, RtlLargeIntegerToChar, RtlInt64ToUnicodeString.
//!
//! Written from the clean-room notes (project folder sake-re/ntdll/large-int.md); checked by
//! Wine's ntdll:large_int tests.

use crate::api::{implement, Ctx};
use crate::mem;
use koji::{EAX, EBX, ECX, EDX, ESP};

/// A shift helper's kind: EDX:EAX by CL
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    /// _allshl
    Left,
    /// _allshr (arithmetic)
    RightSigned,
    /// _aullshr
    RightUnsigned,
}

/// EDX:EAX shifted by CL, and the CL left behind: 64 and more give 0 (or the sign); 32..63 shift
/// the one word that matters by CL & 31 and leave CL that way
pub fn shift(k: Shift, v: u64, cl: u8) -> (u64, u8) {
    let sign = if v >> 63 != 0 { u64::MAX } else { 0 };
    if cl >= 64 {
        return (if k == Shift::RightSigned { sign } else { 0 }, cl);
    }
    let r = match k {
        Shift::Left => v << cl,
        Shift::RightSigned => (v.cast_signed() >> cl).cast_unsigned(),
        Shift::RightUnsigned => v >> cl,
    };
    (r, if cl >= 32 { cl & 31 } else { cl })
}

/// A division helper's kind
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Div {
    /// _alldiv / _allrem / _alldvrm: on the magnitudes, the quotient negative when the signs
    /// differ, the remainder when the dividend is negative (i64::MIN / -1 wraps to i64::MIN)
    Signed,
    /// _aulldiv / _aullrem / _aulldvrm
    Unsigned,
}

/// (quotient, remainder); None: a zero divisor (the DIV faults)
pub fn divide(k: Div, a: u64, b: u64) -> Option<(u64, u64)> {
    match k {
        Div::Unsigned => Some((a.checked_div(b)?, a.checked_rem(b)?)),
        Div::Signed => {
            let (x, y) = (a.cast_signed(), b.cast_signed());
            let (q, r) = (x.unsigned_abs().checked_div(y.unsigned_abs())?, x.unsigned_abs().checked_rem(y.unsigned_abs())?);
            let q = if (x < 0) != (y < 0) { q.wrapping_neg() } else { q };
            let r = if x < 0 { r.wrapping_neg() } else { r };
            Some((q, r))
        }
    }
}

/// RtlExtendedMagicDivide(dividend, magic, shift): the high 64 bits of |dividend| * magic,
/// shifted right, negated back for a negative dividend. x86 shifts in steps (64 and more: 0);
/// x64 does one SHR, which takes the count mod 64.
pub fn magic_divide(a: i64, magic: u64, shift: u8, x64: bool) -> i64 {
    let hi = kome::num::lo64(u128::from(a.unsigned_abs()).wrapping_mul(u128::from(magic)) >> 64);
    let q = if x64 { hi >> (shift & 63) } else { hi.checked_shr(u32::from(shift)).unwrap_or(0) };
    if a < 0 { q.cast_signed().wrapping_neg() } else { q.cast_signed() }
}

/// Returns from a stdcall helper as its `ret 16` does: EDX:EAX = v (and EBX:ECX = rem for the
/// dvrm forms)
fn ret_pop16(c: &mut Ctx, v: u64, rem: Option<u64>) -> u32 {
    let ret = mem::r32(c.esp);
    c.cpu.regs[EAX] = kome::num::lo32(v);
    c.cpu.regs[EDX] = kome::num::lo32(v >> 32);
    if let Some(r) = rem {
        c.cpu.regs[ECX] = kome::num::lo32(r);
        c.cpu.regs[EBX] = kome::num::lo32(r >> 32);
    }
    c.cpu.eip = ret;
    c.cpu.regs[ESP] = (mem::ga(c.esp) + 4 + 16).get();
    c.jumped = true;
    0
}

/// What a division helper answers
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Want {
    Quotient,
    Remainder,
    Both,
}

fn div_call(c: &mut Ctx, k: Div, w: Want) -> u32 {
    let (a, b) = (c.q(0), c.q(2));
    let Some((q, r)) = divide(k, a, b) else {
        // (#DE in the helper: STATUS_INTEGER_DIVIDE_BY_ZERO, raised as if from the call's return)
        let ret = mem::r32(c.esp);
        c.cpu.eip = ret;
        c.cpu.regs[ESP] = (mem::ga(c.esp) + 4 + 16).get();
        crate::seh::raise(c.cpu, 0xc000_0094, 0, c.trap, &[]);
        c.jumped = true;
        return 0;
    };
    match w {
        Want::Quotient => ret_pop16(c, q, None),
        Want::Remainder => ret_pop16(c, r, None),
        Want::Both => ret_pop16(c, q, Some(r)),
    }
}

fn shift_call(c: &mut Ctx, k: Shift) -> u32 {
    let v = u64::from(c.cpu.regs[EAX]) | u64::from(c.cpu.regs[EDX]) << 32;
    let ecx = c.cpu.regs[ECX];
    let (r, cl) = shift(k, v, kome::num::lo8(ecx));
    c.cpu.regs[ECX] = (ecx & !0xff) | u32::from(cl);
    c.ret64(r)
}

/// RtlExtendedMagicDivide(LARGE_INTEGER dividend, LARGE_INTEGER magic, CCHAR shift)
fn RtlExtendedMagicDivide(c: &mut Ctx) -> u32 {
    let x64 = mem::x64();
    let (a, m, s) = if x64 { (c.a64(0), c.a64(1), c.a64(2)) } else { (c.q(0), c.q(2), u64::from(c.a(4))) };
    let r = magic_divide(a.cast_signed(), m, kome::num::lo8(s), x64);
    c.ret64(r.cast_unsigned())
}

/// RtlLargeIntegerToChar(const ULONGLONG *value, ULONG base, LONG length, PCHAR str):
/// RtlIntegerToChar's rules for 64 bits; the copy runs under SEH (a NULL str: the
/// STATUS_ACCESS_VIOLATION it raised)
fn RtlLargeIntegerToChar(c: &mut Ctx) -> u32 {
    let value = mem::r64(c.a(0));
    crate::ntstr::to_char_out(value, c.a(1), c.a(2).cast_signed(), c.a(3))
}

/// RtlInt64ToUnicodeString(ULONGLONG value, ULONG base, UNICODE_STRING *str):
/// RtlLargeIntegerToChar into 65 bytes, then RtlAnsiStringToUnicodeString(str, digits, FALSE)
fn RtlInt64ToUnicodeString(c: &mut Ctx) -> u32 {
    let (value, base, us) = if mem::x64() { (c.a64(0), c.a(1), c.a(2)) } else { (c.q(0), c.a(2), c.a(3)) };
    crate::ntstr::to_unicode_out(value, base, 65, us)
}

pub fn init() {
    let d = "ntdll";
    implement(d, "RtlExtendedMagicDivide", RtlExtendedMagicDivide);
    implement(d, "RtlLargeIntegerToChar", RtlLargeIntegerToChar);
    implement(d, "RtlInt64ToUnicodeString", RtlInt64ToUnicodeString);
    // (x86 only: x64's ntdll does not export them)
    implement(d, "_allmul", |c: &mut Ctx| {
        let v = c.q(0).wrapping_mul(c.q(2));
        ret_pop16(c, v, None)
    });
    implement(d, "_alldiv", |c: &mut Ctx| div_call(c, Div::Signed, Want::Quotient));
    implement(d, "_allrem", |c: &mut Ctx| div_call(c, Div::Signed, Want::Remainder));
    implement(d, "_alldvrm", |c: &mut Ctx| div_call(c, Div::Signed, Want::Both));
    implement(d, "_aulldiv", |c: &mut Ctx| div_call(c, Div::Unsigned, Want::Quotient));
    implement(d, "_aullrem", |c: &mut Ctx| div_call(c, Div::Unsigned, Want::Remainder));
    implement(d, "_aulldvrm", |c: &mut Ctx| div_call(c, Div::Unsigned, Want::Both));
    implement(d, "_allshl", |c: &mut Ctx| shift_call(c, Shift::Left));
    implement(d, "_allshr", |c: &mut Ctx| shift_call(c, Shift::RightSigned));
    implement(d, "_aullshr", |c: &mut Ctx| shift_call(c, Shift::RightUnsigned));
}

#[cfg(test)]
#[path = "llmath_tests.rs"]
mod tests;
