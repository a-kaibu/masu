#![allow(non_snake_case)]
//! ntdll's DOS path rules: RtlDetermineDosPathNameType_U, RtlIsDosDeviceName_U.
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
}

#[cfg(test)]
#[path = "ntpath_tests.rs"]
mod tests;
