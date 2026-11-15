//! Taking a lock: `m.locked()` for a Mutex, `l.r()` / `l.w()` for an RwLock. A lock poisoned by a
//! panic on another thread panics here as `.lock().unwrap()` did (state left half-changed is not
//! read on); the one place that says so, instead of an unwrap at every lock.
//!
//! [`tolerant`] has the same methods taking a poisoned lock all the same: for state each change
//! leaves whole (a server's caches and counters), where failing every later request helps no one.
//! A crate picks its policy by which module it imports.
use std::sync::{Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub trait Lock<T: ?Sized> {
    fn locked(&self) -> MutexGuard<'_, T>;
}
impl<T: ?Sized> Lock<T> for Mutex<T> {
    #[track_caller]
    #[inline]
    fn locked(&self) -> MutexGuard<'_, T> {
        match self.lock() {
            Ok(g) => g,
            Err(e) => poisoned(&e),
        }
    }
}
pub trait Rw<T: ?Sized> {
    fn r(&self) -> RwLockReadGuard<'_, T>;
    fn w(&self) -> RwLockWriteGuard<'_, T>;
}
impl<T: ?Sized> Rw<T> for RwLock<T> {
    #[track_caller]
    #[inline]
    fn r(&self) -> RwLockReadGuard<'_, T> {
        match self.read() {
            Ok(g) => g,
            Err(e) => poisoned(&e),
        }
    }
    #[track_caller]
    #[inline]
    fn w(&self) -> RwLockWriteGuard<'_, T> {
        match self.write() {
            Ok(g) => g,
            Err(e) => poisoned(&e),
        }
    }
}
/// Condvar::wait, a poisoned lock panicking as above.
#[track_caller]
pub fn wait<'a, T>(cv: &Condvar, g: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    match cv.wait(g) {
        Ok(g) => g,
        Err(e) => poisoned(&e),
    }
}
#[track_caller]
#[cold]
fn poisoned(e: &dyn std::fmt::Display) -> ! {
    panic!("a lock poisoned by a panic on another thread: {e}")
}

/// The same `locked()` / `r()` / `w()`, a poisoned lock taken as it is.
pub mod tolerant {
    use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

    pub trait Lock<T: ?Sized> {
        fn locked(&self) -> MutexGuard<'_, T>;
    }
    impl<T: ?Sized> Lock<T> for Mutex<T> {
        #[inline]
        fn locked(&self) -> MutexGuard<'_, T> {
            self.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }
    pub trait Rw<T: ?Sized> {
        fn r(&self) -> RwLockReadGuard<'_, T>;
        fn w(&self) -> RwLockWriteGuard<'_, T>;
    }
    impl<T: ?Sized> Rw<T> for RwLock<T> {
        #[inline]
        fn r(&self) -> RwLockReadGuard<'_, T> {
            self.read().unwrap_or_else(PoisonError::into_inner)
        }
        #[inline]
        fn w(&self) -> RwLockWriteGuard<'_, T> {
            self.write().unwrap_or_else(PoisonError::into_inner)
        }
    }
}

#[cfg(test)]
#[path = "locks_tests.rs"]
mod tests;
