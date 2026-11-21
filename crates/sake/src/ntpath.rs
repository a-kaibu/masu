#![allow(non_snake_case)]
//! ntdll's DOS path rules: RtlDetermineDosPathNameType_U, RtlIsDosDeviceName_U,
//! RtlIsNameLegalDOS8Dot3.
//!
//! Written from the clean-room notes (project folder sake-re/ntdll/dos-paths.md); checked by
//! Wine's ntdll:path tests.

use crate::api::{implement, Ctx};
use crate::mem;

/// RTL_PATH_TYPE
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathType {
    /// \\server\share, //x, \\.x, \\?x
    UncAbsolute,
    /// C:\x, C:/x
    DriveAbsolute,
    /// C:x
    DriveRelative,
    /// \x, /x
    Rooted,
    /// x, the empty path
    Relative,
    /// \\.\x, \\?\x
    LocalDevice,
    /// \\. and \\? alone
    RootLocalDevice,
}

impl PathType {
    pub const fn code(self) -> u32 {
        match self {
            PathType::UncAbsolute => 1,
            PathType::DriveAbsolute => 2,
            PathType::DriveRelative => 3,
            PathType::Rooted => 4,
            PathType::Relative => 5,
            PathType::LocalDevice => 6,
            PathType::RootLocalDevice => 7,
        }
    }
}

const fn sep(u: u16) -> bool {
    u == 0x5c || u == 0x2f
}

/// The type of the NUL-terminated path whose first four units (0 past its end) are `p`: only
/// they are looked at, and no further than the first 0 decides
pub fn path_type(p: [u16; 4]) -> PathType {
    let [a, b, c, d] = p;
    if sep(a) {
        if !sep(b) {
            return PathType::Rooted;
        }
        if c != u16::from(b'.') && c != u16::from(b'?') {
            return PathType::UncAbsolute;
        }
        return if sep(d) {
            PathType::LocalDevice
        } else if d == 0 {
            PathType::RootLocalDevice
        } else {
            PathType::UncAbsolute
        };
    }
    if a != 0 && b == u16::from(b':') {
        return if sep(c) { PathType::DriveAbsolute } else { PathType::DriveRelative };
    }
    PathType::Relative
}

/// The first four units of the guest string at `s`, 0 past its NUL (no unit past it is read)
pub fn head4(s: u32) -> [u16; 4] {
    let mut out = [0u16; 4];
    for (i, o) in out.iter_mut().enumerate() {
        let u = mem::r16(mem::ga(s).at(kome::num::u32_of(i), 2));
        *o = u;
        if u == 0 {
            break;
        }
    }
    out
}

/// What a name's last component told on the way (RtlpIsDosDeviceName_Ustr's flags)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Seen {
    /// a directory or drive came before the name
    path: bool,
    /// something other than spaces after its first '.' or ':'
    extension: bool,
    /// the name is NUL
    nul: bool,
}

fn is(u: u16, c: u8) -> bool {
    u == u16::from(c)
}

/// RtlpIsDosDeviceName_Ustr on the path's units: 0, or the device name's (offset, length) in
/// bytes as MAKELONG(length, offset). `legacy`: the LegacyDosDevicePaths setting (off by
/// default on Windows 11: then a name with a directory or an extension is a device only when it
/// is NUL without one).
pub fn dos_device(p: &[u16], legacy: bool) -> u32 {
    let at = |i: usize| p.get(i).copied().unwrap_or(0);
    // (UNC and \\.\ / \\?\ paths name no DOS device; "\\." and "\\?" alone are looked at)
    if p.len() >= 2 && sep(at(0)) && sep(at(1)) {
        if p.len() == 3 && (is(at(2), b'.') || is(at(2), b'?')) {
            // (scanned below)
        } else {
            return 0;
        }
    }
    let mut n = p.len();
    let mut stripped = 0usize;
    if n == 0 {
        return 0;
    }
    if is(at(n.saturating_sub(1)), b':') {
        n = n.saturating_sub(1);
        stripped = stripped.saturating_add(1);
        if n == 0 {
            return 0;
        }
    }
    while n > 0 && (is(at(n.saturating_sub(1)), b'.') || is(at(n.saturating_sub(1)), b' ')) {
        n = n.saturating_sub(1);
        stripped = stripped.saturating_add(1);
    }
    let mut seen = Seen::default();
    let mut start = 0usize;
    if n > 0 {
        // the last '\', '/' or a ':' second
        let cut = (0..n).rev().find(|&i| sep(at(i)) || (is(at(i), b':') && i == 1));
        if let Some(k) = cut {
            seen.path = true;
            start = k.saturating_add(1);
            if start >= p.len() {
                return 0;
            }
            n = p.len().saturating_sub(start).saturating_sub(stripped);
        }
    }
    let lead = |u: u16| matches!(u8::try_from(u | 0x20), Ok(b'l' | b'c' | b'p' | b'a' | b'n'));
    if n > 0 && !lead(at(start)) {
        return 0;
    }
    let name = p.get(start..start.saturating_add(n)).unwrap_or(&[]);
    // the name ends at its first '.' or ':' (an extension when something but spaces follows),
    // spaces before that dropped
    let mut end = name.iter().position(|&u| is(u, b'.') || is(u, b':')).unwrap_or(name.len());
    if end < name.len() && name.get(end.saturating_add(1)..).unwrap_or(&[]).iter().any(|&u| !is(u, b' ')) {
        seen.extension = true;
    }
    while end > 0 && name.get(end.saturating_sub(1)).is_some_and(|&u| is(u, b' ')) {
        end = end.saturating_sub(1);
    }
    let base = name.get(..end).unwrap_or(&[]);
    let eq = |w: &str| base.len() == w.len() && base.iter().zip(w.bytes()).all(|(&u, b)| u8::try_from(u).is_ok_and(|x| x.eq_ignore_ascii_case(&b)));
    let device = match base.len() {
        3 => {
            if eq("NUL") {
                seen.nul = true;
                true
            } else {
                eq("PRN") || eq("AUX") || eq("CON")
            }
        }
        4 => {
            let d = base.get(3).copied().unwrap_or(0);
            let three = base.get(..3).unwrap_or(&[]);
            let eq3 = |w: &str| three.iter().zip(w.bytes()).all(|(&u, b)| u8::try_from(u).is_ok_and(|x| x.eq_ignore_ascii_case(&b)));
            crate::ntcrt::iswctype(d, crate::ntcrt::Test::Digit.mask()) != 0 && !is(d, b'0') && (eq3("LPT") || eq3("COM"))
        }
        6 => eq("CONIN$"),
        7 => eq("CONOUT$"),
        _ => false,
    };
    if !device {
        return 0;
    }
    let plain = !seen.path && !seen.extension && !seen.nul;
    if !plain && !(seen.nul && !seen.extension) && !legacy {
        return 0;
    }
    let len = kome::num::lo16(base.len().saturating_mul(2));
    let off = kome::num::lo16(start.saturating_mul(2));
    u32::from(off) << 16 | u32::from(len)
}

/// RtlFatIllegalTable: the ASCII bytes a FAT name may not hold (controls and "*+,/:;<=>?[\]|)
const FAT_ILLEGAL: [u32; 4] = [0xffff_ffff, 0xfc00_9c04, 0x3800_0000, 0x1000_0000];

fn fat_illegal(b: u8) -> bool {
    let word = FAT_ILLEGAL.get(usize::from(b >> 5)).copied().unwrap_or(0);
    word >> (b & 31) & 1 != 0
}

/// RtlIsNameLegalDOS8Dot3's rule on the name's upper-case OEM bytes: None when it is no 8.3
/// name, else whether it holds a space. "." and ".." are names; a base of 1..8 bytes, then
/// at most one '.' (not first, not after a space) with at most 3 bytes after it; no FAT-illegal
/// ASCII byte; not ending in ' ' or '.'. A DBCS lead byte (`lead`) takes the next byte along,
/// and may not start at base index 7 or more, nor be last.
pub fn legal_8dot3(s: &[u8], lead: Option<&dyn Fn(u8) -> bool>) -> Option<bool> {
    if s == b"." || s == b".." {
        return Some(false);
    }
    let n = s.len();
    let (mut spaces, mut dot) = (false, false);
    let mut i = 0usize;
    let mut last = 0u8;
    while i < n {
        let b = s.get(i).copied().unwrap_or(0);
        last = b;
        if lead.is_some_and(|f| f(b)) {
            if (!dot && i >= 7) || i == n.saturating_sub(1) {
                return None;
            }
            i = i.saturating_add(2);
            continue;
        }
        if b < 0x80 && fat_illegal(b) {
            return None;
        }
        if b == b' ' {
            spaces = true;
        } else if b == b'.' {
            let rest = n.saturating_sub(i).saturating_sub(1);
            if dot || i == 0 || s.get(i.saturating_sub(1)) == Some(&b' ') || rest > 3 {
                return None;
            }
            dot = true;
        }
        if i >= 8 && !dot {
            return None;
        }
        i = i.saturating_add(1);
    }
    if n > 0 && (last == b' ' || last == b'.') {
        return None;
    }
    Some(spaces)
}

/// RtlIsNameLegalDOS8Dot3(const UNICODE_STRING *name, OEM_STRING *oem, BOOLEAN *spaces): more
/// than 12 units is FALSE; the name upper-cased into `oem` (or 12 bytes of its own) as
/// RtlUpcaseUnicodeStringToCountedOemString does (too long for it: FALSE), then the rule;
/// *spaces only on TRUE
fn RtlIsNameLegalDOS8Dot3(c: &mut Ctx) -> u32 {
    let (name, oem, spaces) = (c.a(0), c.a(1), c.a(2));
    let len = mem::r16(name);
    if len > 0x18 {
        return 0;
    }
    let buf = mem::rp(mem::ga(name) + mem::ps());
    let up: Vec<u16> = (0..u32::from(len / 2)).map(|i| crate::nt::up(mem::r16(mem::ga(buf).at(i, 2)))).collect();
    let bytes = crate::text::w2mb(1, &up);
    let room = if oem != 0 { usize::from(mem::r16(mem::ga(oem) + 2)) } else { 12 };
    if bytes.len() > room {
        return 0;
    }
    if oem != 0 {
        let ob = mem::rp(mem::ga(oem) + mem::ps());
        for (i, &b) in bytes.iter().enumerate() {
            mem::w8(mem::ga(ob) + kome::num::u32_of(i), b);
        }
        mem::w16(oem, kome::num::lo16(bytes.len()));
    }
    let lead = |b: u8| crate::text::is_lead_cp(1, b);
    let dbcs = crate::text::is_dbcs();
    match legal_8dot3(&bytes, if dbcs { Some(&lead) } else { None }) {
        Some(sp) => {
            if spaces != 0 {
                mem::w8(spaces, u8::from(sp));
            }
            1
        }
        None => 0,
    }
}

/// RtlIsDosDeviceName_U(PCWSTR path): RtlInitUnicodeStringEx (too long: 0), then the rule
fn RtlIsDosDeviceName_U(c: &mut Ctx) -> u32 {
    let w = mem::wstr(c.a(0));
    if w.len() > 0x7ffe {
        return 0;
    }
    dos_device(&w, false)
}

/// RtlDetermineDosPathNameType_U(PCWSTR path)
fn RtlDetermineDosPathNameType_U(c: &mut Ctx) -> u32 {
    path_type(head4(c.a(0))).code()
}

pub fn init() {
    implement("ntdll", "RtlDetermineDosPathNameType_U", RtlDetermineDosPathNameType_U);
    implement("ntdll", "RtlIsDosDeviceName_U", RtlIsDosDeviceName_U);
    implement("ntdll", "RtlIsNameLegalDOS8Dot3", RtlIsNameLegalDOS8Dot3);
}

#[cfg(test)]
#[path = "ntpath_tests.rs"]
mod tests;
