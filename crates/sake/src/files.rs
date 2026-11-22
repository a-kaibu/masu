//! Files: Windows paths over the host's (node's) or the page's (kura vfs) file system.
//!
//! Z:\ is the host root (as in Wine), C:\ is SAKE_C (default /tmp/sake-c). Names are
//! matched without case when the exact one is missing.

use kome::locks::Lock as _;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

pub struct File {
    /// the host file: through host(), which keeps a shared file pointer (`shared`)
    f: Mutex<kura::fs::File>,
    pub path: String,
    pub dir: bool,
    /// opened for writing (WriteFile fails with ERROR_ACCESS_DENIED otherwise)
    pub write: bool,
    /// a directory's listing being read by NtQueryDirectoryFile, with the mask it took
    pub scan: Mutex<Option<(Vec<u16>, Find)>>,
    /// its entry in OPEN (sharing checks): held for its Drop, which removes it with the handle
    _share: Option<ShareEntry>,
    /// FILE_FLAG_DELETE_ON_CLOSE: the file goes with the last handle (or the process)
    delete: std::sync::atomic::AtomicBool,
    /// its crate::kern KFile once another process holds it too (0: this process's alone): the
    /// file pointer is the kernel's then
    shared: std::sync::atomic::AtomicU32,
}

/// The host file of a File, at the file pointer: a shared file's is the kernel's, loaded when
/// taken and stored back when dropped (so a child's write moves the parent's tell)
pub struct Host<'a> {
    g: std::sync::MutexGuard<'a, kura::fs::File>,
    shared: u32,
}
impl std::ops::Deref for Host<'_> {
    type Target = kura::fs::File;
    fn deref(&self) -> &kura::fs::File {
        &self.g
    }
}
impl std::ops::DerefMut for Host<'_> {
    fn deref_mut(&mut self) -> &mut kura::fs::File {
        &mut self.g
    }
}
impl Drop for Host<'_> {
    fn drop(&mut self) {
        if self.shared != 0 && let Ok(p) = self.g.stream_position() {
            let id = self.shared;
            crate::kern::with(|k| k.set_file_pos(id, p));
        }
    }
}

impl Drop for File {
    fn drop(&mut self) {
        // (closing the handle releases its byte-range locks)
        let me = self.lock_owner();
        let mut locks = LOCKS.locked();
        if !locks.is_empty() {
            locks.release(me);
        }
        drop(locks);
        // (its sharing entry goes with `share`)
        let shared = self.shared.load(std::sync::atomic::Ordering::Acquire);
        if shared != 0 {
            let me = crate::pid();
            crate::kern::with(|k| k.release_file(shared, me));
        }
        if self.delete.load(std::sync::atomic::Ordering::Relaxed) {
            DELETE_ON_CLOSE.locked().retain(|p| *p != self.path);
            let _ = kura::fs::remove_file(&self.path);
        }
    }
}

/// Where ReadFile / WriteFile with an OVERLAPPED start (NtReadFile / NtWriteFile's ByteOffset):
/// Offset/OffsetHigh from the file's start. Two values are special: all ones, for a write, is
/// the end of the file (FILE_WRITE_TO_END_OF_FILE: WriteFile's documentation, "functionally
/// equivalent to ... FILE_APPEND_DATA"), and 0xfffffffe:0xffffffff
/// (FILE_USE_FILE_POINTER_POSITION) is the file pointer (None). SQLite (CisLugI 体験版's
/// Description database) reads and writes its pages this way and never moves the pointer.
pub fn io_start(lo: u32, hi: u32, write: bool) -> Option<SeekFrom> {
    match (hi, lo) {
        (0xffff_ffff, 0xffff_ffff) if write => Some(SeekFrom::End(0)),
        (0xffff_ffff, 0xffff_fffe) => None,
        _ => Some(SeekFrom::Start(lo as u64 | (hi as u64) << 32)),
    }
}

/// The byte-range locks of all files (LockFileEx; kura::filelock).
pub static LOCKS: Mutex<kura::filelock::Locks> = Mutex::new(kura::filelock::Locks::new());

impl File {
    /// Who holds a lock: the open file (one per CreateFile; DuplicateHandle shares it).
    pub fn lock_owner(&self) -> u64 {
        self as *const File as usize as u64
    }
    /// The file a lock is on (paths match without case).
    pub fn lock_key(&self) -> String {
        self.path.to_lowercase()
    }
    /// FILE_FLAG_DELETE_ON_CLOSE: deleted when its last handle closes, or when the process
    /// ends with it open (as Windows closes a process's handles)
    pub fn delete_on_close(&self) {
        if self.dir || self.delete.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let mut l = DELETE_ON_CLOSE.locked();
        static AT_EXIT: std::sync::Once = std::sync::Once::new();
        AT_EXIT.call_once(|| unsafe {
            libc::atexit(delete_at_exit);
        });
        l.push(self.path.clone());
    }
}

/// The files open with FILE_FLAG_DELETE_ON_CLOSE.
static DELETE_ON_CLOSE: Mutex<Vec<String>> = Mutex::new(Vec::new());

extern "C" fn delete_at_exit() {
    let l = std::mem::take(&mut *DELETE_ON_CLOSE.lock().unwrap_or_else(|e| e.into_inner()));
    for p in l {
        let _ = kura::fs::remove_file(&p);
    }
}

/// CreateFile's sharing: (entry id, host path in lower case, access bits, share mode) of each
/// open handle that asked for a share mode (the C runtime's and sake's own opens share all)
static OPEN: Mutex<Vec<(u64, String, u32, u32)>> = Mutex::new(Vec::new());
const R: u32 = 1;
const W: u32 = 2;
const D: u32 = 4;

/// The read/write/delete bits of a CreateFile access mask.
fn access_bits(access: u32) -> u32 {
    let mut b = 0;
    if access & (0x8000_0000 | 0x1000_0000 | 1) != 0 {
        b |= R;
    }
    if access & (0x4000_0000 | 0x1000_0000 | 2 | 4) != 0 {
        b |= W;
    }
    if access & (0x0001_0000 | 0x1000_0000) != 0 {
        b |= D;
    }
    b
}

/// ERROR_SHARING_VIOLATION when an open handle of `host` does not share what this one wants or
/// this one does not share what that one has.
fn share_conflict(host: &str, want: u32, share: u32) -> bool {
    conflicts(&OPEN.locked(), &host.to_lowercase(), want, share)
}
fn conflicts(open: &[(u64, String, u32, u32)], key: &str, want: u32, share: u32) -> bool {
    open.iter().any(|o| o.1 == key && (want & !o.3 & 7 != 0 || o.2 & !share & 7 != 0))
}

/// A handle's entry in OPEN, checked against the others and put there under one lock (review
/// 6852be1f: the check and the entry were taken under two, so two opens that exclude each other
/// both got in between them). It leaves OPEN when dropped: with its File, or when the open fails
/// after the check.
pub struct ShareEntry(u64);

impl ShareEntry {
    /// The entry of an open of `host` for `access` sharing `share`, Err(ERROR_SHARING_VIOLATION)
    /// when an open handle excludes it or it excludes one.
    fn take(host: &str, access: u32, share: u32) -> Result<ShareEntry, u32> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let key = host.to_lowercase();
        let want = access_bits(access);
        let mut open = OPEN.locked();
        if conflicts(&open, &key, want, share) {
            return Err(32);
        }
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        open.push((id, key, want, share));
        Ok(ShareEntry(id))
    }
}

impl Drop for ShareEntry {
    fn drop(&mut self) {
        OPEN.locked().retain(|o| o.0 != self.0);
    }
}

/// Whether some handle keeps `host` from being deleted or renamed (opened without FILE_SHARE_DELETE).
pub fn delete_blocked(host: &str) -> bool {
    share_conflict(host, D, 7)
}

/// Not found: ERROR_PATH_NOT_FOUND when the directory is missing too, else ERROR_FILE_NOT_FOUND.
/// What sake tells apart in a host I/O error. std's ErrorKind is non_exhaustive, so io_err is
/// the one match over it that needs a catch-all; callers match IoErr without one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IoErr {
    NotFound,
    Exists,
    /// the operation is not there (a file system without hard links)
    Unsupported,
    TooManyLinks,
    TimedOut,
    Other,
}
pub fn io_err(e: &std::io::Error) -> IoErr {
    use std::io::ErrorKind as K;
    match e.kind() {
        K::NotFound => IoErr::NotFound,
        K::AlreadyExists => IoErr::Exists,
        K::Unsupported => IoErr::Unsupported,
        K::TooManyLinks => IoErr::TooManyLinks,
        K::TimedOut => IoErr::TimedOut,
        // (non_exhaustive: every other kind, today's and those std adds)
        _ => IoErr::Other,
    }
}

pub fn missing(w: &str) -> u32 {
    // (no name at all, as NULL: ERROR_PATH_NOT_FOUND)
    if w.is_empty() {
        return ERROR_PATH_NOT_FOUND;
    }
    if invalid_name(w) {
        return ERROR_INVALID_NAME;
    }
    // (a \\?\ name's folders are taken as they are: "C:\windows.\x" is under no "windows")
    let literal = if w.starts_with("\\\\?\\") { "\\\\?\\" } else { "" };
    let full = full_path(w);
    let full = full.trim_prefix("\\\\?\\");
    match full.trim_end_matches('\\').rsplit_once('\\') {
        // (a folder that is a file: "C:\a.txt\b")
        Some((d, _)) if resolve(&format!("{literal}{d}\\")).is_none_or(|h| !is_dir(&h)) => ERROR_PATH_NOT_FOUND,
        _ => ERROR_FILE_NOT_FOUND,
    }
}

/// FILE_ATTRIBUTE_READONLY and the like set by SetFileAttributes or asked of CreateFile (the
/// host files keep none): host path in lower case -> attributes. FILE_ATTRIBUTE_ARCHIVE is kept
/// beside read-only / hidden / system: a file made hidden is 0x22, one set hidden 0x2.
pub static ATTRS: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());
pub fn set_attrs(host: &str, a: u32) {
    let key = host.to_lowercase();
    let mut t = ATTRS.locked();
    t.retain(|x| x.0 != key);
    if a & 0x7 != 0 {
        t.push((key, a & 0x27));
    }
}
pub fn extra_attrs(host: &str) -> u32 {
    let key = host.to_lowercase();
    ATTRS.locked().iter().find(|x| x.0 == key).map_or(0, |x| x.1)
}

/// SetFileTime's last write times (the host files keep the host's own): host path in lower case
/// -> (the FILETIME set, the host's modification time then). A write since (the host's time
/// moved) shows the host's time again.
static TIMES: Mutex<Vec<(String, u64, u64)>> = Mutex::new(Vec::new());
pub fn set_write_time(host: &str, ft: u64) {
    let key = host.to_lowercase();
    let now = kura::fs::metadata(host).ok().and_then(|m| m.modified().ok()).map_or(0, filetime);
    let mut t = TIMES.locked();
    t.retain(|x| x.0 != key);
    t.push((key, ft, now));
}
/// The last write time of host file `host` (metadata `m`) as a FILETIME.
pub fn write_time(host: &str, m: &kura::fs::Metadata) -> u64 {
    let real = m.modified().map_or(0, filetime);
    let key = host.to_lowercase();
    TIMES.locked().iter().find(|x| x.0 == key && x.2 == real).map_or(real, |x| x.1)
}
/// A file's index (BY_HANDLE_FILE_INFORMATION): the host's inode, kept by renames, else a hash
/// of its host path.
pub fn file_index(host: &str, m: &kura::fs::Metadata) -> u64 {
    if m.ino() != 0 {
        return m.ino();
    }
    kome::golden::Fnv::of(host.to_lowercase().as_bytes())
}

#[derive(Default)]
pub struct Find {
    pub entries: Vec<(String, kura::fs::Metadata)>,
    pub at: usize,
    /// the entries' 8.3 short names (shortname.rs), by long name
    pub shorts: std::collections::HashMap<String, String>,
}

static CWD: Mutex<String> = Mutex::new(String::new());

pub fn cwd() -> String {
    let c = CWD.locked();
    if c.is_empty() {
        "C:\\".into()
    } else {
        c.clone()
    }
}

pub fn set_cwd(d: &str) {
    let mut s = d.to_string();
    if !s.ends_with('\\') {
        s.push('\\');
    }
    *CWD.locked() = s.clone();
    // (the PEB's CURDIR: a new reference, its handle and path)
    crate::curdir::publish(&s);
}

fn c_root() -> String {
    static C: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let c = C.get_or_init(|| drive_root(std::env::var("SAKE_C").unwrap_or_else(|_| "/tmp/sake-c".into()))).clone();
    // (the guest's names keep their short ones from here on: shortname's tables)
    crate::shortname::watch();
    c
}

/// The host directory of a drive, made when missing: C:\ is always there on Windows (stat,
/// GetFileAttributes and FindFirstFile of "C:\" find a directory; winref crt_io, #72)
fn drive_root(h: String) -> String {
    let _ = kura::fs::create_dir_all(&h);
    h
}

/// Host path of a host file as a Windows path.
pub fn host_to_win(h: &str) -> String {
    let c = c_root();
    if let Some(rest) = h.strip_prefix(&c) {
        return format!("C:{}", rest.replace('/', "\\"));
    }
    format!("Z:{}", h.replace('/', "\\"))
}

/// Whether host path `h` is a directory (kura's file system, not std's).
pub fn is_dir<P: AsRef<std::path::Path>>(h: P) -> bool {
    kura::fs::metadata(h).is_ok_and(|m| m.is_dir())
}

/// Absolute Windows path of `p`, the one every file operation opens. A DOS path is made full
/// by RtlGetFullPathName's rule, GetFullPathName's own (fullpath.rs: the current directory, the
/// drives' "=X:" variables, "." and "..", trailing dots and spaces), its drive letter made
/// upper case; a \\?\ or \??\ name is literal (its own mode: Literal below). The rule was
/// once this file's own, and "D:x" with "=D:" set opened D:\x while GetFullPathName said
/// D:\games\x (review b5dacf4 2.2).
pub fn full_path(p: &str) -> String {
    match FullMode::of(p) {
        FullMode::Win32 => win32_full(p),
        FullMode::Literal => full_path_trim(p, true),
    }
}

/// How a path is made full (full_path)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FullMode {
    /// RtlGetFullPathName_U's rule
    Win32,
    /// a \\?\ or \??\ name: no normalization but the prefix's
    Literal,
}

impl FullMode {
    fn of(p: &str) -> Self {
        let p = p.replace('/', "\\");
        if p.starts_with("\\\\?\\") || p.starts_with("\\??\\") { FullMode::Literal } else { FullMode::Win32 }
    }
}

/// GetFullPathNameW's answer for `p` with the process's current directory and drives (the
/// drive letter upper case, as the file system's names have it)
fn win32_full(p: &str) -> String {
    let w: Vec<u16> = p.encode_utf16().collect();
    let mut t: Vec<u16> = crate::fullpath::process_env_full(&w, 0xfffe).text();
    if t.get(1) == Some(&u16::from(b':')) && let Some(d) = t.first_mut() && let Ok(b) = u8::try_from(*d) {
        *d = u16::from(b.to_ascii_uppercase());
    }
    String::from_utf16_lossy(&t)
}
/// full_path; `trim` false for a \\?\ path, whose last component keeps trailing dots and
/// spaces (CreateFile of \\?\C:\d\a. makes "a."): such a path stays \\?\-prefixed so that
/// taking its full path again keeps them
/// "C:x" (or "C:"): relative to the current directory `cwd` when that is on the same drive,
/// else to the drive's root (no per-drive directories here)
fn drive_relative(p: &str, cwd: &str) -> String {
    if cwd.get(..2).is_some_and(|d| d.eq_ignore_ascii_case(&p[..2])) {
        format!("{}\\{}", cwd.trim_end_matches('\\'), &p[2..])
    } else {
        format!("{}\\{}", &p[..2], &p[2..])
    }
}
fn full_path_trim(p: &str, trim: bool) -> String {
    let p = p.replace('/', "\\");
    let abs = if p.len() >= 2 && p.as_bytes()[1] == b':' && p.is_char_boundary(2) {
        if p.len() == 2 || p.as_bytes()[2] != b'\\' {
            drive_relative(&p, &cwd())
        } else {
            p
        }
    } else if let Some(rest) = p.strip_prefix("\\\\?\\").or_else(|| p.strip_prefix("\\??\\")) {
        // (\\?\UNC\server\share is the UNC path \\server\share)
        if let Some(unc) = rest.get(..4).filter(|h| h.eq_ignore_ascii_case("UNC\\")).and_then(|_| rest.get(4..)) {
            return full_path_trim(&format!("\\\\{unc}"), false);
        }
        // (an NT or long path; an empty or driveless one is taken from the current directory)
        let rest = if rest.len() >= 2 && rest.as_bytes()[1] == b':' { rest.to_string() } else { format!("{}{}", cwd(), rest) };
        return full_path_trim(&rest, false);
    } else if let Some(rest) = p.strip_prefix("\\\\") {
        // UNC: \\server\share stays the root (".." does not go above it)
        let mut it = rest.splitn(3, '\\');
        let (srv, share, tail) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().unwrap_or(""));
        let mut parts: Vec<&str> = Vec::new();
        for c in tail.split('\\') {
            match c {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                c => parts.push(c),
            }
        }
        let mut out = format!("\\\\{srv}\\{share}");
        for c in parts {
            out.push('\\');
            out.push_str(c);
        }
        return out;
    } else if p.starts_with('\\') {
        format!("{}{}", &cwd()[..2], p)
    } else {
        format!("{}{}", cwd(), p)
    };
    let mut parts: Vec<&str> = Vec::new();
    let (drive, rest) = abs.split_at(2);
    let n = rest.split('\\').count();
    for (i, c) in rest.split('\\').enumerate() {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            // (a folder's name that ends in one period loses it, as Windows' normalization does:
            // "C:\RTP\RPGVX.\RGSS202J.dll" is C:\RTP\RPGVX\RGSS202J.dll, what RGSS2's Game.exe
            // asks for; "x..\" and "x...\" are names of their own; #232)
            c if trim && i + 1 < n && c.ends_with('.') && !c.ends_with("..") => parts.push(&c[..c.len() - 1]),
            c => parts.push(c),
        }
    }
    // (the last component loses its trailing dots and spaces: "trail. ." is "trail")
    let mut kept = false;
    if let Some(l) = parts.last_mut() {
        let t = l.trim_end_matches(['.', ' ']);
        if !t.is_empty() && t.len() != l.len() {
            if trim {
                *l = t;
            } else {
                kept = true;
            }
        }
    }
    let trailing = abs.ends_with('\\') && !parts.is_empty();
    let mut out = format!("{}{}\\{}", if kept { "\\\\?\\" } else { "" }, drive.to_ascii_uppercase(), parts.join("\\"));
    if trailing {
        out.push('\\');
    }
    out
}

/// Host path for a Windows path, without checking that it exists.
pub fn to_host(w: &str) -> String {
    let w = full_path(w);
    let w = admin_share(&w).unwrap_or(w);
    let w = w.trim_prefix("\\\\?\\");
    let w = &wow64_system(w);
    // (no drive left: "\\?\" alone names nothing on the host)
    let Some((drive, rest)) = w.get(..2).zip(w.get(2..)) else { return String::new() };
    let rest = rest.replace('\\', "/");
    match drive {
        "Z:" => if rest.is_empty() { "/".into() } else { rest },
        "C:" => format!("{}{}", c_root(), rest),
        d => format!("{}/{}{}", c_root(), &d[..1].to_ascii_lowercase(), rest),
    }
}

/// The drive path of a UNC path to this machine's administrative share of a drive:
/// \\SAKE\C$\x (also by "localhost" or "127.0.0.1", Windows' own names for itself) is
/// C:\x. `w` is a full path; None for any other
pub fn admin_share(w: &str) -> Option<String> {
    let rest = w.strip_prefix("\\\\")?;
    let (srv, rest) = rest.split_once('\\').unwrap_or((rest, ""));
    let (share, tail) = rest.split_once('\\').unwrap_or((rest, ""));
    let me = [crate::lsa::COMPUTER, "localhost", "127.0.0.1"].iter().any(|n| n.eq_ignore_ascii_case(srv));
    let letter = share.strip_suffix('$').filter(|l| l.len() == 1 && l.bytes().all(|b| b.is_ascii_alphabetic()))?;
    me.then(|| format!("{}:\\{tail}", letter.to_ascii_uppercase()))
}

/// 64-bit Windows' other names for its system folder, as sake has one for both machines (its
/// DLLs are its own, and a third-party one is taken by its machine, pe.rs): SysWOW64, where a
/// 32-bit process's system32 is redirected (so system32 and SysWOW64 are the same files to it,
/// with redirection on or off), and Sysnative, a 32-bit process's name for the 64-bit system32.
/// `w` is a full path; any other is given back as it is.
pub fn wow64_system(w: &str) -> std::borrow::Cow<'_, str> {
    const WIN: &str = "c:\\windows\\";
    let Some(rest) = w.get(..WIN.len()).filter(|h| h.eq_ignore_ascii_case(WIN)).map(|_| &w[WIN.len()..]) else {
        return w.into();
    };
    for (alias, ok) in [("syswow64", true), ("sysnative", !crate::mem::x64())] {
        let n = alias.len();
        let hit = rest.get(..n).is_some_and(|a| a.eq_ignore_ascii_case(alias)) && matches!(rest.as_bytes().get(n), None | Some(b'\\'));
        if ok && hit {
            return format!("{}system32{}", &w[..WIN.len()], &rest[n..]).into();
        }
    }
    w.into()
}

fn exists(h: &str) -> bool {
    kura::fs::symlink_metadata(h).is_ok()
}

/// The host path of an existing file or directory `w` (case-insensitive match); a system DLL
/// sake implements gets its file in system32 then (sysdll.rs)
pub fn resolve(w: &str) -> Option<String> {
    resolve_found(w).or_else(|| crate::sysdll::make(w))
}

fn resolve_found(w: &str) -> Option<String> {
    if w.is_empty() {
        return None;
    }
    let h = to_host(w);
    if h.is_empty() {
        return None;
    }
    if exists(&h) {
        return Some(h);
    }
    let mut at = String::from("/");
    for comp in h.split('/').filter(|c| !c.is_empty()) {
        let cand = if at == "/" { format!("/{comp}") } else { format!("{at}/{comp}") };
        if exists(&cand) {
            at = cand;
            continue;
        }
        // (a short name, PROGRA~1: the entry that has it)
        let found = kura::fs::name_nocase(&at, comp).or_else(|| crate::shortname::long_of(&at, comp))?;
        at = if at == "/" { format!("/{found}") } else { format!("{at}/{found}") };
    }
    Some(at)
}

/// The host path for creating `w`: its directory resolved, the name as given.
pub fn resolve_new(w: &str) -> String {
    let full = full_path(w);
    // (a trailing backslash, "save\\" as CreateDirectory takes it: the name is the last
    // component before it, else the parent's spelling was not looked up and a parent spelled in
    // another case than the host's was not found; YU-RIS makes its save folder this way after
    // lowering the case of the game's path)
    let t = full.trim_end_matches('\\');
    let full = if t.len() > 2 { t.to_string() } else { full };
    match full.rsplit_once('\\') {
        Some((d, n)) if !n.is_empty() => match resolve(&format!("{d}\\")) {
            Some(hd) => format!("{}/{n}", hd.trim_end_matches('/')),
            None => to_host(&full),
        },
        _ => to_host(&full),
    }
}

/// (C:\windows\temp goes into a C:\Windows made before, not beside it on a case-sensitive
/// (C:\\windows\\temp goes into a C:\\Windows made before, not beside it on a case-sensitive
/// host). The host path.
pub fn make_dirs(w: &str) -> String {
    let full = full_path(w);
    let full = full.trim_end_matches('\\');
    let mut at = String::new();
    let mut host = to_host(full);
    for (i, part) in full.split('\\').enumerate() {
        if i > 0 {
            at.push('\\');
        }
        at.push_str(part);
        if i == 0 {
            continue;
        }
        host = match resolve(&at) {
            Some(h) => h,
            None => {
                let h = resolve_new(&at);
                let _ = kura::fs::create_dir(&h);
                h
            }
        };
    }
    host
}

/// Whether a path has a character no file name can have (wildcards, <>|"), anywhere but in a
/// \\?\ or \\.\ prefix: Windows' file functions fail on it with ERROR_INVALID_NAME, before
/// they look for the file (NtCreateFile's STATUS_OBJECT_NAME_INVALID). The MSLU loader
/// (unicows.lib) tells Windows NT from 9x by GetFileAttributesW(L"???.???") giving that error
/// (light's malie.exe, しょぱん！: as 9x it loaded no unicows.dll and crashed).
pub fn invalid_name(w: &str) -> bool {
    let p = w.strip_prefix("\\\\?\\").or_else(|| w.strip_prefix("\\\\.\\")).unwrap_or(w);
    p.contains(['?', '*', '<', '>', '|', '"'])
}

/// FindFirstFile's error when nothing matches `pat`: the wildcards of its last part are its
/// pattern, a bad character before that is ERROR_INVALID_NAME.
pub fn missing_pattern(pat: &str) -> u32 {
    let dir = pat.rsplit_once(['\\', '/']).map_or("", |x| x.0);
    if invalid_name(dir) {
        return ERROR_INVALID_NAME;
    }
    let full = full_path(pat);
    match full.trim_end_matches('\\').rsplit_once('\\') {
        Some((d, _)) if resolve(&format!("{d}\\")).is_none() => ERROR_PATH_NOT_FOUND,
        _ => ERROR_FILE_NOT_FOUND,
    }
}

pub const ERROR_INVALID_NAME: u32 = 123;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_PATH_NOT_FOUND: u32 = 3;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_FILE_EXISTS: u32 = 80;
pub const ERROR_ALREADY_EXISTS: u32 = 183;

/// CreateFile: (handle object, error), sharing everything
pub fn open(w: &str, access: u32, disposition: u32) -> Result<File, u32> {
    open_shared(w, access, disposition, None).map(|f| f.0)
}

/// CreateFile with its share mode (None: no sharing checks): (handle object, whether it existed)
pub fn open_shared(w: &str, access: u32, disposition: u32, share: Option<u32>) -> Result<(File, bool), u32> {
    if w.is_empty() {
        return Err(ERROR_PATH_NOT_FOUND);
    }
    if invalid_name(w) {
        return Err(ERROR_INVALID_NAME);
    }
    let write = access & 0x4000_0000 != 0 || access & 0x1000_0000 != 0 || access & 2 != 0;
    let existing = resolve(w);
    let host = existing.clone().unwrap_or_else(|| resolve_new(w));
    if let Some(h) = &existing {
        if kura::fs::metadata(h).is_ok_and(|m| m.is_dir()) {
            let f = kura::fs::File::open(h).map_err(|_| ERROR_ACCESS_DENIED)?;
            return Ok((File { f: Mutex::new(f), path: h.clone(), dir: true, write: false, scan: Mutex::new(None), _share: None, delete: Default::default(), shared: Default::default() }, true));
        }
    }
    // (checked and entered at once; a file being made has its entry before it exists, so a
    // second CreateFile of it racing this one sees it)
    let entry = share.map(|s| ShareEntry::take(&host, access, s)).transpose()?;
    if let Some(h) = &existing && write && extra_attrs(h) & 1 != 0 {
        return Err(ERROR_ACCESS_DENIED);
    }
    let mut o = kura::fs::OpenOptions::new();
    o.read(true);
    match disposition {
        1 => {
            // CREATE_NEW
            if existing.is_some() {
                return Err(ERROR_FILE_EXISTS);
            }
            o.write(true).create_new(true);
        }
        2 => {
            o.write(true).create(true).truncate(true);
        }
        3 => {
            if existing.is_none() {
                return Err(missing(w));
            }
            if write {
                o.write(true);
            }
        }
        4 => {
            o.write(true).create(true);
        }
        5 => {
            if existing.is_none() {
                return Err(missing(w));
            }
            o.write(true).truncate(true);
        }
        _ => return Err(87),
    }
    match o.open(&host) {
        Ok(f) => Ok((File { f: Mutex::new(f), path: host, dir: false, write, scan: Mutex::new(None), _share: entry, delete: Default::default(), shared: Default::default() }, existing.is_some())),
        Err(e) => Err(match io_err(&e) {
            IoErr::NotFound => {
                if existing.is_none() && disposition != 3 {
                    ERROR_PATH_NOT_FOUND
                } else {
                    ERROR_FILE_NOT_FOUND
                }
            }
            IoErr::Exists => ERROR_FILE_EXISTS,
            IoErr::Unsupported | IoErr::TooManyLinks | IoErr::TimedOut | IoErr::Other => ERROR_ACCESS_DENIED,
        }),
    }
}

/// The bytes after `f`'s pointer
fn left(f: &mut kura::fs::File) -> usize {
    let end = f.metadata().map_or(0, |m| m.len());
    let at = f.stream_position().unwrap_or(end);
    end.saturating_sub(at).min(usize::MAX as u64) as usize
}

/// Why a read or a write of a file did not move what it was asked to (review 6852be1f: a write
/// from a buffer that is not guest memory wrote nothing and answered the count asked, a host
/// read error was the end of the file). The end of the file is not one: a read there moves 0.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IoFail {
    /// the buffer is not all guest memory: ERROR_NOACCESS, nothing moved
    NoAccess,
    /// the host failed the read: ERROR_READ_FAULT
    Read,
    /// the host failed the write: ERROR_WRITE_FAULT
    Write,
}

/// Why read_at read nothing: at or past the end, or the host failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReadAt {
    Eof,
    Fail(IoFail),
}

impl IoFail {
    /// Its Windows error
    pub fn code(self) -> u32 {
        match self {
            IoFail::NoAccess => 998,
            IoFail::Read => 30,
            IoFail::Write => 29,
        }
    }
}

/// Reads `buf` full from `f`: the bytes read (fewer at the end of the file), Err when the host
/// failed (whatever came before it).
fn read_full(f: &mut impl Read, buf: &mut [u8]) -> Result<usize, IoFail> {
    let mut got = 0;
    while let Some(rest) = buf.get_mut(got..).filter(|r| !r.is_empty()) {
        match f.read(rest) {
            Ok(0) => break,
            Ok(k) => got = got.saturating_add(k),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(IoFail::Read),
        }
    }
    Ok(got)
}

impl File {
    /// The host file at the file pointer (see Host)
    pub fn host(&self) -> Host<'_> {
        let mut g = self.f.locked();
        let shared = self.shared.load(std::sync::atomic::Ordering::Acquire);
        if shared != 0 && let Some(p) = crate::kern::with(|k| k.file(shared).map(|f| f.pos)).0 {
            // (a seek that fails leaves the pointer: the next read or write says so)
            if g.seek(SeekFrom::Start(p)).is_err() {}
        }
        Host { g, shared }
    }
    /// This handle's file for another process (CreateProcess inheriting it): its kernel record,
    /// made on the first, with one more holder for the other process (crate::kern::new_process
    /// adds that one)
    pub fn share(&self) -> Option<u32> {
        use std::sync::atomic::Ordering;
        if self.dir {
            return None;
        }
        let now = self.shared.load(Ordering::Acquire);
        if now != 0 {
            return Some(now);
        }
        if !crate::kern::share() {
            return None;
        }
        let pos = self.f.locked().stream_position().ok()?;
        let me = crate::pid();
        let id = crate::kern::with(|k| k.share_file(&self.path, self.write, pos, me)).0;
        self.shared.store(id, Ordering::Release);
        Some(id)
    }
    /// A process's own File of shared record `id` (its inherited handle): the same host file,
    /// access and file pointer
    pub fn adopt(id: u32) -> Option<File> {
        let r = crate::kern::with(|k| k.file(id).cloned()).0?;
        let f = kura::fs::OpenOptions::new().read(true).write(r.write).open(&r.path).ok()?;
        Some(File { f: Mutex::new(f), path: r.path, dir: false, write: r.write, scan: Mutex::new(None), _share: None, delete: Default::default(), shared: std::sync::atomic::AtomicU32::new(id) })
    }
    /// Up to `n` bytes from the file pointer into guest memory at `dst`: the bytes read (0 at
    /// the end of the file).
    pub fn read_into(&self, dst: u32, n: u32) -> Result<u32, IoFail> {
        let mut f = self.host();
        // SAFETY: the closure only reads the host file into the buffer, touching no guest memory;
        // the program handed the buffer to this read (another thread of its writing it meanwhile
        // would race on Windows too).
        let got = unsafe { crate::mem::view_mut_unchecked(dst, n, |buf| read_full(&mut *f, buf)) };
        got.ok_or(IoFail::NoAccess)?.map(kome::num::u32_of)
    }
    /// Up to `n` bytes into host memory (fewer at the end of the file)
    /// The buffer grows with what is read, so a large `n` (fread's "the rest of the file"
    /// 0x7fffffff) costs what the file holds, not `n` bytes of zeros.
    pub fn read_vec(&self, n: usize) -> Vec<u8> {
        let mut f = self.host();
        let mut buf = Vec::with_capacity(n.min(left(&mut f)));
        let _ = (&mut *f).take(n as u64).read_to_end(&mut buf);
        buf
    }
    /// The bytes from the file pointer to the end (0 past it).
    pub fn remaining(&self) -> usize {
        left(&mut self.host())
    }
    /// `b` at the file pointer: its length, or why not
    pub fn write_bytes(&self, b: &[u8]) -> Result<u32, IoFail> {
        let mut f = self.host();
        f.write_all(b).map(|()| kome::num::u32_of(b.len())).map_err(|_| IoFail::Write)
    }
    /// The `n` bytes of guest memory at `src` at the file pointer: `n`, or why not (NoAccess
    /// before anything is written when they are not all guest memory)
    pub fn write_from(&self, src: u32, n: u32) -> Result<u32, IoFail> {
        let b = crate::mem::try_bytes(src, n).ok_or(IoFail::NoAccess)?;
        self.write_bytes(&b)
    }
    pub fn seek(&self, pos: SeekFrom) -> Option<u64> {
        self.host().seek(pos).ok()
    }
    /// ReadFile's read with an OVERLAPPED: from `start` (io_start; None: the file pointer),
    /// under one lock so another thread's seek on the handle can't come between. A handle
    /// opened with FILE_FLAG_OVERLAPPED (`keep`) has no file pointer to move; a synchronous
    /// one's ends after what was read, as NtReadFile leaves it. Err(()): a read of some bytes
    /// that starts at or after the end (STATUS_END_OF_FILE: ReadFile fails with
    /// ERROR_HANDLE_EOF when it was given an OVERLAPPED); a host error is its own (ReadAt::Fail).
    pub fn read_at(&self, start: Option<SeekFrom>, keep: bool, buf: &mut [u8]) -> Result<usize, ReadAt> {
        let mut f = self.host();
        let was = f.stream_position().unwrap_or(0);
        if let Some(s) = start && f.seek(s).is_err() {
            return Err(ReadAt::Fail(IoFail::Read));
        }
        let got = read_full(&mut *f, buf);
        if keep {
            let _ = f.seek(SeekFrom::Start(was));
        }
        match got {
            Ok(0) if !buf.is_empty() => Err(ReadAt::Eof),
            Ok(got) => Ok(got),
            Err(e) => Err(ReadAt::Fail(e)),
        }
    }
    /// WriteFile's write with an OVERLAPPED: at `start` (io_start: the end for
    /// FILE_WRITE_TO_END_OF_FILE; None: the file pointer), under one lock; past the end the
    /// gap reads as zeros. `keep`: as read_at. The bytes written, or why none.
    pub fn write_at(&self, start: Option<SeekFrom>, keep: bool, b: &[u8]) -> Result<u32, IoFail> {
        let mut f = self.host();
        let was = f.stream_position().unwrap_or(0);
        if let Some(s) = start && f.seek(s).is_err() {
            return Err(IoFail::Write);
        }
        let w = f.write_all(b).map(|()| kome::num::u32_of(b.len())).map_err(|_| IoFail::Write);
        if keep {
            let _ = f.seek(SeekFrom::Start(was));
        }
        w
    }
    pub fn size(&self) -> u64 {
        self.host().metadata().map_or(0, |m| m.len())
    }
}

/// FILETIME (100 ns since 1601) of a system time.
pub fn filetime(t: std::time::SystemTime) -> u64 {
    let d = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    116_444_736_000_000_000 + d.as_nanos() as u64 / 100
}

/// Attributes of host file `h` (with those SetFileAttributes gave it).
pub fn attrs_of(h: &str, m: &kura::fs::Metadata) -> u32 {
    // (attributes set replace the archive bit the host file shows)
    match extra_attrs(h) {
        0 => attrs(m),
        x => attrs(m) & 0x10 | x,
    }
}

pub fn attrs(m: &kura::fs::Metadata) -> u32 {
    if m.is_dir() {
        0x10
    } else if m.permissions().readonly() {
        0x21
    } else {
        0x20
    }
}

/// Wildcard match without case (* and ?), the way FindFirstFile matches.
pub fn wild(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    // whether p[i..] matches n[j..], for every i and j, from the ends back (the recursion it
    // was went exponential: "*a*a*a*a*b" over a long name stalled FindFirstFile)
    fn m(p: &[char], n: &[char]) -> bool {
        let w = n.len() + 1;
        let mut next = vec![false; w];
        next[n.len()] = true;
        for i in (0..p.len()).rev() {
            let mut cur = vec![false; w];
            for j in (0..w).rev() {
                cur[j] = match p[i] {
                    '*' => next[j] || j < n.len() && cur[j + 1],
                    // (DOS: '?' also matches nothing at the end of the name or before its dot)
                    '?' => j < n.len() && next[j + 1] || (j == n.len() || n[j] == '.') && next[j],
                    c => j < n.len() && n[j] == c && next[j + 1],
                };
            }
            next = cur;
        }
        next[0]
    }
    // "*.*" matches names without a dot too
    if pat == "*.*" {
        return true;
    }
    // "name." and "*.": names without an extension
    if let Some(base) = pat.strip_suffix('.').filter(|b| !b.is_empty() && !b.ends_with('.')) {
        return (name == "." || name == ".." || !name.contains('.')) && wild(base, name);
    }
    m(&p, &n)
}

pub fn find(w: &str) -> Option<Find> {
    let full = full_path(w);
    let (dir, mut pat) = full.rsplit_once('\\')?;
    // (the pattern as given: GetFullPathName's normalization drops the dot of "*.")
    let given = w.rsplit(['\\', '/']).next().unwrap_or("");
    if !given.is_empty() && given.trim_end_matches(['.', ' ']) == pat {
        pat = given;
    }
    // ("" and "dir\" find nothing, as on Windows)
    if w.is_empty() || pat.is_empty() {
        return None;
    }
    let hd = resolve(&format!("{dir}\\"))?;
    let mut entries = Vec::new();
    if !pat.contains(['*', '?']) {
        let h = resolve(&full)?;
        let m = kura::fs::metadata(&h).ok()?;
        let name = h.rsplit('/').next().unwrap_or("").to_string();
        let shorts = h.rsplit_once('/').and_then(|(d, _)| crate::shortname::short_of(if d.is_empty() { "/" } else { d }, &name)).map(|s| (name.clone(), s)).into_iter().collect();
        return Some(Find { entries: vec![(name, m)], at: 0, shorts });
    }
    if let Ok(m) = kura::fs::metadata(&hd) {
        // ("." and ".." as listed entries: only where the pattern takes them, as "*", not "*.ttf")
        if dir.len() > 3 {
            for d in [".", ".."] {
                if wild(pat, d) {
                    entries.push((d.into(), m.clone()));
                }
            }
        }
    }
    let mut all = Vec::new();
    for e in crate::shortname::read_dir(&hd).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let m = e.metadata().ok();
        all.push((name.encode_utf16().collect::<Vec<u16>>(), m.as_ref().map_or(0, kura::fs::Metadata::ino)));
        if wild(pat, &name) && let Some(m) = m {
            entries.push((name, m));
        }
    }
    if entries.is_empty() {
        return None;
    }
    // (the short names come from every entry of the directory, matched or not)
    let shorts = all.iter().zip(crate::shortname::assign(&all)).filter_map(|((n, _), s)| Some((String::from_utf16_lossy(n), String::from_utf16_lossy(&s?)))).collect();
    Some(Find { entries, at: 0, shorts })
}

#[cfg(test)]
mod tests {
    #[test]
    fn drive_relative_paths() {
        assert_eq!(drive_relative("C:1\\a.dll", "C:\\game\\"), "C:\\game\\1\\a.dll");
        assert_eq!(drive_relative("c:a", "C:\\game"), "C:\\game\\a");
        assert_eq!(drive_relative("C:", "C:\\game\\"), "C:\\game\\");
        assert_eq!(drive_relative("D:a", "C:\\game\\"), "D:\\a");
        assert_eq!(drive_relative("C:a", "C:\\"), "C:\\a");
    }
    proptest::proptest! {
        /// On the current drive the path is the current directory's and ends with what came
        /// after the colon; elsewhere the drive's root's
        #[test]
        fn drive_relative_is_under_cwd(dir in "[a-z]{1,8}(\\\\[a-z]{1,8}){0,3}", rest in "[a-z.]{0,12}", d in proptest::sample::select(vec!["C:", "c:", "D:"])) {
            let cwd = format!("C:\\{dir}\\");
            let p = format!("{d}{rest}");
            let out = drive_relative(&p, &cwd);
            proptest::prop_assert!(out.ends_with(&rest));
            if d.eq_ignore_ascii_case("C:") {
                proptest::prop_assert_eq!(out, format!("C:\\{dir}\\{rest}"));
            } else {
                proptest::prop_assert_eq!(out, format!("D:\\{rest}"));
            }
        }
    }

    use super::*;
    use proptest::prelude::*;

    /// The reviewer's case: with "=Q:" naming Q:\games, "Q:save.dat" opens Q:\games\save.dat,
    /// the file GetFullPathName names, not Q:\save.dat (review b5dacf4 2.2: files.rs had a
    /// normalization of its own that went to the drive's root).
    #[test]
    fn a_drive_relative_name_opens_what_get_full_path_name_names() {
        crate::k32::env_set("=Q:", Some("Q:\\games"));
        let (games, root) = (to_host("Q:\\games\\save.dat"), to_host("Q:\\save.dat"));
        kura::fs::create_dir_all(to_host("Q:\\games")).unwrap();
        kura::fs::write(&games, b"games").unwrap();
        kura::fs::write(&root, b"root").unwrap();
        let w: Vec<u16> = "Q:save.dat".encode_utf16().collect();
        let asked = String::from_utf16_lossy(&crate::fullpath::process_env_full(&w, 520).text());
        assert_eq!(asked, "Q:\\games\\save.dat");
        assert_eq!(full_path("Q:save.dat"), asked);
        let opened = resolve(&full_path("Q:save.dat")).unwrap();
        assert_eq!(kura::fs::read(&opened).unwrap(), b"games");
        assert_eq!(kura::fs::read(resolve(&full_path(&asked)).unwrap()).unwrap(), b"games");
        crate::k32::env_set("=Q:", None);
    }

    proptest! {
        /// Any DOS path (relative, rooted, drive-relative with and without its drive's
        /// variable, UNC, "." "..", trailing periods and spaces): the full path every operation
        /// opens is GetFullPathName's answer (drive letter upper case), and the path that answer
        /// names opens the same (making it full again changes nothing).
        #[test]
        fn operations_open_what_get_full_path_name_names(
            head in proptest::sample::select(vec!["", "\\", "C:", "c:\\", "R:", "R:\\", "\\\\srv\\share\\", ".\\", "..\\"]),
            parts in proptest::collection::vec(proptest::sample::select(vec!["a", "b.", "c..", ".", "..", "d ", "e. .", "f/g"]), 0..5),
            set in proptest::bool::ANY,
        ) {
            crate::k32::env_set("=R:", set.then_some("R:\\"));
            let p = format!("{head}{}", parts.join("\\"));
            let w: Vec<u16> = p.encode_utf16().collect();
            let mut asked: Vec<u16> = crate::fullpath::process_env_full(&w, 0xfffe).text();
            if asked.get(1) == Some(&u16::from(b':')) && let Some(d) = asked.first_mut() && let Ok(b) = u8::try_from(*d) {
                *d = u16::from(b.to_ascii_uppercase());
            }
            let asked = String::from_utf16_lossy(&asked);
            let full = full_path(&p);
            prop_assert_eq!(&full, &asked, "{}", p);
            if !full.is_empty() {
                prop_assert_eq!(full_path(&full), full.clone(), "{}", p);
            }
            crate::k32::env_set("=R:", None);
        }
    }

    /// Folder names that end in one period lose it (RGSS2's Game.exe loads
    /// "<RTP folder>.\RGSS202J.dll", #232); two or more periods stay; the last component
    /// loses all its trailing periods and spaces; \\?\ paths keep theirs.
    #[test]
    fn folder_names_lose_one_trailing_period() {
        let rtp = "C:\\Program Files\\Common Files\\Enterbrain\\RGSS2\\RPGVX";
        assert_eq!(full_path(&format!("{rtp}.\\RGSS202J.dll")), format!("{rtp}\\RGSS202J.dll"));
        assert_eq!(full_path("C:\\a.\\b.\\c"), "C:\\a\\b\\c");
        assert_eq!(full_path("C:\\a..\\c"), "C:\\a..\\c");
        assert_eq!(full_path("C:\\a...\\c"), "C:\\a...\\c");
        assert_eq!(full_path("C:\\a .\\c"), "C:\\a \\c");
        assert_eq!(full_path("C:\\a.\\"), "C:\\a\\");
        assert_eq!(full_path("C:\\d\\a. ."), "C:\\d\\a");
        assert_eq!(full_path("\\\\?\\C:\\a.\\b"), "C:\\a.\\b");
    }

    proptest! {
        /// Windows' normalization as the .NET docs give it, against full_path on random
        /// absolute paths of names, periods and spaces: "." and ".." are relative parts; a
        /// folder's name ending in a single period loses it; the last name loses its trailing
        /// periods and spaces. And taking the full path again changes nothing.
        #[test]
        fn trailing_periods_like_windows(mut parts in proptest::collection::vec("[ab]{0,2}[. ]{0,3}", 0..5), last in "[ab]{1,2}[. ]{0,3}") {
            // (the last name not empty: a path that ends in a separator keeps it)
            parts.push(last);
            let p = format!("C:\\{}", parts.join("\\"));
            let mut want: Vec<String> = Vec::new();
            for (i, c) in parts.iter().enumerate() {
                let last = i + 1 == parts.len();
                match c.as_str() {
                    "" | "." => {}
                    ".." => { want.pop(); }
                    c if last => {
                        let t = c.trim_end_matches(['.', ' ']);
                        want.push(if t.is_empty() { c.to_string() } else { t.to_string() });
                    }
                    c if c.ends_with('.') && !c.ends_with("..") => want.push(c[..c.len() - 1].to_string()),
                    c => want.push(c.to_string()),
                }
            }
            let got = full_path(&p);
            prop_assert_eq!(&got, &format!("C:\\{}", want.join("\\")), "{}", p);
            prop_assert_eq!(full_path(&got), got.clone());
        }
    }

    #[test]
    fn names_with_wildcards_are_invalid() {
        // (winref badnames: ERROR_INVALID_NAME wherever the character is, not in a \\?\ prefix)
        for p in ["???.???", "a?.txt", "a*", "a<b", "a>b", "a|b", "a\"b", "no?dir\\a.txt", "C:\\x\\y*", "\\\\?\\C:\\a?"] {
            assert!(invalid_name(p), "{p}");
            assert_eq!(missing(p), ERROR_INVALID_NAME, "{p}");
        }
        for p in ["a.txt", "\\\\?\\C:\\a.txt", "\\\\.\\pipe\\x", "C:\\dir\\", "", "\u{3042}.txt"] {
            assert!(!invalid_name(p), "{p}");
        }
        // FindFirstFile: the last part's wildcards are its pattern
        assert_eq!(missing_pattern("no?dir\\*"), ERROR_INVALID_NAME);
        assert_ne!(missing_pattern("*.zzz-sake-test"), ERROR_INVALID_NAME);
    }

    proptest! {
        // a path is invalid exactly when it has one of ?*<>|" outside a \\?\ or \\.\ prefix
        #[test]
        fn invalid_names_are_the_ones_with_bad_characters(pre in prop::sample::select(vec!["", "\\\\?\\", "\\\\.\\", "C:\\"]),
                                                         body in "[a-zA-Z0-9 ._\\\\\u{3042}-\u{3093}?*<>|\"]{0,24}") {
            let p = format!("{pre}{body}");
            let bad = body.contains(['?', '*', '<', '>', '|', '"']);
            prop_assert_eq!(invalid_name(&p), bad);
            if bad {
                prop_assert_eq!(missing(&p), ERROR_INVALID_NAME);
            }
        }
    }

    #[test]
    fn a_drive_root_is_made_when_missing() {
        let h = std::env::temp_dir().join(format!("sake-drive-{}", std::process::id())).join("c");
        let h = h.to_string_lossy().into_owned();
        let _ = std::fs::remove_dir_all(&h);
        assert_eq!(drive_root(h.clone()), h);
        assert!(kura::fs::metadata(&h).unwrap().is_dir());
        // (and again, when it is there)
        assert_eq!(drive_root(h.clone()), h);
        // C:\ itself, as stat and GetFileAttributes look it up
        let c = resolve("C:\\").expect("C:\\ is there");
        assert!(kura::fs::metadata(&c).unwrap().is_dir());
        assert!(resolve("C:/").is_some() && resolve("c:\\").is_some());
        let _ = std::fs::remove_dir_all(std::path::Path::new(&h).parent().unwrap());
    }

    #[test]
    fn attributes_made_and_set_as_windows() {
        let f = std::env::temp_dir().join(format!("sake-attrs-{}", std::process::id()));
        std::fs::write(&f, b"abc").unwrap();
        let (h, m) = (f.to_string_lossy().into_owned(), kura::fs::metadata(&f).unwrap());
        assert_eq!(attrs_of(&h, &m), 0x20);
        // made hidden by CreateFile / CreateFile2: hidden and archive (winref files: 34)
        set_attrs(&h, 2 | 0x20);
        assert_eq!(attrs_of(&h, &m), 0x22);
        // SetFileAttributes replaces: hidden alone, then read-only alone (winref files c.dat: 1)
        set_attrs(&h, 2);
        assert_eq!(attrs_of(&h, &m), 2);
        set_attrs(&h, 1);
        assert_eq!(attrs_of(&h, &m), 1);
        set_attrs(&h, 0x23);
        assert_eq!(attrs_of(&h, &m), 0x23);
        // normal, or archive alone: what the host file shows
        set_attrs(&h, 0x80);
        assert_eq!(attrs_of(&h, &m), 0x20);
        set_attrs(&h, 0x20);
        assert_eq!(attrs_of(&h, &m), 0x20);
        // the case of the host path does not matter
        set_attrs(&h.to_uppercase(), 4);
        assert_eq!(extra_attrs(&h), 4);
        set_attrs(&h, 0x80);
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn set_write_times_hold_until_the_file_changes() {
        let f = std::env::temp_dir().join(format!("sake-times-{}", std::process::id()));
        std::fs::write(&f, b"abc").unwrap();
        let h = f.to_string_lossy().into_owned();
        let m = kura::fs::metadata(&h).unwrap();
        let real = write_time(&h, &m);
        assert_ne!(real, 0);
        // _Set_last_write_time to 1970 + 1 s: GetFileTime gives it back
        set_write_time(&h, 116_444_736_010_000_000);
        assert_eq!(write_time(&h, &kura::fs::metadata(&h).unwrap()), 116_444_736_010_000_000);
        // a write after it moves the host's time: the host's time shows again
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&f, b"abcd").unwrap();
        let m2 = kura::fs::metadata(&h).unwrap();
        if m2.modified().ok() != m.modified().ok() {
            assert_eq!(write_time(&h, &m2), m2.modified().map_or(0, filetime));
        }
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn file_indexes_tell_files_apart_and_follow_hard_links() {
        let d = std::env::temp_dir().join(format!("sake-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (a, b, l) = (d.join("a"), d.join("b"), d.join("l"));
        std::fs::write(&a, b"1").unwrap();
        std::fs::write(&b, b"2").unwrap();
        let idx = |p: &std::path::Path| {
            let h = p.to_string_lossy().into_owned();
            file_index(&h, &kura::fs::metadata(&h).unwrap())
        };
        assert_ne!(idx(&a), idx(&b));
        assert_eq!(idx(&a), idx(&a));
        kura::fs::hard_link(&a, &l).unwrap();
        assert_eq!(idx(&a), idx(&l));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn made_directories_follow_the_existing_spelling() {
        let root = std::env::temp_dir().join(format!("sake-mkdirs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Windows").join("System32")).unwrap();
        let r = root.to_string_lossy().into_owned();
        // as the Z: drive maps the host's /, a path under the temporary root
        let w = |p: &str| format!("Z:{}\\{p}", r.replace('/', "\\"));
        let h = make_dirs(&w("windows\\temp"));
        assert!(h.ends_with("Windows/temp"), "{h}");
        assert!(!root.join("windows").exists());
        // a second time: the same directory
        assert_eq!(make_dirs(&w("WINDOWS\\TEMP\\")), h);
        // new parents too
        let h2 = make_dirs(&w("windows\\a\\b"));
        assert!(h2.ends_with("Windows/a/b") && std::path::Path::new(&h2).is_dir(), "{h2}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A new name under a parent spelled in another case, with trailing backslashes or not
    /// (YU-RIS: CreateDirectory("...\\(実況ok)\\save\\") with the folder on the host as
    /// "(実況OK)"): the parent's host spelling, the new name as given.
    #[test]
    fn new_names_follow_the_parent_spelling_with_trailing_backslashes() {
        let root = std::env::temp_dir().join(format!("sake-newname-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Game（実況OK）")).unwrap();
        let r = root.to_string_lossy().into_owned();
        let w = |p: &str| format!("Z:{}\\{p}", r.replace('/', "\\"));
        let want = format!("{r}/Game（実況OK）/save");
        assert_eq!(resolve_new(&w("game（実況ok）\\save")), want);
        assert_eq!(resolve_new(&w("game（実況ok）\\save\\")), want);
        assert_eq!(resolve_new(&w("GAME（実況OK）\\save\\\\")), want);
        kura::fs::create_dir(resolve_new(&w("game（実況ok）\\save\\"))).unwrap();
        assert!(root.join("Game（実況OK）").join("save").is_dir());
        // a drive root stays a root
        assert_eq!(resolve_new("Z:\\"), "/");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A temporary root seen through the Z: drive (the host's /), and `w` mapping a Windows
    /// path under it.
    fn z_root(tag: &str) -> (std::path::PathBuf, impl Fn(&str) -> String + use<>) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!("sake-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let r = root.to_string_lossy().replace('/', "\\");
        (root, move |p: &str| format!("Z:{r}\\{p}"))
    }

    // CreateDirectory("...\\documents\\orcsoft\\") under a host "Documents": the trailing
    // separator names the folder itself, and the parent is found whatever its case (the
    // ORCSOFT KiriKiri Z trial's save folder; its TVPCreateFolders passes each parent so)
    #[test]
    fn create_directory_takes_trailing_separators_and_any_case() {
        let (root, w) = z_root("mkdir-sep");
        std::fs::create_dir_all(root.join("users").join("Documents")).unwrap();
        assert_eq!(crate::k32::make_dir(&w("users\\documents\\ORCSOFT\\")), Ok(()));
        assert!(root.join("users/Documents/ORCSOFT").is_dir());
        assert!(!root.join("users/documents").exists());
        // there now: ERROR_ALREADY_EXISTS, with or without the separator
        assert_eq!(crate::k32::make_dir(&w("users\\DOCUMENTS\\orcsoft")), Err(183));
        assert_eq!(crate::k32::make_dir(&w("users\\documents\\orcsoft\\\\")), Err(183));
        // a missing parent: ERROR_PATH_NOT_FOUND, and nothing made
        assert_eq!(crate::k32::make_dir(&w("users\\nothere\\x\\")), Err(3));
        assert!(!root.join("users/nothere").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    proptest::proptest! {
        /// Any case of the parent's ASCII letters and any number of trailing backslashes: the
        /// same host path, under the parent as the host spells it.
        #[test]
        fn new_name_case_and_trailing_separators(flips in proptest::collection::vec(proptest::bool::ANY, 12), tail in 0usize..4, name in "[a-z0-9]{1,8}") {
            let root = std::env::temp_dir().join(format!("sake-newprop-{}", std::process::id()));
            std::fs::create_dir_all(root.join("SaveData").join("Sub")).unwrap();
            let r = root.to_string_lossy().into_owned();
            let parent: String = "SaveData\\Sub".chars().zip(flips.iter().cycle()).map(|(c, &f)| if f { c.to_ascii_lowercase() } else { c.to_ascii_uppercase() }).collect();
            let w = format!("Z:{}\\{parent}\\{name}{}", r.replace('/', "\\"), "\\".repeat(tail));
            proptest::prop_assert_eq!(resolve_new(&w), format!("{r}/SaveData/Sub/{name}"));
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]
        // for any existing parents, any case of them in the request and any trailing
        // separators: the folder is made inside the existing parents (no second spelling of a
        // parent appears), once (then 183)
        #[test]
        fn mkdir_lands_in_the_existing_spelling(
            parents in proptest::collection::vec("[A-Za-z][A-Za-z0-9]{0,6}", 1..4),
            flips in proptest::collection::vec(proptest::bool::ANY, 32),
            leaf in "[A-Za-z0-9]{1,8}",
            seps in "[\\\\/]{0,3}",
        ) {
            let (root, w) = z_root("mkdir-pbt");
            let host_parent = parents.iter().fold(root.clone(), |p, n| p.join(n));
            std::fs::create_dir_all(&host_parent).unwrap();
            let mut k = 0;
            let asked: Vec<String> = parents.iter().map(|n| n.chars().map(|ch| {
                k += 1;
                if flips[k % flips.len()] { ch.to_ascii_uppercase() } else { ch.to_ascii_lowercase() }
            }).collect()).collect();
            let path = w(&format!("{}\\{leaf}{seps}", asked.join("\\")));
            proptest::prop_assert_eq!(crate::k32::make_dir(&path), Ok(()), "{}", path);
            proptest::prop_assert!(host_parent.join(&leaf).is_dir());
            // no other spelling of any parent level
            let mut at = root.clone();
            for n in &parents {
                let names: Vec<String> = std::fs::read_dir(&at).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
                proptest::prop_assert_eq!(names.iter().filter(|x| x.eq_ignore_ascii_case(n)).count(), 1, "{:?} in {:?}", n, names);
                at = at.join(n);
            }
            proptest::prop_assert_eq!(crate::k32::make_dir(&path), Err(183));
            let _ = std::fs::remove_dir_all(&root);
        }
    }


    // ReadFile / WriteFile with an OVERLAPPED (#154: CisLugI 体験版's SQLite database). Before
    // e43c5ad7 WriteFile wrote at the file pointer whatever Offset said: SQLite's page 1 landed
    // on page 9, the database was "malformed", and the game stopped using its archives.
    #[test]
    fn overlapped_offsets_as_windows() {
        assert_eq!(io_start(5, 0, false), Some(SeekFrom::Start(5)));
        assert_eq!(io_start(1, 2, true), Some(SeekFrom::Start(0x2_0000_0001)));
        // FILE_WRITE_TO_END_OF_FILE: a write's; FILE_USE_FILE_POINTER_POSITION: both
        assert_eq!(io_start(!0, !0, true), Some(SeekFrom::End(0)));
        assert_eq!(io_start(!1, !0, true), None);
        assert_eq!(io_start(!1, !0, false), None);

        let (root, w) = z_root("ovio");
        let f = open(&w("db"), 0xc000_0000, 2).unwrap();
        assert_eq!(f.write_bytes(b"0123456789"), Ok(10));
        f.seek(SeekFrom::Start(2));
        // a synchronous handle: at Offset, and the pointer after what was written / read
        assert_eq!(f.write_at(io_start(5, 0, true), false, b"AB"), Ok(2));
        assert_eq!(f.seek(SeekFrom::Current(0)), Some(7));
        let mut b = [0u8; 3];
        assert_eq!(f.read_at(io_start(1, 0, false), false, &mut b), Ok(3));
        assert_eq!((&b, f.seek(SeekFrom::Current(0))), (b"123", Some(4)));
        // at or past the end: STATUS_END_OF_FILE, the pointer at Offset; nothing asked: fine
        assert_eq!(f.read_at(io_start(10, 0, false), false, &mut b), Err(ReadAt::Eof));
        assert_eq!(f.read_at(io_start(100, 0, false), false, &mut b), Err(ReadAt::Eof));
        assert_eq!(f.seek(SeekFrom::Current(0)), Some(100));
        assert_eq!(f.read_at(io_start(100, 0, false), false, &mut []), Ok(0));
        // fewer at the end
        assert_eq!(f.read_at(io_start(8, 0, false), false, &mut b), Ok(2));
        // past the end: a gap of zeros; the end; the pointer
        assert_eq!(f.write_at(io_start(12, 0, true), false, b"x"), Ok(1));
        assert_eq!(f.write_at(io_start(!0, !0, true), false, b"yz"), Ok(2));
        assert_eq!(f.seek(SeekFrom::Current(0)), Some(15));
        f.seek(SeekFrom::Start(0));
        assert_eq!(f.write_at(io_start(!1, !0, true), false, b"P"), Ok(1));
        assert_eq!(std::fs::read(root.join("db")).unwrap(), b"P1234AB789\0\0xyz");
        // a FILE_FLAG_OVERLAPPED handle keeps no pointer: IO at Offset leaves it alone
        f.seek(SeekFrom::Start(3));
        assert_eq!(f.write_at(io_start(0, 0, true), true, b"Q"), Ok(1));
        assert_eq!(f.read_at(io_start(9, 0, false), true, &mut b), Ok(3));
        assert_eq!(f.read_at(io_start(99, 0, false), true, &mut b), Err(ReadAt::Eof));
        assert_eq!(f.seek(SeekFrom::Current(0)), Some(3));
        drop(f);
        let _ = std::fs::remove_dir_all(&root);
    }

    // Threads writing their own pages at their Offsets while another moves the pointer: each
    // page is where its Offset says (the seek and the write under one lock).
    #[test]
    fn overlapped_writes_from_threads_land_at_their_offsets() {
        let (root, w) = z_root("ovthreads");
        let f = std::sync::Arc::new(open(&w("db"), 0xc000_0000, 2).unwrap());
        const PAGE: usize = 512;
        let threads: Vec<_> = (0..4u8)
            .map(|t| {
                let f = f.clone();
                std::thread::spawn(move || {
                    for round in 0..50u32 {
                        let page = t as u32 * 50 + round;
                        if t == 3 && round % 2 == 0 {
                            f.seek(SeekFrom::Start(round as u64 * 7));
                        }
                        let b = [t ^ (round as u8); PAGE];
                        assert_eq!(f.write_at(io_start(page * PAGE as u32, 0, true), false, &b), Ok(PAGE as u32));
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let data = std::fs::read(root.join("db")).unwrap();
        assert_eq!(data.len(), 200 * PAGE);
        for page in 0..200u32 {
            let (t, round) = ((page / 50) as u8, page % 50);
            assert!(data[page as usize * PAGE..][..PAGE].iter().all(|&x| x == t ^ round as u8), "page {page}");
        }
        drop(f);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[derive(Debug, Clone)]
    enum Op {
        /// WriteFile with an OVERLAPPED (lo, hi), these bytes
        WriteAt(u32, u32, Vec<u8>),
        /// ReadFile with an OVERLAPPED (lo, hi), this many bytes
        ReadAt(u32, u32, usize),
        /// WriteFile / ReadFile without one
        Write(Vec<u8>),
        Read(usize),
        Seek(u64),
    }
    fn op() -> impl Strategy<Value = Op> {
        // (offsets near the data, past it, and the two special values)
        let off = prop_oneof![4 => (0u32..96).prop_map(|o| (o, 0u32)), 1 => Just((!0u32, !0u32)), 1 => Just((!1u32, !0u32))];
        let bytes = prop::collection::vec(any::<u8>(), 0..24);
        prop_oneof![
            (off.clone(), bytes.clone()).prop_map(|((lo, hi), b)| Op::WriteAt(lo, hi, b)),
            (off, 0usize..24).prop_map(|((lo, hi), n)| Op::ReadAt(lo, hi, n)),
            bytes.prop_map(Op::Write),
            (0usize..24).prop_map(Op::Read),
            (0u64..96).prop_map(Op::Seek),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        /// Any mix of IO with and without OVERLAPPED gives what a model of Windows' file gives
        /// (bytes, a pointer that synchronous IO moves and FILE_FLAG_OVERLAPPED IO never does,
        /// zeros in a gap, the end for all ones, the pointer for 0xfffffffe, EOF for reads at or
        /// past the end).
        #[test]
        fn overlapped_io_matches_a_model(ops in prop::collection::vec(op(), 1..40), keep in any::<bool>()) {
            let (root, w) = z_root("ovprop");
            let f = open(&w("db"), 0xc000_0000, 2).unwrap();
            let (mut data, mut pos) = (Vec::<u8>::new(), 0usize);
            for o in ops {
                match o {
                    Op::WriteAt(lo, hi, b) => {
                        let at = match (hi, lo) { (0xffff_ffff, 0xffff_ffff) => data.len(), (0xffff_ffff, 0xffff_fffe) => pos, _ => lo as usize };
                        // (a null write moves nothing on NTFS: no gap either)
                        if !b.is_empty() && data.len() < at + b.len() { data.resize(at + b.len(), 0); }
                        if !b.is_empty() { data[at..at + b.len()].copy_from_slice(&b); }
                        if !keep { pos = at + b.len(); }
                        prop_assert_eq!(f.write_at(io_start(lo, hi, true), keep, &b), Ok(b.len() as u32));
                    }
                    Op::ReadAt(lo, hi, n) => {
                        let at = match (hi, lo) { (0xffff_ffff, 0xffff_fffe) => pos, (0xffff_ffff, _) => usize::MAX, _ => lo as usize };
                        let mut b = vec![0; n];
                        let got = f.read_at(io_start(lo, hi, false), keep, &mut b);
                        if at == usize::MAX {
                            // (all ones for a read: past any end; where the pointer goes is not
                            // modelled, so the model's is taken from the file)
                            prop_assert!(n == 0 || got.is_err());
                            pos = f.seek(SeekFrom::Current(0)).unwrap() as usize;
                            continue;
                        }
                        let want = &data[at.min(data.len())..(at + n).min(data.len())];
                        if n > 0 && want.is_empty() {
                            prop_assert_eq!(got, Err(ReadAt::Eof));
                        } else {
                            prop_assert_eq!(got, Ok(want.len()));
                            prop_assert_eq!(&b[..want.len()], want);
                        }
                        if !keep { pos = at + want.len(); }
                    }
                    Op::Write(b) => {
                        if !b.is_empty() && data.len() < pos + b.len() { data.resize(pos + b.len(), 0); }
                        if !b.is_empty() { data[pos..pos + b.len()].copy_from_slice(&b); }
                        pos += b.len();
                        prop_assert_eq!(f.write_bytes(&b), Ok(b.len() as u32));
                    }
                    Op::Read(n) => {
                        let want = data[pos.min(data.len())..(pos + n).min(data.len())].to_vec();
                        pos += want.len();
                        prop_assert_eq!(f.read_vec(n), want);
                    }
                    Op::Seek(p) => {
                        pos = p as usize;
                        prop_assert_eq!(f.seek(SeekFrom::Start(p)), Some(p));
                    }
                }
                prop_assert_eq!(f.seek(SeekFrom::Current(0)), Some(pos as u64));
            }
            drop(f);
            prop_assert_eq!(std::fs::read(root.join("db")).unwrap(), data);
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// Review 6852be1f: CreateFile's sharing check and its entry were two locks apart, so
    /// opens that exclude each other, racing, both got in. Threads open one file at once, each
    /// asking no sharing, and hold what they got until all have tried: exactly one gets it, the
    /// rest ERROR_SHARING_VIOLATION; for a file that exists and one being made (OPEN_ALWAYS).
    #[test]
    fn exclusive_opens_racing_let_one_in() {
        let (root, w) = z_root("sharerace");
        for round in 0..40 {
            let path = w(&format!("f{}", round % 2));
            if round % 4 < 2 {
                std::fs::write(root.join(format!("f{}", round % 2)), b"x").unwrap();
            } else {
                let _ = std::fs::remove_file(root.join(format!("f{}", round % 2)));
            }
            let n = 8;
            let go = std::sync::Arc::new(std::sync::Barrier::new(n));
            let tried = std::sync::Arc::new(std::sync::Barrier::new(n));
            let got: Vec<Result<bool, u32>> = std::thread::scope(|s| {
                let hs: Vec<_> = (0..n)
                    .map(|_| {
                        let (go, tried, path) = (go.clone(), tried.clone(), path.clone());
                        s.spawn(move || {
                            go.wait();
                            let r = open_shared(&path, 0xc000_0000, 4, Some(0));
                            tried.wait();
                            r.map(|(f, _)| {
                                drop(f);
                                true
                            })
                        })
                    })
                    .collect();
                hs.into_iter().map(|h| h.join().unwrap()).collect()
            });
            assert_eq!(got.iter().filter(|r| r.is_ok()).count(), 1, "round {round}: {got:?}");
            assert!(got.iter().all(|r| matches!(r, Ok(true) | Err(32))), "round {round}: {got:?}");
            assert!(!OPEN.locked().iter().any(|o| o.1.contains("sharerace")), "every entry went with its handle");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]
        /// Review 6852be1f: a write from a buffer that is not all guest memory moves nothing
        /// and says NoAccess (it wrote nothing and answered the count); a read into one says
        /// NoAccess; the end of the file reads 0 bytes, Ok, not an error; a read the host fails
        /// (a directory) is an error, not the end. Against the bytes on the host.
        #[test]
        fn io_results_tell_their_cause(data in proptest::collection::vec(any::<u8>(), 1..300), bad in prop::sample::select(vec![0u32, 0x10, 0xffff_0000]), n in 1u32..64) {
            let g = crate::test_guest::enter();
            let (root, w) = z_root("iofail");
            let f = open(&w("f"), 0xc000_0000, 2).map_err(|e| TestCaseError::fail(format!("{e}")))?;
            prop_assert_eq!(f.write_bytes(&data), Ok(data.len() as u32));
            prop_assert_eq!(f.write_from(bad, n), Err(IoFail::NoAccess));
            prop_assert_eq!(std::fs::read(root.join("f")).ok(), Some(data.clone()), "nothing written");
            f.seek(SeekFrom::Start(0));
            prop_assert_eq!(f.read_into(bad, n), Err(IoFail::NoAccess));
            let buf = crate::mem::Scratch::new(512, true);
            let _ = &g;
            prop_assert_eq!(f.read_into(buf.get(), 512), Ok(data.len() as u32));
            prop_assert_eq!(crate::mem::bytes(buf.get(), data.len() as u32), data.clone());
            prop_assert_eq!(f.read_into(buf.get(), n), Ok(0), "the end of the file");
            prop_assert_eq!(f.write_from(buf.get(), n), Ok(n));
            std::fs::create_dir_all(root.join("d")).map_err(|e| TestCaseError::fail(format!("{e}")))?;
            let d = File { f: Mutex::new(kura::fs::File::open(root.join("d").to_string_lossy().as_ref()).map_err(|e| TestCaseError::fail(format!("{e}")))?), path: String::new(), dir: true, write: false, scan: Mutex::new(None), _share: None, delete: Default::default(), shared: Default::default() };
            prop_assert_eq!(d.read_into(buf.get(), n), Err(IoFail::Read));
            let mut host = [0u8; 4];
            prop_assert_eq!(d.read_at(None, false, &mut host), Err(ReadAt::Fail(IoFail::Read)));
            drop(buf);
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}

#[cfg(test)]
mod wild_tests {
    use super::*;
    use proptest::prelude::*;

    /// The recursion wild was (the reference).
    fn old_m(p: &[char], n: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|i| old_m(&p[1..], &n[i..])),
            Some('?') => !n.is_empty() && old_m(&p[1..], &n[1..]) || (n.is_empty() || n[0] == '.') && old_m(&p[1..], n),
            Some(c) => n.first() == Some(c) && old_m(&p[1..], &n[1..]),
        }
    }

    proptest! {
        /// The table answers as the recursion did, for any pattern and name
        #[test]
        fn wildcards_match_as_before(p in "[ab.*?A]{0,8}", n in "[abAB.]{0,10}") {
            let (pl, nl): (Vec<char>, Vec<char>) = (p.to_lowercase().chars().collect(), n.to_lowercase().chars().collect());
            let want = if p == "*.*" { true } else if let Some(base) = p.strip_suffix('.').filter(|b| !b.is_empty() && !b.ends_with('.')) {
                (n == "." || n == ".." || !n.contains('.')) && {
                    let b: Vec<char> = base.to_lowercase().chars().collect();
                    if base == "*.*" { true } else { old_m(&b, &nl) }
                }
            } else {
                old_m(&pl, &nl)
            };
            prop_assert_eq!(wild(&p, &n), want, "{:?} {:?}", p, n);
        }
    }

    /// Many stars over a long name that does not match: at once
    #[test]
    fn many_stars_are_quick() {
        let t = std::time::Instant::now();
        let name = "a".repeat(250);
        assert!(!wild("*a*a*a*a*a*a*a*a*a*a*a*a*b", &name));
        assert!(wild("*a*a*a*a*a*a*a*a*a*a*a*a*", &name));
        assert!(t.elapsed() < std::time::Duration::from_millis(500));
    }
}

#[cfg(test)]
mod io_err_tests {
    use super::*;
    use std::io::{Error, ErrorKind as K};

    /// each kind sake tells apart, and a spread of the rest (Other)
    #[test]
    fn io_err_names_the_kinds() {
        for (k, want) in [
            (K::NotFound, IoErr::NotFound),
            (K::AlreadyExists, IoErr::Exists),
            (K::Unsupported, IoErr::Unsupported),
            (K::TooManyLinks, IoErr::TooManyLinks),
            (K::TimedOut, IoErr::TimedOut),
            (K::PermissionDenied, IoErr::Other),
            (K::Other, IoErr::Other),
            (K::InvalidData, IoErr::Other),
            (K::Interrupted, IoErr::Other),
            (K::StorageFull, IoErr::Other),
        ] {
            assert_eq!(io_err(&Error::from(k)), want, "{k:?}");
        }
        // (from the OS too: ENOENT, EEXIST)
        assert_eq!(io_err(&Error::from_raw_os_error(2)), IoErr::NotFound);
        assert_eq!(io_err(&Error::from_raw_os_error(17)), IoErr::Exists);
    }
}
