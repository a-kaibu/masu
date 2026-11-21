//! kura::video's page calls on any `.vnv` (#241): `vnhost_video_chunk` writes its two words and
//! no more, and the copy it hands the page is the frame's bytes in the file (none for a frame
//! past the end or a table pointing out of the file); `vnhost_video_list` writes at most `max`
//! rows. The answers go to heap blocks of exactly their size, so AddressSanitizer (cargo fuzz's
//! default) stops a write one word past them.
#![no_main]

use kura::{video, vnv};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&k, rest)) = data.split_first() else { return };
    let (file, asks) = if k & 1 == 0 {
        // any bytes as the file (the asks: its last quarter)
        let cut = rest.len() - rest.len() / 4;
        (rest[..cut].to_vec(), &rest[cut..])
    } else {
        // a file written from the input's frame sizes, some of its bytes then changed (frame
        // tables pointing past the end, sizes that wrap)
        let n = rest.first().map_or(0, |&b| b as usize % 24);
        let Some(sizes) = rest.get(1..1 + n) else { return };
        let frames: Vec<(Vec<u8>, bool)> = sizes.iter().enumerate().map(|(j, &s)| (vec![j as u8; s as usize * 3], j == 0 || s & 7 == 0)).collect();
        let mut tracks = vec![("vp09.00.31.08".to_string(), frames.clone())];
        if k & 2 != 0 {
            tracks.push(("vp09.00.31.08".to_string(), frames));
        }
        let Ok(mut f) = vnv::write(16, 16, &tracks, None) else { return };
        let mut at = 1 + n;
        while let Some(p) = rest.get(at..at + 3) {
            at += 3;
            if p[0] == 0 {
                break;
            }
            let o = u16::from_le_bytes([p[0], p[1]]) as usize;
            if !f.is_empty() {
                let l = f.len();
                f[o % l] ^= p[2];
            }
        }
        if k & 4 != 0 {
            f.truncate(f.len().saturating_sub(rest.len() % 64));
        }
        (f, &rest[at.min(rest.len())..])
    };
    let Some(id) = video::open(file.clone()) else { return };
    let info = vnv::info(&file).expect("an opened stream's file reads");
    for a in asks.chunks(3) {
        let (t, n) = (a[0] as u32 % 4, a.get(1..).map_or(0, |b| b.iter().fold(0u32, |x, &y| x << 8 | y as u32)) % 300);
        let mut size = Box::new([0x5a5a_5a5au32; 2]);
        let p = unsafe { video::vnhost_video_chunk(id, t, n, &mut *size) };
        let frame = info.tracks.get(t as usize).and_then(|tr| tr.1.get(n as usize)).copied();
        let bytes = frame.and_then(|(off, sz, _)| file.get(off as usize..(off as usize).checked_add(sz as usize)?));
        let mut c = Vec::new();
        match bytes {
            Some(b) => {
                assert_ne!(p, 0);
                assert_eq!(size[0] as usize, b.len());
                assert!(video::chunk_into(id, t, n, &mut c).is_some());
                assert_eq!(&c[..], b);
            }
            None => {
                assert_eq!(p, 0, "track {t} frame {n}: not in the file");
                assert_eq!(*size, [0x5a5a_5a5a; 2], "nothing written");
                assert_eq!(video::chunk_into(id, t, n, &mut c), None);
            }
        }
    }
    let max = k as u32 >> 4;
    let mut out = vec![0u32; 6 * max as usize];
    let listed = unsafe { video::vnhost_video_list(out.as_mut_ptr(), max) };
    assert!(listed <= max);
    video::close(id);
});

// What the page or emscripten gives kura (as in profile_cache.rs): no store to fetch from, and
// a clock at 0
#[unsafe(no_mangle)]
pub extern "C" fn vnvm_fetch(_name: *const std::ffi::c_char, _len: *mut usize) -> *mut u8 {
    std::ptr::null_mut()
}
#[unsafe(no_mangle)]
pub extern "C" fn vnvm_fetch_hint(_names: *const std::ffi::c_char, _len: usize) {}
#[unsafe(no_mangle)]
pub extern "C" fn emscripten_get_now() -> f64 {
    0.0
}
