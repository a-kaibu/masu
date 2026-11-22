//! 8.3 short names as NTFS makes them on Windows' C: (RtlGenerate8dot3Name with
//! AllowExtendedCharacters FALSE, NTFS's default), from the clean-room notes (project folder
//! sake-re/ntdll/short-names.md); checked by Wine's kernel32:path.
//!
//! A directory's short names are kept in it, as NTFS keeps them in its index: a hidden file
//! (`TABLE`, a name no Win32 path can spell) lists each long name with the short one it was
//! given. A name gets its short one when it is made (kura::fs tells `watch`), keeps it while
//! other names come and go, and frees it when it is removed or renamed; the table is a file of
//! the directory, so it goes into the saves with the files and comes back with them (review
//! b5dacf4 2.3: the names were made again from each listing, so deleting one file renamed
//! another). A name that was there before sake saw it made (a game's own files) gets its short
//! name the first time its directory is looked at, in creation order (inode numbers), after
//! the ones already kept.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use kome::locks::Lock as _;

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
    let mut taken: BTreeSet<Vec<u16>> = entries.iter().map(|e| upper(&e.0)).collect();
    assign_after(&mut taken, entries)
}

/// `assign` with the names `taken` already holds out of the candidates (each given one is
/// added to it)
fn assign_after(taken: &mut BTreeSet<Vec<u16>>, entries: &[(Vec<u16>, u64)]) -> Vec<Option<Vec<u16>>> {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (entries.get(a), entries.get(b));
        x.map(|e| (e.1, upper(&e.0))).cmp(&y.map(|e| (e.1, upper(&e.0))))
    });
    let mut out = vec![None; entries.len()];
    for k in order {
        let Some((name, _)) = entries.get(k) else { continue };
        if let Some(o) = out.get_mut(k) {
            *o = first_free(name, taken);
        }
    }
    out
}

/// The first of `name`'s candidates `taken` does not hold (added to it); None when the name
/// needs none or they ran out
fn first_free(name: &[u16], taken: &mut BTreeSet<Vec<u16>>) -> Option<Vec<u16>> {
    if !needs_short(name) {
        return None;
    }
    let mut ctx = Context::default();
    while let Some(cand) = generate(name, &mut ctx) {
        if taken.insert(upper(&cand)) {
            return Some(cand);
        }
    }
    None
}

/// The file in each directory that keeps its short names: '<' and '>' are no characters of a
/// Win32 file name, so no guest path names it, and listings for the guest leave it out
/// (`read_dir`)
pub const TABLE: &str = "<8dot3>";

/// A directory's kept short names: long name (as the host has it) -> short name
type Table = BTreeMap<String, String>;

/// One change of a table at a time (made, removed and looked-at names read and write it)
static TABLES: Mutex<()> = Mutex::new(());

fn table_path(host_dir: &str) -> String {
    format!("{}/{TABLE}", host_dir.trim_end_matches('/'))
}

/// The table's lines: "long\tshort" (a Win32 name holds no tab or line break)
fn read_table(host_dir: &str) -> Table {
    let Ok(b) = kura::fs::read(table_path(host_dir)) else { return Table::new() };
    String::from_utf8_lossy(&b).lines().filter_map(|l| l.split_once('\t')).map(|(l, s)| (l.to_string(), s.to_string())).collect()
}

/// Writes the table (removes it when it keeps nothing); a directory that cannot be written
/// keeps it unwritten, and its names are made again the same way at the next look
fn write_table(host_dir: &str, t: &Table) {
    let p = table_path(host_dir);
    if t.is_empty() {
        if kura::fs::symlink_metadata(&p).is_ok() {
            if let Err(e) = kura::fs::remove_file(&p) && crate::env_on!("SAKE_TRACE_FILES") {
                eprintln!("sake: 8.3 table {p}: {e}");
            }
        }
        return;
    }
    let text: String = t.iter().map(|(l, s)| format!("{l}\t{s}\n")).collect();
    if let Err(e) = kura::fs::write(&p, text) && crate::env_on!("SAKE_TRACE_FILES") {
        eprintln!("sake: 8.3 table {p}: {e}");
    }
}

/// The directory's entries for the guest: every one but `TABLE`
pub fn read_dir(host_dir: impl AsRef<std::path::Path>) -> std::io::Result<impl Iterator<Item = std::io::Result<kura::fs::DirEntry>>> {
    Ok(kura::fs::read_dir(host_dir)?.filter(|e| e.as_ref().map_or(true, |e| e.file_name() != TABLE)))
}

/// Removes an empty directory (its table, which keeps nothing then, goes with it)
pub fn remove_dir(host_dir: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    let host_dir = &*host_dir.as_ref().to_string_lossy();
    if read_dir(host_dir)?.next().is_some() {
        return kura::fs::remove_dir(host_dir);
    }
    let p = table_path(host_dir);
    if kura::fs::symlink_metadata(&p).is_ok() {
        kura::fs::remove_file(&p)?;
    }
    kura::fs::remove_dir(host_dir)
}

/// The table brought up to the directory as it is: names no longer there dropped, names
/// without one given theirs in creation order after the kept ones (`made`, a name just made,
/// last of all, with no kept name of its own)
fn settle(host_dir: &str, t: &mut Table, made: Option<&str>) {
    let mut entries: Vec<(String, u64)> = read_dir(host_dir).into_iter().flatten().flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.metadata().map_or(0, |m| m.ino()))).collect();
    if let Some(m) = made {
        t.remove(m);
        entries.retain(|(n, _)| n != m);
    }
    t.retain(|l, _| entries.iter().any(|(n, _)| n == l));
    let mut taken: BTreeSet<Vec<u16>> = entries.iter().map(|(n, _)| upper(&n.encode_utf16().collect::<Vec<u16>>())).collect();
    taken.extend(t.values().map(|s| upper(&s.encode_utf16().collect::<Vec<u16>>())));
    if let Some(m) = made {
        taken.insert(upper(&m.encode_utf16().collect::<Vec<u16>>()));
    }
    let fresh: Vec<(Vec<u16>, u64)> = entries.iter().filter(|(n, _)| !t.contains_key(n)).map(|(n, i)| (n.encode_utf16().collect(), *i)).collect();
    for ((n, _), s) in fresh.iter().zip(assign_after(&mut taken, &fresh)) {
        if let Some(s) = s {
            t.insert(String::from_utf16_lossy(n), String::from_utf16_lossy(&s));
        }
    }
    if let Some(m) = made
        && let Some(s) = first_free(&m.encode_utf16().collect::<Vec<u16>>(), &mut taken)
    {
        t.insert(m.to_string(), String::from_utf16_lossy(&s));
    }
}

/// A name made or removed (kura::fs::watch_names): NTFS gives a made name its short one then,
/// and frees a removed one's. A directory with a table has every name it holds in it (the first
/// look at a directory, or the first name made there, makes it), so a made name takes the first
/// candidate the table and the directory's long names leave, without listing the directory.
fn on_named(e: kura::fs::Named) {
    let (kura::fs::Named::Made(p) | kura::fs::Named::Gone(p)) = e;
    let Some((dir, name)) = p.rsplit_once('/') else { return };
    let w: Vec<u16> = name.encode_utf16().collect();
    let made = matches!(e, kura::fs::Named::Made(_));
    if name == TABLE || made && !needs_short(&w) {
        return;
    }
    let dir = if dir.is_empty() { "/" } else { dir };
    let _g = TABLES.locked();
    let kept = kura::fs::symlink_metadata(table_path(dir)).is_ok();
    if !made && !kept {
        return;
    }
    let mut t = read_table(dir);
    let before = t.clone();
    if !made {
        t.remove(name);
    } else if !kept {
        settle(dir, &mut t, Some(name));
    } else {
        t.remove(name);
        let mut taken: BTreeSet<Vec<u16>> = t.values().map(|s| upper(&s.encode_utf16().collect::<Vec<u16>>())).collect();
        taken.insert(upper(&w));
        let mut ctx = Context::default();
        while let Some(cand) = generate(&w, &mut ctx) {
            let c = String::from_utf16_lossy(&cand);
            if !taken.contains(&upper(&cand)) && kura::fs::name_nocase(dir, &c).is_none() {
                t.insert(name.to_string(), c);
                break;
            }
        }
    }
    if t != before {
        write_table(dir, &t);
    }
}

/// Starts keeping short names as names are made and removed (once; files::c_root calls it)
pub fn watch() {
    kura::fs::watch_names(on_named);
}

/// The host directory's entries with their short names (an entry that has none: None): the
/// kept ones, and the ones given now to names that had none
pub fn of_dir(host_dir: &str) -> Vec<(String, Option<String>)> {
    let _g = TABLES.locked();
    let mut t = read_table(host_dir);
    let before = t.clone();
    settle(host_dir, &mut t, None);
    if t != before {
        write_table(host_dir, &t);
    }
    read_dir(host_dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).map(|n| {
        let s = t.get(&n).cloned();
        (n, s)
    }).collect()
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
