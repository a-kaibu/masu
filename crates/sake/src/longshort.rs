//! GetLongPathName / GetShortPathName: each component that may be the other kind of name is
//! looked up (FindFirstFileExW) and replaced by the found long / short name, from the
//! clean-room notes (project folder sake-re/kernelbase/long-short.md); checked by Wine's
//! kernel32:path.

use crate::ntpath::{path_type, PathType};

const BS: u16 = 0x5c;
const SL: u16 = 0x2f;
const DOT: u16 = 0x2e;

const fn sep(u: u16) -> bool {
    u == BS || u == SL
}

/// kernelbase's IllegalMask: bytes no short name holds (controls, space and "*+,/:;<=>?[\]|)
const ILLEGAL: [u32; 4] = [0xffff_ffff, 0xfc00_9c05, 0x3800_0000, 0x1000_0000];

/// IsLongName_U: a name that cannot be a short one (empty, more than 12 units, a first '.', a
/// second '.', more than 3 after the '.', a 9th unit before one)
pub fn is_long_name(s: &[u16]) -> bool {
    let n = s.len();
    if n == 0 || n > 12 || s.first() == Some(&DOT) {
        return true;
    }
    let mut dot = false;
    for (i, &u) in s.iter().enumerate() {
        if u == DOT {
            if dot || n.saturating_sub(i).saturating_sub(1) > 3 {
                return true;
            }
            dot = true;
        }
        if i >= 8 && !dot {
            return true;
        }
    }
    false
}

/// IsShortName_U on the name's ANSI bytes: "." and ".." are; else at most 12 units, a DBCS
/// pair not starting at 7 before a '.' nor cut, ASCII without IllegalMask bytes, one '.' with
/// at most 3 after it, no 9th byte before it
pub fn is_short_name(s: &[u16], bytes: &[u8], lead: &dyn Fn(u8) -> bool) -> bool {
    let n = s.len();
    if n > 12 {
        return false;
    }
    if n == 0 || s == [DOT] || s == [DOT, DOT] {
        return true;
    }
    if s.first() == Some(&DOT) {
        return false;
    }
    let mut dot = false;
    let mut rem = n;
    let mut i = 0usize;
    while i < bytes.len() {
        rem = rem.saturating_sub(1);
        let b = bytes.get(i).copied().unwrap_or(0);
        if lead(b) {
            if (!dot && i >= 7) || i == bytes.len().saturating_sub(1) {
                return false;
            }
            i = i.saturating_add(2);
            rem = rem.saturating_sub(1);
            continue;
        }
        if b > 0x7f || ILLEGAL.get(usize::from(b >> 5)).is_some_and(|w| w >> (b & 31) & 1 != 0) {
            return false;
        }
        if b == b'.' {
            if dot || rem > 3 {
                return false;
            }
            dot = true;
        }
        if i >= 8 && !dot {
            return false;
        }
        i = i.saturating_add(1);
    }
    true
}

/// SkipPathTypeIndicator_U: where the components start (None: "\\." / "\\?" alone)
pub fn skip_root(p: &[u16]) -> Option<usize> {
    let mut h = [0u16; 4];
    for (o, &u) in h.iter_mut().zip(p) {
        *o = u;
    }
    match path_type(h) {
        PathType::DriveAbsolute => Some(3),
        PathType::DriveRelative => Some(2),
        PathType::Rooted => Some(1),
        PathType::Relative => Some(0),
        PathType::RootLocalDevice => None,
        PathType::UncAbsolute | PathType::LocalDevice => {
            let unc: Vec<u16> = "?\\UNC".encode_utf16().collect();
            let head = p.get(2..7).unwrap_or(&[]);
            let up = |u: u16| crate::nt::up(u);
            let mut i = if head.len() == 5 && head.iter().zip(&unc).all(|(&a, &b)| up(a) == up(b)) { 8 } else { 2 };
            let mut seps = 2u32;
            while seps > 0 {
                let Some(&u) = p.get(i) else { break };
                if u == 0 {
                    break;
                }
                if sep(u) {
                    seps = seps.saturating_sub(1);
                }
                i = i.saturating_add(1);
            }
            Some(i.min(p.len()))
        }
    }
}

/// FindLFNorSFN_U from `from`: the next component (start, end) to look up: for a short path
/// one that is no short name, for a long path one that is no long name
fn next(p: &[u16], from: usize, short: bool, is_short: &dyn Fn(&[u16]) -> bool) -> Option<(usize, usize)> {
    let mut i = from;
    loop {
        while p.get(i).is_some_and(|&u| sep(u)) {
            i = i.saturating_add(1);
        }
        if p.get(i).is_none_or(|&u| u == 0) {
            return None;
        }
        let start = i;
        let mut e = start.saturating_add(1);
        while p.get(e).is_some_and(|&u| u != 0 && !sep(u)) {
            e = e.saturating_add(1);
        }
        let comp = p.get(start..e).unwrap_or(&[]);
        let hit = if short { !is_short(comp) } else { !is_long_name(comp) };
        if hit {
            return Some((start, e));
        }
        if p.get(e).is_none_or(|&u| u == 0) {
            return None;
        }
        i = e.saturating_add(1);
    }
}

/// What FindFirstFileExW found for a path: its long name and its short one (empty: none)
pub struct Found {
    pub long: Vec<u16>,
    pub short: Vec<u16>,
}

/// The converted path, or Err(()) when a lookup found nothing (FindFirstFile's error stands).
/// `exists`: GetFileAttributesW's answer for the whole path (a missing one is an error first).
pub fn convert(p: &[u16], short: bool, is_short: &dyn Fn(&[u16]) -> bool, find: &dyn Fn(&[u16]) -> Option<Found>) -> Result<Vec<u16>, ()> {
    let Some(root) = skip_root(p).filter(|&r| p.get(r).is_some_and(|&u| u != 0)) else {
        return Ok(p.to_vec());
    };
    let Some(mut cur) = next(p, root, short, is_short) else {
        return Ok(p.to_vec());
    };
    let mut out = Vec::new();
    let mut seg = 0usize;
    loop {
        let (s, e) = cur;
        out.extend_from_slice(p.get(seg..s).unwrap_or(&[]));
        let f = find(p.get(..e).unwrap_or(&[])).ok_or(())?;
        let name = if short { f.short } else { f.long };
        if name.is_empty() {
            out.extend_from_slice(p.get(s..e).unwrap_or(&[]));
        } else {
            out.extend_from_slice(&name);
        }
        seg = e;
        if p.get(e).is_none_or(|&u| u == 0) {
            break;
        }
        match next(p, e, short, is_short) {
            Some(c) => cur = c,
            None => break,
        }
    }
    out.extend_from_slice(p.get(seg..).unwrap_or(&[]));
    Ok(out)
}

#[cfg(test)]
#[path = "longshort_tests.rs"]
mod tests;
