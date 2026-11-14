use super::*;
use proptest::prelude::*;

fn poison(f: impl FnOnce() + Send) {
    std::thread::scope(|s| assert!(s.spawn(f).join().is_err()));
}

#[test]
fn a_poisoned_mutex_still_panics() {
    let m = Mutex::new(1);
    poison(|| {
        let _g = m.locked();
        panic!("held");
    });
    assert!(std::panic::catch_unwind(|| *m.locked()).is_err());
}

#[test]
fn a_poisoned_rwlock_still_panics() {
    let l = RwLock::new(1);
    poison(|| {
        let _g = l.w();
        panic!("held");
    });
    assert!(std::panic::catch_unwind(|| *l.r()).is_err());
    assert!(std::panic::catch_unwind(|| *l.w()).is_err());
}

proptest! {
    // (the same values as .lock().unwrap() through any sequence of writes from threads)
    #[test]
    fn locked_sees_what_lock_unwrap_sees(adds in proptest::collection::vec(0u32..1000, 0..40)) {
        let (a, b) = (Mutex::new(0u64), RwLock::new(0u64));
        std::thread::scope(|s| {
            for &x in &adds {
                let (a, b) = (&a, &b);
                s.spawn(move || {
                    *a.locked() += u64::from(x);
                    *b.w() += u64::from(x);
                });
            }
        });
        let want: u64 = adds.iter().map(|&x| u64::from(x)).sum();
        prop_assert_eq!(*a.lock().unwrap(), want);
        prop_assert_eq!(*a.locked(), want);
        prop_assert_eq!(*b.r(), want);
    }
}
