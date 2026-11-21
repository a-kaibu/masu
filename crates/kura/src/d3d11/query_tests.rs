//! ID3D11Query / ID3D11Predicate as Windows answers them (d3d11/query.rs), measured on the
//! page's GPU (the WebGPU model: a Direct3D 11 draw covers its viewport, render/model.rs counts
//! each sample of it): GetData's errors (never ended, building, a wrong size), S_FALSE until
//! the page answers, an occlusion query's samples, an occlusion predicate's BOOL and the draws
//! SetPredication turns off, PIPELINE_STATISTICS of the draws and dispatches between Begin and
//! End, TIMESTAMP_DISJOINT's frequency and disjoint flag, events.

use super::*;
use crate::d3d9::trace_tests::{alloc, attach_page, CALLS, SERIAL};
use proptest::prelude::*;

const S_FALSE: u32 = 1;
const E_INVALIDARG: u32 = 0x8007_0057;
const INVALID_CALL: u32 = 0x887a_0001;

fn mem(n: u32) -> u32 {
    unsafe { alloc(4 * n) as u32 }
}
fn put(p: u32, v: &[u32]) {
    for (i, &x) in v.iter().enumerate() {
        unsafe { *((p + 4 * i as u32) as *mut u32) = x };
    }
}
fn rd1(p: u32) -> u32 {
    unsafe { *(p as *const u32) }
}
fn call1(id: u32, args: &[u32]) -> u32 {
    unsafe { call(id, args.as_ptr(), &CALLS) }
}
fn cc(ctx: u32, m: u32, a: &[u32]) -> u32 {
    call1(IF_CONTEXT << 8 | m, &[&[ctx][..], a].concat())
}
fn dc(dev: u32, m: u32, a: &[u32]) -> u32 {
    call1(IF_DEVICE << 8 | m, &[&[dev][..], a].concat())
}
thread_local! {
    /// what the test made: released at its end (the device is global to the tests that follow)
    static MADE: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
}
fn made(x: u32) -> u32 {
    MADE.with(|m| m.borrow_mut().push(x));
    x
}
fn release_all() {
    for x in MADE.with(|m| std::mem::take(&mut *m.borrow_mut())).into_iter().rev() {
        call1(IF_QUERY << 8 | 2, &[x]);
    }
}
fn code(c: &[u8]) -> (u32, u32) {
    let p = unsafe { alloc(c.len() as u32) as u32 };
    unsafe { std::ptr::copy_nonoverlapping(c.as_ptr(), p as *mut u8, c.len()) };
    (p, c.len() as u32)
}

const VS: &str = "float4 main(uint id : SV_VertexID) : SV_Position { return float4((id & 1) * 4.0 - 1.0, (id >> 1) * 4.0 - 1.0, 0.5, 1); }";
const PS: &str = "float4 main(float4 p : SV_Position) : SV_Target { return float4(1, 0, 1, 1); }";
const CS: &str = "RWTexture2D<float4> u : register(u0); [numthreads(4, 2, 3)] void main(uint3 i : SV_DispatchThreadID) { u[i.xy] = float4(1, 1, 1, 1); }";

/// A device with a 64x64 target bound and the shaders set; (device, context).
fn device() -> (u32, u32) {
    let vt = mem(256);
    put(vt, &(1..=256).map(|i| 0x1000 * i).collect::<Vec<_>>());
    call1(INIT, &[vt, 256]);
    attach_page();
    let desc = mem(16);
    put(desc, &[64, 64, 60, 1, 28, 0, 0, 1, 0, 0x20, 1, 1, 1, 0, 0]);
    let (sc_out, dev_out, level, ctx_out) = (mem(2), mem(2), mem(2), mem(2));
    assert_eq!(call1(CREATE_DEVICE, &[0, 1, 0, 0, 0, 0, 7, desc, sc_out, dev_out, level, ctx_out]), 0);
    let (dev, ctx) = (made(rd1(dev_out)), made(rd1(ctx_out)));
    made(rd1(sc_out));
    let o = mem(2);
    let d = mem(11);
    put(d, &[64, 64, 1, 1, 28, 1, 0, 0, 0x20 | 0x80, 0, 0]);
    assert_eq!(dc(dev, 5, &[d, 0, o]), 0);
    let rt = made(rd1(o));
    assert_eq!(dc(dev, 9, &[rt, 0, o]), 0);
    let arr = mem(1);
    put(arr, &[made(rd1(o))]);
    cc(ctx, 33, &[1, arr, 0]);
    let vs = hlslc::compile(VS, &[], &mut |_, _| None, "main", "vs_4_0", 0).expect("vs");
    let ps = hlslc::compile(PS, &[], &mut |_, _| None, "main", "ps_4_0", 0).expect("ps");
    let (p, n) = code(&vs);
    assert_eq!(dc(dev, 12, &[p, n, 0, o]), 0);
    cc(ctx, 11, &[made(rd1(o)), 0, 0]);
    let (p, n) = code(&ps);
    assert_eq!(dc(dev, 15, &[p, n, 0, o]), 0);
    cc(ctx, 9, &[made(rd1(o)), 0, 0]);
    cc(ctx, 24, &[4]);
    (dev, ctx)
}

fn viewport(ctx: u32, [x, y, w, h]: [u32; 4]) {
    let vp = mem(6);
    put(vp, &[(x as f32).to_bits(), (y as f32).to_bits(), (w as f32).to_bits(), (h as f32).to_bits(), 0, 1f32.to_bits()]);
    cc(ctx, 44, &[1, vp]);
}

fn query(dev: u32, t: u32) -> u32 {
    let (desc, o) = (mem(2), mem(2));
    put(desc, &[t, u32::from(t == 5 || t == 7)]);
    assert_eq!(dc(dev, if t == 5 || t == 7 { 25 } else { 24 }, &[desc, o]), 0, "CreateQuery({t})");
    made(rd1(o))
}

/// GetData into `size` bytes: the answer and the bytes.
fn get(ctx: u32, q: u32, size: u32, flags: u32) -> (u32, Vec<u8>) {
    let p = unsafe { alloc(size.max(1)) as u32 };
    let hr = cc(ctx, 29, &[q, if size == 0 { 0 } else { p }, size, flags]);
    (hr, (0..size).map(|i| unsafe { *((p + i) as *const u8) }).collect())
}

fn wait(ctx: u32, q: u32, size: u32) -> Vec<u8> {
    for _ in 0..8 {
        let (hr, b) = get(ctx, q, size, 0);
        if hr != S_FALSE {
            assert_eq!(hr, 0);
            return b;
        }
    }
    panic!("query never done");
}

fn u64s(b: &[u8]) -> Vec<u64> {
    b.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect()
}

fn no_model_errors() {
    let e = crate::render::model::with(|m| std::mem::take(&mut m.errors));
    assert!(e.is_empty(), "WebGPU errors: {e:?}");
}

#[test]
fn get_data_errors_as_windows() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (dev, ctx) = device();
    for (t, size) in [(0, 4), (1, 8), (2, 8), (3, 16), (4, 88), (5, 4), (6, 16), (7, 4), (8, 16), (15, 4)] {
        let q = query(dev, t);
        assert_eq!(call1(IF_QUERY << 8 | 7, &[q]), size, "GetDataSize of {t}");
        // never ended
        assert_eq!(get(ctx, q, 0, 0).0, INVALID_CALL);
        cc(ctx, 27, &[q]);
        cc(ctx, 28, &[q]);
        assert_eq!(wait(ctx, q, 0).len(), 0);
        for bad in [1, size - 1, size + 1] {
            assert_eq!(get(ctx, q, bad, 0).0, E_INVALIDARG, "type {t} size {bad}");
        }
        assert_eq!(cc(ctx, 29, &[q, 0, size, 0]), E_INVALIDARG);
        // building again: not ended (events and timestamps have no Begin)
        cc(ctx, 27, &[q]);
        assert_eq!(get(ctx, q, 0, 0).0 == INVALID_CALL, t != 0 && t != 2, "type {t} after a Begin");
    }
    no_model_errors();
    release_all();
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, ..ProptestConfig::default() })]
    /// Draws in random viewports, some between Begin and End: the occlusion query counts the
    /// samples of those, the predicate says whether any, and the pipeline statistics count
    /// their vertices and primitives; a predicate set with its answer turns the draws off.
    #[test]
    fn occlusion_predicate_and_statistics(draws in proptest::collection::vec((0u32..32, 0u32..32, 1u32..33, 1u32..33, any::<bool>(), 1u32..3), 1..6)) {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (dev, ctx) = device();
        let (occ, pred, stats) = (query(dev, 1), query(dev, 5), query(dev, 4));
        for q in [occ, pred, stats] {
            cc(ctx, 27, &[q]);
        }
        let (mut samples, mut verts, mut prims) = (0u64, 0u64, 0u64);
        for &(x, y, w, h, inside, inst) in &draws {
            viewport(ctx, [x, y, w, h]);
            if !inside {
                for q in [occ, pred, stats] {
                    cc(ctx, 28, &[q]);
                }
            }
            cc(ctx, 21, &[3, inst, 0, 0]);
            if !inside {
                for q in [occ, pred, stats] {
                    cc(ctx, 27, &[q]);
                }
                samples = 0;
                verts = 0;
                prims = 0;
            } else {
                samples += u64::from(w) * u64::from(h) * u64::from(inst);
                verts += 3 * u64::from(inst);
                prims += u64::from(inst);
            }
        }
        for q in [occ, pred, stats] {
            cc(ctx, 28, &[q]);
        }
        prop_assert_eq!(u64s(&wait(ctx, occ, 8)), vec![samples]);
        prop_assert_eq!(wait(ctx, pred, 4), u32::from(samples != 0).to_le_bytes().to_vec());
        let s = u64s(&wait(ctx, stats, 88));
        prop_assert_eq!(&s[..3], &[verts, prims, verts][..]);
        prop_assert_eq!((s[5], s[6], s[7], s[10]), (prims, prims, samples, 0));
        // predicated: the draws go when the answer is not the value
        for value in [0u32, 1] {
            cc(ctx, 30, &[pred, value]);
            let before = crate::render::model::with(|m| m.calls.get("draw").copied().unwrap_or(0));
            viewport(ctx, [0, 0, 8, 8]);
            cc(ctx, 13, &[3, 0]);
            crate::d3d9::gpu::flush();
            crate::d3d9::gpu::pump();
            let drawn = crate::render::model::with(|m| m.calls.get("draw").copied().unwrap_or(0)) > before;
            prop_assert_eq!(drawn, (samples != 0) != (value != 0), "predicate {} value {}", samples != 0, value);
        }
        cc(ctx, 30, &[0, 0]);
        no_model_errors();
        release_all();
    }
}

#[test]
fn events_timestamps_and_dispatches() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for stamps in [false, true] {
        let (dev, ctx) = device();
        if stamps {
            crate::render::model::with(|m| m.features.push("timestamp-query"));
        }
        let (ev, ts, dis, stats) = (query(dev, 0), query(dev, 2), query(dev, 3), query(dev, 4));
        cc(ctx, 28, &[ev]);
        // DONOTFLUSH: not sent yet
        assert_eq!(get(ctx, ev, 4, 1).0, S_FALSE);
        assert_eq!(wait(ctx, ev, 4), 1u32.to_le_bytes().to_vec());
        cc(ctx, 27, &[dis]);
        cc(ctx, 27, &[stats]);
        let mut last = 0;
        for k in 0..3 {
            viewport(ctx, [0, 0, 1 + k, 1]);
            cc(ctx, 13, &[3, 0]);
            cc(ctx, 28, &[ts]);
            let v = u64s(&wait(ctx, ts, 8))[0];
            if stamps {
                assert!(v > last, "{v} after {last}");
            }
            last = v;
        }
        // a dispatch: its groups' threads
        let cs = hlslc::compile(CS, &[], &mut |_, _| None, "main", "cs_5_0", 0).expect("cs");
        let (p, n) = code(&cs);
        let o = mem(2);
        assert_eq!(dc(dev, 18, &[p, n, 0, o]), 0);
        cc(ctx, 69, &[made(rd1(o)), 0, 0]);
        cc(ctx, 41, &[2, 3, 1]);
        cc(ctx, 28, &[stats]);
        let s = u64s(&wait(ctx, stats, 88));
        assert_eq!((s[0], s[10]), (9, 6 * 24));
        cc(ctx, 28, &[dis]);
        let b = wait(ctx, dis, 16);
        assert_eq!(u64s(&b[..8])[0], crate::d3d9::query::FREQ);
        assert_eq!(b[8..12], [0; 4]);
        cc(ctx, 27, &[dis]);
        crate::render::set_lost_gen(crate::render::lost_gen() + 1);
        cc(ctx, 28, &[dis]);
        assert_eq!(wait(ctx, dis, 16)[8..12], [1, 0, 0, 0]);
        no_model_errors();
        release_all();
    }
}
