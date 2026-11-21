use super::*;
use proptest::prelude::*;

fn poison(f: impl FnOnce() + Send) {
    std::thread::scope(|s| assert!(s.spawn(f).join().is_err()));
}

/// A lock a panicking thread held: locked(), r(), w() and wait() panic too (what the panic
/// left half changed is not used); locked_anyway() takes it, with what was written before.
#[test]
fn a_poisoned_lock_is_not_taken() {
    let (m, l) = (Mutex::new(1), RwLock::new(1));
    poison(|| {
        *m.lock().unwrap() = 2;
        *l.write().unwrap() = 2;
        let _g = (m.lock(), l.write());
        panic!("held");
    });
    assert!(m.is_poisoned() && l.is_poisoned());
    let panics = |f: &mut dyn FnMut()| std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err();
    assert!(panics(&mut || drop(m.locked())));
    assert!(panics(&mut || drop(l.r())));
    assert!(panics(&mut || drop(l.w())));
    assert_eq!(*m.locked_anyway(), 2);
}

/// The panic names what the lock guards
#[test]
#[should_panic(expected = "a lock of alloc::vec::Vec<u8> was poisoned")]
fn the_panic_names_the_type() {
    let m = Mutex::new(Vec::<u8>::new());
    poison(|| {
        let _g = m.lock();
        panic!("held");
    });
    drop(m.locked());
}

/// wait() comes back with the lock after a notify.
#[test]
fn wait_returns_the_guard() {
    let (m, cv) = (Mutex::new(false), Condvar::new());
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

/// wait_timeout() comes back with the lock: timed out with no notify, not after one.
#[test]
fn wait_timeout_says_which() {
    let (m, cv) = (Mutex::new(false), Condvar::new());
    let (g, out) = wait_timeout(&cv, m.locked(), std::time::Duration::from_millis(1));
    assert!(out && !*g);
    drop(g);
    std::thread::scope(|s| {
        let mut g = m.locked();
        s.spawn(|| {
            *m.locked() = true;
            cv.notify_all();
        });
        while !*g {
            let (next, out) = wait_timeout(&cv, g, std::time::Duration::from_secs(60));
            assert!(!out);
            g = next;
        }
    });
}

/// wait_timeout() on a lock a panicking thread held panics too.
#[test]
fn wait_timeout_poisoned_panics() {
    let (m, cv) = (Mutex::new(0), Condvar::new());
    poison(|| {
        let _g = m.locked();
        panic!("poison");
    });
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let g = m.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = wait_timeout(&cv, g, std::time::Duration::from_millis(1));
    }));
    assert!(r.is_err());
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

#[test]
fn with_map_makes_the_map_once_and_survives_a_poisoned_lock() {
    static M: Mutex<Option<std::collections::HashMap<u32, u32>>> = Mutex::new(None);
    super::with_map(&M, |m| m.insert(1, 10));
    let _ = std::thread::spawn(|| {
        let _g = M.lock();
        panic!("poison it");
    })
    .join();
    assert!(M.is_poisoned());
    assert_eq!(super::with_map(&M, |m| m.get(&1).copied()), Some(10));
    assert_eq!(super::with_entry(&M, &1, |v| std::mem::replace(v, 11)), Some(10));
    assert_eq!(super::with_entry(&M, &2, |v| *v), None);
    assert_eq!(super::with_map(&M, |m| m.len()), 1);
}
