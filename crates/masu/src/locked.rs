//! A lock taken without an unwrap: a lock another thread panicked holding is taken all the same
//! (what it guards here are caches and counters, each change whole before the guard drops, so
//! the panic left nothing half-done; failing every later request on it helped no one).

pub trait Locked<T: ?Sized> {
    fn locked(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T: ?Sized> Locked<T> for std::sync::Mutex<T> {
    fn locked(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub trait ReadWrite<T: ?Sized> {
    fn read_locked(&self) -> std::sync::RwLockReadGuard<'_, T>;
    fn write_locked(&self) -> std::sync::RwLockWriteGuard<'_, T>;
}

impl<T: ?Sized> ReadWrite<T> for std::sync::RwLock<T> {
    fn read_locked(&self) -> std::sync::RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn write_locked(&self) -> std::sync::RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, RwLock};

    /// A lock a panicking thread held: taken again, with what was written before the panic.
    #[test]
    fn poisoned_locks_are_taken() {
        let m = Arc::new(Mutex::new(1));
        let r = Arc::new(RwLock::new(1));
        let (m2, r2) = (m.clone(), r.clone());
        let _ = std::thread::spawn(move || {
            *m2.lock().unwrap() = 2;
            *r2.write().unwrap() = 2;
            let _g = (m2.lock(), r2.write());
            panic!("poison");
        })
        .join();
        assert!(m.is_poisoned() && r.is_poisoned());
        assert_eq!(*m.locked(), 2);
        assert_eq!(*r.read_locked(), 2);
        *r.write_locked() = 3;
        assert_eq!(*r.read_locked(), 3);
    }
}
