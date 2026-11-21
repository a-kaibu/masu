#![allow(clippy::as_conversions, clippy::cast_possible_truncation, clippy::cast_possible_wrap, clippy::cast_sign_loss)]
//! The low-bits helpers (koji::cvt's, moved here) are the `as` casts they replace (every source type, every value).
use super::*;
use proptest::prelude::*;

proptest! {
    /// Each helper is the `as` it replaces, for every source type it takes.
    #[test]
    fn helpers_are_the_casts(v: u128, s: i128) {
        prop_assert_eq!(lo8(v), v as u8);
        prop_assert_eq!(lo16(v), v as u16);
        prop_assert_eq!(lo32(v), v as u32);
        prop_assert_eq!(lo64(v), v as u64);
        prop_assert_eq!(us(v), v as usize);
        prop_assert_eq!(lo8(s), s as u8);
        prop_assert_eq!(lo16(s), s as u16);
        prop_assert_eq!(lo32(s), s as u32);
        prop_assert_eq!(lo64(s), s as u64);
        prop_assert_eq!(us(s), s as usize);
        let (v64, s64, s32, s16, s8) = (v as u64, s as i64, s as i32, s as i16, s as i8);
        prop_assert_eq!(lo32(v64), v64 as u32);
        prop_assert_eq!(lo16(s32), s32 as u16);
        prop_assert_eq!(lo8(s16), s16 as u8);
        prop_assert_eq!(lo32(s8), s8 as u32);
        prop_assert_eq!(lo64(s64), s64 as u64);
        prop_assert_eq!(us(s32), s32 as usize);
        let (u, i) = (v as usize, s as isize);
        prop_assert_eq!(lo32(u), u as u32);
        prop_assert_eq!(lo64(u), u as u64);
        prop_assert_eq!(lo32(i), i as u32);
        prop_assert_eq!(lo64(i), i as u64);
        prop_assert_eq!(us(i), i as usize);
    }
}

// (Low's impls, one macro since 2026-11-21: each width of each type is its `as` cast)
macro_rules! low_is_as {
    ($($name:ident: $t:ty),*) => {$(
        proptest! {
            #[test]
            fn $name(v: $t) {
                prop_assert_eq!(Low::low_u8(v), v as u8);
                prop_assert_eq!(Low::low_u16(v), v as u16);
                prop_assert_eq!(Low::low_u32(v), v as u32);
                prop_assert_eq!(Low::low_u64(v), v as u64);
                prop_assert_eq!(Low::low_i16(v), v as i16);
            }
        }
    )*};
}
low_is_as!(low_u8_is_as: u8, low_u16_is_as: u16, low_u32_is_as: u32, low_u64_is_as: u64, low_u128_is_as: u128, low_i8_is_as: i8, low_i16_is_as: i16, low_i32_is_as: i32, low_i64_is_as: i64, low_i128_is_as: i128, low_usize_is_as: usize, low_isize_is_as: isize);
