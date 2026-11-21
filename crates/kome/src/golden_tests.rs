use super::*;
use super::Rng;
use proptest::prelude::{any, prop_assert, prop_assert_eq, prop_assert_ne, proptest};

/// sake's test_guest::Rng and koji's golden Rng as they were (the goldens they made depend on
/// the sequence): kome's must be the same one.
struct OldRng(u64);
impl OldRng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

/// the copies' FNV-1a loop as they were
fn old_fnv(h: &mut u64, b: &[u8]) {
    for &x in b {
        *h ^= u64::from(x);
        *h = h.wrapping_mul(0x100_0000_01b3);
    }
}

#[test]
fn fnv_known_answers() {
    // (the FNV reference test vectors)
    assert_eq!(Fnv::of(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(Fnv::of(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(Fnv::of(b"foobar"), 0x8594_4171_f739_67e8);
}

proptest! {
    #[test]
    fn rng_is_the_old_sequence(seed in 1u64.., n in 0usize..64) {
        let (mut a, mut b) = (Rng(seed), OldRng(seed));
        for _ in 0..n {
            prop_assert_eq!(a.next(), b.next());
        }
    }

    #[test]
    fn fnv_is_the_old_loop(start: u64, parts in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..40), 0..8)) {
        let (mut a, mut h) = (Fnv(start), start);
        for p in &parts {
            a.bytes(p);
            old_fnv(&mut h, p);
        }
        prop_assert_eq!(a.get(), h);
        let words: Vec<u32> = parts.iter().map(|p| u32::try_from(p.len()).unwrap()).collect();
        let mut bytes = Vec::new();
        for w in &words {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        prop_assert_eq!(Fnv::new().words(words.iter().copied()).get(), Fnv::of(&bytes));
    }

    #[test]
    fn ranges_hold(seed: u64, n in 0u64..1000, len in 0usize..50) {
        let mut r = Rng::new(seed);
        prop_assert!(r.below(n) < n.max(1));
        prop_assert!(r.index(len) < len.max(1));
        let v: Vec<usize> = (0..len).collect();
        if len > 0 {
            prop_assert!(r.pick(&v) < len);
        }
        prop_assert_eq!(r.bytes(len).len(), len);
        prop_assert_ne!(Rng::new(0).next(), 0);
    }

    /// No difference exactly when the texts are the same; else at least one line of it.
    #[test]
    fn diff_is_empty_only_for_the_same_text(a in "[ab\\n]{0,30}", b in "[ab\\n]{0,30}") {
        prop_assert_eq!(diff(&a, &b).is_empty(), a == b);
    }
}

#[test]
fn compare_reads_the_file() {
    // (MASU_UPDATE_GOLDEN is not set in the test runs that check goldens)
    if updating() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("kome-golden-{}", std::process::id()));
    let p = dir.join("x.txt");
    assert!(compare(&p, "x\n").is_err(), "a missing golden fails");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&p, "x\ny\n").unwrap();
    assert!(compare(&p, "x\ny\n").is_ok());
    let e = compare(&p, "x\nz\n").unwrap_err();
    assert!(e.contains("2:\n- y\n+ z") && e.contains(UPDATE), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// asf's, mpeg1's, vorbis's and wmcodec's damage() as they were (their pin tables hash its output)
fn old_damage(pkts: &[Vec<u8>], seed: u64) -> Vec<Vec<u8>> {
    let mut r = OldRng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut below = |n: usize| usize::try_from(r.next() % u64::try_from(n.max(1)).unwrap()).unwrap();
    let mut p = pkts.to_vec();
    if p.is_empty() {
        return p;
    }
    for _ in 0..1 + below(6) {
        let i = below(p.len());
        if !p[i].is_empty() {
            let k = below(p[i].len());
            p[i][k] ^= 1 << below(8);
        }
    }
    match below(6) {
        0 => {
            let i = below(p.len());
            let n = below(p[i].len());
            p[i].truncate(n);
        }
        1 => {
            p.remove(below(p.len()));
        }
        2 => {
            let i = below(p.len());
            p.insert(i, p[i].clone());
        }
        3 => {
            let (i, j) = (below(p.len()), below(p.len()));
            p.swap(i, j);
        }
        _ => {}
    }
    p
}

proptest! {
    #[test]
    fn damage_is_the_old_one(seed: u64, pieces in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..12), 0..6)) {
        prop_assert_eq!(damage(&pieces, seed), old_damage(&pieces, seed));
    }

    #[test]
    fn fnv_u64_is_its_bytes(start: u64, v: u64, n: usize) {
        prop_assert_eq!(Fnv(start).u64(v).get(), Fnv(start).bytes(&v.to_le_bytes()).get());
        prop_assert_eq!(Fnv(start).usize(n).get(), Fnv(start).bytes(&u64::try_from(n).unwrap().to_le_bytes()).get());
    }
}

#[test]
fn pins_compare_and_name_what_changed() {
    let dir = std::env::temp_dir().join(format!("kome-pins-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t = dir.join("pin.tsv");
    std::fs::write(&t, "# c\na\t0000000000000001\nb\t00000000000000ff\n").unwrap();
    let got: std::collections::BTreeMap<String, u64> = [("a".to_owned(), 1), ("b".to_owned(), 255)].into();
    if !updating() {
        pins(&t, &got);
        let changed: std::collections::BTreeMap<String, u64> = [("b".to_owned(), 254)].into();
        let e = std::panic::catch_unwind(|| pins(&t, &changed)).unwrap_err();
        let msg = e.downcast_ref::<String>().unwrap();
        assert!(msg.contains("b: 00000000000000fe, pinned Some(\"00000000000000ff\")"), "{msg}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
