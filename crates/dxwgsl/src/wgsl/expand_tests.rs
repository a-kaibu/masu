use super::*;

/// The clipper's CPrimitives of single triangles (winref d3d11gs "pipeline statistics of
/// triangles in clip space", Windows' answers).
#[test]
fn clip_count_of_triangles_as_windows() {
    let t = |a: [f32; 4], b: [f32; 4], c: [f32; 4]| clip_count(&[a, b, c]);
    let h = 0.5;
    assert_eq!(t([-h, -h, h, 1.0], [-h, h, h, 1.0], [h, -h, h, 1.0]), 1, "inside");
    assert_eq!(t([-h, -h, h, 1.0], [h, -h, h, 1.0], [-h, h, h, 1.0]), 1, "counterclockwise");
    assert_eq!(t([2.0, -h, h, 1.0], [2.0, h, h, 1.0], [3.0, -h, h, 1.0]), 0, "outside right");
    assert_eq!(t([-h, -h, h, 1.0], [-h, h, h, 1.0], [1.5, -h, h, 1.0]), 2, "across the right edge");
    assert_eq!(t([-2.0, -h, h, 1.0], [-h, 2.0, h, 1.0], [2.0, -2.0, h, 1.0]), 4, "across two edges");
    assert_eq!(t([-h, -h, h, 1.0], [-h, h, h, 1.0], [100_000.0, -h, h, 1.0]), 2, "far outside the guard band");
    assert_eq!(t([-h, -h, -h, 1.0], [-h, h, h, 1.0], [h, -h, h, 1.0]), 2, "across the near plane");
    assert_eq!(t([-h, -h, 1.5, 1.0], [-h, h, h, 1.0], [h, -h, h, 1.0]), 2, "across the far plane");
    assert_eq!(t([-h, -h, -h, 1.0], [-h, h, -h, 1.0], [h, -h, -h, 1.0]), 0, "behind the near plane");
    assert_eq!(t([-h, -h, h, -1.0], [-h, h, h, 1.0], [h, -h, h, 1.0]), 2, "one w negative");
    assert_eq!(t([h, h, -h, -1.0], [h, -h, -h, -1.0], [-h, h, -h, -1.0]), 0, "all w negative");
    assert_eq!(t([-h, -h, h, 1.0], [-h, -h, h, 1.0], [h, -h, h, 1.0]), 1, "two vertices equal");
    assert_eq!(t([-h, -h, h, 1.0], [0.0, 0.0, h, 1.0], [h, h, h, 1.0]), 1, "on a line");
    assert_eq!(t([0.001, 0.001, h, 1.0], [0.001, 0.002, h, 1.0], [0.002, 0.001, h, 1.0]), 1, "smaller than a pixel");
    assert_eq!(t([-h, -h, h, 0.0], [-h, h, h, 1.0], [h, -h, h, 1.0]), 2, "w zero");
    // (zero-sized viewports: the full-target triangle, clipped to 3)
    assert_eq!(t([-1.0, -1.0, h, 1.0], [3.0, -1.0, h, 1.0], [-1.0, 3.0, h, 1.0]), 3, "a full-target triangle");
}

/// Strips, lines and points of the same draws: the sums Windows answers.
#[test]
fn clip_count_of_strips_lines_points_as_windows() {
    let h = 0.5;
    let q = [[-h, -h, h, 1.0], [-h, h, h, 1.0], [h, -h, h, 1.0], [h, h, h, 1.0], [2.0, 2.0, h, 1.0], [3.0, 2.0, h, 1.0]];
    let at = |i: usize| q.get(i).copied().unwrap_or_default();
    let strip: u32 = [[0, 1, 2], [1, 3, 2], [2, 3, 4], [3, 5, 4]].iter().map(|t| clip_count(&t.map(at))).sum();
    assert_eq!(strip, 5, "strip of 6 (2 outside)");
    let lines: u32 = [[0, 1], [2, 3], [4, 5]].iter().map(|l| clip_count(&l.map(at))).sum();
    assert_eq!(lines, 2, "line list of 6");
    let lstrip: u32 = (0..5).map(|i| clip_count(&[at(i), at(i + 1)])).sum();
    assert_eq!(lstrip, 4, "line strip of 6");
    let points: u32 = (0..6).map(|i| clip_count(&[at(i)])).sum();
    assert_eq!(points, 4, "point list of 6");
    assert_eq!(clip_count(&[at(0), at(2), at(4)]), 2, "triangle list adjacency of 6");
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
