//! ImageHlp certificate-table access. These functions read actual WIN_CERTIFICATE
//! records; they do not assert that a certificate or file signature is trusted.
#![allow(non_snake_case)]
use kome::locks::Lock as _;
use std::io::{Read, Seek, SeekFrom, Write};
use crate::api::{implement, Ctx};
use crate::mem::{self, r32, w32};

/// Reads ranges of an image file: `at(offset, n)` the n bytes there (None past the end or on
/// an error), `len` its size. A file is read range by range, not whole: the certificate calls
/// need its headers and the certificate table, the digest its headers and one part at a time
/// (a game exe can be hundreds of MB).
pub trait Image {
    fn len(&mut self) -> u64;
    fn at(&mut self, off: u64, n: usize) -> Option<Vec<u8>>;
    /// The n bytes at `off` into guest memory at `dst` (false past the end or on an error)
    fn to_guest(&mut self, off: u64, n: u32, dst: u32) -> bool {
        // (in 1 MiB steps: no host copy of a whole section)
        let mut done = 0u32;
        while done < n {
            let k = (n - done).min(1 << 20);
            let Some(b) = self.at(off + done as u64, k as usize) else { return false };
            mem::write(dst + done, &b);
            done += k;
        }
        true
    }
}
impl Image for &[u8] {
    fn len(&mut self) -> u64 {
        <[u8]>::len(self) as u64
    }
    fn at(&mut self, off: u64, n: usize) -> Option<Vec<u8>> {
        let o = usize::try_from(off).ok()?;
        self.get(o..o.checked_add(n)?).map(<[u8]>::to_vec)
    }
}
struct FileImage<'a>(&'a mut kura::fs::File);
impl Image for FileImage<'_> {
    fn len(&mut self) -> u64 {
        self.0.metadata().map_or(0, |m| m.len())
    }
    fn at(&mut self, off: u64, n: usize) -> Option<Vec<u8>> {
        if off.checked_add(n as u64)? > self.len() {
            return None;
        }
        self.0.seek(SeekFrom::Start(off)).ok()?;
        let mut b = vec![0; n];
        self.0.read_exact(&mut b).ok()?;
        Some(b)
    }
}

/// The headers of an image: its first bytes through the section table (None: not one)
fn headers(img: &mut dyn Image) -> Option<Vec<u8>> {
    let len = img.len();
    let first = img.at(0, len.min(0x1000) as usize)?;
    let dword = |d: &[u8], o: usize| Some(u32::from_le_bytes(d.get(o..o.checked_add(4)?)?.try_into().ok()?));
    let word = |d: &[u8], o: usize| Some(u16::from_le_bytes(d.get(o..o.checked_add(2)?)?.try_into().ok()?));
    let pe = dword(&first, 0x3c)? as u64;
    // through the optional header's size field and section count: then the whole table
    let fixed = img.at(pe, 24).or_else(|| img.at(pe, len.saturating_sub(pe).min(24) as usize))?;
    let (count, optional) = (word(&fixed, 6).unwrap_or(0) as u64, word(&fixed, 20).unwrap_or(0) as u64);
    let end = (pe + 24 + optional + 40 * count).max(pe + 24 + 112 + 8 * 16).max(first.len() as u64).min(len);
    img.at(0, end as usize)
}

fn certificates(img: &mut dyn Image) -> Option<Vec<Vec<u8>>> {
    let data = headers(img)?;
    let word = |o: usize| Some(u16::from_le_bytes(data.get(o..o.checked_add(2)?)?.try_into().ok()?));
    let dword = |o: usize| Some(u32::from_le_bytes(data.get(o..o.checked_add(4)?)?.try_into().ok()?));
    if data.get(..2)? != b"MZ" { return None; }
    let pe = dword(0x3c)? as usize;
    if data.get(pe..pe.checked_add(4)?)? != b"PE\0\0" { return None; }
    let opt = pe.checked_add(24)?;
    let dirs = match word(opt)? { 0x10b => 96, 0x20b => 112, _ => return None };
    if (word(pe + 20)? as usize) < dirs + 40 || dword(opt + dirs - 4)? < 5 { return Some(vec![]); }
    let (at, size) = (dword(opt + dirs + 32)? as u64, dword(opt + dirs + 36)? as usize);
    if at == 0 && size == 0 { return Some(vec![]); }
    let table = img.at(at, size)?;
    let mut result = Vec::new(); let mut at = 0usize;
    while at < table.len() {
        let n = u32::from_le_bytes(table.get(at..at.checked_add(4)?)?.try_into().ok()?) as usize;
        if n < 8 { return None; }
        result.push(table.get(at..at.checked_add(n)?)?.to_vec());
        at = at.checked_add(n)?.checked_add(7)? & !7;
        if at > table.len() { return None; }
    }
    Some(result)
}
/// `f` on the open file `h` read as an image, its position kept
fn with_file<R>(h: u32, f: impl FnOnce(&mut dyn Image) -> R) -> Result<R, u32> {
    let file = crate::obj::file(h).ok_or(6u32)?;
    let mut g = file.f.locked();
    let position = g.stream_position().map_err(|_| 87u32)?;
    let r = f(&mut FileImage(&mut g));
    g.seek(SeekFrom::Start(position)).map_err(|_| 87u32)?;
    Ok(r)
}
fn fail(error: u32) -> u32 { crate::k32::last_error(error); 0 }
fn entries_of(h: u32) -> Result<Vec<Vec<u8>>, u32> {
    with_file(h, |img| certificates(img))?.ok_or(193)
}
fn ImageGetCertificateHeader(c: &mut Ctx) -> u32 {
    if c.a(2) == 0 { return fail(87); }
    let entries = match entries_of(c.a(0)) { Ok(e) => e, Err(e) => return fail(e) };
    let Some(certificate) = entries.get(c.a(1) as usize) else { return fail(87) };
    mem::write(c.a(2), &certificate[..8]); 1
}
fn ImageGetCertificateData(c: &mut Ctx) -> u32 {
    if c.a(3) == 0 { return fail(87); }
    let entries = match entries_of(c.a(0)) { Ok(e) => e, Err(e) => return fail(e) };
    let Some(certificate) = entries.get(c.a(1) as usize) else { return fail(87) };
    let capacity = r32(c.a(3)); w32(c.a(3), certificate.len() as u32);
    if c.a(2) == 0 || capacity < certificate.len() as u32 { return fail(122); }
    mem::write(c.a(2), certificate); 1
}
fn ImageEnumerateCertificates(c: &mut Ctx) -> u32 {
    if c.a(2) == 0 { return fail(87); }
    let entries = match entries_of(c.a(0)) { Ok(e) => e, Err(e) => return fail(e) };
    let filter = c.a(1) as u16;
    let indexes: Vec<u32> = entries.iter().enumerate().filter(|(_, b)| filter == 0xff || u16::from_le_bytes([b[6],b[7]]) == filter)
        .map(|(i, _)| i as u32).collect();
    w32(c.a(2), indexes.len() as u32);
    if c.a(3) != 0 {
        if c.a(4) < indexes.len() as u32 { return fail(122); }
        for (i, value) in indexes.into_iter().enumerate() { w32(c.a(3) + i as u32 * 4, value); }
    }
    1
}

/// A piece of the digest stream: bytes made from the headers, or a range of the file
#[derive(Debug, PartialEq)]
pub enum Part { Bytes(Vec<u8>), File(u64, u32) }

/// ImageHlp's stream is not the Authenticode flat-file hashing algorithm:
/// DOS/NT/section headers precede code sections, followed by other sections.
/// Normalize only checksum and the security-directory entry for level 6.
/// Other levels need additional import/resource/debug normalization; reject
/// them until independently verified instead of supplying an incorrect hash.
/// Every range is checked against the file before any part is handed out.
pub fn digest_parts(img: &mut dyn Image, level: u32) -> Result<Vec<Part>, u32> {
    if level != 6 { return Err(50); }
    let invalid = 193u32;
    let data = headers(img).ok_or(invalid)?;
    let len = img.len();
    let word = |o: usize| -> Option<u16> { Some(u16::from_le_bytes(data.get(o..o.checked_add(2)?)?.try_into().ok()?)) };
    let dword = |o: usize| -> Option<u32> { Some(u32::from_le_bytes(data.get(o..o.checked_add(4)?)?.try_into().ok()?)) };
    if data.get(..2) != Some(b"MZ") { return Err(invalid); }
    let pe = dword(0x3c).ok_or(invalid)? as usize;
    if pe < 64 || data.get(pe..pe.checked_add(4).ok_or(invalid)?) != Some(b"PE\0\0") { return Err(invalid); }
    let opt = pe.checked_add(24).ok_or(invalid)?;
    let optional_size = word(pe + 20).ok_or(invalid)? as usize;
    let dirs = match word(opt).ok_or(invalid)? { 0x10b => 96, 0x20b => 112, _ => return Err(invalid) };
    if optional_size < dirs { return Err(invalid); }
    let sections = opt.checked_add(optional_size).ok_or(invalid)?;
    let count = word(pe + 6).ok_or(invalid)? as usize;
    let end = sections.checked_add(count.checked_mul(40).ok_or(invalid)?).ok_or(invalid)?;
    let headers = data.get(..end).ok_or(invalid)?;
    let mut nt = headers[pe..sections].to_vec();
    nt[24 + 64..24 + 68].fill(0);
    if dword(opt + dirs - 4).ok_or(invalid)? > 4 {
        if optional_size < dirs + 40 { return Err(invalid); }
        nt[24 + dirs + 32..24 + dirs + 40].fill(0);
    }
    let mut result = vec![Part::Bytes(headers[..pe].to_vec()), Part::Bytes(nt), Part::Bytes(headers[sections..].to_vec())];
    let mut code = Vec::new(); let mut remaining = Vec::new();
    for i in 0..count {
        let h = sections + i * 40;
        let size = dword(h + 16).ok_or(invalid)?;
        if size == 0 { continue; }
        // Debug bytes need filtering even when embedded in a non-.debug
        // section. Fail closed until that normalization is implemented.
        if data.get(h..h + 8).is_some_and(|name| name == b".debug\0\0") { return Err(50); }
        let at = dword(h + 20).ok_or(invalid)? as u64;
        if at + size as u64 > len { return Err(invalid); }
        let part = Part::File(at, size);
        if dword(h + 36).ok_or(invalid)? & 0x20 != 0 { code.push(part); } else { remaining.push(part); }
    }
    result.extend(code); result.extend(remaining); Ok(result)
}
fn ImageGetDigestStream(c: &mut Ctx) -> u32 {
    if c.a(2) == 0 { return fail(87); }
    let (h, callback, handle) = (c.a(0), c.a(2), c.a(3));
    let parts = match with_file(h, |img| digest_parts(img, c.a(1))) { Ok(Ok(p)) => p, Ok(Err(e)) | Err(e) => return fail(e) };
    // No file/registry/kernel locks may survive guest callbacks: each part is read into its
    // block with the file locked, then handed out unlocked.
    for part in parts {
        let n = match &part { Part::Bytes(b) => b.len() as u32, Part::File(_, n) => *n };
        if n == 0 { continue; }
        let buffer_own = mem::Scratch::new(n, false);
        let buffer = buffer_own.get();
        if buffer == 0 { return fail(8); }
        let ok = match &part {
            Part::Bytes(b) => { mem::write(buffer, b); true }
            Part::File(at, n) => with_file(h, |img| img.to_guest(*at, *n, buffer)).unwrap_or(false),
        };
        if !ok { drop(buffer_own); return fail(5); }
        let ok = c.call(callback, &[handle, buffer, n]);
        drop(buffer_own);
        if ok == 0 { return 0; }
    }
    1
}
/// Where the security directory entry (IMAGE_DIRECTORY_ENTRY_SECURITY: the certificate
/// table's file offset and size) is in the headers; None: not a PE image or no such entry
fn security_entry(img: &mut dyn Image) -> Option<usize> {
    let data = headers(img)?;
    let word = |o: usize| Some(u16::from_le_bytes(data.get(o..o.checked_add(2)?)?.try_into().ok()?));
    let dword = |o: usize| Some(u32::from_le_bytes(data.get(o..o.checked_add(4)?)?.try_into().ok()?));
    if data.get(..2)? != b"MZ" {
        return None;
    }
    let pe = usize::try_from(dword(0x3c)?).ok()?;
    if data.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    let opt = pe.checked_add(24)?;
    let dirs: usize = match word(opt)? {
        0x10b => 96,
        0x20b => 112,
        _ => return None,
    };
    let at = opt.checked_add(dirs)?.checked_add(32)?;
    (dword(opt.checked_add(dirs)?.checked_sub(4)?)? >= 5 && usize::from(word(pe.checked_add(20)?)?) >= dirs.checked_add(40)?).then_some(at)
}

/// The certificate table made of `entries`: each one padded to 8 bytes
pub fn table_of(entries: &[Vec<u8>]) -> Vec<u8> {
    let mut t = Vec::new();
    for e in entries {
        t.extend_from_slice(e);
        t.resize(t.len().saturating_add(7) & !7, 0);
    }
    t
}

/// A file error while the table is rewritten: ERROR_ACCESS_DENIED (what ImageHlp's
/// WriteFile failures report), the error itself going to the trace
fn io_failed(e: std::io::Error) -> u32 {
    if crate::env_on!("SAKE_TRACE_FILES") {
        eprintln!("sake: certificate table: {e}");
    }
    5
}
/// A table past 4 GiB: ERROR_INVALID_PARAMETER
fn too_big(e: std::num::TryFromIntError) -> u32 {
    if crate::env_on!("SAKE_TRACE_FILES") {
        eprintln!("sake: certificate table: {e}");
    }
    87
}

/// The certificate table rewritten as `entries`, at its place (or 8-aligned at the end of the
/// file when there was none); the file ends with it; an empty one leaves the entry 0, 0
fn rewrite(h: u32, entries: &[Vec<u8>]) -> Result<(), u32> {
    let entry = with_file(h, |img| security_entry(img))?.ok_or(193u32)?;
    let file = crate::obj::file(h).ok_or(6u32)?;
    let mut g = file.f.locked();
    let pos = g.stream_position().map_err(io_failed)?;
    let mut cur = [0u8; 8];
    g.seek(SeekFrom::Start(kome::num::u64_of(entry))).and_then(|_| g.read_exact(&mut cur)).map_err(io_failed)?;
    let old_at = u32::from_le_bytes([cur[0], cur[1], cur[2], cur[3]]);
    let len = g.metadata().map_or(0, |m| m.len());
    let at = if old_at != 0 { u64::from(old_at) } else { len.saturating_add(7) & !7 };
    let table = table_of(entries);
    let io = io_failed;
    if table.is_empty() {
        g.set_len(at).map_err(io)?;
        g.seek(SeekFrom::Start(kome::num::u64_of(entry))).and_then(|_| g.write_all(&[0; 8])).map_err(io)?;
    } else {
        let at32 = u32::try_from(at).map_err(too_big)?;
        let size = u32::try_from(table.len()).map_err(too_big)?;
        g.set_len(at).map_err(io)?;
        g.seek(SeekFrom::Start(at)).and_then(|_| g.write_all(&table)).map_err(io)?;
        let mut d = at32.to_le_bytes().to_vec();
        d.extend_from_slice(&size.to_le_bytes());
        g.seek(SeekFrom::Start(kome::num::u64_of(entry))).and_then(|_| g.write_all(&d)).map_err(io)?;
    }
    g.seek(SeekFrom::Start(pos)).map_err(io_failed)?;
    Ok(())
}

/// ImageAddCertificate(h, cert, &index): the WIN_CERTIFICATE (dwLength bytes, at least its
/// 8-byte header) added after the others; its index
fn ImageAddCertificate(c: &mut Ctx) -> u32 {
    let (h, cert, index) = (c.a(0), c.a(1), c.a(2));
    if cert == 0 {
        return fail(87);
    }
    let n = r32(cert);
    if n < 8 {
        return fail(87);
    }
    let mut entries = match entries_of(h) {
        Ok(e) => e,
        Err(e) => return fail(e),
    };
    entries.push(mem::bytes(cert, n));
    if let Err(e) = rewrite(h, &entries) {
        return fail(e);
    }
    if index != 0 {
        w32(index, kome::num::u32_of(entries.len().saturating_sub(1)));
    }
    1
}

/// ImageRemoveCertificate(h, index): the others move up
fn ImageRemoveCertificate(c: &mut Ctx) -> u32 {
    let (h, index) = (c.a(0), c.a(1));
    let mut entries = match entries_of(h) {
        Ok(e) => e,
        Err(e) => return fail(e),
    };
    let Ok(i) = usize::try_from(index) else { return fail(87) };
    if i >= entries.len() {
        return fail(87);
    }
    entries.remove(i);
    match rewrite(h, &entries) {
        Ok(()) => 1,
        Err(e) => fail(e),
    }
}

pub fn init() {
    implement("imagehlp", "ImageGetCertificateHeader", ImageGetCertificateHeader);
    implement("imagehlp", "ImageGetCertificateData", ImageGetCertificateData);
    implement("imagehlp", "ImageEnumerateCertificates", ImageEnumerateCertificates);
    implement("imagehlp", "ImageGetDigestStream", ImageGetDigestStream);
    implement("imagehlp", "ImageAddCertificate", ImageAddCertificate);
    implement("imagehlp", "ImageRemoveCertificate", ImageRemoveCertificate);
}
#[cfg(test)]
mod tests {
    use super::*;

    /// a stub DLL (sysdll.rs) with `entries` as its certificate table, the way rewrite lays it
    fn signed(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut f = crate::sysdll::image(false);
        let entry = security_entry(&mut &f[..]).expect("security entry");
        let table = table_of(entries);
        if !table.is_empty() {
            let at = u32::try_from(f.len()).unwrap_or(0);
            f[entry..entry + 4].copy_from_slice(&at.to_le_bytes());
            f[entry + 4..entry + 8].copy_from_slice(&u32::try_from(table.len()).unwrap_or(0).to_le_bytes());
            f.extend_from_slice(&table);
        }
        f
    }

    proptest::proptest! {
        /// what is written is read back, entry by entry, padding left out
        #[test]
        fn certificate_tables_round_trip(bodies in proptest::collection::vec(proptest::collection::vec(proptest::prelude::any::<u8>(), 0..40), 0..5)) {
            let entries: Vec<Vec<u8>> = bodies.iter().map(|b| {
                let n = u32::try_from(b.len() + 8).unwrap_or(0);
                let mut e = n.to_le_bytes().to_vec();
                e.extend_from_slice(&[0, 2, 2, 0]);
                e.extend_from_slice(b);
                e
            }).collect();
            let f = signed(&entries);
            proptest::prop_assert_eq!(certificates(&mut &f[..]), Some(entries.clone()));
            proptest::prop_assert_eq!(table_of(&entries).len() % 8, 0);
        }
    }

    #[test]
    fn malformed_headers_are_rejected() {
        for n in 0..256 { assert!(certificates(&mut &vec![0; n][..]).is_none()); }
    }

    // The whole-file versions this file had (b07 review: they read the file whole, three
    // copies): the oracle for the range-reading ones
    fn old_certificates(data: &[u8]) -> Option<Vec<&[u8]>> {
        let word = |o: usize| Some(u16::from_le_bytes(data.get(o..o.checked_add(2)?)?.try_into().ok()?));
        let dword = |o: usize| Some(u32::from_le_bytes(data.get(o..o.checked_add(4)?)?.try_into().ok()?));
        if data.get(..2)? != b"MZ" { return None; }
        let pe = dword(0x3c)? as usize;
        if data.get(pe..pe.checked_add(4)?)? != b"PE\0\0" { return None; }
        let opt = pe.checked_add(24)?;
        let dirs = match word(opt)? { 0x10b => 96, 0x20b => 112, _ => return None };
        if (word(pe + 20)? as usize) < dirs + 40 || dword(opt + dirs - 4)? < 5 { return Some(vec![]); }
        let (at, size) = (dword(opt + dirs + 32)? as usize, dword(opt + dirs + 36)? as usize);
        if at == 0 && size == 0 { return Some(vec![]); }
        let table = data.get(at..at.checked_add(size)?)?;
        let mut result = Vec::new(); let mut at = 0usize;
        while at < table.len() {
            let n = u32::from_le_bytes(table.get(at..at.checked_add(4)?)?.try_into().ok()?) as usize;
            if n < 8 { return None; }
            result.push(table.get(at..at.checked_add(n)?)?);
            at = at.checked_add(n)?.checked_add(7)? & !7;
            if at > table.len() { return None; }
        }
        Some(result)
    }
    fn old_digest(data: &[u8], level: u32) -> Result<Vec<Vec<u8>>, u32> {
        if level != 6 { return Err(50); }
        let invalid = 193u32;
        let word = |o: usize| -> Option<u16> { Some(u16::from_le_bytes(data.get(o..o.checked_add(2)?)?.try_into().ok()?)) };
        let dword = |o: usize| -> Option<u32> { Some(u32::from_le_bytes(data.get(o..o.checked_add(4)?)?.try_into().ok()?)) };
        if data.get(..2) != Some(b"MZ") { return Err(invalid); }
        let pe = dword(0x3c).ok_or(invalid)? as usize;
        if pe < 64 || data.get(pe..pe.checked_add(4).ok_or(invalid)?) != Some(b"PE\0\0") { return Err(invalid); }
        let opt = pe.checked_add(24).ok_or(invalid)?;
        let optional_size = word(pe + 20).ok_or(invalid)? as usize;
        let dirs = match word(opt).ok_or(invalid)? { 0x10b => 96, 0x20b => 112, _ => return Err(invalid) };
        if optional_size < dirs { return Err(invalid); }
        let sections = opt.checked_add(optional_size).ok_or(invalid)?;
        let count = word(pe + 6).ok_or(invalid)? as usize;
        let end = sections.checked_add(count.checked_mul(40).ok_or(invalid)?).ok_or(invalid)?;
        let headers = data.get(..end).ok_or(invalid)?;
        let mut nt = headers[pe..sections].to_vec();
        nt[24 + 64..24 + 68].fill(0);
        if dword(opt + dirs - 4).ok_or(invalid)? > 4 {
            if optional_size < dirs + 40 { return Err(invalid); }
            nt[24 + dirs + 32..24 + dirs + 40].fill(0);
        }
        let mut result = vec![headers[..pe].to_vec(), nt, headers[sections..].to_vec()];
        let mut code = Vec::new(); let mut remaining = Vec::new();
        for i in 0..count {
            let h = sections + i * 40;
            let size = dword(h + 16).ok_or(invalid)? as usize;
            if size == 0 { continue; }
            if data.get(h..h + 8).is_some_and(|name| name == b".debug\0\0") { return Err(50); }
            let at = dword(h + 20).ok_or(invalid)? as usize;
            let bytes = data.get(at..at.checked_add(size).ok_or(invalid)?).ok_or(invalid)?.to_vec();
            if dword(h + 36).ok_or(invalid)? & 0x20 != 0 { code.push(bytes); } else { remaining.push(bytes); }
        }
        result.extend(code); result.extend(remaining); Ok(result)
    }

    /// A small PE: `sections` (characteristics, raw bytes) and a certificate table
    fn image(pe64: bool, sections: &[(u32, Vec<u8>)], certs: &[Vec<u8>]) -> Vec<u8> {
        let pe = 0x80usize;
        let dirs = if pe64 { 112 } else { 96 };
        let optional = dirs + 16 * 8;
        let table = pe + 24 + optional;
        let mut d = vec![0u8; table + 40 * sections.len()];
        d[..2].copy_from_slice(b"MZ");
        d[0x3c..0x40].copy_from_slice(&(pe as u32).to_le_bytes());
        d[pe..pe + 4].copy_from_slice(b"PE\0\0");
        d[pe + 6..pe + 8].copy_from_slice(&(sections.len() as u16).to_le_bytes());
        d[pe + 20..pe + 22].copy_from_slice(&(optional as u16).to_le_bytes());
        let opt = pe + 24;
        d[opt..opt + 2].copy_from_slice(&(if pe64 { 0x20bu16 } else { 0x10b }).to_le_bytes());
        d[opt + 64..opt + 68].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        d[opt + dirs - 4..opt + dirs].copy_from_slice(&16u32.to_le_bytes());
        for (i, (ch, bytes)) in sections.iter().enumerate() {
            let h = table + 40 * i;
            let at = d.len();
            d.extend_from_slice(bytes);
            d[h..h + 8].copy_from_slice(format!(".s{i}\0\0\0\0\0\0").as_bytes().get(..8).unwrap());
            d[h + 16..h + 20].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            d[h + 20..h + 24].copy_from_slice(&(at as u32).to_le_bytes());
            d[h + 36..h + 40].copy_from_slice(&ch.to_le_bytes());
        }
        while d.len() % 8 != 0 { d.push(0); }
        let at = d.len();
        for c in certs {
            let n = 8 + c.len();
            d.extend_from_slice(&(n as u32).to_le_bytes());
            d.extend_from_slice(&0x0200u16.to_le_bytes());
            d.extend_from_slice(&0x0002u16.to_le_bytes());
            d.extend_from_slice(c);
            while d.len() % 8 != 0 { d.push(0); }
        }
        if !certs.is_empty() {
            d[opt + dirs + 32..opt + dirs + 36].copy_from_slice(&(at as u32).to_le_bytes());
            let size = (d.len() - at) as u32;
            d[opt + dirs + 36..opt + dirs + 40].copy_from_slice(&size.to_le_bytes());
        }
        d
    }

    fn materialized(data: &[u8], parts: Vec<Part>) -> Vec<Vec<u8>> {
        parts.into_iter().map(|p| match p {
            Part::Bytes(b) => b,
            Part::File(at, n) => data[at as usize..at as usize + n as usize].to_vec(),
        }).collect()
    }

    proptest::proptest! {
        /// Reading the ranges it needs answers as reading the whole file did: valid images
        /// with random sections and certificates, then the same with random bytes changed
        /// (headers, counts, offsets, sizes)
        #[test]
        fn ranges_answer_as_the_whole_file(pe64: bool, sections in proptest::collection::vec((proptest::sample::select(vec![0x20u32, 0x40, 0x6000_0020, 0xc000_0040]), proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300)), 0..5), certs in proptest::collection::vec(proptest::collection::vec(proptest::prelude::any::<u8>(), 0..40), 0..3), flips in proptest::collection::vec((0usize..0x200, proptest::prelude::any::<u8>()), 0..6), cut in 0usize..64) {
            let mut d = image(pe64, &sections, &certs);
            for pass in 0..2 {
                let olds = old_certificates(&d).map(|v| v.into_iter().map(<[u8]>::to_vec).collect::<Vec<_>>());
                let got = certificates(&mut &d[..]);
                proptest::prop_assert_eq!(got, olds, "pass {}", pass);
                let new = digest_parts(&mut &d[..], 6);
                let new = new.map(|p| materialized(&d, p));
                proptest::prop_assert_eq!(new, old_digest(&d, 6), "pass {}", pass);
                for &(at, v) in &flips {
                    if at < d.len() { d[at] ^= v; }
                }
                d.truncate(d.len().saturating_sub(cut));
            }
        }
    }

}
