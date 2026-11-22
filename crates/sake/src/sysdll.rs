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
    image_with(x64, &[])
}

/// The file of system DLL `name` (lower case, "iphlpapi.dll"): image() whose section is a
/// resource section holding its VS_VERSIONINFO (misc::system_version_info: OriginalFilename
/// is `name`), as Windows' files carry one (Wine version:info copies iphlpapi.dll and reads it)
pub fn image_of(x64: bool, name: &str) -> Vec<u8> {
    image_with(x64, &resources(&crate::misc::system_version_info(name)))
}

/// A resource section at RVA 0x1000 holding one RT_VERSION resource (id 1, language 0x411):
/// the root, type and name directories, the data entry, then the data
pub(crate) fn resources(version: &[u8]) -> Vec<u8> {
    const RVA: u32 = 0x1000;
    let dir = |v: &mut Vec<u8>, id: u32, to: u32| {
        // IMAGE_RESOURCE_DIRECTORY: characteristics, time, version, no named entries, 1 id entry
        v.extend_from_slice(&[0; 12]);
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&id.to_le_bytes());
        v.extend_from_slice(&to.to_le_bytes());
    };
    let mut v = Vec::with_capacity(version.len().saturating_add(0x60));
    // (each directory is 24 bytes: the next one, or the data entry, follows it)
    dir(&mut v, 16, 0x8000_0018);
    dir(&mut v, 1, 0x8000_0030);
    dir(&mut v, 0x411, 0x48);
    // IMAGE_RESOURCE_DATA_ENTRY: the data's RVA, size, code page 0
    v.extend_from_slice(&RVA.saturating_add(0x58).to_le_bytes());
    v.extend_from_slice(&kome::num::u32_of(version.len()).to_le_bytes());
    v.extend_from_slice(&[0; 8]);
    v.extend_from_slice(version);
    v
}

/// image() with `section` as its one section's bytes (a resource section when not empty)
fn image_with(x64: bool, section: &[u8]) -> Vec<u8> {
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
    // data directories: only IMAGE_DIRECTORY_ENTRY_RESOURCE (2), when there is a section
    let len = kome::num::u32_of(section.len());
    f.extend_from_slice(&[0; 16]);
    if section.is_empty() { f.extend_from_slice(&[0; 8]) } else { w32(&mut f, 0x1000); w32(&mut f, len) }
    f.extend_from_slice(&[0; 104]);
    // the section: ".rdata" (0x10 bytes) or ".rsrc" at 0x1000, its file bytes after the headers,
    // read-only initialized data
    let raw = len.max(0x10).div_ceil(FILE_ALIGN).saturating_mul(FILE_ALIGN);
    f.extend_from_slice(if section.is_empty() { b".rdata\0\0" } else { b".rsrc\0\0\0" });
    w32(&mut f, len.max(0x10));
    w32(&mut f, 0x1000);
    w32(&mut f, raw);
    w32(&mut f, FILE_ALIGN);
    f.extend_from_slice(&[0; 12]);
    w32(&mut f, 0x4000_0040);
    f.resize(kome::num::us(FILE_ALIGN), 0);
    f.extend_from_slice(section);
    f.resize(kome::num::us(FILE_ALIGN.saturating_add(raw)), 0);
    f
}

/// The host file of system DLL `w` when it has none yet: made (see the module's notes)
pub fn make(w: &str) -> Option<String> {
    let n = name(w)?;
    let dir = crate::files::make_dirs("C:\\windows\\system32");
    let h = format!("{}/{n}", dir.trim_end_matches('/'));
    kura::fs::write(&h, image_of(crate::mem::x64(), &n)).ok()?;
    crate::files::set_write_time(&h, TIME);
    Some(h)
}

#[cfg(test)]
#[path = "sysdll_tests.rs"]
mod tests;
