//! fullpath: RtlGetFullPathName_Ustr's rules (sake-re/ntdll/full-path.md) against Wine's
//! ntdll:path table (a Windows measurement), sizes at every edge, and laws.
use super::*;
use proptest::prelude::*;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn s(v: &[u16]) -> String {
    String::from_utf16_lossy(v)
}
fn run(path: &str, size: u32) -> Full {
    let cur = w("C:\\windows\\");
    let unset = |_: u16| DriveDir::Unset;
    full_path_name(&w(path), size, &Env { cur: &cur, drive: &unset })
}
fn file_of(f: &Full) -> Option<String> {
    f.file_part.map(|j| s(&f.text()[j..]))
}

#[test]
fn wine_table() {
    // (path, name, file part, and another Windows version's name and file part)
    let rows: &[(&str, &str, Option<&str>, Option<&str>, Option<&str>)] = &[
        ("c:/test", "c:\\test", Some("test"), None, None),
        ("c:/test/", "c:\\test\\", None, None, None),
        ("c:/test     ", "c:\\test", Some("test"), None, None),
        ("c:/test.", "c:\\test", Some("test"), None, None),
        ("c:/test  ....   ..   ", "c:\\test", Some("test"), None, None),
        ("c:/test/  ....   ..   ", "c:\\test\\", None, None, None),
        ("c:/test/..", "c:\\", None, None, None),
        ("c:/test/.. ", "c:\\test\\", None, None, None),
        ("c:/TEST", "c:\\TEST", Some("TEST"), None, None),
        ("c:/test/file", "c:\\test\\file", Some("file"), None, None),
        ("c:/test./file", "c:\\test\\file", Some("file"), None, None),
        ("c:/test.. /file", "c:\\test.. \\file", Some("file"), None, None),
        ("c:/test/././file", "c:\\test\\file", Some("file"), None, None),
        ("c:/test\\.\\.\\file", "c:\\test\\file", Some("file"), None, None),
        ("c:/test/\\.\\.\\file", "c:\\test\\file", Some("file"), None, None),
        ("c:/test\\\\.\\.\\file", "c:\\test\\file", Some("file"), None, None),
        ("c:/test\\test1\\..\\.\\file", "c:\\test\\file", Some("file"), None, None),
        ("c:///test\\.\\.\\file//", "c:\\test\\file\\", None, Some("c:\\test\\file"), Some("file")),
        ("c:///test\\..\\file\\..\\//", "c:\\", None, None, None),
        ("c:/test../file", "c:\\test.\\file", Some("file"), Some("c:\\test..\\file"), Some("file")),
        ("c:\\test", "c:\\test", Some("test"), None, None),
        ("c:\\test\\*.", "c:\\test\\*", Some("*"), None, None),
        ("c:\\test\\a*b.*", "c:\\test\\a*b.*", Some("a*b.*"), None, None),
        ("c:\\test\\a*b*.", "c:\\test\\a*b*", Some("a*b*"), None, None),
        ("C:\\test", "C:\\test", Some("test"), None, None),
        ("c:/", "c:\\", None, None, None),
        ("c:.", "C:\\windows", Some("windows"), None, None),
        ("c:foo", "C:\\windows\\foo", Some("foo"), None, None),
        ("c:foo/bar", "C:\\windows\\foo\\bar", Some("bar"), None, None),
        ("c:./foo", "C:\\windows\\foo", Some("foo"), None, None),
        ("\\foo", "C:\\foo", Some("foo"), None, None),
        ("foo", "C:\\windows\\foo", Some("foo"), None, None),
        (".", "C:\\windows", Some("windows"), None, None),
        ("..", "C:\\", None, None, None),
        ("...", "C:\\windows\\", None, None, None),
        ("./foo", "C:\\windows\\foo", Some("foo"), None, None),
        ("foo/..", "C:\\windows", Some("windows"), None, None),
        ("\\windows\\nul", "\\\\.\\nul", None, None, None),
        ("C:\\nonexistent\\nul", "\\\\.\\nul", None, None, None),
        ("C:\\con\\con", "\\\\.\\con", None, Some("C:\\con\\con"), Some("con")),
        ("C:NUL.", "\\\\.\\NUL", None, None, None),
        ("C:NUL", "\\\\.\\NUL", None, None, None),
        ("AUX", "\\\\.\\AUX", None, None, None),
        ("COM1", "\\\\.\\COM1", None, None, None),
        ("?<>*\"|:", "C:\\windows\\?<>*\"|:", Some("?<>*\"|:"), None, None),
        ("\\\\foo", "\\\\foo", None, None, None),
        ("//foo", "\\\\foo", None, None, None),
        ("\\/foo", "\\\\foo", None, None, None),
        ("//", "\\\\", None, None, None),
        ("//foo/", "\\\\foo\\", None, None, None),
        ("//.", "\\\\.\\", None, None, None),
        ("//./", "\\\\.\\", None, None, None),
        ("//.//", "\\\\.\\", None, None, None),
        ("//./foo", "\\\\.\\foo", Some("foo"), None, None),
        ("//./foo/", "\\\\.\\foo\\", None, None, None),
        ("//./foo/bar", "\\\\.\\foo\\bar", Some("bar"), None, None),
        ("//./foo/.", "\\\\.\\foo", Some("foo"), None, None),
        ("//./foo/..", "\\\\.\\", None, None, None),
        ("//?/", "\\\\?\\", None, None, None),
        ("//?//", "\\\\?\\", None, None, None),
        ("//?/foo", "\\\\?\\foo", Some("foo"), None, None),
        ("//?/foo/", "\\\\?\\foo\\", None, None, None),
        ("//?/foo/bar", "\\\\?\\foo\\bar", Some("bar"), None, None),
        ("//?/foo/.", "\\\\?\\foo", Some("foo"), None, None),
        ("//?/foo/..", "\\\\?\\", None, None, None),
        ("CONIN$", "\\\\.\\CONIN$", None, Some("C:\\windows\\CONIN$"), Some("CONIN$")),
        ("CONOUT$", "\\\\.\\CONOUT$", None, Some("C:\\windows\\CONOUT$"), Some("CONOUT$")),
        ("\\??\\foo", "C:\\??\\foo", Some("foo"), None, None),
    ];
    for &(p, name, file, alt, alt_file) in rows {
        let f = run(p, 520);
        let got = s(&f.text());
        let ok = (got == name && file_of(&f).as_deref() == file) || (alt == Some(got.as_str()) && file_of(&f).as_deref() == alt_file);
        assert!(ok, "{p:?}: {got:?} {:?}", file_of(&f));
        assert_eq!(f.ret, kome::num::u32_of(got.len() * 2), "{p:?}");
    }
}

#[test]
fn empty_and_blank_leave_the_buffer() {
    for p in ["", " ", "   "] {
        assert_eq!(run(p, 520), Full { ret: 0, buffer: None, file_part: None, kind: None });
    }
}

#[test]
fn short_buffers_answer_the_size_needed() {
    // "foo" against "C:\windows\": 28 bytes; it needs 22 of prefix + 6 + the NUL
    assert_eq!(run("foo", 520).ret, 28);
    for size in 0..32u32 {
        let f = run("foo", size);
        if size >= 30 {
            assert_eq!(f.ret, 28, "{size}");
        } else {
            assert_eq!(f.ret, 22 + 6 + 2, "{size}");
            // (what did not fit: the buffer's first unit 0)
            assert_eq!(f.buffer.as_ref().and_then(|b| b.first().copied()).unwrap_or(0), 0);
        }
    }
    // "." alone: the directory without its '\' once that fits
    assert_eq!(run(".", 22).ret, 20);
    assert_eq!(s(&run(".", 22).text()), "C:\\windows");
    assert_eq!(run(".", 20).ret, 22);
    // a device: "\\.\" and the name
    assert_eq!(run("AUX", 520).ret, 14);
    assert_eq!(run("AUX", 14).ret, 16);
}

#[test]
fn other_drives() {
    let cur = w("C:\\windows\\");
    let set = |l: u16| if l == u16::from(b'D') { DriveDir::Set(w("D:\\games")) } else { DriveDir::Unset };
    let f = full_path_name(&w("d:save"), 520, &Env { cur: &cur, drive: &set });
    assert_eq!(s(&f.text()), "D:\\games\\save");
    let f = full_path_name(&w("e:x"), 520, &Env { cur: &cur, drive: &set });
    assert_eq!(s(&f.text()), "E:\\x");
    // (a UNC current directory: ".." stops at the share, "\x" goes to its root)
    let unc = w("\\\\srv\\share\\dir\\");
    let f = full_path_name(&w("..\\..\\x"), 520, &Env { cur: &unc, drive: &set });
    assert_eq!(s(&f.text()), "\\\\srv\\share\\x");
    let f = full_path_name(&w("\\y"), 520, &Env { cur: &unc, drive: &set });
    assert_eq!(s(&f.text()), "\\\\srv\\share\\y");
}

fn nt(path: &str) -> Result<NtPath, u32> {
    let cur = w("C:\\windows\\");
    let unset = |_: u16| DriveDir::Unset;
    // (only C:\ and C:\windows exist)
    let exists = |d: &[u16]| matches!(s(d).to_ascii_lowercase().as_str(), "c:\\." | "c:\\windows\\." | "c:\\windows\\system32\\.");
    dos_to_nt(&w(path), &Env { cur: &cur, drive: &unset }, &exists)
}

#[test]
fn nt_wine_table() {
    // (dos, nt, file part offset) from Wine's test_RtlDosPathNameToNtPathName_U (Windows 10/11)
    let rows: &[(&str, &str, Option<usize>)] = &[
        ("c:\\", "\\??\\c:\\", None),
        ("c:\\test\\*.", "\\??\\c:\\test\\*", Some(12)),
        ("c:/", "\\??\\c:\\", None),
        ("c:/foo", "\\??\\c:\\foo", Some(7)),
        ("c:/foo.", "\\??\\c:\\foo", Some(7)),
        ("c:/foo ", "\\??\\c:\\foo", Some(7)),
        ("c:/foo . .", "\\??\\c:\\foo", Some(7)),
        ("CONOUT$", "\\??\\CONOUT$", None),
        ("cOnOuT$", "\\??\\cOnOuT$", None),
        ("CONERR$", "\\??\\C:\\windows\\CONERR$", Some(15)),
    ];
    for &(dos, want, part) in rows {
        let n = nt(dos).unwrap_or_else(|e| panic!("{dos}: {e:#x}"));
        assert_eq!(s(&n.nt), want, "{dos}");
        assert_eq!(n.file_part, part, "{dos}");
    }
    for bad in ["", " ", "C:\\nonexistent\\nul"] {
        assert_eq!(nt(bad), Err(0xc000_0033), "{bad:?}");
    }
}

#[test]
fn nt_forms() {
    // a device in a directory that exists, a UNC path, "\\.\", the "\\?\" and "\??\" passes
    assert_eq!(s(&nt("C:\\windows\\nul").map_or_default(|n| n.nt)), "\\??\\nul");
    let n = nt("\\\\srv\\share\\a.txt").unwrap_or_else(|e| panic!("{e:#x}"));
    assert_eq!((s(&n.nt), n.file_part), ("\\??\\UNC\\srv\\share\\a.txt".to_string(), Some(18)));
    assert_eq!(s(&nt("\\\\.\\pipe\\x").map_or_default(|n| n.nt)), "\\??\\pipe\\x");
    let n = nt("\\\\?\\C:\\a\\..\\b.").unwrap_or_else(|e| panic!("{e:#x}"));
    assert_eq!((s(&n.nt), n.file_part), ("\\??\\C:\\a\\..\\b.".to_string(), Some(12)));
    let n = nt("\\??\\C:\\dir\\").unwrap_or_else(|e| panic!("{e:#x}"));
    assert_eq!((s(&n.nt), n.file_part), ("\\??\\C:\\dir\\".to_string(), None));
    // (exactly "\??\" is not passed: Length must exceed 8 bytes)
    assert!(nt("\\??\\").is_ok_and(|n| s(&n.nt) != "\\??\\"));
    // the relative name: past the current directory, only for a relative input
    let n = nt("sub\\f.txt").unwrap_or_else(|e| panic!("{e:#x}"));
    assert_eq!(n.relative.map(|r| s(&n.nt[r..])), Some("sub\\f.txt".to_string()));
    let n = nt("..\\x").unwrap_or_else(|e| panic!("{e:#x}"));
    assert_eq!(n.relative, None);
    let n = nt("C:\\windows\\x").unwrap_or_else(|e| panic!("{e:#x}"));
    assert_eq!(n.relative, None);
    // longer than the 0x208-byte static buffer: STATUS_NAME_TOO_LONG
    assert_eq!(nt(&format!("C:\\{}", "a".repeat(258))), Err(0xc000_0106));
    assert!(nt(&format!("C:\\{}", "a".repeat(256))).is_ok());
}

proptest! {
    /// a full path's result is full: it starts with a drive and a '\', or "\\"; it has no
    /// "\.\", "\..\" or "\\" past its start, and never ends in a space or a dot
    #[test]
    fn results_are_normal(p in "[a-c./\\\\ ]{1,16}") {
        let f = run(&p, 520);
        let t = s(&f.text());
        if f.ret != 0 && !t.starts_with("\\\\") {
            prop_assert!(t.len() >= 3 && t.as_bytes()[1] == b':' && t.as_bytes()[2] == b'\\', "{p:?} -> {t:?}");
            prop_assert!(!t[2..].contains("\\\\") && !t.contains("\\.\\") && !t.contains("\\..\\"), "{p:?} -> {t:?}");
            prop_assert!(!t.ends_with(' ') && !t.ends_with('.'), "{p:?} -> {t:?}");
        }
        prop_assert_eq!(f.ret, kome::num::u32_of(t.len() * 2));
    }

    /// the same path in a bigger buffer gives the same text
    #[test]
    fn size_does_not_change_the_answer(p in "[a-c./\\\\]{1,12}", extra in 0u32..400) {
        let a = run(&p, 520);
        let b = run(&p, 520 + extra);
        prop_assert_eq!(a.text(), b.text());
        prop_assert_eq!(a.ret, b.ret);
    }
}
