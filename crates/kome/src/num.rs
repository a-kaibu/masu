//! Numeric conversions named for what they do, for what From/TryFrom can't say, as free
//! functions only (the traits behind them are sealed, so there is one way to write each):
//!
//! - `to_f32(x)` / `to_f64(x)`: round to the nearest, ties to even (f64 past f32's range: ±inf).
//! - `trunc_u8(x)` .. `trunc_usize(x)`: a float toward zero, saturating, NaN to 0.
//! - `lo8(x)` / `lo16(x)` / `lo16s(x)` / `lo32(x)` / `lo64(x)`: the low bits of any integer (two's
//!   complement; a narrower signed one sign-extends), as a register keeps them.
//! - `us(x)`: the low bits that fit a usize (a u32 whole: every guest count, size, index).
//! - `to_usize(x)`: an index that fails where it is used: the value, or usize::MAX for one no
//!   usize holds (a negative one). `u32_of(n)`: a length as u32, u32::MAX past 2^32.
//!
//! Each is exactly the `as` (or saturating try_from) it replaced: num_tests.rs pins every one
//! against `as` and num_oracle_tests.rs against definitions that use no `as`.

const _: () = assert!(usize::BITS >= u32::BITS, "an index type narrower than a guest's u32");
const _: () = assert!(usize::BITS <= u64::BITS);

use sealed::{Low, ToF32, ToF64, ToUsize, Trunc};

/// The implementations (one per type; not nameable outside, so callers use the functions).
pub(crate) mod sealed {
    /// To f32, rounded to the nearest, ties to even (exact up to 2^24 in magnitude; an f64 past
    /// f32's range: ±infinity).
    pub trait ToF32: Copy {
        fn to_f32(self) -> f32;
    }

    /// To f64, rounded to the nearest, ties to even (exact up to 2^53 in magnitude).
    pub trait ToF64: Copy {
        fn to_f64(self) -> f64;
    }

    macro_rules! to_float {
        ($($t:ty),*) => {$(
            impl ToF32 for $t {
                #[inline(always)]
                fn to_f32(self) -> f32 {
                    self as f32
                }
            }
            impl ToF64 for $t {
                #[inline(always)]
                fn to_f64(self) -> f64 {
                    self as f64
                }
            }
        )*};
    }
    to_float!(u8, i8, u16, i16, u32, i32, u64, i64, usize, isize);

    impl ToF32 for f64 {
        #[inline(always)]
        fn to_f32(self) -> f32 {
            self as f32
        }
    }
    impl ToF64 for f64 {
        #[inline(always)]
        fn to_f64(self) -> f64 {
            self
        }
    }

    /// A float to an integer: toward zero, saturating at the type's ends, NaN as 0.
    pub trait Trunc: Copy {
        fn trunc_i8(self) -> i8;
        fn trunc_u8(self) -> u8;
        fn trunc_i16(self) -> i16;
        fn trunc_u16(self) -> u16;
        fn trunc_i32(self) -> i32;
        fn trunc_u32(self) -> u32;
        fn trunc_i64(self) -> i64;
        fn trunc_u64(self) -> u64;
        fn trunc_isize(self) -> isize;
        fn trunc_usize(self) -> usize;
    }

    macro_rules! trunc {
        ($($t:ty),*) => {$(
            impl Trunc for $t {
                #[inline(always)]
                fn trunc_i8(self) -> i8 { self as i8 }
                #[inline(always)]
                fn trunc_u8(self) -> u8 { self as u8 }
                #[inline(always)]
                fn trunc_i16(self) -> i16 { self as i16 }
                #[inline(always)]
                fn trunc_u16(self) -> u16 { self as u16 }
                #[inline(always)]
                fn trunc_i32(self) -> i32 { self as i32 }
                #[inline(always)]
                fn trunc_u32(self) -> u32 { self as u32 }
                #[inline(always)]
                fn trunc_i64(self) -> i64 { self as i64 }
                #[inline(always)]
                fn trunc_u64(self) -> u64 { self as u64 }
                #[inline(always)]
                fn trunc_isize(self) -> isize { self as isize }
                #[inline(always)]
                fn trunc_usize(self) -> usize { self as usize }
            }
        )*};
    }
    trunc!(f32, f64);

    /// The low bits of an integer (two's complement: a narrower signed value sign-extends, as `as`
    /// does), as a register keeps them. One byte pick or extension per type (no u128 in between: each
    /// compiles to what the `as` did).
    pub trait Low: Copy {
        fn low_u8(self) -> u8;
        fn low_u16(self) -> u16;
        fn low_u32(self) -> u32;
        fn low_u64(self) -> u64;
        /// the low 16 bits as i16 (two's complement)
        #[inline(always)]
        fn low_i16(self) -> i16 {
            self.low_u16().cast_signed()
        }
    }

    /// The low bits of each fixed-width integer: widened to `$wide` (zero- or sign-extended,
    /// so an i8's low 16 bits are i16::from's), then its first bytes.
    macro_rules! low_via {
        ($wide:ty: $($t:ty),*) => {$(
            impl Low for $t {
                #[inline(always)]
                fn low_u8(self) -> u8 {
                    let [a, ..] = <$wide>::from(self).to_le_bytes();
                    a
                }
                #[inline(always)]
                fn low_u16(self) -> u16 {
                    let [a, b, ..] = <$wide>::from(self).to_le_bytes();
                    u16::from_le_bytes([a, b])
                }
                #[inline(always)]
                fn low_u32(self) -> u32 {
                    let [a, b, c, d, ..] = <$wide>::from(self).to_le_bytes();
                    u32::from_le_bytes([a, b, c, d])
                }
                #[inline(always)]
                fn low_u64(self) -> u64 {
                    let [a, b, c, d, e, f, g, h, ..] = <$wide>::from(self).to_le_bytes();
                    u64::from_le_bytes([a, b, c, d, e, f, g, h])
                }
            }
        )*};
    }
    low_via!(u128: u8, u16, u32, u64, u128);
    low_via!(i128: i8, i16, i32, i64, i128);

    impl Low for usize {
        #[inline(always)]
        fn low_u8(self) -> u8 {
            Ptr::from_le_bytes(self.to_le_bytes()).low_u8()
        }
        #[inline(always)]
        fn low_u16(self) -> u16 {
            Ptr::from_le_bytes(self.to_le_bytes()).low_u16()
        }
        #[inline(always)]
        fn low_u32(self) -> u32 {
            Ptr::from_le_bytes(self.to_le_bytes()).low_u32()
        }
        #[inline(always)]
        fn low_u64(self) -> u64 {
            Ptr::from_le_bytes(self.to_le_bytes()).low_u64()
        }
    }

    impl Low for isize {
        #[inline(always)]
        fn low_u8(self) -> u8 {
            SPtr::from_le_bytes(self.to_le_bytes()).low_u8()
        }
        #[inline(always)]
        fn low_u16(self) -> u16 {
            SPtr::from_le_bytes(self.to_le_bytes()).low_u16()
        }
        #[inline(always)]
        fn low_u32(self) -> u32 {
            SPtr::from_le_bytes(self.to_le_bytes()).low_u32()
        }
        #[inline(always)]
        fn low_u64(self) -> u64 {
            SPtr::from_le_bytes(self.to_le_bytes()).low_u64()
        }
    }

    // (usize/isize as the fixed-width type of their size)
    #[cfg(target_pointer_width = "64")]
    type Ptr = u64;
    #[cfg(target_pointer_width = "64")]
    type SPtr = i64;
    #[cfg(target_pointer_width = "32")]
    type Ptr = u32;
    #[cfg(target_pointer_width = "32")]
    type SPtr = i32;

    /// An integer as an index or a length: itself, or usize::MAX for one no usize holds (a
    /// negative one: an index past any slice, so it fails where it is used).
    pub trait ToUsize: Copy {
        fn to_usize(self) -> usize;
    }

    macro_rules! to_usize {
        ($($t:ty),*) => {$(
            impl ToUsize for $t {
                #[inline(always)]
                fn to_usize(self) -> usize {
                    usize::try_from(self).unwrap_or(usize::MAX)
                }
            }
        )*};
    }
    to_usize!(u8, u16, u32, i32, u64, i64, usize, isize);
}

/// `x` rounded to the nearest f32, ties to even
#[inline(always)]
pub fn to_f32<T: ToF32>(x: T) -> f32 {
    x.to_f32()
}
/// `x` rounded to the nearest f64, ties to even
#[inline(always)]
pub fn to_f64<T: ToF64>(x: T) -> f64 {
    x.to_f64()
}

macro_rules! trunc_fns {
    ($($f:ident -> $t:ty),*) => {$(
        /// toward zero, saturating at the type's range (negatives to 0 for unsigned), NaN to 0
        #[inline(always)]
        pub fn $f<F: Trunc>(x: F) -> $t {
            x.$f()
        }
    )*};
}
trunc_fns!(trunc_i8 -> i8, trunc_u8 -> u8, trunc_i16 -> i16, trunc_u16 -> u16, trunc_i32 -> i32, trunc_u32 -> u32, trunc_i64 -> i64, trunc_u64 -> u64, trunc_isize -> isize, trunc_usize -> usize);

/// the low 8 bits
#[inline(always)]
pub fn lo8<T: Low>(x: T) -> u8 {
    x.low_u8()
}
/// the low 16 bits
#[inline(always)]
pub fn lo16<T: Low>(x: T) -> u16 {
    x.low_u16()
}
/// the low 16 bits as i16 (two's complement, what `as i16` did)
#[inline(always)]
pub fn lo16s<T: Low>(x: T) -> i16 {
    x.low_i16()
}
/// the low 32 bits
#[inline(always)]
pub fn lo32<T: Low>(x: T) -> u32 {
    x.low_u32()
}
/// the low 64 bits
#[inline(always)]
pub fn lo64<T: Low>(x: T) -> u64 {
    x.low_u64()
}
/// The low bits that fit a usize (two's complement, as `as usize`): a u32 whole (the assert at
/// the top), a 64-bit register's low half on wasm32.
#[inline(always)]
pub fn us<T: Low>(x: T) -> usize {
    #[cfg(target_pointer_width = "64")]
    let w = usize::from_le_bytes(x.low_u64().to_le_bytes());
    #[cfg(target_pointer_width = "32")]
    let w = usize::from_le_bytes(x.low_u32().to_le_bytes());
    w
}
/// An index or length: the value, usize::MAX for one no usize holds (a negative one: past any
/// slice, so it fails where it is used).
#[inline(always)]
pub fn to_usize<T: ToUsize>(x: T) -> usize {
    x.to_usize()
}
/// A length or index as u32: itself, u32::MAX past 2^32 (callers' values are far below).
#[inline(always)]
pub fn u32_of(x: usize) -> u32 {
    u32::try_from(x).unwrap_or(u32::MAX)
}
/// A length as u64: lossless (the assert at the top).
#[inline(always)]
pub fn u64_of(x: usize) -> u64 {
    u64::try_from(x).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "num_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "num_oracle_tests.rs"]
mod oracle_tests;

#[cfg(test)]
#[path = "num_cvt_tests.rs"]
mod cvt_tests;

#[cfg(kani)]
mod proofs {
    use super::*;
    /// The low bits are the value mod 2^n, for every input (not only sampled ones).
    #[kani::proof]
    fn low_bits_are_the_value_mod_2_n() {
        let x: u64 = kani::any();
        assert!(u64::from(lo8(x)) == x % 256);
        assert!(u64::from(lo16(x)) == x % 65536);
        assert!(u64::from(lo32(x)) == x % (1 << 32));
        let y: i32 = kani::any();
        assert!(i32::from(lo16s(y)) == ((y & 0xffff) ^ 0x8000) - 0x8000);
    }
    #[kani::proof]
    fn indices_are_the_value_or_max() {
        let x: i64 = kani::any();
        let u = to_usize(x);
        assert!(if x < 0 { u == usize::MAX } else { u64::try_from(x).ok() == u64::try_from(u).ok() || u == usize::MAX });
        let w: u32 = kani::any();
        assert!(u64::from(w) == u64_of(us(w)));
    }
}
