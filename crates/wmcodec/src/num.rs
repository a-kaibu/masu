//! The one conversion the decoders need that kome::num has not: i32z (the others are kome's).

/// A picture dimension or count (up to a few thousand) as i32: its low 31 bits, which is all
/// of any caller's value (and one AND, where a checked conversion costs a compare per call in
/// the motion compensation).
#[inline(always)]
pub fn i32z(n: usize) -> i32 {
    i32::try_from(n & 0x7fff_ffff).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// the value, for every caller's size (below 2^31)
        #[test]
        fn i32z_is_the_value(n in 0usize..0x8000_0000) {
            prop_assert_eq!(i64::from(i32z(n)), i64::try_from(n).unwrap());
        }
    }
}
