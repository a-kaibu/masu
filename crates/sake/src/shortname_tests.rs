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

/// A fresh guest folder C:\<tag> on the host, empty
fn folder(tag: &str) -> String {
    let h = crate::files::to_host(&format!("C:\\{tag}"));
    let _ = std::fs::remove_dir_all(&h);
    kura::fs::create_dir_all(&h).unwrap();
    h
}

fn make(dir: &str, name: &str, body: &str) {
    kura::fs::write(format!("{dir}/{name}"), body).unwrap();
}

/// What a guest opens by the short name
fn opened(dir: &str, short: &str) -> Option<String> {
    let h = crate::files::resolve(&format!("{}\\{short}", crate::files::host_to_win(dir)))?;
    kura::fs::read(h).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

#[test]
fn a_short_name_stays_when_another_file_is_deleted() {
    let d = folder("sn-delete");
    make(&d, "Long File Name A.txt", "a");
    make(&d, "Long File Name B.txt", "b");
    assert_eq!(short_of(&d, "Long File Name B.txt").as_deref(), Some("LONGFI~2.TXT"));
    kura::fs::remove_file(format!("{d}/Long File Name A.txt")).unwrap();
    assert_eq!(short_of(&d, "Long File Name B.txt").as_deref(), Some("LONGFI~2.TXT"));
    assert_eq!(opened(&d, "LONGFI~2.TXT").as_deref(), Some("b"));
    assert_eq!(opened(&d, "LONGFI~1.TXT"), None);
    // (a name made later takes the freed one)
    make(&d, "Long File Name C.txt", "c");
    assert_eq!(short_of(&d, "Long File Name C.txt").as_deref(), Some("LONGFI~1.TXT"));
    assert_eq!(opened(&d, "LONGFI~2.TXT").as_deref(), Some("b"));
    assert_eq!(opened(&d, "LONGFI~1.TXT").as_deref(), Some("c"));
}

#[test]
fn a_short_name_is_given_when_the_file_is_made_not_when_it_is_looked_at() {
    let d = folder("sn-unseen");
    make(&d, "Long File Name A.txt", "a");
    make(&d, "Long File Name B.txt", "b");
    // (nobody looked before the delete: B was given ~2 when it was made, as NTFS does)
    kura::fs::remove_file(format!("{d}/Long File Name A.txt")).unwrap();
    assert_eq!(short_of(&d, "Long File Name B.txt").as_deref(), Some("LONGFI~2.TXT"));
}

#[test]
fn the_table_is_no_guest_entry_and_a_folder_with_one_is_removed_when_empty() {
    let d = folder("sn-hidden");
    make(&d, "Long File Name A.txt", "a");
    let names: Vec<String> = read_dir(&d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["Long File Name A.txt"]);
    assert!(kura::fs::metadata(format!("{d}/{TABLE}")).is_ok());
    kura::fs::remove_file(format!("{d}/Long File Name A.txt")).unwrap();
    remove_dir(&d).unwrap();
    assert!(kura::fs::metadata(&d).is_err());
    // (a folder that still holds a file is not removed)
    let d = folder("sn-full");
    make(&d, "Long File Name A.txt", "a");
    assert!(remove_dir(&d).is_err());
    assert_eq!(opened(&d, "LONGFI~1.TXT").as_deref(), Some("a"));
}

/// The saves bring a folder back as its files (each written whole, in any order, with no names
/// made through the watcher): its table comes back with them.
fn reload(from: &str, tag: &str) -> String {
    let to = crate::files::to_host(&format!("C:\\{tag}"));
    let _ = std::fs::remove_dir_all(&to);
    std::fs::create_dir_all(&to).unwrap();
    let mut names: Vec<String> = std::fs::read_dir(from).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names.reverse();
    for n in names {
        std::fs::write(format!("{to}/{n}"), std::fs::read(format!("{from}/{n}")).unwrap()).unwrap();
    }
    to
}

#[test]
fn short_names_come_back_with_the_saves() {
    let d = folder("sn-save");
    make(&d, "Long File Name A.txt", "a");
    make(&d, "Long File Name B.txt", "b");
    make(&d, "Long File Name C.txt", "c");
    kura::fs::remove_file(format!("{d}/Long File Name A.txt")).unwrap();
    let r = reload(&d, "sn-save-back");
    assert_eq!(short_of(&r, "Long File Name B.txt").as_deref(), Some("LONGFI~2.TXT"));
    assert_eq!(short_of(&r, "Long File Name C.txt").as_deref(), Some("LONGFI~3.TXT"));
    assert_eq!(opened(&r, "LONGFI~3.TXT").as_deref(), Some("c"));
}

#[derive(Clone, Debug)]
enum Op {
    Make(usize),
    Remove(usize),
    Rename(usize, usize),
    Look,
    Reload,
}

/// Long names sharing a base (and one that is the first candidate itself)
const POOL: [&str; 7] = ["Long File Name 0.txt", "Long File Name 1.txt", "long file name 2.TXT", "Long File Name 3.txt", "Long File Name 4.txt", "Long File Name 5.txt", "LONGFI~1.TXT"];

/// NTFS's rule: a name made takes its first candidate that no short name kept in the folder and
/// no long name there takes; it keeps it until it is removed or renamed
fn model_make(m: &mut std::collections::BTreeMap<String, Option<String>>, name: &str) {
    let mut taken: std::collections::BTreeSet<Vec<u16>> = m.keys().map(|n| upper(&w(n))).collect();
    taken.extend(m.values().flatten().map(|s| upper(&w(s))));
    taken.insert(upper(&w(name)));
    let short = needs_short(&w(name)).then(|| {
        let mut c = Context::default();
        std::iter::from_fn(|| generate(&w(name), &mut c)).find(|cand| !taken.contains(&upper(cand))).map(|v| s(&v))
    }).flatten();
    m.insert(name.to_string(), short);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    /// Any history of names made, removed, renamed, looked at and brought back from the saves:
    /// each name's short name is the one NTFS gave it when it was made, and it opens that name.
    #[test]
    fn short_names_are_kept_through_any_history(ops in proptest::collection::vec(prop_oneof![
        (0..POOL.len()).prop_map(Op::Make),
        (0..POOL.len()).prop_map(Op::Remove),
        (0..POOL.len(), 0..POOL.len()).prop_map(|(a, b)| Op::Rename(a, b)),
        Just(Op::Look),
        Just(Op::Reload),
    ], 1..24)) {
        let mut d = folder("sn-pbt");
        let mut m = std::collections::BTreeMap::<String, Option<String>>::new();
        let mut reloads = 0u32;
        // (a name a file of the folder has, long or short: Windows opens that file by it, so no
        // second file gets it)
        let held = |m: &std::collections::BTreeMap<String, Option<String>>, n: &str| m.iter().any(|(l, s)| l.eq_ignore_ascii_case(n) || s.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(n)));
        for op in ops {
            match op {
                Op::Make(i) if !held(&m, POOL[i]) => {
                    make(&d, POOL[i], POOL[i]);
                    model_make(&mut m, POOL[i]);
                }
                Op::Remove(i) if m.contains_key(POOL[i]) => {
                    kura::fs::remove_file(format!("{d}/{}", POOL[i])).unwrap();
                    m.remove(POOL[i]);
                }
                Op::Rename(a, b) if m.contains_key(POOL[a]) && !held(&m, POOL[b]) => {
                    kura::fs::rename(format!("{d}/{}", POOL[a]), format!("{d}/{}", POOL[b])).unwrap();
                    m.remove(POOL[a]);
                    model_make(&mut m, POOL[b]);
                    // (the file under its new name holds what the old one did)
                    let short = m.get(POOL[b]).cloned().flatten();
                    if let Some(sh) = short {
                        prop_assert_eq!(opened(&d, &sh), Some(POOL[a].to_string()));
                    }
                    std::fs::write(format!("{d}/{}", POOL[b]), POOL[b]).unwrap();
                }
                Op::Reload => {
                    reloads += 1;
                    d = reload(&d, &format!("sn-pbt-{reloads}"));
                }
                Op::Make(_) | Op::Remove(_) | Op::Rename(..) | Op::Look => {}
            }
            let got: std::collections::BTreeMap<String, Option<String>> = of_dir(&d).into_iter().collect();
            prop_assert_eq!(&got, &m);
            for (n, sh) in &m {
                if let Some(sh) = sh {
                    prop_assert_eq!(opened(&d, sh), Some(n.clone()));
                }
            }
        }
    }
}
