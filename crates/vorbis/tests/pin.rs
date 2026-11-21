//! Behaviour pins: every sample (f32 bits), packet and error over testdata/ and seeded damaged
//! copies of it (audio packets, setup headers and the Ogg pages themselves: bits flipped,
//! packets cut, dropped, swapped), hashed and compared with testdata/pin.tsv, so a refactor that
//! changes one bit of output fails here. `MASU_UPDATE_GOLDEN=1 cargo test -p vorbis --release --test pin`
//! writes the table again (only for a change of behaviour that is meant).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use vorbis::{ogg, Decoder};

use kome::golden::{Fnv as H, Rng, damage, pins};

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

const NAMES: [&str; 5] = ["stereo-q5", "mono-q0", "surround-q3", "stereo-q10-silence", "stereo-48k"];

fn read(name: &str) -> Vec<u8> {
    std::fs::read(testdata().join(format!("{name}.ogg"))).unwrap()
}

/// The Ogg layer: packets of the whole file and of a Reader fed in pieces (with a small limit too).
fn pages(h: &mut H, d: &[u8]) -> Vec<Vec<u8>> {
    let (p, g) = ogg::packets(d);
    h.u64(g);
    for q in &p {
        h.usize(q.len());
        h.bytes(q);
    }
    for (cut, limit) in [(4096, None), (333, Some(700))] {
        let mut r = limit.map_or_else(ogg::Reader::default, ogg::Reader::with_limit);
        for c in d.chunks(cut) {
            r.push(c);
            while let Some(q) = r.pop() {
                h.usize(q.len());
                h.bytes(&q);
            }
            h.u64(r.granule());
        }
        r.finish();
        while let Some(q) = r.pop() {
            h.bytes(&q);
        }
    }
    p
}

/// Every sample of the audio packets, and the decoder's state carried and resumed now and then.
fn audio(h: &mut H, ident: &[u8], setup: &[u8], pkts: &[Vec<u8>]) {
    let mut d = match Decoder::new(ident, setup) {
        Ok(d) => d,
        Err(e) => { h.bytes(format!("{e:?}").as_bytes()); return; }
    };
    let x = &d.setup.ident;
    h.usize(x.channels);
    h.u64(u64::from(x.rate));
    h.usize(x.blocksize[0]);
    h.usize(x.blocksize[1]);
    let mut out = Vec::new();
    let mut saved = None;
    for (i, q) in pkts.iter().enumerate() {
        if i % 11 == 3 {
            saved = Some(d.carry());
        }
        out.clear();
        d.packet(q, &mut out);
        h.usize(out.len());
        out.iter().for_each(|v| { h.bytes(&v.to_bits().to_le_bytes()); });
        if i % 11 == 9 && let Some(c) = &saved {
            d.resume(c);
        }
    }
}

#[test]
fn pin() {
    let mut rows = BTreeMap::new();
    for name in NAMES {
        let d = read(name);
        let mut h = H::new();
        let p = pages(&mut h, &d);
        audio(&mut h, &p[0], &p[2], &p[3..]);
        rows.insert(name.to_string(), h.0);
        // damaged audio packets
        let mut h = H::new();
        for s in 0..40 {
            audio(&mut h, &p[0], &p[2], &damage(&p[3..], s));
        }
        rows.insert(format!("{name} packets damaged x40"), h.0);
        // damaged setup and ident headers (past their signatures)
        let mut h = H::new();
        let mut r = Rng(3);
        for s in 0..40 {
            let (mut id, mut st) = (p[0].clone(), p[2].clone());
            let which = if s % 4 == 0 { &mut id } else { &mut st };
            let n = which.len();
            for _ in 0..1 + r.index(4) {
                which[7 + r.index(n - 7)] ^= 1 << r.index(8);
            }
            if s % 5 == 0 {
                which.truncate(7 + r.index(n - 7));
            }
            audio(&mut h, &id, &st, &p[3..p.len().min(40)]);
        }
        rows.insert(format!("{name} headers damaged x40"), h.0);
        // the file's pages damaged
        let chunks: Vec<Vec<u8>> = d.chunks(1500).map(<[u8]>::to_vec).collect();
        let mut h = H::new();
        for s in 0..16 {
            let p = pages(&mut h, &damage(&chunks, s).concat());
            if p.len() > 3 {
                audio(&mut h, &p[0], &p[2], &p[3..]);
            }
        }
        rows.insert(format!("{name} pages damaged x16"), h.0);
    }
    // any bytes as audio packets of each stream
    let mut h = H::new();
    let mut r = Rng(9);
    for i in 0..100 {
        let p = ogg::packets(&read(NAMES[i % 5])).0;
        let pkts: Vec<Vec<u8>> = (0..1 + r.index(20)).map(|_| (0..r.index(600)).map(|_| r.next().to_le_bytes()[0]).collect()).collect();
        audio(&mut h, &p[0], &p[2], &pkts);
    }
    rows.insert("any bytes".into(), h.0);
    pins(&testdata().join("pin.tsv"), &rows);
}
