//! tmpname: GetTempFileName's rules (Wine's kernel32:path rows, Windows) and laws.
use super::*;
use proptest::prelude::*;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn s(v: &[u16]) -> String {
    String::from_utf16_lossy(v)
}
fn dirs(p: &[u16]) -> Option<u32> {
    matches!(s(p).to_ascii_lowercase().as_str(), "c:\\t" | "c:\\t\\" | "\\" | "c:\\").then_some(0x10)
}

#[test]
fn rows() {
    let mut never = |_: &[u16]| Made::Yes;
    // a nonexistent directory, and a file, are ERROR_DIRECTORY
    assert_eq!(temp_file_name(&w("C:\\t\\nope"), &w("tfn"), 0, 1, &dirs, &mut never), Err(267));
    let file = |p: &[u16]| (s(p) == "C:\\f").then_some(0x20);
    assert_eq!(temp_file_name(&w("C:\\f"), &w("tfn"), 0, 1, &file, &mut never), Err(267));
    // unique: its low word, the name without padding, the prefix cut to 3
    let r = temp_file_name(&w("C:\\t\\"), &w("path"), 0x1_00a5, 1, &dirs, &mut never).map(|(n, v)| (n, s(&v)));
    assert_eq!(r, Ok((0xa5, "C:\\t\\patA5.tmp".to_string())));
    let r = temp_file_name(&w("C:\\t"), &[], 7, 1, &dirs, &mut never).map(|(n, v)| (n, s(&v)));
    assert_eq!(r, Ok((7, "C:\\t\\7.tmp".to_string())));
    // an empty path is the current drive's root
    assert_eq!(temp_file_name(&[], &w("a"), 1, 1, &dirs, &mut never).map(|(_, v)| s(&v)), Ok("\\a1.tmp".to_string()));
    // too long
    let long = w(&format!("C:\\{}", "a".repeat(246)));
    assert_eq!(temp_file_name(&long, &[], 1, 1, &|_| Some(0x10), &mut never), Err(111));
    let ok = w(&format!("C:\\{}", "a".repeat(244)));
    assert!(temp_file_name(&ok, &[], 1, 1, &|_| Some(0x10), &mut never).is_ok());
}

#[test]
fn search() {
    // taken names go on, from the start (0 skipped); a fatal error ends it
    let mut taken = |p: &[u16]| if s(p).ends_with("1.tmp") || s(p).ends_with("2.tmp") { Made::No(80, false) } else { Made::Yes };
    assert_eq!(temp_file_name(&w("C:\\t"), &w("x"), 0, 0x1_0000, &dirs, &mut taken).map(|r| r.0), Ok(3));
    let mut denied = |_: &[u16]| Made::No(5, false);
    assert_eq!(temp_file_name(&w("C:\\t"), &w("x"), 0, 9, &dirs, &mut denied), Err(5));
    let mut dir_there = |p: &[u16]| if s(p).ends_with("9.tmp") { Made::No(5, true) } else { Made::Yes };
    assert_eq!(temp_file_name(&w("C:\\t"), &w("x"), 0, 9, &dirs, &mut dir_there).map(|r| r.0), Ok(10));
    let mut full = |_: &[u16]| Made::No(80, false);
    assert_eq!(temp_file_name(&w("C:\\t"), &w("x"), 0, 9, &dirs, &mut full), Err(80));
}

proptest! {
    /// a made name is the directory, '\', the prefix's first 3 units, the hex number, ".tmp"
    #[test]
    fn name_shape(pre in "[a-z]{0,6}", n in 1u16.., unique: bool) {
        let mut yes = |_: &[u16]| Made::Yes;
        let (got, v) = temp_file_name(&w("C:\\t"), &w(&pre), if unique { u32::from(n) } else { 0 }, u32::from(n), &dirs, &mut yes).unwrap_or_default();
        prop_assert_eq!(got, u32::from(n));
        prop_assert_eq!(s(&v), format!("C:\\t\\{}{:X}.tmp", &pre[..pre.len().min(3)], n));
    }
}
