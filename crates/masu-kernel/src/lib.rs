//! Pure state transitions for masu's single-library server.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod access;
pub mod jit_cache;
pub mod library;
pub mod save;
pub mod session;
pub mod store;
pub mod telemetry;
pub mod update;
