//! The system DLL files' images read back by sake's own PE reader
use super::*;

#[test]
fn the_image_is_a_dll_of_the_machine() {
    for x64 in [false, true] {
        let f = image(x64);
        assert_eq!(f.get(..2), Some(&b"MZ"[..]));
        assert_eq!(f.get(0x44..0x46), Some(&(if x64 { [0x64, 0x86] } else { [0x4c, 0x01] })[..]));
        let l = crate::imageload::layout(&f).map(|l| (l.count, l.characteristics & 0x2000, l.size_of_image));
        assert_eq!(l, Some((1, 0x2000, 0x2000)), "x64 {x64}");
    }
}
