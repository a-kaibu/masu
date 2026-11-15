//! The range checks and accessors against a model in u128 (no wrapping), pinned before their
//! casts became typed conversions: any address and length.
use super::*;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]
    #[test]
    fn range_checks_as_the_model(a: u32, n: u32) {
        let end = u128::from(a) + u128::from(n);
        // (one rule for reading and writing, review 2026-11-15)
        let ok = n == 0 || (a >= NULL_TOP && end <= u128::from(GUEST_TOP));
        prop_assert_eq!(range(a, n).is_some(), ok);
        prop_assert_eq!(range(a, n).map(|r| (r.addr(), r.len())), ok.then_some((a, n)));
        prop_assert_eq!(range_of(a, n as usize), range(a, n));
    }

    /// A host length past 4 GiB is never a range (write() checked it cut to 32 bits)
    #[test]
    fn long_host_lengths_are_no_range(a: u32, hi in 1u64..4, lo: u32) {
        let n = (hi << 32 | u64::from(lo)) as usize;
        prop_assert!(range_of(a, n).is_none());
    }

    /// Every accessor given memory that is not the guest's (the NULL pages, sake's own above
    /// GUEST_TOP, a range that runs past it or wraps) reads 0 / nothing and writes nothing.
    /// Natively only LO..GUEST_TOP is mapped (test_guest): an access the rule let through would
    /// crash the test; and guest memory just inside stays as it was.
    #[test]
    fn nothing_outside_the_guests_memory_is_touched(bad in prop_oneof![0u32..NULL_TOP, GUEST_TOP..=u32::MAX], n in 1u32..64, v: u64) {
        let _g = crate::test_guest::enter();
        let p = heap_alloc(0x100, true);
        let edge = GUEST_TOP - 2;
        prop_assert_eq!((r8(bad), r16(bad), r32(bad), r64(bad)), (0, 0, 0, 0));
        prop_assert_eq!(rp(bad), 0);
        prop_assert!(bytes(bad, n).is_empty());
        prop_assert_eq!(array::<4>(bad), [0; 4]);
        prop_assert!(view(bad, n, |s| s.is_empty()));
        prop_assert!(view_mut(bad, n, |_| ()).is_none());
        prop_assert!(host_ptr(bad, n).is_none() && host_ptr_mut(bad, n).is_none());
        w8(bad, v as u8);
        w16(bad, v as u16);
        w32(bad, v as u32);
        w64(bad, v);
        wp(bad, v as u32);
        put32(bad, 1);
        fill(bad, n, 0xab);
        zero(bad, n.saturating_mul(0x1000));
        write(bad, &[1, 2, 3]);
        // (a copy is checked on both sides: either one bad, nothing copied)
        w32(p, 0x1122_3344);
        copy(bad, p, 4);
        copy(p, bad, 4);
        prop_assert_eq!(r32(p), 0x1122_3344);
        // (past the top: the 2 bytes inside stay)
        let before = r16(edge);
        w32(edge, 0xffff_ffff);
        w64(edge, u64::MAX);
        prop_assert_eq!(r32(edge), 0);
        prop_assert_eq!(r16(edge), before);
        prop_assert_eq!(atomic_rmw32(bad, |x| x + 1), 0);
        heap_free(p);
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
