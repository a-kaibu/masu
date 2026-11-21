//! Decoding speed of a file's video (`cargo run --release --example bench FILE.wmv`; for wasm,
//! see wasm-test.sh): ms per picture, the decoder alone, and a hash of the pictures (the same
//! from every build: scalar or SIMD, host or wasm).
use wmcodec::msmpeg4::{Decoder, Version};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn dbg<E: std::fmt::Debug>(e: E) -> Box<dyn std::error::Error> {
    format!("{e:?}").into()
}

fn main() -> Res<()> {
    let file = std::env::args().nth(1).ok_or("usage: bench FILE.wmv")?;
    let mut r = asf::Reader::new(std::fs::read(&file)?).map_err(dbg)?;
    let s = r.header.streams.iter().find(|s| matches!(s.kind, asf::Kind::Video { .. })).ok_or("no video stream")?.clone();
    let f = &s.format;
    let rd32 = |o: usize| -> Res<usize> {
        let b: [u8; 4] = f.get(o..o + 4).ok_or("short format")?.try_into()?;
        Ok(usize::try_from(u32::from_le_bytes(b))?)
    };
    let (w, h) = (rd32(4)?, rd32(8)?);
    let fourcc = f.get(16..20).ok_or("short format")?;
    let extra = f.get(40..).unwrap_or_default();
    let mut pics = Vec::new();
    while let Some(x) = r.next_sample() {
        if x.stream == s.number {
            pics.push(x.data);
        }
    }
    let mut spent = std::time::Duration::ZERO;
    let (mut n, mut hash) = (0u32, kome::golden::Fnv::new());
    let mut see = |p: &wmcodec::msmpeg4::Picture| {
        n += 1;
        for plane in &p.planes {
            hash.bytes(plane);
        }
    };
    if fourcc == b"WMV3" {
        let mut d = wmcodec::vc1::Decoder::new(w, h, extra).map_err(dbg)?;
        for p in &pics {
            let t = std::time::Instant::now();
            let got = d.decode(p);
            spent += t.elapsed();
            if let Ok(Some(p)) = got {
                see(&p);
            }
        }
        if let Some(p) = d.flush() {
            see(&p);
        }
    } else {
        let v = match fourcc {
            b"WMV1" => Version::Wmv1,
            b"WMV2" => Version::Wmv2,
            _ => Version::V3,
        };
        let mut d = Decoder::new(v, w, h, extra).map_err(dbg)?;
        for p in &pics {
            let t = std::time::Instant::now();
            let got = d.decode(p);
            spent += t.elapsed();
            if let Ok(Some(p)) = got {
                see(&p);
            }
        }
    }
    let ms = spent.as_secs_f64() * 1000.0;
    println!("{file}: {w}x{h}, {n} pictures, {:.2} ms per picture, hash {:016x}", ms / f64::from(n.max(1)), hash.get());
    Ok(())
}
