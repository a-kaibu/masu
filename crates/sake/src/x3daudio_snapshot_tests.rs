//! X3DAudio pinned before its code changes shape: X3DAudioInitialize over masks and speeds,
//! and X3DAudioCalculate on seeded scenes (positions, orientations, velocities, cones, inner
//! radii, multichannel emitters with azimuths, distance curves, every flag mix, speaker masks,
//! garbage counts) written through the guest structures; each scene's DSP settings (matrix,
//! delays, levels, Doppler, angle, distance, velocities) are a line of golden/x3daudio.txt.
use super::*;
use crate::test_guest::{call, enter, guest_thread, snapshot, Rng};
use std::fmt::Write;

fn f(r: &mut Rng, lo: f32, hi: f32) -> f32 {
    let t = f32::from(u16::try_from(r.below(65536)).unwrap_or(0)) / 65535.0;
    lo + (hi - lo) * t
}

fn put_f(p: u32, v: f32) {
    w32(p, v.to_bits());
}

fn put_v3(p: u32, v: V3) {
    for (i, x) in (0u32..).zip(v) {
        put_f(p + 4 * i, x);
    }
}

fn unit(r: &mut Rng) -> V3 {
    let v = [f(r, -1.0, 1.0), f(r, -1.0, 1.0), f(r, -1.0, 1.0)];
    let n = len(v);
    if n < 1e-3 { [0.0, 0.0, 1.0] } else { scale(v, 1.0 / n) }
}

/// A front and a top at right angles (or, sometimes, the axes as they come)
fn orientation(r: &mut Rng) -> (V3, V3) {
    match r.below(4) {
        0 => ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        1 => (unit(r), unit(r)),
        _ => {
            let fr = unit(r);
            let t = cross(fr, unit(r));
            let n = len(t);
            (fr, if n < 1e-3 { [0.0, 1.0, 0.0] } else { scale(t, 1.0 / n) })
        }
    }
}

fn cone(r: &mut Rng) -> u32 {
    if r.below(3) != 0 {
        return 0;
    }
    let p = mem::heap_alloc(32, true);
    let a = f(r, 0.0, TWO_PI);
    let vals = [a, f(r, a, TWO_PI), f(r, 0.0, 2.0), f(r, 0.0, 2.0), f(r, 0.0, 1.0), f(r, 0.0, 1.0), f(r, 0.0, 2.0), f(r, 0.0, 2.0)];
    for (i, v) in (0u32..).zip(vals) {
        put_f(p + 4 * i, v);
    }
    p
}

/// An X3DAUDIO_DISTANCE_CURVE of a few increasing points (or none, or an empty one)
fn curve_at(r: &mut Rng, blocks: &mut Vec<u32>) -> u32 {
    if r.below(3) != 0 {
        return 0;
    }
    let n = u32::try_from(r.below(5)).unwrap_or(0);
    let pts = mem::heap_alloc(8 * n + 4, true);
    let mut d = 0.0;
    for i in 0..n {
        put_f(pts + 8 * i, d);
        put_f(pts + 8 * i + 4, f(r, 0.0, 1.0));
        d += f(r, 0.0, 0.6);
    }
    let c = mem::heap_alloc(8, true);
    w32(c, if r.below(8) == 0 { 0 } else { pts });
    w32(c + 4, n);
    blocks.push(pts);
    blocks.push(c);
    c
}

fn bits(v: &[u32]) -> String {
    v.iter().map(|b| if f32::from_bits(*b).is_nan() { format!("nan:{b:x}") } else { format!("{:e}", f32::from_bits(*b)) }).collect::<Vec<_>>().join(",")
}

#[test]
fn x3daudio_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0x3d_a0d1_0);
    let h = mem::heap_alloc(20, true);

    // Initialize over masks and speeds
    let masks = [0u32, 0x4, 0x3, 0x7, 0x33, 0x3f, 0x63f, 0xff, 0x3ff, 0x7ff, 0x3ffff, 0xffff_ffff, 0x8];
    for &m in &masks {
        for sp in [343.5f32, 0.0, -1.0, f32::MIN_POSITIVE, f32::INFINITY, f32::NAN, 1e-39, 1e30] {
            mem::fill(h, 20, 0xee);
            let hr = call(X3DAudioInitialize, &[m, sp.to_bits(), h]);
            let w: Vec<u32> = (0..5).map(|i| r32(h + 4 * i)).collect();
            writeln!(out, "init {m:#x} {sp:e}: {hr:#x} {w:x?}").unwrap();
        }
    }
    writeln!(out, "init null: {:#x}", call(X3DAudioInitialize, &[3, 343.5f32.to_bits(), 0])).unwrap();

    let flag_bits = [MATRIX, DELAY, LPF_DIRECT, LPF_REVERB, REVERB, DOPPLER, EMITTER_ANGLE, ZEROCENTER, REDIRECT_TO_LFE];
    for s in 0..3000 {
        let mask = r.pick(&masks[..12]);
        let speed = r.pick(&[343.5f32, 343.5, 1.0, 1000.0]);
        call(X3DAudioInitialize, &[mask, speed.to_bits(), h]);
        let mut blocks = Vec::new();
        // the listener
        let lp = mem::heap_alloc(52, true);
        let (lf, lt) = orientation(&mut r);
        put_v3(lp, lf);
        put_v3(lp + 12, lt);
        let lpos = if r.below(4) == 0 { [0.0; 3] } else { [f(&mut r, -20.0, 20.0), f(&mut r, -5.0, 5.0), f(&mut r, -20.0, 20.0)] };
        put_v3(lp + 24, lpos);
        put_v3(lp + 36, if r.below(2) == 0 { [0.0; 3] } else { scale(unit(&mut r), f(&mut r, 0.0, 400.0)) });
        let lc = cone(&mut r);
        w32(lp + 48, lc);
        // the emitter
        let ep = mem::heap_alloc(104, true);
        let ec = cone(&mut r);
        w32(ep, ec);
        let (ef, et) = orientation(&mut r);
        put_v3(ep + 4, ef);
        put_v3(ep + 16, et);
        let off = match r.below(6) {
            0 => [0.0; 3],
            1 => scale(unit(&mut r), f(&mut r, 0.0, 1.0)),
            _ => scale(unit(&mut r), f(&mut r, 0.0, 60.0)),
        };
        put_v3(ep + 28, add(lpos, off));
        put_v3(ep + 40, if r.below(2) == 0 { [0.0; 3] } else { scale(unit(&mut r), f(&mut r, 0.0, 400.0)) });
        put_f(ep + 52, if r.below(3) == 0 { f(&mut r, 0.0, 10.0) } else { 0.0 });
        put_f(ep + 56, f(&mut r, 0.0, PI));
        let channels = r.pick(&[1u32, 1, 1, 2, 6, 8, 0, 0xcccc_cccc, 65]);
        w32(ep + 60, channels);
        put_f(ep + 64, f(&mut r, 0.0, 3.0));
        let paz = if channels > 1 && r.below(5) != 0 {
            let n = channels.min(MAX_CHANNELS);
            let p = mem::heap_alloc(4 * n, true);
            for i in 0..n {
                put_f(p + 4 * i, if r.below(6) == 0 { TWO_PI } else { f(&mut r, 0.0, TWO_PI) });
            }
            blocks.push(p);
            p
        } else {
            0
        };
        w32(ep + 68, paz);
        for i in 0..5 {
            let c = curve_at(&mut r, &mut blocks);
            w32(ep + 72 + 4 * i, c);
        }
        put_f(ep + 92, r.pick(&[1.0f32, 1.0, 0.0, 5.0, 20.0, -1.0]));
        put_f(ep + 96, r.pick(&[1.0f32, 1.0, 0.0, 2.0, -1.0]));
        // the settings
        let flags = flag_bits.iter().fold(0, |a, &b| if r.below(2) == 0 { a | b } else { a });
        let dst = r.pick(&[1u32, 2, 2, 6, 8, 0, 0xcccc_cccc, 64]);
        let (src, d) = counts(channels, dst);
        let pm = mem::heap_alloc(4 * 64 * 64, true);
        let pd = mem::heap_alloc(4 * 64, true);
        let dp = mem::heap_alloc(48, true);
        w32(dp, if r.below(10) == 0 { 0 } else { pm });
        w32(dp + 4, if r.below(10) == 0 { 0 } else { pd });
        w32(dp + 8, channels);
        w32(dp + 12, dst);
        for i in 0..8 {
            w32(dp + 16 + 4 * i, 0x7fc0_dead);
        }
        let hr = call(X3DAudioCalculate, &[h, lp, ep, flags, dp]);
        let n = u32::try_from(src * d).unwrap_or(0);
        let m: Vec<u32> = (0..n + 1).map(|i| r32(pm + 4 * i)).collect();
        let dl: Vec<u32> = (0..u32::try_from(d).unwrap_or(0) + 1).map(|i| r32(pd + 4 * i)).collect();
        let tail: Vec<u32> = (0..9).map(|i| r32(dp + 12 + 4 * i)).collect();
        writeln!(out, "scene {s} mask={mask:#x} flags={flags:#x} ch={channels:#x} dst={dst:#x}: {hr:#x}\n  m=[{}]\n  d=[{}]\n  s=[{}]", bits(&m), bits(&dl), bits(&tail)).unwrap();
        for p in blocks.into_iter().chain([lp, ep, pm, pd, dp]) {
            mem::heap_free(p);
        }
        for p in [lc, ec] {
            if p != 0 {
                mem::heap_free(p);
            }
        }
    }
    writeln!(out, "calculate nulls: {:#x} {:#x}", call(X3DAudioCalculate, &[0, 1, 1, 1, 1]), call(X3DAudioCalculate, &[h, 0, 0, 1, 0])).unwrap();
    mem::heap_free(h);
    snapshot("x3daudio", &out);
}
