//! Locks between processes (a store's writers, a backup's publisher, the web app's cache): each
//! on a lock file of its own, opened for writing and created when missing. Never a directory:
//! Windows' File::open opens no directory, and `masu add` failed there on the store's lock
//! (review e18bf44); nor a file opened read-only, which Windows does not lock exclusively
//! everywhere. masu-lint's file-lock rule keeps the locks here.
use anyhow::{Context, Result, ensure};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

/// An open lock file (the lock goes with the value: dropped, closed, unlocked).
pub(crate) struct LockFile(File);

impl LockFile {
    /// Opens (creates) the lock file `path`: a regular file, never a directory.
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let f = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(path)
            .with_context(|| format!("opening lock file {}", path.display()))?;
        ensure!(f.metadata()?.is_file(), "lock file {} is not a file", path.display());
        Ok(Self(f))
    }

    /// The lock file `path` if there is one (a NotFound error says there is none).
    pub(crate) fn existing(path: &Path) -> std::io::Result<Self> {
        let f = OpenOptions::new().read(true).write(true).open(path)?;
        if !f.metadata()?.is_file() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("lock file {} is not a file", path.display())));
        }
        Ok(Self(f))
    }

    /// The lock file `name` in `dir` (made when missing).
    pub(crate) fn in_dir(dir: &Path, name: &str) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        Self::open(&dir.join(name))
    }

    /// Waits for the exclusive lock.
    pub(crate) fn lock(&self) -> std::io::Result<()> {
        self.0.lock()
    }

    /// The exclusive lock if nobody holds one.
    pub(crate) fn try_lock(&self) -> Result<(), TryLockError> {
        self.0.try_lock()
    }

    /// Waits for a shared lock.
    pub(crate) fn lock_shared(&self) -> std::io::Result<()> {
        self.0.lock_shared()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The store's lock between threads standing in for processes (each opens the file anew, as
    /// another process would): one holder at a time, a directory refused, the lock file made.
    #[test]
    fn one_holder_at_a_time_and_never_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        let a = LockFile::in_dir(&store, ".store-write.lock").unwrap();
        a.lock().unwrap();
        assert!(store.join(".store-write.lock").is_file());
        let b = LockFile::in_dir(&store, ".store-write.lock").unwrap();
        assert!(matches!(b.try_lock(), Err(TryLockError::WouldBlock)));
        drop(a);
        b.try_lock().unwrap();
        assert!(LockFile::open(&store).is_err(), "a directory is no lock file");
        // (shared locks together; an exclusive one waits for them)
        let s1 = LockFile::open(&dir.path().join("shared")).unwrap();
        let s2 = LockFile::open(&dir.path().join("shared")).unwrap();
        s1.lock_shared().unwrap();
        s2.lock_shared().unwrap();
        let x = LockFile::open(&dir.path().join("shared")).unwrap();
        assert!(matches!(x.try_lock(), Err(TryLockError::WouldBlock)));
    }
}
