//! dshow_play pinned before its code changes shape: a seeded sweep of the clock (every
//! Timeline call at seeded times, rates, seeks and stops), frame_at at edge rates, letterbox,
//! clip and blit over seeded rectangles, and the Vorbis sound taken in seeded pieces with its
//! restarts, each a line of golden/dshow_play.txt.
use super::*;
use crate::test_guest::{snapshot, Rng};
use std::fmt::Write;

fn fnv(v: impl IntoIterator<Item = u32>) -> u64 {
    v.into_iter().fold(0xcbf2_9ce4_8422_2325u64, |h, x| x.to_le_bytes().iter().fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)))
}

#[test]
fn dshow_play_snapshot() {
    let mut out = String::new();
    let mut r = Rng(0xd5_0901);
    // the clock
    for run in 0..40 {
        let d = r.pick(&[0u64, 1, SEC, 10 * SEC, 3_600 * SEC, u64::MAX / 4]);
        let mut tl = Timeline::new(d);
        let mut t = r.below(5 * SEC);
        writeln!(out, "timeline {run} duration={d}").unwrap();
        for _ in 0..40 {
            let k = r.below(10);
            let v = r.pick(&[0u64, 1, SEC / 3, SEC, 7 * SEC, 11 * SEC, d, d.saturating_add(1), u64::MAX]);
            let rate = r.pick(&[1.0f64, 0.0, -1.0, 0.5, 2.0, 1e-9, 1e9, f64::NAN, f64::INFINITY]);
            let what = match k {
                0 => { tl.run(t); "run".to_string() }
                1 => { tl.pause(t); "pause".into() }
                2 => { tl.stop(t); "stop".into() }
                3 => { tl.seek(t, v); format!("seek {v}") }
                4 => { tl.set_stop(t, v); format!("set_stop {v}") }
                5 => { tl.set_rate(t, rate); format!("set_rate {rate}") }
                6 => format!("complete {}", tl.complete(t)),
                7 => format!("until_end {:?}", tl.until_end(t)),
                _ => { t = t.saturating_add(r.pick(&[0u64, 1, SEC / 10, SEC, 5 * SEC, u64::MAX / 2])); "wait".into() }
            };
            writeln!(out, "  t={t} {what}: pos={} running={} done={} stop={} rate={}", tl.position(t), tl.running(), tl.done, tl.stop, tl.rate).unwrap();
        }
    }
    // the frame shown
    for _ in 0..300 {
        let any = r.below(100 * SEC);
        let pos = r.pick(&[0u64, 1, SEC - 1, SEC, any, u64::MAX]);
        let rate = (r.pick(&[0u32, 1, 24, 30_000, 60, u32::MAX]), r.pick(&[0u32, 1, 1001, u32::MAX]));
        let frames = r.pick(&[0u32, 1, 2, 100, 10_000, u32::MAX]);
        writeln!(out, "frame_at {pos} {rate:?} {frames}: {}", frame_at(pos, rate, frames)).unwrap();
    }
    // the picture
    let n = |r: &mut Rng| r.pick(&[-5i32, 0, 1, 3, 7, 16, 33, 640, 480, i32::MAX / 4]);
    for _ in 0..300 {
        let dst = (n(&mut r), n(&mut r), n(&mut r), n(&mut r));
        let (w, h) = (n(&mut r), n(&mut r));
        let b = (n(&mut r), n(&mut r), n(&mut r), n(&mut r));
        writeln!(out, "letterbox {dst:?} {w}x{h}: {:?} clip {b:?}: {:?}", letterbox(dst, w, h), clip(dst, b)).unwrap();
    }
    let s = |r: &mut Rng| r.pick(&[-3i32, 0, 1, 2, 5, 8, 12, 20]);
    for i in 0..300 {
        let (fw, fh) = (i32::try_from(1 + r.below(9)).unwrap_or(1), i32::try_from(1 + r.below(9)).unwrap_or(1));
        let frame: Vec<u32> = (0..fw * fh).map(|_| u32::try_from(r.next() & 0x00ff_ffff).unwrap_or(0)).collect();
        let src = (s(&mut r), s(&mut r), s(&mut r), s(&mut r));
        let view = (s(&mut r).max(0), s(&mut r).max(0), 1 + s(&mut r).max(0), 1 + s(&mut r).max(0));
        let dst = (s(&mut r), s(&mut r), s(&mut r), s(&mut r));
        let mut px = vec![0x1234_5678u32; usize::try_from(view.2 * view.3).unwrap_or(0)];
        blit(&frame, fw, fh, src, &mut px, view, dst);
        writeln!(out, "blit {i} {fw}x{fh} src={src:?} view={view:?} dst={dst:?}: {:016x}", fnv(px.iter().copied())).unwrap();
    }
    // the sound
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vorbis/testdata/");
    for name in ["stereo-q5", "mono-q0", "stereo-48k"] {
        let Ok(ogg) = std::fs::read(format!("{dir}{name}.ogg")) else { panic!("{name}") };
        let Some(mut v) = Vorbis::new(&ogg) else { panic!("{name}") };
        v.every = r.pick(&[0u64, 300, 2000]);
        v.step = r.pick(&[1usize, 97, 512, 4096]);
        v.restart();
        writeln!(out, "vorbis {name} rate={} every={} step={}", v.rate, v.every, v.step).unwrap();
        for _ in 0..40 {
            let (a, b) = (r.below(20_000), r.below(200_000));
            let from = r.pick(&[0u64, 1, a, b, u64::from(v.rate)]);
            let n = usize::try_from(r.below(5000)).unwrap_or(0);
            let mut got = Vec::new();
            v.take(from, n, &mut got);
            writeln!(out, "  take {from} {n}: {} {:016x} base={} restarts={} marks={}", got.len(), fnv(got.iter().map(|f| f.to_bits())), v.base, v.restarts, v.marks.len()).unwrap();
        }
    }
    let bad = |b: &[u8]| Vorbis::new(b).is_none();
    let junk = r.bytes(300);
    writeln!(out, "vorbis empty: {} junk: {}", bad(&[]), bad(&junk)).unwrap();
    snapshot("dshow_play", &out);
}
