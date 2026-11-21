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

fn itu_bytes(units: &[u16]) -> Vec<u8> {
    units.iter().flat_map(|u| u.to_le_bytes()).collect()
}

fn itu(b: &[u8], length: usize, mask: Option<u32>) -> (bool, u32) {
    text_unicode(&b[..length.min(b.len()).min(512)], length as u32, None, mask)
}

/// Wine's ntdll:rtlstr test_RtlIsTextUnicode (Windows' answers), without a DBCS code page
#[test]
fn text_unicode_follows_windows() {
    let w = |s: &str| -> Vec<u16> { s.encode_utf16().chain([0]).collect() };
    let ascii = b"A simple string\0".to_vec();
    assert!(!itu(&ascii, ascii.len(), None).0);
    let unicode = itu_bytes(&w("A Unicode string"));
    let n = unicode.len();
    assert!(itu(&unicode, n, None).0);
    assert!(!itu(&unicode, n - 1, None).0);
    assert_eq!(itu(&unicode, n, Some(0xf)), (true, itu::STATISTICS | itu::CONTROLS));
    assert_eq!(itu(&unicode, n, Some(0xf0)), (false, 0));
    assert_eq!(itu(&unicode, n - 1, Some(itu::ODD_LENGTH)), (false, itu::ODD_LENGTH));
    let swap = |v: &[u16]| -> Vec<u16> { v.iter().map(|u| u.swap_bytes()).collect() };
    let mut be: Vec<u16> = vec![0xfffe];
    be.extend(swap(&w("A Unicode string")));
    let be = itu_bytes(&be);
    assert!(!itu(&be, n + 2, None).0);
    assert!(!itu(&be[2..], n, None).0);
    assert_eq!(itu(&be[2..], n, Some(0xf0)).0, false);
    assert_eq!(itu(&be, n + 2, Some(0xf0)), (false, itu::REVERSE_CONTROLS | itu::REVERSE_SIGNATURE));
    let nc = w("AUnicodestring");
    let unc = itu_bytes(&nc);
    let mut benc: Vec<u16> = vec![0xfffe];
    benc.extend(swap(&nc));
    let benc = itu_bytes(&benc);
    let m = unc.len();
    let mixed = itu_bytes(&[9, 0x9000, 0x0d00, 10, 0]);
    let c = Some(itu::CONTROLS);
    let rc = Some(itu::REVERSE_CONTROLS);
    assert_eq!(itu(&unc, m, c), (false, 0));
    assert_eq!(itu(&benc, m, c), (false, 0));
    assert_eq!(itu(&unicode, n, c), (true, itu::CONTROLS));
    assert_eq!(itu(&benc, m + 2, c), (false, 0));
    assert_eq!(itu(&mixed, mixed.len(), c), (true, itu::CONTROLS));
    assert_eq!(itu(&benc, m, rc), (false, 0));
    assert_eq!(itu(&unc, m, rc), (false, 0));
    assert_eq!(itu(&unicode, n, rc), (false, 0));
    assert_eq!(itu(&be, n + 2, rc), (false, itu::REVERSE_CONTROLS));
    assert_eq!(itu(&mixed, mixed.len(), rc), (false, itu::REVERSE_CONTROLS));
    assert_eq!(itu(&mixed, mixed.len(), Some(itu::CONTROLS | itu::REVERSE_CONTROLS)), (false, itu::CONTROLS | itu::REVERSE_CONTROLS));
    assert!(!itu(&0x0d0au16.to_le_bytes(), 2, None).0);
    assert_eq!(itu(&[0x41, 0x0a, 0x0d, 0x1d], 4, Some(itu::STATISTICS)), (true, itu::STATISTICS));
}

proptest::proptest! {
    /// The flags only ever keep what the caller asked about, and the answer follows from them
    /// alone (the same flags as the mask give the same answer)
    #[test]
    fn text_unicode_masks(b in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..600), mask in proptest::prelude::any::<u32>()) {
        let length = b.len();
        let (_, all) = itu(&b, length, None);
        let (yes, got) = itu(&b, length, Some(mask));
        if length >= 2 && !(length == 2 && b[0] != 0 && b[1] == 0) {
            proptest::prop_assert_eq!(got, all & mask);
            proptest::prop_assert_eq!(itu(&b, length, Some(got)), (yes, got));
        }
    }
}

/// Windows' case table (crate::nt_case, from kikizake's ntcase goldens): sorted both ways, one
/// the other's inverse, and every pair one of Unicode's simple mappings (Windows leaves some
/// out, adds none)
#[test]
fn case_table_is_a_subset_of_unicode_one_to_one() {
    use crate::nt_case::{BY_LOWER, BY_UPPER};
    assert!(BY_LOWER.windows(2).all(|w| w[0].0 < w[1].0));
    assert!(BY_UPPER.windows(2).all(|w| w[0].0 < w[1].0));
    let mut inv: Vec<(u16, u16)> = BY_UPPER.iter().map(|&(u, l)| (l, u)).collect();
    inv.sort();
    assert_eq!(inv, BY_LOWER.to_vec());
    for &(l, u) in BY_LOWER.iter() {
        // (Rust has full mappings: U+1F80's upper case is two letters, its simple one U+1F88,
        // whose lower case is U+1F80)
        let one = |c: u16, up: bool| {
            let c = char::from_u32(u32::from(c)).unwrap();
            let v: Vec<u32> = if up { c.to_uppercase().map(|x| x as u32).collect() } else { c.to_lowercase().map(|x| x as u32).collect() };
            v
        };
        assert!(one(l, true) == [u32::from(u)] || one(u, false) == [u32::from(l)], "{l:04x} {u:04x}");
    }
    for x in 0..=u16::MAX {
        assert_eq!(crate::nt::down(crate::nt::up(x)), if crate::nt::up(x) == x { crate::nt::down(x) } else { x }, "{x:04x}");
    }
}

/// Wine's ntdll:rtlstr expectations (from Windows): Latin-1 upper case, U+00B5 and U+03F4 kept
#[test]
fn case_table_follows_wine_expectations() {
    for ch in 0u16..=255 {
        let want = match ch {
            0x61..=0x7a => ch - 0x20,
            0xe0..=0xfe if ch != 0xf7 => ch - 0x20,
            0xff => 0x178,
            _ => ch,
        };
        assert_eq!(crate::nt::up(ch), want, "{ch:02x}");
    }
    assert_eq!(crate::nt::down(0x3f4), 0x3f4);
    assert_eq!(crate::nt::down(0x3f9), 0x3f2);
    assert_eq!(crate::nt::down(0x3fd), 0x37b);
    assert_eq!(crate::nt::up(0x131), 0x131);
    assert_eq!(crate::nt::up(0x17f), 0x17f);
}

#[test]
fn compare_string_rows() {
    // RtlCompareString: unsigned bytes, RtlUpperChar when asked, else the lengths
    assert_eq!(compare_bytes(b"abc", b"abc", false), 0);
    assert_eq!(compare_bytes(b"abc", b"ABC", false), 0x20);
    assert_eq!(compare_bytes(b"abc", b"ABC", true), 0);
    assert_eq!(compare_bytes(b"ab", b"abc", false), -1);
    assert_eq!(compare_bytes(b"\xff", b"a", false), 0xff - 0x61);
    assert_eq!(compare_bytes(b"", b"", true), 0);
}

proptest::proptest! {
    /// antisymmetric, 0 exactly for equal (case-folded) strings, and agreeing with the unit
    /// compare on ASCII
    #[test]
    fn compare_string_laws(a in proptest::collection::vec(0x20u8..0x7f, 0..8), b in proptest::collection::vec(0x20u8..0x7f, 0..8), ci: bool) {
        let x = compare_bytes(&a, &b, ci);
        proptest::prop_assert_eq!(x, compare_bytes(&b, &a, ci).wrapping_neg());
        let fold = |v: &[u8]| v.iter().map(|&c| if ci { c.to_ascii_uppercase() } else { c }).collect::<Vec<u8>>();
        proptest::prop_assert_eq!(x == 0, fold(&a) == fold(&b));
        let w = |v: &[u8]| v.iter().map(|&c| u16::from(c)).collect::<Vec<u16>>();
        proptest::prop_assert_eq!(x.signum(), compare_units(&w(&a), &w(&b), ci).signum());
    }
}
