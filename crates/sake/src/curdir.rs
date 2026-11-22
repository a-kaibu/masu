#![allow(non_snake_case)]
//! The current directory as the PEB keeps it: RTL_USER_PROCESS_PARAMETERS.CurrentDirectory
//! (a CURDIR: the DOS path ending in '\' and a handle to the directory), and
//! RtlSetCurrentDirectory_U, whose rules SetCurrentDirectory goes through.
//!
//! Written from the clean-room notes (project folder sake-re/ntdll/curdir.md); checked by
//! Wine's ntdll:path tests. files::cwd stays the host-side truth; every files::set_cwd makes a
//! new reference here (a handle and the path).

use crate::api::{implement, Ctx};
use crate::mem;
use crate::ntstr::Status;
use kome::locks::Lock as _;
use std::sync::Mutex;

/// The buffer's size (MAX_PATH units + NUL, as Windows sizes the process parameters' one)
const MAX_BYTES: u16 = 0x208;
/// What the directory handle opens with: FILE_TRAVERSE | SYNCHRONIZE, share read | write
/// (not delete: renaming the directory or a parent fails, ERROR_SHARING_VIOLATION)
const ACCESS: u32 = 0x0010_0020;
const SHARE: u32 = 3;

/// The current reference: its handle and the length (bytes) of the path it was made for. The
/// path's units live in the guest buffer the PEB points at (the comparison reads them there, as
/// Windows reads the reference's buffer, which the PEB shares).
struct Cur {
    /// RTL_USER_PROCESS_PARAMETERS (0: not made yet)
    pp: u32,
    buf: u32,
    handle: u32,
    len: u16,
}

static CUR: Mutex<Cur> = Mutex::new(Cur { pp: 0, buf: 0, handle: 0, len: 0 });

/// CURDIR's place in the process parameters: (DosPath, Handle)
fn offsets() -> (u32, u32) {
    if mem::x64() { (0x38, 0x48) } else { (0x24, 0x2c) }
}

/// At process start, once the process parameters are there: the buffer, and the first reference
pub fn install(pp: u32) {
    let buf = mem::heap_alloc(u32::from(MAX_BYTES), true);
    {
        let mut c = CUR.locked();
        c.pp = pp;
        c.buf = buf;
    }
    let (dos, _) = offsets();
    mem::w16(mem::ga(pp) + dos + 2, MAX_BYTES);
    mem::wp(mem::ga(pp) + dos + mem::ps(), buf);
    publish(&crate::files::cwd());
}

/// A new reference for the full path `dir` (files::set_cwd): a handle opened on it, the path
/// with its '\' into the buffer, the old handle closed
pub fn publish(dir: &str) {
    if CUR.locked().pp == 0 {
        return;
    }
    let mut w: Vec<u16> = dir.encode_utf16().collect();
    if w.last() != Some(&u16::from(b'\\')) {
        w.push(u16::from(b'\\'));
    }
    w.truncate(usize::from(MAX_BYTES / 2).saturating_sub(1));
    // (opened before the reference is taken: opening makes the name full, which reads the
    // current reference, dos_path)
    let handle = match crate::files::open_shared(dir, ACCESS, 3, Some(SHARE)) {
        Ok((f, _)) if f.dir => crate::obj::add(crate::obj::Obj::File(std::sync::Arc::new(f))),
        _ => 0,
    };
    let mut c = CUR.locked();
    for (i, &u) in w.iter().enumerate() {
        mem::w16(mem::ga(c.buf).at(kome::num::u32_of(i), 2), u);
    }
    let len = kome::num::lo16(mem::wbytes(w.len()));
    mem::w16(mem::ga(c.buf) + u32::from(len), 0);
    let (dos, h) = offsets();
    mem::w16(mem::ga(c.pp) + dos, len);
    mem::wp(mem::ga(c.pp) + h, handle);
    let old = std::mem::replace(&mut c.handle, handle);
    c.len = len;
    drop(c);
    if old != 0 {
        crate::obj::close(old);
    }
}

/// The current directory's DOS path as the reference holds it (ending in '\'); before the
/// process parameters exist, files::cwd's
pub fn dos_path() -> Vec<u16> {
    let c = CUR.locked();
    if c.pp == 0 {
        drop(c);
        let mut w: Vec<u16> = crate::files::cwd().encode_utf16().collect();
        if w.last() != Some(&0x5c) {
            w.push(0x5c);
        }
        return w;
    }
    (0..u32::from(c.len / 2)).map(|i| mem::r16(mem::ga(c.buf).at(i, 2))).collect()
}

/// RtlpCheckForSameCurdir: whether `input` (a DOS path's units) names the current reference
/// as it is written: past 3 units ("C:\"), the path without its '\' (so "C:\dir" is the same as
/// "C:\dir\" made, while "C:\dir\" itself, ".", "\\?\C:\dir" are not); else exactly. Case
/// matters; `units` are the buffer's as they are now (a program may have written there).
pub fn same(units: &[u16], len: u16, input: &[u16]) -> bool {
    let n = usize::from(len / 2);
    let cmp = if len > 6 { n.saturating_sub(1) } else { n };
    input.len() == cmp && units.get(..cmp) == Some(input)
}

pub fn same_as_current(input: &[u16]) -> bool {
    let c = CUR.locked();
    if c.pp == 0 {
        return false;
    }
    let units: Vec<u16> = (0..u32::from(c.len / 2)).map(|i| mem::r16(mem::ga(c.buf).at(i, 2))).collect();
    same(&units, c.len, input)
}

/// The directory `full` (a full DOS path) as RtlpCreateNewDirectoryReference opens it: Ok, or
/// the NTSTATUS NtOpenFile(FILE_DIRECTORY_FILE) answers
pub fn check_dir(full: &str) -> Result<(), u32> {
    match crate::files::resolve(full).map(|h| kura::fs::metadata(h).map(|m| m.is_dir())) {
        Some(Ok(true)) => Ok(()),
        Some(_) => Err(0xc000_0103), // STATUS_NOT_A_DIRECTORY
        None => {
            // (the parent missing: OBJECT_PATH_NOT_FOUND; just the last name: OBJECT_NAME_NOT_FOUND)
            let parent = full.trim_end_matches('\\').rsplit_once('\\').map(|(p, _)| format!("{p}\\"));
            if parent.as_deref().and_then(crate::files::resolve).is_some() { Err(0xc000_0034) } else { Err(0xc000_003a) }
        }
    }
}

/// RtlSetCurrentDirectory_U(const UNICODE_STRING *path): the same reference: nothing done,
/// STATUS_SUCCESS; else its full path opened as a directory and made the current one
fn RtlSetCurrentDirectory_U(c: &mut Ctx) -> u32 {
    let us = c.a(0);
    let n = u32::from(mem::r16(us)) / 2;
    let w = mem::wstrn(mem::rp(mem::ga(us) + mem::ps()), n);
    if same_as_current(&w) {
        return Status::Success.code();
    }
    let full = crate::files::full_path(&String::from_utf16_lossy(&w));
    if let Err(st) = check_dir(&full) {
        return st;
    }
    crate::files::set_cwd(&full);
    Status::Success.code()
}

pub fn init() {
    implement("ntdll", "RtlSetCurrentDirectory_U", RtlSetCurrentDirectory_U);
}

#[cfg(test)]
#[path = "curdir_tests.rs"]
mod tests;
