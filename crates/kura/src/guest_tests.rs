//! Each helper answers what the `as` it replaced did, for any value.
use super::*;
use proptest::prelude::*;

proptest! {
    #[test]
    fn each_is_its_as(v: u32, w: usize, x: u64) {
        prop_assert_eq!(us(v), v as usize);
        prop_assert_eq!(gp::<u8>(v), v as usize as *mut u8);
        prop_assert_eq!(gc::<u32>(v), v as usize as *const u32);
        prop_assert_eq!(lo32(w), w as u32);
        prop_assert_eq!(lo32w(x), x as u32);
    }
}

#[test]
fn the_edges() {
    for v in [0, 1, 0x7fff_ffff, 0x8000_0000, u32::MAX] {
        assert_eq!(us(v), v as usize);
    }
    for w in [0, usize::MAX, 0x1_0000_0001usize, 0xffff_ffff] {
        assert_eq!(lo32(w), w as u32);
    }
}
