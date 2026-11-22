//! ImageHlp's MapAndLoad / UnMapAndLoad / ImageLoad / ImageUnload: a PE file mapped flat (as
//! the file is, not laid out as an image) and described by a LOADED_IMAGE. KiriKiri's
//! signed-plugin check (lavender.tpm, akabeesoft2's 光輪の町、ラベンダーの少女) ImageLoads the
//! game exe and walks its section table.
//! The view is a copy of the file in guest heap memory (sake's file mappings are copies too):
//! with ReadOnly FALSE, changes made to it are not written back to the file.
#![allow(non_snake_case)]
use kome::locks::Rw as _;
use crate::api::{implement, Ctx};
use crate::mem::{self, r32, w32};

/// What a LOADED_IMAGE says about a mapped PE file, from its bytes: the offset of the NT
/// headers, of the section table, the number of sections, the file header's Characteristics and
/// the optional header's SizeOfImage. None: not a PE file (ERROR_BAD_EXE_FORMAT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub nt: u32,
    pub sections: u32,
    pub count: u32,
    pub characteristics: u32,
    pub size_of_image: u32,
}

pub fn layout(data: &[u8]) -> Option<Layout> {
    let word = |o: usize| Some(u16::from_le_bytes(data.get(o..o.checked_add(2)?)?.try_into().ok()?));
    let dword = |o: usize| Some(u32::from_le_bytes(data.get(o..o.checked_add(4)?)?.try_into().ok()?));
    if data.get(..2)? != b"MZ" {
        return None;
    }
    let nt = dword(0x3c)? as usize;
    if data.get(nt..nt.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    let count = word(nt + 6)? as usize;
    let opt_size = word(nt + 20)? as usize;
    let opt = nt + 24;
    // SizeOfImage: at 56 in both PE32 and PE32+
    let size_of_image = match word(opt)? {
        0x10b | 0x20b if opt_size >= 60 => dword(opt + 56)?,
        _ => return None,
    };
    let sections = opt.checked_add(opt_size)?;
    // (the whole section table inside the file)
    sections.checked_add(count.checked_mul(40)?).filter(|&end| end <= data.len())?;
    Some(Layout { nt: nt as u32, sections: sections as u32, count: count as u32, characteristics: word(nt + 22)? as u32, size_of_image })
}

/// The file `name` is looked for as MapAndLoad does: as given when it has a path of its own,
/// else in `dll_path` (';'-separated) when given, else the exe's folder, the current folder,
/// system32 and the Windows folder. `ext` is added to a name without one.
fn find(name: &str, dll_path: &str, ext: &str) -> Option<String> {
    let mut n = name.to_string();
    if !n.rsplit(['\\', '/']).next().unwrap_or("").contains('.') {
        n += ext;
    }
    let b = name.as_bytes();
    let sep = |ch: u8| ch == b'\\' || ch == b'/';
    let own = !b.is_empty() && (sep(b[0]) || (b.len() > 1 && b[1] == b':') || name.contains(['\\', '/']));
    let dirs: Vec<String> = if own {
        vec![String::new()]
    } else if !dll_path.is_empty() {
        dll_path.split(';').filter(|d| !d.is_empty()).map(String::from).collect()
    } else {
        let exe = crate::pe::MODULES.r().iter().find(|m| !m.dll).map_or_default(|m| m.path.clone());
        let exe_dir = exe.rfind('\\').map_or_default(|i| exe[..i].to_string());
        vec![exe_dir, crate::files::cwd(), "C:\\windows\\system32".into(), "C:\\windows".into()]
    };
    dirs.iter().map(|d| crate::files::full_path(&if own { n.clone() } else { format!("{}\\{n}", d.trim_end_matches('\\')) }))
        .find(|w| crate::files::resolve(w).is_some_and(|h| !crate::files::is_dir(&h)))
}

/// LOADED_IMAGE's field offsets (pointer size p): ModuleName 0, hFile p, MappedAddress 2p,
/// FileHeader 3p, LastRvaSection 4p, NumberOfSections 5p, Sections 6p, Characteristics 7p,
/// fSystemImage 7p+4, fDOSImage +5, fReadOnly +6, Version +7, Links (2p) after them aligned to p
/// (x86 36, x64 64), SizeOfImage after Links (x86 44, x64 80).
fn ptr() -> u32 {
    if mem::x64() { 8 } else { 4 }
}
pub fn loaded_image_size() -> u32 {
    if mem::x64() { 88 } else { 48 }
}

/// Fills the LOADED_IMAGE at `li` for the file `name`: false with the error set on failure.
fn map_and_load(name: &str, dll_path: &str, li: u32, dot_dll: bool, read_only: bool) -> bool {
    let Some(w) = find(name, dll_path, if dot_dll { ".dll" } else { ".exe" }) else {
        crate::k32::last_error(2);
        return false;
    };
    let Some(mut f) = crate::files::resolve(&w).and_then(|h| kura::fs::File::open(&h).ok()) else {
        crate::k32::last_error(2);
        return false;
    };
    // (read straight into the view: no host copy of the whole file)
    let len = f.metadata().map_or(0, |m| m.len());
    let Ok(n) = u32::try_from(len) else {
        crate::k32::last_error(8);
        return false;
    };
    let view = mem::heap_alloc(n.max(1), false);
    let path = crate::text::s2a(&w);
    let module = mem::heap_alloc(path.len() as u32 + 1, true);
    let free = || {
        mem::heap_free(view);
        mem::heap_free(module);
    };
    if view == 0 || module == 0 {
        free();
        crate::k32::last_error(8);
        return false;
    }
    let mut done = 0u32;
    while done < n {
        let k = (n - done).min(1 << 20);
        let mut b = vec![0u8; k as usize];
        if std::io::Read::read_exact(&mut f, &mut b).is_err() {
            free();
            crate::k32::last_error(30); // ERROR_READ_FAULT
            return false;
        }
        mem::write(view + done, &b);
        done += k;
    }
    let Some(l) = layout(&mem::bytes(view, n)) else {
        free();
        crate::k32::last_error(193);
        return false;
    };
    mem::write(module, &path);
    let p = ptr();
    mem::fill(li, loaded_image_size(), 0);
    mem::putp(li, module);
    mem::putp(li + p, u32::MAX); // (no file handle kept: the view is a copy)
    mem::putp(li + 2 * p, view);
    mem::putp(li + 3 * p, view + l.nt);
    mem::putp(li + 4 * p, view + l.sections);
    w32(li + 5 * p, l.count);
    mem::putp(li + 6 * p, view + l.sections);
    w32(li + 7 * p, l.characteristics);
    mem::w8(li + 7 * p + 6, read_only as u8);
    mem::w8(li + 7 * p + 7, 1);
    // Links: an empty list (Flink = Blink = itself)
    let links = li + (7 * p + 8).next_multiple_of(p);
    mem::putp(links, links);
    mem::putp(links + p, links);
    w32(links + 2 * p, l.size_of_image);
    true
}

fn unmap(li: u32) {
    let p = ptr();
    let rp = |a: u32| if p == 8 { mem::r64(a) as u32 } else { r32(a) };
    mem::heap_free(rp(li + 2 * p));
    mem::heap_free(rp(li));
    mem::putp(li, 0);
    mem::putp(li + 2 * p, 0);
}

fn arg(a: u32) -> String {
    if a == 0 { String::new() } else { crate::text::a2s(&mem::cstr(a)) }
}

fn MapAndLoad(c: &mut Ctx) -> u32 {
    if c.a(0) == 0 || c.a(2) == 0 {
        crate::k32::last_error(87);
        return 0;
    }
    map_and_load(&arg(c.a(0)), &arg(c.a(1)), c.a(2), c.a(3) != 0, c.a(4) != 0) as u32
}
fn UnMapAndLoad(c: &mut Ctx) -> u32 {
    if c.a(0) == 0 {
        return 0;
    }
    unmap(c.a(0));
    1
}
/// ImageLoad(DllName, DllPath): MapAndLoad with ".dll" added and read-only, into a
/// LOADED_IMAGE it allocates (ImageUnload frees both)
fn ImageLoad(c: &mut Ctx) -> u32 {
    if c.a(0) == 0 {
        crate::k32::last_error(87);
        return 0;
    }
    let li = mem::heap_alloc(loaded_image_size(), true);
    if li == 0 {
        crate::k32::last_error(8);
        return 0;
    }
    if !map_and_load(&arg(c.a(0)), &arg(c.a(1)), li, true, true) {
        mem::heap_free(li);
        return 0;
    }
    li
}
fn ImageUnload(c: &mut Ctx) -> u32 {
    let li = c.a(0);
    if li == 0 || !mem::heap_owns(li) {
        crate::k32::last_error(87);
        return 0;
    }
    unmap(li);
    mem::heap_free(li);
    1
}

/// CheckSumMappedFile's sum of `data`: 16-bit little-endian words (an odd last byte alone)
/// added with their carries folded back in; then, when `stored` (the image's CheckSum field) is
/// given, its two halves taken out again (borrows folded the same way); then the length added
pub fn pe_checksum(data: &[u8], stored: Option<u32>) -> u32 {
    let mut sum: u32 = 0;
    for w in data.chunks(2) {
        let v = u32::from(*w.first().unwrap_or(&0)) | u32::from(*w.get(1).unwrap_or(&0)) << 8;
        sum = sum.wrapping_add(v);
        sum = (sum & 0xffff).wrapping_add(sum >> 16);
    }
    if let Some(cs) = stored {
        for half in [cs & 0xffff, cs >> 16] {
            if sum & 0xffff < half {
                sum = sum.wrapping_sub(1);
            }
            sum = sum.wrapping_sub(half) & 0xffff;
        }
    }
    (sum & 0xffff).wrapping_add(kome::num::u32_of(data.len()))
}

/// Where `data`'s optional header CheckSum field is, from its NT headers inside `data`: (NT
/// headers' offset, the field's offset)
pub fn checksum_field(data: &[u8]) -> Option<(u32, u32)> {
    // (RtlImageNtHeader's test: "MZ", then "PE\0\0" at e_lfanew; the section table need not
    // be there)
    if data.get(..2)? != b"MZ" {
        return None;
    }
    let nt = u32::from_le_bytes(data.get(0x3c..0x40)?.try_into().ok()?);
    let n = usize::try_from(nt).ok()?;
    if data.get(n..n.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    // (CheckSum: 64 bytes into the optional header, which starts 24 bytes into the NT headers)
    let at = nt.checked_add(24 + 64)?;
    data.get(usize::try_from(at).ok()?..usize::try_from(at.checked_add(4)?).ok()?)?;
    Some((nt, at))
}

/// CheckSumMappedFile(base, length, &HeaderSum, &CheckSum): the NT headers (NULL for no PE
/// image in those bytes, HeaderSum 0 then); an empty view or none is ERROR_INVALID_PARAMETER
fn CheckSumMappedFile(c: &mut Ctx) -> u32 {
    let (base, len, header_sum, sum) = (c.a(0), c.a(1), c.a(2), c.a(3));
    if base == 0 || len == 0 || mem::accessible(base, len, false, true) < len {
        crate::k32::last_error(87);
        return 0;
    }
    let data = mem::bytes(base, len);
    let field = checksum_field(&data);
    let stored = field.and_then(|(_, at)| data.get(usize::try_from(at).ok()?..).and_then(|d| d.get(..4)).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes));
    w32(header_sum, stored.unwrap_or(0));
    w32(sum, pe_checksum(&data, stored));
    field.map_or(0, |(nt, _)| base.wrapping_add(nt))
}

pub fn init() {
    for dll in ["imagehlp", "dbghelp"] {
        implement(dll, "ImageNtHeader", crate::pe::RtlImageNtHeader);
    }
    implement("imagehlp", "CheckSumMappedFile", CheckSumMappedFile);
    for dll in ["imagehlp", "dbghelp"] {
        implement(dll, "MapAndLoad", MapAndLoad);
        implement(dll, "UnMapAndLoad", UnMapAndLoad);
    }
    implement("imagehlp", "ImageLoad", ImageLoad);
    implement("imagehlp", "ImageUnload", ImageUnload);
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Windows' answers (Wine imagehlp:integrity): no PE header, so the plain sum plus length
    #[test]
    fn checksum_of_plain_bytes() {
        assert_eq!(pe_checksum(&[0x11; 20], None), 0xaabe);
        assert_eq!(pe_checksum(&[0x22; 20], None), 0x5569);
        assert_eq!(pe_checksum(&[0x22; 10], None), 0xaab4);
        assert_eq!(pe_checksum(&[0x22; 11], None), 0xaad7);
    }

    proptest! {
        /// the stored CheckSum taken out of a sum that holds it gives the sum without it
        #[test]
        fn stored_checksum_cancels(mut d in proptest::collection::vec(any::<u8>(), 8..64), cs in any::<u32>()) {
            let without = { let mut z = d.clone(); z[4..8].fill(0); pe_checksum(&z, None) };
            d[4..8].copy_from_slice(&cs.to_le_bytes());
            prop_assert_eq!(pe_checksum(&d, Some(cs)), without);
        }
    }

    /// A PE file: DOS header with e_lfanew `nt`, `count` sections, optional header of `opt`
    /// bytes (magic `magic`), `tail` more bytes.
    fn pe(nt: u32, count: u16, opt: u16, magic: u16, chars: u16, soi: u32, tail: usize) -> Vec<u8> {
        let end = nt as usize + 24 + opt as usize + 40 * count as usize + tail;
        let mut d = vec![0u8; end.max(0x40)];
        d[..2].copy_from_slice(b"MZ");
        d[0x3c..0x40].copy_from_slice(&nt.to_le_bytes());
        let n = nt as usize;
        d[n..n + 4].copy_from_slice(b"PE\0\0");
        d[n + 6..n + 8].copy_from_slice(&count.to_le_bytes());
        d[n + 20..n + 22].copy_from_slice(&opt.to_le_bytes());
        d[n + 22..n + 24].copy_from_slice(&chars.to_le_bytes());
        d[n + 24..n + 26].copy_from_slice(&magic.to_le_bytes());
        if opt >= 60 {
            d[n + 24 + 56..n + 24 + 60].copy_from_slice(&soi.to_le_bytes());
        }
        d
    }

    #[test]
    fn pe32_and_pe32plus() {
        let d = pe(0x80, 3, 0xe0, 0x10b, 0x102, 0x5000, 16);
        assert_eq!(layout(&d), Some(Layout { nt: 0x80, sections: 0x80 + 24 + 0xe0, count: 3, characteristics: 0x102, size_of_image: 0x5000 }));
        let d = pe(0x40, 1, 0xf0, 0x20b, 0x22, 0x9000, 0);
        assert_eq!(layout(&d).map(|l| (l.sections, l.size_of_image)), Some((0x40 + 24 + 0xf0, 0x9000)));
    }

    #[test]
    fn not_pe() {
        assert_eq!(layout(b""), None);
        assert_eq!(layout(b"MZ"), None);
        let mut d = pe(0x80, 1, 0xe0, 0x10b, 0, 0, 0);
        d[0x80] = b'N';
        assert_eq!(layout(&d), None);
        // (ROM / unknown optional header magic, too small an optional header)
        assert_eq!(layout(&pe(0x80, 1, 0xe0, 0x107, 0, 0, 0)), None);
        assert_eq!(layout(&pe(0x80, 1, 40, 0x10b, 0, 0, 0)), None);
        // the section table cut short by the end of the file
        let d = pe(0x80, 4, 0xe0, 0x10b, 0, 0, 0);
        assert_eq!(layout(&d[..d.len() - 1]), None);
    }

    proptest! {
        /// Any bytes: no panic, and what comes back lies inside the data.
        #[test]
        fn any_bytes(d in proptest::collection::vec(any::<u8>(), 0..600), lfanew in 0u32..700, mz in any::<bool>(), pe_sig in any::<bool>()) {
            let mut d = d;
            if mz && d.len() >= 0x40 {
                d[..2].copy_from_slice(b"MZ");
                d[0x3c..0x40].copy_from_slice(&lfanew.to_le_bytes());
                let n = lfanew as usize;
                if pe_sig && n + 4 <= d.len() { d[n..n + 4].copy_from_slice(b"PE\0\0"); }
            }
            if let Some(l) = layout(&d) {
                prop_assert!(l.sections as usize + 40 * l.count as usize <= d.len());
                prop_assert_eq!(&d[l.nt as usize..l.nt as usize + 4], b"PE\0\0");
            }
        }

        /// Well-formed files give their own fields back.
        #[test]
        fn round_trip(nt in 0x40u32..0x400, count in 0u16..40, plus in any::<bool>(), extra in 0u16..64, chars in any::<u16>(), soi in any::<u32>(), tail in 0usize..64) {
            let (magic, opt) = if plus { (0x20b, 0xf0 + extra) } else { (0x10b, 0xe0 + extra) };
            let d = pe(nt, count, opt, magic, chars, soi, tail);
            prop_assert_eq!(layout(&d), Some(Layout { nt, sections: nt + 24 + opt as u32, count: count as u32, characteristics: chars as u32, size_of_image: soi }));
        }
    }
}
