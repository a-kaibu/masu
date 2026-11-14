//! The mixer API pinned before its code changes shape: a seeded sweep over every entry point
//! (caps A/W, open, GetID, GetLineInfo A/W, GetLineControls A/W, Get/SetControlDetails) with
//! handles, object kinds, query flags, structure sizes, counts and values near and away from
//! the valid ones; each call's result and what it wrote (a hash of the caller's block) is a
//! line of golden/mixer.txt. mixerOpen's handles show as base + order.
use super::*;
use crate::test_guest::{call, enter, guest_thread, hash, snapshot, Rng};
use std::fmt::Write;

fn caps_a(c: &mut Ctx) -> u32 {
    caps(c, false)
}
fn caps_w(c: &mut Ctx) -> u32 {
    caps(c, true)
}
fn line_a(c: &mut Ctx) -> u32 {
    line_info_api(c, false)
}
fn line_w(c: &mut Ctx) -> u32 {
    line_info_api(c, true)
}
fn controls_a(c: &mut Ctx) -> u32 {
    line_controls_api(c, false)
}
fn controls_w(c: &mut Ctx) -> u32 {
    line_controls_api(c, true)
}
fn details_a(c: &mut Ctx) -> u32 {
    get_details(c, false)
}
fn details_w(c: &mut Ctx) -> u32 {
    get_details(c, true)
}

fn handle(r: &mut Rng, opened: &[u32]) -> u32 {
    match r.below(6) {
        0 => 0,
        1 => r.pick(&[1u32, 2, 0xffff_ffff, HMIXER_BASE + 0x1_0000]),
        2 => u32::try_from(r.next() & 0xffff_ffff).unwrap_or(0),
        _ => opened.last().copied().unwrap_or(HMIXER_BASE),
    }
}

fn obj_flags(r: &mut Rng) -> u32 {
    r.pick(&[OBJ_MIXER, OBJ_HMIXER, OBJ_WAVEOUT, OBJ_HWAVEOUT, 0x2000_0000, 0x3000_0000, 0xa000_0000, 0x4000_0000])
}

#[test]
fn mixer_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0x0141_e7);
    let blk = mem::heap_alloc(4096, true);
    let arr = mem::heap_alloc(4096, true);
    let pa = mem::heap_alloc(64, true);
    let mut opened = Vec::new();
    let base = OPENS.load(Ordering::Relaxed);
    writeln!(out, "numdevs: {}", call(mixerGetNumDevs, &[])).unwrap();
    for i in 0..4000 {
        mem::fill(blk, 4096, 0xcd);
        mem::fill(arr, 4096, 0xcd);
        let which = r.below(10);
        let line = match which {
            0 | 1 => {
                let wide = which == 1;
                let id = r.pick(&[0u32, 1, 0xffff_ffff, HMIXER_BASE]);
                let size = r.pick(&[0u32, 4, 8, 47, 48, 80, 200]);
                let p = if r.below(8) == 0 { 0 } else { blk };
                let hr = call(if wide { caps_w } else { caps_a }, &[id, p, size]);
                format!("caps{} id={id:#x} size={size} p={}: {hr} {:#x}", if wide { "W" } else { "A" }, p != 0, hash(blk, 200))
            }
            2 => {
                let id = r.pick(&[0u32, 0, 1, 7]);
                let cb = r.pick(&[0u32, 0, 0x1234, 0xdead_beef]);
                let flags = r.pick(&[0u32, 0x1_0000, 0x2_0000, 0x3_0000, 0x5_0000, 0x7_0000, 0x8000_0000]);
                let p = if r.below(8) == 0 { 0 } else { blk };
                let hr = call(mixerOpen, &[p, id, cb, 0, flags]);
                let h = r32(blk);
                let shown = if hr == 0 && p != 0 {
                    opened.push(h);
                    format!("base+{}", (h & 0xffff).wrapping_sub(base & 0xffff))
                } else {
                    format!("{h:#x}")
                };
                format!("open id={id} cb={cb:#x} flags={flags:#x}: {hr} {shown} (high {:#x})", h & 0xffff_0000)
            }
            3 => {
                let h = handle(&mut r, &opened);
                let flags = obj_flags(&mut r);
                let p = if r.below(8) == 0 { 0 } else { blk };
                let hr = call(mixerGetID, &[h, p, flags]);
                format!("getid h={} flags={flags:#x}: {hr} {:#x}", h == opened.last().copied().unwrap_or(1), r32(blk))
            }
            4 | 5 => {
                let wide = which == 5;
                let at = line_at(wide);
                let h = handle(&mut r, &opened);
                let size = r.pick(&[at.size, at.size, at.size, at.size + 4, at.size - 4, 0]);
                w32(blk, size);
                w32(blk + 4, r.pick(&[0u32, 0, 1, 2]));
                w32(blk + 8, r.pick(&[0u32, 0, 1, 0xffff_ffff]));
                w32(blk + 12, r.pick(&[DEST_LINE, SRC_LINE, 5, 0xffff_0001]));
                w32(blk + at.comp, r.pick(&[DST_SPEAKERS, SRC_WAVEOUT, 7, 0x1003, 0]));
                let flags = obj_flags(&mut r) | u32::try_from(r.below(7)).unwrap_or(0) | if r.below(10) == 0 { 0x100 } else { 0 };
                let p = if r.below(10) == 0 { 0 } else { blk };
                let hr = call(if wide { line_w } else { line_a }, &[h, p, flags]);
                format!("line{} h={} size={size} flags={flags:#x}: {hr} {:#x}", if wide { "W" } else { "A" }, h == opened.last().copied().unwrap_or(1), hash(blk, at.size + 8))
            }
            6 | 7 => {
                let wide = which == 7;
                let h = handle(&mut r, &opened);
                let size = lines_controls_size();
                let cs = control_size(wide);
                let sz = r.pick(&[size, size, size, size + 4, 0]);
                w32(blk, sz);
                w32(blk + 4, r.pick(&[DEST_LINE, SRC_LINE, 3]));
                w32(blk + 8, r.pick(&[0u32, 1, 2, 3, CONTROL_VOLUME, CONTROL_MUTE]));
                w32(blk + 12, r.pick(&[0u32, 1, 2, 2, 3]));
                let cb = r.pick(&[cs, cs, cs, cs + 8, cs - 4, 0]);
                w32(blk + 16, cb);
                mem::wp(blk + 20, if r.below(10) == 0 { 0 } else { arr });
                let flags = r.pick(&[OBJ_MIXER, OBJ_HMIXER, OBJ_WAVEOUT, OBJ_HWAVEOUT]) | u32::try_from(r.below(5)).unwrap_or(0);
                let p = if r.below(10) == 0 { 0 } else { blk };
                let hr = call(if wide { controls_w } else { controls_a }, &[h, p, flags]);
                format!("controls{} h={} size={sz} cb={cb} flags={flags:#x} line={:#x} what={:#x} n={}: {hr} {:#x}", if wide { "W" } else { "A" }, h == opened.last().copied().unwrap_or(1), r32(blk + 4), r32(blk + 8), r32(blk + 12), hash(arr, 3 * (cs + 8)))
            }
            _ => {
                let set = which == 9 && r.below(2) == 0;
                let wide = r.below(2) == 0;
                let h = handle(&mut r, &opened);
                let (cb_at, pa_at, size) = details_at();
                w32(blk, r.pick(&[size, size, size, size + 4, 0]));
                w32(blk + 4, r.pick(&[0u32, 0, 1, 1, 2, 3, 4]));
                w32(blk + 8, r.pick(&[1u32, 2]));
                w32(blk + cb_at, r.pick(&[4u32, 4, 0, 8]));
                mem::wp(blk + pa_at, if r.below(10) == 0 { 0 } else { pa });
                let any = u32::try_from(r.next() & 0xffff).unwrap_or(0);
                let v = r.pick(&[0u32, 1, 0x8000, 0xffff, 0x10000, 5, any]);
                w32(pa, v);
                let flags = obj_flags(&mut r) | u32::try_from(r.below(3)).unwrap_or(0);
                let p = if r.below(10) == 0 { 0 } else { blk };
                let (name, hr) = if set {
                    ("set", call(mixerSetControlDetails, &[h, p, flags]))
                } else {
                    (if wide { "getW" } else { "getA" }, call(if wide { details_w } else { details_a }, &[h, p, flags]))
                };
                format!("{name} h={} id={} v={v:#x} flags={flags:#x}: {hr} {:#x} gain={:e}", h == opened.last().copied().unwrap_or(1), r32(blk + 4), r32(pa), SESSION.gain())
            }
        };
        writeln!(out, "{i} {line}").unwrap();
    }
    writeln!(out, "close {} message {}", call(mixerClose, &[0]), call(mixerMessage, &[0, 0, 0, 0])).unwrap();
    // (leave the session as a fresh process has it, for the tests after)
    SESSION.set(0, 0xffff).ok();
    SESSION.set(1, 0).ok();
    for p in [blk, arr, pa] {
        mem::heap_free(p);
    }
    snapshot("mixer", &out);
}
