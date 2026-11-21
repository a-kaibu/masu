//! Behaviour pins: headers, packets, samples, seeks and the muxer's output over wmcodec's
//! testdata/ files (and WMV_DIR's when set) and seeded damaged copies of them (bits flipped,
//! chunks cut, dropped, swapped), hashed and compared with testdata/pin.tsv, so a refactor that
//! changes one field of the output fails here. `MASU_UPDATE_GOLDEN=1 cargo test -p asf --release --test
//! pin` writes the table again (only for a change of behaviour that is meant).

use asf::{Kind, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kome::golden::{Fnv as H, damage, pins};

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn header(h: &mut H, x: &asf::Header) {
    for s in &x.streams {
        h.bytes(&[s.number.get(), u8::from(s.encrypted)]);
        match &s.kind {
            Kind::Audio => { h.bytes(b"a"); }
            Kind::Video { width, height } => {
                h.u64(u64::from(*width));
                h.u64(u64::from(*height));
            }
            Kind::Other(g) => { h.bytes(g); }
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
            Value::Str(s) => { h.bytes(s.as_bytes()); }
            Value::Bytes(b) => { h.bytes(b); }
            Value::Bool(b) => { h.bytes(&[u8::from(*b)]); }
            Value::U32(v) => { h.u64(u64::from(*v)); }
            Value::U64(v) => { h.u64(*v); }
            Value::U16(v) => { h.u64(u64::from(*v)); }
            Value::Guid(g) => { h.bytes(g); }
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
        Some(n) => { h.u64(n); }
        None => { h.bytes(b"no size"); }
    }
    match asf::header(d) {
        Ok(x) => header(h, &x),
        Err(e) => { h.bytes(format!("{e:?}").as_bytes()); }
    }
    let mut r = match asf::Reader::new(d.to_vec()) {
        Ok(r) => r,
        Err(e) => { h.bytes(format!("{e:?}").as_bytes()); return; }
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
                Err(e) => { h.bytes(format!("{e:?}").as_bytes()); }
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
    pins(&testdata().join("pin.tsv"), &rows);
}
