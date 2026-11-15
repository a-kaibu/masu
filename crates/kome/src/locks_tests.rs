use super::*;
use proptest::prelude::*;

fn poison(f: impl FnOnce() + Send) {
    std::thread::scope(|s| assert!(s.spawn(f).join().is_err()));
}

/// A lock a panicking thread held: taken again, with what was written before the panic.
#[test]
fn a_poisoned_lock_is_taken() {
    let (m, l) = (Mutex::new(1), RwLock::new(1));
    poison(|| {
        *m.lock().unwrap() = 2;
        *l.write().unwrap() = 2;
        let _g = (m.lock(), l.write());
        panic!("held");
    });
    assert!(m.is_poisoned() && l.is_poisoned());
    assert_eq!(*m.locked(), 2);
    assert_eq!(*l.r(), 2);
    *l.w() = 3;
    assert_eq!(*l.r(), 3);
}

/// wait() comes back with the lock after a notify, poisoned or not.
#[test]
fn wait_returns_the_guard() {
    let (m, cv) = (Mutex::new(false), Condvar::new());
    poison(|| {
        let _g = m.lock();
        panic!("held");
    });
    std::thread::scope(|s| {
        s.spawn(|| {
            *m.locked() = true;
            cv.notify_all();
        });
        let mut g = m.locked();
        while !*g {
            g = wait(&cv, g);
        }
    });
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
