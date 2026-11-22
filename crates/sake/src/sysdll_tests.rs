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

proptest::proptest! {
    /// a system DLL's file holds its version block where Windows' do (RT_VERSION 1), read back by
    /// sake's PE resource reader from the file as data and as the reader that seeks; the image
    /// stays one section of one page (Wine version:info copies iphlpapi.dll and reads its
    /// OriginalFilename)
    #[test]
    fn the_file_carries_its_version_block(name in "[a-z][a-z0-9]{0,11}\\.dll", x64 in proptest::bool::ANY) {
        let f = image_of(x64, &name);
        let want = crate::misc::system_version_info(&name);
        proptest::prop_assert_eq!(crate::pe::file_resource(&f, 16), Some(&want[..]));
        let mut read = |at: u64, n: usize| f.get(usize::try_from(at).ok()?..).map(|r| r[..n.min(r.len())].to_vec());
        proptest::prop_assert_eq!(crate::pe::file_resource_read(&mut read, 16), Some(want.clone()));
        let l = crate::imageload::layout(&f).map(|l| (l.count, l.characteristics & 0x2000, l.size_of_image));
        proptest::prop_assert_eq!(l, Some((1, 0x2000, 0x2000)));
        let name16: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        proptest::prop_assert!(want.windows(name16.len()).any(|w| w == &name16[..]));
    }
}
