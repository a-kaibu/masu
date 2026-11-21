use super::*;
use proptest::prelude::*;

/// The x86 code's own steps, word by word (32x32 partial products, the shift in steps of 31):
/// an independent model of magic_divide's x86 branch
fn magic_x86_words(a: i64, b: u64, shift: u8) -> i64 {
    let neg = a < 0;
    let a = a.unsigned_abs();
    let (al, ah, bl, bh) = (a & 0xffff_ffff, a >> 32, b & 0xffff_ffff, b >> 32);
    let t = (bl * al) >> 32;
    let p1 = bl * ah;
    let p2 = bh * al;
    let mid = (p2 & 0xffff_ffff) + t + (p1 & 0xffff_ffff);
    let carry = mid >> 32;
    let mut hi = (bh * ah).wrapping_add(p2 >> 32).wrapping_add(p1 >> 32).wrapping_add(carry);
    let mut cl = shift;
    while cl > 31 {
        cl -= 31;
        hi >>= 31;
    }
    hi >>= cl;
    if neg { hi.cast_signed().wrapping_neg() } else { hi.cast_signed() }
}

#[test]
fn wine_magic_divide_table() {
    // (Wine's large_int.c magic_divide rows: a, b, shift, result; a Windows measurement)
    let rows: &[(u64, u64, u8, u64)] = &[
        (0x3, 0x5555555555555555, 0, 0x0),
        (0x13de4355, 0x5555555555555555, 0, 0x69f6bc6),
        (0x7fffffffffffffff, 0x5555555555555555, 0, 0x2aaaaaaaaaaaaaaa),
        (0x3, 0xaaaaaaaaaaaaaaaa, 1, 0x0),
        (0x13de4355, 0xaaaaaaaaaaaaaaaa, 1, 0x69f6bc6),
        (0x7fffffffffffffff, 0xaaaaaaaaaaaaaaaa, 1, 0x2aaaaaaaaaaaaaaa),
        (0xfffffffffffffffd, 0x5555555555555555, 0, 0x0),
        (0xffffffffec21bcab, 0x5555555555555555, 0, 0xfffffffff960943a),
        (0x8000000000000001, 0x5555555555555555, 0, 0xd555555555555556),
        (0xfffffffffffffffd, 0xaaaaaaaaaaaaaaaa, 1, 0x0),
        (0xffffffffec21bcab, 0xaaaaaaaaaaaaaaaa, 1, 0xfffffffff960943a),
        (0x8000000000000001, 0xaaaaaaaaaaaaaaaa, 1, 0xd555555555555556),
        (0xfffffffffffffffd, 0xaaaaaaaaaaaaaaab, 0, 0xfffffffffffffffe),
        (0xffffffffec21bcab, 0xaaaaaaaaaaaaaaab, 0, 0xfffffffff2c12872),
        (0x8000000000000001, 0xaaaaaaaaaaaaaaab, 0, 0xaaaaaaaaaaaaaaac),
        (0xfffffffffffffffd, 0x5555555555555556, 1, 0x0),
        (0xffffffffec21bcab, 0x5555555555555556, 1, 0xfffffffffcb04a1d),
        (0x8000000000000001, 0x5555555555555556, 1, 0xeaaaaaaaaaaaaaab),
        (0x3, 0xaaaaaaaaaaaaaaab, 0, 0x2),
        (0x13de4355, 0xaaaaaaaaaaaaaaab, 0, 0xd3ed78e),
        (0x7fffffffffffffff, 0xaaaaaaaaaaaaaaab, 0, 0x5555555555555554),
        (0x3, 0x5555555555555556, 1, 0x0),
        (0x13de4355, 0x5555555555555556, 1, 0x34fb5e3),
        (0x7fffffffffffffff, 0x5555555555555556, 1, 0x1555555555555555),
        (0x3, 0xaaaaaaaaaaaaa800, 1, 0x0),
        (0x13de4355, 0xaaaaaaaaaaaaa800, 1, 0x69f6bc6),
        (0x7fffffffffffffff, 0xaaaaaaaaaaaaa800, 1, 0x2aaaaaaaaaaaa9ff),
        (0x5, 0x33333333333333ff, 0, 0x1),
        (0x211d1ae3, 0x33333333333333ff, 0, 0x69f6bc7),
        (0x7fffffffffffffff, 0x33333333333333ff, 0, 0x19999999999999ff),
        (0x5, 0x66666666666667fe, 1, 0x1),
        (0x211d1ae3, 0x66666666666667fe, 1, 0x69f6bc7),
        (0x7fffffffffffffff, 0x66666666666667fe, 1, 0x19999999999999ff),
        (0x5, 0xcccccccccccccffd, 2, 0x1),
        (0x211d1ae3, 0xcccccccccccccffd, 2, 0x69f6bc7),
        (0x7fffffffffffffff, 0xcccccccccccccffd, 2, 0x19999999999999ff),
        (0xaddcafeface, 0x2f1e28fd1b5cca, 33, 0x1),
        (0x81ac1b9c2310a80, 0x2f1e28fd1b5cca, 33, 0xbeef),
        (0x74ae3b5f1558c800, 0x2f1e28fd1b5cca, 33, 0xabcde),
        (0xaddcafeface, 0x2f1e28fd1b5cca00, 41, 0x1),
        (0x81ac1b9c2310a80, 0x2f1e28fd1b5cca00, 41, 0xbeef),
        (0x74ae3b5f1558c800, 0x2f1e28fd1b5cca00, 41, 0xabcde),
    ];
    for &(a, b, s, r) in rows {
        for x64 in [false, true] {
            assert_eq!(magic_divide(a.cast_signed(), b, s, x64).cast_unsigned(), r, "{a:#x} {b:#x} {s} x64={x64}");
        }
    }
}

#[test]
fn magic_divide_shift_counts_differ_by_arch() {
    // (x86: a count of 64+ empties the word; x64: SHR takes it mod 64)
    let a = 0x7fff_ffff_ffff_ffff;
    let b = u64::MAX;
    for s in 0..=255u8 {
        let hi = 0x7fff_ffff_ffff_fffe_u64;
        assert_eq!(magic_divide(a, b, s, false), hi.checked_shr(u32::from(s)).unwrap_or(0).cast_signed(), "{s}");
        assert_eq!(magic_divide(a, b, s, true), (hi >> (s & 63)).cast_signed(), "{s}");
    }
}

proptest! {
    #[test]
    fn magic_divide_x86_is_its_word_steps(a in any::<i64>(), b in any::<u64>(), s in any::<u8>()) {
        prop_assert_eq!(magic_divide(a, b, s, false), magic_x86_words(a, b, s));
    }

    #[test]
    fn divide_matches_i128(a in any::<u64>(), b in any::<u64>()) {
        let u = divide(Div::Unsigned, a, b);
        let s = divide(Div::Signed, a, b);
        if b == 0 {
            prop_assert!(u.is_none() && s.is_none());
        } else {
            prop_assert_eq!(u, Some((a / b, a % b)));
            // (on the magnitudes; truncating, so i128's own / and % with the wrap of MIN / -1)
            let (x, y) = (i128::from(a.cast_signed()), i128::from(b.cast_signed()));
            let q = kome::num::lo64((x / y).cast_unsigned());
            let r = kome::num::lo64((x % y).cast_unsigned());
            prop_assert_eq!(s, Some((q, r)));
        }
    }

    #[test]
    fn shifts_match_u128(v in any::<u64>(), cl in any::<u8>()) {
        let wide = u128::from(v);
        let (l, _) = shift(Shift::Left, v, cl);
        let (r, _) = shift(Shift::RightUnsigned, v, cl);
        let (sr, _) = shift(Shift::RightSigned, v, cl);
        prop_assert_eq!(l, if cl >= 64 { 0 } else { kome::num::lo64(wide << cl) });
        prop_assert_eq!(r, if cl >= 64 { 0 } else { kome::num::lo64(wide >> cl) });
        let si = i128::from(v.cast_signed());
        prop_assert_eq!(sr, kome::num::lo64((si >> cl.min(127)).cast_unsigned()));
    }
}

#[test]
fn shift_leaves_cl_as_the_code_does() {
    // (every count: 0..31 untouched, 32..63 and-ed with 31, 64.. untouched)
    for cl in 0..=255u8 {
        for k in [Shift::Left, Shift::RightSigned, Shift::RightUnsigned] {
            let want = if (32..64).contains(&cl) { cl & 31 } else { cl };
            assert_eq!(shift(k, 0x8123_4567_0000_0000, cl).1, want);
        }
    }
    // (Wine's large_int.c shift rows, the count as the ECX it passes: only CL matters)
    let (a, b) = (0x0123_4567_89ab_cdef_u64, 0x8123_4567_89ab_cdef_u64);
    let rows: &[(Shift, u64, u32, u64)] = &[
        (Shift::Left, a, 12, 0x3456789abcdef000),
        (Shift::Left, a, 44, 0xbcdef00000000000),
        (Shift::Left, a, 88, 0),
        (Shift::Left, a, 0x88, 0),
        (Shift::Left, a, 0x108, 0x23456789abcdef00),
        (Shift::RightSigned, a, 12, 0x0123456789abc),
        (Shift::RightSigned, a, 44, 0x01234),
        (Shift::RightSigned, a, 88, 0),
        (Shift::RightSigned, b, 12, 0xfff8123456789abc),
        (Shift::RightSigned, b, 44, 0xfffffffffff81234),
        (Shift::RightSigned, b, 88, u64::MAX),
        (Shift::RightSigned, b, 0x108, 0xff8123456789abcd),
        (Shift::RightUnsigned, b, 12, 0x8123456789abc),
        (Shift::RightUnsigned, b, 44, 0x81234),
        (Shift::RightUnsigned, b, 88, 0),
        (Shift::RightUnsigned, b, 0x108, 0x8123456789abcd),
    ];
    for &(k, v, ecx, want) in rows {
        assert_eq!(shift(k, v, kome::num::lo8(ecx)).0, want, "{k:?} {v:#x} {ecx:#x}");
    }
}

#[test]
fn signed_division_edges() {
    let m = i64::MIN.cast_unsigned();
    assert_eq!(divide(Div::Signed, m, u64::MAX), Some((m, 0)));
    assert_eq!(divide(Div::Signed, (-7i64).cast_unsigned(), 2), Some(((-3i64).cast_unsigned(), (-1i64).cast_unsigned())));
    assert_eq!(divide(Div::Signed, 7, (-2i64).cast_unsigned()), Some(((-3i64).cast_unsigned(), 1)));
}
