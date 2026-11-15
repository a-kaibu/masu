//! Taking a lock without an unwrap at every call: `m.locked()` for a Mutex, `l.r()` / `l.w()`
//! for an RwLock, `wait(cv, g)` for a Condvar. A lock another thread panicked holding is NOT
//! taken (review 2026-11-15: taking it anyway used whatever the panic left half changed): the
//! caller panics too, naming the lock's type, as `.lock().unwrap()` would. The browser build
//! aborts on a panic, so this only shows in tests and the server, where a request that panics
//! ends that request and every later one that needs the same state, instead of serving from it.
//! State that each change leaves whole (a counter, a log line) may be taken poisoned or not
//! with `locked_anyway()`, named at the call so that each such choice is a reviewed one.
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

#[cold]
#[inline(never)]
#[track_caller]
fn poisoned<T: ?Sized>() -> ! {
    panic!("a lock of {} was poisoned: a thread panicked while changing what it guards", std::any::type_name::<T>())
}

pub trait Lock<T: ?Sized> {
    /// The lock; a panic if a thread panicked holding it
    #[track_caller]
    fn locked(&self) -> MutexGuard<'_, T>;
    /// The lock, poisoned or not: only for state that every change leaves whole
    fn locked_anyway(&self) -> MutexGuard<'_, T>;
}
impl<T: ?Sized> Lock<T> for Mutex<T> {
    #[inline]
    #[track_caller]
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|_| poisoned::<T>())
    }
    #[inline]
    fn locked_anyway(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub trait Rw<T: ?Sized> {
    #[track_caller]
    fn r(&self) -> RwLockReadGuard<'_, T>;
    #[track_caller]
    fn w(&self) -> RwLockWriteGuard<'_, T>;
}
impl<T: ?Sized> Rw<T> for RwLock<T> {
    #[inline]
    #[track_caller]
    fn r(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(|_| poisoned::<T>())
    }
    #[inline]
    #[track_caller]
    fn w(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(|_| poisoned::<T>())
    }
}

/// Condvar::wait; a panic if the lock was poisoned meanwhile (as `locked`)
#[inline]
#[track_caller]
pub fn wait<'a, T>(cv: &Condvar, g: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    cv.wait(g).unwrap_or_else(|_| poisoned::<T>())
}

#[cfg(test)]
#[path = "locks_tests.rs"]
mod tests;
