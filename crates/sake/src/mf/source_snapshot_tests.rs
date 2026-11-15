//! Golden sweep of what the Media Foundation sources read out of a file (mf/source.rs,
//! mf/mp4.rs, and audiofile / asf under them) before their typed rewrite: the test files and
//! seeded damage to them (bytes changed, cut short, runs zeroed), each one's tracks, types and
//! the H.264 samples as Annex B. A panic would be a line of its own (none may happen).

use super::*;
use crate::test_guest::{snapshot, Rng};
use std::fmt::Write;

fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3))
}

fn store_line(s: &Store) -> String {
    s.items
        .iter()
        .map(|(k, v)| {
            let v = match v {
                Val::Blob(b) => format!("Blob({}:{:x})", b.len(), fnv(b)),
                Val::U32(_) | Val::U64(_) | Val::F64(_) | Val::Guid(_) | Val::Str(_) | Val::Unk(_) => format!("{v:?}"),
            };
            format!("{:02x?}={v}", &k[..4])
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the sources make of `bytes`, as lines.
fn read_out(bytes: Vec<u8>) -> String {
    let d = Data::mem_arc(Arc::new(bytes.clone()));
    let Some(m) = open_media(None, &d) else { return "none".into() };
    let mut s = format!("{} dur={:?} packed={}", m.mime, m.duration, m.packed.as_ref().map_or(0, |p| p.len));
    for t in &m.tracks {
        match t {
            mp4::Track::Video(v) => {
                let samples: Vec<_> = v.samples.iter().map(|x| (x.off, x.size, x.pts, x.dts, x.dur, x.key, x.order)).collect();
                write!(s, "\n  video {} {}x{} p{} l{} nal{} head={}:{:x} rate={:?} br={} wm={:?} samples={}:{:x}", v.codec, v.w, v.h, v.profile, v.level, v.nal, v.head.len(), fnv(&v.head), v.rate, v.bitrate, v.wm.is_some(), samples.len(), fnv(format!("{samples:?}").as_bytes())).unwrap();
                write!(s, "\n    native {}", store_line(&video_native(v))).unwrap();
                for sub in DECODED {
                    if let Some(t) = video_decoded(v, sub) {
                        write!(s, "\n    {sub:08x} {}", store_line(&t)).unwrap();
                    }
                }
                // (an ASF track's samples go out as they are, vstream.rs)
                for x in v.samples.iter().take(if v.wm.is_some() { 0 } else { 6 }) {
                    let raw = usize::try_from(x.off).ok().and_then(|o| bytes.get(o..o + usize::try_from(x.size).unwrap_or(0)));
                    let ab = raw.and_then(|r| mp4::annexb(v, x.key, r));
                    write!(s, "\n    annexb {} {:?}", x.off, ab.map(|a| (a.len(), fnv(&a)))).unwrap();
                }
            }
            mp4::Track::Audio(_) => {}
        }
        write!(s, "\n  type {}", store_line(&native_of(t))).unwrap();
    }
    s
}

#[test]
fn mf_source_snapshot() {
    let mut out = String::new();
    let mut r = Rng(0x5005_ce00);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/mf/testdata");
    for name in ["bframes.mp4", "colors.wmv", "tone.flac", "tone.ogg", "tone.wma"] {
        let base = std::fs::read(dir.join(name)).unwrap();
        writeln!(out, "== {name} {} bytes\n{}", base.len(), read_out(base.clone())).unwrap();
        for k in 0..60 {
            let mut b = base.clone();
            let how = match r.below(4) {
                0 => {
                    let n = 1 + usize::try_from(r.below(8)).unwrap_or(0);
                    for _ in 0..n {
                        let at = usize::try_from(r.below(u64::try_from(b.len()).unwrap_or(1))).unwrap_or(0);
                        b[at] = r.bytes(1)[0];
                    }
                    format!("{n} bytes changed")
                }
                1 => {
                    let at = usize::try_from(r.below(u64::try_from(b.len()).unwrap_or(1))).unwrap_or(0);
                    b.truncate(at);
                    format!("cut at {at}")
                }
                2 => {
                    let at = usize::try_from(r.below(u64::try_from(b.len()).unwrap_or(1))).unwrap_or(0);
                    let n = usize::try_from(r.below(64)).unwrap_or(0).min(b.len() - at);
                    b[at..at + n].fill(0);
                    format!("{n} zeroed at {at}")
                }
                _ => {
                    // a header word made huge (a size or a count)
                    let at = usize::try_from(r.below(u64::try_from(b.len().min(512)).unwrap_or(1))).unwrap_or(0) & !3;
                    if at + 4 <= b.len() {
                        b[at..at + 4].copy_from_slice(&r.pick(&[0xffff_ffffu32, 0x7fff_ffff, 0x10000]).to_be_bytes());
                    }
                    format!("word at {at}")
                }
            };
            let t0 = std::time::Instant::now();
            let got = std::panic::catch_unwind(|| read_out(b)).unwrap_or_else(|_| "PANIC".into());
            // (a damaged count must not make a source loop: bframes.mp4's ctts took 18 s)
            assert!(t0.elapsed() < std::time::Duration::from_secs(2), "{name} #{k} {how}: {:?}", t0.elapsed());
            writeln!(out, "-- {name} #{k} {how}: {got}").unwrap();
        }
    }
    assert!(!out.contains("PANIC"), "a source panicked on a damaged file: see the golden");
    snapshot("mf_source", &out);
}
