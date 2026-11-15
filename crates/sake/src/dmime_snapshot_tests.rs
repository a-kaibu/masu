//! dmime pinned before its code changes shape: a seeded sweep over the performance, its audio
//! paths, the loader (wave images in memory and files, cache on and off, search directories),
//! segments, clones and segment states (every method, IIDs right and wrong, null outs, flags
//! and volumes in and out of range), each result and what it wrote a line of golden/dmime.txt.
//! The mixer thread is held, so what plays stays where it started; times the clock gives
//! (GetTime, a state's start) show as their results only. Objects show by the order this test
//! made them in.
use super::*;
use crate::mem::{r32, r64, rp, w32};
use crate::test_guest::{call, enter, guest_thread, snapshot, Rng};
use std::fmt::Write;
use std::sync::atomic::Ordering::SeqCst;

struct Names(Vec<u32>);
impl Names {
    fn add(&mut self, o: u32) -> String {
        self.0.push(o);
        self.of(o)
    }
    fn drop_(&mut self, o: u32) {
        self.0.iter_mut().filter(|x| **x == o).for_each(|x| *x = 0);
    }
    fn of(&self, o: u32) -> String {
        if o == 0 {
            return "0".into();
        }
        if o == 0xdead_beef {
            return "untouched".into();
        }
        self.0.iter().position(|&x| x == o).map_or_else(|| "?".into(), |i| format!("#{i}"))
    }
}

struct Held;
impl Drop for Held {
    fn drop(&mut self) {
        crate::sound::MIXER_HELD.store(false, SeqCst);
    }
}

/// A RIFF WAVE image: PCM of seeded format and frames, or (sometimes) something else
fn image(r: &mut Rng) -> Vec<u8> {
    let (ch, bits, rate) = (r.pick(&[1u16, 2]), r.pick(&[8u16, 16, 16]), r.pick(&[22050u32, 44100, 11025]));
    let block = u32::from(ch * bits / 8);
    let data = block * u32::try_from(r.below(3000)).unwrap_or(0);
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&r.pick(&[1u16, 1, 1, 2]).to_le_bytes());
    b.extend_from_slice(&ch.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * block).to_le_bytes());
    b.extend_from_slice(&u16::try_from(block).unwrap_or(0).to_le_bytes());
    b.extend_from_slice(&bits.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    b.extend(r.bytes(usize::try_from(data).unwrap_or(0)));
    match r.below(10) {
        0 => b"MThd\0\0\0\x06\0\0\0\x01\0\x60".to_vec(),
        1 => b[..20].to_vec(),
        _ => b,
    }
}

#[test]
fn dmime_snapshot() {
    let _g = enter();
    guest_thread();
    crate::sound::MIXER_HELD.store(true, SeqCst);
    let _held = Held;
    let mut out = String::new();
    let mut r = Rng(0x0d_313e);
    let (cls, iid, po, po2, desc, gp) = (mem::heap_alloc(16, true), mem::heap_alloc(16, true), mem::heap_alloc(64, true), mem::heap_alloc(64, true), mem::heap_alloc(1024, true), mem::heap_alloc(16, true));
    let wpath = mem::heap_alloc(1024, true);
    let mut names = Names(Vec::new());
    let (mut perfs, mut loaders, mut segs, mut states, mut paths): (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>) = Default::default();
    let mut blobs: Vec<u32> = Vec::new();
    // files for LoadObjectFromFile / a file name in a descriptor
    let dir = std::env::temp_dir().join(format!("masu-dmime-golden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut fr = Rng(0x0f11e);
    for k in 0..4 {
        std::fs::write(dir.join(format!("w{k}.wav")), image(&mut fr)).unwrap();
    }
    let zdir = format!("Z:{}", dir.to_str().unwrap().replace('/', "\\"));
    let mut piid = PERF_IIDS[2];
    piid[15] ^= 1;
    let iids = [IID_UNKNOWN, PERF_IIDS[0], PERF_IIDS[2], LOADER_IIDS[1], SEGMENT_IIDS[0], SEGMENT_IIDS[2], STATE_IIDS[1], AUDIOPATH_IIDS[0], piid];
    let pick = |r: &mut Rng, v: &[u32]| if v.is_empty() || r.below(30) == 0 { 0x0bad_0000 } else { v[usize::try_from(r.below(u64::try_from(v.len()).unwrap_or(1))).unwrap_or(0)] };
    for i in 0..4000 {
        mem::fill(po, 64, 0);
        mem::fill(po2, 64, 0);
        w32(po, 0xdead_beef);
        let null = r.below(14) == 0;
        let o_p = if null { 0 } else { po };
        let k = usize::try_from(r.below(u64::try_from(iids.len()).unwrap_or(1))).unwrap_or(0);
        mem::write(iid, &iids[k]);
        let line = match r.below(30) {
            0 | 1 => {
                let c = r.pick(&[CLSID_PERFORMANCE, CLSID_LOADER, CLSID_SEGMENT, CLSID_SEGMENT_STATE]);
                mem::write(cls, &c);
                let outer = if r.below(15) == 0 { 0x1234 } else { 0 };
                let hr = create_instance(cls, outer, if r.below(15) == 0 { 0 } else { iid }, crate::api::Out::at(o_p));
                let o = rp(po);
                let shown = if hr == Some(0) {
                    match kind_of(o) {
                        Some(Kind::Performance) => perfs.push(o),
                        Some(Kind::Loader) => loaders.push(o),
                        _ => segs.push(o),
                    }
                    names.add(o)
                } else {
                    names.of(o)
                };
                format!("create {:x?} iid#{k} outer={outer:#x} out={}: {hr:x?} {shown}", &c[..4], !null)
            }
            2 => {
                let all: Vec<u32> = perfs.iter().chain(&loaders).chain(&segs).chain(&states).chain(&paths).copied().collect();
                let o = pick(&mut r, &all);
                let hr = call(qi, &[o, iid, o_p]);
                format!("qi {} iid#{k}: {hr:#x} {}", names.of(o), if rp(po) == o { "self".into() } else { names.of(rp(po)) })
            }
            3 => {
                let all: Vec<u32> = perfs.iter().chain(&loaders).chain(&segs).chain(&states).chain(&paths).copied().collect();
                let o = pick(&mut r, &all);
                if r.below(2) == 0 {
                    format!("addref {}: {}", names.of(o), call(add_ref, &[o]))
                } else {
                    let n = call(release, &[o]);
                    let shown = names.of(o);
                    if n == 0 && o != 0x0bad_0000 {
                        for v in [&mut perfs, &mut loaders, &mut segs, &mut states, &mut paths] {
                            v.retain(|&x| x != o);
                        }
                        names.drop_(o);
                    }
                    format!("release {shown}: {n}")
                }
            }
            4 => {
                let p = pick(&mut r, &perfs);
                let audio = r.below(2) == 0;
                w32(po, 0);
                w32(po2, if r.below(3) == 0 { 0x77 } else { 0 });
                let ty = r.pick(&[0u32, 1, 2, 7]);
                let hr = if audio { call(p_init_audio, &[p, if null { 0 } else { po }, po2, 0, ty, 64, 0, 0]) } else { call(p_init, &[p, if null { 0 } else { po }, 0, 0]) };
                let path = with(|w| w.perfs.get(&p).map(|x| x.path)).unwrap_or(0);
                let shown = if path != 0 && !paths.contains(&path) {
                    paths.push(path);
                    names.add(path)
                } else {
                    names.of(path)
                };
                format!("init{} {} type={ty}: {hr:#x} dm={} ds={:#x} path={shown}", if audio { "audio" } else { "" }, names.of(p), rp(po) != 0, if rp(po2) == 0x77 { 0x77 } else { u32::from(rp(po2) != 0) })
            }
            5 => {
                let p = pick(&mut r, &perfs);
                let std = r.below(2) == 0;
                let hr = if std { call(p_create_std_path, &[p, 7, 64, 1, o_p]) } else { call(p_create_audio_path, &[p, 0, 1, o_p]) };
                let o = rp(po);
                let shown = if hr == 0 && o_p != 0 {
                    paths.push(o);
                    names.add(o)
                } else {
                    names.of(o)
                };
                format!("create path std={std} {}: {hr:#x} {shown}", names.of(p))
            }
            6 => {
                let p = pick(&mut r, &perfs);
                let path = if r.below(4) == 0 { 0 } else { pick(&mut r, &paths) };
                let a = call(p_set_default_path, &[p, path]);
                let b = call(p_get_default_path, &[p, o_p]);
                format!("default path {} {}: {a:#x} {b:#x} {}", names.of(p), names.of(path), names.of(rp(po)))
            }
            7..=9 => {
                let p = pick(&mut r, &perfs);
                let s = if r.below(8) == 0 { pick(&mut r, &paths) } else { pick(&mut r, &segs) };
                let flags = r.pick(&[0u32, 0x80, 0x80, 0x100, 0x82]);
                let ex = r.below(2) == 0;
                let path = if r.below(3) == 0 { pick(&mut r, &paths) } else { 0 };
                let hr = if ex { call(p_play_segment_ex, &[p, s, 0, 0, flags, 0, 0, o_p, 0, path]) } else { call(p_play_segment, &[p, s, flags, 0, 0, o_p]) };
                let o = rp(po);
                let shown = if hr == 0 && o_p != 0 {
                    states.push(o);
                    format!("{} playing={} pos={:?}", names.add(o), crate::sound::music_playing(o), crate::sound::music_pos(o))
                } else {
                    names.of(o)
                };
                let left: Vec<String> = with(|w| w.perfs.get(&p).map(|x| x.plays.states.iter().map(|s| (s.0, s.1.primary, s.1.path)).collect::<Vec<_>>())).unwrap_or_default().into_iter().map(|(k, pr, pa)| format!("{}{}{}", names.of(k), if pr { "p" } else { "s" }, names.of(pa))).collect();
                format!("play ex={ex} {} {} flags={flags:#x} path={}: {hr:#x} {shown} states {left:?}", names.of(p), names.of(s), names.of(path))
            }
            10 => {
                let p = pick(&mut r, &perfs);
                let which = r.below(4);
                let (a, b) = match which {
                    0 => (0, 0),
                    1 => (pick(&mut r, &segs), 0),
                    2 => (0, pick(&mut r, &states)),
                    _ => (pick(&mut r, &paths), 0),
                };
                let ex = r.below(2) == 0;
                let hr = if ex { call(p_stop_ex, &[p, if b != 0 { b } else { a }, 0, 0, 0]) } else { call(p_stop, &[p, a, b, 0, 0]) };
                format!("stop ex={ex} {} {} {}: {hr:#x} left {}", names.of(p), names.of(a), names.of(b), with(|w| w.perfs.get(&p).map_or(0, |x| x.plays.states.len())))
            }
            11 => {
                let p = pick(&mut r, &perfs);
                let (s, st) = (if r.below(2) == 0 { pick(&mut r, &segs) } else { 0 }, if r.below(2) == 0 { pick(&mut r, &states) } else { 0 });
                let a = call(p_is_playing, &[p, s, st]);
                let b = call(p_get_segment_state, &[p, o_p, 0]);
                format!("is playing {} {} {}: {a:#x}; segment state: {b:#x} {}", names.of(p), names.of(s), names.of(st), names.of(rp(po)))
            }
            12 => {
                let p = pick(&mut r, &perfs);
                let mt = r.pick(&[0u32, 1, 1536, 0x7fff_ffff, 0x8000_0000, u32::MAX]);
                let a = call(p_music_to_ref, &[p, mt, o_p]);
                let ra = r64(po);
                let rt = r.pick(&[0u64, 1, 10_000_000, u64::MAX, 1 << 40]);
                let lo = u32::try_from(rt & 0xffff_ffff).unwrap_or(0);
                let hi = u32::try_from(rt >> 32).unwrap_or(0);
                let b = call(p_ref_to_music, &[p, lo, hi, o_p]);
                let rb = r32(po);
                let c = call(p_resolved, &[p, lo, hi, if null { 0 } else { po2 }, 0]);
                let d = call(p_get_time, &[p, if null { 0 } else { po }, 0]);
                let e = call(p_get_rt_now, &[p, o_p]);
                format!("times {} mt={mt} rt={rt}: {a:#x} {ra} {b:#x} {rb} {c:#x} {} {d:#x} {e:#x}", names.of(p), r64(po2))
            }
            13 => {
                let p = pick(&mut r, &perfs);
                let g = r.pick(&[GUID_PERF_MASTER_VOLUME, GUID_PERF_MASTER_VOLUME, GUID_PERF_MASTER_TEMPO, GUID_ALL_TYPES]);
                mem::write(gp, &g);
                let v = r.pick(&[0i32, -100, -5000, -10000, -20000, 500]);
                let size = r.pick(&[4u32, 4, 2, 8]);
                w32(po2, v.cast_unsigned());
                let set = r.below(2) == 0;
                let a = if set { call(p_set_global, &[p, gp, if null { 0 } else { po2 }, size]) } else { 0 };
                let b = call(p_get_global, &[p, gp, o_p, size]);
                format!("global {:x?} set={set} {v} size={size}: {a:#x} {b:#x} {:#x}", &g[..4], r32(po))
            }
            14 => {
                let p = pick(&mut r, &perfs);
                let v = [call(p_get_dword_1000, &[p, o_p]), r32(po), call(p_get_dword_50, &[p, o_p]), r32(po), call(not_found_out, &[p, o_p]), call(s_false_out, &[p, o_p]), call(not_found, &[p]), call(not_impl, &[p]), call(ok, &[p])];
                format!("perf misc {}: {v:x?}", names.of(p))
            }
            15 if r.below(4) == 0 => {
                let p = pick(&mut r, &perfs);
                let hr = call(p_close_down, &[p]);
                format!("close down {}: {hr:#x} left {}", names.of(p), with(|w| w.perfs.get(&p).map_or(0, |x| x.plays.states.len())))
            }
            16 => {
                let path = pick(&mut r, &paths);
                let v = r.pick(&[0u32, -100i32 as u32, -9600i32 as u32, -9601i32 as u32, 1]);
                let a = call(path_volume, &[path, v, 0]);
                let b = call(path_convert, &[path, 5, o_p]);
                let cv = r32(po);
                let c = call(path_object, &[path, 0, 0, 0, 0, 0, 0, o_p]);
                format!("path {} vol={}: {a:#x} {b:#x} {cv} {c:#x} {}", names.of(path), v.cast_signed(), names.of(rp(po)))
            }
            17 | 18 => {
                let s = pick(&mut r, &segs);
                let v = r.pick(&[0u32, 1, 100, 1536, u32::MAX]);
                let which = r.below(6);
                let (a, b) = match which {
                    0 => (call(seg_set_length, &[s, v]), call(seg_get_length, &[s, o_p])),
                    1 => (call(seg_set_repeats, &[s, v]), call(seg_get_repeats, &[s, o_p])),
                    2 => (call(seg_set_start, &[s, v]), call(seg_get_start, &[s, o_p])),
                    3 => (call(seg_set_loop, &[s, v, v / 2]), call(seg_get_loop, &[s, o_p, if null { 0 } else { po + 4 }])),
                    4 => (call(seg_get_zero, &[s, o_p]), call(ok, &[s])),
                    _ => (call(not_found, &[s]), call(not_found_out, &[s, o_p])),
                };
                format!("segment {} op{which} {v}: {a:#x} {b:#x} {:#x} {:#x}", names.of(s), r32(po), r32(po + 4))
            }
            19 => {
                let s = pick(&mut r, &segs);
                let hr = call(seg_clone, &[s, 0, 0, o_p]);
                let o = rp(po);
                let shown = if hr == 0 && o_p != 0 {
                    segs.push(o);
                    names.add(o)
                } else {
                    names.of(o)
                };
                format!("clone {}: {hr:#x} {shown} refs={}", names.of(s), r32(s + mem::ps()))
            }
            20 | 21 => {
                let st = pick(&mut r, &states);
                let which = r.below(5);
                let hr = match which {
                    0 => call(st_get_repeats, &[st, o_p]),
                    1 => call(st_get_segment, &[st, o_p]),
                    2 => call(st_get_start, &[st, o_p]),
                    3 => call(st_get_seek, &[st, o_p]),
                    _ => call(path_object, &[st, 0, 0, 0, 0, 0, 0, o_p]),
                };
                let v = match which {
                    1 | 4 => names.of(rp(po)),
                    // (the clock's)
                    2 => "-".into(),
                    _ => format!("{:#x}", r32(po)),
                };
                format!("state {} op{which}: {hr:#x} {v} playing={}", names.of(st), crate::sound::music_playing(st))
            }
            22..=24 => {
                // the loader's GetObject of an image in memory, or a file by name
                let l = pick(&mut r, &loaders);
                mem::fill(desc, 1024, 0);
                let memory = r.below(3) != 0;
                let mut valid = if memory { 0x400 } else { 0x10 };
                if r.below(3) == 0 {
                    valid |= 2;
                    mem::write(desc + 24, &r.pick(&[CLSID_SEGMENT, GUID_ALL_TYPES, CLSID_LOADER]));
                }
                if r.below(8) == 0 {
                    valid = r.pick(&[0u32, 0x20]);
                }
                w32(desc + 4, valid);
                let shown_src;
                if memory {
                    let img = image(&mut r);
                    let b = mem::heap_alloc(u32::try_from(img.len()).unwrap_or(0) + 4, true);
                    mem::write(b, &img);
                    blobs.push(b);
                    let len = if r.below(12) == 0 { 0 } else { u64::try_from(img.len()).unwrap_or(0) };
                    mem::w64(desc + 832, len);
                    w32(desc + 840, if r.below(12) == 0 { 0 } else { b });
                    shown_src = format!("{} bytes", img.len());
                } else {
                    let full = r.below(2) == 0;
                    let k = r.below(5);
                    let name = if full { format!("{zdir}\\w{k}.wav") } else { format!("w{k}.wav") };
                    if full {
                        w32(desc + 4, valid | 0x20);
                    }
                    mem::put_wstr(desc + 312, 260, &name.encode_utf16().collect::<Vec<_>>());
                    shown_src = format!("w{k} full={full}");
                }
                let hr = call(l_get_object, &[l, if r.below(20) == 0 { 0 } else { desc }, if r.below(5) == 0 { 0 } else { iid }, o_p]);
                let o = rp(po);
                let shown = if hr == 0 && o_p != 0 {
                    if segs.contains(&o) {
                        names.of(o)
                    } else {
                        segs.push(o);
                        names.add(o)
                    }
                } else {
                    names.of(o)
                };
                let sound = with(|w| w.segments.get(&o).map(|s| s.sound.map(|x| (x.size, x.rate, x.channels, x.bits, x.float, s.length))));
                format!("get object {} valid={:#x} {shown_src} iid#{k}: {hr:#x} {shown} {sound:?} refs={}", names.of(l), r32(desc + 4), if hr == 0 { r32(o + mem::ps()) } else { 0 })
            }
            25 => {
                let l = pick(&mut r, &loaders);
                let k = r.below(5);
                let name = r.pick(&["", "dir"]);
                let path = if name.is_empty() { format!("w{k}.wav") } else { format!("{zdir}\\w{k}.wav") };
                mem::put_wstr(wpath, 512, &path.encode_utf16().collect::<Vec<_>>());
                mem::write(cls, &r.pick(&[CLSID_SEGMENT, CLSID_SEGMENT, CLSID_LOADER]));
                let hr = call(l_load_from_file, &[l, if r.below(6) == 0 { 0 } else { cls }, if r.below(5) == 0 { 0 } else { iid }, if r.below(12) == 0 { 0 } else { wpath }, o_p]);
                let o = rp(po);
                let shown = if hr == 0 && o_p != 0 {
                    if segs.contains(&o) {
                        names.of(o)
                    } else {
                        segs.push(o);
                        names.add(o)
                    }
                } else {
                    names.of(o)
                };
                format!("load file {} w{k} abs={}: {hr:#x} {shown}", names.of(l), !name.is_empty())
            }
            26 => {
                let l = pick(&mut r, &loaders);
                let to_dir = r.below(2) == 0;
                mem::put_wstr(wpath, 512, &(if to_dir { zdir.clone() } else { String::new() }).encode_utf16().collect::<Vec<_>>());
                let a = call(l_set_search_dir, &[l, 0, if r.below(10) == 0 { 0 } else { wpath }, 0]);
                let on = r.below(3) != 0;
                let b = call(l_enable_cache, &[l, 0, u32::from(on)]);
                format!("search dir {} dir={to_dir} cache={on}: {a:#x} {b:#x}", names.of(l))
            }
            27 => {
                let l = pick(&mut r, &loaders);
                let s = pick(&mut r, &segs);
                let clear = r.below(3) == 0;
                let hr = if clear { call(l_clear_cache, &[l, 0]) } else { call(l_release_object, &[l, s]) };
                format!("{} {} {}: {hr:#x} cached {}", if clear { "clear cache" } else { "release object" }, names.of(l), names.of(s), with(|w| w.loaders.get(&l).map_or(0, |x| x.2.len())))
            }
            _ => {
                let l = pick(&mut r, &loaders);
                format!("loader misc {}: {:#x} {:#x} {:#x}", names.of(l), call(s_false3, &[l, 0, 0, 0]), call(ok, &[l]), call(not_impl, &[l]))
            }
        };
        writeln!(out, "{i} {line}").unwrap();
        // (an object whose last reference another object gave back is retired: off the lists
        // before its block can be freed, 256 frees later, and come back as a new object)
        for v in [&mut perfs, &mut loaders, &mut segs, &mut states, &mut paths] {
            for &o in v.iter() {
                if kind_of(o).is_none() {
                    names.drop_(o);
                }
            }
            v.retain(|&o| kind_of(o).is_some());
        }
    }
    for p in perfs.clone() {
        call(p_close_down, &[p]);
    }
    for o in perfs.into_iter().chain(loaders).chain(states).chain(paths).chain(segs) {
        while kind_of(o).is_some() && call(release, &[o]) > 0 {}
    }
    for p in blobs.into_iter().chain([cls, iid, po, po2, desc, gp, wpath]) {
        mem::heap_free(p);
    }
    std::fs::remove_dir_all(&dir).ok();
    snapshot("dmime", &out);
}
