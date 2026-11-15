//! Golden sweep of the Windows Media sync reader's COM side (wmvcore.rs) before its typed
//! rewrite: IWMSyncReader2, IWMProfile3, IWMHeaderInfo3 and what they hand out (IWMStreamConfig,
//! IWMOutputMediaProps, INSSBuffer), over the test files (and over no file). Every handler is
//! called straight from its method table with seeded arguments; each line is its answer and what
//! it wrote.

use super::*;
use crate::test_guest::{call_stacked as call, enter, guest_thread, snapshot, Rng};
use kome::num::{lo16, lo32, u32_of};
use std::fmt::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Sync,
    Profile,
    Header,
    Config,
    Props,
    Buffer,
}

impl Kind {
    fn methods(self) -> &'static [(Handler, u32)] {
        match self {
            Kind::Sync => M_SYNC,
            Kind::Profile => M_PROFILE,
            Kind::Header => M_HEADER,
            Kind::Config => M_CONFIG,
            Kind::Props => M_PROPS,
            Kind::Buffer => M_BUFFER,
        }
    }
    /// The method's x86 argument words (after this): i an IID, C an interface out (a new child
    /// or one known), p an out, P a pointer out, l a length in/out, b a buffer, M a media type
    /// buffer (in or out), v a value, q a QWORD, w a string, U an object of the sweep, z zero,
    /// n a buffer of stream numbers, s a buffer of selections. Unnamed ones: values.
    fn spec(self, j: usize) -> String {
        let named = match (self, j) {
            (_, 0) => Some("iC"),
            (Kind::Sync, 3) => Some("w"),
            (Kind::Sync, 5) => Some("qq"),
            (Kind::Sync, 6) => Some("vqq"),
            (Kind::Sync, 7) => Some("vCppppp"),
            (Kind::Sync, 8) => Some("vns"),
            (Kind::Sync, 9 | 11 | 17 | 19 | 20 | 21 | 22 | 27 | 29) => Some("vp"),
            (Kind::Sync, 12) => Some("vwpbl"),
            (Kind::Sync, 13) => Some("vwvbv"),
            (Kind::Sync, 14) => Some("p"),
            (Kind::Sync, 15) => Some("vC"),
            (Kind::Sync, 16) => Some("vU"),
            (Kind::Sync, 18) => Some("vvC"),
            (Kind::Sync, 23) => Some("z"),
            (Kind::Sync, 26 | 28) => Some("vz"),
            (Kind::Profile, 3 | 8 | 16 | 24 | 29) => Some("p"),
            (Kind::Profile, 4 | 6) => Some("bl"),
            (Kind::Profile, 9 | 10) => Some("vC"),
            (Kind::Header, 3 | 17) => Some("vp"),
            (Kind::Header, 4) => Some("vlblpbl"),
            (Kind::Header, 5) => Some("lwpbl"),
            (Kind::Header, 7 | 11 | 15) => Some("p"),
            (Kind::Config, 3 | 4 | 10 | 12) => Some("p"),
            (Kind::Config, 6 | 8) => Some("bl"),
            (Kind::Props, 3) => Some("p"),
            (Kind::Props, 4) => Some("Ml"),
            (Kind::Props, 5) => Some("M"),
            (Kind::Props, 6 | 7) => Some("bl"),
            (Kind::Buffer, 3 | 5) => Some("p"),
            (Kind::Buffer, 4) => Some("v"),
            (Kind::Buffer, 6) => Some("P"),
            (Kind::Buffer, 7) => Some("Pp"),
            _ => None,
        };
        let words = usize::try_from(self.methods()[j].1 / 4 - 1).unwrap_or(0);
        named.map_or_else(|| "v".repeat(words), str::to_string)
    }
    /// What a method's C out is (None: by the pointer, for QueryInterface).
    fn child(self, j: usize) -> Option<Kind> {
        match (self, j) {
            (Kind::Sync, 7) => Some(Kind::Buffer),
            (Kind::Sync, 15 | 18) => Some(Kind::Props),
            (Kind::Profile, 9 | 10) => Some(Kind::Config),
            _ => None,
        }
    }
}

const ALL: [Kind; 6] = [Kind::Sync, Kind::Profile, Kind::Header, Kind::Config, Kind::Props, Kind::Buffer];

fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3))
}

/// The bytes at `p` (`n` of them), words that point into them made offsets (a media type's
/// pbFormat), hashed.
fn norm_hash(p: u32, n: u32) -> String {
    let mut b = mem::bytes(p, n);
    for w in b.chunks_exact_mut(4) {
        let v = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
        if (p..p + n).contains(&v) {
            w.copy_from_slice(&(v - p).to_le_bytes());
        }
    }
    format!("{:x}", fnv(&b))
}

struct Sweep {
    out: String,
    r: Rng,
    /// every object the sweep holds, by the order it came (a released one's address can come back)
    objs: Vec<(u32, Kind)>,
    names: Vec<u32>,
    reader: u32,
    bufs: [u32; 3],
    lens: [u32; 3],
    outs: [u32; 6],
    mt: u32,
    strs: Vec<u32>,
    iids: Vec<(u32, &'static str)>,
    nums: u32,
    sels: u32,
    /// the file, opened again after each Close
    file: Option<Vec<u8>>,
}

impl Sweep {
    fn name(&self, o: u32) -> String {
        if o == 0 {
            return "0".into();
        }
        self.names.iter().rposition(|&x| x == o).map_or_else(|| format!("{o:#x}?"), |i| format!("#{i}"))
    }
    fn add(&mut self, o: u32, k: Kind) -> String {
        self.names.push(o);
        self.objs.push((o, k));
        format!("{}:{k:?}", self.name(o))
    }
    /// What kind of object `p` is: one of the reader's interfaces, or one held.
    fn kind_of(&self, p: u32) -> Option<Kind> {
        let w = ps();
        match p.checked_sub(self.reader) {
            Some(0) => Some(Kind::Sync),
            Some(d) if d == w => Some(Kind::Profile),
            Some(d) if d == 2 * w => Some(Kind::Header),
            _ => self.objs.iter().rev().find(|o| o.0 == p).map(|o| o.1).or_else(|| {
                // (a stream config's second word is its IWMMediaProps)
                let held = |q: u32| self.objs.iter().any(|o| o.0 == q && o.1 == Kind::Config);
                p.checked_sub(w).filter(|&q| held(q)).map(|_| Kind::Props)
            }),
        }
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.r.below(u64::try_from(n).unwrap_or(1))).unwrap_or(0)
    }

    fn step(&mut self, i: usize) {
        let k = self.below(self.objs.len());
        let (this, kind) = self.objs[k];
        let ms = kind.methods();
        let j = self.below(ms.len());
        // (a release is the end of an object: the sweep lets its children go at the cap only)
        if matches!(j, 1 | 2) {
            // (an AddRef, then the Release: the count where it was)
            let up = call(ms[1].0, &[this]);
            let down = call(ms[2].0, &[this]);
            writeln!(self.out, "{i} {} {kind:?}.ref: {up} {down}", self.name(this)).unwrap();
            return;
        }
        let spec = kind.spec(j);
        let mut args = vec![this];
        let mut shown = Vec::new();
        let (mut nb, mut nl, mut no) = (0, 0, 0);
        // what to read back after the call: (kind, address)
        let mut back: Vec<(char, u32)> = Vec::new();
        for ch in spec.chars() {
            match ch {
                'v' => {
                    let v = self.r.pick(&[0u32, 0, 1, 1, 2, 3, 0xffff, 0xffff_ffff]);
                    args.push(v);
                    shown.push(v.to_string());
                }
                'q' => {
                    let v = self.r.pick(&[0u64, 5_000_000, 10_000_000, 1 << 40, u64::MAX]);
                    args.extend([lo32(v), lo32(v >> 32)]);
                    shown.push(v.to_string());
                }
                'z' => {
                    args.push(0);
                    shown.push("0".into());
                }
                'w' => {
                    let k = self.below(self.strs.len() + 1);
                    let p = self.strs.get(k).copied().unwrap_or(0);
                    args.push(p);
                    shown.push(if p == 0 { "0".into() } else { format!("w{k}") });
                }
                'i' => {
                    let k = self.below(self.iids.len());
                    let (iid, n) = self.iids[k];
                    args.push(iid);
                    shown.push(n.into());
                }
                'U' => {
                    let k = self.below(self.objs.len() + 1);
                    let p = self.objs.get(k).map_or(0, |o| o.0);
                    args.push(p);
                    shown.push(self.name(p));
                }
                'n' | 's' => {
                    let (p, vals): (u32, &[u32]) = if ch == 'n' { (self.nums, &[0, 1, 2, 3]) } else { (self.sels, &[0, 1, 2, 3]) };
                    let v: Vec<u32> = (0..4).map(|_| self.r.pick(vals)).collect();
                    for (k, &x) in (0u32..).zip(&v) {
                        if ch == 'n' {
                            w16(p + 2 * k, lo16(x));
                        } else {
                            w32(p + 4 * k, x);
                        }
                    }
                    args.push(p);
                    shown.push(format!("{v:?}"));
                }
                'b' => {
                    let p = if self.r.below(8) == 0 { 0 } else { self.bufs[nb] };
                    nb += 1;
                    if p != 0 {
                        mem::fill(p, 512, 0xcc);
                        back.push(('b', p));
                    }
                    args.push(p);
                    shown.push(if p == 0 { "0".into() } else { "b".into() });
                }
                'M' => {
                    let p = if self.r.below(8) == 0 { 0 } else { self.mt };
                    if p != 0 {
                        back.push(('M', p));
                    }
                    args.push(p);
                    shown.push(if p == 0 { "0".into() } else { "M".into() });
                }
                'l' => {
                    let p = if self.r.below(8) == 0 { 0 } else { self.lens[nl] };
                    nl += 1;
                    let v = self.r.pick(&[0u32, 2, 8, 32, 64, 0x100]);
                    if p != 0 {
                        w32(p, v);
                        back.push(('l', p));
                    }
                    args.push(p);
                    shown.push(if p == 0 { "0".into() } else { format!("l{v}") });
                }
                'p' | 'P' | 'C' => {
                    let p = if self.r.below(10) == 0 { 0 } else { self.outs[no] };
                    no += 1;
                    if p != 0 {
                        mem::fill(p, 16, 0xcc);
                        back.push((ch, p));
                    }
                    args.push(p);
                    shown.push(if p == 0 { "0".into() } else { ch.to_string() });
                }
                _ => unreachable!("{ch}"),
            }
        }
        let hr = call(ms[j].0, &args);
        let mut line = format!("{i} {} {kind:?}.m{j}({}): {hr:#x}", self.name(this), shown.join(","));
        for (ch, p) in back {
            let s = match ch {
                'b' => norm_hash(p, 512),
                'M' => norm_hash(p, 512),
                'l' => r32(p).to_string(),
                'p' => format!("{:02x?}", &mem::array::<8>(p)),
                'P' => (if rp(p) == 0 { "null" } else { "ptr" }).into(),
                'C' => {
                    let o = rp(p);
                    if o == 0xcccc_cccc && !mem::x64() {
                        "untouched".into()
                    } else if o == 0 {
                        "0".into()
                    } else if hr != 0 {
                        format!("{o:#x}!")
                    } else if let Some(k) = self.kind_of(o).or_else(|| (j == 0 && kind == Kind::Props && o.checked_add(ps()) == Some(this)).then_some(Kind::Config)).filter(|_| j == 0 || kind.child(j).is_none()) {
                        // (QueryInterface: a reference more on what it names)
                        self.objs.push((o, k));
                        if !self.names.contains(&o) {
                            self.names.push(o);
                        }
                        format!("{}:{k:?}", self.name(o))
                    } else if let Some(k) = kind.child(j) {
                        self.add(o, k)
                    } else {
                        format!("{o:#x}?")
                    }
                }
                _ => unreachable!(),
            };
            write!(line, " {ch}={s}").unwrap();
        }
        if (kind, j, hr) == (Kind::Sync, 4, 0) && let Some(b) = &self.file {
            let rr = super::reader(self.reader).unwrap();
            write!(line, " reopened {:#x}", open_src(&mut lock(&rr), Box::new(b.clone()))).unwrap();
        }
        writeln!(self.out, "{line}").unwrap();
        // (no more than a few children held: the oldest let go)
        let kids: Vec<usize> = (0..self.objs.len()).filter(|&k| !matches!(self.objs[k].1, Kind::Sync | Kind::Profile | Kind::Header)).collect();
        if kids.len() > 12 {
            let (o, k) = self.objs.remove(kids[0]);
            let left = call(k.methods()[2].0, &[o]);
            writeln!(self.out, "  let go {} {k:?}: {left}", self.name(o)).unwrap();
        }
    }
}

fn wide(s: &str) -> u32 {
    let p = mem::heap_alloc(2 * u32_of(s.len()) + 2, true);
    for (i, ch) in (0u32..).zip(s.encode_utf16()) {
        w16(p + 2 * i, ch);
    }
    p
}

fn iid(g: &Guid) -> u32 {
    let p = mem::heap_alloc(16, true);
    mem::write(p, g);
    p
}

#[test]
fn wmvcore_sync_reader_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    for (k, kind) in ALL.iter().enumerate() {
        let ms = kind.methods();
        for j in 0..ms.len() {
            let s = kind.spec(j);
            let words = s.len() + s.matches('q').count();
            assert_eq!(4 * (1 + u32_of(words)), ms[j].1, "{kind:?} m{j} {s}");
        }
        writeln!(out, "{k} {kind:?}: {} methods", ms.len()).unwrap();
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/mf/testdata");
    let strs = ["Duration", "Title", "WM/VideoWidth", "EnableDiscreteOutput", "bogus", ""].map(wide).to_vec();
    let iids = vec![
        (iid(&IID_IUNKNOWN), "unk"),
        (iid(&IID_SYNC_READER), "sync"),
        (iid(&IID_SYNC_READER2), "sync2"),
        (iid(&IID_PROFILE), "profile"),
        (iid(&IID_PROFILE3), "profile3"),
        (iid(&IID_HEADER_INFO), "header"),
        (iid(&IID_HEADER_INFO3), "header3"),
        (iid(&IID_STREAM_CONFIG), "config"),
        (iid(&IID_MEDIA_PROPS), "props"),
        (iid(&IID_OUTPUT_MEDIA_PROPS), "oprops"),
        (iid(&IID_NSSBUFFER), "buffer"),
        (iid(&asyncr::IID_READER), "reader"),
        (iid(&[7; 16]), "other"),
    ];
    let a = |n| mem::heap_alloc(n, true);
    for (seed, file) in [(1u32, Some("colors.wmv")), (2, Some("tone.wma")), (3, None)] {
        let reader = new_sync_reader();
        let bytes = file.map(|f| std::fs::read(dir.join(f)).unwrap());
        let opened = bytes.clone().map(|b| open_src(&mut lock(&super::reader(reader).unwrap()), Box::new(b)));
        writeln!(out, "== {file:?} {opened:#x?}").unwrap();
        let w = ps();
        let mut s = Sweep {
            out: String::new(),
            r: Rng(0x0057_ee90 + u64::from(seed)),
            objs: vec![(reader, Kind::Sync), (reader + w, Kind::Profile), (reader + 2 * w, Kind::Header)],
            names: vec![reader, reader + w, reader + 2 * w],
            reader,
            bufs: [a(512), a(512), a(512)],
            lens: [a(4), a(4), a(4)],
            outs: [a(16), a(16), a(16), a(16), a(16), a(16)],
            mt: a(512),
            strs: strs.clone(),
            iids: iids.clone(),
            nums: a(8),
            sels: a(16),
            file: bytes,
        };
        for i in 0..if file.is_some() { 8000 } else { 600 } {
            s.step(i);
        }
        out.push_str(&s.out);
        // (every child the sweep holds let go, then the reader's references)
        for (o, k) in s.objs.iter().rev() {
            if !matches!(k, Kind::Sync | Kind::Profile | Kind::Header) {
                call(k.methods()[2].0, &[*o]);
            }
        }
        while table(|m| m.contains_key(&reader)) {
            call(release, &[reader]);
        }
    }
    snapshot("wmvcore_sync_reader", &out);
}
