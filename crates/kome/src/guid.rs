//! GUIDs as the 16 bytes Windows keeps them in (Data1, Data2, Data3 little-endian, Data4 as
//! written): built from their fields, parsed from and written as their text forms. sake, kura and
//! asf each had their own copies (eight builders, four parsers, three printers).

/// A GUID's 16 bytes, in memory order.
pub type Guid = [u8; 16];

/// The GUID {d1-d2-d3-d4[0..2]-d4[2..8]}.
pub const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Guid {
    let [a0, a1, a2, a3] = d1.to_le_bytes();
    let [b0, b1] = d2.to_le_bytes();
    let [c0, c1] = d3.to_le_bytes();
    let [e0, e1, e2, e3, e4, e5, e6, e7] = d4;
    [a0, a1, a2, a3, b0, b1, c0, c1, e0, e1, e2, e3, e4, e5, e6, e7]
}

/// The fields `guid` takes: (Data1, Data2, Data3, Data4).
pub const fn fields(g: &Guid) -> (u32, u16, u16, [u8; 8]) {
    let [a0, a1, a2, a3, b0, b1, c0, c1, e0, e1, e2, e3, e4, e5, e6, e7] = *g;
    (u32::from_le_bytes([a0, a1, a2, a3]), u16::from_le_bytes([b0, b1]), u16::from_le_bytes([c0, c1]), [e0, e1, e2, e3, e4, e5, e6, e7])
}

/// IUnknown's IID {00000000-0000-0000-C000-000000000046}.
pub const IID_IUNKNOWN: Guid = guid(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);

/// "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX" (hex digits of either case, nothing around it).
pub const fn parse(s: &str) -> Option<Guid> {
    let mut rest = s.as_bytes();
    // (the 32 hex digits as one big-endian number, the dashes where they belong)
    let (mut v, mut at) = (0u128, 0u32);
    while let Some((&c, tail)) = rest.split_first() {
        let d = match c {
            b'0'..=b'9' => c.wrapping_sub(b'0'),
            b'a'..=b'f' => c.wrapping_sub(b'a').wrapping_add(10),
            b'A'..=b'F' => c.wrapping_sub(b'A').wrapping_add(10),
            b'-' => 16,
            _ => return None,
        };
        if (d == 16) != matches!(at, 8 | 13 | 18 | 23) {
            return None;
        }
        if d != 16 {
            v = v.wrapping_shl(4) | u128::from_le_bytes([d, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
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

/// [`parse`] for a constant: `const { of_text("..") }`, a malformed text fails the build.
///
/// ```
/// const WB: kome::guid::Guid = kome::guid::of_text("8856F961-340A-11D0-A96B-00C04FD705A2");
/// assert_eq!(WB[..4], [0x61, 0xf9, 0x56, 0x88]);
/// ```
/// ```compile_fail
/// const BAD: kome::guid::Guid = kome::guid::of_text("8856F961-340A-11D0-A96B-00C04FD705A");
/// ```
pub const fn of_text(s: &str) -> Guid {
    match parse(s) {
        Some(g) => g,
        None => panic!("a GUID's text is XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX (36 characters: hex digits, dashes at 8, 13, 18, 23)"),
    }
}

/// "{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}" (the braces required).
pub fn parse_braced(s: &str) -> Option<Guid> {
    parse(s.strip_prefix('{')?.strip_suffix('}')?)
}

/// The registry's and the API's form: "{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}", upper case.
pub fn braced(g: &Guid) -> String {
    format!("{{{}}}", text(g).to_ascii_uppercase())
}

/// "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx", lower case, no braces.
pub fn text(g: &Guid) -> String {
    let (d1, d2, d3, [e0, e1, e2, e3, e4, e5, e6, e7]) = fields(g);
    format!("{d1:08x}-{d2:04x}-{d3:04x}-{e0:02x}{e1:02x}-{e2:02x}{e3:02x}{e4:02x}{e5:02x}{e6:02x}{e7:02x}")
}

#[cfg(test)]
#[path = "guid_tests.rs"]
mod tests;
