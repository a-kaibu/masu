//! 8.3 short names as NTFS makes them on Windows' C: (RtlGenerate8dot3Name with
//! AllowExtendedCharacters FALSE, NTFS's default), from the clean-room notes (project folder
//! sake-re/ntdll/short-names.md); checked by Wine's kernel32:path.
//!
//! sake keeps no short names on disk: a directory's are made again from its entries in the
//! order NTFS would have made them (their inode numbers: creation order), so a name keeps its
//! short name unless an older one that took the same base is deleted.

/// GENERATE_NAME_CONTEXT
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    checksum: u16,
    checksum_inserted: bool,
    /// the base made (at most 6 units, 8 with no index yet)
    name: Vec<u16>,
    /// ".", then up to 3 units
    ext: Vec<u16>,
    last_index: u32,
}

const DOT: u16 = 0x2e;
const TILDE: u16 = 0x7e;

/// RtlFatIllegalTable (the FAT-illegal ASCII units become '_')
const FAT_ILLEGAL: [u32; 4] = [0xffff_ffff, 0xfc00_9c04, 0x3800_0000, 0x1000_0000];

/// GetNextWchar without extended characters: from `*i`, the next unit that goes into a short
/// name (spaces, controls and non-ASCII skipped; dots too when `skip_dots`), FAT-illegal ones
/// as '_', upper-cased; 0 at the end
fn next_unit(name: &[u16], i: &mut usize, skip_dots: bool) -> u16 {
    while let Some(&c) = name.get(*i) {
        *i = i.saturating_add(1);
        if c <= 0x20 || c >= 0x7f || (c == DOT && skip_dots) {
            continue;
        }
        let ill = FAT_ILLEGAL.get(usize::from(c >> 5)).is_some_and(|w| w >> (c & 31) & 1 != 0);
        let c = if ill { u16::from(b'_') } else { c };
        return if (0x61..=0x7a).contains(&c) { c.wrapping_sub(0x20) } else { c };
    }
    0
}

/// RtlComputeLfnChecksum
pub fn checksum(name: &[u16]) -> u16 {
    let h = name.iter().fold(0u16, |h, &u| h.wrapping_mul(0x25).wrapping_add(u));
    let x = i32::from(h).wrapping_mul(0x12b9_b0a5).wrapping_abs();
    kome::num::lo16(x.wrapping_rem(1_000_000_007).cast_unsigned())
}

/// The checksum's 4 hex digits, low nibble first
fn hex4(cs: u16) -> [u16; 4] {
    let d = |n: u16| if n <= 9 { 0x30u16.wrapping_add(n) } else { 0x37u16.wrapping_add(n) };
    [d(cs & 15), d(cs >> 4 & 15), d(cs >> 8 & 15), d(cs >> 12 & 15)]
}

/// RtlGenerate8dot3Name(name, FALSE, ctx): the next candidate; None when the names ran out
/// (STATUS_FILE_SYSTEM_LIMITATION)
pub fn generate(name: &[u16], ctx: &mut Context) -> Option<Vec<u16>> {
    if ctx.name.is_empty() {
        let lead_dot = name.first() == Some(&DOT);
        // the last dot that has something after it starts the extension
        let mut i = 0usize;
        let mut last_dot = None;
        let mut c = next_unit(name, &mut i, lead_dot);
        while c != 0 {
            if c == DOT {
                last_dot = Some(i);
            }
            c = next_unit(name, &mut i, false);
        }
        let last_dot = last_dot.filter(|&d| d != name.len());
        let mut i = 0usize;
        loop {
            let c = next_unit(name, &mut i, true);
            if c == 0 || last_dot.is_some_and(|d| i >= d) || ctx.name.len() >= 6 {
                break;
            }
            ctx.name.push(c);
        }
        if ctx.name.len() <= 2 {
            ctx.checksum = checksum(name);
            ctx.name.extend(hex4(ctx.checksum));
            ctx.checksum_inserted = true;
        }
        ctx.ext.clear();
        if let Some(d) = last_dot {
            ctx.ext.push(DOT);
            let mut j = d;
            loop {
                let c = next_unit(name, &mut j, true);
                if c == 0 {
                    break;
                }
                // (ntdll's RtlGenerate8dot3Name puts a '~' last when it cuts; NTFS on Windows'
                // C: does not: "Long File test.path" is LONGFI~1.PAT in Wine's kernel32:path)
                if ctx.ext.len() >= 4 {
                    break;
                }
                ctx.ext.push(c);
            }
        }
    }
    ctx.last_index = ctx.last_index.saturating_add(1);
    if ctx.last_index > 4 && !ctx.checksum_inserted {
        ctx.checksum = checksum(name);
        ctx.name.truncate(2);
        ctx.name.extend(hex4(ctx.checksum));
        ctx.last_index = 1;
        ctx.checksum_inserted = true;
    }
    let digits: Vec<u16> = ctx.last_index.to_string().encode_utf16().collect();
    let all_nines = digits.iter().all(|&d| d == 0x39);
    let mut out = ctx.name.clone();
    out.push(TILDE);
    out.extend(&digits);
    out.extend(&ctx.ext);
    if all_nines {
        // (the next index is a digit longer: the base gives one up)
        ctx.name.pop();
        if ctx.name.is_empty() {
            return None;
        }
    }
    Some(out)
}

/// Whether NTFS gives a name a short one: it is no legal 8.3 name, or holds a space
pub fn needs_short(name: &[u16]) -> bool {
    if name == [DOT] || name == [DOT, DOT] {
        return false;
    }
    !matches!(crate::ntpath::name_8dot3(name, 12).1, Some(false))
}

fn upper(v: &[u16]) -> Vec<u16> {
    v.iter().map(|&u| crate::nt::up(u)).collect()
}

/// A directory's short names: `entries` (name, creation order) get theirs in that order, each
/// the first candidate no other entry's long or short name takes (case-insensitively)
pub fn assign(entries: &[(Vec<u16>, u64)]) -> Vec<Option<Vec<u16>>> {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (entries.get(a), entries.get(b));
        x.map(|e| (e.1, upper(&e.0))).cmp(&y.map(|e| (e.1, upper(&e.0))))
    });
    let mut taken: std::collections::BTreeSet<Vec<u16>> = entries.iter().map(|e| upper(&e.0)).collect();
    let mut out = vec![None; entries.len()];
    for k in order {
        let Some((name, _)) = entries.get(k) else { continue };
        if !needs_short(name) {
            continue;
        }
        let mut ctx = Context::default();
        while let Some(cand) = generate(name, &mut ctx) {
            if taken.insert(upper(&cand)) {
                if let Some(o) = out.get_mut(k) {
                    *o = Some(cand);
                }
                break;
            }
        }
    }
    out
}

/// The host directory's entries with their short names (an entry that has none: None)
pub fn of_dir(host_dir: &str) -> Vec<(String, Option<String>)> {
    let Ok(rd) = kura::fs::read_dir(host_dir) else { return Vec::new() };
    let names: Vec<(String, u64)> = rd.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.metadata().map_or(0, |m| m.ino()))).collect();
    let w: Vec<(Vec<u16>, u64)> = names.iter().map(|(n, i)| (n.encode_utf16().collect(), *i)).collect();
    let s = assign(&w);
    names.into_iter().zip(s).map(|((n, _), s)| (n, s.map(|v| String::from_utf16_lossy(&v)))).collect()
}

/// The short name of `name` in host directory `host_dir`
pub fn short_of(host_dir: &str, name: &str) -> Option<String> {
    of_dir(host_dir).into_iter().find(|(n, _)| n == name).and_then(|(_, s)| s)
}

/// The entry of host directory `host_dir` whose short name is `short` (case-insensitive)
pub fn long_of(host_dir: &str, short: &str) -> Option<String> {
    if !short.contains('~') {
        return None;
    }
    let up = short.to_ascii_uppercase();
    of_dir(host_dir).into_iter().find(|(_, s)| s.as_deref() == Some(up.as_str())).map(|(n, _)| n)
}

#[cfg(test)]
#[path = "shortname_tests.rs"]
mod tests;
