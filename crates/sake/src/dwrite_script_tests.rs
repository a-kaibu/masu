//! Script runs against the code as it was before a script became a type: every code point's
//! script, and random texts split, compared with the old numbers.
use super::*;
use proptest::prelude::*;

fn old_script(k: u32) -> u8 {
    match k {
        0x41..=0x5a | 0x61..=0x7a | 0xc0..=0x24f => 1,
        0x3005 => 2,
        0..=0x7f | 0x80..=0xbf | 0x3000..=0x303f | 0x30fb | 0x30fc | 0xff01..=0xff20 | 0xff3b..=0xff40 | 0xff5b..=0xff65 => 0,
        0x3400..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x3ffff => 2,
        0x3040..=0x309f => 3,
        0x30a0..=0x30ff | 0x31f0..=0x31ff | 0xff66..=0xff9f => 4,
        0x2600..=0x27bf | 0x1f000.. => 5,
        0xff21..=0xff3a | 0xff41..=0xff5a => 1,
        _ => 6,
    }
}

fn old_split(ch: &[u32], from: usize, to: usize) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let (mut start, mut cur) = (from, 0u8);
    for i in from..to {
        let s = old_script(ch[i]);
        if s == 0 || s == cur {
            continue;
        }
        if cur != 0 {
            v.push((start, i));
            start = i;
        }
        cur = s;
    }
    if start < to {
        v.push((start, to));
    }
    v
}

fn number(s: Option<Script>) -> u8 {
    match s {
        None => 0,
        Some(Script::Latin) => 1,
        Some(Script::Han) => 2,
        Some(Script::Hiragana) => 3,
        Some(Script::Katakana) => 4,
        Some(Script::Symbol) => 5,
        Some(Script::Other) => 6,
    }
}

#[test]
fn every_code_point_has_its_script() {
    for k in (0..0x4_0000).chain([0x10_ffff, 0x11_0000, u32::MAX]) {
        assert_eq!(number(script(k)), old_script(k), "{k:#x}");
    }
}

proptest! {
    #[test]
    fn texts_split_as_before(ch in proptest::collection::vec(prop_oneof![
        0u32..0x80, 0x3000u32..0x3100, 0x3400u32..0x3410, 0xff00u32..0xffa0, 0x2600u32..0x2610, 0x1f600u32..0x1f610, 0x400u32..0x410, Just(0x3005u32)], 0..24),
        a in 0usize..24, b in 0usize..24) {
        let (from, to) = (a.min(b).min(ch.len()), a.max(b).min(ch.len()));
        prop_assert_eq!(split_scripts(&ch, from, to), old_split(&ch, from, to));
    }
}
