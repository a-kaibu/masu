//! Behaviour pins: every sample (f32 bits), packet and error over testdata/ and seeded damaged
//! copies of it (audio packets, setup headers and the Ogg pages themselves: bits flipped,
//! packets cut, dropped, swapped), hashed and compared with testdata/pin.tsv, so a refactor that
//! changes one bit of output fails here. `PIN_RECORD=1 cargo test -p vorbis --release --test pin`
//! writes the table again (only for a change of behaviour that is meant).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use vorbis::{ogg, Decoder};

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
        Err(e) => return h.bytes(format!("{e:?}").as_bytes()),
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
        out.iter().for_each(|v| h.bytes(&v.to_bits().to_le_bytes()));
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
            for _ in 0..1 + r.below(4) {
                which[7 + r.below(n - 7)] ^= 1 << r.below(8);
            }
            if s % 5 == 0 {
                which.truncate(7 + r.below(n - 7));
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
        let pkts: Vec<Vec<u8>> = (0..1 + r.below(20)).map(|_| (0..r.below(600)).map(|_| r.next().to_le_bytes()[0]).collect()).collect();
        audio(&mut h, &p[0], &p[2], &pkts);
    }
    rows.insert("any bytes".into(), h.0);
    check(&testdata().join("pin.tsv"), &rows);
}
