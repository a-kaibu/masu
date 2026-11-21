//! SetSearchPathMode's flags (RtlSetSearchPathMode, sake-re/kernelbase/search-path.md) over
//! every combination of the bits it looks at and two it refuses, and the permanent lock.
use super::*;

#[test]
fn every_flag_combination() {
    let bits = [1u32, 0x8000, 0x1_0000, 2, 0x80];
    for m in 0u32..32 {
        let f: u32 = bits.iter().enumerate().filter(|(i, _)| m >> i & 1 != 0).map(|(_, b)| b).sum();
        SEARCH_PATH_MODE.store(0, Ordering::Relaxed);
        let ok = matches!(f, 1 | 0x8001 | 0x1_0000);
        assert_eq!(set_search_path_mode(f) == 0, ok, "{f:#x}");
    }
    // once permanent, only another permanent ENABLE goes
    SEARCH_PATH_MODE.store(0, Ordering::Relaxed);
    assert_eq!(set_search_path_mode(0x8001), 0);
    assert_eq!(set_search_path_mode(0x1_0000), 0xc000_0022);
    assert_eq!(set_search_path_mode(1), 0xc000_0022);
    assert_eq!(set_search_path_mode(0x8001), 0);
    SEARCH_PATH_MODE.store(0, Ordering::Relaxed);
}
