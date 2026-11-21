//! ntstr: the rules of the clean-room notes on their edges, and laws over random inputs
//! (proptest): every value printed in every base parses back, RtlCharToInteger and
//! RtlUnicodeStringToInteger agree on ASCII text, the lengths of RtlIntegerToChar.
use super::*;
use proptest::prelude::*;

fn a2i(s: &[u8], base: u32) -> (Status, Option<u32>) {
    let at = |k: usize| s.get(k).copied().unwrap_or(0);
    char_to_integer(&at, base)
}
fn u2i(s: &[u16], length: u16, base: u32) -> (Status, u32) {
    let at = |k: usize| s[k];
    unicode_to_integer(&at, length, base)
}
fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn full(s: &str, base: u32) -> (Status, u32) {
    let v = w(s);
    u2i(&v, kome::num::lo16(2 * v.len()), base)
}

#[test]
fn char_to_integer_edges() {
    let ok = |v: u32| (Status::Success, Some(v));
    assert_eq!(a2i(b"123", 0), ok(123));
    assert_eq!(a2i(b"-123", 0), ok(123u32.wrapping_neg()));
    assert_eq!(a2i(b"0x1F", 0), ok(0x1f));
    // (uppercase X is no prefix: the 0 is dropped and X ends the number)
    assert_eq!(a2i(b"0X1F", 0), ok(0));
    assert_eq!(a2i(b"0o17", 0), ok(0o17));
    assert_eq!(a2i(b"0b101", 0), ok(5));
    assert_eq!(a2i(b"08", 0), ok(8));
    assert_eq!(a2i(b"0x1F", 16), ok(0));
    assert_eq!(a2i(b"4294967296", 0), ok(0));
    // signed-char white space: 0x80..0xff are passed
    assert_eq!(a2i(b"\x80 12", 0), ok(12));
    assert_eq!(a2i(b"\xff7", 0), ok(7));
    // a NUL first is passed while the byte after it is not NUL
    assert_eq!(a2i(b"\x0012", 0), ok(12));
    assert_eq!(a2i(b"  ", 0), ok(0));
    assert_eq!(a2i(b"", 0), ok(0));
    assert_eq!(a2i(b"12", 3), (Status::InvalidParameter, None));
    assert_eq!(a2i(b"12", 36), (Status::InvalidParameter, None));
}

#[test]
fn unicode_to_integer_edges() {
    assert_eq!(u2i(&[], 0, 0), (Status::InvalidParameter, 0));
    assert_eq!(u2i(&w("12"), 3, 0), (Status::InvalidParameter, 0));
    assert_eq!(full("12", 7), (Status::InvalidParameter, 0));
    assert_eq!(full("  -0x10", 0), (Status::Success, 16u32.wrapping_neg()));
    // only Length / 2 units: "12" of "123"
    assert_eq!(u2i(&w("123"), 4, 0), (Status::Success, 12));
    assert_eq!(full("1\u{0}2", 0), (Status::Success, 1));
    // units past ASCII are not white space nor digits
    assert_eq!(full("\u{3000}1", 0), (Status::Success, 0));
    assert_eq!(full("\u{ff11}", 0), (Status::Success, 0));
    // a white space only string: 0
    assert_eq!(full("   ", 0), (Status::Success, 0));
}

#[test]
fn integer_to_char_lengths() {
    assert_eq!(integer_to_char(255, 16, 40), Ok(b"FF\0".to_vec()));
    assert_eq!(integer_to_char(255, 16, 2), Ok(b"FF".to_vec()));
    assert_eq!(integer_to_char(255, 16, 1), Err(Status::BufferOverflow));
    assert_eq!(integer_to_char(255, 16, 0), Err(Status::BufferOverflow));
    assert_eq!(integer_to_char(255, 16, -5), Ok(b"000FF".to_vec()));
    assert_eq!(integer_to_char(255, 16, -2), Ok(b"FF".to_vec()));
    assert_eq!(integer_to_char(255, 16, -1), Err(Status::BufferOverflow));
    assert_eq!(integer_to_char(0, 0, 1), Ok(b"0".to_vec()));
    assert_eq!(integer_to_char(0xffff_ffff, 10, 40), Ok(b"4294967295\0".to_vec()));
    assert_eq!(integer_to_char(1, 7, 40), Err(Status::InvalidParameter));
}

const BASES: [u32; 4] = [2, 8, 10, 16];

proptest! {
    /// printed in a base, a value parses back (both parsers, the base given)
    #[test]
    fn printed_values_parse_back(v: u32, b in 0usize..4) {
        let base = BASES[b];
        let mut d = integer_to_char(u64::from(v), base, 40).unwrap();
        prop_assert_eq!(d.pop(), Some(0));
        prop_assert_eq!(a2i(&d, base), (Status::Success, Some(v)));
        let u: Vec<u16> = d.iter().map(|&x| u16::from(x)).collect();
        prop_assert_eq!(u2i(&u, kome::num::lo16(2 * u.len()), base), (Status::Success, v));
    }

    /// on printable ASCII (no blank inside the leading run) the two parsers agree
    #[test]
    fn parsers_agree_on_ascii(s in "[ +\\-0-9a-fA-FxobXOB.z]{1,16}", b in 0usize..5) {
        let base = [0, 2, 8, 10, 16][b];
        let (sa, va) = a2i(s.as_bytes(), base);
        let (su, vu) = full(&s, base);
        prop_assert_eq!(sa, su);
        prop_assert_eq!(va, Some(vu));
    }

    /// RtlIntegerToChar writes the digits, a NUL only where it fits, and |length| bytes when
    /// the length is negative
    #[test]
    fn lengths_of_integer_to_char(v: u32, b in 0usize..4, len in -40i32..=40) {
        let base = BASES[b];
        let d = integer_to_char(u64::from(v), base, 40).unwrap();
        let n = kome::num::u32_of(d.len() - 1);
        match integer_to_char(u64::from(v), base, len) {
            Ok(x) if len >= 0 => prop_assert_eq!(kome::num::u32_of(x.len()), n + u32::from(n < len.unsigned_abs())),
            Ok(x) => prop_assert_eq!(kome::num::u32_of(x.len()), len.unsigned_abs()),
            Err(st) => {
                prop_assert_eq!(st, Status::BufferOverflow);
                prop_assert!(n > len.unsigned_abs());
            }
        }
    }
}
