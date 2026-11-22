//! The immediate and deferred contexts give one meaning to every state call (op.rs, stage 3 of
//! the review of 6852be1): through the real entry point (`d3d11::call`), any sequence of
//! them (every stage's resource views, samplers and constant buffers with and without ranges,
//! slots up to the last, vertex buffers to slot 31, the index buffer, render targets with and
//! without KEEP, either stage's UAVs with initial counts, stream output, viewports, scissors,
//! blend factors, ClearState) leaves the immediate context, the deferred context that recorded
//! it, and the command list it finished (run from its start) in the same state, which every
//! getter reads alike; and what a list names lives as long as the list, then goes.

use kome::locks::Lock as _;
use super::*;
use super::deferred_tests::{call1, m, mem, put, rd1, setup};
use proptest::prelude::*;

fn id_of(a: u32) -> u32 {
    let g = D11S.locked();
    g.as_ref().map_or(0, |d| d.id_of(a))
}
fn release(a: u32) -> u32 {
    let iface = {
        let g = D11S.locked();
        let d = g.as_ref().unwrap();
        d.objs[&d.id_of(a)].faces.iter().find(|f| f.1 == a).unwrap().0
    };
    call1(m(iface, 2), &[a])
}
/// Every object's (refs, binds).
fn counts() -> Vec<(u32, u32, u32)> {
    let g = D11S.locked();
    let d = g.as_ref().unwrap();
    let mut v: Vec<_> = d.objs.iter().map(|(&id, o)| (id, o.refs, o.binds)).collect();
    v.sort();
    v
}

/// The objects a case binds: pools by kind (index 0 NULL in each).
struct Pool {
    dev: u32,
    imm: u32,
    srvs: Vec<u32>,
    rtvs: Vec<u32>,
    dsvs: Vec<u32>,
    uavs: Vec<u32>,
    bufs: Vec<u32>,
    cbs: Vec<u32>,
    samplers: Vec<u32>,
}

fn pool() -> Pool {
    let (dev_out, level, ctx_out) = (mem(2), mem(2), mem(2));
    assert_eq!(call1(CREATE_DEVICE, &[0, 1, 0, 0, 0, 0, 7, 0, 0, dev_out, level, ctx_out]), 0);
    let (dev, imm, o) = (rd1(dev_out), rd1(ctx_out), mem(2));
    let tex = |w: u32, mips: u32, fmt: u32, bind: u32| {
        let d = mem(11);
        put(d, &[w, w, mips, 1, fmt, 1, 0, 0, bind, 0, 0]);
        assert_eq!(call1(m(IF_DEVICE, 5), &[dev, d, 0, o]), 0);
        rd1(o)
    };
    let a = tex(16, 2, 28, 0x8 | 0x20 | 0x80);
    let z = tex(16, 1, 44, 0x8 | 0x40);
    let view = |kind: u32, res: u32, words: &[u32]| {
        let d = mem(8);
        put(d, words);
        assert_eq!(call1(m(IF_DEVICE, kind), &[dev, res, d, o]), 0, "view {kind} {words:?}");
        rd1(o)
    };
    let buffer = |size: u32, bind: u32| {
        let d = mem(6);
        put(d, &[size, 0, bind, 0, 0, 0]);
        assert_eq!(call1(m(IF_DEVICE, 3), &[dev, d, 0, o]), 0);
        rd1(o)
    };
    let one = 1f32.to_bits();
    let s = mem(13);
    put(s, &[0x15, 1, 1, 1, 0, 16, 8, 0, one, 0, one, 0, 16f32.to_bits()]);
    assert_eq!(call1(m(IF_DEVICE, 23), &[dev, s, o]), 0);
    let sampler = rd1(o);
    let p = Pool {
        dev,
        imm,
        srvs: vec![0, view(7, a, &[28, 4, 0, 1, 0, 0]), view(7, a, &[28, 4, 1, 1, 0, 0]), view(7, z, &[46, 4, 0, 1, 0, 0])],
        rtvs: vec![0, view(9, a, &[28, 4, 0, 0, 0]), view(9, a, &[28, 4, 1, 0, 0])],
        dsvs: vec![0, view(10, z, &[45, 3, 0, 0, 0, 0]), view(10, z, &[45, 3, 3, 0, 0, 0])],
        uavs: vec![0, view(8, a, &[28, 4, 0, 0, 0])],
        bufs: vec![0, buffer(256, 0x1 | 0x2 | 0x10), buffer(64, 0x1)],
        cbs: vec![0, buffer(256, 0x4), buffer(4096, 0x4)],
        samplers: vec![0, sampler],
    };
    // (the views hold their textures)
    release(a);
    release(z);
    p
}

/// One state call: pools' indices (any index: modulo the pool's size).
#[derive(Debug, Clone)]
enum Call {
    /// (setter method, start, views)
    Srvs(u32, u32, Vec<u8>),
    Samplers(u32, u32, Vec<u8>),
    /// (setter, start, buffers, (first, count) per slot for *SetConstantBuffers1)
    Cbs(u32, u32, Vec<u8>, Option<Vec<(u32, u32)>>),
    /// (start, (buffer, stride, offset))
    Vbs(u32, Vec<(u8, u32, u32)>),
    Ib(u8, u32, u32),
    /// OMSetRenderTargets(rtvs, dsv)
    Om(Vec<u8>, u8),
    /// ...AndUnorderedAccessViews: render targets (None: KEEP), UAVs (None: KEEP) from start,
    /// with or without initial counts
    OmUavs(Option<(Vec<u8>, u8)>, Option<(u32, Vec<u8>, bool)>),
    CsUavs(u32, Vec<u8>, bool),
    So(Vec<u8>, Option<Vec<u32>>),
    Viewports(Option<Vec<[u32; 6]>>),
    Scissors(Option<Vec<[u32; 4]>>),
    Blend(Option<[u32; 4]>, u32),
    Topology(u32),
    ClearState,
}

fn call_strategy() -> impl Strategy<Value = Call> {
    let objs = |n: usize| prop::collection::vec(any::<u8>(), 0..n);
    // (starts near the ends too: the last slots, and past them)
    let start = |last: u32| prop_oneof![0u32..4, (last - 3)..=last, Just(last + 1)];
    prop_oneof![
        (prop::sample::select(vec![25u32, 8, 31, 59, 63, 67]), start(127), objs(6)).prop_map(|(m, s, v)| Call::Srvs(m, s, v)),
        (prop::sample::select(vec![26u32, 10, 32, 61, 65, 70]), start(15), objs(4)).prop_map(|(m, s, v)| Call::Samplers(m, s, v)),
        (prop::sample::select(vec![7u32, 16, 22, 62, 66, 71, 119, 120, 121, 122, 123, 124]), start(14), objs(4), prop::option::of(prop::collection::vec((prop::sample::select(vec![0u32, 16, 8, 32]), prop::sample::select(vec![16u32, 4096, 8, 0, 4112])), 4)))
            .prop_map(|(m, s, v, r)| Call::Cbs(m, s, v, if m >= 119 { r } else { None })),
        (start(31), prop::collection::vec((any::<u8>(), 0u32..64, 0u32..64), 0..4)).prop_map(|(s, v)| Call::Vbs(s, v)),
        (any::<u8>(), prop::sample::select(vec![42u32, 57]), 0u32..16).prop_map(|(b, f, o)| Call::Ib(b, f, o)),
        (objs(3), any::<u8>()).prop_map(|(r, d)| Call::Om(r, d)),
        (prop::option::of((objs(3), any::<u8>())), prop::option::of((0u32..9, objs(3), any::<bool>()))).prop_map(|(r, u)| Call::OmUavs(r, u)),
        (0u32..9, objs(3), any::<bool>()).prop_map(|(s, v, c)| Call::CsUavs(s, v, c)),
        (objs(5), prop::option::of(prop::collection::vec(prop::sample::select(vec![0u32, 16, u32::MAX]), 5))).prop_map(|(b, o)| Call::So(b, o)),
        // (floats as bits: whole numbers, NaN compares unequal to itself)
        prop::option::of(prop::collection::vec(prop::array::uniform6(0u32..2000).prop_map(|v| v.map(|x| (x as f32).to_bits())), 0..17)).prop_map(Call::Viewports),
        prop::option::of(prop::collection::vec(any::<[u32; 4]>(), 0..17)).prop_map(Call::Scissors),
        (prop::option::of(prop::array::uniform4(0u32..4).prop_map(|v| v.map(|x| (x as f32 / 2.0).to_bits()))), any::<u32>()).prop_map(|(f, s)| Call::Blend(f, s)),
        (0u32..40).prop_map(Call::Topology),
        Just(Call::ClearState),
    ]
}

/// An array of the pool's objects at `ix` (as guest pointers).
fn arr(pool: &[u32], ix: &[u8]) -> u32 {
    let p = mem(lo32(ix.len()).max(1) * 2);
    put(p, &ix.iter().map(|&i| pool[usize::from(i) % pool.len()]).collect::<Vec<_>>());
    p
}
fn words(v: &[u32]) -> u32 {
    let p = mem(lo32(v.len()).max(1));
    put(p, v);
    p
}

fn make(p: &Pool, ctx: u32, c: &Call) {
    let pick = |pool: &[u32], i: u8| pool[usize::from(i) % pool.len()];
    match c {
        Call::Srvs(meth, s, v) => {
            call1(m(IF_CONTEXT, *meth), &[ctx, *s, lo32(v.len()), arr(&p.srvs, v)]);
        }
        Call::Samplers(meth, s, v) => {
            call1(m(IF_CONTEXT, *meth), &[ctx, *s, lo32(v.len()), arr(&p.samplers, v)]);
        }
        Call::Cbs(meth, s, v, r) => {
            let n = lo32(v.len());
            let (firsts, nums) = r.as_ref().map_or((0, 0), |r| (words(&r.iter().map(|x| x.0).collect::<Vec<_>>()), words(&r.iter().map(|x| x.1).collect::<Vec<_>>())));
            call1(m(IF_CONTEXT, *meth), &[ctx, *s, n, arr(&p.cbs, v), firsts, nums]);
        }
        Call::Vbs(s, v) => {
            let bufs: Vec<u8> = v.iter().map(|x| x.0).collect();
            let strides = words(&v.iter().map(|x| x.1).collect::<Vec<_>>());
            let offs = words(&v.iter().map(|x| x.2).collect::<Vec<_>>());
            call1(m(IF_CONTEXT, 18), &[ctx, *s, lo32(v.len()), arr(&p.bufs, &bufs), strides, offs]);
        }
        Call::Ib(b, f, o) => {
            call1(m(IF_CONTEXT, 19), &[ctx, pick(&p.bufs, *b), *f, *o]);
        }
        Call::Om(r, d) => {
            call1(m(IF_CONTEXT, 33), &[ctx, lo32(r.len()), arr(&p.rtvs, r), pick(&p.dsvs, *d)]);
        }
        Call::OmUavs(r, u) => {
            let (n, rtvs, dsv) = r.as_ref().map_or((0xffff_ffff, 0, 0), |(r, d)| (lo32(r.len()), arr(&p.rtvs, r), pick(&p.dsvs, *d)));
            let (start, un, uavs, init) = u.as_ref().map_or((0, 0xffff_ffff, 0, 0), |(s, v, c)| (*s, lo32(v.len()), arr(&p.uavs, v), if *c { words(&[3, u32::MAX, 7]) } else { 0 }));
            call1(m(IF_CONTEXT, 34), &[ctx, n, rtvs, dsv, start, un, uavs, init]);
        }
        Call::CsUavs(s, v, c) => {
            call1(m(IF_CONTEXT, 68), &[ctx, *s, lo32(v.len()), arr(&p.uavs, v), if *c { words(&[5, 6, u32::MAX]) } else { 0 }]);
        }
        Call::So(b, o) => {
            call1(m(IF_CONTEXT, 37), &[ctx, lo32(b.len()), arr(&p.bufs, b), o.as_ref().map_or(0, |o| words(o))]);
        }
        Call::Viewports(v) => {
            let n = v.as_ref().map_or(1, |v| lo32(v.len()));
            call1(m(IF_CONTEXT, 44), &[ctx, n, v.as_ref().map_or(0, |v| words(&v.concat()))]);
        }
        Call::Scissors(v) => {
            let n = v.as_ref().map_or(1, |v| lo32(v.len()));
            call1(m(IF_CONTEXT, 45), &[ctx, n, v.as_ref().map_or(0, |v| words(&v.concat()))]);
        }
        Call::Blend(f, mask) => {
            call1(m(IF_CONTEXT, 35), &[ctx, 0, f.map_or(0, |f| words(&f)), *mask]);
        }
        Call::Topology(t) => {
            call1(m(IF_CONTEXT, 24), &[ctx, *t]);
        }
        Call::ClearState => {
            call1(m(IF_CONTEXT, 110), &[ctx]);
        }
    }
}

/// What every getter gives on `ctx` (the references they give back dropped).
fn gets(ctx: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut take = |meth: u32, args: &[u32], words: &[(u32, u32, bool)]| {
        // (each array filled with a mark first; `true`: an array of references)
        let ps: Vec<u32> = words.iter().map(|&(n, _, _)| {
            let p = mem(n.max(1) * 2);
            put(p, &vec![0xdead; us(n.max(1) * 2)]);
            p
        }).collect();
        let mut a = vec![ctx];
        a.extend_from_slice(args);
        let mut k = 0;
        for x in a.iter_mut() {
            if *x == u32::MAX - 1 {
                *x = ps[k];
                k += 1;
            }
        }
        call1(m(IF_CONTEXT, meth), &a);
        for (&p, &(n, _, refs)) in ps.iter().zip(words) {
            for i in 0..n {
                let v = rd1(p + 4 * i);
                if refs && v != 0 && v != 0xdead {
                    release(v);
                }
                out.push(v);
            }
        }
    };
    const P: u32 = u32::MAX - 1;
    let mut counts = Vec::new();
    for meth in [84, 73, 87, 97, 101, 105] {
        take(meth, &[0, 128, P], &[(128, 0, true)]);
    }
    for meth in [85, 75, 88, 99, 103, 108] {
        take(meth, &[0, 16, P], &[(16, 0, true)]);
    }
    for meth in 125..=130 {
        take(meth, &[0, 15, P, P, P], &[(15, 0, true), (15, 0, false), (15, 0, false)]);
    }
    take(79, &[0, 32, P, P, P], &[(32, 0, true), (32, 0, false), (32, 0, false)]);
    take(80, &[P, P, P], &[(1, 0, true), (1, 0, false), (1, 0, false)]);
    take(90, &[8, P, P, 0, 8, P], &[(8, 0, true), (1, 0, true), (8, 0, true)]);
    take(106, &[0, 8, P], &[(8, 0, true)]);
    take(93, &[4, P], &[(4, 0, true)]);
    take(91, &[P, P, P], &[(1, 0, true), (4, 0, false), (1, 0, false)]);
    take(83, &[P], &[(1, 0, false)]);
    for meth in [95, 96] {
        let n = mem(1);
        put(n, &[16]);
        take(meth, &[n, P], &[(16 * 6, 0, false)]);
        counts.push(rd1(n));
    }
    out.extend(counts);
    out
}

fn state_imm() -> State {
    let g = D11S.locked();
    g.as_ref().unwrap().imm_state()
}
fn state_def(def: u32) -> State {
    let g = D11S.locked();
    let d = g.as_ref().unwrap();
    deferred::state_of(d, d.id_of(def)).unwrap().clone()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn immediate_deferred_and_list_agree(calls in prop::collection::vec(call_strategy(), 1..24)) {
        let _g = setup();
        let p = pool();
        let before = counts();
        // the immediate context, from its default state
        call1(m(IF_CONTEXT, 110), &[p.imm]);
        for c in &calls {
            make(&p, p.imm, c);
        }
        let (want, want_gets) = (state_imm(), gets(p.imm));
        call1(m(IF_CONTEXT, 110), &[p.imm]);
        // a deferred context: its own state the same, read alike
        let o = mem(2);
        prop_assert_eq!(call1(m(IF_DEVICE, 27), &[p.dev, 0, o]), 0);
        let def = rd1(o);
        for c in &calls {
            make(&p, def, c);
        }
        prop_assert_eq!(&state_def(def), &want);
        prop_assert_eq!(&gets(def), &want_gets);
        // the list it finishes, run from its start: the same again
        put(o, &[0]);
        prop_assert_eq!(call1(m(IF_CONTEXT, 114), &[def, 0, o]), 0);
        let list = rd1(o);
        let got = {
            let mut g = D11S.locked();
            let d = g.as_mut().unwrap();
            let mut st = default_state();
            let mut after = After::None;
            // SAFETY: a list of this device
            let was = unsafe { deferred::run(d, &mut st, d.id_of(list), &mut after) };
            prop_assert!(was.is_some());
            let got = st.clone();
            unbind_state(d, &st);
            got
        };
        prop_assert_eq!(&got, &want);
        // ... and executed on the immediate context (restore: its state back, the default)
        call1(m(IF_CONTEXT, 58), &[p.imm, list, 1]);
        prop_assert_eq!(&state_imm(), &default_state());
        // what the list names lives as long as it: the pools' references let go of, the list
        // keeps what it binds (and nothing else is held once it goes)
        // (a deferred context let go of goes, its state's references with it: it was kept for
        // good, and all it held)
        prop_assert_eq!(release(def), 0);
        prop_assert_eq!(id_of(def), 0);
        let named: Vec<u32> = {
            let g = D11S.locked();
            let d = g.as_ref().unwrap();
            let Some(Data::CmdList(l)) = d.objs.get(&d.id_of(list)).map(|o| &o.data) else { panic!("no list") };
            deferred::list_objects(l)
        };
        for &x in &named {
            prop_assert!(counts().iter().any(|c| c.0 == x && c.2 > 0), "object {} the list names is not held", x);
        }
        prop_assert_eq!(release(list), 0);
        prop_assert_eq!(&counts(), &before, "references left after the list went");
        for x in p.srvs.iter().chain(&p.rtvs).chain(&p.dsvs).chain(&p.uavs).chain(&p.bufs).chain(&p.cbs).chain(&p.samplers).copied().filter(|&x| x != 0) {
            prop_assert_eq!(release(x), 0);
            prop_assert_eq!(id_of(x), 0);
        }
        release(p.imm);
        release(p.dev);
    }
}
