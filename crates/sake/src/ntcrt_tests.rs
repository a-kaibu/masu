//! ntcrt: the tables' rules over every value (exhaustive), the conversions' edges as Wine's
//! ntdll:string tests and the clean-room notes give them, and laws (proptest): _itoa's text
//! parses back through strtol, wrapping atoi is strtol modulo 2^32 where strtol does not saturate.
use super::*;
use proptest::prelude::*;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn wtoi(s: &str) -> i32 {
    let w = wide(s);
    let at = |k: usize| w.get(k).copied().unwrap_or(0);
    kome::num::lo32(strtox(&at, 10, Kind::Wrapping, 32).unwrap().value).cast_signed()
}
fn strto(s: &[u8], base: u32, kind: Kind, bits: u32) -> Option<Parsed> {
    let at = |k: usize| s.get(k).copied().unwrap_or(0);
    strtox(&at, base, kind, bits)
}

#[test]
fn towlower_is_latin1_upper_only() {
    for c in 0..=0xffffu16 {
        let want = if (0x41..=0x5a).contains(&c) || (0xc0..=0xd6).contains(&c) || (0xd8..=0xde).contains(&c) { c + 32 } else { c };
        assert_eq!(towlower(c), want, "{c:04x}");
    }
}

#[test]
fn tolower_keeps_the_whole_int() {
    for c in -512..0xffff {
        let low = kome::num::lo8(c);
        let want = if (0x41..=0x5a).contains(&low) && c != -1 { c + 32 } else { c };
        assert_eq!(tolower(c), want, "{c}");
    }
}

#[test]
fn wcsicmp_folds_ascii_only() {
    for a in 0..0x300u16 {
        for b in (0..0x300u16).step_by(7) {
            let (x, y) = ([a, 0], [b, 0]);
            let la = if (0x41..=0x5a).contains(&a) { a + 32 } else { a };
            let lb = if (0x41..=0x5a).contains(&b) { b + 32 } else { b };
            let want = i32::from(la) - i32::from(lb);
            assert_eq!(wcsnicmp(&|k| x[k], &|k| y[k], None), want);
            assert_eq!(wcsnicmp(&|k| x[k], &|k| y[k], Some(2)), want);
        }
    }
}

#[test]
fn classes_of_the_tables() {
    // (U+00AA, U+00B5, U+00BA are punctuation in ntdll's table)
    for c in [0xaau16, 0xb5, 0xba] {
        assert_eq!(iswctype(c, Test::Alpha.mask()), 0);
        assert_ne!(iswctype(c, Test::Punct.mask()), 0);
    }
    assert_eq!(iswctype(0x100, 0xffff), 0);
    assert_eq!(isctype(0xc0, Test::Alpha.mask()), 0);
    assert_eq!(isctype(0x141, Test::Alpha.mask()), 1);
    assert_eq!(isctype(-1, 0xffff), 0);
    assert_eq!(char_class(0x09).bits(), 0x28);
    assert_eq!(wide_class(0x09).bits(), 0x68);
}

#[test]
fn itoa_has_no_base_check() {
    let s = |v: Vec<u16>| String::from_utf16(&v).unwrap();
    assert_eq!(s(xtoa(71_270_178, 37, false, 32).unwrap()), "111111");
    assert_eq!(s(xtoa(36, 37, false, 32).unwrap()), "{");
    assert_eq!(s(xtoa(u64::from((-5i32).cast_unsigned()), 10, true, 32).unwrap()), "-5");
    assert_eq!(s(xtoa(0x8000_0000, 10, true, 32).unwrap()), "-2147483648");
    assert_eq!(s(xtoa(255, 16, false, 32).unwrap()), "ff");
    assert_eq!(xtoa(1, 1, false, 32), None);
}

#[test]
fn wtoi_wraps() {
    assert_eq!(wtoi("-9999999999"), -1_410_065_407);
    assert_eq!(wtoi("-2147483649"), 2_147_483_647);
    assert_eq!(wtoi("4294967295"), -1);
    assert_eq!(wtoi("4294967296"), 0);
    assert_eq!(wtoi(" \u{a0}+44"), 44);
    assert_eq!(wtoi("\u{e50}\u{e51}7"), 17);
    assert_eq!(wtoi("\u{3000}5"), 0);
}

#[test]
fn strtol_saturates_and_prefixes() {
    let p = strto(b"2147483648", 10, Kind::Signed, 32).unwrap();
    assert_eq!((p.value, p.range, p.used), (0x7fff_ffff, true, 10));
    let p = strto(b"-2147483648", 10, Kind::Signed, 32).unwrap();
    assert_eq!((p.value, p.range), (0x8000_0000, false));
    let p = strto(b"-2147483649", 10, Kind::Signed, 32).unwrap();
    assert_eq!((p.value, p.range), (0x8000_0000, true));
    let p = strto(b"0x1fz", 0, Kind::Signed, 32).unwrap();
    assert_eq!((p.value, p.used), (0x1f, 4));
    let p = strto(b"0X1f", 16, Kind::Signed, 32).unwrap();
    assert_eq!(p.value, 0x1f);
    let p = strto(b"017", 0, Kind::Signed, 32).unwrap();
    assert_eq!(p.value, 0o17);
    let p = strto(b"  -", 10, Kind::Signed, 32).unwrap();
    assert_eq!((p.value, p.used), (0, 0));
    assert_eq!(strto(b"1", 37, Kind::Signed, 32), None);
    let p = strto(b"99999999999", 10, Kind::Unsigned, 32).unwrap();
    assert_eq!((p.value, p.range), (0xffff_ffff, true));
    // (Wine's wcstoul table: all ones, then negated)
    for t in [&b"-4294967296"[..], b"-9223372036854775807"] {
        let p = strto(t, 10, Kind::Unsigned, 32).unwrap();
        assert_eq!((p.value, p.range), (1, true));
    }
    let p = strto(b"-4294967295", 10, Kind::Unsigned, 32).unwrap();
    assert_eq!((p.value, p.range), (1, false));
    let p = strto(b"9223372036854775808", 10, Kind::Signed, 64).unwrap();
    assert_eq!((p.value, p.range), (0x7fff_ffff_ffff_ffff, true));
}

proptest! {
    /// _itoa's text of a value in bases 2..36 reads back through strtoul
    #[test]
    fn itoa_reads_back(v: u32, base in 2u32..=36) {
        let t: Vec<u8> = xtoa(u64::from(v), base, false, 32).unwrap().iter().map(|&u| kome::num::lo8(u)).collect();
        let p = strto(&t, base, Kind::Unsigned, 32).unwrap();
        prop_assert_eq!((p.value, p.used, p.range), (u64::from(v), t.len(), false));
    }

    /// unsigned overflow is all ones before the sign: "-" + anything past 2^32 - 1 is 1
    #[test]
    fn strtoul_overflow_negated(extra in 1u64..1_000_000_000_000) {
        let t = format!("-{}", u64::from(u32::MAX) + extra);
        let p = strto(t.as_bytes(), 10, Kind::Unsigned, 32).unwrap();
        prop_assert_eq!((p.value, p.range), (1, true));
    }

    /// atoi wraps where strtoul would not overflow: the same value modulo 2^32
    #[test]
    fn atoi_is_strtoul_modulo(v: u32, neg: bool) {
        let t = format!("{}{v}", if neg { "-" } else { "" });
        let a = strto(t.as_bytes(), 10, Kind::Wrapping, 32).unwrap();
        let u = strto(t.as_bytes(), 10, Kind::Unsigned, 32).unwrap();
        prop_assert_eq!(a.value, u.value);
    }

    /// the wide and the char parsers agree on ASCII
    #[test]
    fn wide_and_char_agree(s in "[ \t+\\-0-9a-zA-Z]{0,14}", b in 0usize..6) {
        let base = [0, 2, 8, 10, 16, 36][b];
        let w = wide(&s);
        let at = |k: usize| w.get(k).copied().unwrap_or(0);
        prop_assert_eq!(strtox(&at, base, Kind::Signed, 32), strto(s.as_bytes(), base, Kind::Signed, 32));
    }
}

/// Wine's ntdll:string toupper loop: every int from -512 to 0xfffe against RtlAnsiCharToUnicodeChar
/// + RtlUpcaseUnicodeChar + WideCharToMultiByte over the bytes (i, i >> 8, 0) (exhaustive)
#[test]
fn toupper_is_ansi_char_upcased_back() {
    for i in -512i32..0xffff {
        let s = [kome::num::lo8(i.cast_unsigned()), kome::num::lo8(i.cast_unsigned() >> 8), 0];
        let (u, n) = ansi_char(&|k| s.get(k).copied().unwrap_or(0));
        assert!(n == 1 || n == 2, "{i:#x}");
        let a = crate::text::w2a(&[crate::nt::up(u)]);
        let want = match a.as_slice() {
            [x] => u32::from(*x),
            [x, y] => u32::from(*x) << 8 | u32::from(*y),
            _ => u32::from(kome::num::lo16(i.cast_unsigned())),
        };
        assert_eq!(kome::num::lo16(toupper(i.cast_unsigned())), kome::num::lo16(want), "toupper({i:#x})");
    }
}

/// a byte that is no lead byte is one character; a lead byte takes the next one with it, NUL too
#[test]
fn ansi_char_lengths() {
    for b in 0..=0xffu8 {
        let s = [b, 0];
        let (_, n) = ansi_char(&|k| s.get(k).copied().unwrap_or(0));
        assert_eq!(n, if crate::text::is_lead(b) { 2 } else { 1 }, "{b:#x}");
    }
    assert_eq!(ansi_char(&|k| [0x41u8, 0x42].get(k).copied().unwrap_or(0)), (0x41, 1));
}
