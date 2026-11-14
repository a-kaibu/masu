//! Conversions between floats and integers, each with its rounding stated in its name: the
//! one place these crates use `as` on numbers (std has no typed conversion for them), so a
//! call site says which rounding it means, and the tests below pin each one.
//!
//! - `to_f32` / `to_f64`: round to nearest, ties to even (IEEE 754's default); a value past
//!   the type's range (f64 to f32) is ±infinity.
//! - `trunc_*`: round toward zero, saturating at the integer type's range; NaN is 0.

/// A number that rounds to the nearest f32 (ties to even).
pub trait ToF32: Copy {
    fn rne_f32(self) -> f32;
}

/// A number that rounds to the nearest f64 (ties to even).
pub trait ToF64: Copy {
    fn rne_f64(self) -> f64;
}

/// A float that truncates (toward zero, saturating, NaN to 0) to each integer type.
pub trait Trunc: Copy {
    fn t_i8(self) -> i8;
    fn t_u8(self) -> u8;
    fn t_i16(self) -> i16;
    fn t_u16(self) -> u16;
    fn t_i32(self) -> i32;
    fn t_u32(self) -> u32;
    fn t_i64(self) -> i64;
    fn t_u64(self) -> u64;
    fn t_usize(self) -> usize;
}

macro_rules! to_float {
    ($($t:ty),*) => {$(
        #[allow(clippy::as_conversions, clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_lossless)]
        impl ToF32 for $t {
            fn rne_f32(self) -> f32 {
                self as f32
            }
        }
        #[allow(clippy::as_conversions, clippy::cast_precision_loss, clippy::cast_lossless)]
        impl ToF64 for $t {
            fn rne_f64(self) -> f64 {
                self as f64
            }
        }
    )*};
}
to_float!(u8, i8, u16, i16, u32, i32, u64, i64, usize, isize, f64);

macro_rules! trunc {
    ($($t:ty),*) => {$(
        #[allow(clippy::as_conversions, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        impl Trunc for $t {
            fn t_i8(self) -> i8 { self as i8 }
            fn t_u8(self) -> u8 { self as u8 }
            fn t_i16(self) -> i16 { self as i16 }
            fn t_u16(self) -> u16 { self as u16 }
            fn t_i32(self) -> i32 { self as i32 }
            fn t_u32(self) -> u32 { self as u32 }
            fn t_i64(self) -> i64 { self as i64 }
            fn t_u64(self) -> u64 { self as u64 }
            fn t_usize(self) -> usize { self as usize }
        }
    )*};
}
trunc!(f32, f64);

/// `x` rounded to the nearest f32, ties to even (f64 past f32's range: ±infinity)
pub fn to_f32<T: ToF32>(x: T) -> f32 {
    x.rne_f32()
}

/// `x` rounded to the nearest f64, ties to even
pub fn to_f64<T: ToF64>(x: T) -> f64 {
    x.rne_f64()
}

/// toward zero, saturating at i8's range, NaN to 0
pub fn trunc_i8<F: Trunc>(x: F) -> i8 {
    x.t_i8()
}
/// toward zero, saturating at u8's range, NaN to 0
pub fn trunc_u8<F: Trunc>(x: F) -> u8 {
    x.t_u8()
}
/// toward zero, saturating at i16's range, NaN to 0
pub fn trunc_i16<F: Trunc>(x: F) -> i16 {
    x.t_i16()
}
/// toward zero, saturating at u16's range, NaN to 0
pub fn trunc_u16<F: Trunc>(x: F) -> u16 {
    x.t_u16()
}
/// toward zero, saturating at i32's range, NaN to 0
pub fn trunc_i32<F: Trunc>(x: F) -> i32 {
    x.t_i32()
}
/// toward zero, saturating at u32's range (negatives to 0), NaN to 0
pub fn trunc_u32<F: Trunc>(x: F) -> u32 {
    x.t_u32()
}
/// toward zero, saturating at i64's range, NaN to 0
pub fn trunc_i64<F: Trunc>(x: F) -> i64 {
    x.t_i64()
}
/// toward zero, saturating at u64's range (negatives to 0), NaN to 0
pub fn trunc_u64<F: Trunc>(x: F) -> u64 {
    x.t_u64()
}
/// toward zero, saturating at usize's range (negatives to 0), NaN to 0
pub fn trunc_usize<F: Trunc>(x: F) -> usize {
    x.t_usize()
}

#[cfg(test)]
mod tests;
