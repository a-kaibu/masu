//! kome::guid against the copies it replaced (kept here verbatim as oracles: actctx's, iphlp's,
//! xof's and webbrowser's parsers, misc's/iphlp's and com's printers, the builders), on random
//! GUIDs, their text with random case and random damage.
use super::*;
use proptest::prelude::*;

/// sake actctx.rs parse_guid (trimmed, braces optional)
fn old_actctx(s: &str) -> Option<Guid> {
    let s = s.trim();
    let s = s.strip_prefix('{').and_then(|s| s.strip_suffix('}')).unwrap_or(s);
    let p: Vec<&str> = s.split('-').collect();
    if p.len() != 5 || [8, 4, 4, 4, 12].iter().zip(&p).any(|(n, x)| x.len() != *n || !x.chars().all(|c| c.is_ascii_hexdigit())) {
        return None;
    }
    let mut g = [0u8; 16];
    g[..4].copy_from_slice(&u32::from_str_radix(p[0], 16).ok()?.to_le_bytes());
    g[4..6].copy_from_slice(&u16::from_str_radix(p[1], 16).ok()?.to_le_bytes());
    g[6..8].copy_from_slice(&u16::from_str_radix(p[2], 16).ok()?.to_le_bytes());
    let tail = format!("{}{}", p[3], p[4]);
    for i in 0..8 {
        g[8 + i] = u8::from_str_radix(&tail[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(g)
}

/// sake iphlp.rs parse_guid (braces required)
#[allow(clippy::cast_possible_truncation)]
fn old_iphlp(s: &str) -> Option<[u8; 16]> {
    let b = s.as_bytes();
    if b.len() != 38 || b[0] != b'{' || b[37] != b'}' || [9, 14, 19, 24].iter().any(|&i| b[i] != b'-') {
        return None;
    }
    let hex = |r: std::ops::Range<usize>| u64::from_str_radix(&s[r], 16).ok();
    if !s[1..37].chars().all(|c| c == '-' || c.is_ascii_hexdigit()) {
        return None;
    }
    let mut g = [0u8; 16];
    g[0..4].copy_from_slice(&(hex(1..9)? as u32).to_le_bytes());
    g[4..6].copy_from_slice(&(hex(10..14)? as u16).to_le_bytes());
    g[6..8].copy_from_slice(&(hex(15..19)? as u16).to_le_bytes());
    g[8] = hex(20..22)? as u8;
    g[9] = hex(22..24)? as u8;
    for i in 0..6 {
        g[10 + i] = hex(25 + 2 * i..27 + 2 * i)? as u8;
    }
    Some(g)
}

/// sake xof.rs parse_guid (no braces)
#[allow(clippy::cast_possible_truncation)]
fn old_xof(s: &[u8]) -> Option<Guid> {
    fn hexval(c: u8) -> Option<u8> {
        (c as char).to_digit(16).map(|v| v as u8)
    }
    if s.len() != 36 || s[8] != b'-' || s[13] != b'-' || s[18] != b'-' || s[23] != b'-' {
        return None;
    }
    let hex: Vec<u8> = s.iter().copied().filter(|&c| c != b'-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut v = [0u8; 16];
    for (i, b) in v.iter_mut().enumerate() {
        *b = hexval(hex[2 * i])? << 4 | hexval(hex[2 * i + 1])?;
    }
    v[..4].reverse();
    v[4..6].reverse();
    v[6..8].reverse();
    Some(v)
}

/// sake webbrowser.rs guid (for its own well-formed constants)
#[allow(clippy::needless_range_loop)]
fn old_webbrowser(s: &str) -> [u8; 16] {
    let h: Vec<u8> = s.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
    let byte = |i: usize| u8::from_str_radix(std::str::from_utf8(&h[2 * i..2 * i + 2]).unwrap(), 16).unwrap();
    let mut g = [0u8; 16];
    for (o, (at, n)) in [(0usize, (0usize, 4usize)), (4, (4, 2)), (6, (6, 2))] {
        for k in 0..n {
            g[o + k] = byte(at + n - 1 - k);
        }
    }
    for k in 8..16 {
        g[k] = byte(k);
    }
    g
}

/// sake iphlp.rs guid_string (misc.rs's read the same fields from guest memory)
fn old_braced(g: &[u8; 16]) -> String {
    let d1 = u32::from_le_bytes(g[0..4].try_into().unwrap());
    let d2 = u16::from_le_bytes([g[4], g[5]]);
    let d3 = u16::from_le_bytes([g[6], g[7]]);
    format!("{{{d1:08X}-{d2:04X}-{d3:04X}-{:02X}{:02X}-{}}}", g[8], g[9], g[10..].iter().map(|b| format!("{b:02X}")).collect::<String>())
}

/// sake com.rs fmt_guid (on the fields)
fn old_com(g: (u32, u16, u16, [u8; 8])) -> String {
    format!("{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{}", g.0, g.1, g.2, g.3[0], g.3[1], g.3[2..].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// sake mf/reader.rs guid (the other builders spelled the same bytes with an array literal)
fn old_builder(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Guid {
    let mut g = [0u8; 16];
    g[0..4].copy_from_slice(&d1.to_le_bytes());
    g[4..6].copy_from_slice(&d2.to_le_bytes());
    g[6..8].copy_from_slice(&d3.to_le_bytes());
    g[8..].copy_from_slice(&d4);
    g
}

/// sake mf/source.rs g (the const parser; a malformed text failed the build)
fn old_mf_g(s: &str) -> Option<Guid> {
    let mut rest = s.as_bytes();
    let (mut v, mut at) = (0u128, 0u32);
    while let Some((&c, tail)) = rest.split_first() {
        let d = match c {
            b'0'..=b'9' => c.wrapping_sub(b'0'),
            b'a'..=b'f' => c.wrapping_sub(b'a').wrapping_add(10),
            b'A'..=b'F' => c.wrapping_sub(b'A').wrapping_add(10),
            b'-' => 16,
            _ => 17,
        };
        if d == 17 || (d == 16) != matches!(at, 8 | 13 | 18 | 23) {
            return None;
        }
        if d != 16 {
            v = v.wrapping_shl(4) | u128::from(d);
        }
        at = at.wrapping_add(1);
        rest = tail;
    }
    if at != 36 {
        return None;
    }
    let [a0, a1, a2, a3, b0, b1, c0, c1, d0, d1, d2, d3, d4, d5, d6, d7] = v.to_be_bytes();
    Some([a3, a2, a1, a0, b1, b0, c1, c0, d0, d1, d2, d3, d4, d5, d6, d7])
}

#[test]
fn a_constant_is_parsed_at_compile_time() {
    const WB: Guid = of_text("8856F961-340A-11D0-A96B-00C04FD705A2");
    assert_eq!(Some(WB), parse("8856f961-340a-11d0-a96b-00c04fd705a2"));
}

#[test]
#[should_panic(expected = "a GUID's text is")]
fn a_malformed_constant_panics() {
    let _ = of_text(std::hint::black_box("8856F961-340A-11D0-A96B-00C04FD705A"));
}

#[test]
fn known_answers() {
    // IUnknown and CLSID_WebBrowser, as Windows' headers spell them
    assert_eq!(IID_IUNKNOWN, [0, 0, 0, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
    let wb = guid(0x8856_f961, 0x340a, 0x11d0, [0xa9, 0x6b, 0x00, 0xc0, 0x4f, 0xd7, 0x05, 0xa2]);
    assert_eq!(wb[..4], [0x61, 0xf9, 0x56, 0x88]);
    assert_eq!(parse("8856F961-340A-11D0-A96B-00C04FD705A2"), Some(wb));
    assert_eq!(parse("8856f961-340a-11d0-a96b-00c04fd705a2"), Some(wb));
    assert_eq!(braced(&wb), "{8856F961-340A-11D0-A96B-00C04FD705A2}");
    assert_eq!(text(&wb), "8856f961-340a-11d0-a96b-00c04fd705a2");
    assert_eq!(parse_braced("{8856F961-340A-11D0-A96B-00C04FD705A2}"), Some(wb));
    assert_eq!(fields(&wb), (0x8856_f961, 0x340a, 0x11d0, [0xa9, 0x6b, 0x00, 0xc0, 0x4f, 0xd7, 0x05, 0xa2]));
    for bad in ["", "8856F961-340A-11D0-A96B-00C04FD705A", "8856F961-340A-11D0-A96B-00C04FD705A2 ", "8856F961+340A-11D0-A96B-00C04FD705A2", "8856F96G-340A-11D0-A96B-00C04FD705A2", "+856F961-340A-11D0-A96B-00C04FD705A2", "8856F961-340A-11D0-A96B-00C04FD705\u{e9}", "1652c33d-d6b2-4012-b834-72030849a37dd", "1652c33dd6b2-4012-b834-72030849a37d-", "1652c33d-d6b2-4012-b834-72030849a37g", "{652c33d-d6b2-4012-b834-72030849a37d"] {
        assert_eq!(parse(bad), None, "{bad:?}");
    }
}

/// a GUID's text with each letter's case chosen by `case`, and `damage` applied (an index into
/// the text and a byte to put there; none when the index is past the end)
fn spelled(g: &Guid, case: u64, damage: Option<(usize, u8)>) -> String {
    let mut s: Vec<u8> = text(g).bytes().enumerate().map(|(k, c)| if case >> (k % 64) & 1 == 1 { c.to_ascii_uppercase() } else { c }).collect();
    if let Some((at, b)) = damage
        && let Some(x) = s.get_mut(at)
    {
        *x = b;
    }
    String::from_utf8_lossy(&s).into_owned()
}

fn damage() -> impl Strategy<Value = Option<(usize, u8)>> {
    prop_oneof![Just(None), (0usize..40, prop::sample::select(b"-+0aAfFgG {}\x00\xff".to_vec())).prop_map(Some)]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn builders_and_fields_agree(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) {
        let g = guid(d1, d2, d3, d4);
        prop_assert_eq!(g, old_builder(d1, d2, d3, d4));
        prop_assert_eq!(fields(&g), (d1, d2, d3, d4));
        prop_assert_eq!(braced(&g), old_braced(&g));
        prop_assert_eq!(text(&g), old_com((d1, d2, d3, d4)));
        prop_assert_eq!(parse(&text(&g)), Some(g));
        prop_assert_eq!(old_webbrowser(&braced(&g)), g);
    }

    #[test]
    fn parsers_agree_with_the_old_ones(g: [u8; 16], case: u64, bad in damage(), pad in prop::sample::select(vec!["", " ", "\t"])) {
        let s = spelled(&g, case, bad);
        prop_assert_eq!(parse(&s), old_xof(s.as_bytes()));
        prop_assert_eq!(parse(&s), old_mf_g(&s));
        let b = format!("{{{s}}}");
        prop_assert_eq!(parse_braced(&b), old_iphlp(&b));
        // (actctx's: trimmed, then braces optional)
        let t = format!("{pad}{b}{pad}");
        let mine = |t: &str| parse_braced(t.trim()).or_else(|| parse(t.trim()));
        prop_assert_eq!(mine(&t), old_actctx(&t));
        let t = format!("{pad}{s}");
        prop_assert_eq!(mine(&t), old_actctx(&t));
    }
}
