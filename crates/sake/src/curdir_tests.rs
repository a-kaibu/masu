//! curdir: RtlpCheckForSameCurdir's rule (sake-re/ntdll/curdir.md; Wine's ntdll:path
//! test_RtlSetCurrentDirectory_U) and its laws.
use super::*;
use proptest::prelude::*;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn made(s: &str) -> (Vec<u16>, u16) {
    let u = w(s);
    let len = kome::num::lo16(u.len() * 2);
    (u, len)
}

#[test]
fn wine_cases() {
    let (u, len) = made("C:\\users\\me\\");
    assert!(same(&u, len, &w("C:\\users\\me")));
    assert!(!same(&u, len, &w("C:\\users\\me\\")));
    assert!(!same(&u, len, &w(".")));
    assert!(!same(&u, len, &w("\\\\?\\C:\\users\\me")));
    // (case matters)
    assert!(!same(&u, len, &w("c:\\users\\me")));
    // (the program wrote a 0 over the buffer's first unit: not the same any more)
    let mut z = u.clone();
    z[0] = 0;
    assert!(!same(&z, len, &w("C:\\users\\me")));
    // a root: the whole of it
    let (r, rl) = made("C:\\");
    assert!(same(&r, rl, &w("C:\\")));
    assert!(!same(&r, rl, &w("C:")));
    // (an emptied Length)
    assert!(!same(&u, 0, &w("C:\\users\\me")));
    assert!(same(&u, 0, &[]));
}

proptest! {
    /// past a root, exactly the made path less its last unit is the same, and nothing longer or
    /// shorter
    #[test]
    fn only_the_path_without_its_last_unit(s in "[A-Za-z]:\\\\[a-z\\\\]{2,20}", extra in "[a-z]{0,3}") {
        let (u, len) = made(&s);
        let cut = &u[..u.len() - 1];
        prop_assert!(same(&u, len, cut));
        let mut longer = cut.to_vec();
        longer.extend(w(&extra));
        prop_assert_eq!(same(&u, len, &longer), extra.is_empty());
        prop_assert!(!same(&u, len, &u));
    }
}
