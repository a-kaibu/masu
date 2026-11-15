//! Taking a lock without an unwrap: `m.locked()` for a Mutex, `l.r()` / `l.w()` for an RwLock,
//! `wait(cv, g)` for a Condvar. One policy everywhere: a lock another thread panicked holding is
//! taken all the same (a browser build aborts on a panic, so poisoning only happens in tests and
//! the server, whose state each change leaves whole).
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

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

/// Condvar::wait, a poisoned lock taken as above.
#[inline]
pub fn wait<'a, T>(cv: &Condvar, g: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    cv.wait(g).unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
#[path = "locks_tests.rs"]
mod tests;
