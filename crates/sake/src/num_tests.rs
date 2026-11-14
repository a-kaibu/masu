//! Every named conversion against the `as` it stands for: edge values and random ones.
#![allow(clippy::as_conversions, clippy::cast_possible_truncation, clippy::cast_possible_wrap, clippy::cast_sign_loss, clippy::cast_precision_loss)]
use super::*;
use proptest::prelude::*;

fn same_f32(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits()
}

macro_rules! ints_as {
    ($name:ident, $t:ty) => {
        proptest! {
            #![proptest_config(ProptestConfig::with_cases(2000))]
            #[test]
            fn $name(v: $t, small in -70000i64..70000) {
                for v in [v, small as $t, <$t>::MIN, <$t>::MAX, 0 as $t, (1u64 << 24) as $t, ((1u64 << 24) + 1) as $t] {
                    prop_assert!(same_f32(v.to_f32(), v as f32));
                    prop_assert_eq!(v.to_f64().to_bits(), (v as f64).to_bits());
                    prop_assert_eq!(v.low_u8(), v as u8);
                    prop_assert_eq!(v.low_u16(), v as u16);
                    prop_assert_eq!(v.low_u32(), v as u32);
                }
            }
        }
    };
}
ints_as!(u32_as, u32);
ints_as!(i32_as, i32);
ints_as!(u64_as, u64);
ints_as!(i64_as, i64);
ints_as!(usize_as, usize);
ints_as!(isize_as, isize);

const EDGES: [f64; 14] = [0.0, -0.0, 0.5, -0.5, 255.9, 256.0, -1.0, 65535.5, 4294967295.9, 4294967296.0, 1e30, -1e30, f64::INFINITY, f64::NEG_INFINITY];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4000))]
    #[test]
    fn floats_as(bits: u64, small in -70000f64..70000.0) {
        for x in EDGES.into_iter().chain([f64::from_bits(bits), small, f64::NAN]) {
            prop_assert_eq!(x.trunc_u8(), x as u8);
            prop_assert_eq!(x.trunc_u16(), x as u16);
            prop_assert_eq!(x.trunc_u32(), x as u32);
            prop_assert_eq!(x.trunc_i32(), x as i32);
            prop_assert_eq!(x.trunc_i64(), x as i64);
            prop_assert_eq!(x.trunc_isize(), x as isize);
            prop_assert_eq!(x.trunc_usize(), x as usize);
            prop_assert!(same_f32(x.to_f32(), x as f32) || x.is_nan());
            let y = x as f32;
            prop_assert_eq!(y.trunc_u8(), y as u8);
            prop_assert_eq!(y.trunc_u16(), y as u16);
            prop_assert_eq!(y.trunc_u32(), y as u32);
            prop_assert_eq!(y.trunc_i32(), y as i32);
            prop_assert_eq!(y.trunc_i64(), y as i64);
            prop_assert_eq!(y.trunc_isize(), y as isize);
            prop_assert_eq!(y.trunc_usize(), y as usize);
        }
    }
}

proptest! {
    /// what `as usize` gives for the values a usize holds, usize::MAX past them
    #[test]
    fn indices(a: u32, b: i32, c: u64, d: i64) {
        prop_assert_eq!(a.to_usize(), a as usize);
        prop_assert_eq!(b.to_usize(), if b >= 0 { b as usize } else { usize::MAX });
        prop_assert_eq!(c.to_usize(), c as usize);
        prop_assert_eq!(d.to_usize(), if d >= 0 { d as usize } else { usize::MAX });
    }
}
