//! fxdsp pinned before its code changes shape: every effect (EQ, limiter, echo, both reverb
//! shapes) run directly over seeded parameter blocks, channel counts and rates (in and out of
//! XAudio2's limits), a few passes each with the parameters changing between them; the output
//! bits of each pass are a line of golden/fxdsp.txt.
use super::*;
use crate::test_guest::{snapshot, Rng};
use std::fmt::Write;

fn h(v: &[f32]) -> u64 {
    v.iter().flat_map(|x| x.to_bits().to_le_bytes()).fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

fn fl(r: &mut Rng, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * f32::from(u16::try_from(r.below(65536)).unwrap_or(0)) / 65535.0
}

fn block(r: &mut Rng, ranges: &[(f32, f32)]) -> Vec<u8> {
    match r.below(6) {
        0 => Vec::new(),
        1 => {
            let n = usize::try_from(r.below(60)).unwrap_or(0);
            r.bytes(n)
        }
        2 => [f32::NAN, f32::INFINITY, -1e30, 0.0].iter().cycle().take(ranges.len()).flat_map(|x| x.to_le_bytes()).collect(),
        _ => ranges.iter().flat_map(|&(lo, hi)| fl(r, lo, hi).to_le_bytes()).collect(),
    }
}

#[test]
fn fxdsp_snapshot() {
    let mut out = String::new();
    let mut r = Rng(0xf8d5_9);
    let eq: Vec<(f32, f32)> = (0..4).flat_map(|_| [(0.0, 25000.0), (0.0, 9.0), (0.0, 2.5)]).collect();
    let echo = [(-0.5, 1.5), (-0.5, 1.5), (0.0, 2500.0)];
    let fxr = [(-0.5, 1.5), (0.0, 1.2)];
    let aur: Vec<(f32, f32)> = (0..13).map(|i| if i == 0 || i == 11 || i == 12 { (0.0, 110.0) } else if i == 10 { (0.0, 30.0) } else { (-110.0, 25.0) }).collect();
    for s in 0..400 {
        let ch = r.pick(&[1usize, 1, 2, 2, 6, 0, 64, 65]);
        let rate = r.pick(&[8000u32, 22050, 44100, 48000, 96000, 999, 1000, 200_000, 200_001]);
        let kind = r.below(5);
        let mut eqs = Eq::default();
        let mut lim = Limiter::default();
        let mut ech = Echo::with_max(r.pick(&[0.0f32, 0.5, 10.0, 300.0, 2000.0, 5000.0]));
        let mut rev = Reverb::default();
        for pass in 0..r.below(4) + 1 {
            let frames = usize::try_from(r.below(300)).unwrap_or(0);
            let mut buf: Vec<f32> = (0..frames * ch).map(|_| fl(&mut r, -1.5, 1.5)).collect();
            let what = match kind {
                0 => {
                    let p = block(&mut r, &eq);
                    eqs.run(&p, &mut buf, ch, rate);
                    "eq"
                }
                1 => {
                    let p: Vec<u8> = if r.below(2) == 0 { block(&mut r, &[]) } else { [r.below(25), r.below(2000)].iter().flat_map(|&v| u32::try_from(v).unwrap_or(0).to_le_bytes()).collect() };
                    lim.run(&p, &mut buf, ch, rate);
                    "limiter"
                }
                2 => {
                    let p = block(&mut r, &echo);
                    ech.run(&p, &mut buf, ch, rate);
                    "echo"
                }
                3 => {
                    let p = block(&mut r, &fxr);
                    rev.run(&ReverbShape::fx(&p), &mut buf, ch, rate);
                    "fxreverb"
                }
                _ => {
                    let p = block(&mut r, &aur);
                    rev.run(&ReverbShape::audio(&p), &mut buf, ch, rate);
                    "audioreverb"
                }
            };
            writeln!(out, "{s}.{pass} {what} ch={ch} rate={rate} frames={frames}: {:#x}", h(&buf)).unwrap();
        }
    }
    for p in [&[][..], &[0, 0, 0x80, 0x3f][..], &[0, 0, 0xc0, 0x7f, 1][..]] {
        writeln!(out, "f/u {p:?}: {:?} {:?} {:?}", f(p, 0, 7.0).to_bits(), u(p, 0, 7), u(p, 1, 7)).unwrap();
    }
    writeln!(out, "db: {:?}", [-100.0f32, -6.0, 0.0, 20.0].map(|v| db(v).to_bits())).unwrap();
    snapshot("fxdsp", &out);
}
