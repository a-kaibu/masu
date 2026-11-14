//! Behaviour pins: headers, packets, samples, seeks and the muxer's output over wmcodec's
//! testdata/ files (and WMV_DIR's when set) and seeded damaged copies of them (bits flipped,
//! chunks cut, dropped, swapped), hashed and compared with testdata/pin.tsv, so a refactor that
//! changes one field of the output fails here. `PIN_RECORD=1 cargo test -p asf --release --test
//! pin` writes the table again (only for a change of behaviour that is meant).

use asf::{Kind, Value};
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

fn header(h: &mut H, x: &asf::Header) {
    for s in &x.streams {
        h.bytes(&[s.number.get(), u8::from(s.encrypted)]);
        match &s.kind {
            Kind::Audio => h.bytes(b"a"),
            Kind::Video { width, height } => {
                h.u64(u64::from(*width));
                h.u64(u64::from(*height));
            }
            Kind::Other(g) => h.bytes(g),
        }
        h.bytes(&s.format);
        h.u64(s.time_offset);
        if let Some(p) = s.spread {
            h.bytes(&[p.span]);
            h.u64(u64::from(p.packet_size));
            h.u64(u64::from(p.chunk_size));
        }
        h.u64(s.avg_time_per_frame);
        h.u64(u64::from(s.bitrate));
        h.u64(u64::from(s.codec()));
    }
    for v in [x.duration, x.preroll, u64::from(x.flags), u64::from(x.packet_size), u64::from(x.max_bitrate), x.data_offset, x.packets] {
        h.u64(v);
    }
    for t in [&x.title, &x.author, &x.copyright, &x.description] {
        h.bytes(t.as_bytes());
    }
    for a in &x.attributes {
        h.u64(u64::from(a.stream));
        h.bytes(a.name.as_bytes());
        match &a.value {
            Value::Str(s) => h.bytes(s.as_bytes()),
            Value::Bytes(b) => h.bytes(b),
            Value::Bool(b) => h.bytes(&[u8::from(*b)]),
            Value::U32(v) => h.u64(u64::from(*v)),
            Value::U64(v) => h.u64(*v),
            Value::U16(v) => h.u64(u64::from(*v)),
            Value::Guid(g) => h.bytes(g),
        }
    }
}

fn sample(h: &mut H, s: &asf::Sample) {
    h.bytes(&[s.stream.get(), u8::from(s.key)]);
    h.u64(s.time);
    h.usize(s.data.len());
    h.bytes(&s.data);
}

/// Everything of one file's bytes.
fn all(h: &mut H, d: &[u8]) {
    match asf::header_size(d) {
        Some(n) => h.u64(n),
        None => h.bytes(b"no size"),
    }
    match asf::header(d) {
        Ok(x) => header(h, &x),
        Err(e) => h.bytes(format!("{e:?}").as_bytes()),
    }
    let mut r = match asf::Reader::new(d.to_vec()) {
        Ok(r) => r,
        Err(e) => return h.bytes(format!("{e:?}").as_bytes()),
    };
    // each packet alone, then the assembled samples, a seek, the samples again
    let (ps, n) = (usize::try_from(r.header.packet_size).unwrap(), usize::try_from(r.header.data_offset).unwrap_or(usize::MAX));
    if ps > 0 && n < d.len() {
        for p in d[n..].chunks(ps).take(400) {
            match asf::packet(p) {
                Ok(p) => {
                    h.u64(u64::from(p.send_time));
                    h.u64(u64::from(p.duration));
                    for q in &p.payloads {
                        h.bytes(&[q.stream.get(), u8::from(q.key)]);
                        for v in [q.object, q.offset, q.object_size, q.time] {
                            h.u64(u64::from(v));
                        }
                        h.bytes(&q.data);
                    }
                }
                Err(e) => h.bytes(format!("{e:?}").as_bytes()),
            }
        }
    }
    let mut samples = Vec::new();
    for _ in 0..5000 {
        let Some(s) = r.next_sample() else { break };
        sample(h, &s);
        samples.push(s);
    }
    let end = samples.last().map_or(0, |s| s.time);
    for t in [0, end / 3, end / 2, end + 1000] {
        h.u64(r.key_before(t));
        r.seek(t);
        for _ in 0..20 {
            match r.next_sample() {
                Some(s) => sample(h, &s),
                None => break,
            }
        }
    }
    r.rewind();
    if let Some(s) = r.next_sample() {
        sample(h, &s);
    }
    // the samples muxed again, and read back
    let mux = asf::write::Mux { packet_size: 1500, preroll: 0, streams: r.header.streams.clone(), attributes: vec![("WM/Title".into(), Value::Str("pin".into()))], compress_small: true };
    let ss: Vec<_> = samples.iter().take(300).map(|s| (s.stream, s.key, u32::try_from(s.time).unwrap_or(u32::MAX), s.data.clone())).collect();
    let w = mux.write(&ss, end).unwrap();
    h.bytes(&w);
    if let Ok(mut r) = asf::Reader::new(w) {
        while let Some(s) = r.next_sample() {
            sample(h, &s);
        }
    }
    for s in r.header.streams.iter().filter_map(|s| s.spread) {
        let n = usize::from(s.span) * usize::from(s.packet_size);
        if n > 0 && n <= d.len() {
            h.bytes(&asf::descramble(&d[..n], s));
            h.bytes(&asf::scramble(&d[..n], s));
        }
    }
}

#[test]
fn pin() {
    let mut rows = BTreeMap::new();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../wmcodec/testdata");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "wmv" || e == "wma")).collect();
    files.sort();
    if let Ok(d) = std::env::var("WMV_DIR") {
        let mut fs: Vec<_> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "wmv")).collect();
        fs.sort();
        files.extend(fs);
    }
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let d = std::fs::read(f).unwrap();
        let mut h = H::new();
        all(&mut h, &d);
        rows.insert(name.clone(), h.0);
        let chunks: Vec<Vec<u8>> = d.chunks(1024).map(<[u8]>::to_vec).collect();
        let mut h = H::new();
        for s in 0..24 {
            let mut x = damage(&chunks, s);
            // (the header too, now and then)
            if s % 3 == 0 && !x.is_empty() && !x[0].is_empty() {
                let k = usize::try_from(s).unwrap() * 37 % x[0].len();
                x[0][k] ^= 0x10;
            }
            all(&mut h, &x.concat());
        }
        rows.insert(format!("{name} damaged x24"), h.0);
    }
    check(&testdata().join("pin.tsv"), &rows);
}
