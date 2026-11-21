//! Little- (and big-) endian numbers in bytes, without a panic or an unwrap: from an array at an
//! offset fixed at compile time (one past the array's end does not compile), or from a slice at
//! any offset (None past its end).

/// Bytes `AT..AT + L` of `b` (one past its end does not compile).
pub fn arr<const AT: usize, const L: usize, const N: usize>(b: &[u8; N]) -> [u8; L] {
    const { assert!(AT + L <= N, "read past the end of the array") };
    std::array::from_fn(|i| b[AT + i])
}

pub fn u16<const AT: usize, const N: usize>(b: &[u8; N]) -> u16 {
    u16::from_le_bytes(arr::<AT, 2, N>(b))
}

pub fn u32<const AT: usize, const N: usize>(b: &[u8; N]) -> u32 {
    u32::from_le_bytes(arr::<AT, 4, N>(b))
}

/// `L` bytes of `b` at `at`; None when they are not all there.
pub fn bytes_at<const L: usize>(b: &[u8], at: usize) -> Option<[u8; L]> {
    b.get(at..)?.first_chunk::<L>().copied()
}

pub fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    bytes_at(b, at).map(u16::from_le_bytes)
}

pub fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    bytes_at(b, at).map(u32::from_le_bytes)
}

pub fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    bytes_at(b, at).map(u64::from_le_bytes)
}

/// The big-endian ones (MP4 boxes, TrueType tables, AIFF chunks).
pub fn u16_be_at(b: &[u8], at: usize) -> Option<u16> {
    bytes_at(b, at).map(u16::from_be_bytes)
}

pub fn u32_be_at(b: &[u8], at: usize) -> Option<u32> {
    bytes_at(b, at).map(u32::from_be_bytes)
}

pub fn u64_be_at(b: &[u8], at: usize) -> Option<u64> {
    bytes_at(b, at).map(u64::from_be_bytes)
}

/// `b` as whole `L`-byte pieces (a short last one left out).
pub fn pieces<const L: usize>(b: &[u8]) -> &[[u8; L]] {
    b.as_chunks::<L>().0
}

/// `b`'s 16-bit little-endian units (a short last byte left out, as [`pieces`] does).
pub fn units16(b: &[u8]) -> impl Iterator<Item = u16> + '_ {
    pieces::<2>(b).iter().map(|&p| u16::from_le_bytes(p))
}

/// `b` as UTF-16LE text, a unit that is no character as U+FFFD (Rust 1.98's
/// String::from_utf16le_lossy, with no Vec<u16> between; a short last byte is no unit and is
/// left out, as [`pieces`] does: from_utf16le_lossy alone would make it a U+FFFD).
pub fn utf16_lossy(b: &[u8]) -> String {
    String::from_utf16le_lossy(pieces::<2>(b).as_flattened())
}

/// [`utf16_lossy`] up to the first NUL unit (all of `b` without one).
pub fn utf16z_lossy(b: &[u8]) -> String {
    let p = pieces::<2>(b);
    let n = p.iter().position(|u| *u == [0, 0]).unwrap_or(p.len());
    String::from_utf16le_lossy(p.get(..n).unwrap_or(p).as_flattened())
}

#[cfg(test)]
mod tests {
    use super::*;
    proptest::proptest! {
        /// The same numbers as slicing and from_le_bytes (the old way), None just where that
        /// slice would have panicked.
        #[test]
        fn as_slicing(b in proptest::collection::vec(proptest::num::u8::ANY, 0..24), at in 0usize..30) {
            let old = |n: usize| (at + n <= b.len()).then(|| b[at..at + n].to_vec());
            proptest::prop_assert_eq!(u16_at(&b, at).map(|v| v.to_le_bytes().to_vec()), old(2));
            proptest::prop_assert_eq!(u32_at(&b, at).map(|v| v.to_le_bytes().to_vec()), old(4));
            proptest::prop_assert_eq!(u64_at(&b, at).map(|v| v.to_le_bytes().to_vec()), old(8));
            proptest::prop_assert_eq!(u16_be_at(&b, at).map(|v| v.to_be_bytes().to_vec()), old(2));
            proptest::prop_assert_eq!(u32_be_at(&b, at).map(|v| v.to_be_bytes().to_vec()), old(4));
            proptest::prop_assert_eq!(u64_be_at(&b, at).map(|v| v.to_be_bytes().to_vec()), old(8));
            let p = pieces::<4>(&b);
            proptest::prop_assert_eq!(p.len(), b.len() / 4);
            proptest::prop_assert!(p.iter().zip(b.chunks_exact(4)).all(|(a, c)| a[..] == *c));
        }

        /// The units and texts as the hand-written readers made them (chunks_exact(2) and
        /// from_le_bytes into a Vec<u16>, then String::from_utf16_lossy), odd lengths and NULs too.
        #[test]
        fn utf16_as_by_hand(b in proptest::collection::vec(proptest::num::u8::ANY, 0..40)) {
            let old: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            proptest::prop_assert_eq!(units16(&b).collect::<Vec<_>>(), old.clone());
            proptest::prop_assert_eq!(utf16_lossy(&b), String::from_utf16_lossy(&old));
            let end = old.iter().position(|&c| c == 0).unwrap_or(old.len());
            proptest::prop_assert_eq!(utf16z_lossy(&b), String::from_utf16_lossy(&old[..end]));
        }

        #[test]
        fn arrays(b in proptest::array::uniform16(proptest::num::u8::ANY)) {
            proptest::prop_assert_eq!(u32::<12, 16>(&b), u32::from_le_bytes([b[12], b[13], b[14], b[15]]));
            proptest::prop_assert_eq!(u32::<0, 16>(&b), u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            proptest::prop_assert_eq!(arr::<3, 5, 16>(&b), [b[3], b[4], b[5], b[6], b[7]]);
        }
    }
}
