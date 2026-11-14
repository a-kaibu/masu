//! Stream names in the compound file: pairs of [0-9A-Za-z._] packed into one character
//! (0x3800 + second << 6 + first), a lone one as 0x4800 + it, and tables prefixed by 0x4840.

pub const TABLE_PREFIX: char = '\u{4840}';

fn b64(c: char) -> Option<u32> {
    Some(match c {
        '0'..='9' => c as u32 - '0' as u32,
        'A'..='Z' => c as u32 - 'A' as u32 + 10,
        'a'..='z' => c as u32 - 'a' as u32 + 36,
        '.' => 62,
        '_' => 63,
        _ => return None,
    })
}

/// The character of 6 bits (the bits above them not looked at).
fn unb64(v: u32) -> char {
    let v = v.to_le_bytes()[0] & 0x3f;
    char::from(match v {
        0..=9 => b'0' + v,
        10..=35 => b'A' + v - 10,
        36..=61 => b'a' + v - 36,
        62 => b'.',
        63.. => b'_',
    })
}

/// A character of the encoded name: 0x3800.. (a pair) and 0x4800.. (one) are never surrogates.
fn encoded(v: u32) -> char {
    char::from_u32(v).unwrap_or(char::REPLACEMENT_CHARACTER)
}

pub fn encode(name: &str, table: bool) -> String {
    let mut out = String::new();
    if table {
        out.push(TABLE_PREFIX);
    }
    let mut it = name.chars().peekable();
    while let Some(c) = it.next() {
        match b64(c) {
            Some(a) => match it.peek().and_then(|&n| b64(n)) {
                Some(b) => {
                    it.next();
                    out.push(encoded(0x3800 + (b << 6) + a));
                }
                None => out.push(encoded(0x4800 + a)),
            },
            None => out.push(c),
        }
    }
    out
}

/// The name and whether it is a table's stream.
pub fn decode(name: &str) -> (String, bool) {
    let mut out = String::new();
    let mut it = name.chars().peekable();
    let table = it.peek() == Some(&TABLE_PREFIX);
    if table {
        it.next();
    }
    for c in it {
        let v = c as u32;
        if (0x3800..0x4800).contains(&v) {
            out.push(unb64((v - 0x3800) & 0x3f));
            out.push(unb64((v - 0x3800) >> 6));
        } else if (0x4800..0x4840).contains(&v) {
            out.push(unb64(v - 0x4800));
        } else {
            out.push(c);
        }
    }
    (out, table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn known() {
        assert_eq!(encode("_Columns", true), "\u{4840}\u{3b3f}\u{43f2}\u{4438}\u{45b1}");
        assert_eq!(encode("_Tables", true), "\u{4840}\u{3f7f}\u{4164}\u{422f}\u{4836}");
        assert_eq!(encode("App.exe", false), "\u{44ca}\u{47b3}\u{46e8}\u{4828}");
        assert_eq!(decode("\u{44ca}\u{47b3}\u{46e8}\u{4828}"), ("App.exe".into(), false));
        assert_eq!(decode("\u{5}SummaryInformation"), ("\u{5}SummaryInformation".into(), false));
    }

    proptest! {
        /// Every encoded character of any name (b64 pairs and singles): a character, never the
        /// replacement; and unb64 as the old from_u32 arithmetic.
        #[test]
        fn encoded_chars_are_chars(a in 0u32..64, b in 0u32..64) {
            prop_assert_ne!(encoded(0x3800 + (b << 6) + a), char::REPLACEMENT_CHARACTER);
            prop_assert_ne!(encoded(0x4800 + a), char::REPLACEMENT_CHARACTER);
            let old = char::from_u32(match a { 0..=9 => '0' as u32 + a, 10..=35 => 'A' as u32 + a - 10, 36..=61 => 'a' as u32 + a - 36, 62 => '.' as u32, _ => '_' as u32 }).unwrap();
            prop_assert_eq!(unb64(a), old);
        }

        #[test]
        fn round_trip(s in "[0-9A-Za-z._#!あ-ん]{0,40}", t: bool) {
            prop_assert_eq!(decode(&encode(&s, t)), (s, t));
        }
    }
}
