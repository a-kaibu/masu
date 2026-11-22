#![allow(non_snake_case)]
//! The loader's search paths as ntdll builds them: RtlGetSearchPath (SearchPath), RtlGetExePath
//! (CreateProcess), LdrGetDllPath (LoadLibrary), and the folders that shape them
//! (LdrAddDllDirectory, LdrRemoveDllDirectory, LdrSetDefaultDllDirectories), from the
//! clean-room notes (project folder sake-re/ntdll/dll-path.md); checked by Wine's kernel32:path.
//!
//! A path is a list of elements (RtlpComputePath's codes), each giving folders followed by ';',
//! the last ';' cut.

use crate::api::Ctx;
use crate::k32::last_error;
use crate::mem;
use kome::locks::{Lock as _, Rw as _};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

const STATUS_INVALID_PARAMETER: u32 = 0xc000_000d;
const STATUS_NAME_TOO_LONG: u32 = 0xc000_0106;

/// LOAD_WITH_ALTERED_SEARCH_PATH
const ALTERED: u32 = 0x8;
/// LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR
const DLL_LOAD_DIR: u32 = 0x100;
/// LOAD_LIBRARY_SEARCH_APPLICATION_DIR
const APPLICATION_DIR: u32 = 0x200;
/// LOAD_LIBRARY_SEARCH_USER_DIRS
const USER_DIRS: u32 = 0x400;
/// LOAD_LIBRARY_SEARCH_SYSTEM32
const SYSTEM32: u32 = 0x800;
/// LOAD_LIBRARY_SEARCH_DEFAULT_DIRS
const DEFAULT_DIRS: u32 = 0x1000;
/// LOAD_LIBRARY_SAFE_CURRENT_DIRS
const SAFE_CURRENT_DIRS: u32 = 0x2000;
/// LOAD_LIBRARY_SEARCH_SYSTEM32_NO_FORWARDER
const SYSTEM32_NO_FORWARDER: u32 = 0x4000;
/// LdrpValidPathComponentsMask (the policy bit for user folders set: (4 | 0x7b) << 8)
const VALID_COMPONENTS: u32 = 0x7f00;

/// RtlpComputePath's element codes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elem {
    /// 0: SetDllDirectory's folder, when one is set
    DllDirectory,
    /// 1: the exe's folder
    App,
    /// 2: system32, system, the Windows folder (RtlpSystemDirs)
    SystemDirs,
    /// 3: the PATH variable, when not empty
    PathVar,
    /// 4: "."
    Dot,
    /// 5: the module's folder when one is given, else the exe's
    ModuleOrApp,
    /// 6: AddDllDirectory's folders, newest first
    UserDirs,
    /// 7: system32 (RtlpSystem32Dirs)
    System32,
    /// 8: the app packages' path (none here)
    Packages,
}

use Elem::*;

/// Search order with the current folder before the system ones (safe search off)
pub const UNSAFE: [Elem; 5] = [ModuleOrApp, Dot, SystemDirs, PathVar, Packages];
/// Safe search: the current folder after the Windows one
pub const SAFE: [Elem; 5] = [ModuleOrApp, SystemDirs, Dot, PathVar, Packages];
/// RtlGetExePath for a name that needs no current folder
pub const EXE_NO_CUR: [Elem; 4] = [App, SystemDirs, PathVar, Packages];
/// A DLL path while SetDllDirectory has a folder
pub const WITH_DLL_DIRECTORY: [Elem; 5] = [ModuleOrApp, DllDirectory, SystemDirs, PathVar, Packages];

/// What the elements are made of
pub struct Parts<'a> {
    pub app: &'a str,
    pub module: Option<&'a str>,
    pub dll_directory: Option<&'a str>,
    pub user_dirs: &'a [String],
    pub path_var: &'a str,
}

const SYSTEM_DIRS: &str = "C:\\windows\\system32;C:\\windows\\system;C:\\windows";
const SYSTEM32_DIR: &str = "C:\\windows\\system32";

/// RtlpComputePath: the elements' folders, each followed by ';', the last one cut
pub fn compute(elems: &[Elem], p: &Parts) -> String {
    let mut out = String::new();
    let mut add = |s: &str| {
        if !s.is_empty() {
            out.push_str(s);
            out.push(';');
        }
    };
    for e in elems {
        match e {
            DllDirectory => add(p.dll_directory.unwrap_or("")),
            App => add(p.app),
            SystemDirs => add(SYSTEM_DIRS),
            PathVar => add(p.path_var),
            Dot => add("."),
            ModuleOrApp => add(p.module.unwrap_or(p.app)),
            UserDirs => p.user_dirs.iter().rev().for_each(|d| add(d)),
            System32 => add(SYSTEM32_DIR),
            Packages => {}
        }
    }
    out.pop();
    out
}

/// RtlpComputeDllPathWithOptions: the LOAD_LIBRARY_SEARCH_* folders in their order
pub fn with_options(flags: u32, dll_directory_set: bool) -> Vec<Elem> {
    let mut v = Vec::new();
    if flags & DLL_LOAD_DIR != 0 {
        v.push(ModuleOrApp);
    }
    if flags & APPLICATION_DIR != 0 {
        v.push(App);
    }
    if flags & USER_DIRS != 0 {
        v.push(UserDirs);
        if dll_directory_set {
            v.push(DllDirectory);
        }
    }
    if flags & (SYSTEM32 | SYSTEM32_NO_FORWARDER) != 0 {
        v.push(System32);
    }
    v.push(Packages);
    v
}

/// The folder part of a module name: up to its last '\' (none without one)
pub fn module_dir(m: &str) -> Option<&str> {
    m.rfind('\\').and_then(|i| m.get(..i))
}

/// What LdrpGetDllPath asks for: the elements and the module whose folder ModuleOrApp is
#[derive(Debug, PartialEq, Eq)]
pub struct Plan<'a> {
    pub elems: Vec<Elem>,
    pub module: Option<&'a str>,
}

/// LdrpGetDllPath(module, flags): LOAD_WITH_ALTERED_SEARCH_PATH only alone (it then takes the
/// module's folder first); no flags take SetDefaultDllDirectories'; LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR
/// needs an absolute module (C:\x, UNC, \\?\C:\x), else STATUS_INVALID_PARAMETER, while an
/// altered search with a relative module still takes its folder; DEFAULT_DIRS is the
/// application, user and system32 folders. `safe`: SafeDllSearchMode
pub fn plan(module: Option<&str>, flags: u32, default_dirs: u32, dll_directory_set: bool, safe: bool) -> Result<Plan<'_>, u32> {
    let mut f = flags & !SAFE_CURRENT_DIRS;
    let altered = f & ALTERED != 0;
    if altered {
        if f != ALTERED {
            return Err(STATUS_INVALID_PARAMETER);
        }
        f = 0;
    }
    if f == 0 && default_dirs != 0 {
        f = default_dirs & !SAFE_CURRENT_DIRS;
    }
    let mut module = if f & DLL_LOAD_DIR != 0 || altered { module } else { None };
    if let Some(m) = module {
        let w: Vec<u16> = m.encode_utf16().collect();
        let head = |s: &[u16]| {
            let mut h = [0u16; 4];
            for (o, &u) in h.iter_mut().zip(s) {
                *o = u;
            }
            crate::ntpath::path_type(h)
        };
        use crate::ntpath::PathType as T;
        let absolute = match head(&w) {
            T::UncAbsolute | T::DriveAbsolute => true,
            T::LocalDevice => w.get(2) == Some(&u16::from(b'?')) && head(w.get(4..).unwrap_or(&[])) == T::DriveAbsolute,
            T::DriveRelative | T::Rooted | T::Relative | T::RootLocalDevice => false,
        };
        if !absolute && f != 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }
    }
    if f == 0 {
        let elems = if dll_directory_set { WITH_DLL_DIRECTORY.to_vec() } else if safe { SAFE.to_vec() } else { UNSAFE.to_vec() };
        return Ok(Plan { elems, module });
    }
    if f & DEFAULT_DIRS != 0 {
        f |= APPLICATION_DIR | USER_DIRS | SYSTEM32;
    }
    if f & DLL_LOAD_DIR == 0 {
        module = None;
    }
    Ok(Plan { elems: with_options(f, dll_directory_set), module })
}

/// SetDefaultDllDirectories' folders (0: none set)
static DEFAULT_DLL_DIRECTORIES: AtomicU32 = AtomicU32::new(0);
/// AddDllDirectory's folders, oldest first, with their cookies
static USER_DLL_DIRS: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());
static NEXT_COOKIE: AtomicU32 = AtomicU32::new(0x10);

/// LdrSetDefaultDllDirectories(flags): some of LdrpValidPathComponentsMask's bits but never
/// DLL_LOAD_DIR
pub fn set_default(flags: u32) -> u32 {
    if flags == 0 || flags & (!VALID_COMPONENTS | DLL_LOAD_DIR) != 0 {
        return STATUS_INVALID_PARAMETER;
    }
    DEFAULT_DLL_DIRECTORIES.store(flags, Ordering::Relaxed);
    0
}

/// The folders AddDllDirectory added, oldest first
pub fn user_dirs() -> Vec<String> {
    USER_DLL_DIRS.locked().iter().map(|(_, d)| d.clone()).collect()
}

/// LdrAddDllDirectory(dir): an absolute folder (no "C:x", relative or empty name) that is
/// there, kept as given; its cookie
pub fn add(dir: &[u16]) -> Result<u32, u32> {
    let mut h = [0u16; 4];
    for (o, &u) in h.iter_mut().zip(dir) {
        *o = u;
    }
    use crate::ntpath::PathType as T;
    match crate::ntpath::path_type(h) {
        T::DriveRelative | T::Relative => return Err(STATUS_INVALID_PARAMETER),
        T::UncAbsolute | T::DriveAbsolute | T::Rooted | T::LocalDevice | T::RootLocalDevice => {}
    }
    if dir.is_empty() {
        return Err(STATUS_INVALID_PARAMETER);
    }
    crate::fullpath::process_dos_to_nt(dir)?;
    let s = String::from_utf16_lossy(dir);
    if crate::files::resolve(&s).is_none_or(|p| !crate::files::is_dir(&p)) {
        return Err(0xc000_0034); // STATUS_OBJECT_NAME_NOT_FOUND
    }
    let mut v = USER_DLL_DIRS.locked();
    let total = v.iter().map(|(_, d)| d.encode_utf16().count().saturating_add(1)).sum::<usize>().saturating_add(dir.len()).saturating_add(1);
    if total.saturating_mul(2) > 0xfffe {
        return Err(STATUS_NAME_TOO_LONG);
    }
    let cookie = NEXT_COOKIE.fetch_add(0x10, Ordering::Relaxed);
    v.push((cookie, s));
    Ok(cookie)
}

/// LdrRemoveDllDirectory(cookie); an unknown cookie is STATUS_INVALID_PARAMETER (Windows fails
/// fast on a list entry that is not one)
pub fn remove(cookie: u32) -> u32 {
    let mut v = USER_DLL_DIRS.locked();
    match v.iter().position(|(c, _)| *c == cookie) {
        Some(i) => {
            v.remove(i);
            0
        }
        None => STATUS_INVALID_PARAMETER,
    }
}

/// The exe's folder
pub fn app_dir() -> String {
    let exe = crate::pe::MODULES.r().iter().find(|m| !m.dll).map_or_default(|m| m.path.clone());
    exe.rsplit_once('\\').map_or_default(|(d, _)| d.to_string())
}

fn dll_directory() -> Option<String> {
    crate::pe::DLL_DIRS.locked().first().cloned()
}

fn path_var() -> String {
    crate::k32::env_get("PATH").unwrap_or_default()
}

/// RtlpSearchPathMode's order: ENABLE (1) safe, DISABLE (0x10000) or nothing set (no
/// SafeProcessSearchMode) the current folder first
pub fn search_order() -> [Elem; 5] {
    if crate::k32::search_path_mode() & 1 != 0 { SAFE } else { UNSAFE }
}

/// RtlGetSearchPath's text
pub fn search_path() -> String {
    let (app, user, pv) = (app_dir(), user_dirs(), path_var());
    compute(&search_order(), &Parts { app: &app, module: None, dll_directory: None, user_dirs: &user, path_var: &pv })
}

/// RtlGetExePath(name)'s text: "." only when NeedCurrentDirectoryForExePath
pub fn exe_path(need_cur: bool) -> String {
    let (app, user, pv) = (app_dir(), user_dirs(), path_var());
    let p = Parts { app: &app, module: None, dll_directory: None, user_dirs: &user, path_var: &pv };
    if need_cur { compute(&UNSAFE, &p) } else { compute(&EXE_NO_CUR, &p) }
}

/// LdrGetDllPath's text (SafeDllSearchMode on, its default)
pub fn dll_path(module: Option<&str>, flags: u32) -> Result<String, u32> {
    let dd = dll_directory();
    let plan = plan(module, flags, DEFAULT_DLL_DIRECTORIES.load(Ordering::Relaxed), dd.is_some(), true)?;
    let (app, user, pv) = (app_dir(), user_dirs(), path_var());
    let p = Parts { app: &app, module: plan.module.and_then(module_dir), dll_directory: dd.as_deref(), user_dirs: &user, path_var: &pv };
    Ok(compute(&plan.elems, &p))
}

/// The path's guest copy, as RtlpGetCachedPath hands it out: the text 0x50 bytes into its
/// block (RtlReleasePath frees the block); 0 when out of memory
fn hand_out(s: &str) -> u32 {
    let w: Vec<u16> = s.encode_utf16().chain([0]).collect();
    let bytes = kome::num::u32_of(w.len().saturating_mul(2));
    let block = mem::heap_alloc(bytes.saturating_add(0x50), true);
    if block == 0 {
        return 0;
    }
    let at = block.wrapping_add(0x50);
    for (i, &u) in w.iter().enumerate() {
        mem::w16(at.wrapping_add(kome::num::u32_of(i.saturating_mul(2))), u);
    }
    at
}

fn put_path(out: u32, s: &str) -> u32 {
    match hand_out(s) {
        0 => 0xc000_0017, // STATUS_NO_MEMORY
        p => {
            mem::wp(out, p);
            0
        }
    }
}

fn RtlGetSearchPath(c: &mut Ctx) -> u32 {
    put_path(c.a(0), &search_path())
}

fn RtlGetExePath(c: &mut Ctx) -> u32 {
    let name = if c.a(0) == 0 { Vec::new() } else { mem::wstr(c.a(0)) };
    put_path(c.a(1), &exe_path(crate::k32::need_cur_dir(&name) != 0))
}

fn RtlReleasePath(c: &mut Ctx) -> u32 {
    if c.a(0) != 0 {
        mem::heap_free(c.a(0).wrapping_sub(0x50));
    }
    0
}

fn LdrGetDllPath(c: &mut Ctx) -> u32 {
    if c.a(3) != 0 {
        mem::wp(c.a(3), 0);
    }
    let module = (c.a(0) != 0).then(|| String::from_utf16_lossy(&mem::wstr(c.a(0))));
    match dll_path(module.as_deref(), c.a(1)) {
        Ok(s) => put_path(c.a(2), &s),
        Err(st) => st,
    }
}

/// LdrAddDllDirectory(&UNICODE_STRING, &cookie)
fn LdrAddDllDirectory(c: &mut Ctx) -> u32 {
    let s = crate::rtlstr::Counted::read(c.a(0)).units();
    match add(&s) {
        Ok(k) => {
            mem::wp(c.a(1), k);
            0
        }
        Err(st) => st,
    }
}

fn LdrRemoveDllDirectory(c: &mut Ctx) -> u32 {
    remove(c.a(0))
}

fn LdrSetDefaultDllDirectories(c: &mut Ctx) -> u32 {
    set_default(c.a(0))
}

fn win32(st: u32) -> u32 {
    if st == 0 {
        1
    } else {
        last_error(crate::nt::dos_error(st));
        0
    }
}

/// AddDllDirectory(dir): the cookie, 0 with the error
fn AddDllDirectory(c: &mut Ctx) -> u32 {
    let s = if c.a(0) == 0 { Vec::new() } else { mem::wstr(c.a(0)) };
    match add(&s) {
        Ok(k) => k,
        Err(st) => {
            last_error(crate::nt::dos_error(st));
            0
        }
    }
}

fn RemoveDllDirectory(c: &mut Ctx) -> u32 {
    win32(remove(c.a(0)))
}

fn SetDefaultDllDirectories(c: &mut Ctx) -> u32 {
    win32(set_default(c.a(0)))
}

pub fn init() {
    use crate::api::implement;
    let ntdll: [(&str, crate::api::Handler); 7] = [
        ("RtlGetSearchPath", RtlGetSearchPath),
        ("RtlGetExePath", RtlGetExePath),
        ("RtlReleasePath", RtlReleasePath),
        ("LdrGetDllPath", LdrGetDllPath),
        ("LdrAddDllDirectory", LdrAddDllDirectory),
        ("LdrRemoveDllDirectory", LdrRemoveDllDirectory),
        ("LdrSetDefaultDllDirectories", LdrSetDefaultDllDirectories),
    ];
    for (n, f) in ntdll {
        implement("ntdll", n, f);
    }
    for d in ["kernel32", "kernelbase"] {
        implement(d, "AddDllDirectory", AddDllDirectory);
        implement(d, "RemoveDllDirectory", RemoveDllDirectory);
        implement(d, "SetDefaultDllDirectories", SetDefaultDllDirectories);
    }
}

#[cfg(test)]
#[path = "dllpath_tests.rs"]
mod tests;
