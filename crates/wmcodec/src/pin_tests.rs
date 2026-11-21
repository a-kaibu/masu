//! Behaviour pins: every decoder's output, bit for bit, over a fixed corpus (testdata/: FFmpeg
//! encodes of each codec, the hand-made VC-1 pictures, WMV_DIR's real files when set) and over
//! seeded damaged copies of it (bits flipped, packets cut, dropped, swapped), hashed and compared
//! with testdata/pin.tsv. A refactor that changes one sample, one pixel or one error anywhere
//! fails here; `MASU_UPDATE_GOLDEN=1 cargo test -p wmcodec --release pin` writes the table again (only
//! for a change of behaviour that is meant, say why in the commit).

use crate::msmpeg4::{Decoder, Error, Picture, Version};
use crate::vc1::synth;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kome::golden::{Fnv as H, Rng, damage, pins};

pub fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn picture(h: &mut H, p: &Picture) {
    h.usize(p.width);
    h.usize(p.height);
    h.usize(p.stride[0]);
    h.usize(p.stride[1]);
    h.bytes(&[u8::from(p.key)]);
    for pl in &p.planes {
        h.usize(pl.len());
        h.bytes(pl);
    }
}

fn error(h: &mut H, e: &Error) {
    h.bytes(format!("{e:?}").as_bytes());
}

enum V {
    M(Decoder),
    C(crate::vc1::Decoder),
}

/// Every picture, error and flag of `pkts` through a fresh decoder.
fn video(h: &mut H, ver: Option<Version>, w: usize, hgt: usize, extra: &[u8], pkts: &[Vec<u8>]) {
    let d = match ver {
        Some(v) => Decoder::new(v, w, hgt, extra).map(V::M),
        None => crate::vc1::Decoder::new(w, hgt, extra).map(V::C),
    };
    let mut d = match d {
        Ok(d) => d,
        Err(e) => return error(h, &e),
    };
    for p in pkts {
        let got = match &mut d {
            V::M(d) => d.decode(p),
            V::C(d) => d.decode(p),
        };
        match got {
            Ok(None) => { h.bytes(b"-"); }
            Ok(Some(p)) => picture(h, &p),
            Err(e) => error(h, &e),
        }
        if let V::C(d) = &d {
            h.bytes(&[u8::from(d.ffmpeg_conceals), u8::from(d.ffmpeg_drops), u8::from(d.delayed()), u8::from(d.last_was_b())]);
        }
    }
    if let V::C(d) = &mut d {
        match d.flush() {
            Some(p) => picture(h, &p),
            None => { h.bytes(b"."); }
        }
    }
}

/// Every block's samples (planar f32 bits and the interleaved i16 of a second decoder) and error.
fn audio(h: &mut H, fmt: &[u8], pkts: &[Vec<u8>]) {
    let rd16 = |o: usize| u16::from_le_bytes([fmt[o], fmt[o + 1]]);
    let rd32 = |o: usize| u32::from_le_bytes(fmt[o..o + 4].try_into().unwrap());
    let cb = usize::from(rd16(16));
    let mk = || crate::wma::Decoder::new(rd16(0), rd16(2), rd32(4), rd32(8) * 8, rd16(12), &fmt[18..18 + cb]);
    let (mut a, mut b) = match (mk(), mk()) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => { h.bytes(format!("{e:?}").as_bytes()); return; }
    };
    let align = usize::from(rd16(12));
    for (i, p) in pkts.iter().flat_map(|p| p.chunks(align)).enumerate() {
        match a.decode_raw(p) {
            Ok(planes) => {
                for c in &planes {
                    h.usize(c.len());
                    c.iter().for_each(|v| { h.bytes(&v.to_bits().to_le_bytes()); });
                }
            }
            Err(e) => { h.bytes(format!("{e:?}").as_bytes()); }
        }
        match b.decode(p) {
            Ok(s) => s.iter().for_each(|v| { h.bytes(&v.to_le_bytes()); }),
            Err(e) => { h.bytes(format!("{e:?}").as_bytes()); }
        }
        if i % 7 == 6 {
            // (a seek now and then)
            b.flush();
        }
    }
    b.drain().iter().for_each(|v| { h.bytes(&v.to_le_bytes()); });
}

/// The streams of an ASF file: (format, video?, packets) each.
fn streams(file: &Path) -> Vec<(Vec<u8>, bool, Vec<Vec<u8>>)> {
    let mut r = asf::Reader::new(std::fs::read(file).unwrap()).unwrap();
    let ss = r.header.streams.clone();
    let mut pk: Vec<Vec<Vec<u8>>> = vec![Vec::new(); ss.len()];
    while let Some(x) = r.next_sample() {
        if let Some(i) = ss.iter().position(|s| s.number == x.stream) {
            pk[i].push(x.data);
        }
    }
    ss.iter().zip(pk).filter_map(|(s, p)| match s.kind {
        asf::Kind::Video { .. } => Some((s.format.clone(), true, p)),
        asf::Kind::Audio => Some((s.format.clone(), false, p)),
        asf::Kind::Other(_) => None,
    }).collect()
}

fn version(fourcc: &[u8]) -> Option<Option<Version>> {
    match fourcc {
        b"WMV1" => Some(Some(Version::Wmv1)),
        b"WMV2" => Some(Some(Version::Wmv2)),
        b"MP43" | b"DIV3" => Some(Some(Version::V3)),
        b"WMV3" => Some(None),
        _ => None,
    }
}

/// One stream and its damaged copies, as table rows.
fn stream_rows(rows: &mut BTreeMap<String, u64>, name: &str, fmt: &[u8], video_stream: bool, pkts: &[Vec<u8>], copies: u64) {
    let one = |pk: &[Vec<u8>]| {
        let mut h = H::new();
        if video_stream {
            let rd32 = |o: usize| u32::from_le_bytes(fmt[o..o + 4].try_into().unwrap());
            let Some(ver) = version(&fmt[16..20]) else { return None };
            video(&mut h, ver, usize::try_from(rd32(4)).unwrap(), usize::try_from(rd32(8)).unwrap(), &fmt[40..], pk);
        } else if matches!(u16::from_le_bytes([fmt[0], fmt[1]]), 0x160 | 0x161) {
            audio(&mut h, fmt, pk);
        } else {
            return None;
        }
        Some(h.0)
    };
    if let Some(v) = one(pkts) {
        rows.insert(name.to_string(), v);
        let mut h = H::new();
        for s in 0..copies {
            h.u64(one(&damage(pkts, s)).unwrap());
        }
        rows.insert(format!("{name} damaged x{copies}"), h.0);
    }
}

fn file_rows(rows: &mut BTreeMap<String, u64>, f: &Path, copies: u64) {
    let name = f.file_name().unwrap().to_string_lossy().into_owned();
    if name.to_ascii_lowercase().ends_with(".rcv") {
        let (w, h, extra, pkts) = rcv(f);
        let mut hh = H::new();
        video(&mut hh, None, w, h, &extra, &pkts);
        rows.insert(name.clone(), hh.0);
        let mut hh = H::new();
        for s in 0..copies {
            let mut x = H::new();
            video(&mut x, None, w, h, &extra, &damage(&pkts, s));
            hh.u64(x.0);
        }
        rows.insert(format!("{name} damaged x{copies}"), hh.0);
        return;
    }
    for (i, (fmt, v, pkts)) in streams(f).iter().enumerate() {
        stream_rows(rows, &format!("{name} #{i}"), fmt, *v, pkts, copies);
    }
}

/// An RCV file's size, sequence header and frames.
fn rcv(file: &Path) -> (usize, usize, Vec<u8>, Vec<Vec<u8>>) {
    let b = std::fs::read(file).unwrap();
    let rd32 = |o: usize| usize::try_from(u32::from_le_bytes(b[o..o + 4].try_into().unwrap())).unwrap();
    let ext = rd32(4);
    let extra = b[8..12].to_vec();
    let mut o = 8 + ext;
    let (h, w) = (rd32(o), rd32(o + 4));
    o += 24;
    let mut pkts = Vec::new();
    while o + 8 <= b.len() {
        let n = rd32(o) & 0xff_ffff;
        o += 8;
        pkts.push(b[o..(o + n).min(b.len())].to_vec());
        o += n;
    }
    (w, h, extra, pkts)
}

#[test]
fn pin() {
    let mut rows = BTreeMap::new();
    let mut files: Vec<_> = std::fs::read_dir(testdata()).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "wmv" || e == "wma")).collect();
    files.sort();
    assert!(files.len() >= 7);
    for f in &files {
        file_rows(&mut rows, f, 48);
    }
    // the hand-made VC-1 Main pictures, with and without B pictures and IntraX8, any order
    for (name, extra) in [("vc1 synth", synth::extra(false)), ("vc1 synth b", synth::extra(true))] {
        let b = name.ends_with(" b");
        let mut pkts = Vec::new();
        for (i, k) in [0u8, 1, 2, 1, 2, 2, 0, 1].iter().enumerate() {
            let kind = if b && *k == 2 { 'B' } else { 'P' };
            pkts.push(synth::picture(kind, b, 1 + i % 4));
        }
        let mut h = H::new();
        video(&mut h, None, 80, 48, &extra, &pkts);
        for s in 0..96 {
            video(&mut h, None, 80, 48, &extra, &damage(&pkts, s));
        }
        rows.insert(name.to_string(), h.0);
    }
    let mut h = H::new();
    let mut r = Rng(7);
    for _ in 0..48 {
        let body: Vec<u8> = (0..r.index(600)).map(|_| r.next().to_le_bytes()[0]).collect();
        let (w, hg) = (1 + r.index(70), 1 + r.index(70));
        video(&mut h, None, w, hg, &synth::extra_x8(), &[synth::x8_picture(&body), synth::picture('P', false, hg.div_ceil(16))]);
    }
    rows.insert("vc1 synth x8".into(), h.0);
    // any bytes as any codec's pictures and sequence headers
    let mut h = H::new();
    let mut r = Rng(11);
    for i in 0..400 {
        let ver = [Some(Version::V3), Some(Version::Wmv1), Some(Version::Wmv2), None][i % 4];
        let extra: [u8; 4] = r.next().to_le_bytes()[..4].try_into().unwrap();
        let (w, hg) = (1 + r.index(90), 1 + r.index(90));
        let pkts: Vec<Vec<u8>> = (0..1 + r.index(5)).map(|_| (0..r.index(400)).map(|_| r.next().to_le_bytes()[0]).collect()).collect();
        video(&mut h, ver, w, hg, &extra, &pkts);
    }
    rows.insert("any bytes".into(), h.0);
    if let Ok(d) = std::env::var("WMV_DIR") {
        let mut fs: Vec<_> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().path()).collect();
        fs.sort();
        for f in &fs {
            file_rows(&mut rows, f, 4);
        }
    }
    pins(&testdata().join("pin.tsv"), &rows);
}
