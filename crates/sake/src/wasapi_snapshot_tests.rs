//! WASAPI pinned before its code changes shape:
//! - the shared-mode format rules (check, shared_initialize, shared_supported) and
//!   buffer_frames over seeded formats, flags and durations;
//! - render() on seeded queues, rates and gains (what a period takes and gives);
//! - a seeded sweep over the guest interfaces: IAudioClient (Initialize, the getters,
//!   IsFormatSupported, Start/Stop/Reset, SetEventHandle, GetService of every service), the
//!   render client's GetBuffer/ReleaseBuffer, the clock, stream / session / channel volumes,
//!   the session control, the session manager and its enumerator, the endpoint volume.
//!
//! The engine thread mixes running clients, so the sweep starts a client only while it has
//! nothing queued and queues nothing while it runs, and gives no client an event it would
//! signal: what each call answers is then the same on every run. Each answer is a line of
//! golden/wasapi.txt; objects show by kind, sessions as counted from the first this test saw.
use super::*;
use crate::test_guest::{call, enter, guest_thread, hash, snapshot, Rng};
use std::fmt::Write;

fn put_wave(p: u32, w: &Wave, mask: u32, base: bool) {
    mem::fill(p, 48, 0);
    w16(p, w.tag);
    w16(p + 2, w.channels);
    w32(p + 4, w.rate);
    w32(p + 8, w.avg);
    w16(p + 12, w.block);
    w16(p + 14, w.bits);
    w16(p + 16, w.cb);
    w16(p + 18, w.valid);
    w32(p + 20, mask);
    w32(p + 24, w.sub.unwrap_or(1));
    if base {
        w16(p + 28, 0);
        w16(p + 30, 0x10);
        mem::write(p + 32, &[0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71]);
    }
}

fn seeded_wave(r: &mut Rng) -> Wave {
    let tag = r.pick(&[1u16, 3, 0xfffe, 0xfffe, 2, 6]);
    let channels = r.pick(&[1u16, 2, 2, 2, 0, 3, 6, 8, 32, 33]);
    let rate = r.pick(&[RATE, RATE, 48000, 22050, 8000, 0, 192_000]);
    let bits = r.pick(&[8u16, 16, 16, 24, 32, 32, 64, 12, 0]);
    let block = if r.below(8) == 0 { r.pick(&[0u16, 1, 3, 7]) } else { channels.wrapping_mul(bits) / 8 };
    let avg = if r.below(8) == 0 { r.pick(&[0u32, 1, 12345]) } else { u32::from(block).wrapping_mul(rate) };
    let cb = if tag == 0xfffe { r.pick(&[22u16, 22, 22, 21, 0, 30]) } else { r.pick(&[0u16, 0, 22]) };
    let valid = r.pick(&[bits, bits, bits, 24, 20, 0, 33]);
    let sub = if tag == 0xfffe { r.pick(&[Some(1u32), Some(3), Some(3), Some(2), None]) } else { None };
    Wave { tag, channels, rate, avg, block, bits, cb, valid: if tag == 0xfffe { valid } else { 0 }, sub }
}

fn res(r: Result<Fmt, Hr>) -> String {
    match r {
        Ok(f) => format!("ok({},{},{},{})", f.rate, f.channels, f.bits, f.float),
        Err(e) => format!("{:#x}", e.code()),
    }
}

fn wave_text(w: &Wave) -> String {
    format!("{:#x}/{}ch/{}/{}b/blk{}/avg{}/cb{}/v{}/{:?}", w.tag, w.channels, w.rate, w.bits, w.block, w.avg, w.cb, w.valid, w.sub)
}

#[test]
fn wasapi_formats_snapshot() {
    let mut out = String::new();
    let mut r = Rng(0xa5a_91);
    let flag_sets = [Flags(0), RATEADJUST, AUTOCONVERTPCM, Flags(AUTOCONVERTPCM.0 | 0x800_0000), EVENTCALLBACK];
    for _ in 0..3000 {
        let w = seeded_wave(&mut r);
        let inits: Vec<String> = flag_sets.iter().map(|&f| res(shared_initialize(&w, f))).collect();
        writeln!(out, "{}: check {} / {}; init {}; supported {:#x}", wave_text(&w), res(check(&w, true)), res(check(&w, false)), inits.join(" "), hr(shared_supported(&w))).unwrap();
    }
    for _ in 0..500 {
        let rate = r.pick(&[RATE, 48000, 8000, 1, 0, 384_000, u32::MAX]);
        let any = r.next() % 1_000_000_000;
        let d = r.pick(&[0u64, 1, 30_000, 100_000, 200_000, 200_001, 333_333, 10_000_000, MAX_DURATION, any]);
        writeln!(out, "buffer_frames({rate}, {d}) = {}", buffer_frames(rate, d)).unwrap();
    }
    snapshot("wasapi_formats", &out);
}

fn bare_client(rate: u32, channels: u32) -> Client {
    Client {
        refs: 1,
        fmt: Some(Fmt { rate, channels, bits: 32, float: true, law: 0 }),
        flags: Flags::default(),
        frames: 0,
        event: 0,
        running: true,
        started: true,
        queue: VecDeque::new(),
        frac: 0.0,
        rate: 0.0,
        played: 0,
        taken: None,
        mem: 0,
        channel: [1.0; MAX_CHANNELS],
        session: 0,
        ctl: 0,
    }
}

#[test]
fn wasapi_render_snapshot() {
    let mut out = String::new();
    let mut r = Rng(0x7e4d_e7);
    for s in 0..300 {
        let mut c = bare_client(r.pick(&[RATE, 48000, 22050, 8000, 96000]), r.pick(&[1, 2]));
        if r.below(3) == 0 {
            c.rate = r.pick(&[44000.0f32, 47999.5, 11025.0, 0.0]);
        }
        let n = usize::try_from(r.below(3000)).unwrap_or(0);
        c.queue.extend((0..n).map(|i| {
            let x = f32::from(u16::try_from(i % 1000).unwrap_or(0)) / 1000.0;
            [x, -x]
        }));
        let gain = [r.pick(&[1.0f32, 0.5, 0.0, 0.25]), r.pick(&[1.0f32, 0.75, 0.0])];
        let mut line = format!("{s} rate={} adj={} queued={n} gain={gain:?}:", c.fmt.map_or(0, |f| f.rate), c.rate);
        for _ in 0..r.below(6) + 1 {
            let mut buf = vec![0f32; STEP * 2];
            let dry = render(&mut c, gain, &mut buf);
            let h = buf.iter().flat_map(|x| x.to_bits().to_le_bytes()).fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
            write!(line, " [{dry} played={} left={} frac={:e} {h:#x}]", c.played, c.queue.len(), c.frac).unwrap();
        }
        writeln!(out, "{line}").unwrap();
    }
    snapshot("wasapi_render", &out);
}

/// A guest IID block for `g` (the rest of the GUID as Windows' is irrelevant: sake looks at
/// the first three fields)
fn iid_at(p: u32, g: Iid) -> u32 {
    mem::fill(p, 16, 0);
    w32(p, g.0);
    w16(p + 4, g.1);
    w16(p + 6, g.2);
    p
}

#[derive(Clone, Copy, PartialEq)]
enum Svc {
    Render,
    Clock,
    Stream,
    Simple,
    Channel,
    Session,
    Enumerator,
}

#[test]
fn wasapi_api_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0x0a5a_91c0);
    let blk = mem::heap_alloc(256, true);
    let po = mem::heap_alloc(64, true);
    let ig = mem::heap_alloc(16, true);
    let fmt = mem::heap_alloc(64, true);
    let sg = mem::heap_alloc(16, true);
    // (sessions are global: the process's default one may come from a test before; shown as
    // "default" or by the order this test made them in)
    let default_session = with(|e| session_of(e, [0; 4]));
    let first_session = with(|e| e.sessions.len());
    let sess_name = |i: usize| if i == default_session { "default".to_string() } else { i.wrapping_sub(first_session).to_string() };
    let mut clients: Vec<u32> = Vec::new();
    let mut svcs: Vec<(Svc, u32)> = Vec::new();
    let iids: [Iid; 14] = [IID_CLIENT, IID_RENDER, IID_CLOCK, IID_CLOCK_ADJUST, IID_SIMPLE_VOLUME, IID_STREAM_VOLUME, IID_CHANNEL_VOLUME, IID_SESSION, IID_SESSION2, IID_MANAGER, IID_MANAGER2, IID_ENUMERATOR, IID_ENDPOINT_VOLUME, (0, 0, 0)];
    let unknown = mem::heap_alloc(16, true);
    mem::write(unknown, &[0, 0, 0, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
    let name = |p: u32, clients: &[u32], svcs: &[(Svc, u32)]| -> String {
        if p == 0 {
            "0".into()
        } else if p == 0xdead_beef {
            "untouched".into()
        } else if let Some(i) = clients.iter().position(|&c| c == p) {
            format!("client#{i}")
        } else if let Some(i) = clients.iter().position(|&c| c + ps() == p) {
            format!("adjust#{i}")
        } else if p == manager() {
            "manager".into()
        } else if p == endpoint_volume() {
            "endpoint".into()
        } else if svcs.iter().any(|s| s.1 == p) {
            "known-service".into()
        } else {
            "new".into()
        }
    };
    for i in 0..5000 {
        putp(po, 0xdead_beef);
        let null = r.below(14) == 0;
        let o = if null { 0 } else { po };
        let cl = clients.last().copied().unwrap_or(0);
        let line = match r.below(30) {
            0 | 1 => {
                let c = new_client();
                clients.push(c);
                format!("new client#{}", clients.len() - 1)
            }
            2..=4 => {
                let w = seeded_wave(&mut r);
                let w = if r.below(2) == 0 { Wave { tag: 0xfffe, channels: 2, rate: RATE, avg: RATE * 8, block: 8, bits: 32, cb: 22, valid: 32, sub: Some(3) } } else { w };
                put_wave(fmt, &w, 3, w.sub.is_some());
                let mode = r.pick(&[0u32, 0, 0, 1, 2]);
                let flags = r.pick(&[0u32, 0, RATEADJUST.0, AUTOCONVERTPCM.0, EVENTCALLBACK.0, LOOPBACK.0, 0x1, 0x100_0000]);
                let d = r.pick(&[0u64, 100_000, 1_000_000, 10_000_000, MAX_DURATION + 1]);
                let guid = if r.below(3) == 0 {
                    mem::fill(sg, 16, 0);
                    w32(sg, 0x1234 + u32::try_from(r.below(3)).unwrap_or(0));
                    sg
                } else {
                    0
                };
                let f = if r.below(15) == 0 { 0 } else { fmt };
                let d_lo = u32::try_from(d & 0xffff_ffff).unwrap_or(0);
                let d_hi = u32::try_from(d >> 32).unwrap_or(0);
                let hr = call(ac_initialize, &[cl, mode, flags, d_lo, d_hi, 0, 0, f, guid]);
                let sess = with(|e| e.clients.get(&cl).map(|c| sess_name(c.session)));
                format!("initialize {} mode={mode} flags={flags:#x} d={d} guid={}: {hr:#x} session={sess:?}", wave_text(&w), guid != 0)
            }
            5 => format!("buffer size: {:#x} {}", call(ac_buffer_size, &[cl, o]), if o != 0 { r32(po) } else { 0 }),
            6 => format!("latency: {:#x} {}", call(ac_latency, &[cl, o]), if o != 0 { r32(po) } else { 0 }),
            7 => format!("padding: {:#x} {}", call(ac_padding, &[cl, o]), if o != 0 { r32(po) } else { 0 }),
            8 => {
                let w = seeded_wave(&mut r);
                put_wave(fmt, &w, 3, w.sub.is_some());
                let mode = r.pick(&[0u32, 0, 1, 2]);
                let closest = if r.below(5) == 0 { 0 } else { po };
                let hr = call(ac_supported, &[cl, mode, if r.below(15) == 0 { 0 } else { fmt }, closest]);
                let c = rp(po);
                let shown = if closest == 0 { "-".into() } else if c == 0 || c == 0xdead_beef { format!("{c:#x}") } else { format!("{:#x}", hash(c, 40)) };
                format!("supported {} mode={mode}: {hr:#x} {shown}", wave_text(&w))
            }
            9 => {
                let hr = call(ac_mix_format, &[cl, o]);
                format!("mix format: {hr:#x} {}", if hr == 0 { format!("{:#x}", hash(rp(po), 40)) } else { String::new() })
            }
            10 => {
                let (d, m) = (if r.below(4) == 0 { 0 } else { po }, if r.below(4) == 0 { 0 } else { po + 8 });
                mem::fill(po, 16, 0);
                format!("period: {:#x} {:x?}", call(ac_period, &[cl, d, m]), mem::slice(po, 16))
            }
            11 => {
                // (only a client with nothing queued: the engine has nothing to take from it)
                let empty = with(|e| e.clients.get(&cl).is_none_or(|c| c.queue.is_empty() && c.event == 0));
                if !empty {
                    continue;
                }
                format!("start: {:#x}", call(ac_start, &[cl]))
            }
            12 => format!("stop: {:#x}", call(ac_stop, &[cl])),
            13 => format!("reset: {:#x}", call(ac_reset, &[cl])),
            14 => {
                // (an event handle only on a stream that does not take one, or none: no event
                // is ever signalled)
                let ev_ok = with(|e| e.clients.get(&cl).is_some_and(|c| c.flags.has(EVENTCALLBACK)));
                let h = if ev_ok || r.below(2) == 0 { 0 } else { 0x44 };
                format!("set event {h:#x}: {:#x}", call(ac_set_event, &[cl, h]))
            }
            15 | 16 => {
                let k = r.below(u64::try_from(iids.len()).unwrap_or(1));
                let g = iids[usize::try_from(k).unwrap_or(0)];
                let p = if r.below(15) == 0 { 0 } else { iid_at(ig, g) };
                let hr = call(ac_service, &[cl, p, o]);
                let s = rp(po);
                let kind = match g {
                    IID_RENDER => Some(Svc::Render),
                    IID_CLOCK => Some(Svc::Clock),
                    IID_STREAM_VOLUME => Some(Svc::Stream),
                    IID_SIMPLE_VOLUME => Some(Svc::Simple),
                    IID_CHANNEL_VOLUME => Some(Svc::Channel),
                    IID_SESSION => Some(Svc::Session),
                    _ => None,
                };
                let shown = name(s, &clients, &svcs);
                if let (0, Some(k), true) = (hr, kind, s != 0 && s != 0xdead_beef) {
                    svcs.push((k, s));
                }
                format!("service iid#{k}: {hr:#x} {shown}")
            }
            17 => {
                let use_unknown = r.below(3) == 0;
                let k = r.below(u64::try_from(iids.len()).unwrap_or(1));
                let p = if use_unknown { unknown } else { iid_at(ig, iids[usize::try_from(k).unwrap_or(0)]) };
                let adj = r.below(3) == 0 && cl != 0;
                let hr = call(if adj { adj_qi } else { ac_qi }, &[if adj { cl + ps() } else { cl }, p, o]);
                format!("client qi{} iid#{}: {hr:#x} {}", if adj { "(adj)" } else { "" }, if use_unknown { 99 } else { k }, name(rp(po), &clients, &svcs))
            }
            18 => {
                let adj = r.below(3) == 0 && cl != 0;
                let rel = r.below(3) == 0;
                let n = match (adj, rel) {
                    (false, false) => call(ac_addref, &[cl]),
                    (true, false) => call(adj_addref, &[cl + ps()]),
                    (false, true) => call(ac_release, &[cl]),
                    (true, true) => call(adj_release, &[cl + ps()]),
                };
                if rel && n == 0 && cl != 0 {
                    clients.pop();
                    svcs.retain(|s| owner(s.1) != cl || s.0 == Svc::Enumerator);
                }
                format!("client {}{}: {n}", if rel { "release" } else { "addref" }, if adj { "(adj)" } else { "" })
            }
            19 => {
                let rate = r.pick(&[44100.0f32, 0.0, -1.0, 384_000.5, 22050.0]);
                format!("set rate {rate}: {:#x}", call(adj_rate, &[cl + ps(), rate.to_bits()]))
            }
            20 | 21 => {
                // the render client
                let Some(&(_, s)) = svcs.iter().rev().find(|s| s.0 == Svc::Render) else { continue };
                let running = with(|e| e.clients.get(&owner(s)).is_some_and(|c| c.running));
                if r.below(2) == 0 {
                    let n = r.pick(&[0u32, 1, 100, 441, 882, 4410, 100_000]);
                    let hr = call(rc_get, &[s, n, o]);
                    let p = rp(po);
                    format!("get buffer {n}: {hr:#x} {}", if p == 0 || p == 0xdead_beef { format!("{p:#x}") } else { "buffer".into() })
                } else {
                    let n = if running { 0 } else { r.pick(&[0u32, 1, 100, 441, 5000]) };
                    let flags = r.pick(&[0u32, 0, SILENT, 1, 4]);
                    let mut written = String::new();
                    if let Some(Some((m, taken, block))) = with(|e| e.clients.get(&owner(s)).map(|c| c.taken.map(|t| (c.mem, t, c.fmt.map_or(8, |f| f.block()))))) {
                        let bytes = r.bytes(usize::try_from(taken * block).unwrap_or(0).min(4096));
                        mem::write(m, &bytes);
                        written = format!(" taken={taken}");
                    }
                    let hr = call(rc_release, &[s, n, flags]);
                    format!("release buffer {n} flags={flags}{written}: {hr:#x} padding={:?}", with(|e| e.clients.get(&owner(s)).map(|c| c.queue.len())))
                }
            }
            22 => {
                let Some(&(_, s)) = svcs.iter().rev().find(|s| s.0 == Svc::Clock) else { continue };
                mem::fill(po, 16, 0);
                let a = call(ck_freq, &[s, o]);
                let f = r32(po);
                let b = call(ck_pos, &[s, o, 0]);
                format!("clock freq {a:#x} {f}, pos {b:#x} {}, chars {:#x}", r32(po), call(ck_chars, &[s, o]))
            }
            23 => {
                let Some(&(_, s)) = svcs.iter().rev().find(|s| s.0 == Svc::Stream) else { continue };
                let i = r.pick(&[0u32, 1, 2, 5, 31, 32]);
                let v = r.pick(&[0.5f32, 0.0, 1.0, 1.5, -0.1, f32::NAN]);
                let a = call(st_set, &[s, i, v.to_bits()]);
                let b = call(st_get, &[s, i, o]);
                let gv = r32(po);
                let n = r.pick(&[1u32, 2, 6, 3]);
                for k in 0..n {
                    w32(blk + 4 * k, r.pick(&[0.25f32, 1.0, 2.0]).to_bits());
                }
                let c = call(st_set_all, &[s, n, blk]);
                let d = call(st_get_all, &[s, n, blk + 64]);
                format!("stream vol {i} {v}: {a:#x} get {b:#x} {gv:#x}; all {n}: {c:#x} {d:#x} {:x?}; count {:#x} {}", (0..n).map(|k| r32(blk + 64 + 4 * k)).collect::<Vec<_>>(), call(st_count, &[s, po]), r32(po))
            }
            24 => {
                let s = svcs.iter().rev().find(|s| s.0 == Svc::Simple).map(|s| s.1);
                let s = match s {
                    Some(s) if r.below(3) != 0 => s,
                    _ => {
                        let g = if r.below(2) == 0 { 0 } else { sg };
                        call(sm_volume, &[manager(), g, 0, po]);
                        let s = rp(po);
                        svcs.push((Svc::Simple, s));
                        s
                    }
                };
                let v = r.pick(&[0.5f32, 0.0, 1.0, 1.5, f32::NAN]);
                let a = call(sv_set_volume, &[s, v.to_bits(), 0]);
                let b = call(sv_get_volume, &[s, o]);
                let gv = if b == 0 { r32(po) } else { 0 };
                let m = r.pick(&[0u32, 1, 7]);
                let c = call(sv_set_mute, &[s, m, 0]);
                let d = call(sv_get_mute, &[s, o]);
                format!("simple vol {v}: {a:#x} get {b:#x} {gv:#x}; mute {m}: {c:#x} get {d:#x} {}", if d == 0 { r32(po) } else { 0 })
            }
            25 => {
                let Some(&(_, s)) = svcs.iter().rev().find(|s| s.0 == Svc::Channel) else { continue };
                let i = r.pick(&[0u32, 1, 2]);
                let v = r.pick(&[0.5f32, 0.0, 1.0, 1.5]);
                let a = call(cv_set, &[s, i, v.to_bits(), 0]);
                let b = call(cv_get, &[s, i, o]);
                let gv = r32(po);
                let n = r.pick(&[2u32, 1, 3]);
                for k in 0..n {
                    w32(blk + 4 * k, r.pick(&[0.25f32, 1.0, 2.0]).to_bits());
                }
                let c = call(cv_set_all, &[s, n, if r.below(10) == 0 { 0 } else { blk }, 0]);
                let d = call(cv_get_all, &[s, n, blk + 64]);
                format!("channel vol {i} {v}: {a:#x} get {b:#x} {gv:#x}; all {n}: {c:#x} {d:#x} {:x?}; count {:#x} {}", (0..n).map(|k| r32(blk + 64 + 4 * k)).collect::<Vec<_>>(), call(cv_count, &[s, o]), r32(po))
            }
            26 => {
                // the session control: its own, or the manager's
                let s = match svcs.iter().rev().find(|s| s.0 == Svc::Session).map(|s| s.1) {
                    Some(s) if r.below(3) != 0 => s,
                    _ => {
                        let g = if r.below(2) == 0 { 0 } else { sg };
                        call(sm_control, &[manager(), g, 0, po]);
                        let s = rp(po);
                        svcs.push((Svc::Session, s));
                        s
                    }
                };
                match r.below(8) {
                    0 => format!("session state: {:#x} {}", call(se_state, &[s, o]), if o != 0 { r32(po) } else { 0 }),
                    1 => {
                        let txt = r.pick(&["", "Game", "音楽"]);
                        mem::put_wstr(blk, 64, &txt.encode_utf16().collect::<Vec<_>>());
                        let a = call(se_set_name, &[s, if r.below(8) == 0 { 0 } else { blk }, 0]);
                        let b = call(se_get_name, &[s, o]);
                        format!("session name {txt:?}: {a:#x} get {b:#x} {:?}", if b == 0 { String::from_utf16_lossy(&mem::wstr(rp(po))) } else { String::new() })
                    }
                    2 => {
                        mem::put_wstr(blk, 64, &"C:\\icon.ico,1".encode_utf16().collect::<Vec<_>>());
                        let a = call(se_set_icon, &[s, blk, 0]);
                        let b = call(se_get_icon, &[s, o]);
                        format!("session icon: {a:#x} get {b:#x} {:?}", if b == 0 { String::from_utf16_lossy(&mem::wstr(rp(po))) } else { String::new() })
                    }
                    3 => {
                        for k in 0..4 {
                            w32(blk + 4 * k, 0x1111 * (k + 1));
                        }
                        let set = r.below(2) == 0;
                        let a = if set { call(se_set_group, &[s, blk, 0]) } else { 0 };
                        mem::fill(po, 16, 0);
                        let b = call(se_get_group, &[s, o]);
                        let g: Vec<u32> = (0..4).map(|k| r32(po + 4 * k)).collect();
                        format!("session group set={set}: {a:#x} get {b:#x} {}", if g == [0x1111, 0x2222, 0x3333, 0x4444] { "the set one".into() } else if g == [0; 4] { "zero".into() } else { "generated".to_string() })
                    }
                    4 => format!("session notify: {:#x} {:#x} system {:#x} ducking {:#x}", call(se_notify, &[s, 0]), call(se_notify, &[s, 0x1000]), call(se_system, &[s]), call(se_ducking, &[s, 1])),
                    5 => {
                        let k = r.below(u64::try_from(iids.len()).unwrap_or(1));
                        let hr = call(se_qi, &[s, iid_at(ig, iids[usize::try_from(k).unwrap_or(0)]), o]);
                        let p = rp(po);
                        format!("session qi iid#{k}: {hr:#x} {}", if p == s { "self".into() } else { name(p, &clients, &svcs) })
                    }
                    6 => format!("session pid: {:#x} {}", call(se_pid, &[s, o]), r32(po) == crate::pid()),
                    _ => {
                        let a = call(se_identifier, &[s, o]);
                        let id = if a == 0 { String::from_utf16_lossy(&mem::wstr(rp(po))) } else { String::new() };
                        putp(po, 0);
                        let b = call(se_instance, &[s, o]);
                        let inst = if b == 0 { String::from_utf16_lossy(&mem::wstr(rp(po))) } else { String::new() };
                        // (the session guid is a counter of the run; the pid the process's)
                        let cut = |s: &str| s.split("%b").next().unwrap_or("").to_string();
                        format!("session ids: {a:#x} {:?} {b:#x} {:?} ends with pid {}", cut(&id), cut(&inst), inst.ends_with(&format!("|1%b{}", crate::pid())))
                    }
                }
            }
            27 => {
                // the manager and its enumerator
                let m = manager();
                match r.below(4) {
                    0 => {
                        let use_unknown = r.below(3) == 0;
                        let k = r.below(u64::try_from(iids.len()).unwrap_or(1));
                        let p = if use_unknown { unknown } else { iid_at(ig, iids[usize::try_from(k).unwrap_or(0)]) };
                        format!("manager qi iid#{}: {:#x} {} ref {}", if use_unknown { 99 } else { k }, call(sm_qi, &[m, p, o]), name(rp(po), &clients, &svcs), call(sm_ref, &[m]))
                    }
                    1 => {
                        let hr = call(sm_enum, &[m, o]);
                        let e = rp(po);
                        if hr == 0 {
                            let n = call(en_count, &[e, po]);
                            let cnt = r32(po);
                            let k = r.pick(&[0u32, 1, cnt.wrapping_sub(1), cnt, 0xffff_ffff]);
                            let g = call(en_get, &[e, k, po]);
                            format!("manager enum: {hr:#x} count {n:#x} {} get {} {g:#x} {}", cnt.wrapping_sub(u32::try_from(first_session).unwrap_or(0)), if k == cnt { "count".into() } else if k == cnt.wrapping_sub(1) { "last".into() } else { k.to_string() }, name(rp(po), &clients, &svcs))
                        } else {
                            format!("manager enum: {hr:#x}")
                        }
                    }
                    2 => format!("manager notify {:#x} {:#x} duck {:#x} {:#x}", call(sm_notify, &[m, 0]), call(sm_notify, &[m, 0x1000]), call(sm_duck, &[m, 0, 0]), call(sm_duck, &[m, 0, 0x1000])),
                    _ => format!("manager nulls: {:#x} {:#x} {:#x}", call(sm_control, &[m, 0, 0, 0]), call(sm_volume, &[m, 0, 0, 0]), call(sm_enum, &[m, 0])),
                }
            }
            _ => {
                // the endpoint volume
                let ep = endpoint_volume();
                let db = r.pick(&[0.0f32, -10.0, -65.25, -70.0, 1.0, -0.03125]);
                let sc = r.pick(&[0.0f32, 0.5, 1.0, 1.5]);
                let ch = r.pick(&[0u32, 1, 2]);
                let a = match r.below(8) {
                    0 => format!("set db {db}: {:#x}", call(ev_set_db, &[ep, db.to_bits(), 0])),
                    1 => format!("set scalar {sc}: {:#x}", call(ev_set_scalar, &[ep, sc.to_bits(), 0])),
                    2 => format!("set ch {ch} db {db}: {:#x}", call(ev_set_ch_db, &[ep, ch, db.to_bits(), 0])),
                    3 => format!("set ch {ch} scalar {sc}: {:#x}", call(ev_set_ch_scalar, &[ep, ch, sc.to_bits(), 0])),
                    4 => format!("mute {}: {:#x}", ch & 1, call(ev_set_mute, &[ep, ch & 1, 0])),
                    5 => format!("step up {:#x}", call(ev_step_up, &[ep, 0])),
                    6 => format!("step down {:#x}", call(ev_step_down, &[ep, 0])),
                    _ => format!("qi {:#x} {}", call(ev_qi, &[ep, if r.below(2) == 0 { unknown } else { iid_at(ig, IID_ENDPOINT_VOLUME) }, o]), name(rp(po), &clients, &svcs)),
                };
                mem::fill(blk, 64, 0);
                let reads = [
                    call(ev_get_db, &[ep, blk]),
                    call(ev_get_scalar, &[ep, blk + 4]),
                    call(ev_get_ch_db, &[ep, ch, blk + 8]),
                    call(ev_get_ch_scalar, &[ep, ch, blk + 12]),
                    call(ev_get_mute, &[ep, blk + 16]),
                    call(ev_step_info, &[ep, blk + 20, blk + 24]),
                    call(ev_count, &[ep, blk + 28]),
                    call(ev_hardware, &[ep, blk + 32]),
                    call(ev_range, &[ep, blk + 36, blk + 40, blk + 44]),
                ];
                format!("endpoint {a}; reads {reads:x?} {:x?}", (0..12).map(|k| r32(blk + 4 * k)).collect::<Vec<_>>())
            }
        };
        writeln!(out, "{i} {line}").unwrap();
    }
    // (stop what runs, leave the endpoint as a process starts with it)
    for c in &clients {
        call(ac_stop, &[*c]);
    }
    with(|e| e.endpoint = Endpoint { db: MAX_DB, mute: false, channel: [MAX_DB; 2] });
    snapshot("wasapi_api", &out);
}
