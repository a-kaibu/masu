//! ntpath: RtlDetermineDosPathNameType_U's rule (sake-re/ntdll/dos-paths.md) against Wine's
//! ntdll:path table (a Windows measurement), every 4-unit head of interesting units, and laws.
use super::*;
use proptest::prelude::*;

fn head(s: &str) -> [u16; 4] {
    let mut h = [0u16; 4];
    for (o, u) in h.iter_mut().zip(s.encode_utf16().chain(std::iter::once(0))) {
        *o = u;
        if u == 0 {
            break;
        }
    }
    h
}

#[test]
fn wine_table() {
    let rows: &[(&str, u32)] = &[
        ("\\\\foo", 1),
        ("//foo", 1),
        ("\\/foo", 1),
        ("/\\foo", 1),
        ("\\\\", 1),
        ("//", 1),
        ("c:\\foo", 2),
        ("c:/foo", 2),
        ("c://foo", 2),
        ("c:\\", 2),
        ("c:/", 2),
        ("c:foo", 3),
        ("c:f\\oo", 3),
        ("c:foo/bar", 3),
        ("\\foo", 4),
        ("/foo", 4),
        ("\\", 4),
        ("/", 4),
        ("foo", 5),
        ("", 5),
        ("\0:foo", 5),
        ("\\\\.\\foo", 6),
        ("//./foo", 6),
        ("/\\./foo", 6),
        ("\\\\.foo", 1),
        ("//.foo", 1),
        ("\\\\.", 7),
        ("//.", 7),
        ("\\\\?\\foo", 6),
        ("//?/foo", 6),
        ("/\\?/foo", 6),
        ("\\\\?foo", 1),
        ("//?foo", 1),
        ("\\\\?", 7),
        ("//?", 7),
        ("CONIN$", 5),
        ("CONOUT$", 5),
        ("CONERR$", 5),
        ("\\\\.\\CONIN$", 6),
        ("\\\\.\\CONOUT$", 6),
    ];
    for &(p, want) in rows {
        assert_eq!(path_type(head(p)).code(), want, "{p:?}");
    }
}

/// An independent spelling of the rule, by string prefixes
fn by_prefix(s: &str) -> u32 {
    let t: String = s.chars().take_while(|&c| c != '\0').map(|c| if c == '/' { '\\' } else { c }).collect();
    if t.starts_with("\\\\.\\") || t.starts_with("\\\\?\\") {
        6
    } else if t == "\\\\." || t == "\\\\?" {
        7
    } else if t.starts_with("\\\\") {
        1
    } else if t.starts_with('\\') {
        4
    } else if t.chars().nth(1) == Some(':') {
        if t.chars().nth(2) == Some('\\') { 2 } else { 3 }
    } else {
        5
    }
}

#[test]
fn every_head_of_the_units_that_matter() {
    let alphabet = ['\0', '\\', '/', '.', '?', ':', 'a'];
    for a in alphabet {
        for b in alphabet {
            for c in alphabet {
                for d in alphabet {
                    let s: String = [a, b, c, d].iter().collect();
                    assert_eq!(path_type(head(&s)).code(), by_prefix(&s), "{s:?}");
                }
            }
        }
    }
}

proptest! {
    /// what follows the fourth unit never matters
    #[test]
    fn four_units_decide(s in "[\\\\/.?:a-z]{0,4}", tail in "[\\\\/.?:a-z]{0,8}") {
        let full = format!("{s}{tail}");
        let t: String = full.chars().take(4).collect();
        prop_assert_eq!(path_type(head(&full)), path_type(head(&t)));
    }
}

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[test]
fn wine_dos_device_table() {
    // (path, offset, length, may fail: Wine's ntdll:path table; a Windows measurement, the
    // "may fail" rows answering 0 on some versions)
    let rows: &[(&str, u32, u32, bool)] = &[
        ("\\\\.\\CON", 8, 6, true),
        ("\\\\.\\con", 8, 6, true),
        ("\\\\.\\CON2", 0, 0, false),
        ("\\\\.\\CONIN$", 0, 0, false),
        ("\\\\.\\CONOUT$", 0, 0, false),
        ("", 0, 0, false),
        ("\\\\foo\\nul", 0, 0, false),
        ("c:\\nul:", 6, 6, false),
        ("c:\\nul\\", 0, 0, false),
        ("c:\\nul\\foo", 0, 0, false),
        ("c:\\nul::", 6, 6, false),
        ("c:\\nul::::::", 6, 6, true),
        ("c:prn     ", 4, 6, true),
        ("c:prn.......", 4, 6, true),
        ("c:prn... ...", 4, 6, true),
        ("c:NUL  ....  ", 4, 6, false),
        ("c: . . .", 0, 0, false),
        ("c:", 0, 0, false),
        (" . . . :", 0, 0, false),
        (":", 0, 0, false),
        ("c:nul. . . :", 4, 6, false),
        ("c:nul . . :", 4, 6, false),
        ("c:nul0", 0, 0, false),
        ("c:prn:aaa", 4, 6, true),
        ("c:PRN:.txt", 4, 6, true),
        ("c:aux:.txt...", 4, 6, true),
        ("c:prn:.txt:", 4, 6, true),
        ("c:nul:aaa", 4, 6, true),
        ("con:", 0, 6, false),
        ("lpt1:", 0, 8, false),
        ("c:com5:", 4, 8, true),
        ("CoM4:", 0, 8, false),
        ("lpt9:", 0, 8, false),
        ("c:\\lpt0.txt", 0, 0, false),
        ("CONIN$", 0, 12, true),
        ("CONOUT$", 0, 14, true),
        ("CONERR$", 0, 0, false),
        ("CON", 0, 6, false),
        ("PIPE", 0, 0, false),
        ("\\??\\CONIN$", 8, 12, true),
        ("\\??\\CONOUT$", 8, 14, true),
        ("\\??\\CONERR$", 0, 0, false),
        ("\\??\\CON", 8, 6, true),
        ("c:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\\nul.txt", 1000, 6, true),
    ];
    for &(p, pos, len, may_fail) in rows {
        let got = dos_device(&units(p), false);
        let want = pos << 16 | len;
        assert!(got == want || (may_fail && got == 0), "{p:?}: {:#x} want {want:#x}", got);
        // (with LegacyDosDevicePaths on, every row is the device the table says)
        assert_eq!(dos_device(&units(p), true), if p.starts_with("\\\\.\\") { 0 } else { want }, "legacy {p:?}");
    }
}

#[test]
fn windows_11_answers() {
    // (legacy off: a directory or an extension leaves only NUL without one)
    assert_eq!(dos_device(&units("c:\\nul"), false), 6 << 16 | 6);
    assert_eq!(dos_device(&units("c:\\nul.txt"), false), 0);
    assert_eq!(dos_device(&units("c:\\con"), false), 0);
    assert_eq!(dos_device(&units("con.txt"), false), 0);
    assert_eq!(dos_device(&units("nul.txt"), false), 0);
    assert_eq!(dos_device(&units("CONIN$"), false), 12);
    assert_eq!(dos_device(&units("\\\\."), false), 0);
}

proptest! {
    /// what a device answer says is there: a device name of that length at that offset
    #[test]
    fn the_answer_points_at_a_device_name(s in "[a-zA-Z:.\\\\/ $0-9]{0,12}", legacy in any::<bool>()) {
        let u = units(&s);
        let r = dos_device(&u, legacy);
        if r != 0 {
            let (off, len) = (usize::try_from(r >> 17).unwrap_or(0), usize::try_from((r & 0xffff) / 2).unwrap_or(0));
            let name: String = String::from_utf16_lossy(&u[off..off + len]).to_ascii_uppercase();
            let ok = ["NUL", "PRN", "AUX", "CON", "CONIN$", "CONOUT$"].contains(&name.as_str())
                || (name.len() == 4 && (name.starts_with("LPT") || name.starts_with("COM")) && name.as_bytes()[3].is_ascii_digit() && name.as_bytes()[3] != b'0');
            prop_assert!(ok, "{s:?} -> {name:?}");
        }
    }
}

#[test]
fn dos83_wine_table() {
    // Wine's test_RtlIsNameLegalDOS8Dot3 (Windows): (name, legal, spaces)
    let rows: &[(&str, Option<bool>)] = &[
        ("12345678", Some(false)),
        ("123 5678", Some(true)),
        ("12345678.", None),
        ("1234 678.", None),
        ("12345678.a", Some(false)),
        ("12345678.a ", None),
        ("12345678.a c", Some(true)),
        (" 2345678.a ", None),
        ("1 345678.abc", Some(true)),
        ("1      8.a c", Some(true)),
        ("1 3 5 7 .abc", None),
        ("12345678.  c", Some(true)),
        ("123456789.a", None),
        ("12345.abcd", None),
        ("12345.ab d", None),
        (".abc", None),
        ("12.abc.d", None),
        (".", Some(false)),
        ("..", Some(false)),
        ("...", None),
    ];
    for &(name, want) in rows {
        assert_eq!(legal_8dot3(name.to_ascii_uppercase().as_bytes(), None), want, "{name:?}");
    }
}

#[test]
fn dos83_dbcs() {
    let lead = |b: u8| matches!(b, 0x81..=0x9f | 0xe0..=0xfc);
    // a pair counts two bytes; it may not start at 7 in the base, nor be cut at the end
    assert_eq!(legal_8dot3(b"\x82\xa0\x82\xa2\x82\xa4\x82\xa6", Some(&lead)), Some(false));
    assert_eq!(legal_8dot3(b"1234567\x82\xa0", Some(&lead)), None);
    assert_eq!(legal_8dot3(b"ABC.\x82", Some(&lead)), None);
    assert_eq!(legal_8dot3(b"ABC.\x82\xa0", Some(&lead)), Some(false));
    // (a trail byte is not checked: '\' after a lead byte is the pair's second half)
    assert_eq!(legal_8dot3(b"\x95\x5c", Some(&lead)), Some(false));
    assert_eq!(legal_8dot3(b"\x95\x5c", None), None);
}

/// Every illegal ASCII byte, alone and inside a name
#[test]
fn dos83_every_byte() {
    for b in 0u8..=0x7f {
        let bad = b < 0x20 || b"\"*+,/:;<=>?[\\]|".contains(&b);
        let got = legal_8dot3(&[b'A', b, b'B'], None);
        if bad {
            assert_eq!(got, None, "{b:#x}");
        } else if b != b'.' {
            assert_eq!(got, Some(b == b' '), "{b:#x}");
        }
    }
}

proptest! {
    /// a legal name (SBCS) is "."/"..", or base 1..=8 + optional ".ext" of 0..=3 with no
    /// space before the dot and no trailing space; spaces says whether it holds one
    #[test]
    fn dos83_model(v in proptest::collection::vec(prop_oneof![Just(b'A'), Just(b' '), Just(b'.'), Just(b'*')], 0..14)) {
        let got = legal_8dot3(&v, None);
        let model = if v == b"." || v == b".." {
            Some(false)
        } else {
            let dots = v.iter().filter(|&&b| b == b'.').count();
            let (base, ext) = match v.iter().position(|&b| b == b'.') {
                Some(k) => (&v[..k], Some(&v[k + 1..])),
                None => (&v[..], None),
            };
            let ok = !v.contains(&b'*') && dots <= 1 && base.len() <= 8
                && (v.is_empty() || !base.is_empty())
                && ext.is_none_or(|e| e.len() <= 3 && base.last() != Some(&b' '))
                && !matches!(v.last(), Some(b' ' | b'.'));
            ok.then(|| v.contains(&b' '))
        };
        prop_assert_eq!(got, model, "{:?}", String::from_utf8_lossy(&v));
    }
}
