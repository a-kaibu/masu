//! The XAPO objects pinned before their code changes shape: CreateFX of every class (and some
//! that are not), QueryInterface of every IID on both interfaces, the registration properties,
//! IsInput/OutputFormatSupported over seeded formats, and seeded sessions per class
//! (Initialize, LockForProcess, Process runs on seeded audio, Set/GetParameters, Reset,
//! CalcFrames, Release), each answer a line of golden/xapo.txt. Object addresses show as
//! "obj" / "obj+ps" / other.
use super::*;
use crate::test_guest::{call, enter, guest_thread, hash, snapshot, Rng};
use std::fmt::Write;

fn fx_init(c: &mut Ctx) -> u32 {
    create_fx(c, true)
}
fn fx_plain(c: &mut Ctx) -> u32 {
    create_fx(c, false)
}

fn clsids() -> Vec<[u8; 16]> {
    let fx27 = |n: u8| guid_bytes(0xa90b_c001, 0xe897, 0xe897, [0x74, 0x39, 0x43, 0x55, 0, 0, 0, n]);
    vec![
        guid_bytes(0xf5e0_1117, 0xd6c4, 0x485a, [0xa3, 0xf5, 0x69, 0x51, 0x96, 0xf3, 0xdb, 0xfa]),
        guid_bytes(0xc413_7916, 0x2be1, 0x46fd, [0x85, 0x99, 0x44, 0x15, 0x36, 0xf4, 0x98, 0x56]),
        guid_bytes(0x7d9a_ca56, 0xcb68, 0x4807, [0xb6, 0x32, 0xb1, 0x37, 0x35, 0x2e, 0x85, 0x96]),
        guid_bytes(0x5039_d740, 0xf736, 0x449a, [0x84, 0xd3, 0xa5, 0x62, 0x02, 0x55, 0x7b, 0x87]),
        fx27(0),
        fx27(1),
        fx27(2),
        fx27(3),
        fx27(4),
        guid_bytes(0xc0c5_6f46, 0x29b1, 0x44e9, [0x99, 0x39, 0xa3, 0x2c, 0xe8, 0x68, 0x67, 0xe2]),
        guid_bytes(0xcac1_105f, 0x619b, 0x4d04, [0x83, 0x1a, 0x44, 0xe1, 0xcb, 0xf1, 0x2d, 0x57]),
        guid_bytes(0x4fc3_b166, 0x972a, 0x40cf, [0xbc, 0x37, 0x7d, 0xb0, 0x3d, 0xb2, 0xfb, 0xa3]),
        guid_bytes(0x6f6e_a3a9, 0x2cf5, 0x41cf, [0x91, 0xc1, 0x21, 0x70, 0xb1, 0x54, 0x00, 0x63]),
        guid_bytes(0x6a93_130e, 0x1d53, 0x41d1, [0xa9, 0xcf, 0xe7, 0x58, 0x80, 0x0b, 0xb1, 0x79]),
        guid_bytes(0xc263_3b16, 0x471b, 0x4498, [0xb8, 0xc5, 0x4f, 0x09, 0x59, 0xe2, 0xec, 0x09]),
        [0; 16],
        [0xff; 16],
    ]
}

fn iids() -> Vec<[u8; 16]> {
    let x27 = |n: u8| guid_bytes(0xa90b_c001, 0xe897, 0xe897, [0x55, 0xe4, 0x9e, 0x47, 0, 0, 0, n]);
    vec![
        guid_bytes(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]),
        guid_bytes(0xa410_b984, 0x9839, 0x4819, [0xa0, 0xbe, 0x28, 0x56, 0xae, 0x6b, 0x3a, 0xdb]),
        guid_bytes(0x26d9_5c66, 0x80f2, 0x499a, [0xad, 0x54, 0x5a, 0xe7, 0xf0, 0x1c, 0x6d, 0x98]),
        x27(0),
        x27(1),
        x27(2),
        guid_bytes(1, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]),
    ]
}

fn rel(p: u32, o: u32) -> String {
    if p == 0 {
        "0".into()
    } else if p == o {
        "obj".into()
    } else if p == o + ps() {
        "obj+ps".into()
    } else {
        "other".into()
    }
}

/// A WAVEFORMATEX(TENSIBLE) at a new guest block
fn format(tag: u16, ch: u16, rate: u32, bits: u16, cb: u16, sub: u32) -> u32 {
    let f = mem::heap_alloc(40, true);
    mem::w16(f, tag);
    mem::w16(f + 2, ch);
    w32(f + 4, rate);
    w32(f + 8, rate.wrapping_mul(u32::from(ch)).wrapping_mul(u32::from(bits / 8)));
    mem::w16(f + 12, ch.wrapping_mul(bits / 8));
    mem::w16(f + 14, bits);
    mem::w16(f + 16, cb);
    w32(f + 24, sub);
    f
}

fn seeded_format(r: &mut Rng) -> (u32, String) {
    let tag = r.pick(&[1u16, 3, 0xfffe, 2, 0x55]);
    let ch = r.pick(&[0u16, 1, 2, 6, 8, 64, 65]);
    let rate = r.pick(&[0u32, 8000, 22050, 44100, 48000, 96000, 200_000]);
    let bits = r.pick(&[8u16, 16, 24, 32, 64]);
    let cb = r.pick(&[0u16, 22, 21]);
    let sub = r.pick(&[1u32, 3, 0]);
    (format(tag, ch, rate, bits, cb, sub), format!("fmt({tag:#x},{ch},{rate},{bits},{cb},{sub})"))
}

fn floats(r: &mut Rng, n: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(n * 4);
    for _ in 0..n {
        let x = f32::from(u16::try_from(r.below(20001)).unwrap_or(0)) / 10000.0 - 1.0;
        v.extend_from_slice(&x.to_le_bytes());
    }
    v
}

/// Parameters: plausible floats per kind, random bytes, or a short block
fn seeded_params(r: &mut Rng, kind: Kind) -> Vec<u8> {
    match r.below(4) {
        0 => {
            let n = usize::try_from(r.below(80)).unwrap_or(0);
            r.bytes(n)
        }
        1 => Vec::new(),
        _ => {
            let n = match kind {
                Kind::Eq => 12,
                Kind::Limiter => 2,
                Kind::Reverb => 2,
                Kind::Echo => 3,
                Kind::VolumeMeter => 3,
                Kind::AudioReverb => 13,
            };
            let mut v = Vec::new();
            for i in 0..n {
                if kind == Kind::Limiter {
                    let u = u32::try_from(r.below(2000)).unwrap_or(0);
                    v.extend_from_slice(&u.to_le_bytes());
                } else {
                    let s = [2000.0f32, 2.0, 1.0, 100.0][i % 4];
                    let x = f32::from(u16::try_from(r.below(1001)).unwrap_or(0)) / 1000.0 * s - if r.below(5) == 0 { s / 2.0 } else { 0.0 };
                    v.extend_from_slice(&x.to_le_bytes());
                }
            }
            v
        }
    }
}

fn guest_bytes(v: &[u8]) -> (u32, u32) {
    let p = mem::heap_alloc(u32::try_from(v.len()).unwrap_or(0).max(4), true);
    mem::write(p, v);
    (p, u32::try_from(v.len()).unwrap_or(0))
}

#[test]
fn xapo_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0x0a90_c0de);
    let pg = mem::heap_alloc(16, true);
    let po = mem::heap_alloc(8, true);

    // CreateFX of every class, both export forms
    for (i, g) in clsids().iter().enumerate() {
        mem::write(pg, g);
        for (form, f) in [("init", fx_init as Handler), ("plain", fx_plain as Handler)] {
            crate::mem::putp(po, 0xdead_beef);
            let hr = call(f, &[pg, po, 0, 0]);
            let o = rp(po);
            let k = if hr == 0 { with_this(o, |x| format!("{:?}", x.kind)) } else { None };
            writeln!(out, "createfx class#{i} {form}: {hr:#x} out={} kind={k:?} builtin={:?}", if o == 0xdead_beef { "untouched".into() } else if o == 0 { "0".into() } else { "obj".to_string() }, builtin(o).map(|b| b.addr() == o)).unwrap();
            if hr == 0 {
                writeln!(out, "  builtin(obj+ps)={:?} create_instance={:?}", builtin(o + ps()).map(|b| b.addr() == o), create_instance(pg).is_some()).unwrap();
                call(Release, &[o]);
            }
        }
    }
    writeln!(out, "createfx null clsid: {:#x}, null out: {:#x}", call(fx_init, &[0, po, 0, 0]), call(fx_init, &[pg, 0, 0, 0])).unwrap();

    // QueryInterface / AddRef / Release on both interfaces
    let fx = clsids()[3];
    mem::write(pg, &fx);
    call(fx_plain, &[pg, po]);
    let o = rp(po);
    for (i, iid) in iids().iter().enumerate() {
        for this in [o, o + ps()] {
            let pi = mem::heap_alloc(16, true);
            mem::write(pi, iid);
            crate::mem::putp(po, 0xdead_beef);
            let hr = call(QueryInterface, &[this, pi, po]);
            writeln!(out, "qi iid#{i} on {}: {hr:#x} -> {}", rel(this, o), rel(rp(po), o)).unwrap();
            mem::heap_free(pi);
        }
    }
    writeln!(out, "qi null out: {:#x}", call(QueryInterface, &[o, pg, 0])).unwrap();
    writeln!(out, "addref: {} {}", call(AddRef, &[o]), call(AddRef, &[o + ps()])).unwrap();
    let mut rels = Vec::new();
    for _ in 0..12 {
        rels.push(call(Release, &[o + ps()]));
    }
    writeln!(out, "releases: {rels:?} alive={}", builtin(o).is_some()).unwrap();
    writeln!(out, "addref/release of a dead object: {} {}", call(AddRef, &[o]), call(Release, &[o])).unwrap();

    // GetRegistrationProperties of every class
    for (i, g) in clsids().iter().enumerate() {
        let Some(o) = create(g).map(Obj::addr) else { continue };
        crate::mem::putp(po, 0);
        let hr = call(GetRegistrationProperties, &[o, po]);
        let p = rp(po);
        let name = String::from_utf16_lossy(&mem::wstr(p + 16));
        let words: Vec<u32> = (0..7).map(|j| r32(p + 1040 + 4 * j)).collect();
        writeln!(out, "regprops class#{i}: {hr:#x} clsid={} name={name:?} words={words:?} hash={:#x}", &mem::array::<16>(p) == g, hash(p, 1068)).unwrap();
        writeln!(out, "  null out: {:#x}", call(GetRegistrationProperties, &[o + ps(), 0])).unwrap();
        call(Release, &[o]);
    }
    writeln!(out, "regprops of no object: {:#x}", call(GetRegistrationProperties, &[0x1234_5678, po])).unwrap();

    // IsInput/OutputFormatSupported over seeded formats
    for _ in 0..300 {
        let (f, d) = seeded_format(&mut r);
        let with_out = r.below(4) != 0;
        crate::mem::putp(po, 0xdead_beef);
        let hr = call(IsFormatSupported, &[0, 0, f, if with_out { po } else { 0 }]);
        let p = rp(po);
        let got = if !with_out { "-".to_string() } else if p == 0xdead_beef { "untouched".into() } else if p == 0 { "0".into() } else { format!("{:?}", &mem::array::<18>(p)) };
        writeln!(out, "supported {d} out={with_out}: {hr:#x} {got}").unwrap();
        mem::heap_free(f);
    }
    writeln!(out, "supported null: {:#x}", call(IsFormatSupported, &[0, 0, 0, po])).unwrap();

    // Seeded sessions
    let all = clsids();
    for s in 0..600 {
        let g = all[usize::try_from(r.below(15)).unwrap_or(0)];
        mem::write(pg, &g);
        let init = seeded_params(&mut r, kind_of(&g).unwrap_or(Kind::Eq));
        let init = if r.below(3) == 0 && kind_of(&g) == Some(Kind::Echo) { u32::try_from(r.below(3000)).unwrap_or(0).to_le_bytes().to_vec() } else { init };
        let (ip, il) = guest_bytes(&init);
        let use_init = r.below(2) == 0;
        let hr = call(fx_init, &[pg, po, if use_init { ip } else { 0 }, il]);
        let o = rp(po);
        writeln!(out, "session {s} kind={:?} init={} ({il} bytes): {hr:#x}", kind_of(&g), use_init).unwrap();
        let this = if r.below(2) == 0 { o } else { o + ps() };
        if r.below(3) == 0 {
            let (p, n) = guest_bytes(&seeded_params(&mut r, kind_of(&g).unwrap_or(Kind::Eq)));
            writeln!(out, "  initialize: {:#x}", call(Initialize, &[this, p, n])).unwrap();
        }
        // lock
        let (fi, di) = if r.below(8) == 0 { seeded_format(&mut r) } else {
            let ch = r.pick(&[1u16, 2, 6]);
            let rate = r.pick(&[22050u32, 44100, 48000]);
            (format(r.pick(&[3u16, 0xfffe]), ch, rate, 32, 22, 3), format!("float({ch},{rate})"))
        };
        let fo = if r.below(12) == 0 { seeded_format(&mut r).0 } else { fi };
        let max = r.pick(&[0u32, 1, 64, 256, 480, 480, 480, 480]);
        let lp = mem::heap_alloc(16, true);
        crate::mem::putp(lp, fi);
        w32(lp + ps(), max);
        let lo = mem::heap_alloc(16, true);
        crate::mem::putp(lo, fo);
        w32(lo + ps(), max);
        let (nin, nout) = if r.below(10) == 0 { (r.pick(&[0u32, 2]), 1) } else { (1, 1) };
        let hr = call(LockForProcess, &[o, nin, lp, nout, lo]);
        writeln!(out, "  lock {di} out_same={} max={max} n={nin}/{nout}: {hr:#x}", fo == fi).unwrap();
        // runs
        let runs = r.below(5);
        for _ in 0..runs {
            if r.below(5) == 0 {
                let (p, n) = guest_bytes(&seeded_params(&mut r, kind_of(&g).unwrap_or(Kind::Eq)));
                writeln!(out, "  set {n}: {:#x}", call(SetParameters, &[o + ps(), p, n])).unwrap();
            }
            let ch = u32::from(r16(fi + 2)).min(64);
            let valid = r.pick(&[0u32, 1, 17, 64, 200, 480, 480, 0xcccc_cccc]);
            let frames = valid.min(max).min(480);
            let n = usize::try_from(frames * ch.max(1)).unwrap_or(0);
            let (ib, _) = guest_bytes(&floats(&mut r, n));
            let inplace = r.below(2) == 0;
            let ob = if inplace { ib } else { mem::heap_alloc(u32::try_from(n * 4).unwrap_or(0).max(4), true) };
            let flags = r.pick(&[0u32, 1, 1, 1]);
            let enabled = r.below(5) != 0;
            let pin = mem::heap_alloc(16, true);
            crate::mem::putp(pin, ib);
            w32(pin + ps(), flags);
            w32(pin + ps() + 4, valid);
            let pout = mem::heap_alloc(16, true);
            crate::mem::putp(pout, ob);
            w32(pout + ps(), 7);
            w32(pout + ps() + 4, 9);
            let hr = call(Process, &[this, 1, pin, 1, pout, u32::from(enabled)]);
            writeln!(out, "  process valid={valid:#x} flags={flags} en={enabled} inplace={inplace}: {hr:#x} out flags={} frames={} hash={:#x}", r32(pout + ps()), r32(pout + ps() + 4), hash(ob, u32::try_from(n * 4).unwrap_or(0))).unwrap();
            mem::heap_free(pin);
            mem::heap_free(pout);
            if !inplace {
                mem::heap_free(ob);
            }
            mem::heap_free(ib);
            // read back
            let gp = mem::heap_alloc(64, true);
            let peaks = mem::heap_alloc(256, true);
            let rmss = mem::heap_alloc(256, true);
            crate::mem::putp(gp, peaks);
            crate::mem::putp(gp + ps(), rmss);
            w32(gp + 2 * ps(), r.pick(&[0u32, 1, 2, 8]));
            let n = r.pick(&[0u32, 4, 12, 52]);
            let hr = call(GetParameters, &[o + ps(), gp, n]);
            // (the level pointers are heap addresses: zeros where they were left)
            if rp(gp) == peaks {
                crate::mem::putp(gp, 0);
            }
            if rp(gp + ps()) == rmss {
                crate::mem::putp(gp + ps(), 0);
            }
            writeln!(out, "  get {n}: {hr:#x} params={:#x} peaks={:#x} rms={:#x}", hash(gp, 64), hash(peaks, 256), hash(rmss, 256)).unwrap();
            mem::heap_free(gp);
            mem::heap_free(peaks);
            mem::heap_free(rmss);
        }
        if r.below(4) == 0 {
            writeln!(out, "  reset: {:#x}", call(Reset, &[this])).unwrap();
        }
        if r.below(4) == 0 {
            writeln!(out, "  unlock: {:#x}; process after: {:#x}", call(UnlockForProcess, &[this]), call(Process, &[this, 1, lp, 1, lo, 1])).unwrap();
        }
        writeln!(out, "  calcframes: {} {}", call(CalcFrames, &[this, 123]), call(CalcFrames, &[this, 0xffff_ffff])).unwrap();
        writeln!(out, "  process null: {:#x}", call(Process, &[this, 1, 0, 1, 0, 1])).unwrap();
        writeln!(out, "  release: {}", call(Release, &[o])).unwrap();
        for p in [ip, lp, lo, fi] {
            mem::heap_free(p);
        }
        if fo != fi {
            mem::heap_free(fo);
        }
    }
    mem::heap_free(pg);
    mem::heap_free(po);
    snapshot("xapo", &out);
}
