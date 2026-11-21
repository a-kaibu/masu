#![allow(non_snake_case)]
//! RtlGetFullPathName_Ustr: a DOS path made full against the current directory (and the
//! drives' "=X:" variables), as ntdll does it; RtlGetFullPathName_U(Ex) on top.
//!
//! Written from the clean-room notes (project folder sake-re/ntdll/full-path.md); checked by
//! Wine's ntdll:path tests.

use crate::api::{implement, Ctx};
use crate::mem;
use crate::ntpath::{dos_device, path_type, PathType};

const BS: u16 = 0x5c;
const DOT: u16 = 0x2e;
const SPACE: u16 = 0x20;

const fn sep(u: u16) -> bool {
    u == BS || u == 0x2f
}

/// What the other drive's "=X:" variable gives (after RtlpCheckRelativeDrive put "X:\" back
/// where it named no directory)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriveDir {
    /// the variable's value
    Set(Vec<u16>),
    /// no variable
    Unset,
}

/// What the call needs from the process: the current directory (its DOS path, ending in '\')
/// and the drives' variables
pub struct Env<'a> {
    pub cur: &'a [u16],
    pub drive: &'a dyn Fn(u16) -> DriveDir,
}

/// The answer: the return value (bytes), and what the buffer of `size` bytes holds afterwards
/// (None: untouched), and the file part's index in it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Full {
    pub ret: u32,
    pub buffer: Option<Vec<u16>>,
    pub file_part: Option<usize>,
    pub kind: Option<PathType>,
}

impl Full {
    fn untouched() -> Full {
        Full { ret: 0, buffer: None, file_part: None, kind: None }
    }
    /// The text written (to its NUL)
    pub fn text(&self) -> Vec<u16> {
        self.buffer.as_deref().map_or_default(|b| b.iter().copied().take_while(|&u| u != 0).collect())
    }
}

fn at(p: &[u16], i: usize) -> u16 {
    p.get(i).copied().unwrap_or(0)
}

/// A size that does not fit in a USHORT answers 0
fn capped(n: usize) -> u32 {
    u32::try_from(n).ok().filter(|&v| v <= 0xffff).unwrap_or(0)
}

/// RtlpComputeBackupIndex: how far ".." may climb in `cur`: past "\\server\share\" for a UNC
/// path (the index after its second separator from unit 2), else 3 ("C:\")
fn backup_index(cur: &[u16]) -> usize {
    let mut h = [0u16; 4];
    for (o, &u) in h.iter_mut().zip(cur) {
        *o = u;
    }
    if path_type(h) != PathType::UncAbsolute {
        return 3;
    }
    let mut seps = 0u32;
    let mut i = 2;
    while i < cur.len() {
        if sep(at(cur, i)) {
            seps = seps.saturating_add(1);
            if seps == 2 {
                return i.saturating_add(1);
            }
        }
        i = i.saturating_add(1);
    }
    i
}

/// RtlGetFullPathName_Ustr(path, size bytes, buffer, file part)
pub fn full_path_name(p: &[u16], size: u32, env: &Env) -> Full {
    let size = usize::try_from(size.min(0xffff)).unwrap_or(0);
    let n = p.len();
    if n == 0 || at(p, 0) == 0 || p.iter().all(|&u| u == SPACE) {
        return Full::untouched();
    }
    // (a path not ending in a separator loses one the result would end in)
    let mut strip_sep = !sep(at(p, n.saturating_sub(1)));
    let cap = size / 2;
    let mut out = vec![0u16; cap];
    let dev = dos_device(p, false);
    if dev != 0 {
        let (off, len) = (usize::try_from(dev >> 17).unwrap_or(0), usize::try_from((dev & 0xffff) / 2).unwrap_or(0));
        let need = len.saturating_mul(2).saturating_add(8);
        if need < cap.saturating_mul(2) {
            let mut w: Vec<u16> = "\\\\.\\".encode_utf16().collect();
            w.extend_from_slice(p.get(off..off.saturating_add(len)).unwrap_or(&[]));
            for (o, &u) in out.iter_mut().zip(&w) {
                *o = u;
            }
            return Full { ret: capped(need), buffer: Some(out), file_part: None, kind: None };
        }
        return Full { ret: capped(need.saturating_add(2)), buffer: Some(out), file_part: None, kind: None };
    }
    let kind = {
        let mut h = [0u16; 4];
        for (o, &u) in h.iter_mut().zip(p) {
            *o = u;
        }
        path_type(h)
    };
    let done = |ret: u32, out: Vec<u16>| {
        let mut out = out;
        if usize::try_from(ret).unwrap_or(usize::MAX) >= cap.saturating_mul(2)
            && let Some(f) = out.first_mut()
        {
            *f = 0;
        }
        Full { ret, buffer: Some(out), file_part: None, kind: Some(kind) }
    };
    // the prefix (written first), its length as the size check counts it (bytes), how far ".."
    // climbs (root), and where the input's own units start
    let (prefix, prefix_bytes, root, from): (Vec<u16>, usize, usize, usize) = match kind {
        PathType::UncAbsolute => {
            let mut seps = 0u32;
            let mut i = 2;
            while i < n {
                if sep(at(p, i)) {
                    seps = seps.saturating_add(1);
                    if seps == 2 {
                        break;
                    }
                }
                i = i.saturating_add(1);
            }
            let pre = p.get(..i).unwrap_or(&[]).to_vec();
            (pre, i.saturating_mul(2), i, i)
        }
        PathType::DriveAbsolute => (Vec::new(), 0, 3, 0),
        PathType::DriveRelative => {
            let up = |u: u16| crate::nt::up(u);
            let letter = up(at(p, 0));
            if up(at(env.cur, 0)) == letter {
                (env.cur.to_vec(), env.cur.len().saturating_mul(2), 3, 2)
            } else {
                match (env.drive)(letter) {
                    DriveDir::Set(v) => {
                        if v.len().saturating_add(1) > cap {
                            // (STATUS_BUFFER_TOO_SMALL: the size it needs)
                            return done(capped(v.len().saturating_mul(2).saturating_add(2)), out);
                        }
                        let mut v = v;
                        if v.len() > 3 && v.len() < cap {
                            v.push(BS);
                        }
                        let b = v.len().saturating_mul(2);
                        (v, b, 3, 2)
                    }
                    // ("X:\", counted as 8 bytes)
                    DriveDir::Unset => (vec![letter, u16::from(b':'), BS], 8, 3, 2),
                }
            }
        }
        PathType::Rooted => {
            let r = backup_index(env.cur);
            let keep = if r == 3 { 2 } else { r };
            let pre = env.cur.get(..keep).unwrap_or(env.cur).to_vec();
            (pre, keep.saturating_mul(2), r, 0)
        }
        PathType::Relative => (env.cur.to_vec(), env.cur.len().saturating_mul(2), backup_index(env.cur), 0),
        PathType::LocalDevice | PathType::RootLocalDevice => {
            if kind == PathType::RootLocalDevice {
                strip_sep = false;
            }
            let mut pre: Vec<u16> = "\\\\.\\".encode_utf16().collect();
            if let Some(x) = pre.get_mut(2) {
                *x = at(p, 2);
            }
            (pre, 8, 4, n.min(4))
        }
    };
    let mut k = 0usize;
    for (o, &u) in out.iter_mut().zip(&prefix) {
        *o = u;
        k = k.saturating_add(1);
    }
    let need = prefix_bytes.saturating_sub(from.saturating_mul(2)).saturating_add(n.saturating_mul(2)).saturating_add(2);
    if need > size {
        // ("." alone: the prefix itself, its last '\' dropped, when that fits)
        if n == 1 && at(p, 0) == DOT {
            if prefix_bytes == 8 {
                if size <= 8 {
                    return done(10, out);
                }
            } else if size < prefix_bytes {
                return done(capped(prefix_bytes), out);
            } else if k > 0 && at(&out, k.saturating_sub(1)) == BS {
                k = k.saturating_sub(1);
                if let Some(x) = out.get_mut(k) {
                    *x = 0;
                }
            }
        } else {
            return done(capped(need), out);
        }
    }
    for x in out.iter_mut().take(k) {
        if *x == 0x2f {
            *x = BS;
        }
    }
    let put = |out: &mut Vec<u16>, k: &mut usize, u: u16| {
        if let Some(x) = out.get_mut(*k) {
            *x = u;
        }
        *k = k.saturating_add(1);
    };
    let mut i = from;
    while i < n {
        let c = at(p, i);
        if sep(c) {
            if k == 0 || at(&out, k.saturating_sub(1)) != BS {
                put(&mut out, &mut k, BS);
            }
            i = i.saturating_add(1);
            continue;
        }
        if c == DOT {
            let next = i.saturating_add(1);
            if next == n {
                // (a last "." is dropped)
                i = next;
                continue;
            }
            let d = at(p, next);
            if sep(d) {
                // ("./" is skipped)
                i = next.saturating_add(1);
                continue;
            }
            let after = next.saturating_add(1);
            if d == DOT && (after == n || sep(at(p, after))) {
                // "..": back to the previous '\', no further than the root
                loop {
                    if k < root {
                        k = k.saturating_add(1);
                        break;
                    }
                    let ch = at(&out, k);
                    if let Some(x) = out.get_mut(k) {
                        *x = 0;
                    }
                    if ch == BS {
                        // (then on to the '\' before the component)
                        loop {
                            if k < root {
                                k = k.saturating_add(1);
                                break;
                            }
                            let ch2 = at(&out, k);
                            if let Some(x) = out.get_mut(k) {
                                *x = 0;
                            }
                            if ch2 == BS {
                                if k < root {
                                    k = k.saturating_add(1);
                                }
                                break;
                            }
                            k = k.saturating_sub(1);
                        }
                        break;
                    }
                    k = k.saturating_sub(1);
                }
                i = after;
                continue;
            }
        }
        // a component: up to the next separator; "x." before one loses its dot ("x.." keeps both)
        while i < n && !sep(at(p, i)) {
            put(&mut out, &mut k, at(p, i));
            i = i.saturating_add(1);
        }
        if i < n && k >= 2 && at(&out, k.saturating_sub(1)) == DOT && at(&out, k.saturating_sub(2)) != DOT {
            k = k.saturating_sub(1);
            if let Some(x) = out.get_mut(k) {
                *x = 0;
            }
        }
    }
    if strip_sep && k > root && at(&out, k.saturating_sub(1)) == BS {
        k = k.saturating_sub(1);
    }
    // trailing spaces and dots go, the NUL after what is left
    loop {
        if k < cap
            && let Some(x) = out.get_mut(k)
        {
            *x = 0;
        }
        if k == 0 {
            break;
        }
        k = k.saturating_sub(1);
        let c = at(&out, k);
        if c != SPACE && c != DOT {
            k = k.saturating_add(1);
            break;
        }
    }
    let mut j = k;
    while j > 0 && at(&out, j.saturating_sub(1)) != BS {
        j = j.saturating_sub(1);
    }
    let file_part = (j >= root && j < k).then_some(j);
    let ret = capped(k.saturating_mul(2));
    let mut f = done(ret, out);
    f.file_part = file_part;
    f
}

/// The process's: the current directory, and "=X:" with RtlpCheckRelativeDrive's reset (a
/// value naming no directory becomes "X:\")
pub fn process_env_full(p: &[u16], size: u32) -> Full {
    let cur = crate::curdir::dos_path();
    full_path_name(p, size, &Env { cur: &cur, drive: &process_drive })
}

/// A drive's "=X:" variable, put back to "X:\" when it names no directory
fn process_drive(letter: u16) -> DriveDir {
    let name = format!("={}:", char::from_u32(u32::from(letter)).unwrap_or('C'));
    match crate::k32::env_get(&name) {
        Some(v) => {
            let dir = crate::files::resolve(&v).and_then(|h| kura::fs::metadata(h).ok()).is_some_and(|m| m.is_dir());
            if dir {
                DriveDir::Set(v.encode_utf16().collect())
            } else {
                let root = format!("{}:\\", char::from_u32(u32::from(letter)).unwrap_or('C'));
                crate::k32::env_set(&name, Some(&root));
                DriveDir::Set(root.encode_utf16().collect())
            }
        }
        None => DriveDir::Unset,
    }
}

/// RtlDosPathNameToNtPathName_U's answer: the NT name, the file part's index in it, and where
/// the name relative to the current directory starts in it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NtPath {
    pub nt: Vec<u16>,
    pub file_part: Option<usize>,
    pub relative: Option<usize>,
}

const STATUS_OBJECT_NAME_INVALID: u32 = 0xc000_0033;
const STATUS_NAME_TOO_LONG: u32 = 0xc000_0106;
/// The static buffer the full path is first made in (bytes)
const STATIC_BYTES: u32 = 0x208;

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// RtlpCheckDeviceName: a device named after a directory is a name only when that directory
/// exists (RtlDoesFileExists_UEx on "<dir>.")
fn device_dir_missing(p: &[u16], exists: &dyn Fn(&[u16]) -> bool) -> bool {
    let off = usize::try_from(dos_device(p, false) >> 17).unwrap_or(0);
    if off == 0 {
        return false;
    }
    let mut d: Vec<u16> = p.get(..off).unwrap_or(&[]).to_vec();
    d.push(DOT);
    !exists(&d)
}

/// The last component's index (after the last '\'), None when the name ends there
fn last_part(nt: &[u16]) -> Option<usize> {
    let k = nt.iter().rposition(|&u| u == BS)?.saturating_add(1);
    (k < nt.len()).then_some(k)
}

/// RtlpDosPathNameToRelativeNtPathName (flags 0): "\??\" and "\\?\" names pass with the
/// prefix made "\??\"; others are made full (in 0x208 bytes: longer is STATUS_NAME_TOO_LONG,
/// the process not being long-path aware), a UNC path under "\??\UNC\", a "\\.\" one under
/// "\??\"; a relative input names its rest past the current directory when that leads it
pub fn dos_to_nt(p: &[u16], env: &Env, exists: &dyn Fn(&[u16]) -> bool) -> Result<NtPath, u32> {
    let head: Vec<u16> = p.iter().copied().take(4).collect();
    if p.len() > 4 && (head == units("\\??\\") || head == units("\\\\?\\")) {
        let mut nt = units("\\??\\");
        nt.extend_from_slice(p.get(4..).unwrap_or(&[]));
        return Ok(NtPath { file_part: last_part(&nt), nt, relative: None });
    }
    if device_dir_missing(p, exists) {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    let f = full_path_name(p, STATIC_BYTES, env);
    if f.ret == 0 {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    if f.ret > STATIC_BYTES {
        return Err(STATUS_NAME_TOO_LONG);
    }
    let full = f.text();
    let mut h = [0u16; 4];
    for (o, &u) in h.iter_mut().zip(&full) {
        *o = u;
    }
    let (mut nt, skip) = match path_type(h) {
        PathType::UncAbsolute => (units("\\??\\UNC\\"), 2usize),
        PathType::LocalDevice => (units("\\??\\"), 4),
        PathType::DriveAbsolute | PathType::DriveRelative | PathType::Rooted | PathType::Relative | PathType::RootLocalDevice => (units("\\??\\"), 0),
    };
    let lead = nt.len().saturating_sub(skip);
    nt.extend_from_slice(full.get(skip..).unwrap_or(&[]));
    if nt.len().saturating_add(1).saturating_mul(2) > 0xfffe {
        return Err(STATUS_NAME_TOO_LONG);
    }
    let file_part = f.file_part.map(|j| j.saturating_add(lead));
    let mut input = [0u16; 4];
    for (o, &u) in input.iter_mut().zip(p) {
        *o = u;
    }
    let up = crate::nt::up;
    let prefixed = full.len() >= env.cur.len() && full.iter().zip(env.cur).all(|(&a, &b)| up(a) == up(b));
    let relative = (path_type(input) == PathType::Relative && prefixed).then(|| {
        let r = lead.saturating_add(env.cur.len());
        if at(&nt, r) == BS { r.saturating_add(1) } else { r }
    });
    Ok(NtPath { nt, file_part, relative })
}

/// The process's dos_to_nt: the device's directory looked up in the guest's files
fn process_dos_to_nt(p: &[u16]) -> Result<NtPath, u32> {
    let exists = |d: &[u16]| {
        let s = String::from_utf16_lossy(d);
        crate::files::resolve(&crate::files::full_path(&s)).is_some_and(|h| kura::fs::metadata(h).is_ok())
    };
    let cur = crate::curdir::dos_path();
    dos_to_nt(p, &Env { cur: &cur, drive: &process_drive }, &exists)
}

/// GetTempPathW's answer (units) and the buffer of `count` units afterwards: the first of
/// TMP, TEMP, USERPROFILE whose value fits RtlQueryEnvironmentVariable_U's 0x20a bytes (else
/// the Windows directory), made full in the caller's buffer; a '\' added when it fits, the
/// answer then counting it (and its NUL: 2 more when it did not fit)
pub fn temp_path(count: u32, var: &dyn Fn(&str) -> Option<Vec<u16>>, env: &Env) -> (u32, Option<Vec<u16>>) {
    let size = count.wrapping_mul(2);
    let v = ["TMP", "TEMP", "USERPROFILE"].into_iter().filter_map(var).find(|v| v.len() <= 260).unwrap_or_else(|| units("C:\\windows"));
    let add_slash = v.last() != Some(&BS);
    let f = full_path_name(&v, size, env);
    let mut buffer = f.buffer.clone();
    if f.ret == 0 || f.ret >= size {
        let r = if add_slash { f.ret.saturating_add(2) } else { f.ret };
        return (r / 2, buffer);
    }
    let n = usize::try_from(f.ret / 2).unwrap_or(0);
    if f.text().last() == Some(&BS) {
        return (f.ret / 2, buffer);
    }
    if f.ret.saturating_add(2) < size {
        if let Some(b) = buffer.as_mut() {
            if let Some(x) = b.get_mut(n) {
                *x = BS;
            }
            if let Some(x) = b.get_mut(n.saturating_add(1)) {
                *x = 0;
            }
        }
        return (f.ret.saturating_add(2) / 2, buffer);
    }
    (f.ret.saturating_add(4) / 2, buffer)
}

/// The process's temp_path
pub fn process_temp_path(count: u32) -> (u32, Option<Vec<u16>>) {
    let cur = crate::curdir::dos_path();
    let var = |n: &str| crate::k32::env_get(n).map(|v| units(&v));
    temp_path(count, &var, &Env { cur: &cur, drive: &process_drive })
}

/// Writes a buffer image to the guest
pub fn put_units(buf: u32, b: &[u16]) {
    for (i, &u) in b.iter().enumerate() {
        mem::w16(mem::ga(buf).at(kome::num::u32_of(i), 2), u);
    }
}

/// Writes a Full to the guest: the buffer image, the file part pointer
fn write_out(f: &Full, buf: u32, file_part: u32) {
    if file_part != 0 {
        let fp = f.file_part.map_or(0, |j| (mem::ga(buf).at(kome::num::u32_of(j), 2)).get());
        mem::wp(file_part, fp);
    }
    if let Some(b) = &f.buffer {
        for (i, &u) in b.iter().enumerate() {
            mem::w16(mem::ga(buf).at(kome::num::u32_of(i), 2), u);
        }
    }
}

/// RtlGetFullPathName_UEx(name, size, buffer, file_part, *bytes): STATUS_SUCCESS with the
/// length, STATUS_OBJECT_NAME_INVALID (0) for 0
fn RtlGetFullPathName_UEx(c: &mut Ctx) -> u32 {
    let (name, size, buf, fp, outp) = (c.a(0), c.a(1), c.a(2), c.a(3), c.a(4));
    if outp != 0 {
        mem::w32(outp, 0);
    }
    let w = mem::wstr(name);
    if w.len() > 0x7ffe {
        return 0xc000_0106; // STATUS_NAME_TOO_LONG (RtlInitUnicodeStringEx)
    }
    if fp != 0 {
        mem::wp(fp, 0);
    }
    let f = process_env_full(&w, size);
    write_out(&f, buf, fp);
    if f.ret == 0 {
        return 0xc000_0033;
    }
    if outp != 0 {
        mem::w32(outp, f.ret);
    }
    0
}

/// RtlGetFullPathName_U(name, size, buffer, file_part): the length in bytes, 0 for none
fn RtlGetFullPathName_U(c: &mut Ctx) -> u32 {
    let (name, size, buf, fp) = (c.a(0), c.a(1), c.a(2), c.a(3));
    let w = mem::wstr(name);
    if w.len() > 0x7ffe {
        return 0;
    }
    if fp != 0 {
        mem::wp(fp, 0);
    }
    let f = process_env_full(&w, size);
    write_out(&f, buf, fp);
    f.ret
}

/// Writes an NtPath to the guest: nt gets a process-heap buffer (Length, room for the NUL),
/// *file_part a pointer into it (or NULL), the RTL_RELATIVE_NAME_U zeroed and then its
/// RelativeName (no directory handle: flags 0)
fn nt_out(n: &NtPath, us: u32, file_part: u32, rel: u32) {
    let len = kome::num::u32_of(n.nt.len()).saturating_mul(2);
    let buf = mem::heap_alloc(len.saturating_add(2), false);
    for (i, &u) in n.nt.iter().chain(&[0]).enumerate() {
        mem::w16(mem::ga(buf).at(kome::num::u32_of(i), 2), u);
    }
    crate::k32::put_counted(us, len, len.saturating_add(2), buf);
    let ptr_at = |j: usize| mem::ga(buf).at(kome::num::u32_of(j), 2).get();
    if file_part != 0 {
        mem::wp(file_part, n.file_part.map_or(0, ptr_at));
    }
    if rel != 0 {
        for i in 0..4 {
            mem::wp(mem::ga(rel).at(i, mem::ps()), 0);
        }
        if let Some(r) = n.relative {
            let rl = kome::num::u32_of(n.nt.len().saturating_sub(r)).saturating_mul(2);
            crate::k32::put_counted(rel, rl, rl, ptr_at(r));
        }
    }
}

/// Both callers: RtlInitUnicodeStringEx on the name first (too long: STATUS_NAME_TOO_LONG)
fn dos_to_nt_call(c: &mut Ctx) -> u32 {
    let (dos, us, fp, rel) = (c.a(0), c.a(1), c.a(2), c.a(3));
    let w = mem::wstr(dos);
    if w.len() > 0x7ffe {
        return STATUS_NAME_TOO_LONG;
    }
    if fp != 0 {
        mem::wp(fp, 0);
    }
    match process_dos_to_nt(&w) {
        Ok(n) => {
            nt_out(&n, us, fp, rel);
            0
        }
        Err(st) => st,
    }
}

/// RtlDosPathNameToNtPathName_U_WithStatus(dos, &nt, &file_part, &relative)
fn RtlDosPathNameToNtPathName_U_WithStatus(c: &mut Ctx) -> u32 {
    dos_to_nt_call(c)
}

/// RtlDosPathNameToNtPathName_U(dos, &nt, &file_part, &relative): TRUE on success
fn RtlDosPathNameToNtPathName_U(c: &mut Ctx) -> u32 {
    u32::from(dos_to_nt_call(c) == 0)
}

pub fn init() {
    implement("ntdll", "RtlDosPathNameToNtPathName_U", RtlDosPathNameToNtPathName_U);
    implement("ntdll", "RtlDosPathNameToNtPathName_U_WithStatus", RtlDosPathNameToNtPathName_U_WithStatus);
    implement("ntdll", "RtlGetFullPathName_U", RtlGetFullPathName_U);
    implement("ntdll", "RtlGetFullPathName_UEx", RtlGetFullPathName_UEx);
}

#[cfg(test)]
#[path = "fullpath_tests.rs"]
mod tests;
