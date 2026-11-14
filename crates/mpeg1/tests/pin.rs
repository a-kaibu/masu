//! Behaviour pins: everything the crate gives (packets, pictures' whole planes, BGRX, samples,
//! probes, frame headers) over testdata/ and seeded damaged copies of it (bits flipped, chunks
//! cut, dropped, swapped), hashed and compared with testdata/pin.tsv, so a refactor that changes
//! one byte of output fails here. `PIN_RECORD=1 cargo test -p mpeg1 --release --test pin`
//! writes the table again (only for a change of behaviour that is meant).

use mpeg1::{audio, ps, Audio, Demux, Video};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// FNV-1a, 64 bits.
struct H(u64);
impl H {
    pub fn new() -> H {
        H(0xcbf2_9ce4_8422_2325)
    }
    pub fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= u64::from(x);
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }
    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn usize(&mut self, v: usize) {
        self.u64(u64::try_from(v).unwrap());
    }
}

/// xorshift64*: the damage is the same on every run.
struct Rng(u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n.max(1)).unwrap()).unwrap()
    }
}

/// `pkts` damaged by seed `seed`: a few bits flipped in a few packets, and now and then a packet
/// cut short, dropped, repeated, or two swapped.
fn damage(pkts: &[Vec<u8>], seed: u64) -> Vec<Vec<u8>> {
    let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut p = pkts.to_vec();
    if p.is_empty() {
        return p;
    }
    for _ in 0..1 + r.below(6) {
        let i = r.below(p.len());
        if !p[i].is_empty() {
            let k = r.below(p[i].len());
            p[i][k] ^= 1 << r.below(8);
        }
    }
    match r.below(6) {
        0 => {
            let i = r.below(p.len());
            let n = r.below(p[i].len());
            p[i].truncate(n);
        }
        1 => {
            p.remove(r.below(p.len()));
        }
        2 => {
            let i = r.below(p.len());
            p.insert(i, p[i].clone());
        }
        3 => {
            let (i, j) = (r.below(p.len()), r.below(p.len()));
            p.swap(i, j);
        }
        _ => {}
    }
    p
}

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

/// The table: name -> hash; PIN_RECORD=1 writes the ones computed (keeping rows of files not
/// here this run, WMV_DIR's), otherwise each computed one must be the table's.
fn check(table: &Path, got: &BTreeMap<String, u64>) {
    let mut want: BTreeMap<String, u64> = BTreeMap::new();
    if let Ok(s) = std::fs::read_to_string(table) {
        for l in s.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
            let (k, v) = l.split_once('\t').unwrap();
            want.insert(k.to_string(), u64::from_str_radix(v, 16).unwrap());
        }
    }
    if std::env::var_os("PIN_RECORD").is_some() {
        want.extend(got.iter().map(|(k, v)| (k.clone(), *v)));
        let mut s = String::from("# behaviour pins: name, FNV-1a of every output (PIN_RECORD=1 writes it)\n");
        for (k, v) in &want {
            s += &format!("{k}\t{v:016x}\n");
        }
        std::fs::write(table, s).unwrap();
        return;
    }
    let bad: Vec<_> = got.iter().filter(|(k, v)| want.get(*k) != Some(v)).map(|(k, v)| format!("{k}: {v:016x}, pinned {:?}", want.get(k).map(|w| format!("{w:016x}")))).collect();
    assert!(bad.is_empty(), "behaviour changed (or new, PIN_RECORD=1):\n{}", bad.join("\n"));
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
    let mut drain = |h: &mut H, v: &mut Video, a: &mut Audio| {
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
            a.out.iter().for_each(|s| h.bytes(&s.to_bits().to_le_bytes()));
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
        None => h.bytes(b"none"),
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
        d.extend((0..r.below(4000)).map(|_| r.next().to_le_bytes()[0]));
        all(&mut h, &d);
    }
    rows.insert("any bytes".into(), h.0);
    check(&testdata().join("pin.tsv"), &rows);
}
