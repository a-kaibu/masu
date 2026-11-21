//! The syntax the object manager and the file system ask of an NT name before a file is looked
//! up (NtCreateFile / NtOpenFile / NtQueryAttributesFile / NtDeleteFile). Rules fitted to
//! Windows' answers in Wine's ntdll:path test_nt_names (Windows 10/11 measurements).

const BS: u16 = 0x5c;

const STATUS_INVALID_PARAMETER: u32 = 0xc000_000d;
const STATUS_OBJECT_NAME_INVALID: u32 = 0xc000_0033;
const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xc000_0034;
const STATUS_OBJECT_PATH_NOT_FOUND: u32 = 0xc000_003a;
const STATUS_OBJECT_PATH_SYNTAX_BAD: u32 = 0xc000_003b;

/// A unit no file name holds: controls and "*/<>?| (':' is a stream's)
const fn illegal(u: u16) -> bool {
    u < 0x20 || matches!(u, 0x22 | 0x2a | 0x2f | 0x3c | 0x3e | 0x3f | 0x7c)
}

/// The part of a name the file system parses (after "\??\X:", or all of a name under a root
/// directory): its components are not empty (but one '\' may end it), not "." or "..", and
/// hold no illegal unit
fn fs_part(p: &[u16]) -> Result<(), u32> {
    if p.is_empty() {
        return Ok(());
    }
    let body = p.trim_suffix(&[BS]);
    if body.is_empty() {
        return Ok(());
    }
    for comp in body.split(|&u| u == BS) {
        let dots = comp == [0x2e] || comp == [0x2e, 0x2e];
        if comp.is_empty() || dots || comp.iter().copied().any(illegal) {
            return Err(STATUS_OBJECT_NAME_INVALID);
        }
    }
    Ok(())
}

/// Not found: a NAME when it was the last component, else on the PATH
fn not_found(more: bool) -> u32 {
    if more { STATUS_OBJECT_PATH_NOT_FOUND } else { STATUS_OBJECT_NAME_NOT_FOUND }
}

/// The name's syntax: Ok, or the status the open answers. `rooted`: OBJECT_ATTRIBUTES has a
/// RootDirectory (the name is then relative to it).
/// - rooted: a name starting with '\' is STATUS_INVALID_PARAMETER; the rest is the file
///   system's.
/// - else the name starts with '\' (STATUS_OBJECT_PATH_SYNTAX_BAD). A first component
///   "??<more>" is no object (not found); under "\??\" an empty name is invalid, a name with
///   '/' no object; after "\??\X:" one extra '\' is let through, then the file system's rules.
pub fn check(name: &[u16], rooted: bool) -> Result<(), u32> {
    if rooted {
        if name.first() == Some(&BS) {
            return Err(STATUS_INVALID_PARAMETER);
        }
        return fs_part(name);
    }
    let Some(rest) = name.strip_prefix(&[BS]) else {
        return Err(STATUS_OBJECT_PATH_SYNTAX_BAD);
    };
    let first_end = rest.iter().position(|&u| u == BS).unwrap_or(rest.len());
    let (first, after) = rest.split_at(first_end);
    let qq = [0x3f, 0x3f];
    if first.starts_with(&qq) && first != qq {
        return Err(not_found(!after.is_empty()));
    }
    if first != qq {
        return Ok(());
    }
    // (under \??: an object named up to the next '\')
    let Some(dev_and_rest) = after.strip_prefix(&[BS]) else { return Ok(()) };
    let dev_end = dev_and_rest.iter().position(|&u| u == BS).unwrap_or(dev_and_rest.len());
    let (dev, tail) = dev_and_rest.split_at(dev_end);
    if dev.is_empty() {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    if dev.contains(&0x2f) {
        return Err(not_found(!tail.is_empty()));
    }
    let drive = dev.len() == 2 && dev.get(1) == Some(&0x3a) && dev.first().is_some_and(|&u| u8::try_from(u).is_ok_and(|b| b.is_ascii_alphabetic()));
    if !drive {
        return Ok(());
    }
    // the volume's root: "\" or (let through) "\\"
    let tail = tail.trim_prefix(&[BS]);
    let tail = tail.trim_prefix(&[BS]);
    fs_part(tail)
}

#[cfg(test)]
#[path = "ntname_tests.rs"]
mod tests;
