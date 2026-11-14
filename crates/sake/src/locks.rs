//! Taking a lock: `m.locked()` for a Mutex, `l.r()` / `l.w()` for an RwLock. A lock poisoned by a
//! panic on another thread panics here as `.lock().unwrap()` did (state left half-changed is not
//! read on); the one place that says so, instead of an unwrap at every lock.
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

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
#[track_caller]
#[cold]
fn poisoned(e: &dyn std::fmt::Display) -> ! {
    panic!("sake: a lock poisoned by a panic on another thread: {e}")
}

#[cfg(test)]
#[path = "locks_tests.rs"]
mod tests;
