//! Named numeric conversions for what From/TryFrom can't say: each is exactly the `as` it
//! replaces (num_tests.rs pins them against `as` itself), but its name says what may happen,
//! and the casts are here once instead of at every call site.

/// To f32, rounded to the nearest (exact up to 2^24 in magnitude).
pub trait ToF32: Copy {
    fn to_f32(self) -> f32;
}

/// To f64, rounded to the nearest (exact up to 2^53 in magnitude).
pub trait ToF64: Copy {
    fn to_f64(self) -> f64;
}

macro_rules! to_float {
    ($($t:ty),*) => {$(
        impl ToF32 for $t {
            #[inline]
            fn to_f32(self) -> f32 {
                self as f32
            }
        }
        impl ToF64 for $t {
            #[inline]
            fn to_f64(self) -> f64 {
                self as f64
            }
        }
    )*};
}
to_float!(u32, i32, u64, i64, usize, isize);

impl ToF32 for f64 {
    /// rounded to the nearest f32 (out of range: infinity)
    #[inline]
    fn to_f32(self) -> f32 {
        self as f32
    }
}

/// A float to an integer: toward zero, saturating at the type's ends, NaN as 0.
pub trait Trunc: Copy {
    fn trunc_u8(self) -> u8;
    fn trunc_u16(self) -> u16;
    fn trunc_u32(self) -> u32;
    fn trunc_i32(self) -> i32;
    fn trunc_i64(self) -> i64;
    fn trunc_isize(self) -> isize;
}

macro_rules! trunc {
    ($($t:ty),*) => {$(
        impl Trunc for $t {
            #[inline]
            fn trunc_u8(self) -> u8 {
                self as u8
            }
            #[inline]
            fn trunc_u16(self) -> u16 {
                self as u16
            }
            #[inline]
            fn trunc_u32(self) -> u32 {
                self as u32
            }
            #[inline]
            fn trunc_i32(self) -> i32 {
                self as i32
            }
            #[inline]
            fn trunc_i64(self) -> i64 {
                self as i64
            }
            #[inline]
            fn trunc_isize(self) -> isize {
                self as isize
            }
        }
    )*};
}
trunc!(f32, f64);

/// The low bits of an integer (two's complement), as a register keeps them.
pub trait Low: Copy {
    fn low_u8(self) -> u8;
    fn low_u16(self) -> u16;
    fn low_u32(self) -> u32;
}

macro_rules! low {
    ($($t:ty),*) => {$(
        impl Low for $t {
            #[inline]
            fn low_u8(self) -> u8 {
                self.to_le_bytes()[0]
            }
            #[inline]
            fn low_u16(self) -> u16 {
                let b = self.to_le_bytes();
                u16::from_le_bytes([b[0], b[1]])
            }
            #[inline]
            fn low_u32(self) -> u32 {
                let b = self.to_le_bytes();
                u32::from_le_bytes([b[0], b[1], b[2], b[3]])
            }
        }
    )*};
}
low!(u32, i32, u64, i64, usize, isize);

#[cfg(test)]
#[path = "num_tests.rs"]
mod tests;
