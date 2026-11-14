//! Named integer conversions for the decoders' hot loops: each says what it does (widen,
//! reinterpret, truncate) where a bare `as` would not, and costs what `as` costs.

const _: () = assert!(usize::BITS >= 32);

/// u32 to usize (lossless: usize has at least 32 bits).
#[inline(always)]
pub fn us(x: u32) -> usize {
    usize::try_from(x).unwrap_or(usize::MAX)
}

/// i32 to usize for indexing: two's complement, so a negative index is huge and fails the
/// bounds check as `as usize` did.
#[inline(always)]
pub fn ix(x: i32) -> usize {
    us(x.cast_unsigned())
}

/// A picture dimension or count (up to a few thousand) as i32: its low 31 bits, which is all
/// of any caller's value (and one AND, where a checked conversion costs a compare per call in
/// the motion compensation).
#[inline(always)]
pub fn i32z(n: usize) -> i32 {
    i32::try_from(n & 0x7fff_ffff).unwrap_or(0)
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

/// The low 16 bits of a u32.
#[inline(always)]
pub const fn lo16u(x: u32) -> u16 {
    let [a, b, _, _] = x.to_le_bytes();
    u16::from_le_bytes([a, b])
}

/// The low 8 bits of a usize.
#[inline(always)]
pub const fn lo8z(x: usize) -> u8 {
    x.to_le_bytes()[0]
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
            prop_assert_eq!(ix(x), usize::try_from(x.cast_unsigned()).unwrap());
            prop_assert_eq!(lo8(x.cast_unsigned()), u8::try_from(x & 0xff).unwrap());
            prop_assert_eq!(lo16(x), i16::try_from(((x & 0xffff) ^ 0x8000) - 0x8000).unwrap());
            prop_assert_eq!(lo32(u), u32::try_from(u & 0xffff_ffff).unwrap());
            let n = usize::try_from(u & 0x7fff_ffff).unwrap();
            prop_assert_eq!(i64::from(i32z(n)), i64::try_from(n).unwrap());
            prop_assert_eq!(lo16u(lo32(u)), u16::try_from(u & 0xffff).unwrap());
            prop_assert_eq!(lo8z(usize::try_from(u & 0xffff_ffff).unwrap()), u8::try_from(u & 0xff).unwrap());
        }
    }
}
