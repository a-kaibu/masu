//! longshort: the name tests' rules and conversions over a made-up file system.
use super::*;
use proptest::prelude::*;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn s(v: &[u16]) -> String {
    String::from_utf16_lossy(v)
}
fn short(x: &[u16]) -> bool {
    let b: Vec<u8> = x.iter().map(|&u| u8::try_from(u).unwrap_or(0x80)).collect();
    is_short_name(x, &b, &|_| false)
}
/// C:\Program Files (PROGRA~1)\a.txt, C:\dir\LongFileName.txt (LONGFI~1.TXT)
fn fs(p: &[u16]) -> Option<Found> {
    let t = s(p).to_ascii_lowercase();
    let (long, short) = match t.as_str() {
        "c:\\program files" | "c:\\progra~1" => ("Program Files", "PROGRA~1"),
        "c:\\progra~1\\a.txt" | "c:\\program files\\a.txt" => ("a.txt", ""),
        "c:\\dir" => ("dir", ""),
        "c:\\dir\\longfilename.txt" | "c:\\dir\\longfi~1.txt" => ("LongFileName.txt", "LONGFI~1.TXT"),
        _ => return None,
    };
    Some(Found { long: w(long), short: w(short) })
}

#[test]
fn names() {
    for n in ["a", "abcdefgh", "abcdefgh.ext", "a.b", "PROGRA~1"] {
        assert!(!is_long_name(&w(n)), "{n}");
        assert!(short(&w(n)), "{n}");
    }
    for n in ["", "abcdefghi", "a.b.c", "a.extn", ".x", "abcdefghijklm"] {
        assert!(is_long_name(&w(n)), "{n:?}");
    }
    for n in ["a b", "a+b", "Program Files", "abcdefghi", ".x", "a.extn"] {
        assert!(!short(&w(n)), "{n}");
    }
    assert!(short(&w(".")) && short(&w("..")) && short(&[]));
}

#[test]
fn conversions() {
    let long = |p: &str| convert(&w(p), false, &short, &fs).map(|v| s(&v));
    let shrt = |p: &str| convert(&w(p), true, &short, &fs).map(|v| s(&v));
    assert_eq!(long("C:\\PROGRA~1\\a.txt"), Ok("C:\\Program Files\\a.txt".to_string()));
    assert_eq!(long("c:\\dir\\LONGFI~1.TXT"), Ok("c:\\dir\\LongFileName.txt".to_string()));
    assert_eq!(shrt("C:\\Program Files\\a.txt"), Ok("C:\\PROGRA~1\\a.txt".to_string()));
    assert_eq!(shrt("C:\\dir\\LongFileName.txt"), Ok("C:\\dir\\LONGFI~1.TXT".to_string()));
    // nothing to look up: as it is (separators kept)
    assert_eq!(shrt("C:\\dir\\x/y"), Ok("C:\\dir\\x/y".to_string()));
    assert_eq!(long("C:\\"), Ok("C:\\".to_string()));
    // a component not there
    assert_eq!(long("C:\\nope\\a"), Err(()));
    assert_eq!(shrt("C:\\Program Files\\missing file.txt"), Err(()));
    // UNC and \\?\ roots are skipped
    assert_eq!(skip_root(&w("\\\\srv\\share\\x")), Some(12));
    assert_eq!(skip_root(&w("\\\\?\\UNC\\srv\\share\\x")), Some(18));
    assert_eq!(skip_root(&w("\\\\?\\C:\\x")), Some(7));
    assert_eq!(skip_root(&w("\\\\.")), None);
}

proptest! {
    /// a long name never is a name IsShortName_U takes as short, for plain ASCII letters/dots
    #[test]
    fn long_not_short(n in "[a-z.]{0,14}") {
        let u = w(&n);
        if is_long_name(&u) && !u.is_empty() && u != w(".") && u != w("..") {
            prop_assert!(!short(&u), "{}", n);
        }
    }

    /// converting a path whose components are all found keeps the separators and the rest
    #[test]
    fn convert_is_identity_without_other_names(comps in prop::collection::vec("[a-z]{1,8}", 1..4)) {
        let p = format!("C:\\{}", comps.join("\\"));
        let found = |q: &[u16]| Some(Found { long: w(s(q).rsplit('\\').next().unwrap_or("")), short: Vec::new() });
        prop_assert_eq!(convert(&w(&p), false, &short, &found).map(|v| s(&v)), Ok(p.clone()));
        prop_assert_eq!(convert(&w(&p), true, &short, &found).map(|v| s(&v)), Ok(p));
    }
}
