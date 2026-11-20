//! XAudio2 pinned before its code changes shape: seeded sessions (2.7 and 2.8 engines) of
//! voice graphs (a mastering voice, submix voices, source voices of seeded formats, some
//! with callbacks, some with sake's XAPOs in their chains) driven through the guest methods
//! (SubmitSourceBuffer with seeded play and loop regions, Start / Stop, FlushSourceBuffers,
//! ExitLoop, SetFrequencyRatio, SetSourceSampleRate, SetVolume, SetOutputVoices, effect
//! enable / parameters, GetState, GetVoiceDetails, DestroyVoice, ...) and processing passes
//! run by hand (pass(): what the engine thread does each 10 ms). Every answer, each pass's
//! output and the callbacks it made are lines of golden/xaudio2.txt. The engine thread is
//! never started (the engine is marked as having one).
use super::*;
use crate::api::Handler;
use crate::test_guest::{call_stacked, enter, guest_thread, hash, snapshot, Rng};
use std::fmt::Write;

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
fn log(s: String) {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s);
}
fn on_pass_start(c: &mut Ctx) -> u32 {
    log(format!("start({})", c.a(1)));
    0
}
fn on_pass_end(_: &mut Ctx) -> u32 {
    log("end".into());
    0
}
fn on_stream_end(_: &mut Ctx) -> u32 {
    log("stream-end".into());
    0
}
fn on_buffer_start(c: &mut Ctx) -> u32 {
    log(format!("buf-start({})", c.a(1)));
    0
}
fn on_buffer_end(c: &mut Ctx) -> u32 {
    log(format!("buf-end({})", c.a(1)));
    0
}
fn on_loop_end(c: &mut Ctx) -> u32 {
    log(format!("loop-end({})", c.a(1)));
    0
}
fn on_error(_: &mut Ctx) -> u32 {
    log("error".into());
    0
}

/// An IXAudio2VoiceCallback whose methods log what they are called with
fn callback_object() -> u32 {
    static O: OnceLock<u32> = OnceLock::new();
    *O.get_or_init(|| {
        let fns: [(&str, Handler, u32); 7] = [
            ("xa2t_pass_start", on_pass_start, 8),
            ("xa2t_pass_end", on_pass_end, 4),
            ("xa2t_stream_end", on_stream_end, 4),
            ("xa2t_buffer_start", on_buffer_start, 8),
            ("xa2t_buffer_end", on_buffer_end, 8),
            ("xa2t_loop_end", on_loop_end, 8),
            ("xa2t_error", on_error, 12),
        ];
        let vt = mem::heap_alloc(4 * 7, true);
        for (at, (n, f, b)) in (vt..).step_by(4).zip(fns) {
            w32(at, host_fn(n, f, b));
        }
        let o = mem::heap_alloc(8, true);
        w32(o, vt);
        o
    })
}

static PASS_OUT: Mutex<u64> = Mutex::new(0);
fn run_pass(c: &mut Ctx) -> u32 {
    let mut out = vec![0f32; STEP * 2];
    pass(c, &mut out);
    let h = out.iter().flat_map(|x| x.to_bits().to_le_bytes()).fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    *PASS_OUT.lock().unwrap_or_else(|e| e.into_inner()) = h;
    0
}

fn take_log() -> String {
    std::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner())).join(" ")
}

/// A WAVEFORMATEX at a new block
fn wave(tag: u16, ch: u16, rate: u32, bits: u16, sub: u32) -> u32 {
    let f = mem::heap_alloc(40, true);
    w16(f, tag);
    w16(f + 2, ch);
    w32(f + 4, rate);
    let block = ch.wrapping_mul(bits / 8);
    w32(f + 8, rate.wrapping_mul(u32::from(block)));
    w16(f + 12, block);
    w16(f + 14, bits);
    w16(f + 16, 22);
    w32(f + 24, sub);
    f
}

/// XAUDIO2_VOICE_SENDS of one send to `to`
fn sends(to: u32) -> u32 {
    let d = mem::heap_alloc(8, true);
    w32(d + 4, to);
    let s = mem::heap_alloc(8, true);
    w32(s, 1);
    w32(s + 4, d);
    s
}

/// An XAUDIO2_EFFECT_CHAIN of sake's XAPOs of the given classes
fn chain(classes: &[[u8; 16]], on: &[bool]) -> u32 {
    let d = mem::heap_alloc(12 * u32::try_from(classes.len()).unwrap_or(0) + 4, true);
    for ((at, g), &e) in (d..).step_by(12).zip(classes).zip(on) {
        w32(at, crate::xapo::create(g).map_or(0, crate::xapo::Obj::addr));
        w32(at + 4, u32::from(e));
        w32(at + 8, 2);
    }
    let c = mem::heap_alloc(8, true);
    w32(c, u32::try_from(classes.len()).unwrap_or(0));
    w32(c + 4, d);
    c
}

use kome::guid::guid;

#[test]
fn xaudio2_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0x0a2d_10);
    let cb = callback_object();
    let po = mem::heap_alloc(64, true);
    let fx_classes = [
        guid(0xf5e0_1117, 0xd6c4, 0x485a, [0xa3, 0xf5, 0x69, 0x51, 0x96, 0xf3, 0xdb, 0xfa]),
        guid(0xc413_7916, 0x2be1, 0x46fd, [0x85, 0x99, 0x44, 0x15, 0x36, 0xf4, 0x98, 0x56]),
        guid(0x5039_d740, 0xf736, 0x449a, [0x84, 0xd3, 0xa5, 0x62, 0x02, 0x55, 0x7b, 0x87]),
        guid(0x4fc3_b166, 0x972a, 0x40cf, [0xbc, 0x37, 0x7d, 0xb0, 0x3d, 0xb2, 0xfb, 0xa3]),
        guid(0xc263_3b16, 0x471b, 0x4498, [0xb8, 0xc5, 0x4f, 0x09, 0x59, 0xe2, 0xec, 0x09]),
    ];
    // (2.7's classes are other GUIDs: their chains are refused, which is pinned too)
    for session in 0..40 {
        // a fresh engine, its thread marked as running so that none starts
        with(|e| *e = Engine { voices: HashMap::default(), master: 0, callbacks: Vec::new(), running: false, stream: -1, thread: true, v28: false, seq: 0 });
        let v28 = session % 2 == 0;
        let xa = if v28 {
            call_stacked(XAudio2Create, &[po, 0, 0]);
            rp(po)
        } else {
            let g = mem::heap_alloc(16, true);
            w32(g, 0x5a50_8685);
            let o = create_instance(g).unwrap_or(0);
            mem::heap_free(g);
            o
        };
        writeln!(out, "session {session} v28={v28}").unwrap();
        // every voice made, named by its place; a destroyed one's entry becomes 0, because its
        // address can come back for a new voice (and whether it does depends on the heap)
        let mut voices: Vec<u32> = Vec::new();
        let name = |v: u32, voices: &[u32]| voices.iter().position(|&x| x == v && x != 0).map_or_else(|| format!("{v:#x}"), |i| format!("v{i}"));
        // the mastering voice
        let mfx = if r.below(3) == 0 { chain(&fx_classes[3..4], &[true]) } else { 0 };
        let hr = call_stacked(X_CreateMasteringVoice, &[xa, po, 2, RATE, 0, 0, mfx, 0]);
        let mut master = rp(po);
        voices.push(master);
        writeln!(out, "  master: {hr:#x} {} (again: {:#x})", name(master, &voices), call_stacked(X_CreateMasteringVoice, &[xa, po, 2, RATE, 0, 0, 0, 0])).unwrap();
        let mut sources: Vec<u32> = Vec::new();
        let mut submixes: Vec<u32> = Vec::new();
        let mut datas: Vec<u32> = Vec::new();
        for step in 0..120 {
            let line = match r.below(16) {
                0 if submixes.len() < 3 => {
                    let to = if r.below(2) == 0 { submixes.last().copied().map_or(0, sends) } else { 0 };
                    let fx = if r.below(3) == 0 {
                        let k = usize::try_from(r.below(3)).unwrap_or(0);
                        chain(&fx_classes[k..k + 1], &[r.below(4) != 0])
                    } else {
                        0
                    };
                    let hr = call_stacked(X_CreateSubmixVoice, &[xa, po, 2, r.pick(&[0, RATE, 48000]), 0, 0, to, fx]);
                    let v = rp(po);
                    if hr == 0 {
                        voices.push(v);
                        submixes.push(v);
                    }
                    format!("submix (sends={} fx={}): {hr:#x} {}", to != 0, fx != 0, name(v, &voices))
                }
                1 | 2 if sources.len() < 6 => {
                    let (tag, sub) = r.pick(&[(1u16, 0u32), (3, 0), (0xfffe, 1), (0xfffe, 3), (2, 0), (0xfffe, 2)]);
                    let bits = r.pick(&[16u16, 16, 8, 24, 32, 12]);
                    let ch = r.pick(&[1u16, 2, 2, 6]);
                    let rate = r.pick(&[RATE, 22050, 48000, 8000, 0]);
                    let f = wave(tag, ch, rate, bits, sub);
                    let with_cb = r.below(3) != 0;
                    let to = if r.below(2) == 0 { submixes.last().copied().map_or(0, sends) } else { 0 };
                    let fx = if r.below(4) == 0 {
                        let k = usize::try_from(r.below(5)).unwrap_or(0);
                        chain(&fx_classes[k..k + 1], &[r.below(4) != 0])
                    } else {
                        0
                    };
                    let hr = call_stacked(X_CreateSourceVoice, &[xa, po, f, 0, 2.0f32.to_bits(), if with_cb { cb } else { 0 }, to, fx]);
                    mem::heap_free(f);
                    let v = rp(po);
                    if hr == 0 {
                        voices.push(v);
                        sources.push(v);
                    }
                    format!("source {tag:#x}/{sub}/{bits}b/{ch}ch/{rate} cb={with_cb} sends={} fx={}: {hr:#x} {}", to != 0, fx != 0, name(v, &voices))
                }
                3..=5 => {
                    let Some(&v) = sources.get(usize::try_from(r.below(6)).unwrap_or(0)) else { continue };
                    let frames = r.pick(&[0u32, 1, 100, 441, 2000, 9000]);
                    let block = with_voice(v, |x| x.fmt.block()).unwrap_or(4);
                    let bytes = frames * block;
                    let data = mem::heap_alloc(bytes + 4, true);
                    datas.push(data);
                    let raw = r.bytes(usize::try_from(bytes).unwrap_or(0));
                    mem::write(data, &raw);
                    let b = mem::heap_alloc(36, true);
                    let flags = r.pick(&[0u32, 0, END_OF_STREAM]);
                    let fields = [
                        flags,
                        if r.below(12) == 0 { bytes + 7 } else { bytes },
                        if r.below(15) == 0 { 0 } else { data },
                        r.pick(&[0u32, 0, 10, 5000]),
                        r.pick(&[0u32, 0, 50, 300, u32::MAX]),
                        r.pick(&[0u32, 20, 100]),
                        r.pick(&[0u32, 30, 80, 4000]),
                        r.pick(&[0u32, 0, 0, 1, 3, LOOP_INFINITE]),
                        1 + u32::try_from(r.below(1000)).unwrap_or(0),
                    ];
                    for (at, x) in (b..).step_by(4).zip(fields) {
                        w32(at, x);
                    }
                    let hr = call_stacked(S_SubmitSourceBuffer, &[v, b, 0]);
                    mem::heap_free(b);
                    // (the data pointer is a heap address: shown as whether it was given)
                    let mut shown = fields;
                    shown[2] = u32::from(fields[2] != 0);
                    format!("submit {} {shown:?}: {hr:#x} queued={:?}", name(v, &voices), with_voice(v, |x| x.queue.len()))
                }
                6 => {
                    let Some(&v) = sources.get(usize::try_from(r.below(6)).unwrap_or(0)) else { continue };
                    let start = r.below(3) != 0;
                    let hr = call_stacked(if start { S_Start } else { S_Stop }, &[v, 0, 0]);
                    format!("{} {}: {hr:#x}", if start { "start" } else { "stop" }, name(v, &voices))
                }
                7 => {
                    let Some(&v) = sources.get(usize::try_from(r.below(6)).unwrap_or(0)) else { continue };
                    let what = r.below(5);
                    let hr = match what {
                        0 => call_stacked(S_FlushSourceBuffers, &[v]),
                        1 => call_stacked(S_ExitLoop, &[v, 0]),
                        2 => call_stacked(S_SetFrequencyRatio, &[v, r.pick(&[0.5f32, 1.0, 2.0, 0.0, 5000.0, f32::NAN]).to_bits(), 0]),
                        3 => call_stacked(S_SetSourceSampleRate, &[v, r.pick(&[0u32, 22050, 48000])]),
                        _ => call_stacked(S_Discontinuity, &[v]),
                    };
                    format!("op{what} {}: {hr:#x} {}", name(v, &voices), take_log())
                }
                8 => {
                    let Some(&v) = voices.get(usize::try_from(r.below(8)).unwrap_or(0)).filter(|&&v| v != 0) else { continue };
                    let vol = r.pick(&[1.0f32, 0.5, 0.0, 2.0, f32::INFINITY]);
                    let a = call_stacked(V_SetVolume, &[v, vol.to_bits(), 0]);
                    let b = call_stacked(V_GetVolume, &[v, po]);
                    format!("volume {} {vol}: {a:#x} {b:#x} {:#x}", name(v, &voices), r32(po))
                }
                9 => {
                    let Some(&v) = sources.get(usize::try_from(r.below(6)).unwrap_or(0)) else { continue };
                    mem::fill(po, 32, 0);
                    let hr = if v28 && r.below(2) == 0 { call_stacked(S_GetState28, &[v, po, 0x100]) } else if v28 { call_stacked(S_GetState28, &[v, po, 0]) } else { call_stacked(S_GetState, &[v, po]) };
                    format!("state {}: {hr:#x} {:x?} ratio {:#x}", name(v, &voices), &mem::array::<16>(po), {
                        call_stacked(S_GetFrequencyRatio, &[v, po + 32]);
                        r32(po + 32)
                    })
                }
                10 => {
                    let Some(&v) = voices.get(usize::try_from(r.below(8)).unwrap_or(0)).filter(|&&v| v != 0) else { continue };
                    mem::fill(po, 64, 0);
                    let a = call_stacked(V_GetVoiceDetails, &[v, po]);
                    let b = call_stacked(V_GetFilterParameters, &[v, po + 16]);
                    let c = call_stacked(V_GetOutputMatrix, &[v, 0, 1, 2, po + 32]);
                    format!("details {}: {a:#x} {b:#x} {c:#x} {:x?}", name(v, &voices), (0..12).map(|k| r32(po + 4 * k)).collect::<Vec<_>>())
                }
                11 => {
                    let Some(&v) = voices.get(usize::try_from(r.below(8)).unwrap_or(0)).filter(|&&v| v != 0) else { continue };
                    let i = r.pick(&[0u32, 0, 1]);
                    let what = r.below(4);
                    let hr = match what {
                        0 => call_stacked(V_EnableEffect, &[v, i, 0]),
                        1 => call_stacked(V_DisableEffect, &[v, i, 0]),
                        2 => {
                            let p = mem::heap_alloc(64, true);
                            for (at, x) in (p..).step_by(4).zip([0.5f32, 0.4, 300.0, 1.0, 2.0, 1.0]) {
                                w32(at, x.to_bits());
                            }
                            let h = call_stacked(V_SetEffectParameters, &[v, i, p, 24, 0]);
                            mem::heap_free(p);
                            h
                        }
                        _ => {
                            let p = mem::heap_alloc(64, true);
                            let h = call_stacked(V_GetEffectParameters, &[v, i, p, 24]);
                            let s = hash(p, 24);
                            mem::heap_free(p);
                            log(format!("{s:#x}"));
                            h
                        }
                    };
                    mem::fill(po, 4, 0xff);
                    call_stacked(V_GetEffectState, &[v, i, po]);
                    format!("effect{what} {} #{i}: {hr:#x} state {:#x} {}", name(v, &voices), r32(po), take_log())
                }
                12 => {
                    let Some(&v) = sources.get(usize::try_from(r.below(6)).unwrap_or(0)) else { continue };
                    let to = if r.below(2) == 0 { submixes.last().copied().map_or(0, sends) } else { 0 };
                    format!("output voices {} sends={}: {:#x}", name(v, &voices), to != 0, call_stacked(V_SetOutputVoices, &[v, to]))
                }
                13 if r.below(3) == 0 => {
                    // DestroyVoice: a source, a submix (kept while sent to), or the master
                    let v = r.pick(&[sources.last().copied().unwrap_or(0), submixes.last().copied().unwrap_or(0), master]);
                    if v == 0 {
                        continue;
                    }
                    let before = with(|e| e.voices.contains_key(&v));
                    let hr = call_stacked(V_DestroyVoice, &[v]);
                    let gone = before && !with(|e| e.voices.contains_key(&v));
                    let shown = name(v, &voices);
                    if gone {
                        sources.retain(|&x| x != v);
                        submixes.retain(|&x| x != v);
                        voices.iter_mut().filter(|x| **x == v).for_each(|x| *x = 0);
                        if v == master {
                            master = 0;
                        }
                    }
                    format!("destroy {shown}: {hr:#x} gone={gone}")
                }
                _ => {
                    let n = r.below(4) + 1;
                    let mut s = String::new();
                    for _ in 0..n {
                        call_stacked(run_pass, &[]);
                        write!(s, " [{:#x} {}]", *PASS_OUT.lock().unwrap_or_else(|e| e.into_inner()), take_log()).unwrap();
                    }
                    format!("pass x{n}:{s}")
                }
            };
            writeln!(out, "  {step} {line}").unwrap();
        }
        // the end: Release (the graph goes)
        let n = call_stacked(X_AddRef, &[xa]);
        let a = call_stacked(X_Release, &[xa]);
        let b = call_stacked(X_Release, &[xa]);
        writeln!(out, "  refs {n} {a} {b}; voices left {}", with(|e| e.voices.len())).unwrap();
        for d in datas {
            mem::heap_free(d);
        }
    }
    with(|e| *e = Engine { voices: HashMap::default(), master: 0, callbacks: Vec::new(), running: false, stream: -1, thread: false, v28: false, seq: 0 });
    mem::heap_free(po);
    snapshot("xaudio2", &out);
}
