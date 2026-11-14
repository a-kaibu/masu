//! CompareString / lstrcmp ordering: Windows' "word sort" by default (hyphens and apostrophes
//! only break ties), SORT_STRINGSORT making them symbols. A string's key is, per character, a
//! primary weight (symbols < digits < Latin < Greek < Cyrillic < kana < the rest), a diacritic
//! weight (a Latin letter's marks; a kana's size and voicing), a case weight (upper over wide:
//! e < ｅ < E < Ｅ), a kana kind (katakana < hiragana) and a kana width (half < full); keys
//! compare level by level, whole string by whole string, the hyphens and apostrophes last.
//! Measured on Windows Server 2025 (tests/winref/probes/collate.c, #242): ア + a < ｱ + A (the
//! case of the second letter decides before the width of the first kana), ｱ + あ > ア + ア (the
//! kana kind before the width), あ > ぁ and ignoring marks あ = ぁ (size is a diacritic). Measured on Windows Server 2025 (tests/winref/probes/nls.c,
//! 22 strings against each other in five ways): a < A < b, -a after a and before A, _a before
//! a, ' < - < _ < ・ < digits, あ > ア > ｱ (case-insensitive too), "10" < "2".

use std::cmp::Ordering;

pub const NORM_IGNORECASE: u32 = 1;
pub const NORM_IGNORENONSPACE: u32 = 2;
pub const NORM_IGNORESYMBOLS: u32 = 4;
pub const SORT_STRINGSORT: u32 = 0x1000;
pub const NORM_IGNOREKANATYPE: u32 = 0x10000;
pub const NORM_IGNOREWIDTH: u32 = 0x20000;

/// ASCII symbols in Windows' order (string sort puts ' and - first)
const SYMBOLS: &[u8] = b"'-\t\n\x0b\x0c\r !\"#$%&()*,./:;?@[\\]^_`{|}~+<=>";

/// Each ASCII character's place in SYMBOLS (0xff: not a symbol), so a letter is not compared
/// with the 34 symbols first
const SYMBOL_AT: [u8; 128] = {
    let mut t = [0xff; 128];
    let mut i = 0;
    while i < SYMBOLS.len() {
        t[SYMBOLS[i] as usize] = i as u8;
        i += 1;
    }
    t
};

const G_SYMBOL: u32 = 1;
const G_DIGIT: u32 = 2;
const G_LATIN: u32 = 3;
const G_GREEK: u32 = 4;
const G_CYRILLIC: u32 = 5;
const G_KANA: u32 = 6;
const G_OTHER: u32 = 7;

/// Case weight bits: upper case over width
const UPPER: u8 = 2;
const WIDE: u8 = 1;
/// Kana kind
const KATAKANA: u8 = 0;
const HIRAGANA: u8 = 1;
/// Kana width
const HALF: u8 = 0;
const FULL: u8 = 1;
/// A kana's diacritic: small, plain, voiced, semi-voiced
const SMALL: u16 = 0;
const PLAIN: u16 = 1;

/// What one character weighs at each level after the primary
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Weights {
    primary: u32,
    diacritic: u16,
    case: u8,
    kind: u8,
    width: u8,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Weight {
    Char(Weights),
    /// a hyphen or apostrophe in word sort: its order among them
    Special(u8),
    Ignore,
}

/// The marks of Latin letters, in the order they sort (Unicode's DUCET secondary order; on Windows
/// measured: acute < grave, tests/winref/golden/collate-*): ́ ̀ ̆ ̂ ̌ ̊ ̈ ̋ ̃ ̇ ̧ ̨ ̄ ̣ ̛ ̦ ̏ ̑ ̉ ̤ ̭ ̰ ̱ ̮ ̥
/// The Latin letters with marks of U+00C0..U+024F (their canonical decompositions): the letter,
/// its base letter and its marks' places in MARKS (1 the first; 0 none), sorted by letter
const MARKED: [(u16, u8, [u8; 2]); 244] = [
    (0xc0, b'A', [2, 0]), (0xc1, b'A', [1, 0]), (0xc2, b'A', [4, 0]), (0xc3, b'A', [9, 0]),
    (0xc4, b'A', [7, 0]), (0xc5, b'A', [6, 0]), (0xc7, b'C', [11, 0]), (0xc8, b'E', [2, 0]),
    (0xc9, b'E', [1, 0]), (0xca, b'E', [4, 0]), (0xcb, b'E', [7, 0]), (0xcc, b'I', [2, 0]),
    (0xcd, b'I', [1, 0]), (0xce, b'I', [4, 0]), (0xcf, b'I', [7, 0]), (0xd1, b'N', [9, 0]),
    (0xd2, b'O', [2, 0]), (0xd3, b'O', [1, 0]), (0xd4, b'O', [4, 0]), (0xd5, b'O', [9, 0]),
    (0xd6, b'O', [7, 0]), (0xd9, b'U', [2, 0]), (0xda, b'U', [1, 0]), (0xdb, b'U', [4, 0]),
    (0xdc, b'U', [7, 0]), (0xdd, b'Y', [1, 0]), (0xe0, b'a', [2, 0]), (0xe1, b'a', [1, 0]),
    (0xe2, b'a', [4, 0]), (0xe3, b'a', [9, 0]), (0xe4, b'a', [7, 0]), (0xe5, b'a', [6, 0]),
    (0xe7, b'c', [11, 0]), (0xe8, b'e', [2, 0]), (0xe9, b'e', [1, 0]), (0xea, b'e', [4, 0]),
    (0xeb, b'e', [7, 0]), (0xec, b'i', [2, 0]), (0xed, b'i', [1, 0]), (0xee, b'i', [4, 0]),
    (0xef, b'i', [7, 0]), (0xf1, b'n', [9, 0]), (0xf2, b'o', [2, 0]), (0xf3, b'o', [1, 0]),
    (0xf4, b'o', [4, 0]), (0xf5, b'o', [9, 0]), (0xf6, b'o', [7, 0]), (0xf9, b'u', [2, 0]),
    (0xfa, b'u', [1, 0]), (0xfb, b'u', [4, 0]), (0xfc, b'u', [7, 0]), (0xfd, b'y', [1, 0]),
    (0xff, b'y', [7, 0]), (0x100, b'A', [13, 0]), (0x101, b'a', [13, 0]), (0x102, b'A', [3, 0]),
    (0x103, b'a', [3, 0]), (0x104, b'A', [12, 0]), (0x105, b'a', [12, 0]), (0x106, b'C', [1, 0]),
    (0x107, b'c', [1, 0]), (0x108, b'C', [4, 0]), (0x109, b'c', [4, 0]), (0x10a, b'C', [10, 0]),
    (0x10b, b'c', [10, 0]), (0x10c, b'C', [5, 0]), (0x10d, b'c', [5, 0]), (0x10e, b'D', [5, 0]),
    (0x10f, b'd', [5, 0]), (0x112, b'E', [13, 0]), (0x113, b'e', [13, 0]), (0x114, b'E', [3, 0]),
    (0x115, b'e', [3, 0]), (0x116, b'E', [10, 0]), (0x117, b'e', [10, 0]), (0x118, b'E', [12, 0]),
    (0x119, b'e', [12, 0]), (0x11a, b'E', [5, 0]), (0x11b, b'e', [5, 0]), (0x11c, b'G', [4, 0]),
    (0x11d, b'g', [4, 0]), (0x11e, b'G', [3, 0]), (0x11f, b'g', [3, 0]), (0x120, b'G', [10, 0]),
    (0x121, b'g', [10, 0]), (0x122, b'G', [11, 0]), (0x123, b'g', [11, 0]), (0x124, b'H', [4, 0]),
    (0x125, b'h', [4, 0]), (0x128, b'I', [9, 0]), (0x129, b'i', [9, 0]), (0x12a, b'I', [13, 0]),
    (0x12b, b'i', [13, 0]), (0x12c, b'I', [3, 0]), (0x12d, b'i', [3, 0]), (0x12e, b'I', [12, 0]),
    (0x12f, b'i', [12, 0]), (0x130, b'I', [10, 0]), (0x134, b'J', [4, 0]), (0x135, b'j', [4, 0]),
    (0x136, b'K', [11, 0]), (0x137, b'k', [11, 0]), (0x139, b'L', [1, 0]), (0x13a, b'l', [1, 0]),
    (0x13b, b'L', [11, 0]), (0x13c, b'l', [11, 0]), (0x13d, b'L', [5, 0]), (0x13e, b'l', [5, 0]),
    (0x143, b'N', [1, 0]), (0x144, b'n', [1, 0]), (0x145, b'N', [11, 0]), (0x146, b'n', [11, 0]),
    (0x147, b'N', [5, 0]), (0x148, b'n', [5, 0]), (0x14c, b'O', [13, 0]), (0x14d, b'o', [13, 0]),
    (0x14e, b'O', [3, 0]), (0x14f, b'o', [3, 0]), (0x150, b'O', [8, 0]), (0x151, b'o', [8, 0]),
    (0x154, b'R', [1, 0]), (0x155, b'r', [1, 0]), (0x156, b'R', [11, 0]), (0x157, b'r', [11, 0]),
    (0x158, b'R', [5, 0]), (0x159, b'r', [5, 0]), (0x15a, b'S', [1, 0]), (0x15b, b's', [1, 0]),
    (0x15c, b'S', [4, 0]), (0x15d, b's', [4, 0]), (0x15e, b'S', [11, 0]), (0x15f, b's', [11, 0]),
    (0x160, b'S', [5, 0]), (0x161, b's', [5, 0]), (0x162, b'T', [11, 0]), (0x163, b't', [11, 0]),
    (0x164, b'T', [5, 0]), (0x165, b't', [5, 0]), (0x168, b'U', [9, 0]), (0x169, b'u', [9, 0]),
    (0x16a, b'U', [13, 0]), (0x16b, b'u', [13, 0]), (0x16c, b'U', [3, 0]), (0x16d, b'u', [3, 0]),
    (0x16e, b'U', [6, 0]), (0x16f, b'u', [6, 0]), (0x170, b'U', [8, 0]), (0x171, b'u', [8, 0]),
    (0x172, b'U', [12, 0]), (0x173, b'u', [12, 0]), (0x174, b'W', [4, 0]), (0x175, b'w', [4, 0]),
    (0x176, b'Y', [4, 0]), (0x177, b'y', [4, 0]), (0x178, b'Y', [7, 0]), (0x179, b'Z', [1, 0]),
    (0x17a, b'z', [1, 0]), (0x17b, b'Z', [10, 0]), (0x17c, b'z', [10, 0]), (0x17d, b'Z', [5, 0]),
    (0x17e, b'z', [5, 0]), (0x1a0, b'O', [15, 0]), (0x1a1, b'o', [15, 0]), (0x1af, b'U', [15, 0]),
    (0x1b0, b'u', [15, 0]), (0x1cd, b'A', [5, 0]), (0x1ce, b'a', [5, 0]), (0x1cf, b'I', [5, 0]),
    (0x1d0, b'i', [5, 0]), (0x1d1, b'O', [5, 0]), (0x1d2, b'o', [5, 0]), (0x1d3, b'U', [5, 0]),
    (0x1d4, b'u', [5, 0]), (0x1d5, b'U', [7, 13]), (0x1d6, b'u', [7, 13]), (0x1d7, b'U', [7, 1]),
    (0x1d8, b'u', [7, 1]), (0x1d9, b'U', [7, 5]), (0x1da, b'u', [7, 5]), (0x1db, b'U', [7, 2]),
    (0x1dc, b'u', [7, 2]), (0x1de, b'A', [7, 13]), (0x1df, b'a', [7, 13]), (0x1e0, b'A', [10, 13]),
    (0x1e1, b'a', [10, 13]), (0x1e6, b'G', [5, 0]), (0x1e7, b'g', [5, 0]), (0x1e8, b'K', [5, 0]),
    (0x1e9, b'k', [5, 0]), (0x1ea, b'O', [12, 0]), (0x1eb, b'o', [12, 0]), (0x1ec, b'O', [12, 13]),
    (0x1ed, b'o', [12, 13]), (0x1f0, b'j', [5, 0]), (0x1f4, b'G', [1, 0]), (0x1f5, b'g', [1, 0]),
    (0x1f8, b'N', [2, 0]), (0x1f9, b'n', [2, 0]), (0x1fa, b'A', [6, 1]), (0x1fb, b'a', [6, 1]),
    (0x200, b'A', [17, 0]), (0x201, b'a', [17, 0]), (0x202, b'A', [18, 0]), (0x203, b'a', [18, 0]),
    (0x204, b'E', [17, 0]), (0x205, b'e', [17, 0]), (0x206, b'E', [18, 0]), (0x207, b'e', [18, 0]),
    (0x208, b'I', [17, 0]), (0x209, b'i', [17, 0]), (0x20a, b'I', [18, 0]), (0x20b, b'i', [18, 0]),
    (0x20c, b'O', [17, 0]), (0x20d, b'o', [17, 0]), (0x20e, b'O', [18, 0]), (0x20f, b'o', [18, 0]),
    (0x210, b'R', [17, 0]), (0x211, b'r', [17, 0]), (0x212, b'R', [18, 0]), (0x213, b'r', [18, 0]),
    (0x214, b'U', [17, 0]), (0x215, b'u', [17, 0]), (0x216, b'U', [18, 0]), (0x217, b'u', [18, 0]),
    (0x218, b'S', [16, 0]), (0x219, b's', [16, 0]), (0x21a, b'T', [16, 0]), (0x21b, b't', [16, 0]),
    (0x21e, b'H', [5, 0]), (0x21f, b'h', [5, 0]), (0x226, b'A', [10, 0]), (0x227, b'a', [10, 0]),
    (0x228, b'E', [11, 0]), (0x229, b'e', [11, 0]), (0x22a, b'O', [7, 13]), (0x22b, b'o', [7, 13]),
    (0x22c, b'O', [9, 13]), (0x22d, b'o', [9, 13]), (0x22e, b'O', [10, 0]), (0x22f, b'o', [10, 0]),
    (0x230, b'O', [10, 13]), (0x231, b'o', [10, 13]), (0x232, b'Y', [13, 0]),
    (0x233, b'y', [13, 0]),
];

// (marked's binary search needs MARKED sorted, each letter once: checked when compiling)
const _: () = {
    let mut i = 1;
    while i < MARKED.len() {
        assert!(MARKED[i - 1].0 < MARKED[i].0, "MARKED out of order");
        i += 1;
    }
};

/// A Latin letter of U+00C0..U+024F with marks: its base letter and diacritic weight (the
/// first mark's place, then the second's)
fn marked(c: u16) -> Option<(u8, u16)> {
    let i = MARKED.binary_search_by_key(&c, |m| m.0).ok()?;
    let (_, base, [m1, m2]) = MARKED[i];
    Some((base, u16::from(m1) * 32 + u16::from(m2)))
}

/// The katakana of a kana (hiragana and half-width ones too), with its kind and width
fn kana(c: u16) -> Option<(u16, u8, u8)> {
    match c {
        0x3041..=0x3096 | 0x309d | 0x309e => Some((c + 0x60, HIRAGANA, FULL)),
        0x30a1..=0x30fa | 0x30fc..=0x30fe => Some((c, KATAKANA, FULL)),
        0xff66..=0xff9d => Some((crate::text::half_kana_full(c), KATAKANA, HALF)),
        _ => None,
    }
}

/// A katakana's plain large form and its diacritic (small < plain < voiced < semi-voiced)
fn kana_base(k: u16) -> (u16, u16) {
    if let Some((b, m)) = crate::text::kana_voiced(k) {
        return (b, if m == 0xff9e { PLAIN + 1 } else { PLAIN + 2 });
    }
    match k {
        0x30a1 | 0x30a3 | 0x30a5 | 0x30a7 | 0x30a9 | 0x30c3 | 0x30e3 | 0x30e5 | 0x30e7 | 0x30ee => (k + 1, SMALL),
        0x30f5 => (0x30ab, SMALL),
        0x30f6 => (0x30b1, SMALL),
        _ => (k, PLAIN),
    }
}

fn weight(c: u16, flags: u32) -> Weight {
    let p = |g: u32, w: u32| (g << 24) | w;
    let ch = |primary: u32, case: u8| Weight::Char(Weights { primary, diacritic: 0, case, kind: 0, width: 0 });
    // full-width ASCII forms: the ASCII character, wide
    let (c, wide) = match c {
        0xff01..=0xff5e => (c - 0xfee0, WIDE),
        0x3000 => (0x20, WIDE),
        // (the half-width ｡｢｣､･ are the full-width 。「」、・ narrow)
        0xff61..=0xff65 => (crate::text::half_kana_full(c), 0),
        0x3001 | 0x3002 | 0x300c | 0x300d | 0x30fb => (c, WIDE),
        _ => (c, 0),
    };
    if flags & SORT_STRINGSORT == 0 && matches!(c, 0x27 | 0x2d | 0xad | 0x2010 | 0x2011) {
        return Weight::Special(if c == 0x27 { 0 } else { 1 });
    }
    if let Some(&at) = SYMBOL_AT.get(usize::from(c)).filter(|&&at| at != 0xff) {
        return ch(p(G_SYMBOL, u32::from(at)), wide);
    }
    match c {
        0..=0x1f | 0x7f..=0x9f => Weight::Ignore,
        0x30..=0x39 => ch(p(G_DIGIT, u32::from(c - 0x30)), wide),
        0x41..=0x5a => ch(p(G_LATIN, u32::from(c - 0x41)), wide | UPPER),
        0x61..=0x7a => ch(p(G_LATIN, u32::from(c - 0x61)), wide),
        _ => {
            let latin = |b: u8, diacritic: u16| {
                let case = if b.is_ascii_uppercase() { UPPER } else { 0 };
                Weight::Char(Weights { primary: p(G_LATIN, u32::from(b.to_ascii_lowercase() - b'a')), diacritic, case, kind: 0, width: 0 })
            };
            if let Some((b, diacritic)) = marked(c) {
                return latin(b, diacritic);
            }
            let chr = char::from_u32(u32::from(c));
            if let Some(b) = chr.and_then(crate::text::best_fit) {
                // (a letter of its own that Windows' best fit makes a Latin one, ø, đ, ł:
                // after every mark of MARKED)
                return latin(b, 0x400 | (crate::text::lower16(c) & 0xff));
            }
            if let Some((k, kind, width)) = kana(c) {
                let (base, diacritic) = kana_base(k);
                return Weight::Char(Weights { primary: p(G_KANA, u32::from(base)), diacritic, case: 0, kind, width });
            }
            let lower = crate::text::lower16(c);
            let up = if lower != c { UPPER } else { 0 };
            let is_alnum = chr.is_some_and(char::is_alphanumeric);
            match c {
                0x370..=0x3ff => ch(p(G_GREEK, u32::from(lower)), up),
                0x400..=0x4ff => ch(p(G_CYRILLIC, u32::from(lower)), up),
                0xc0..=0x24f => ch(p(G_LATIN, 26 + u32::from(lower)), up),
                _ if !is_alnum => ch(p(G_SYMBOL, 0x100 + u32::from(c)), 0),
                _ => ch(p(G_OTHER, u32::from(lower)), up),
            }
        }
    }
}

/// A character's place in the key under `flags`: Ok(its weights), Err(kind) for a
/// hyphen/apostrophe special, None for nothing
fn level(c: u16, flags: u32) -> Option<Result<Weights, u8>> {
    match weight(c, flags) {
        Weight::Ignore => None,
        Weight::Special(k) => (flags & NORM_IGNORESYMBOLS == 0).then_some(Err(k)),
        Weight::Char(mut w) => {
            if flags & NORM_IGNORESYMBOLS != 0 && w.primary >> 24 == G_SYMBOL {
                return None;
            }
            if flags & NORM_IGNORENONSPACE != 0 {
                w.diacritic = 0;
            }
            if flags & NORM_IGNORECASE != 0 {
                w.case &= !UPPER;
            }
            if flags & NORM_IGNOREWIDTH != 0 {
                w.case &= !WIDE;
                w.width = 0;
            }
            if flags & NORM_IGNOREKANATYPE != 0 {
                w.kind = 0;
            }
            Some(Ok(w))
        }
    }
}

/// The levels of a string's key, one after the other: primaries, diacritics, case, kana kind,
/// kana width, specials (position, kind)
#[cfg(test)]
#[derive(PartialEq, Eq, Debug, Default)]
struct Key {
    primary: Vec<u32>,
    diacritic: Vec<u16>,
    case: Vec<u8>,
    kind: Vec<u8>,
    width: Vec<u8>,
    specials: Vec<(usize, u8)>,
}

#[cfg(test)]
fn key(s: &[u16], flags: u32) -> Key {
    let mut k = Key::default();
    for (i, &c) in s.iter().enumerate() {
        match level(c, flags) {
            None => {}
            Some(Err(kind)) => k.specials.push((i, kind)),
            Some(Ok(w)) => {
                k.primary.push(w.primary);
                k.diacritic.push(w.diacritic);
                k.case.push(w.case);
                k.kind.push(w.kind);
                k.width.push(w.width);
            }
        }
    }
    k
}

/// CompareString's answer for UTF-16 strings `a` and `b` under `flags`: the key's levels one
/// after the other, each read from the strings as it is compared (no key is built: most
/// comparisons end at the first primary that differs)
pub fn compare(a: &[u16], b: &[u16], flags: u32) -> Ordering {
    fn chars(s: &[u16], flags: u32) -> impl Iterator<Item = Weights> + '_ {
        s.iter().filter_map(move |&c| level(c, flags)?.ok())
    }
    fn specials(s: &[u16], flags: u32) -> impl Iterator<Item = u8> + '_ {
        s.iter().filter_map(move |&c| level(c, flags)?.err())
    }
    let chars = |s| chars(s, flags);
    let specials = |s| specials(s, flags);
    chars(a).map(|w| w.primary).cmp(chars(b).map(|w| w.primary))
        .then_with(|| chars(a).map(|w| w.diacritic).cmp(chars(b).map(|w| w.diacritic)))
        .then_with(|| chars(a).map(|w| w.case).cmp(chars(b).map(|w| w.case)))
        .then_with(|| chars(a).map(|w| w.kind).cmp(chars(b).map(|w| w.kind)))
        .then_with(|| chars(a).map(|w| w.width).cmp(chars(b).map(|w| w.width)))
        .then_with(|| specials(a).cmp(specials(b)))
}

#[cfg(test)]
#[path = "collate_tests.rs"]
mod tests;
