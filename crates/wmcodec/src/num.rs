//! Named integer conversions for the decoders' hot loops: each says what it does (widen,
//! reinterpret, truncate) where a bare `as` would not, and costs what `as` costs.

const _: () = assert!(usize::BITS >= 32);

/// u32 to usize (lossless: usize has at least 32 bits).
#[inline(always)]
pub const fn us(x: u32) -> usize {
    x as usize
}

/// i32 to usize for indexing: two's complement, so a negative index is huge and fails the
/// bounds check as `as usize` did.
#[inline(always)]
pub const fn ix(x: i32) -> usize {
    us(x.cast_unsigned())
}

/// The low 8 bits (wrapping).
#[inline(always)]
pub const fn lo8(x: u32) -> u8 {
    x.to_le_bytes()[0]
}

/// The low 16 bits of an i32, as i16 (wrapping, what `as i16` did).
#[inline(always)]
pub const fn lo16(x: i32) -> i16 {
    let [a, b, _, _] = x.to_le_bytes();
    i16::from_le_bytes([a, b])
}

/// The low 32 bits of a u64.
#[inline(always)]
pub const fn lo32(x: u64) -> u32 {
    let [a, b, c, d, _, _, _, _] = x.to_le_bytes();
    u32::from_le_bytes([a, b, c, d])
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// each is the `as` it replaces
        #[test]
        fn same_as_as(x in any::<i32>(), u in any::<u64>()) {
            prop_assert_eq!(ix(x), x as u32 as usize);
            prop_assert_eq!(lo8(x.cast_unsigned()), x as u8);
            prop_assert_eq!(lo16(x), x as i16);
            prop_assert_eq!(lo32(u), u as u32);
        }
    }
}
