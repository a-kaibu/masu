//! Taking a lock without an unwrap: kome's (a poisoned lock panics, as `.lock().unwrap()` did;
//! kome/src/locks_tests.rs pins it).
pub use kome::locks::{wait, Lock, Rw};
