//! shortname: names as Windows' NTFS gives them (well-known ones: PROGRA~1, PROGRA~2,
//! DOCUME~1, the checksum forms) and laws over generated names.
use super::*;
use proptest::prelude::*;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn s(v: &[u16]) -> String {
    String::from_utf16_lossy(v)
}
fn gen_n(name: &str, n: usize) -> Vec<String> {
    let mut c = Context::default();
    (0..n).filter_map(|_| generate(&w(name), &mut c)).map(|v| s(&v)).collect()
}

#[test]
fn known_names() {
    assert_eq!(gen_n("Program Files", 2), ["PROGRA~1", "PROGRA~2"]);
    assert_eq!(gen_n("Documents and Settings", 1), ["DOCUME~1"]);
    assert_eq!(gen_n("longfilename.longext", 1), ["LONGFI~1.LON"]);
    // (a base of 2 or fewer gets the checksum at once)
    let a: String = hex4(checksum(&w("a.html"))).iter().map(|&u| char::from(u8::try_from(u).unwrap_or(b'?'))).collect();
    assert_eq!(gen_n("a.html", 1), [format!("A{a}~1.HTM")]);
    assert_eq!(gen_n("my file.txt", 1), ["MYFILE~1.TXT"]);
    assert_eq!(gen_n(".bashrc", 1), ["BASHRC~1"]);
    assert_eq!(gen_n("a+b=c.txt", 1), ["A_B_C~1.TXT"]);
    assert_eq!(gen_n("x.tar.gz", 1), ["XTAR~1.GZ"]);
    // the 5th is the checksum form, from ~1 again
    let v = gen_n("Program Files", 6);
    assert_eq!(&v[..4], ["PROGRA~1", "PROGRA~2", "PROGRA~3", "PROGRA~4"]);
    let h: String = hex4(checksum(&w("Program Files"))).iter().map(|&u| char::from(u8::try_from(u).unwrap_or(b'?'))).collect();
    assert_eq!(v[4], format!("PR{h}~1"));
    assert_eq!(v[5], format!("PR{h}~2"));
    // a short base gets the checksum at once
    assert_eq!(gen_n("a b", 1).first().map(|n| n.len()), Some(8));
}

#[test]
fn needs() {
    for n in ["Program Files", "longfilename", "a b.txt", "a.html", ".x", "a+b"] {
        assert!(needs_short(&w(n)), "{n}");
    }
    for n in ["README.TXT", "readme.txt", "a", "x.y", ".", ".."] {
        assert!(!needs_short(&w(n)), "{n}");
    }
}

#[test]
fn assign_in_creation_order() {
    let e = vec![(w("Program Files (x86)"), 7), (w("Program Files"), 3), (w("PROGRA~1"), 9)];
    // (a long name that is itself PROGRA~1 takes it)
    let got: Vec<Option<String>> = assign(&e).into_iter().map(|o| o.map(|v| s(&v))).collect();
    assert_eq!(got, [Some("PROGRA~3".to_string()), Some("PROGRA~2".to_string()), None]);
}

proptest! {
    /// every candidate is a legal 8.3 name; the ones a name gets are all different
    #[test]
    fn candidates_are_83(name in "[A-Za-z0-9 .+_~-]{0,19}[A-Za-z0-9+_~-]", n in 1usize..12) {
        // (Win32 names do not end in '.' or ' ': CreateFile drops them)
        let v = gen_n(&name, n);
        for c in &v {
            let u = w(c);
            prop_assert!(matches!(crate::ntpath::name_8dot3(&u, 12).1, Some(false)), "{} -> {}", name, c);
        }
        let set: std::collections::BTreeSet<&String> = v.iter().collect();
        prop_assert_eq!(set.len(), v.len());
    }

    /// assign gives distinct short names that no long name takes
    #[test]
    fn assign_distinct(names in prop::collection::btree_set("[a-z]{1,10}( [a-z]{1,4})?(\\.[a-z]{1,5})?", 1..12)) {
        let e: Vec<(Vec<u16>, u64)> = names.iter().enumerate().map(|(i, n)| (w(n), kome::num::u32_of(i).into())).collect();
        let a = assign(&e);
        let longs: std::collections::BTreeSet<String> = names.iter().map(|n| n.to_ascii_uppercase()).collect();
        let mut seen = std::collections::BTreeSet::new();
        for (k, sh) in a.iter().enumerate() {
            let need = needs_short(&e[k].0);
            prop_assert_eq!(sh.is_some(), need);
            if let Some(sh) = sh {
                prop_assert!(!longs.contains(&s(sh)));
                prop_assert!(seen.insert(s(sh)));
            }
        }
    }
}
