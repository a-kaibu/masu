//! dshow_mpeg pinned before its code changes shape. The pure parts over seeded inputs
//! (VIDEOINFOHEADERs, DIB strides and packing, WAVEFORMATEXs, how a source's file is told
//! apart, ASF stream lists, read spans, a Vnv feed's steps, resampling), then a seeded sweep of
//! every method of every interface of each filter kind (read off the vtables, arguments by
//! kind: pins ours and stand-ins, media types seeded, files there and not), each result, what it
//! wrote and the filter's state after it a line of golden/dshow_mpeg_*.txt; objects show by
//! the order this test made them in.
use super::*;
use crate::api::{handler_of, host_fn};
use crate::test_guest::{call_stacked, enter, guest_thread, snapshot, Rng};
use std::fmt::Write;

fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3))
}

#[test]
fn dshow_mpeg_pure_snapshot() {
    let mut out = String::new();
    let mut r = Rng(0xd5_3e01);
    let bits = [32u16, 24, 16, 15, 8, 0];
    for _ in 0..200 {
        let (w, h) = (r.pick(&[0u32, 1, 2, 3, 5, 320, 641]), r.pick(&[0u32, 1, 2, 7, 240]));
        let b = r.pick(&bits);
        let rate = r.pick(&[0.0f64, 29.97, 30.0, 23.976, 1e-9, f64::NAN, 1e9]);
        let v = video_info(w, h, b, r.pick(&[0u32, 3, 0x3156_4d57]), rate);
        writeln!(out, "video_info {w}x{h} {b} {rate}: {} {:016x} stride={}", v.len(), fnv(&v), dib_stride(w, b)).unwrap();
    }
    for _ in 0..300 {
        let (sw, sh) = (r.pick(&[0u32, 1, 2, 3, 5, 9]), r.pick(&[0u32, 1, 2, 4, 7]));
        let (w, h) = (r.pick(&[0u32, 1, 2, 3, 5, 9]), r.pick(&[0u32, 1, 2, 4, 7]));
        let b = r.pick(&bits[..4]);
        let src_len = usize::try_from(4 * sw * sh).unwrap_or(0).saturating_sub(usize::try_from(r.below(2)).unwrap_or(0));
        let src = r.bytes(src_len);
        let stride = usize::try_from(dib_stride(w, b)).unwrap_or(0);
        let dlen = (stride * usize::try_from(h).unwrap_or(0)).saturating_sub(usize::try_from(r.below(2)).unwrap_or(0));
        let mut dst = vec![0xa5u8; dlen];
        let flip = r.below(2) == 0;
        let ok = pack_into(&src, (sw, sh), flip, (w, h, b), &mut dst);
        let whole = if sw * sh * 4 == u32::try_from(src.len()).unwrap_or(0) { format!("{:016x}", fnv(&pack_bgrx(&src, sw, sh, b))) } else { "-".into() };
        writeln!(out, "pack {sw}x{sh} -> {w}x{h}x{b} flip={flip} len={dlen}: {ok} {:016x} bgrx={whole}", fnv(&dst)).unwrap();
    }
    for _ in 0..100 {
        let tag = r.pick(&[1u16, 0x50, 0x55, 3]);
        let (ch, rate, b) = (r.pick(&[0u16, 1, 2, 6]), r.pick(&[0u32, 8000, 44100, 48000]), r.pick(&[0u16, 8, 16, 24, 32]));
        let ne = usize::try_from(r.below(3) * 6).unwrap_or(0);
        let extra = r.bytes(ne);
        writeln!(out, "wave_format {tag} {ch} {rate} {b} +{}: {:02x?}", extra.len(), wave_format(tag, ch, rate, b, &extra)).unwrap();
    }
    let asf = |r: &mut Rng, n: usize| {
        let mut h = Vec::new();
        h.extend(ASF_HEADER);
        let mut objs = Vec::new();
        for i in 0..n {
            let mut o = Vec::new();
            o.extend(if r.below(5) == 0 { [0x11u8; 16] } else { ASF_STREAM_PROPERTIES });
            o.extend(0u64.to_le_bytes());
            o.extend(if r.below(2) == 0 { ASF_VIDEO_MEDIA } else { ASF_AUDIO_MEDIA });
            o.extend([0u8; 16]);
            o.extend(0u64.to_le_bytes());
            o.extend([0u8; 8]);
            o.extend((u16::try_from(i + 1).unwrap_or(0) | 0x8000).to_le_bytes());
            o.extend(0u32.to_le_bytes());
            let len = u64::try_from(o.len()).unwrap_or(0);
            o[16..24].copy_from_slice(&len.to_le_bytes());
            objs.push(o);
        }
        let size = 30 + objs.iter().map(Vec::len).sum::<usize>();
        h.extend(u64::try_from(size).unwrap_or(0).to_le_bytes());
        h.extend(u32::try_from(objs.len()).unwrap_or(0).to_le_bytes());
        h.extend([1, 2]);
        for o in objs {
            h.extend(o);
        }
        let cut = usize::try_from(r.below(8)).unwrap_or(0);
        if cut == 0 {
            h.truncate(h.len() / 2);
        }
        h
    };
    let heads: Vec<Vec<u8>> = vec![vec![0, 0, 1, 0xba, 0x44], vec![0, 0, 1, 0xb3], b"RIFF\x10\0\0\0AVI LIST".to_vec(), b"RIFF\x10\0\0\0WAVEfmt ".to_vec(), b"RIF".to_vec(), Vec::new()];
    for i in 0..120 {
        let head = if i % 3 == 0 { r.pick(&[0usize, 1, 2, 3, 4, 5]).min(heads.len() - 1) } else { 99 };
        let ns = usize::try_from(r.below(5)).unwrap_or(0);
        let h = if head == 99 { asf(&mut r, ns) } else { heads[head].clone() };
        let name = r.pick(&["movie.wmv", "a.WMA", "x.asf", "OP01.dat", "logo.DAT", "movie", "a.wmv.dat"]);
        let st = asf_streams(&h);
        writeln!(out, "source {name} {:016x}/{}: asf={} sub={:02x?} streams={st:?} pins={:?}", fnv(&h), h.len(), source_is_asf(name, &h), &stream_subtype(&h)[..4], st.as_ref().map(|s| asf_pin_names(s))).unwrap();
    }
    for _ in 0..100 {
        let (pos, n, len) = (r.pick(&[0u64, 1, 100, 1 << 32, u64::MAX]), r.pick(&[0u32, 1, 100, u32::MAX]), r.pick(&[0u64, 1, 50, 100, 1 << 33]));
        writeln!(out, "read_span {pos} {n} {len}: {:?}", read_span(pos, n, len)).unwrap();
    }
    for last in [None, Some(0u32), Some(5)] {
        for now in [None, Some(0u32), Some(5), Some(6)] {
            for ended in [false, true] {
                for eos in [false, true] {
                    writeln!(out, "feed {last:?} {now:?} {ended} {eos}: {:?}", feed_step(last, now, ended, eos)).unwrap();
                }
            }
        }
    }
    for _ in 0..60 {
        let from = r.pick(&[8000u32, 22050, 32000, 44100, 48000]);
        let to = r.pick(&[44100u32, 48000, 22050]);
        let n = 2 * usize::try_from(r.below(600)).unwrap_or(0);
        let x: Vec<f32> = (0..n).map(|_| f32::from(u8::try_from(r.below(200)).unwrap_or(0)) / 100.0 - 1.0).collect();
        let mut rs = Resampler::new(from, to);
        let mut o = VecDeque::new();
        let mut at = 0;
        while at < n {
            let e = (at + 2 * usize::try_from(1 + r.below(100)).unwrap_or(1)).min(n);
            rs.push(&x[at..e], &mut o);
            at = e;
        }
        let bytes: Vec<u8> = o.iter().flat_map(|f| f.iter().flat_map(|v| v.to_le_bytes())).collect();
        writeln!(out, "resample {from}->{to} {}: {} {:016x}", n / 2, o.len(), fnv(&bytes)).unwrap();
    }
    snapshot("dshow_mpeg_pure", &out);
}

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
fn log(s: String) {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s);
}
fn take_log() -> String {
    std::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner())).join(" ")
}
fn fake_qi(c: &mut Ctx) -> u32 {
    log(format!("f{}.qi", r32(c.a(0) + 4)));
    crate::mem::putp(c.a(2), 0);
    Hr::NoInterface.code()
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
    Hr::NotImpl.code()
}
/// A stand-in object (a pin, a graph, a sample): IUnknown that logs, every other call logged
/// and E_NOTIMPL.
fn fake(tag: u32) -> u32 {
    static VT: OnceLock<u32> = OnceLock::new();
    let vt = *VT.get_or_init(|| {
        let v = mem::heap_alloc(4 * 40, true);
        w32(v, host_fn("dsmpeg_gold_qi", fake_qi, 12));
        w32(v + 4, host_fn("dsmpeg_gold_addref", fake_addref, 4));
        w32(v + 8, host_fn("dsmpeg_gold_release", fake_release, 4));
        let other = host_fn("dsmpeg_gold_other", fake_other, 4);
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

/// The kinds of each method's arguments, by interface word w of a filter's block (vtable
/// order): p an out pointer, i an IID (never null), w a name, P a pin, G a graph or sample
/// stand-in, m a media type, v a value, B a big buffer, S an array of stand-ins.
fn specs(w: Word) -> Vec<&'static str> {
    let unk = ["ip", "", ""];
    let tail: &[&str] = match w {
        Word::Filter => &["p", "", "", "vv", "vp", "", "p", "p", "wp", "B", "Gw", ""],
        Word::Pin(_) => &["Pm", "Pm", "", "p", "p", "B", "p", "p", "m", "", "", "", "", "", "vvvvvv"],
        Word::StreamSelect => &["p", "vppppppp", "vv"],
        Word::FileSource => &["wm", "pp"],
        Word::AsyncReader => &["", "", "", "", "vvvB", "pp", "", ""],
        Word::DmoWrapper => &["i"],
        Word::MemInput => &["p", "", "", "G", "Svp", ""],
    };
    unk.iter().chain(tail).copied().collect()
}
fn methods(vt: u32) -> u32 {
    let mut n = 0;
    while n < 64 && handler_of(rp(vt + ps() * n)).is_some() {
        n += 1;
    }
    n
}
struct Names {
    filters: Vec<u32>,
    fakes: Vec<u32>,
}
impl Names {
    fn of(&self, v: u32) -> String {
        match v {
            0 => return "0".into(),
            0xdead_beef => return "untouched".into(),
            _ => {}
        }
        for (i, &b) in self.filters.iter().enumerate() {
            if b != 0 && v >= b && v <= Word::MemInput.of(b) && (v - b) % ps() == 0 {
                return format!("F{i}.{}", (v - b) / ps());
            }
        }
        if let Some(i) = self.fakes.iter().position(|&f| f == v) {
            return format!("f{i}");
        }
        if v >= 8 && mem::accessible(v - 8, 8, false, false) == 8 && mem::heap_owns(v) { "heap".into() } else { format!("{v:#x}") }
    }
}
/// The FNV of `n` bytes at `p` with the pointers in them (ours, the stand-ins', the heap's) as
/// 0xaaaaaaaa: what a buffer holds without where things happened to be.
fn scrubbed(p: u32, n: u32, names: &Names) -> u64 {
    let words: Vec<u8> = (0..n / 4)
        .flat_map(|i| {
            let w = r32(p + 4 * i);
            let named = w != 0 && w != 0xdead_beef && !names.of(w).starts_with("0x");
            (if named { 0xaaaa_aaaa } else { w }).to_le_bytes()
        })
        .collect();
    fnv(&words)
}
fn state(b: u32, n: &Names) -> String {
    let Some((line, peers)) = fwith(b, |f| {
        let pins: Vec<String> = f.pins.iter().map(|p| format!("{}{}{}{}", if p.out { "o" } else { "i" }, p.name, if p.on { "" } else { "(off)" }, if p.mt != 0 { "+mt" } else { "" })).collect();
        (
            format!("{:?} refs={} graph={} name={:?} st={} bits={} src={:?} pins=[{}]", f.kind, f.refs, f.graph != 0, String::from_utf16_lossy(&f.name), f.state.code(), f.bits, f.src.as_ref().map(|s| (s.len, s.sub[0])), pins.join(",")),
            f.pins.iter().map(|p| (p.peer, p.mem)).collect::<Vec<_>>(),
        )
    }) else {
        return "gone".into();
    };
    let peers: Vec<String> = peers.iter().map(|&(p, m)| format!("{}/{}", n.of(p), n.of(m))).collect();
    format!("{line} peers=[{}]", peers.join(","))
}

#[test]
fn dshow_mpeg_objects_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0xd5_3e02);
    // (a fixed path: the names given show in the lines and in the buffers hashed)
    let dir = std::path::PathBuf::from("/tmp/masu-dsmpeg-golden");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("junk.mpg"), b"not a movie at all").unwrap();
    std::fs::write(dir.join("riff.avi"), b"RIFF\x10\0\0\0AVI LIST and some more").unwrap();
    let mut asf = Vec::new();
    asf.extend(ASF_HEADER);
    asf.extend(30u64.to_le_bytes());
    asf.extend(0u32.to_le_bytes());
    asf.extend([1, 2]);
    std::fs::write(dir.join("empty.wmv"), &asf).unwrap();
    let zdir = format!("Z:{}", dir.to_str().unwrap().replace('/', "\\"));
    let files = [format!("{zdir}\\junk.mpg"), format!("{zdir}\\riff.avi"), format!("{zdir}\\empty.wmv"), format!("{zdir}\\none.mpg"), "Input".into(), "Output".into(), "Video".into(), "".into()];
    let (cls, iid, po, po2, po3, wname, big, mt, fmt, arr) = (
        mem::heap_alloc(16, true),
        mem::heap_alloc(16, true),
        mem::heap_alloc(128, true),
        mem::heap_alloc(128, true),
        mem::heap_alloc(128, true),
        mem::heap_alloc(1024, true),
        mem::heap_alloc(2048, true),
        mem::heap_alloc(128, true),
        mem::heap_alloc(128, true),
        mem::heap_alloc(16, true),
    );
    let fakes: Vec<u32> = (0..3).map(fake).collect();
    for (i, &f) in (0u32..).zip(&fakes[..2]) {
        wp(arr + ps() * i, f);
    }
    let classes = [CLSID_MPEG1_SPLITTER, CLSID_MPEG_VIDEO_CODEC, CLSID_MPEG_AUDIO_CODEC, CLSID_DSOUND_RENDER, CLSID_ASYNC_READER, CLSID_WMASF_READER, dmo::CLSID_DMO_WRAPPER, IID_IPIN];
    let iids = [IID_IUNKNOWN, IID_IPERSIST, IID_IBASEFILTER, IID_IMEDIAFILTER, IID_IPIN, IID_IMEMINPUTPIN, IID_IFILESOURCEFILTER, IID_IASYNCREADER, IID_IAMSTREAMSELECT, IID_IENUMPINS, dmo::IID_IDMOWRAPPERFILTER, [7; 16]];
    let majors = [MEDIATYPE_VIDEO, MEDIATYPE_AUDIO, MEDIATYPE_STREAM, GUID_NULL];
    let subs = [SUBTYPE_RGB32, SUBTYPE_RGB24, SUBTYPE_RGB565, SUBTYPE_RGB555, SUBTYPE_MPEG1_PAYLOAD, SUBTYPE_MPEG1_AUDIO_PAYLOAD, SUBTYPE_PCM, SUBTYPE_MPEG1_SYSTEM, SUBTYPE_MPEG1_VIDEO, SUBTYPE_ASF, GUID_NULL];
    let ftypes = [FORMAT_VIDEO_INFO, FORMAT_WAVE_FORMAT_EX, FORMAT_MPEG_VIDEO, GUID_NULL];
    let mut names = Names { filters: Vec::new(), fakes: fakes.clone() };
    for i in 0..4000 {
        for p in [po, po2, po3] {
            mem::fill(p, 128, 0);
            w32(p, 0xdead_beef);
        }
        mem::fill(big, 2048, 0);
        mem::write(iid, &r.pick(&iids));
        let fname = &files[usize::try_from(r.below(8)).unwrap_or(0)];
        mem::write(wname, &fname.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect::<Vec<u8>>());
        let live: Vec<usize> = (0..names.filters.len()).filter(|&j| names.filters[j] != 0).collect();
        if live.len() < 3 || r.below(50) == 0 {
            if names.filters.len() >= 30 {
                continue;
            }
            let c = usize::try_from(r.below(8)).unwrap_or(0);
            mem::write(cls, &classes[c]);
            let got = create_instance(cls, iid);
            let shown = match got {
                Some(p) if p != 0 => {
                    let b = ours(p).map_or(p, |x| x.0);
                    names.filters.push(b);
                    names.of(p)
                }
                g => format!("{g:?}"),
            };
            writeln!(out, "{i} create cls#{c}: {shown}").unwrap();
            continue;
        }
        let j = live[usize::try_from(r.below(u64::try_from(live.len()).unwrap_or(1))).unwrap_or(0)];
        let b = names.filters[j];
        let k = u32::try_from(r.below(u64::from(Word::N))).unwrap_or(0);
        let w = Word::ALL.into_iter().find(|w| w.index() == k).unwrap_or(Word::Filter);
        let this = w.of(b);
        let vt = rp(this);
        let n = methods(vt);
        let spec = specs(w);
        assert_eq!(spec.len(), usize::try_from(n).unwrap_or(0), "interface {k}");
        let m = u32::try_from(r.below(u64::from(n))).unwrap_or(0);
        let Some(f) = handler_of(rp(vt + ps() * m)) else { continue };
        let pins: Vec<u32> = live.iter().flat_map(|&j| (1..4).map(move |k| (j, k))).map(|(j, k)| names.filters[j] + ps() * k).chain(fakes.iter().copied()).collect();
        let mut args = vec![this];
        let mut shown = Vec::new();
        let ptr = |r: &mut Rng, p: u32| if r.below(10) == 0 { 0 } else { p };
        for kind in spec[usize::try_from(m).unwrap_or(0)].chars() {
            let (a, s) = match kind {
                'p' => {
                    let p = if r.below(8) == 0 { 0 } else { r.pick(&[po, po2, po3]) };
                    (p, match p { 0 => "0".to_string(), p if p == po => "po".into(), p if p == po2 => "po2".into(), _ => "po3".into() })
                }
                'i' => (iid, "iid".into()),
                'w' => (ptr(&mut r, wname), format!("{:?}", fname.rsplit('\\').next().unwrap_or(""))),
                'P' => {
                    let p = if r.below(8) == 0 { 0 } else { pins[usize::try_from(r.below(u64::try_from(pins.len()).unwrap_or(1))).unwrap_or(0)] };
                    (p, names.of(p))
                }
                'G' => {
                    let g = if r.below(4) == 0 { 0 } else { r.pick(&fakes) };
                    (g, names.of(g))
                }
                'S' => (arr, "arr".into()),
                'm' => {
                    let (ma, su, ft) = (usize::try_from(r.below(4)).unwrap_or(0), usize::try_from(r.below(11)).unwrap_or(0), usize::try_from(r.below(4)).unwrap_or(0));
                    mem::fill(mt, 128, 0);
                    mem::write(mt, &majors[ma]);
                    mem::write(mt + 16, &subs[su]);
                    let (wd, ht) = (r.pick(&[0u32, 1, 16, 320]), r.pick(&[0u32, 1, 16, 240]));
                    let b = r.pick(&[32u16, 24, 16, 0]);
                    let fb = if ft == 1 { wave_format(1, 2, 44100, 16, &[]) } else { video_info(wd, ht, b, 0, 30.0) };
                    let fb = &fb[..fb.len().min(128)];
                    mem::write(fmt, fb);
                    // (AM_MEDIA_TYPE: formattype at 44, then pUnk, cbFormat, pbFormat)
                    let (cb, pb) = mt_fmt_at();
                    mem::write(mt + 44, &ftypes[ft]);
                    w32(mt + cb, u32::try_from(fb.len()).unwrap_or(0));
                    wp(mt + pb, fmt);
                    (ptr(&mut r, mt), format!("mt{ma}/{su}/{ft}/{wd}x{ht}x{b}"))
                }
                'v' => {
                    let v = r.pick(&[0u32, 1, 2, 3, 100, 0xffff_ffff, 0x8000_0000, 1000]);
                    (v, format!("{v:#x}"))
                }
                _ => (ptr(&mut r, big), "big".into()),
            };
            args.push(a);
            shown.push(if a == 0 { "0".into() } else { s });
        }
        let hr = call_stacked(f, &args);
        let outs = [po, po2, po3].map(|p| format!("{}/{}/{}/{:016x}", names.of(rp(p)), names.of(r32(p + 4)), names.of(r32(p + 8)), scrubbed(p, 128, &names)));
        let info = format!("{}/{:016x}", names.of(rp(big + 256)), scrubbed(big, 2048, &names));
        let st = state(b, &names);
        writeln!(out, "{i} F{j}.{k}.{m} [{}]: {hr:#x} out=[{}] big={info} log=[{}] {st}", shown.join(" "), outs.join(" "), take_log()).unwrap();
        if st == "gone" {
            names.filters[j] = 0;
        }
    }
    for p in [cls, iid, po, po2, po3, wname, big, mt, fmt, arr] {
        mem::heap_free(p);
    }
    std::fs::remove_dir_all(&dir).ok();
    snapshot("dshow_mpeg_objects", &out);
}
