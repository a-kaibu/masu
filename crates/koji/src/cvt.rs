//! Integer conversions that say what they do, instead of `as`: kome's (one copy for every
//! crate; kome/src/num_tests.rs pins each against `as` for every source type). `lo32(v)` keeps
//! the low 32 bits of any integer (a narrower signed value sign-extends, as `as` does); `us(v)` is
//! the low bits of any integer that fit a usize (kome's wrap_usize: a u32 whole, a 64-bit
//! register's low half on wasm32, as `as usize` did). A lo16 where a lo32 was meant gives a u16, which does not
//! type-check where a u32 is needed.
pub use kome::num::{lo16, lo32, lo64, lo8, wrap_usize as us, Low};
