//! decbench DIR [REPS]: times sake's own decoders on the inputs tests/decbench/run.sh writes to
//! DIR (pic.png, pic.jpg, movie.mpg; the Vorbis stream is the vorbis crate's test data), one
//! line per case: `BENCH <name> <ms per rep>`. Built for wasm32 (with and without SIMD128) and
//! run under node, as the page's sake.wasm runs in V8.
use std::time::Instant;

type Res = Result<(), Box<dyn std::error::Error>>;

/// The benchmark groups, picked by name prefix (`decbench DIR REPS png,mpeg1`).
#[derive(Clone, Copy)]
enum Case {
    Png,
    Jpeg,
    Idec,
    Mpeg1,
    Yuv,
    Vorbis,
    Dxt,
}

impl Case {
    const ALL: [Case; 7] = [Case::Png, Case::Jpeg, Case::Idec, Case::Mpeg1, Case::Yuv, Case::Vorbis, Case::Dxt];

    fn name(self) -> &'static str {
        match self {
            Case::Png => "png",
            Case::Jpeg => "jpeg",
            Case::Idec => "idec",
            Case::Mpeg1 => "mpeg1",
            Case::Yuv => "yuv",
            Case::Vorbis => "vorbis",
            Case::Dxt => "dxt",
        }
    }
}

fn time(name: &str, reps: u32, mut f: impl FnMut() -> Result<u64, Box<dyn std::error::Error>>) -> Res {
    // (one untimed rep: tier-up, caches)
    let mut sink = f()?;
    let t = Instant::now();
    for _ in 0..reps {
        sink = sink.wrapping_add(f()?);
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / f64::from(reps);
    println!("BENCH {name} {ms:.3} (check {sink:x})");
    Ok(())
}

fn sum(b: &[u8]) -> u64 {
    b.iter().step_by(61).fold(0u64, |a, &x| a.wrapping_mul(31).wrapping_add(u64::from(x)))
}

/// The low byte of a pseudo-random fill value.
fn low(x: u32) -> u8 {
    x.to_le_bytes()[0]
}

fn us(x: u32) -> Result<usize, std::num::TryFromIntError> {
    usize::try_from(x)
}

/// mpeg1's video stream of a program stream, ended.
fn video(data: &[u8]) -> mpeg1::Video {
    let mut v = mpeg1::Video::new();
    for p in mpeg1::ps::packets(data) {
        if mpeg1::ps::is_video(p.id) {
            v.push(&p.data);
        }
    }
    v.end();
    v
}

fn main() -> Res {
    let a: Vec<String> = std::env::args().collect();
    let arg = |i: usize| a.get(i).ok_or("usage: decbench DIR [REPS [CASES]] | decbench jpeg IN OUT");
    // (decbench jpeg IN OUT: jpeg-decoder's pixels of IN, raw, to set this build's against
    // Windows' answers: tests/decbench/run.sh jpeg)
    if arg(1)? == "jpeg" {
        let mut d = jpeg_decoder::Decoder::new(std::fs::File::open(arg(2)?)?);
        std::fs::write(arg(3)?, d.decode()?)?;
        return Ok(());
    }
    let dir = std::path::PathBuf::from(arg(1)?);
    let reps: u32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let only = a.get(3).cloned().unwrap_or_default();
    let want = |c: Case| only.is_empty() || only.split(',').any(|o| c.name().starts_with(o));

    for case in Case::ALL.into_iter().filter(|&c| want(c)) {
        match case {
            Case::Png => {
                let data = std::fs::read(dir.join("pic.png"))?;
                time("png_1280x720", reps, || {
                    let d = png::Decoder::new(&data[..]);
                    let mut r = d.read_info()?;
                    let mut buf = vec![0; r.output_buffer_size()];
                    r.next_frame(&mut buf)?;
                    Ok(sum(&buf))
                })?;
            }
            Case::Jpeg => {
                let data = std::fs::read(dir.join("pic.jpg"))?;
                time("jpeg_1280x720", reps, || {
                    let mut d = jpeg_decoder::Decoder::new(&data[..]);
                    Ok(sum(&d.decode()?))
                })?;
            }
            Case::Idec => {
                // (what the guest still does for a JPEG the page decoded: its planes -> BGR)
                let (w, h) = (1280u32, 720u32);
                let layout = kura::idec::Layout::I420;
                let n = u32::try_from(layout.size(w, h))?;
                let bytes = (0..n).map(|i| low(i.wrapping_mul(2654435761) >> 13)).collect();
                let p = kura::idec::Picture { w, h, layout, bytes };
                time("idec_i420_bgr24_1280x720", reps.saturating_mul(5), || Ok(sum(&p.bgr24().ok_or("bgr24")?)))?;
            }
            Case::Mpeg1 => {
                let data = std::fs::read(dir.join("movie.mpg"))?;
                let mut frames = 0u32;
                time("mpeg1_640x480_frame", reps, || {
                    let mut v = video(&data);
                    let mut s = 0;
                    frames = 0;
                    while let Some(f) = v.decode() {
                        s ^= sum(&f.y);
                        frames = frames.saturating_add(1);
                    }
                    Ok(s)
                })?;
                println!("(mpeg1: {frames} frames a rep)");
                // the picture -> DirectShow RGB32 conversion
                let mut v = video(&data);
                let f = v.decode().ok_or("no mpeg1 frame")?;
                let stride = f.width.checked_mul(4).ok_or("width")?;
                let mut out = vec![0u8; stride.checked_mul(f.height).ok_or("size")?];
                time("mpeg1_to_bgrx_640x480", reps.saturating_mul(10), || {
                    mpeg1::video::to_bgrx(&f, &mut out, stride, true);
                    Ok(sum(&out))
                })?;
            }
            Case::Yuv => {
                const W: u32 = 1280;
                const H: u32 = 720;
                let buf: Vec<u8> = (0..W * H * 3 / 2).map(|i| low(i.wrapping_mul(2654435761) >> 13)).collect();
                let layout = [[0, 0, W, W * H, W / 2, W * H * 5 / 4, W / 2], [0; 7]];
                let pitch = us(4 * W)?;
                let mut d = vec![0u8; us(4 * W * H)?];
                for (name, out) in [("yuv_rgb601_1280x720", kura::video::Out::Rgb601), ("yuv_rgb_1280x720", kura::video::Out::Rgb), ("yuv_ycc_1280x720", kura::video::Out::Ycc)] {
                    time(name, reps.saturating_mul(5), || {
                        // SAFETY: buf holds the I420 picture layout describes (W*H*3/2 bytes)
                        // and d is W*H rows of pitch bytes.
                        unsafe { kura::video::convert(buf.as_ptr(), &layout, 1, W, H, d.as_mut_ptr(), pitch, W, H, out) };
                        Ok(sum(&d))
                    })?;
                }
            }
            Case::Vorbis => {
                let data = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../vorbis/testdata/stereo-q5.ogg"))?;
                let (packets, _) = vorbis::ogg::packets(&data);
                let [ident, _, setup, audio @ ..] = &packets[..] else {
                    return Err("stereo-q5.ogg: no headers".into());
                };
                let mut n = 0;
                time("vorbis_stereo_q5", reps.saturating_mul(5), || {
                    let mut d = vorbis::Decoder::new(ident, setup).map_err(|e| format!("vorbis: {e:?}"))?;
                    let mut out = Vec::new();
                    for p in audio {
                        d.packet(p, &mut out);
                    }
                    n = out.len();
                    Ok(out.iter().step_by(97).fold(0u64, |a, x| a.wrapping_add(u64::from(x.to_bits()))))
                })?;
                println!("(vorbis: {n} samples a rep)");
            }
            Case::Dxt => {
                use kura::d3d9::format as f;
                const W: u32 = 1024;
                const H: u32 = 1024;
                for (name, fmt, bs) in [("dxt1_1024", f::DXT1, 8u32), ("dxt5_1024", f::DXT5, 16)] {
                    let src: Vec<u8> = (0..(W / 4) * (H / 4) * bs).map(|i| low(i.wrapping_mul(2246822519) >> 11)).collect();
                    let pitch = us(W / 4 * bs)?;
                    let mut out = Vec::new();
                    time(name, reps, || {
                        out.clear();
                        // SAFETY: src holds H/4 block rows of pitch bytes, the W x H texture.
                        unsafe { f::convert(fmt, src.as_ptr(), pitch, W, H, &mut out) };
                        Ok(sum(&out))
                    })?;
                }
            }
        }
    }
    Ok(())
}
