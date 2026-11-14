//! dshow pinned before its code changes shape: filter graphs, EVRs and VMRs made with
//! create_instance (classes and IIDs right and wrong), then a seeded sweep of every method of
//! every interface of them (read off their vtables), with seeded arguments: null and real out
//! pointers, IIDs right and wrong, names, files not there, rectangles, stand-in filters that log
//! what is called on them, and each other's interfaces. A graph's end without a movie (500 ms
//! after Run on Windows' clock) is taken at once, so the lines do not depend on timing. Each
//! call's result, what it wrote, and the object's state after it are a line of
//! golden/dshow.txt; objects show by the order this test made them in.
use super::*;
use crate::api::{handler_of, host_fn};
use crate::test_guest::{call_stacked, enter, guest_thread, snapshot, Rng};
use std::fmt::Write;

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
fn log(s: String) {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s);
}
fn take_log() -> String {
    std::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner())).join(" ")
}
fn fake_qi(c: &mut Ctx) -> u32 {
    log(format!("f{}.qi", r32(c.a(0) + 4)));
    put32(c.a(2), 0);
    E_NOINTERFACE
}
fn fake_addref(c: &mut Ctx) -> u32 {
    log(format!("f{}.addref", r32(c.a(0) + 4)));
    2
}
fn fake_release(c: &mut Ctx) -> u32 {
    log(format!("f{}.release", r32(c.a(0) + 4)));
    1
}
fn fake_other(c: &mut Ctx) -> u32 {
    log(format!("f{}.other", r32(c.a(0) + 4)));
    E_NOTIMPL
}
/// A stand-in filter (or pin): IUnknown that logs, every other call logged and E_NOTIMPL.
fn fake(tag: u32) -> u32 {
    static VT: OnceLock<u32> = OnceLock::new();
    let vt = *VT.get_or_init(|| {
        let v = mem::heap_alloc(4 * 40, true);
        w32(v, host_fn("dshow_gold_qi", fake_qi, 12));
        w32(v + 4, host_fn("dshow_gold_addref", fake_addref, 4));
        w32(v + 8, host_fn("dshow_gold_release", fake_release, 4));
        let other = host_fn("dshow_gold_other", fake_other, 4);
        for i in 3..40 {
            w32(v + 4 * i, other);
        }
        v
    });
    let o = mem::heap_alloc(8, true);
    w32(o, vt);
    w32(o + 4, tag);
    o
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Graph,
    Evr,
    Vmr,
}
struct Obj {
    kind: Kind,
    /// its block (0: gone)
    b: u32,
}
impl Obj {
    const fn ifaces(&self) -> u32 {
        match self.kind {
            Kind::Graph => NI,
            Kind::Evr | Kind::Vmr => 3,
        }
    }
}
/// The methods of the vtable at `vt` (its entries up to the first that is no host function).
fn methods(vt: u32) -> u32 {
    let mut n = 0;
    while n < 64 && handler_of(rp(vt + ps() * n)).is_some() {
        n += 1;
    }
    n
}
fn name(objs: &[Obj], fakes: &[u32], v: u32) -> String {
    match v {
        0 => return "0".into(),
        0xdead_beef => return "untouched".into(),
        _ => {}
    }
    for (i, o) in objs.iter().enumerate() {
        if o.b != 0 && v >= o.b && v < o.b + ps() * o.ifaces() && (v - o.b) % ps() == 0 {
            let c = match o.kind {
                Kind::Graph => 'g',
                Kind::Evr => 'e',
                Kind::Vmr => 'v',
            };
            return format!("{c}{i}.{}", (v - o.b) / ps());
        }
    }
    for (i, o) in objs.iter().enumerate() {
        if o.kind == Kind::Graph && o.b != 0 && with(o.b, |g| g.event) == Some(v) {
            return format!("ev{i}");
        }
    }
    if let Some(i) = fakes.iter().position(|&f| f == v) {
        return format!("f{i}");
    }
    // (read only where it can be: a value that is no pointer faults)
    if v >= 8 && mem::accessible(v - 8, 8, false, false) == 8 && mem::heap_owns(v) { "heap".into() } else { format!("{v:#x}") }
}
fn state(o: &Obj, nm: impl Fn(u32) -> String) -> String {
    match o.kind {
        Kind::Graph => {
            // (taken under the lock, named outside it: naming looks at the graphs too)
            let Some((ev, line)) = with(o.b, |g| {
                (g.events.iter().copied().collect::<Vec<_>>(), format!(
                    "refs={} st={} done={} ended={} runs={} notify={:x?} flags={} owner={:#x} style={:#x}/{:#x} win={:?} vis={} auto={} drain={} full={} src={:?} dst={:?} vol={} bal={} evr={} filters={} named={} wired={}",
                    g.refs, g.state, g.done, g.ended, g.runs, g.notify, g.flags, g.owner, g.style, g.style_ex, g.window, g.visible, g.auto_show, g.drain, g.full_screen, g.src, g.dst, g.volume, g.balance, g.evr != 0, g.filters.len(), g.named.len(), g.wired
                ))
            }) else {
                return "gone".into();
            };
            let ev: Vec<String> = ev.iter().map(|e| format!("{:#x}/{}/{}", e.0, nm(e.1), nm(e.2))).collect();
            format!("{line} ev=[{}]", ev.join(" "))
        }
        Kind::Evr | Kind::Vmr => evr_with(o.b, |e| format!("refs={:?} hwnd={:#x} src={:?} dst={:?} mode={} px={:?} wl={} border={:#x} key={:#x} graph={}", e.refs, e.hwnd, e.src, e.dst, e.mode, e.src_px, e.windowless, e.border, e.key, e.graph != 0)).unwrap_or_else(|| "gone".into()),
    }
}

/// What each method's arguments are, by interface (vtable order): p an out pointer, i an IID,
/// w a name, f a filter (or pin), G a graph, v a value, z 0 (no waits), q a pointer to seeded words, g a time
/// format GUID, d a double (two words), r a RECT, n a normalized rectangle, B a big buffer.
fn specs(kind: Kind, k: u32) -> Vec<&'static str> {
    const UNK: [&str; 3] = ["ip", "", ""];
    const DISP: [&str; 4] = ["", "", "", ""];
    let with = |tail: &[&'static str], disp: bool| -> Vec<&'static str> { UNK.iter().chain(if disp { &DISP[..] } else { &[] }).chain(tail).copied().collect() };
    let bf = ["", "", "", "", "", "", "", "", "", "B", "Gw", ""];
    match (kind, k) {
        (Kind::Graph, GB) => with(&["fw", "f", "p", "wp", "ffp", "", "f", "", "ff", "", "ww", "wwp", "", "", "", "", "", ""], false),
        (Kind::Graph, MC) => with(&["", "", "", "vp", "w", "", "", "", ""], true),
        (Kind::Graph, ME) => with(&["p", "pppz", "zp", "", "", "", "vvv", "v", "p"], true),
        (Kind::Graph, MS) => with(&["p", "q", "g", "p", "p", "g", "g", "p", "p", "p", "pgvvg", "qvqv", "pp", "pp", "d", "p", "p"], false),
        (Kind::Graph, MP) => with(&["p", "d", "p", "p", "d", "p", "d", "d", "p", "p", "p"], true),
        (Kind::Graph, BA) => with(&["v", "p", "v", "p"], true),
        (Kind::Graph, VW | VW_NONE) => {
            let props = (0..30).map(|i| if (26..30).contains(&i) == (i % 2 == 1) { if i == 0 { "w" } else { "v" } } else { "p" });
            let tail: Vec<&str> = props.chain(["v", "vvvv", "vvvv", "pppp", "pp", "pp", "pppp", "v", "p"]).collect();
            with(&tail, true)
        }
        (Kind::Graph, BV | BV_NONE) => {
            let props = (0..16).map(|i| if i % 2 == 0 { "v" } else { "p" });
            let tail: Vec<&str> = ["p", "p", "p", "p", "p"].into_iter().chain(props).chain(["vvvv", "pppp", "", "vvvv", "pppp", "", "pp", "", "pp", "", "", "pp"]).collect();
            with(&tail, true)
        }
        (Kind::Graph, GC) => with(&["", "", "", "", "", "", "", "", "", "fv"], false),
        (Kind::Graph, _) => with(&["vvv"], false),
        (Kind::Evr | Kind::Vmr, 0) => with(&bf, false),
        (Kind::Evr, 1) => with(&["iip"], false),
        (Kind::Evr, _) => with(&["pp", "pp", "nr", "pp", "v", "p", "v", "p", "", "", "", "p", "", "p", "", "p"], false),
        (Kind::Vmr, 1) => with(&["", "", "p", "", "p", "v", "p"], false),
        (Kind::Vmr, _) => with(&["pppp", "pp", "pp", "rr", "pp", "p", "v", "v", "", "", "p", "v", "p", "v", "p"], false),
    }
}

const IIDS: [[u32; 4]; 16] = [
    [0, 0, 0xc0, 0x4600_0000],
    [0x56a8_68a9, 0x11ce_0ad4, 0x2000_3ab0, 0x70a7_0baf],
    [0x56a8_68b1, 0x11ce_0ad4, 0x2000_3ab0, 0x70a7_0baf],
    [0x56a8_68c0, 0x11ce_0ad4, 0x2000_3ab0, 0x70a7_0baf],
    [0x36b7_3880, 0x11d1_c2c8, 0xa000_8f84, 0x2a32_5ac9],
    [0x56a8_68b2, 0x11ce_0ad4, 0x2000_3ab0, 0x70a7_0baf],
    [0x56a8_68b3, 0x11ce_0ad4, 0x2000_3ab0, 0x70a7_0baf],
    [0x56a8_68b4, 0x11ce_0ad4, 0x2000_3ab0, 0x70a7_0baf],
    [0x56a8_68b5, 0x11ce_0ad4, 0x2000_3ab0, 0x70a7_0baf],
    [0x03a1_eb8e, 0x4245_32bf, 0x084d_1185, 0x88cb_a908],
    [0xfa99_3888, 0x4383_4fc0, 0x8d1a_3fb6, 0x8b4b_9e1a],
    [0xa490_b1e4, 0x4dd3_ab84, 0x5ff4_dc8c, 0x8653_8b7f],
    [0x9e55_30c5, 0x4436_7d1d, 0x8b5f_aa6e, 0x53c2_ad5d],
    [0x0eb1_088c, 0x4f2f_4be2, 0, 0],
    [0x56a8_68a2, 0x11ce_0ad4, 0x2000_3ab0, 0x70a7_0baf],
    [0x1234_5678, 0, 0, 0],
];
const CLASSES: [[u32; 4]; 5] = [
    [0xe436_ebb3, 0x11ce_524f, 0x2000_539f, 0x70a7_0baf],
    [0xe436_ebb8, 0x11ce_524f, 0x2000_539f, 0x70a7_0baf],
    [0xfa10_746c, 0x4b6c_9b63, 0x30fc_49bc, 0x56f2_a50e],
    [0xb87b_eb7b, 0x423f_8d29, 0x8265_4dae, 0xac75_01c1],
    [0xe436_ebb3, 0x11ce_5240, 0, 0],
];
/// TIME_FORMAT_MEDIA_TIME, TIME_FORMAT_FRAME, and none
const FORMATS: [[u32; 4]; 3] = [[0x7b78_5574, 0x11cf_8c82, 0xaa00_0cbc, 0xf674_ac00], [0x7b78_5570, 0x11cf_8c82, 0xaa00_0cbc, 0xf674_ac00], [0, 0, 0, 0]];

/// The ends ENDS holds for `ours`, taken now (as if PLAY went by at once): what the movie ends
/// thread does with them.
fn ends_now(ours: &[u32]) {
    let due: Vec<(u32, u32)> = {
        let mut v = ENDS.lock().unwrap_or_else(|e| e.into_inner());
        let d = v.iter().filter(|e| ours.contains(&e.1)).map(|e| (e.1, e.2)).collect();
        v.retain(|e| !ours.contains(&e.1));
        d
    };
    for (b, n) in due {
        if with(b, |g| g.runs == n && g.done && !g.playing) == Some(true) {
            completed(b);
        }
    }
}

#[test]
fn dshow_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0xd5_0a01);
    let (cls, iid, po, po2, po3, rect, wname, big) = (mem::heap_alloc(16, true), mem::heap_alloc(16, true), mem::heap_alloc(64, true), mem::heap_alloc(64, true), mem::heap_alloc(64, true), mem::heap_alloc(16, true), mem::heap_alloc(256, true), mem::heap_alloc(1024, true));
    let (qbuf, fmt, nrect) = (mem::heap_alloc(16, true), mem::heap_alloc(16, true), mem::heap_alloc(16, true));
    let fakes: Vec<u32> = (0..3).map(fake).collect();
    let mut objs: Vec<Obj> = Vec::new();
    let names = ["", "Video Renderer", "Z:\\masu-golden-none\\movie.mpg", "movie.wmv", "x.avi"];
    for i in 0..4000 {
        for p in [po, po2, po3] {
            mem::fill(p, 64, 0);
            w32(p, 0xdead_beef);
        }
        mem::write(iid, &IIDS[usize::try_from(r.below(16)).unwrap_or(0)].map(u32::to_le_bytes).concat());
        mem::fill(big, 1024, 0);
        let wn = r.pick(&names);
        mem::write(wname, &wn.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect::<Vec<u8>>());
        let live: Vec<usize> = (0..objs.len()).filter(|&j| objs[j].b != 0).collect();
        if live.len() < 3 || r.below(60) == 0 {
            if objs.len() >= 24 {
                continue;
            }
            let c = usize::try_from(r.below(5)).unwrap_or(0);
            mem::write(cls, &CLASSES[c].map(u32::to_le_bytes).concat());
            let got = create_instance(cls, iid);
            let kind = match c {
                2 => Kind::Evr,
                3 => Kind::Vmr,
                _ => Kind::Graph,
            };
            let shown = match got {
                Some(p) => {
                    let b = objs.len();
                    objs.push(Obj { kind, b: 0 });
                    // (the block: the interface pointer less its slot)
                    let base = (0..12).map(|k| p - ps() * k).find(|&b| mem::heap_owns(b) && rp(b) != 0 && handler_of(rp(rp(b))).is_some()).unwrap_or(p);
                    objs[b].b = base;
                    name(&objs, &fakes, p)
                }
                None => "None".into(),
            };
            writeln!(out, "{i} create cls#{c}: {shown}").unwrap();
            continue;
        }
        let j = live[usize::try_from(r.below(u64::try_from(live.len()).unwrap_or(1))).unwrap_or(0)];
        let o = &objs[j];
        let k = u32::try_from(r.below(u64::from(o.ifaces()))).unwrap_or(0);
        let this = o.b + ps() * k;
        let vt = rp(this);
        let n = methods(vt);
        assert_eq!(specs(o.kind, k).len(), usize::try_from(n).unwrap_or(0), "{}.{k}", name(&objs, &fakes, o.b));
        let m = u32::try_from(r.below(u64::from(n))).unwrap_or(0);
        let Some(f) = handler_of(rp(vt + ps() * m)) else { continue };
        // (filters: the stand-ins, our EVRs' and VMRs' IBaseFilter; graphs: their IFilterGraph. An
        // interface that is neither given as one faults on Windows too)
        let filters: Vec<u32> = objs.iter().filter(|o| o.b != 0 && o.kind != Kind::Graph).map(|o| o.b).chain(fakes.iter().copied()).collect();
        let graphs: Vec<u32> = objs.iter().filter(|o| o.b != 0 && o.kind == Kind::Graph).map(|o| o.b).chain(fakes.iter().copied()).collect();
        let spec = specs(o.kind, k).get(usize::try_from(m).unwrap_or(0)).copied().unwrap_or("");
        let mut args = vec![this];
        let mut shown_args = Vec::new();
        let ptr = |r: &mut Rng, p: u32| if r.below(10) == 0 { 0 } else { p };
        for kind in spec.chars() {
            let (a, shown) = match kind {
                'p' => {
                    let p = if r.below(8) == 0 { 0 } else { r.pick(&[po, po2, po3]) };
                    (p, match p { 0 => "0".to_string(), p if p == po => "po".into(), p if p == po2 => "po2".into(), _ => "po3".into() })
                }
                // (never null: a QueryInterface of a null IID faults on Windows too)
                'i' => (iid, "iid".into()),
                'w' => (ptr(&mut r, wname), format!("{wn:?}")),
                'f' | 'G' => {
                    let v = if kind == 'f' { &filters } else { &graphs };
                    let f = if r.below(6) == 0 { 0 } else { v[usize::try_from(r.below(u64::try_from(v.len()).unwrap_or(1))).unwrap_or(0)] };
                    (f, name(&objs, &fakes, f))
                }
                'v' => {
                    let v = r.pick(&[0u32, 1, 2, 3, 100, 640, 480, 1280, 0xffff_ffff, 0x8000_0000, 0xffff_fff6, 0xffff_d8f0, 10_000]);
                    (v, format!("{v:#x}"))
                }
                'z' => (0, "0".into()),
                'q' => {
                    for k in 0..4 {
                        w32(qbuf + 4 * k, r.pick(&[0u32, 1, 0x37, 0x40, 0xffff_ffff, 0x0098_9680, 0x02fa_f080]));
                    }
                    (ptr(&mut r, qbuf), format!("q{:x?}", (0..4).map(|k| r32(qbuf + 4 * k)).collect::<Vec<_>>()))
                }
                'g' => {
                    let g = r.pick(&[0usize, 0, 1, 2]);
                    mem::write(fmt, &FORMATS[g].map(u32::to_le_bytes).concat());
                    (ptr(&mut r, fmt), format!("fmt{g}"))
                }
                'd' => {
                    let d = r.pick(&[0.0f64, 1.0, 2.5, -1.0, 0.5, 1e9, f64::NAN, 3600.0]);
                    let [lo, hi] = [d.to_bits() & 0xffff_ffff, d.to_bits() >> 32].map(|x| u32::try_from(x).unwrap_or(0));
                    args.push(lo);
                    (hi, format!("{d}"))
                }
                'r' => {
                    for k in 0..4 {
                        w32(rect + 4 * k, r.pick(&[0u32, 1, 10, 100, 640, 0xffff_fff6, 2000]));
                    }
                    (ptr(&mut r, rect), format!("r{:?}", (0..4).map(|k| r32(rect + 4 * k).cast_signed()).collect::<Vec<_>>()))
                }
                'n' => {
                    for k in 0..4 {
                        w32(nrect + 4 * k, r.pick(&[0.0f32, 0.25, 0.5, 1.0, -0.5, 2.0]).to_bits());
                    }
                    (ptr(&mut r, nrect), format!("n{:?}", (0..4).map(|k| f32::from_bits(r32(nrect + 4 * k))).collect::<Vec<_>>()))
                }
                _ => (ptr(&mut r, big), "big".into()),
            };
            args.push(a);
            shown_args.push(if a == 0 { "0".into() } else { shown });
        }
        let ours: Vec<u32> = objs.iter().filter(|o| o.kind == Kind::Graph && o.b != 0).map(|o| o.b).collect();
        let hr = call_stacked(f, &args);
        ends_now(&ours);
        let outs = [po, po2, po3].map(|p| format!("{}/{:#x}/{:#x}/{:#x}", name(&objs, &fakes, rp(p)), r32(p + 4), r32(p + 8), r32(p + 12)));
        let o = &objs[j];
        let st = state(o, |v| if v < 0x1_0000 { format!("{v:#x}") } else { name(&objs, &fakes, v) });
        let gone = st == "gone";
        writeln!(out, "{i} {}.{k}.{m} [{}]: {hr:#x} out=[{}] info={} log=[{}] {st}", name(&objs, &fakes, o.b), shown_args.join(" "), outs.join(" "), name(&objs, &fakes, rp(big + 256)), take_log()).unwrap();
        if gone {
            objs[j].b = 0;
        }
    }
    for p in [cls, iid, po, po2, po3, rect, wname, big, qbuf, fmt, nrect] {
        mem::heap_free(p);
    }
    snapshot("dshow", &out);
}
