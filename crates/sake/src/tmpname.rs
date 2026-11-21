//! GetTempFileName: the name kernelbase makes and the file it makes, from the clean-room notes
//! (project folder sake-re/kernelbase/temp-file-name.md); checked by Wine's kernel32:path.

const BS: u16 = 0x5c;

/// What CreateFileW(name, GENERIC_READ, 0, NULL, CREATE_NEW, NORMAL) said
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Made {
    Yes,
    /// its error, and for ERROR_ACCESS_DENIED whether the status was FILE_IS_A_DIRECTORY or
    /// DELETE_PENDING (a name that is taken)
    No(u32, bool),
}

/// The errors that end the search (the rest try the next number)
const fn fatal(err: u32, taken_dir: bool) -> bool {
    match err {
        5 => !taken_dir,
        1..=3 | 19 | 53 | 65 | 82 | 87 | 112 | 123 | 161 | 1392 | 1393 | 1816 => true,
        _ => false,
    }
}

/// "<dir>\<up to 3 of prefix><hex>.tmp" (hex: RtlIntegerToChar base 16, up to 4 digits,
/// no padding)
pub fn name(dir: &[u16], prefix: &[u16], n: u16) -> Vec<u16> {
    let mut out = dir.to_vec();
    out.push(BS);
    out.extend(prefix.iter().take(3));
    out.extend(format!("{n:X}").encode_utf16().take(4));
    out.extend(".tmp".encode_utf16());
    out
}

/// GetTempFileNameW(path, prefix, unique): Ok(answer, name) or Err(the error).
/// - path (as RtlInitUnicodeStringEx took it), with its '\' when it has none (an empty one gets
///   one), may be 0x1f0 bytes: more is ERROR_BUFFER_OVERFLOW
/// - the path without a trailing '\' (else with one) must be a directory: ERROR_DIRECTORY
/// - unique != 0: its low word, nothing made. 0: from `start` (the tick count and a counter;
///   0 skipped) each number tried in turn: a device name, or a taken one, goes on to the next
///   (after 0xffff tries: the last error / ERROR_INVALID_NAME), another error ends it.
pub fn temp_file_name(path: &[u16], prefix: &[u16], unique: u32, start: u32, attrs: &dyn Fn(&[u16]) -> Option<u32>, create: &mut dyn FnMut(&[u16]) -> Made) -> Result<(u32, Vec<u16>), u32> {
    let len = path.len().saturating_mul(2);
    let needs = path.last() != Some(&BS);
    let n = if needs { len.saturating_add(2) } else { len };
    if n > 0x1f0 {
        return Err(111);
    }
    // the directory: the path cut where its '\' is (or goes)
    let keep = n / 2;
    let dir: Vec<u16> = path.iter().copied().take(keep.saturating_sub(1)).collect();
    let mut with: Vec<u16> = dir.clone();
    with.push(BS);
    let a = attrs(&dir).or_else(|| attrs(&with));
    if a.is_none_or(|a| a & 0x10 == 0) {
        return Err(267);
    }
    let pre: Vec<u16> = prefix.iter().copied().take(3).collect();
    let u = unique & 0xffff;
    if unique != 0 {
        return Ok((u, name(&dir, &pre, kome::num::lo16(u))));
    }
    let mut v = kome::num::lo16(start);
    let mut tries = 0u32;
    loop {
        if v == 0 {
            v = 1;
        }
        let nm = name(&dir, &pre, v);
        let err = if crate::ntpath::dos_device(&nm, false) != 0 {
            123
        } else {
            match create(&nm) {
                Made::Yes => return Ok((u32::from(v), nm)),
                Made::No(e, taken) if fatal(e, taken) => return Err(e),
                Made::No(e, _) => e,
            }
        };
        tries = tries.saturating_add(1);
        if tries > 0xffff {
            return Err(err);
        }
        v = v.wrapping_add(1);
    }
}

#[cfg(test)]
#[path = "tmpname_tests.rs"]
mod tests;
