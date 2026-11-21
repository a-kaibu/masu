use super::*;

#[test]
fn clip_count_rejects_what_one_plane_has_outside() {
    let inside = [[-0.5, -0.5, 0.5, 1.0], [-0.5, 0.5, 0.5, 1.0], [0.5, -0.5, 0.5, 1.0]];
    assert_eq!(clip_count(&inside), 1);
    let right = [[2.0, -0.5, 0.5, 1.0], [2.0, 0.5, 0.5, 1.0], [3.0, -0.5, 0.5, 1.0]];
    assert_eq!(clip_count(&right), 0);
    let behind = [[-0.5, -0.5, -0.5, 1.0], [-0.5, 0.5, -0.5, 1.0], [0.5, -0.5, -0.5, 1.0]];
    assert_eq!(clip_count(&behind), 0);
}

#[test]
fn vertex_formats_have_their_sizes() {
    for (f, b) in [(2, 16), (6, 12), (16, 8), (41, 4), (28, 4), (87, 4), (10, 8), (35, 4), (24, 4), (49, 2), (61, 1), (54, 2)] {
        assert_eq!(vformat_bytes(f), Some(b), "format {f}");
    }
    assert_eq!(vformat_bytes(0), None);
}

/// The passes read xd's words by their names (`xw(XD_ITEMS)`, the consts xd_decl writes from
/// xd::NAMED), never by a number that could drift from the Rust side's.
#[test]
fn passes_name_the_words_they_read() {
    let src = include_str!("expand.rs");
    let bare = src.match_indices("xw(").filter(|(i, _)| src.as_bytes().get(i.saturating_add(3)).is_some_and(u8::is_ascii_digit)).count();
    assert_eq!(bare, 0, "xw(<number>) in expand.rs");
    let mut words: Vec<usize> = xd::NAMED.iter().map(|&(_, w)| w).collect();
    words.sort_unstable();
    words.dedup();
    assert_eq!(words.len(), xd::NAMED.len(), "two names for one word");
    assert!(words.iter().all(|&w| w < kome::num::us(XD_VECS).saturating_mul(4)));
}
