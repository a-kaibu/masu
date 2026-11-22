//! The files of the system DLLs sake implements: Windows has C:\windows\system32\kernel32.dll
//! and the rest as files that programs open, copy and read (Wine's kernel32:path copies
//! kernel32.dll; ntdll:path opens it by NT name). sake has no such files of its own, so one is
//! made in the guest's system32 the first time a lookup misses it: a PE image of this process's
//! machine with no code (sake's own DLL stays the one that loads: pe.rs never takes a known
//! system DLL from a system folder), dated as Windows 10's files are.

/// The lower-case file name of `w` when it names a DLL sake implements in system32 (or
/// SysWOW64, Sysnative: files::wow64_system)
pub fn name(w: &str) -> Option<String> {
    let full = crate::files::wow64_system(&crate::files::full_path(w)).to_ascii_lowercase();
    let n = full.strip_prefix("c:\\windows\\system32\\")?;
    let d = n.strip_suffix(".dll")?;
    (!d.contains('\\') && crate::api::is_system(d)).then(|| n.to_string())
}

/// When they were written (2019-12-07 09:09:00 UTC, as Windows 10's files carry)
pub const TIME: u64 = 132_201_833_400_000_000;

/// FileAlignment: the headers' and the section's size in the file
const FILE_ALIGN: u32 = 0x200;

/// A PE DLL of `x64`'s machine: headers and one empty read-only section, no entry point
pub fn image(x64: bool) -> Vec<u8> {
    let mut f: Vec<u8> = Vec::with_capacity(0x400);
    let w16 = |f: &mut Vec<u8>, v: u16| f.extend_from_slice(&v.to_le_bytes());
    let w32 = |f: &mut Vec<u8>, v: u32| f.extend_from_slice(&v.to_le_bytes());
    // DOS header: "MZ", e_lfanew 0x40
    f.extend_from_slice(b"MZ");
    f.resize(0x3c, 0);
    w32(&mut f, 0x40);
    f.extend_from_slice(b"PE\0\0");
    // IMAGE_FILE_HEADER: machine, 1 section, no symbols, the optional header's size, EXECUTABLE |
    // DLL with 32BIT_MACHINE (x86) or LARGE_ADDRESS_AWARE (x64)
    let (machine, opt_size, chars, magic): (u16, u16, u16, u16) = if x64 { (0x8664, 0xf0, 0x2022, 0x20b) } else { (0x14c, 0xe0, 0x2102, 0x10b) };
    w16(&mut f, machine);
    w16(&mut f, 1);
    f.extend_from_slice(&[0; 12]);
    w16(&mut f, opt_size);
    w16(&mut f, chars);
    // optional header: magic, linker 14.0, no code, no entry point, BaseOfCode 0x1000
    w16(&mut f, magic);
    f.extend_from_slice(&[14, 0]);
    f.extend_from_slice(&[0; 16]);
    w32(&mut f, 0x1000);
    if x64 {
        f.extend_from_slice(&0x1_8000_0000u64.to_le_bytes());
    } else {
        w32(&mut f, 0x1000); // BaseOfData
        w32(&mut f, 0x1000_0000);
    }
    w32(&mut f, 0x1000); // SectionAlignment
    w32(&mut f, FILE_ALIGN);
    for v in [10u16, 0, 10, 0, 10, 0] {
        w16(&mut f, v); // OS, image, subsystem versions
    }
    w32(&mut f, 0); // Win32VersionValue
    w32(&mut f, 0x2000); // SizeOfImage
    w32(&mut f, FILE_ALIGN); // SizeOfHeaders
    w32(&mut f, 0); // CheckSum
    w16(&mut f, 3); // WINDOWS_CUI
    w16(&mut f, 0x0140); // DYNAMIC_BASE | NX_COMPAT
    for v in [0x4_0000u64, 0x1000, 0x10_0000, 0x1000] {
        // stack and heap reserve / commit: pointer-sized
        if x64 { f.extend_from_slice(&v.to_le_bytes()) } else { w32(&mut f, kome::num::lo32(v)) }
    }
    w32(&mut f, 0); // LoaderFlags
    w32(&mut f, 16); // NumberOfRvaAndSizes
    f.extend_from_slice(&[0; 128]);
    // the section: ".rdata", 0x10 bytes at 0x1000, its file bytes after the headers, read-only
    // initialized data
    f.extend_from_slice(b".rdata\0\0");
    w32(&mut f, 0x10);
    w32(&mut f, 0x1000);
    w32(&mut f, FILE_ALIGN);
    w32(&mut f, FILE_ALIGN);
    f.extend_from_slice(&[0; 12]);
    w32(&mut f, 0x4000_0040);
    f.resize(0x400, 0);
    f
}

/// The host file of system DLL `w` when it has none yet: made (see the module's notes)
pub fn make(w: &str) -> Option<String> {
    let n = name(w)?;
    let dir = crate::files::make_dirs("C:\\windows\\system32");
    let h = format!("{}/{n}", dir.trim_end_matches('/'));
    kura::fs::write(&h, image(crate::mem::x64())).ok()?;
    crate::files::set_write_time(&h, TIME);
    Some(h)
}

#[cfg(test)]
#[path = "sysdll_tests.rs"]
mod tests;
