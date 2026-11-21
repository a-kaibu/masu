//! kome: what each masu crate kept its own copy of (and the copies drifted: three spellings of
//! "u32 as an index", two lock policies, four golden-file env vars). One definition each, each
//! pinned by its tests here; every crate uses `kome::...` directly (no re-exports).
//!
//! - [`num`]: numeric conversions named for what they do (round, truncate, keep the low bits),
//!   free functions only.
//! - [`le`]: little-endian numbers from bytes without a panic.
//! - [`locks`]: a Mutex/RwLock taken without an unwrap at every call (one policy: a poisoned
//!   lock is taken as it is).
//! - [`guid`]: GUIDs as their 16 bytes: built from fields, parsed, printed.
//! - [`rect`]: a Windows RECT and its box arithmetic (intersection, union, what is left).
//! - [`golden`]: golden-file comparison for tests (one env var rewrites them all).
//! - [`time`]: deadlines that saturate instead of panicking (`Instant + Duration`).
pub mod golden;
pub mod guid;
pub mod rect;
pub mod le;
pub mod locks;
pub mod num;
pub mod time;
