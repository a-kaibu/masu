//! Conversions between floats and integers with the rounding in the name: kome's (to_f32 /
//! to_f64 round to nearest even, trunc_* toward zero saturating, NaN 0; kome's num tests pin
//! each against definitions that use no `as`).
pub use kome::num::{to_f32, to_f64, trunc_i16, trunc_i32, trunc_i64, trunc_i8, trunc_u16, trunc_u32, trunc_u64, trunc_u8, trunc_usize, ToF32, ToF64, Trunc};
