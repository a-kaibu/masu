//! Golden sweeps of Media Foundation's COM side (mf.rs) before its typed rewrite: the attribute
//! store's IMFAttributes, the Media Engine's IMFMediaEngine(Ex) without a movie (its sources
//! failing, its state and events), the class factory and the DXGI device manager. Every handler
//! is called straight from its method table with seeded arguments; each line is its answer and
//! what it left behind.

use super::*;
use crate::test_guest::{call_st0, call_stacked as call, enter, guest_thread, snapshot, Rng};
use std::fmt::Write;

/// The keys the sweeps set and get by (k0..k3), as guest GUIDs.
fn keys() -> [u32; 4] {
    let k = mem::heap_alloc(64, true);
    for i in 0..4u32 {
        mem::write(k + 16 * i, &[u8::try_from(i).unwrap_or(0) + 1, 0x5a, 0, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
    }
    [k, k + 16, k + 32, k + 48]
}

/// Objects by the order they were made in (a released one's address can come back).
#[derive(Default)]
struct Names(Vec<u32>);
impl Names {
    fn add(&mut self, o: u32) -> String {
        self.0.push(o);
        self.of(o)
    }
    fn of(&self, o: u32) -> String {
        if o == 0 {
            return "0".into();
        }
        self.0.iter().rposition(|&x| x == o).map_or_else(|| format!("{o:#x}?"), |i| format!("#{i}"))
    }
}

fn key_name(g: &Guid, keys: &[u32; 4]) -> String {
    keys.iter().position(|&k| guid_at(k) == *g).map_or_else(|| format!("{:02x?}", &g[..4]), |i| format!("k{i}"))
}
fn val_str(v: &Val, names: &Names) -> String {
    match v {
        Val::Unk(u) => format!("Unk({})", names.of(*u)),
        Val::Str(s) => format!("Str({:?})", String::from_utf16_lossy(s)),
        Val::Blob(b) => format!("Blob({:02x?})", b),
        Val::U32(_) | Val::U64(_) | Val::F64(_) | Val::Guid(_) => format!("{v:?}"),
    }
}
fn store_str(b: u32, keys: &[u32; 4], names: &Names) -> String {
    match attrs(b, |a| (a.0, a.1.clone())) {
        None => "gone".into(),
        Some((refs, s)) => format!("refs={refs} [{}]", s.items.iter().map(|(k, v)| format!("{}={}", key_name(k, keys), val_str(v, names))).collect::<Vec<_>>().join(" ")),
    }
}

/// A seeded PROPVARIANT at `p` (some of a type the store refuses).
fn seeded_prop(r: &mut Rng, p: u32, wname: u32, guid: u32, blob: u32, unk: u32) -> String {
    mem::fill(p, 24, 0);
    let vt = r.pick(&[19u16, 21, 5, 72, 31, 0x1011, 13, 3, 0]);
    w16(p, vt);
    match vt {
        19 => w32(p + 8, r.pick(&[0, 1, 0xffff_ffff])),
        21 => w64(p + 8, r.pick(&[0, 1 << 40])),
        5 => w64(p + 8, r.pick(&[0.0f64, 1.5, -2.25]).to_bits()),
        72 => wp(p + 8, guid),
        31 => wp(p + 8, wname),
        0x1011 => {
            w32(p + 8, r.pick(&[0, 3, 8]));
            wp(p + 8 + ps(), blob);
        }
        13 => wp(p + 8, unk),
        _ => {}
    }
    format!("vt{vt}")
}

#[test]
fn mf_attributes_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0x00af_a77e);
    let keys = keys();
    let (po, po2, po3) = (mem::heap_alloc(64, true), mem::heap_alloc(64, true), mem::heap_alloc(64, true));
    let pv = mem::heap_alloc(32, true);
    let buf = mem::heap_alloc(64, true);
    let wname = mem::heap_alloc(32, true);
    for (i, ch) in (0u32..).zip("movie".encode_utf16().chain([0])) {
        w16(wname + 2 * i, ch);
    }
    let guid = mem::heap_alloc(16, true);
    mem::write(guid, &[9; 16]);
    let blob = mem::heap_alloc(16, true);
    mem::write(blob, &[1, 2, 3, 4, 5, 6, 7, 8]);
    let iid = mem::heap_alloc(16, true);
    let mut names = Names::default();
    let mut objs: Vec<u32> = Vec::new();
    let ms = attrs_methods();
    // the store's methods as their x86 argument words (after this): k a key, p an out pointer,
    // P a PROPVARIANT out, R a PROPVARIANT in, A another store, v a value, d a double, g a GUID,
    // w a string, b a buffer, i an IID, U an interface
    let specs = ["ip", "", "", "kP", "kp", "kRp", "Avp", "kp", "kp", "kp", "kp", "kp", "kbvp", "kpp", "kp", "kbvp", "kpp", "kip", "kR", "k", "", "kv", "kvv", "kd", "kg", "kw", "kbv", "kU", "", "", "p", "vpP", "A"];
    assert_eq!(specs.len(), ms.len());
    for (s, m) in specs.iter().zip(&ms) {
        assert_eq!(4 * (1 + u32::try_from(s.len() + s.matches('d').count()).unwrap_or(0)), m.1, "{s}");
    }
    for i in 0..2500 {
        if objs.is_empty() || r.below(30) == 0 {
            let hr = call(MFCreateAttributes, &[if r.below(10) == 0 { 0 } else { po }, 4]);
            let o = if hr == 0 { rp(po) } else { 0 };
            let shown = if o != 0 {
                objs.push(o);
                names.add(o)
            } else {
                "-".into()
            };
            writeln!(out, "{i} create: {hr:#x} {shown}").unwrap();
            continue;
        }
        let this = objs[usize::try_from(r.below(u64::try_from(objs.len()).unwrap_or(1))).unwrap_or(0)];
        let j = usize::try_from(r.below(u64::try_from(ms.len()).unwrap_or(1))).unwrap_or(0);
        // (a store's last release is the guest's to get right: one released too often while
        // another holds it is freed under that one; the sweep keeps every store it made)
        if j == 2 && attrs(this, |a| a.0).unwrap_or(0) <= 1 {
            writeln!(out, "{i} {} release: kept", names.of(this)).unwrap();
            continue;
        }
        let mut args = vec![this];
        let mut shown = Vec::new();
        // (each out pointer of a call its own)
        let mut outs = [po, po2, po3].into_iter();
        for kind in specs[j].chars() {
            let (a, s): (Vec<u32>, String) = match kind {
                'k' => {
                    let k = if r.below(12) == 0 { 0 } else { r.pick(&keys) };
                    (vec![k], keys.iter().position(|&x| x == k).map_or("0".into(), |n| format!("k{n}")))
                }
                'p' => {
                    let o = outs.next().unwrap_or(po3);
                    let p = if r.below(10) == 0 { 0 } else { o };
                    if p != 0 {
                        mem::fill(p, 64, 0xcc);
                    }
                    (vec![p], if p == 0 { "0".into() } else { "p".into() })
                }
                'P' => {
                    let p = if r.below(6) == 0 { 0 } else { pv };
                    mem::fill(pv, 32, 0);
                    (vec![p], if p == 0 { "0".into() } else { "pv".into() })
                }
                'R' => {
                    let unk = objs[usize::try_from(r.below(u64::try_from(objs.len()).unwrap_or(1))).unwrap_or(0)];
                    let s = seeded_prop(&mut r, pv, wname, guid, blob, unk);
                    (vec![pv], s)
                }
                'A' => {
                    let o = if r.below(10) == 0 { 0 } else { objs[usize::try_from(r.below(u64::try_from(objs.len()).unwrap_or(1))).unwrap_or(0)] };
                    (vec![o], names.of(o))
                }
                'U' => {
                    let o = if r.below(6) == 0 { 0 } else { objs[usize::try_from(r.below(u64::try_from(objs.len()).unwrap_or(1))).unwrap_or(0)] };
                    (vec![o], names.of(o))
                }
                'v' => {
                    let v = r.pick(&[0u32, 1, 2, 3, 4, 5, 8, 64, 0xffff_ffff]);
                    (vec![v], v.to_string())
                }
                'd' => {
                    let d = r.pick(&[0.0f64, 1.0, -0.5, f64::NAN]);
                    let b = d.to_bits();
                    (vec![lo32(b), lo32(b >> 32)], format!("{d}"))
                }
                'g' => (vec![if r.below(8) == 0 { 0 } else { guid }], "g".into()),
                'w' => (vec![if r.below(8) == 0 { 0 } else { wname }], "w".into()),
                'b' => {
                    mem::fill(buf, 64, 0xcc);
                    (vec![if r.below(8) == 0 { 0 } else { buf }], "b".into())
                }
                'i' => {
                    mem::fill(iid, 16, 0);
                    let k = r.pick(&[0u32, 0x2cd2_d921, 0x44ae_0fa8, 0x1234]);
                    w32(iid, k);
                    if k == 0 {
                        mem::write(iid + 8, &[0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
                    }
                    (vec![iid], format!("{k:#x}"))
                }
                _ => unreachable!(),
            };
            args.extend(a);
            shown.push(s);
        }
        let hr = call(ms[j].0, &args);
        let mut line = format!("{i} {} m{j}({}): {hr:#x}", names.of(this), shown.join(","));
        // what the outs hold (a pointer only in the first out of a call that allocates)
        let mut nth = 0;
        for (&a, kind) in args[1..].iter().zip(specs[j].chars().flat_map(|k| if k == 'd' { vec!['d', 'd'] } else { vec![k] })) {
            if kind == 'p' {
                nth += 1;
            }
            if a == 0 {
                continue;
            }
            match kind {
                'p' => {
                    let w0 = r32(a);
                    let what = match (j, nth) {
                        // GetAllocatedString / GetAllocatedBlob: the copy's bytes, not where it is
                        (13, 1) if hr == 0 => format!("{:?}", String::from_utf16_lossy(&mem::wstr(w0))),
                        (16, 1) if hr == 0 => format!("{:02x?}", mem::slice(w0, args.get(3).map_or(0, |&n| if n == 0 { 0 } else { r32(n) }).min(64))),
                        (17 | 0, 1) if hr == 0 => names.of(w0),
                        _ => format!("{:x?}", (0..4).map(|k| r32(a + 4 * k)).collect::<Vec<_>>()),
                    };
                    write!(line, " out={what}").unwrap();
                }
                'P' if hr == 0 => {
                    let v = get_prop(a).map(|v| val_str(&v, &names));
                    write!(line, " pv={v:?}").unwrap();
                }
                'b' => write!(line, " buf={:02x?}", mem::slice(a, 20)).unwrap(),
                _ => {}
            }
        }
        // the stores gone with their last release (this one, or one a store let go)
        objs.retain(|&o| attrs(o, |_| ()).is_some());
        write!(line, " | {}", store_str(this, &keys, &names)).unwrap();
        writeln!(out, "{line}").unwrap();
    }
    snapshot("mf_attributes", &out);
}

/// IMFMediaEngineNotify::EventNotify(event, p1, p2) of the stand-in: what it was told.
static TOLD: Mutex<Vec<(u32, u32, u32)>> = Mutex::new(Vec::new());
fn nt_event(c: &mut Ctx) -> u32 {
    TOLD.lock().unwrap_or_else(|e| e.into_inner()).push((c.a(1), c.a(2), c.a(3)));
    0
}
fn nt_one(_: &mut Ctx) -> u32 {
    1
}
fn nt_noint(_: &mut Ctx) -> u32 {
    Hr::NoInterface.code()
}

#[test]
fn mf_engine_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0x00e9_61e5);
    // the sources: none there, a file there that is no movie (fixed paths: lines name them)
    let dir = std::env::temp_dir().join("masu-mf-golden");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("junk.mp4"), [0u8; 64]).unwrap();
    let host = dir.to_string_lossy().replace('/', "\\");
    let urls: Vec<String> = vec![format!("Z:{host}\\junk.mp4"), format!("Z:{host}\\none.mp4"), format!("file:///Z:{}/junk%20.mp4", dir.to_string_lossy()), String::new()];
    let wurls: Vec<u32> = urls
        .iter()
        .map(|u| {
            let w: Vec<u16> = u.encode_utf16().chain([0]).collect();
            let p = mem::heap_alloc(2 * lo32(w.len()) + 2, true);
            for (i, &ch) in (0u32..).zip(&w) {
                w16(p + 2 * i, ch);
            }
            p
        })
        .collect();
    let (po, po2) = (mem::heap_alloc(64, true), mem::heap_alloc(64, true));
    let rect = mem::heap_alloc(16, true);
    let irect = mem::heap_alloc(16, true);
    let color = mem::heap_alloc(4, true);
    let iid = mem::heap_alloc(16, true);
    let notify = object(vtable("test IMFMediaEngineNotify", &[(nt_noint as Handler, 12), (nt_one, 4), (nt_one, 4), (nt_event, 16)]), 1);
    let ms = engine_methods();
    // x86 argument words after this: p out, v value, d double, w a URL (BSTR), i IID, r a
    // normalized rect, R a RECT, c a colour, t a texture
    let specs = [
        "ip", "", "", "p", "v", "p", "w", "p", "", "", "v", "p", "", "wp", "", "", "", "d", "", "", "", "", "d", "", "d", "p", "p", "", "", "v", "", "v", "", "", "", "v", "", "d", "", "", "pp", "pp", "", "trRc", "p", //
        "pw", "vp", "rRc", "", "d", "d", "v", "p", "ip", "p", "vip", "vp", "vv", "", "p", "pv", "pv", "", "d", "p", "", "", "p", "v", "p", "v", "v", "p", "v", "p", "v", "p", "v", "p", "v", "dv", "v",
    ];
    assert_eq!(specs.len(), ms.len());
    for (k, (s, m)) in specs.iter().zip(&ms).enumerate() {
        assert_eq!(4 * (1 + u32::try_from(s.len() + s.matches('d').count()).unwrap_or(0)), m.1, "method {k}: {s}");
    }
    let mut names = Names::default();
    let mut live: Vec<u32> = Vec::new();
    let vt = vtable("IMFMediaEngine", &ms);
    for i in 0..3000 {
        if live.is_empty() || r.below(60) == 0 {
            let b = object(vt, 1);
            let e = Engine { refs: 1, st: State::default(), notify, shut: false, src: None, movie: false, ticked: None, category: 0, role: 0, thread: false, frames: Frames::default() };
            ENGINES.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(b, e);
            live.push(b);
            writeln!(out, "{i} engine {}", names.add(b)).unwrap();
            continue;
        }
        let b = live[usize::try_from(r.below(u64::try_from(live.len()).unwrap_or(1))).unwrap_or(0)];
        let mut j = usize::try_from(r.below(u64::try_from(ms.len()).unwrap_or(1))).unwrap_or(0);
        // (Release and Shutdown seldom: most calls on an engine that is still there)
        if matches!(j, 2 | 42) && r.below(10) != 0 {
            // (one of IMFMediaEngine's own, past IUnknown, before Shutdown)
            j = 3 + usize::try_from(r.below(39)).unwrap_or(0);
        }
        let mut args = vec![b];
        let mut shown = Vec::new();
        for kind in specs[j].chars() {
            let (a, s): (Vec<u32>, String) = match kind {
                'p' => {
                    let p = if r.below(10) == 0 { 0 } else { r.pick(&[po, po2]) };
                    if p != 0 {
                        mem::fill(p, 64, 0xcc);
                    }
                    (vec![p], if p == 0 { "0".into() } else { "p".into() })
                }
                'v' => {
                    let v = r.pick(&[0u32, 1, 2, 3, 4, 6, 0xffff_ffff]);
                    (vec![v], v.to_string())
                }
                'd' => {
                    let d = r.pick(&[0.0f64, 0.5, 1.0, 2.0, -1.0, 1e9, f64::NAN]);
                    let x = d.to_bits();
                    (vec![lo32(x), lo32(x >> 32)], format!("{d}"))
                }
                'w' => {
                    let k = usize::try_from(r.below(5)).unwrap_or(0);
                    (vec![wurls.get(k).copied().unwrap_or(0)], format!("url{k}"))
                }
                'i' => {
                    mem::fill(iid, 16, 0);
                    let k = r.pick(&[0u32, 0x98a1_b0bb, 0x8301_5ead, 0x1234]);
                    w32(iid, k);
                    match k {
                        0 => mem::write(iid + 8, &[0xc0, 0, 0, 0, 0, 0, 0, 0x46]),
                        0x98a1_b0bb => mem::write(iid + 4, &[0xeb, 0x03, 0x35, 0x49]),
                        0x8301_5ead => mem::write(iid + 4, &[0xe6, 0xb1, 0xd0, 0x40]),
                        _ => {}
                    }
                    (vec![iid], format!("{k:#x}"))
                }
                'r' => {
                    for (k, v) in (0u32..).zip([0.0f32, 0.0, 1.0, 1.0]) {
                        w32(rect + 4 * k, v.to_bits());
                    }
                    (vec![if r.below(3) == 0 { 0 } else { rect }], "r".into())
                }
                'R' => {
                    for (k, v) in (0u32..).zip([0u32, 0, 64, 48]) {
                        w32(irect + 4 * k, v);
                    }
                    (vec![if r.below(3) == 0 { 0 } else { irect }], "R".into())
                }
                'c' => {
                    w32(color, 0xff10_2030);
                    (vec![if r.below(3) == 0 { 0 } else { color }], "c".into())
                }
                // (no texture: a sweep has no D3D11 device; nothing is loaded for it anyway)
                't' => (vec![if r.below(3) == 0 { 0 } else { po2 }], "t".into()),
                _ => unreachable!(),
            };
            args.extend(a);
            shown.push(s);
        }
        TOLD.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let (hr, st0) = call_st0(ms[j].0, &args);
        let mut line = format!("{i} {} m{j}({}): {hr:#x}", names.of(b), shown.join(","));
        if let Some(x) = st0 {
            write!(line, " st0={x}").unwrap();
        }
        for (&a, kind) in args[1..].iter().zip(specs[j].chars().flat_map(|k| if k == 'd' { vec!['d', 'd'] } else { vec![k] })) {
            if kind == 'p' && a != 0 {
                let w0 = r32(a);
                let what = match j {
                    // GetError: an IMFMediaError of its own, or none; GetCurrentSource: the URL
                    3 => format!("{}", w0 != 0 && w0 != 0xcccc_cccc),
                    7 if hr == 0 && w0 != 0 => format!("{:?}", String::from_utf16_lossy(&mem::wstr(w0))),
                    0 if hr == 0 => names.of(w0),
                    _ => format!("{:x?}", (0..2).map(|k| r32(a + 4 * k)).collect::<Vec<_>>()),
                };
                write!(line, " out={what}").unwrap();
            }
        }
        // the events it queued (its thread would tell them), then what it is
        let st = engine(b, |e| {
            let ev: Vec<_> = e.st.events.drain(..).collect();
            (ev, e.refs, e.shut, e.st.clone(), e.category, e.role, e.src.as_ref().map(|s| String::from_utf16_lossy(s)))
        });
        match st {
            Some((ev, refs, shut, s, cat, role, src)) => {
                let ev: Vec<(u32, u32, u32)> = ev.iter().map(|&(e, p1, p2)| (e.code(), p1, p2)).collect();
                write!(line, " | ev={ev:?} refs={refs} shut={shut} ready={} net={} paused={} ended={} seeking={} auto={} loop={} muted={} vol={} rate={}/{} err={:x?} dur={} video={:?} audio={} cat={cat} role={role} src={:?}", s.ready.code(), s.network.code(), s.paused, s.ended, s.seeking, s.autoplay, s.looping, s.muted, s.volume, s.rate, s.default_rate, (s.error.0.code(), s.error.1), s.duration, s.video, s.audio, src.map(|s| s.replace(&host, "<dir>").replace(&*dir.to_string_lossy(), "<dir>"))).unwrap();
            }
            None => {
                live.retain(|&x| x != b);
                write!(line, " | gone").unwrap();
            }
        }
        let told = std::mem::take(&mut *TOLD.lock().unwrap_or_else(|e| e.into_inner()));
        if !told.is_empty() {
            write!(line, " told={told:?}").unwrap();
        }
        writeln!(out, "{line}").unwrap();
    }
    for b in live {
        while call(en_release, &[b]) > 0 {}
    }
    snapshot("mf_engine", &out);
}

#[test]
fn mf_factory_and_manager_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0x00fa_c70e);
    let (po, po2) = (mem::heap_alloc(64, true), mem::heap_alloc(64, true));
    let clsid = mem::heap_alloc(16, true);
    let iid = mem::heap_alloc(16, true);
    let set = |p: u32, d1: u32, rest: &[u8]| {
        mem::fill(p, 16, 0);
        w32(p, d1);
        mem::write(p + 4, rest);
    };
    // CoCreateInstance's side: the class factory for this CLSID and IID, or not
    for (c, i) in [(0xb443_92da, 0x4d64_5ace), (0xb443_92da, 0), (0xb443_92da, 0x1234), (0x1234, 0x4d64_5ace)] {
        set(clsid, c, if c == 0xb443_92da { &[0x9b, 0x49, 0x6b, 0x44] } else { &[] });
        set(iid, i, &[]);
        match i {
            0 => mem::write(iid + 8, &[0xc0, 0, 0, 0, 0, 0, 0, 0x46]),
            0x4d64_5ace => mem::write(iid + 4, &[0xaa, 0x26, 0x88, 0x46]),
            _ => {}
        }
        writeln!(out, "create_instance {c:#x} {i:#x}: {:?}", create_instance(clsid, iid).map(|o| o != 0)).unwrap();
    }
    // the DXGI device manager: tokens, devices (a stand-in counting its references), handles
    static REFS: Mutex<u32> = Mutex::new(0);
    fn d_add(_: &mut Ctx) -> u32 {
        let mut g = REFS.lock().unwrap_or_else(|e| e.into_inner());
        *g += 1;
        *g
    }
    fn d_rel(_: &mut Ctx) -> u32 {
        let mut g = REFS.lock().unwrap_or_else(|e| e.into_inner());
        *g = g.saturating_sub(1);
        *g
    }
    fn d_qi(c: &mut Ctx) -> u32 {
        putp(c.a(2), 0);
        Hr::NoInterface.code()
    }
    let dev = object(vtable("test device", &[(d_qi as Handler, 12), (d_add, 4), (d_rel, 4)]), 1);
    *REFS.lock().unwrap_or_else(|e| e.into_inner()) = 1;
    let mut mgrs: Vec<(u32, u32)> = Vec::new();
    let mut names = Names::default();
    for i in 0..600 {
        let op = r.below(8);
        if mgrs.is_empty() || op == 0 {
            let p = if r.below(8) == 0 { 0 } else { po };
            let t = if r.below(8) == 0 { 0 } else { po2 };
            let hr = call(MFCreateDXGIDeviceManager, &[t, p]);
            let line = if hr == 0 {
                let (m, token) = (rp(p), r32(t));
                mgrs.push((m, token));
                format!("create: {hr:#x} {}", names.add(m))
            } else {
                format!("create t={} p={}: {hr:#x}", t != 0, p != 0)
            };
            writeln!(out, "{i} {line}").unwrap();
            continue;
        }
        let (m, token) = mgrs[usize::try_from(r.below(u64::try_from(mgrs.len()).unwrap_or(1))).unwrap_or(0)];
        let vt = rp(m);
        let f = |k: u32| crate::api::handler_of(rp(vt + 4 * k)).unwrap();
        let line = match op {
            1 => {
                let (d, t) = (if r.below(5) == 0 { 0 } else { dev }, if r.below(4) == 0 { token ^ 1 } else { token });
                format!("reset dev={} good={}: {:#x}", d != 0, t == token, call(f(7), &[m, d, t]))
            }
            2 => {
                let p = if r.below(5) == 0 { 0 } else { po };
                mem::fill(po, 8, 0xcc);
                format!("open: {:#x} {:#x}", call(f(6), &[m, p]), r32(po))
            }
            3 => {
                set(iid, 0x1234, &[]);
                mem::fill(po, 8, 0xcc);
                format!("lock: {:#x} {:#x}", call(f(5), &[m, 1, iid, po, 1]), r32(po))
            }
            4 => {
                set(iid, r.pick(&[0u32, 0xeb53_3d5d, 0x77]), &[]);
                if r32(iid) == 0xeb53_3d5d {
                    mem::write(iid + 4, &[0xb6, 0x2d, 0xf8, 0x40]);
                }
                if r32(iid) == 0 {
                    mem::write(iid + 8, &[0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
                }
                let hr = call(f(0), &[m, iid, po]);
                format!("qi {:#x}: {hr:#x} self={}", r32(iid), rp(po) == m)
            }
            5 => format!("addref: {}", call(f(1), &[m])),
            6 => {
                let n = call(f(2), &[m]);
                if n == 0 {
                    mgrs.retain(|x| x.0 != m);
                }
                format!("release: {n}")
            }
            _ => {
                let n = methods_of(vt);
                format!("methods: {n}")
            }
        };
        writeln!(out, "{i} {} {line} | dev refs={}", names.of(m), *REFS.lock().unwrap_or_else(|e| e.into_inner())).unwrap();
    }
    snapshot("mf_factory_manager", &out);
}

/// How many methods a vtable of ours has (its entries that are traps).
fn methods_of(vt: u32) -> u32 {
    let mut n = 0;
    while n < 64 && crate::api::handler_of(rp(vt + 4 * n)).is_some() {
        n += 1;
    }
    n
}
