//! ntname: Wine's test_nt_names rows (Windows' answers) whose status the syntax decides, and
//! laws over generated names.
use super::*;
use proptest::prelude::*;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[test]
fn wine_rows() {
    // (root, name, syntax answer: None = passes to the lookup)
    let rows: &[(Option<&str>, &str, Option<u32>)] = &[
        (None, "\\??\\C:\\windows\\system32\\kernel32.dll", None),
        (None, "\\??\\C:\\\\windows\\system32\\kernel32.dll", None),
        (None, "\\??\\C:\\windows\\system32\\", None),
        (None, "\\??\\C:\\\\\\windows\\system32\\kernel32.dll", Some(0xc000_0033)),
        (None, "\\??\\C:\\windows\\\\system32\\kernel32.dll", Some(0xc000_0033)),
        (None, "\\??\\C:\\windows\\system32\\.\\kernel32.dll", Some(0xc000_0033)),
        (None, "\\??\\C:\\windows\\system32\\..\\system32\\kernel32.dll", Some(0xc000_0033)),
        (None, "\\??\\C:\\.\\windows\\system32\\kernel32.dll", Some(0xc000_0033)),
        (None, "\\??\\C:\\windows\\system32\\kernel32.dll   ", None),
        (None, "\\??\\C:\\windows\\system32\\kernel32.dll..", None),
        (None, "\\??\\C:\\windows   \\system32   \\kernel32.dll", None),
        (None, "\\??\\C:\\windows/system32/kernel32.dll", Some(0xc000_0033)),
        (None, "\\??\\C:\\windows\\system32\\kernel32.dll*", Some(0xc000_0033)),
        (None, "\\??\\C:\\windows\\system32?\\kernel32.dll", Some(0xc000_0033)),
        (None, "C:\\windows\\system32?\\kernel32.dll", Some(0xc000_003b)),
        (None, "/??\\C:\\windows\\system32\\kernel32.dll", Some(0xc000_003b)),
        (None, "\\??/C:\\windows\\system32\\kernel32.dll", Some(0xc000_003a)),
        (None, "\\??\\C:/windows\\system32\\kernel32.dll", Some(0xc000_003a)),
        (None, "\\??\\C:\\windows\\system32\\\\", Some(0xc000_0033)),
        (None, "\\??\\C:\\windows\\sys\u{1}", Some(0xc000_0033)),
        (None, "\\??\\", Some(0xc000_0033)),
        (None, "\\??\\C:\\", None),
        (None, "\\??\\C:\\\\", None),
        (None, "/??\\C:\\", Some(0xc000_003b)),
        (None, "\\??\\C:/", Some(0xc000_0034)),
        (None, "\\??/C:", Some(0xc000_0034)),
        (None, "\\??/C:\\", Some(0xc000_003a)),
        (None, "\\??\\C:\\windows", None),
        (None, "\\??\\C:\\windows\\", None),
        (None, "\\??\\C:\\windows\\.", Some(0xc000_0033)),
        (None, "\\??\\C:\\windows\\.\\", Some(0xc000_0033)),
        (None, "\\??\\C:\\windows\\..", Some(0xc000_0033)),
        (None, "\\??\\C:\\windows\\..\\", Some(0xc000_0033)),
        (None, "C:\\", Some(0xc000_003b)),
        (Some("root"), "windows\\system32\\kernel32.dll", None),
        (Some("root"), "system32\\", None),
        (Some("root"), "system32\\\\", Some(0xc000_0033)),
        (Some("root"), "\\system32\\kernel32.dll", Some(0xc000_000d)),
        (Some("root"), "/system32\\kernel32.dll", Some(0xc000_0033)),
        (Some("root"), ".\\system32\\kernel32.dll", Some(0xc000_0033)),
        (Some("root"), "..\\windows\\system32\\kernel32.dll", Some(0xc000_0033)),
        (Some("root"), ".", Some(0xc000_0033)),
        (Some("root"), "..", Some(0xc000_0033)),
        (Some("root"), "sys\u{1}", Some(0xc000_0033)),
        // (names kernel32 makes, and other object directories, pass)
        (None, "\\??\\UNC\\srv\\share\\x", None),
        (None, "\\??\\pipe\\x", None),
        (None, "\\??\\CONOUT$", None),
        (None, "\\Device\\HarddiskVolume3\\x", None),
        (None, "\\DosDevices\\C:\\x", None),
    ];
    for &(root, name, want) in rows {
        assert_eq!(check(&w(name), root.is_some()).err(), want, "{root:?} {name:?}");
    }
}

proptest! {
    /// the names RtlDosPathNameToNtPathName_U makes of ordinary full paths pass
    #[test]
    fn dos_made_names_pass(drive in prop::sample::select(vec!['A', 'C', 'z']), comps in prop::collection::vec("[A-Za-z0-9 _.$~-]{1,8}", 0..5), dir: bool) {
        let comps: Vec<String> = comps.into_iter().filter(|c| c != "." && c != "..").collect();
        let mut s = format!("\\??\\{drive}:\\{}", comps.join("\\"));
        if dir && !comps.is_empty() {
            s.push('\\');
        }
        prop_assert_eq!(check(&w(&s), false), Ok(()), "{}", s);
        prop_assert_eq!(check(&w(&comps.join("\\")), true), Ok(()));
    }

    /// one illegal unit anywhere in a component makes the name invalid
    #[test]
    fn illegal_unit(pre in "[a-z]{0,4}", post in "[a-z]{0,4}", u in prop::sample::select(vec![0x1u16, 0x1f, 0x22, 0x2a, 0x2f, 0x3c, 0x3e, 0x3f, 0x7c])) {
        let mut n = w(&format!("\\??\\C:\\d\\{pre}"));
        n.push(u);
        n.extend(w(&post));
        prop_assert_eq!(check(&n, false), Err(0xc000_0033));
    }
}
