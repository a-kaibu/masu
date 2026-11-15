//! Slots against a set model and k32's old code (copied as the oracle); place against the TEB
//! layouts' offsets.
use super::*;
use proptest::prelude::*;
use std::collections::BTreeSet;

/// k32's TlsAlloc / TlsFree bookkeeping before Slots (the oracle).
fn old_alloc(u: &mut Vec<bool>) -> u32 {
    if u.is_empty() {
        u.resize(1088, false);
        u[0] = true;
    }
    match u.iter().position(|x| !x) {
        Some(i) => {
            u[i] = true;
            i as u32
        }
        None => 0xffff_ffff,
    }
}
fn old_free(u: &mut Vec<bool>, i: u32) -> bool {
    match u.get_mut(i as usize) {
        Some(x) if *x && i != 0 => {
            *x = false;
            true
        }
        _ => false,
    }
}

proptest! {
    /// Any sequence of allocs and frees: the old code's answers, and a set's (the lowest free
    /// one, never 0, a slot only once).
    #[test]
    fn as_the_old_code_and_a_set(ops in proptest::collection::vec(prop_oneof![Just(None), (0u32..1200).prop_map(Some)], 0..3000)) {
        let (mut s, mut old, mut model) = (Slots::default(), vec![false; 1088], BTreeSet::new());
        old[0] = true;
        for o in ops {
            match o {
                None => {
                    let got = s.alloc();
                    prop_assert_eq!(got.unwrap_or(0xffff_ffff), old_alloc(&mut old));
                    let want = (1..SLOTS).find(|i| !model.contains(i));
                    prop_assert_eq!(got, want);
                    if let Some(i) = want {
                        model.insert(i);
                    }
                }
                Some(i) => {
                    let got = s.free(i);
                    prop_assert_eq!(got, old_free(&mut old, i));
                    prop_assert_eq!(got, model.remove(&i));
                }
            }
        }
    }
}

#[test]
fn places_are_the_teb_layouts() {
    // (winternl.h TEB: TlsSlots at 0xe10 / 0x1480, TlsExpansionSlots at 0xf94 / 0x1780)
    assert_eq!(place(false, 0), Place::Teb(0xe10));
    assert_eq!(place(false, 63), Place::Teb(0xe10 + 4 * 63));
    assert_eq!(place(false, 64), Place::Expansion { table: 0xf94, off: 0 });
    assert_eq!(place(false, SLOTS - 1), Place::Expansion { table: 0xf94, off: 4 * 1023 });
    assert_eq!(place(true, 1), Place::Teb(0x1488));
    assert_eq!(place(true, 65), Place::Expansion { table: 0x1780, off: 8 });
    // (no inline slot reaches the expansion pointer)
    for x64 in [false, true] {
        let Place::Teb(last) = place(x64, 63) else { panic!() };
        let Place::Expansion { table, .. } = place(x64, 64) else { panic!() };
        assert!(last + if x64 { 8 } else { 4 } <= table);
    }
}
