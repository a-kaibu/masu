//! amstream pinned before its code changes shape: `draw` over seeded frames, rectangles,
//! surfaces and pixel formats, `next` over seeded clocks and rates, and a seeded sweep of the
//! multimedia stream, its streams and samples (every method, purposes and IIDs right and
//! wrong, null outs, files that are no movie and none at all; DirectDraw objects and surfaces
//! stand-ins that log their AddRef / Release), each result and what it wrote a line of
//! golden/amstream_*.txt. Objects show by the order this test made them in.
use super::*;
use crate::api::host_fn;
use crate::test_guest::{call_stacked, enter, guest_thread, snapshot, Rng};
use std::fmt::Write;

#[test]
fn amstream_pure_snapshot() {
    let mut out = String::new();
    let mut r = Rng(0x0a_5d7a);
    let formats: [(u16, [u32; 3]); 6] = [(32, [0xff0000, 0xff00, 0xff]), (32, [0xff, 0xff00, 0xff0000]), (16, [0xf800, 0x7e0, 0x1f]), (16, [0x7c00, 0x3e0, 0x1f]), (24, [0xff0000, 0xff00, 0xff]), (16, [0, 0x7e0, 0])];
    for i in 0..1500 {
        let (w, h) = (r.pick(&[1u32, 2, 3, 7, 16]), r.pick(&[1u32, 2, 5, 9]));
        let px: Vec<u32> = (0..w * h).map(|_| u32::try_from(r.next() & 0xff_ffff).unwrap_or(0)).collect();
        let (sw, sh) = (r.pick(&[1i32, 4, 8, 13]), r.pick(&[1i32, 3, 8, 11]));
        let (bpp, masks) = r.pick(&formats);
        let bytes = usize::from(bpp / 8);
        let pitch = usize::try_from(sw).unwrap_or(0) * bytes + usize::try_from(r.below(3)).unwrap_or(0) * 4;
        let len = pitch * usize::try_from(sh).unwrap_or(0) - usize::try_from(r.below(2)).unwrap_or(0) * bytes;
        let c = |r: &mut Rng| r.pick(&[-3i32, 0, 0, 1, 2, 5, 8, 13, 20]);
        let dst = (c(&mut r), c(&mut r), c(&mut r), c(&mut r));
        let mut o = vec![0xccu8; len];
        draw(&px, w, h, &mut o, pitch, bpp, masks, dst, (sw, sh));
        let hsh = o.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
        writeln!(out, "draw#{i} {w}x{h} into {sw}x{sh} pitch={pitch} len={len} bpp={bpp} {masks:x?} dst={dst:?}: {hsh:#x}").unwrap();
    }
    for i in 0..1500 {
        let rate = (r.pick(&[0u32, 1, 15, 30, 30000, 24000]), r.pick(&[0u32, 1, 1, 1001]));
        let frames = r.pick(&[0u32, 1, 2, 100, 1000]);
        let pos = r.pick(&[0u64, 1, SEC / 30, SEC, 10 * SEC, 33 * SEC + 7, u64::MAX / 2]);
        let stop = r.pick(&[0u64, SEC, 4 * SEC, 40 * SEC, u64::MAX]);
        let have = if r.below(2) == 0 { None } else { Some(r.pick(&[0u32, 1, 29, 30, 99, 999])) };
        writeln!(out, "next#{i} pos={pos} stop={stop} rate={rate:?} frames={frames} have={have:?}: {:?}", next(pos, stop, rate, frames, have)).unwrap();
    }
    snapshot("amstream_pure", &out);
}

static LOG: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
fn log(s: String) {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s);
}
fn take_log() -> String {
    std::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner())).join(" ")
}
fn fake_addref(c: &mut Ctx) -> u32 {
    log(format!("addref({:#x})", r32(c.a(0) + 4)));
    2
}
fn fake_release(c: &mut Ctx) -> u32 {
    log(format!("release({:#x})", r32(c.a(0) + 4)));
    1
}
fn fake_qi(_: &mut Ctx) -> u32 {
    0x8000_4002
}
/// A stand-in COM object (IUnknown only) whose AddRef / Release log its tag
fn fake(tag: u32) -> u32 {
    static VT: OnceLock<u32> = OnceLock::new();
    let vt = *VT.get_or_init(|| {
        let v = mem::heap_alloc(12, true);
        w32(v, host_fn("amst_qi", fake_qi, 12));
        w32(v + 4, host_fn("amst_addref", fake_addref, 4));
        w32(v + 8, host_fn("amst_release", fake_release, 4));
        v
    });
    let o = mem::heap_alloc(8, true);
    w32(o, vt);
    w32(o + 4, tag);
    o
}

struct Names(Vec<u32>);
impl Names {
    fn add(&mut self, o: u32) -> String {
        self.0.push(o);
        self.of(o)
    }
    fn of(&self, o: u32) -> String {
        match o {
            0 => "0".into(),
            0xdead_beef => "untouched".into(),
            _ => self.0.iter().position(|&x| x == o).map_or_else(|| "?".into(), |i| format!("#{i}")),
        }
    }
}

#[test]
fn amstream_objects_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0x0a_5d7b);
    let (cls, iid, po, po2, po3, rect, desc, name) = (mem::heap_alloc(16, true), mem::heap_alloc(16, true), mem::heap_alloc(64, true), mem::heap_alloc(64, true), mem::heap_alloc(64, true), mem::heap_alloc(16, true), mem::heap_alloc(128, true), mem::heap_alloc(1024, true));
    let mspid = mem::heap_alloc(16, true);
    let dir = std::env::temp_dir().join(format!("masu-amstream-golden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notamovie.mpg"), b"not a movie").unwrap();
    let zdir = format!("Z:{}", dir.to_str().unwrap().replace('/', "\\"));
    let dds = [fake(0xdd1), fake(0xdd2)];
    let surfs = [fake(0x5f1), fake(0x5f2)];
    let mut names = Names(Vec::new());
    let (mut mmss, mut streams, mut samples): (Vec<u32>, Vec<u32>, Vec<u32>) = Default::default();
    let mut eos: Vec<u32> = Vec::new();
    let iid_words: [[u32; 4]; 7] = [[0, 0, 0xc0, 0x4600_0000], [MMS_IDS[0], 1, 2, 3], [MMS_IDS[1], 0, 0, 0], [DDSTREAM_IDS[1], 0, 0, 0], [AUDIOSTREAM_IDS[1], 0, 0, 0], [SAMPLE_IDS[0], 0, 0, 0], [0x1234, 0, 0, 0]];
    let pick = |r: &mut Rng, v: &[u32]| if v.is_empty() || r.below(30) == 0 { 0x0bad_0000 } else { v[usize::try_from(r.below(u64::try_from(v.len()).unwrap_or(1))).unwrap_or(0)] };
    for i in 0..3000 {
        for p in [po, po2, po3] {
            mem::fill(p, 64, 0);
            w32(p, 0xdead_beef);
        }
        let null = r.below(12) == 0;
        let o_p = if null { 0 } else { po };
        let k = usize::try_from(r.below(7)).unwrap_or(0);
        for (j, w) in iid_words[k].iter().enumerate() {
            w32(iid + 4 * u32::try_from(j).unwrap_or(0), *w);
        }
        // a purpose id: video, audio, or neither
        let pid = r.pick(&[VIDEO, AUDIO, AUDIO, 0xa35f_f56c]);
        w32(mspid, pid);
        mem::write(mspid + 4, &MSPID_TAIL);
        if r.below(15) == 0 {
            mem::w8(mspid + 15, 0);
        }
        let line = match r.below(26) {
            0 if mmss.len() < 6 => {
                w32(cls, 0x49c4_7ce5);
                w32(cls + 4, if r.below(10) == 0 { 0 } else { 0x11d0_9ba4 });
                let hr = create_instance(cls, if r.below(5) == 0 { 0 } else { iid });
                let shown = match hr {
                    Some(b) if b != 0 => {
                        mmss.push(b);
                        eos.push(mms(b, |x| x.eos).unwrap_or(0));
                        names.add(b)
                    }
                    _ => format!("{hr:?}"),
                };
                format!("create iid#{k}: {shown}")
            }
            1 => {
                let all: Vec<u32> = mmss.iter().chain(&streams).chain(&samples).copied().collect();
                let o = pick(&mut r, &all);
                let f: Handler = if mmss.contains(&o) { mms_qi } else if samples.contains(&o) { sample_qi } else if stream(o, |s| s.purpose) == Some(AUDIO) { audiostream_qi } else { ddstream_qi };
                let hr = call_stacked(f, &[o, if r.below(10) == 0 { 0 } else { iid }, o_p]);
                format!("qi {} iid#{k}: {hr:#x} {} refs={}", names.of(o), if rp(po) == o { "self".into() } else { names.of(rp(po)) }, refs(o, 0))
            }
            2 => {
                let all: Vec<u32> = mmss.iter().chain(&streams).chain(&samples).copied().collect();
                let o = pick(&mut r, &all);
                let add = r.below(2) == 0;
                let n = call_stacked(if add { add_ref } else { release }, &[o]);
                format!("{} {}: {n}", if add { "addref" } else { "release" }, names.of(o))
            }
            3 => {
                let m = pick(&mut r, &mmss);
                let a = call_stacked(mms_info, &[m, if null { 0 } else { po }, po2]);
                let b = call_stacked(mms_get_state, &[m, o_p]);
                let st = r32(po);
                let s = r.pick(&[0u32, 1, 1, 2]);
                let c = call_stacked(mms_set_state, &[m, s]);
                let d = call_stacked(mms_get_state, &[m, po3]);
                format!("info/state {}: {a:#x} {:#x} {:#x} {b:#x} {st:#x} set {s}: {c:#x} {d:#x} {:#x}", names.of(m), r32(po), r32(po2), r32(po3))
            }
            4 => {
                let m = pick(&mut r, &mmss);
                let a = call_stacked(mms_time, &[m, o_p]);
                let t = (r32(po), r32(po + 4));
                let b = call_stacked(mms_duration, &[m, if null { 0 } else { po2 }]);
                let to = r.pick(&[0u32, 1000, 0xffff_ffff]);
                let c = call_stacked(mms_seek, &[m, to, 0]);
                let d = call_stacked(mms_eos_handle, &[m, if null { 0 } else { po3 }]);
                let e = rp(po3);
                format!("time {}: {a:#x} {t:?} {b:#x} {:?} seek {to}: {c:#x} eos {d:#x} {}", names.of(m), (r32(po2), r32(po2 + 4)), if e == 0xdead_beef { "untouched".into() } else { eos.iter().position(|&x| x == e).map_or_else(|| format!("{e:#x}"), |j| format!("eos{j}")) })
            }
            5 => {
                let m = pick(&mut r, &mmss);
                let a = call_stacked(mms_get_null, &[m, o_p]);
                let g = if r.below(3) == 0 { dds[0] } else { 0 };
                let b = call_stacked(mms_initialize, &[m, 0, 0, g]);
                let log = take_log();
                format!("filter/init {} graph={}: {a:#x} {:#x} {b:#x} [{log}] graph={}", names.of(m), g != 0, rp(po), mms(m, |x| u32::from(x.graph != 0)).unwrap_or(9))
            }
            6 => {
                let m = pick(&mut r, &mmss);
                let obj = r.pick(&[0, dds[0], dds[1]]);
                let flags = r.pick(&[0u32, 0, AMMSF_ADDDEFAULTRENDERER, 2]);
                let p = if r.below(12) == 0 { 0 } else { mspid };
                let hr = call_stacked(mms_add_stream, &[m, obj, p, flags, o_p]);
                let s = rp(po);
                let shown = if hr == 0 && s != 0 && s != 0xdead_beef && !streams.contains(&s) {
                    streams.push(s);
                    names.add(s)
                } else {
                    names.of(s)
                };
                format!("add stream {} obj={} {pid:#x} flags={flags}: {hr:#x} {shown} [{}] sound={:?}", names.of(m), obj & 0xf, take_log(), mms(m, |x| x.sound))
            }
            7 => {
                let m = pick(&mut r, &mmss);
                let p = if r.below(12) == 0 { 0 } else { mspid };
                let a = call_stacked(mms_get_stream, &[m, p, o_p]);
                let s1 = rp(po);
                let idx = r.pick(&[0u32, 1, 2, u32::MAX]);
                let b = call_stacked(mms_enum, &[m, idx, if null { 0 } else { po2 }]);
                format!("get stream {} {pid:#x}: {a:#x} {} enum {idx}: {b:#x} {}", names.of(m), names.of(s1), names.of(rp(po2)))
            }
            8 => {
                let m = pick(&mut r, &mmss);
                let which = r.below(4);
                let n = match which {
                    0 => format!("{zdir}\\notamovie.mpg"),
                    1 => format!("{zdir}\\missing.mpg"),
                    2 => String::new(),
                    _ => format!("{zdir}\\notamovie"),
                };
                mem::put_wstr(name, 512, &n.encode_utf16().collect::<Vec<_>>());
                let flags = r.pick(&[0u32, 1, 2, 8, 9, 0xa]);
                let hr = call_stacked(mms_open_file, &[m, if r.below(12) == 0 { 0 } else { name }, flags]);
                format!("open file {} #{which} flags={flags:#x}: {hr:#x} {:?}", names.of(m), mms(m, |x| (x.open, x.playing, x.size, x.rate, x.duration, x.state, x.sound)))
            }
            9 | 10 => {
                let s = pick(&mut r, &streams);
                let a = call_stacked(st_get_mms, &[s, o_p]);
                let m = rp(po);
                let b = call_stacked(st_info, &[s, if null { 0 } else { po2 }, po3]);
                format!("stream {}: mms {a:#x} {} info {b:#x} {:#x} {:x?} {:#x}", names.of(s), names.of(m), r32(po2), &mem::array::<12>(po2 + 4), r32(po3))
            }
            11 | 12 => {
                let s = pick(&mut r, &streams);
                mem::fill(desc, 128, 0xcc);
                w32(desc, r.pick(&[108u32, 124, 0]));
                let a = call_stacked(dd_get_format, &[s, if null { 0 } else { desc }, po2, 0, po3]);
                let shown = format!("{:x?}", (0..27).map(|j| r32(desc + 4 * j)).collect::<Vec<_>>());
                // SetFormat of a pixel format, or a desc without one
                mem::fill(desc, 128, 0);
                w32(desc, 108);
                w32(desc + 4, r.pick(&[0x1000u32, 0]));
                w32(desc + 72, 32);
                w32(desc + 76, 0x40);
                w32(desc + 84, 16);
                w32(desc + 88, 0xf800);
                w32(desc + 92, 0x7e0);
                w32(desc + 96, 0x1f);
                let b = if r.below(2) == 0 { call_stacked(dd_set_format, &[s, if r.below(10) == 0 { 0 } else { desc }, 0]) } else { 0 };
                let c = call_stacked(dd_time_per_frame, &[s, if null { 0 } else { po }]);
                format!("format {}: {a:#x} {shown} {:#x} {:#x} set {b:#x} per frame {c:#x} {:?}", names.of(s), rp(po2), r32(po3), (r32(po), r32(po + 4)))
            }
            13 => {
                let s = pick(&mut r, &streams);
                let dd = r.pick(&[dds[0], dds[1], dds[0]]);
                let a = call_stacked(dd_set_dd, &[s, dd]);
                let log1 = take_log();
                let b = call_stacked(dd_get_dd, &[s, o_p]);
                let got = rp(po);
                format!("set/get dd {} {:#x}: {a:#x} [{log1}] {b:#x} [{}] {}", names.of(s), r32(dd + 4), take_log(), if got == dds[0] { "dd1" } else if got == dds[1] { "dd2" } else if got == 0xdead_beef { "untouched" } else { "?" })
            }
            14..=16 => {
                let s = pick(&mut r, &streams);
                let surf = r.pick(&surfs);
                let with_rect = r.below(2) == 0;
                for (j, v) in [r.pick(&[0i32, 5, -1]), r.pick(&[0i32, 3]), r.pick(&[640i32, 100, 2]), r.pick(&[480i32, 50, 1])].into_iter().enumerate() {
                    w32(rect + 4 * u32::try_from(j).unwrap_or(0), v.cast_unsigned());
                }
                let hr = call_stacked(dd_create_sample, &[s, surf, if with_rect { rect } else { 0 }, 0, o_p]);
                let o = rp(po);
                let shown = if hr == 0 && o != 0 && o_p != 0 {
                    samples.push(o);
                    names.add(o)
                } else {
                    names.of(o)
                };
                format!("create sample {} surf={:#x} rect={with_rect}: {hr:#x} {shown} [{}] {:?}", names.of(s), r32(surf + 4), take_log(), sample(o, |x| x.rect))
            }
            17 | 18 => {
                let b = pick(&mut r, &samples);
                let a = call_stacked(sm_get_stream, &[b, o_p]);
                let s = rp(po);
                let t = call_stacked(sm_times, &[b, po2, if null { 0 } else { po2 + 8 }, po3]);
                format!("sample {}: stream {a:#x} {} times {t:#x} {:?} {:?} {:?}", names.of(b), names.of(s), (r32(po2), r32(po2 + 4)), (r32(po2 + 8), r32(po2 + 12)), (r32(po3), r32(po3 + 4)))
            }
            19..=21 => {
                let b = pick(&mut r, &samples);
                let flags = r.pick(&[0u32, 0, SSUPDATE_ASYNC]);
                let ev = if r.below(3) == 0 { eos.first().copied().unwrap_or(0) } else { 0 };
                let a = call_stacked(sm_update, &[b, flags, ev, 0, 0]);
                let st = call_stacked(sm_status, &[b, 0, 0]);
                format!("update {} flags={flags} event={}: {a:#x} status {st:#x}", names.of(b), ev != 0)
            }
            22 | 23 => {
                let b = pick(&mut r, &samples);
                let a = call_stacked(sm_surface, &[b, if null { 0 } else { po }, po2]);
                let got = rp(po);
                let rc = (0..4).map(|j| r32(po2 + 4 * j).cast_signed()).collect::<Vec<_>>();
                for (j, v) in [1i32, 2, 30, 40].into_iter().enumerate() {
                    w32(rect + 4 * u32::try_from(j).unwrap_or(0), v.cast_unsigned());
                }
                let c = if r.below(2) == 0 { call_stacked(sm_set_rect, &[b, if r.below(10) == 0 { 0 } else { rect }]) } else { 0 };
                format!("surface {}: {a:#x} {} {rc:?} [{}] set rect {c:#x} {:?}", names.of(b), if got == surfs[0] { "s1" } else if got == surfs[1] { "s2" } else if got == 0xdead_beef { "untouched" } else { "?" }, take_log(), sample(b, |x| x.rect))
            }
            _ => {
                let m = pick(&mut r, &mmss);
                let v = [call_stacked(notimpl, &[m]), call_stacked(ok, &[m])];
                format!("misc {}: {v:x?} ended={}", names.of(m), ended(m))
            }
        };
        writeln!(out, "{i} {line}").unwrap();
    }
    std::fs::remove_dir_all(&dir).ok();
    snapshot("amstream_objects", &out);
}
