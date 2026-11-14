//! dump IN.mpg OUT.yuv OUT.f32: the pictures (yuv420p, picture area) and audio (f32 stereo
//! interleaved) crates/mpeg1 decodes, to compare with other decoders.
use mpeg1::{ps, Audio, Video};
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = std::env::args().skip(1);
    let (Some(inp), Some(out_yuv), Some(out_pcm)) = (a.next(), a.next(), a.next()) else {
        return Err("usage: dump IN.mpg OUT.yuv OUT.f32".into());
    };
    let s = std::fs::read(inp)?;
    let mut v = Video::new();
    let mut au = Audio::new();
    for p in ps::packets(&s) {
        if ps::is_video(p.id) {
            v.push(&p.data);
        } else if ps::is_audio(p.id) {
            au.push(&p.data);
        }
    }
    v.end();
    au.end();
    let mut yuv = std::io::BufWriter::new(std::fs::File::create(out_yuv)?);
    while let Some(f) = v.decode() {
        for y in f.y.chunks(f.y_stride).take(f.height) {
            yuv.write_all(y.get(..f.width).ok_or("short luma row")?)?;
        }
        for p in [&f.cb, &f.cr] {
            for y in p.chunks(f.c_stride).take(f.height / 2) {
                yuv.write_all(y.get(..f.width / 2).ok_or("short chroma row")?)?;
            }
        }
    }
    let mut pcm = std::io::BufWriter::new(std::fs::File::create(out_pcm)?);
    while au.decode() {
        for x in &au.out {
            pcm.write_all(&x.to_le_bytes())?;
        }
    }
    Ok(())
}
