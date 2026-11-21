//! Behaviour pins: everything the crate gives (packets, pictures' whole planes, BGRX, samples,
//! probes, frame headers) over testdata/ and seeded damaged copies of it (bits flipped, chunks
//! cut, dropped, swapped), hashed and compared with testdata/pin.tsv, so a refactor that changes
//! one byte of output fails here. `MASU_UPDATE_GOLDEN=1 cargo test -p mpeg1 --release --test pin`
//! writes the table again (only for a change of behaviour that is meant).

use mpeg1::{audio, ps, Audio, Demux, Video};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kome::golden::{Fnv as H, Rng, damage, pins};

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn frame(h: &mut H, f: &mpeg1::Frame) {
    for v in [f.width, f.height, f.y_stride, f.c_stride] {
        h.usize(v);
    }
    for p in [&f.y, &f.cb, &f.cr] {
        h.usize(p.len());
        h.bytes(p);
    }
    for up in [false, true] {
        let stride = f.width * 4 + 4;
        let mut out = vec![0u8; stride * f.height];
        mpeg1::video::to_bgrx(f, &mut out, stride, up);
        h.bytes(&out);
    }
}

/// Demux (or not, for an elementary stream) and decode `d` fed in chunks of `cut` bytes.
fn decode(h: &mut H, d: &[u8], cut: usize) {
    let (mut v, mut a) = (Video::new(), Audio::new());
    let drain = |h: &mut H, v: &mut Video, a: &mut Audio| {
        while let Some(f) = v.decode() {
            frame(h, f);
        }
        if let Some(x) = v.header() {
            h.usize(x.width);
            h.usize(x.height);
            h.u64(x.rate.to_bits());
        }
        h.u64(v.pictures);
        while a.decode() {
            h.usize(a.out.len());
            a.out.iter().for_each(|s| { h.bytes(&s.to_bits().to_le_bytes()); });
        }
        if let Some(x) = a.header() {
            h.u64(u64::from(x.rate));
            h.usize(x.size);
        }
        h.u64(a.frames);
    };
    if ps::is_system(d) {
        let mut m = Demux::new();
        for c in d.chunks(cut.max(1)) {
            m.push(c);
            while let Some(p) = m.next() {
                h.bytes(&[p.id]);
                h.u64(p.pts.map_or(u64::MAX, f64::to_bits));
                h.usize(p.data.len());
                h.bytes(&p.data);
                if ps::is_video(p.id) {
                    v.push(&p.data);
                } else if ps::is_audio(p.id) {
                    a.push(&p.data);
                }
            }
            drain(h, &mut v, &mut a);
        }
        m.end();
        while let Some(p) = m.next() {
            h.bytes(&[p.id]);
            h.bytes(&p.data);
        }
    } else {
        let video = ps::is_video_stream(d);
        for c in d.chunks(cut.max(1)) {
            if video {
                v.push(c);
            } else {
                a.push(c);
            }
            drain(h, &mut v, &mut a);
        }
    }
    v.end();
    a.end();
    drain(h, &mut v, &mut a);
}

fn info(h: &mut H, i: Option<mpeg1::Info>) {
    match i {
        None => { h.bytes(b"none"); }
        Some(i) => {
            h.usize(i.width);
            h.usize(i.height);
            h.u64(i.rate.to_bits());
            h.u64(u64::from(i.audio_rate));
            h.u64(i.duration.to_bits());
        }
    }
}

/// Everything of one input: decoded whole and in pieces, probed, frame headers at each offset.
fn all(h: &mut H, d: &[u8]) {
    for cut in [usize::MAX, 2048, 333] {
        decode(h, d, cut);
    }
    info(h, mpeg1::probe(d, d, true));
    info(h, mpeg1::probe(&d[..d.len().min(4096)], &d[d.len().saturating_sub(4096)..], false));
    let len = u64::try_from(d.len()).unwrap();
    info(h, mpeg1::probe_stream(len, 1024, |pos, n| {
        let pos = usize::try_from(pos).unwrap();
        d.get(pos..pos + n).map(<[u8]>::to_vec)
    }));
    for i in 0..d.len().min(20000) {
        if let Some(x) = audio::header(&d[i..]) {
            // (its private fields show in the samples)
            h.usize(i);
            h.u64(u64::from(x.rate));
            h.usize(x.size);
        }
    }
}

#[test]
fn pin() {
    let mut rows = BTreeMap::new();
    for name in ["testsrc.mpg", "mono32k.mp2"] {
        let d = std::fs::read(testdata().join(name)).unwrap();
        let mut h = H::new();
        all(&mut h, &d);
        rows.insert(name.to_string(), h.0);
        let chunks: Vec<Vec<u8>> = d.chunks(2048).map(<[u8]>::to_vec).collect();
        let mut h = H::new();
        for s in 0..32 {
            all(&mut h, &damage(&chunks, s).concat());
        }
        rows.insert(format!("{name} damaged x32"), h.0);
    }
    // any bytes after each kind of start code
    let mut h = H::new();
    let mut r = Rng(5);
    for i in 0..150 {
        let mut d = [&[0u8, 0, 1, 0xba][..], &[0, 0, 1, 0xb3], &[0xff, 0xfd]][i % 3].to_vec();
        d.extend((0..r.index(4000)).map(|_| r.next().to_le_bytes()[0]));
        all(&mut h, &d);
    }
    rows.insert("any bytes".into(), h.0);
    pins(&testdata().join("pin.tsv"), &rows);
}
