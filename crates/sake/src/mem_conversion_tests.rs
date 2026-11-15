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
        prop_assert_eq!(bytes(a, 2), &(v as u16).to_le_bytes()[..]);
        w32(a, v as u32);
        prop_assert_eq!(r32(a), v as u32);
        w64(a, v);
        prop_assert_eq!(r64(a), v);
        prop_assert_eq!(bytes(a, 8), &v.to_le_bytes()[..]);
        prop_assert!(bytes(p, off).iter().all(|&b| b == 0) && bytes(a + 8, 0x200 - off - 8).iter().all(|&b| b == 0));
        fill(a, 8, 0xab);
        prop_assert!(bytes(a, 8).iter().all(|&b| b == 0xab));
        copy(a + 16, a, 8);
        prop_assert_eq!(bytes(a + 16, 8), bytes(a, 8));
        zero(a, 0x40);
        prop_assert!(bytes(p, 0x200).iter().all(|&b| b == 0));
        write(a, b"sake\0");
        prop_assert_eq!(cstr(a), b"sake".to_vec());
        heap_free(p);
    }
}


proptest! {
    // (Ga's arithmetic is u32's wrapping arithmetic; end() the unwrapped sum)
    #[test]
    fn ga_is_wrapping_u32(a in any::<u32>(), o in any::<u32>(), i in any::<u32>(), size in 0u32..64) {
        prop_assert_eq!((ga(a) + o).get(), a.wrapping_add(o));
        prop_assert_eq!((ga(a) - o).get(), a.wrapping_sub(o));
        prop_assert_eq!(ga(a).at(i, size).get(), a.wrapping_add(i.wrapping_mul(size)));
        prop_assert_eq!(ga(a).since(ga(o)), a.wrapping_sub(o));
        prop_assert_eq!(u128::from(ga(a).end(o)), u128::from(a) + u128::from(o));
        let mut x = ga(a);
        x += o;
        prop_assert_eq!(u32::from(x), a.wrapping_add(o));
    }
}
