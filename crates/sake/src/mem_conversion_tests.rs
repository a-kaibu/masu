//! The range checks and accessors against a model in u128 (no wrapping), pinned before their
//! casts became typed conversions: any address and length.
use super::*;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]
    #[test]
    fn range_checks_as_the_model(a: u32, n: u32) {
        let end = u128::from(a) + u128::from(n);
        prop_assert_eq!(in_guest(a, n), end <= u128::from(GUEST_TOP));
        prop_assert_eq!(guest_range(a, n), n == 0 || (a >= NULL_TOP && end <= u128::from(GUEST_TOP)));
    }

    /// What w8/w16/w32/w64 write r8..r64 read back (little-endian, unaligned), and the
    /// bytes around stay
    #[test]
    fn accessors_round_trip(off in 0u32..0x100, v: u64) {
        let _g = crate::test_guest::enter();
        let p = heap_alloc(0x200, true);
        let a = p + off;
        w8(a, v as u8);
        prop_assert_eq!(r8(a), v as u8);
        w16(a, v as u16);
        prop_assert_eq!(r16(a), v as u16);
        prop_assert_eq!(slice(a, 2), &(v as u16).to_le_bytes()[..]);
        w32(a, v as u32);
        prop_assert_eq!(r32(a), v as u32);
        w64(a, v);
        prop_assert_eq!(r64(a), v);
        prop_assert_eq!(slice(a, 8), &v.to_le_bytes()[..]);
        prop_assert!(slice(p, off).iter().all(|&b| b == 0) && slice(a + 8, 0x200 - off - 8).iter().all(|&b| b == 0));
        fill(a, 8, 0xab);
        prop_assert!(slice(a, 8).iter().all(|&b| b == 0xab));
        copy(a + 16, a, 8);
        prop_assert_eq!(slice(a + 16, 8), slice(a, 8));
        zero(a, 0x40);
        prop_assert!(slice(p, 0x200).iter().all(|&b| b == 0));
        write(a, b"sake\0");
        prop_assert_eq!(cstr(a), b"sake".to_vec());
        heap_free(p);
    }
}

proptest! {
    // (len32 is `as u32` wherever the length fits, and saturates past it)
    #[test]
    fn len32_is_the_cast_below_4g(n in any::<usize>()) {
        if n <= u32::MAX as usize {
            prop_assert_eq!(len32(n), n as u32);
        } else {
            prop_assert_eq!(len32(n), u32::MAX);
        }
    }
}

proptest! {
    // (lo16/lo8 are `as u16`/`as u8`: the low bits)
    #[test]
    fn lo16_lo8_are_the_truncating_casts(x in any::<u32>()) {
        prop_assert_eq!(lo16(x), x as u16);
        prop_assert_eq!(lo8(x), x as u8);
    }
}
