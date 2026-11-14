//! The page's host calls (`vnhost_*`) write their answers only into the words they say they
//! write (#241: `vnhost_video_chunk` wrote a second word past a caller's one-word buffer, which
//! AddressSanitizer saw as a stack-buffer-overflow). Each call here gets its words between
//! canaries, in a heap block of their own: the canaries catch a write past them in a plain
//! `cargo test` (and so under cargo-mutants), ASan (tools/ci/sanitize.sh) a write further out.
//!
//! The fixed-size answers are typed as arrays (`*mut [u32; 2]` ...), so a Rust caller with too
//! small a buffer does not compile; these tests hold the functions to those sizes, and the
//! listing calls (`out`, `max`) to `max` rows, for any `max` and any number of rows.

use crate::{adec, idec, render, video, vnv};
use proptest::prelude::*;
use std::sync::Mutex;

/// The host's state (streams, the display) is the process's: one test at a time.
static ONE: Mutex<()> = Mutex::new(());

fn one() -> std::sync::MutexGuard<'static, ()> {
    ONE.lock().unwrap_or_else(|e| e.into_inner())
}

const GUARD: usize = 16;

/// `words` words filled with `fill` between GUARD canary words on each side; `f` gets the first
/// of the words. Returns the words after `f`; panics when a canary changed.
pub(crate) fn guarded<T: Copy + PartialEq + std::fmt::Debug>(words: usize, fill: T, canary: T, f: impl FnOnce(*mut T)) -> Vec<T> {
    let mut b = vec![canary; words + 2 * GUARD].into_boxed_slice();
    b[GUARD..GUARD + words].fill(fill);
    f(unsafe { b.as_mut_ptr().add(GUARD) });
    for (k, &w) in b.iter().enumerate() {
        if !(GUARD..GUARD + words).contains(&k) {
            assert_eq!(w, canary, "word {} of a buffer of {words} written", k as isize - GUARD as isize);
        }
    }
    b[GUARD..GUARD + words].to_vec()
}

/// [`guarded`] for a call that takes `N` words as an array.
pub(crate) fn guarded_n<T: Copy + PartialEq + std::fmt::Debug, const N: usize>(fill: T, canary: T, f: impl FnOnce(*mut [T; N])) -> [T; N] {
    guarded(N, fill, canary, |p| f(p.cast())).try_into().unwrap()
}

pub(crate) const C32: u32 = 0xdead_beef;

/// A `.vnv` of one or two tracks of `frames` (sizes and key flags).
fn vnv_of(frames: &[(usize, bool)], two: bool) -> Vec<u8> {
    let track = |seed: u8| -> Vec<(Vec<u8>, bool)> {
        frames.iter().enumerate().map(|(k, &(n, key))| ((0..n).map(|j| (k * 13 + j) as u8 ^ seed).collect(), key || k == 0)).collect()
    };
    let mut t = vec![("vp09.00.31.08".to_string(), track(0))];
    if two {
        t.push(("vp09.00.31.08".to_string(), track(0x3c)));
    }
    vnv::write(16, 16, &t, None)
}

#[test]
fn video_chunk_writes_two_words() {
    let _o = one();
    let frames = [(5, true), (0, false), (1, false), (300, true)];
    let d = vnv_of(&frames, true);
    let info = vnv::info(&d).unwrap();
    let id = video::open(d.clone()).unwrap();
    for t in 0..3u32 {
        for n in [0u32, 1, 2, 3, 4, 5, u32::MAX] {
            let mut ret = 0;
            let w = guarded_n::<u32, 2>(C32 ^ 1, C32, |p| ret = unsafe { video::vnhost_video_chunk(id, t, n, p) });
            let frame = info.tracks.get(t as usize).and_then(|tr| tr.1.get(n as usize));
            match frame {
                Some(&(off, sz, _)) => {
                    assert_ne!(ret, 0, "track {t} frame {n}");
                    assert_eq!(w, [sz, n], "track {t} frame {n}: size and presentation place");
                    let mut c = vec![9u8; 3];
                    assert_eq!(video::chunk_into(id, t, n, &mut c), Some(n));
                    assert_eq!(&c[..], &d[off as usize..(off + sz) as usize]);
                }
                None => {
                    assert_eq!(ret, 0, "track {t} frame {n}: none");
                    assert_eq!(w, [C32 ^ 1; 2], "track {t} frame {n}: nothing written");
                    let mut c = vec![9u8; 3];
                    assert_eq!(video::chunk_into(id, t, n, &mut c), None);
                    assert_eq!(c, [9; 3], "the copy stays as it was");
                }
            }
        }
    }
    video::close(id);
    let w = guarded_n::<u32, 2>(7, C32, |p| assert_eq!(unsafe { video::vnhost_video_chunk(id, 0, 0, p) }, 0, "closed"));
    assert_eq!(w, [7, 7]);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any stream, any track and frame asked for (in range or not, a closed or unknown id): two
    /// words or none, the bytes the file has for that frame.
    #[test]
    fn video_chunk_in_bounds(
        frames in prop::collection::vec((0usize..400, prop::bool::weighted(0.2)), 1..40),
        two: bool,
        asks in prop::collection::vec((0u32..3, 0u32..48, 0u32..3), 1..40),
    ) {
        let _o = one();
        let d = vnv_of(&frames, two);
        let info = vnv::info(&d).unwrap();
        let id = video::open(d.clone()).unwrap();
        for &(t, n, which) in &asks {
            let ask = match which { 0 => id, 1 => id.wrapping_add(1000), _ => u32::MAX };
            let mut ret = 0;
            let w = guarded_n::<u32, 2>(C32 ^ 2, C32, |p| ret = unsafe { video::vnhost_video_chunk(ask, t, n, p) });
            let frame = (ask == id).then(|| info.tracks.get(t as usize).and_then(|tr| tr.1.get(n as usize))).flatten();
            match frame {
                Some(&(_, sz, _)) => {
                    prop_assert!(ret != 0);
                    prop_assert_eq!(w, [sz, n]);
                }
                None => {
                    prop_assert_eq!(ret, 0);
                    prop_assert_eq!(w, [C32 ^ 2; 2]);
                }
            }
        }
        video::close(id);
    }

    /// The page's listing calls write at most `max` rows of their row's words, and say how many.
    /// (vnhost_video_list's: video::tests::a_fresh_stream_holds_nothing, the one test that may
    /// list streams.)
    #[test]
    fn lists_hold_to_max(streams in 0usize..7, max in 0u32..9) {
        {
            let _g = idec::tests::ONE.lock().unwrap_or_else(|e| e.into_inner());
            let mut n = 0;
            let rows = guarded(3 * 16, C32 ^ 5, C32, |p| n = unsafe { idec::vnhost_idec_list(p, max) });
            prop_assert!(n <= max);
            prop_assert!(rows[3 * n as usize..].iter().all(|&w| w == C32 ^ 5));
            guarded(3 * max as usize, 0, C32, |p| unsafe { idec::vnhost_idec_list(p, max); });
        }
        let _g = adec::tests::page_on();
        let a: Vec<u32> = (0..streams).filter_map(|k| adec::open("opus", 48000, 1 + k as u32 % 2, &[1, 2, 3])).collect();
        prop_assert_eq!(a.len(), streams);
        let mut n = 0;
        let rows = guarded(10 * 16, C32 ^ 4, C32, |p| n = unsafe { adec::vnhost_adec_list(p, max) });
        prop_assert!(n <= max);
        prop_assert!(n as usize >= streams.min(max as usize), "{} of {} listed, max {}", n, streams, max);
        prop_assert!(rows[10 * n as usize..].iter().all(|&w| w == C32 ^ 4));
        guarded(10 * max as usize, 0, C32, |p| unsafe { adec::vnhost_adec_list(p, max); });
        for id in a {
            adec::close(id);
        }
    }
}

/// The fixed-size answers: exactly their words. (The display's own: display::tests::host_calls,
/// the one test on the global display; vnhost_d3d9_poll would take another test's batch.)
#[test]
fn fixed_answers_write_their_words() {
    let _o = one();
    unsafe {
        guarded_n::<i32, 10>(0, i32::MIN, |p| render::vnhost_render_layout(p));
        guarded_n::<u32, 3>(0, C32, |p| {
            render::vnhost_render_shot(p);
        });
        guarded_n::<u32, 8>(0, C32, |p| render::vnhost_render_info(p));
        guarded_n::<f64, 6>(0.0, -1.5, |p| render::gpu::vnhost_gpu_pipelines(p));
        guarded_n::<f64, 12>(0.0, -1.5, |p| crate::vfs::vnhost_net_stats(p));
    }
}
