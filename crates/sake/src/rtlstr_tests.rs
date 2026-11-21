//! rtlstr: the rules' edges (sake-re/ntdll/rtl-strings.md, Wine's ntdll:rtlstr) and laws
//! (proptest): an append never writes past MaximumLength and keeps Length within it, Find agrees
//! with a scan of its own, Duplicate's buffer holds what it says.
use super::*;
use proptest::prelude::*;

fn h(len: u16, max: u16) -> Counted {
    Counted { len, max, buf: 0x1000 }
}
fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[test]
fn appends() {
    assert_eq!(append_asciiz(h(2, 5), 3), Ok(Append { at: 2, n: 3, len: 5, nul: None }));
    assert_eq!(append_asciiz(h(2, 5), 4), Err(Status::BufferTooSmall));
    assert_eq!(append_asciiz(h(0, 0xffff), 0x10000), Err(Status::BufferTooSmall));
    // (an empty source still checks the header)
    assert_eq!(append_asciiz(h(6, 5), 0), Err(Status::BufferTooSmall));
    assert_eq!(append_string(h(6, 5), 0), Ok(None));
    assert_eq!(append_string(h(2, 4), 3), Err(Status::BufferTooSmall));
    // a 0 unit only when one more unit fits
    assert_eq!(append_unicode_string(h(2, 8), 4), Ok(Some(Append { at: 2, n: 4, len: 6, nul: Some(6) })));
    assert_eq!(append_unicode_string(h(2, 6), 4), Ok(Some(Append { at: 2, n: 4, len: 6, nul: None })));
    assert_eq!(append_unicode_string(h(2, 7), 4), Ok(Some(Append { at: 2, n: 4, len: 6, nul: None })));
    // (an odd Length: written at its whole units, the 0 after the source's whole units)
    assert_eq!(append_unicode_string(h(3, 12), 5), Ok(Some(Append { at: 2, n: 5, len: 8, nul: Some(6) })));
    assert_eq!(append_unicode(h(0, 4), 0), Ok(Append { at: 0, n: 0, len: 0, nul: Some(0) }));
    assert_eq!(append_unicode(h(0, 0xffff), 0x7fff), Err(Status::BufferTooSmall));
}

#[test]
fn init_ex_edges() {
    assert_eq!(init_ex(0, 0), (Counted { len: 0, max: 0, buf: 0 }, Status::Success));
    assert_eq!(init_ex(8, 3), (Counted { len: 6, max: 8, buf: 8 }, Status::Success));
    assert_eq!(init_ex(8, 0x7ffe), (Counted { len: 0xfffc, max: 0xfffe, buf: 8 }, Status::Success));
    assert_eq!(init_ex(8, 0x7fff), (Counted { len: 0, max: 0, buf: 8 }, Status::NameTooLong));
}

/// every byte: only a..z change (exhaustive)
#[test]
fn upper_char_is_ascii_only() {
    for b in 0..=0xffu8 {
        assert_eq!(upper_char(b), if b.is_ascii_lowercase() { b - 0x20 } else { b }, "{b:#x}");
    }
}

#[test]
fn case_targets() {
    assert_eq!(case_target(h(0, 0), 0, false), Ok(CaseTarget::Empty));
    assert_eq!(case_target(h(0, 0), 0, true), Ok(CaseTarget::Allocate));
    assert_eq!(case_target(h(0, 4), 6, false), Err(Status::BufferOverflow));
    assert_eq!(case_target(Counted { len: 0, max: 8, buf: 0 }, 2, false), Err(Status::BufferOverflow));
    assert_eq!(case_target(h(0, 6), 6, false), Ok(CaseTarget::Into));
}

#[test]
fn duplicates() {
    let s = Some(h(6, 8));
    assert_eq!(duplicate(0, s), Ok((6, 6)));
    assert_eq!(duplicate(DUP_NUL, s), Ok((6, 8)));
    assert_eq!(duplicate(4, s), Err(Status::InvalidParameter));
    assert_eq!(duplicate(0, Some(h(3, 8))), Err(Status::InvalidParameter));
    assert_eq!(duplicate(0, Some(h(8, 6))), Err(Status::InvalidParameter));
    assert_eq!(duplicate(0, Some(Counted { len: 0, max: 2, buf: 0 })), Err(Status::InvalidParameter));
    assert_eq!(duplicate(DUP_NUL, Some(h(0xfffe, 0xfffe))), Err(Status::NameTooLong));
    // empty: no buffer unless asked for
    assert_eq!(duplicate(DUP_NUL, None), Ok((0, 0)));
    assert_eq!(duplicate(DUP_NUL | DUP_ALLOCATE_EMPTY, None), Ok((0, 2)));
    // (Wine's table: flag 2 alone is always invalid)
    assert_eq!(duplicate(DUP_ALLOCATE_EMPTY, Some(h(0, 0))), Err(Status::InvalidParameter));
    assert_eq!(duplicate(DUP_ALLOCATE_EMPTY, None), Err(Status::InvalidParameter));
}

#[test]
fn finds() {
    let s = w("Some Wild String");
    assert_eq!(find(0, &s, &w("W")), Ok(12));
    assert_eq!(find(FIND_FROM_END, &s, &w("S")), Ok(20));
    assert_eq!(find(0, &s, &w("S")), Ok(2));
    assert_eq!(find(0, &s, &w("x")), Err(Status::NotFound));
    assert_eq!(find(FIND_CASE_INSENSITIVE, &s, &w("w")), Ok(12));
    assert_eq!(find(FIND_COMPLEMENT, &s, &w("Some")), Ok(10));
    assert_eq!(find(0, &[], &w("a")), Err(Status::NotFound));
    assert_eq!(find(FIND_COMPLEMENT, &s, &[]), Ok(2));
    assert_eq!(find(0, &s, &[]), Err(Status::NotFound));
}

proptest! {
    /// an append that succeeds stays inside the buffer: its bytes and its 0 unit end at or
    /// before MaximumLength, and the new Length is the old plus the source's
    #[test]
    fn appends_stay_inside(len in 0u16..64, max in 0u16..64, n in 0u16..64, kind in 0u8..3) {
        let d = h(len, max);
        let a = match kind {
            0 => append_asciiz(d, usize::from(n)).ok(),
            1 => append_string(d, n).ok().flatten(),
            _ => append_unicode_string(d, n).ok().flatten(),
        };
        if let Some(a) = a {
            prop_assert!(u32::from(a.at) + u32::from(a.n) <= u32::from(max));
            prop_assert_eq!(u32::from(a.len), u32::from(len) + u32::from(n));
            if let Some(z) = a.nul {
                prop_assert!(u32::from(z) + 2 <= u32::from(max));
            }
        } else if kind == 0 || n != 0 {
            prop_assert!(u32::from(len) + u32::from(n) > u32::from(max));
        }
    }

    /// Find against a scan of its own, every flag
    #[test]
    fn find_is_a_scan(s in proptest::collection::vec(0x40u16..0x60, 0..12), set in proptest::collection::vec(0x40u16..0x60, 0..4), flags in 0u32..8) {
        let low = |u: u16| if flags & FIND_CASE_INSENSITIVE != 0 { crate::nt::down(u) } else { u };
        let hit = |u: u16| set.iter().any(|&x| low(x) == low(u)) != (flags & FIND_COMPLEMENT != 0);
        let idx: Vec<usize> = (0..s.len()).filter(|&i| hit(s[i])).collect();
        let want = if flags & FIND_FROM_END != 0 { idx.last().map(|&i| i * 2) } else { idx.first().map(|&i| (i + 1) * 2) };
        prop_assert_eq!(find(flags, &s, &set).ok().map(usize::from), want);
    }
}

#[test]
fn compare_units_edges() {
    assert_eq!(compare_units(&w("abc"), &w("abd"), false), -1);
    assert_eq!(compare_units(&w("abc"), &w("ABC"), true), 0);
    assert_eq!(compare_units(&w("abc"), &w("ABC"), false), 0x20);
    assert_eq!(compare_units(&w("ab"), &w("abcd"), false), -2);
}

#[test]
fn guids() {
    let g = [4, 3, 2, 1, 6, 5, 8, 7, 9, 0xa, 0xb, 0xc, 0xd, 0xe, 0xf, 0xa];
    assert_eq!(guid_from(&w("{01020304-0506-0708-090A-0B0C0D0E0F0A}")), Some(g));
    assert_eq!(guid_from(&w("{01020304-0506-0708-090a-0b0c0d0e0f0a}\0junk")), Some(g));
    // (Wine's szGuid2: the wrong closing bracket)
    assert_eq!(guid_from(&w("{01020304-0506-0708-090A-0B0C0D0E0F0A]")), None);
    assert_eq!(guid_from(&w("{01020304-0506-0708-090A-0B0C0D0E0F0A}x")), None);
    assert_eq!(guid_from(&w("{01020304-0506-0708-090A-0B0C0D0E0F0}")), None);
    assert_eq!(guid_from(&w("{0102030g-0506-0708-090A-0B0C0D0E0F0A}")), None);
    assert_eq!(guid_text(&g), "{01020304-0506-0708-090a-0b0c0d0e0f0a}");
}

#[test]
fn utf8_edges() {
    let one = |b: &[u8]| utf8_units(b);
    assert_eq!(one(b"A\xc3\xa9"), (vec![0x41, 0xe9], false));
    assert_eq!(one(b"\xf0\x9f\x98\x80"), (vec![0xd83d, 0xde00], false));
    // each maximal invalid subpart one U+FFFD; a cut sequence's breaking byte read again
    // (Wine's utf8_to_unicode table, as Windows answers it)
    assert_eq!(one(b"\xc0\x80"), (vec![0xfffd, 0xfffd], true));
    assert_eq!(one(b"\xe2\x82A"), (vec![0xfffd, 0x41], true));
    assert_eq!(one(b"-\xed\xa0\x80-\xed\xaf\xbf-"), (vec![0x2d, 0xfffd, 0xfffd, 0x2d, 0xfffd, 0xfffd, 0x2d], true));
    assert_eq!(one(b"\xf4\x90\x80\x80"), (vec![0xfffd; 3], true));
    assert_eq!(one(b"\xf7\xbf\xbf\xbf"), (vec![0xfffd; 4], true));
    assert_eq!(one(b"\xfc\x84\x80\x80\x80\x80"), (vec![0xfffd; 6], true));
    assert_eq!(one(b"\xe0\x80\xad\xe0\x9f\xbf"), (vec![0xfffd; 4], true));
    assert_eq!(one(b"\xf4\x8f\xbf\xbf"), (vec![0xdbff, 0xdfff], false));
    assert_eq!(one(b"\xe2\x82"), (vec![0xfffd], true));
    let enc = |u: &[u16]| { let (c, bad) = utf8_chars(u); (c.concat(), bad) };
    assert_eq!(enc(&[0x41, 0xe9]), (b"A\xc3\xa9".to_vec(), false));
    assert_eq!(enc(&[0xd800, 0x41]), (b"\xef\xbf\xbdA".to_vec(), true));
    assert_eq!(enc(&[0xdc00]), (b"\xef\xbf\xbd".to_vec(), true));
    assert_eq!(enc(&[0xd83d, 0xde00]), ("\u{1f600}".as_bytes().to_vec(), false));
}

/// Wine's ustr2astr table's targets ("abcdef": size 7)
#[test]
fn ansi_targets() {
    assert_eq!(ansi_target(12, 7, false), Ok(AnsiTarget::Into { cap: 6, cut: false }));
    assert_eq!(ansi_target(0, 7, false), Err(Status::BufferOverflow));
    assert_eq!(ansi_target(1, 7, false), Ok(AnsiTarget::Into { cap: 0, cut: true }));
    assert_eq!(ansi_target(5, 7, false), Ok(AnsiTarget::Into { cap: 4, cut: true }));
    assert_eq!(ansi_target(7, 7, false), Ok(AnsiTarget::Into { cap: 6, cut: false }));
    assert_eq!(ansi_target(0, 7, true), Ok(AnsiTarget::Allocate));
    assert_eq!(ansi_target(0, 0x10000, true), Err(Status::InvalidParameter2));
}

/// Wine's hash_test table
#[test]
fn hashes() {
    for (t, ci, want) in [("T", false, 0x54u32), ("Test", false, 0x766bb952), ("TeSt", false, 0x764bb172), ("test", false, 0x4745d132),
        ("test", true, 0x6689c132), ("TEST", false, 0x6689c132), ("t\u{e9}st", false, 0x8845cfb6), ("t\u{e9}st", true, 0xa789bfb6),
        ("T\u{c9}ST", false, 0xa789bfb6), ("abcdef", false, 0x971318c3)] {
        assert_eq!(hash(&w(t), ci), want, "{t} {ci}");
    }
}

proptest! {
    /// GUID text reads back to the same GUID
    #[test]
    fn guid_round_trip(g: [u8; 16]) {
        prop_assert_eq!(guid_from(&w(&guid_text(&g))), Some(g));
    }

    /// valid UTF-16 -> UTF-8 -> UTF-16 is the identity, with nothing replaced
    #[test]
    fn utf8_round_trip(s: String) {
        let u = w(&s);
        let (b, bad) = utf8_chars(&u);
        prop_assert!(!bad);
        prop_assert_eq!(utf8_units(&b.concat()), (u, false));
    }

    /// any bytes: replaced exactly when they are not UTF-8 (then at least one U+FFFD, never more
    /// than one per byte); valid UTF-8 as std decodes it
    #[test]
    fn utf8_replaces_only_invalid(b: Vec<u8>) {
        let (u, bad) = utf8_units(&b);
        prop_assert_eq!(bad, std::str::from_utf8(&b).is_err());
        match std::str::from_utf8(&b) {
            Ok(s) => prop_assert_eq!(u, w(s)),
            Err(_) => {
                let n = u.iter().filter(|&&x| x == 0xfffd).count();
                prop_assert!(n >= 1 && n <= b.len());
            }
        }
    }
}
