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
