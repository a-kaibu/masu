//! IDirect3DQuery9 as Windows answers it (Wine's d3d9 device.c test_occlusion_query,
//! test_timestamp_query; winref d3d9query), measured on the page's GPU (the WebGPU model:
//! render/model.rs counts every sample a draw covers):
//! a query never issued fills what is asked for with 0xdd; an occlusion query counts the
//! samples of the draws between its begin and end (not a clear's, not the draws outside it),
//! with the bytes asked for of its u64; events end TRUE once the page ran what came before;
//! timestamps go up, at 1 GHz; a disjoint query says whether the device was lost in between.

use super::draw_tests::{call, out, put_bytes, rd};
use super::query::{self, le};
use super::trace_tests::{alloc, attach_page, CALLS, SERIAL};
use super::*;
use proptest::prelude::*;

const S_FALSE: u32 = 1;
const NOTAVAILABLE: u32 = 0x8876_086a;

fn init() {
    unsafe {
        let vt: Vec<u32> = (1..=64).map(|i| 0x1000 * i).collect();
        vnhost_d3d9_init(vt.as_ptr(), vt.len() as u32);
    }
}

/// A windowed 64x64 device (no depth buffer) with the page attached.
fn device() -> u32 {
    attach_page();
    let o = out();
    unsafe { vnhost_d3d9_create(0x20, o as *mut u32, &CALLS) };
    let d3d = rd(o);
    let pp = unsafe { alloc(56) as u32 };
    for (i, x) in [64, 64, 22, 1, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0].into_iter().enumerate() {
        unsafe { *((pp + 4 * i as u32) as *mut u32) = x };
    }
    let o = out();
    assert_eq!(call(IF_D3D9, 16, &[d3d, 0, 1, 1, 0x20, pp, o]), 0);
    let dev = rd(o);
    // XYZRHW | DIFFUSE, no lighting or culling
    assert_eq!(call(IF_DEVICE, 89, &[dev, 0x44]), 0);
    assert_eq!(call(IF_DEVICE, 57, &[dev, 22, 1]), 0);
    dev
}

fn create(dev: u32, t: u32) -> u32 {
    let o = out();
    assert_eq!(call(IF_DEVICE, 118, &[dev, t, o]), 0, "CreateQuery({t})");
    rd(o)
}

/// A quad over pixels [x0, x1) x [y0, y1).
fn quad(dev: u32, [x0, y0, x1, y1]: [u32; 4]) {
    let v: Vec<u8> = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
        .into_iter()
        .flat_map(|(x, y)| [x as f32, y as f32, 0.5, 1.0].into_iter().flat_map(f32::to_le_bytes).chain(0xff00_ff00u32.to_le_bytes()))
        .collect();
    let up = unsafe { alloc(v.len() as u32) as u32 };
    put_bytes(up, &v);
    assert_eq!(call(IF_DEVICE, 83, &[dev, 5, 2, up, 20]), 0);
}

/// The samples the model counts for `quad(r)`: the box the device says the draw covers (a
/// pixel more on each side than its vertices, device.rs `bbox`), in the 64x64 target. A GPU
/// counts the quad's own pixels; the model tells the counting, not the rasterizing.
fn samples([x0, y0, x1, y1]: [u32; 4]) -> u64 {
    u64::from((x1 + 1).min(64) - x0.saturating_sub(1)) * u64::from((y1 + 1).min(64) - y0.saturating_sub(1))
}

/// GetData into an 16-byte buffer of 0xff: the answer and the buffer after.
fn get(q: u32, size: u32, flags: u32) -> (u32, Vec<u8>) {
    let p = unsafe { alloc(16) as u32 };
    put_bytes(p, &[0xff; 16]);
    let hr = call(IF_QUERY, 7, &[q, if size == u32::MAX { 0 } else { p }, if size == u32::MAX { 0 } else { size }, flags]);
    (hr, (0..16).map(|i| unsafe { *((p + i) as *const u8) }).collect())
}

/// GetData(FLUSH) until it is no S_FALSE (the page runs what was sent meanwhile).
fn wait(q: u32, size: u32) -> Vec<u8> {
    for _ in 0..8 {
        let (hr, b) = get(q, size, 1);
        if hr != S_FALSE {
            assert_eq!(hr, 0);
            return b;
        }
    }
    panic!("query {q} never done");
}

fn no_model_errors() {
    let e = crate::render::model::with(|m| std::mem::take(&mut m.errors));
    assert!(e.is_empty(), "WebGPU errors: {e:?}");
}

#[test]
fn query_support_and_sizes_as_windows() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init();
    let dev = device();
    // (Wine test_query_support: 8..=12 made; the ones Windows has no more fail both ways)
    for t in 0..40 {
        let o = out();
        put_bytes(o, &0xdead_beefu32.to_le_bytes());
        let want = if (8..=12).contains(&t) { 0 } else { NOTAVAILABLE };
        assert_eq!(call(IF_DEVICE, 118, &[dev, t, 0]), want, "support of {t}");
        assert_eq!(call(IF_DEVICE, 118, &[dev, t, o]), want, "CreateQuery({t})");
        if want != 0 {
            assert_eq!(rd(o), 0xdead_beef);
        }
    }
    for (t, size) in [(8, 4), (9, 4), (10, 8), (11, 4), (12, 8)] {
        let q = create(dev, t);
        assert_eq!(call(IF_QUERY, 4, &[q]), t);
        assert_eq!(call(IF_QUERY, 5, &[q]), size);
        // never issued: S_OK, 0xdd over what is asked for up to the data size
        assert_eq!(get(q, u32::MAX, 1).0, 0);
        for n in 0..=12u32 {
            let (hr, b) = get(q, n, 1);
            assert_eq!(hr, 0);
            let k = n.min(size) as usize;
            assert!(b[..k].iter().all(|&x| x == 0xdd) && b[k..].iter().all(|&x| x == 0xff), "type {t} size {n}: {b:02x?}");
        }
        assert_eq!(call(IF_QUERY, 2, &[q]), 0);
    }
    no_model_errors();
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]
    /// Any quads, some inside the query and some outside, and a partial clear inside: the count
    /// is the samples of the quads inside, given as the bytes asked for of the u64.
    #[test]
    fn occlusion_counts_the_samples_drawn_inside(rects in proptest::collection::vec((0u32..64, 0u32..64, 1u32..64, 1u32..64, any::<bool>()), 1..6), size in prop_oneof![Just(2u32), Just(4), Just(8), Just(12)], clear in any::<bool>()) {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        init();
        let dev = device();
        let q = create(dev, 9);
        assert_eq!(call(IF_DEVICE, 41, &[dev]), 0);
        let mut want = 0u64;
        let mut began = false;
        for (k, &(x, y, w, h, inside)) in rects.iter().enumerate() {
            let r = [x, y, (x + w).min(64), (y + h).min(64)];
            if inside && !began {
                prop_assert_eq!(call(IF_QUERY, 6, &[q, 2]), 0);
                began = true;
                if clear && k == 0 {
                    let rc = unsafe { alloc(16) as u32 };
                    put_bytes(rc, &[4u32, 4, 20, 20].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>());
                    prop_assert_eq!(call(IF_DEVICE, 43, &[dev, 1, rc, 1, 0xff12_3456, 0, 0]), 0);
                }
            }
            if !inside && began {
                prop_assert_eq!(call(IF_QUERY, 6, &[q, 1]), 0);
                began = false;
                // (ended: what comes after is not counted; a begin later starts over)
                want = 0;
                let b = wait(q, size);
                let _ = b;
            }
            quad(dev, r);
            if began {
                want += samples(r);
            }
        }
        // the last stretch inside (or a begin-end around nothing)
        if !began {
            want = 0;
            prop_assert_eq!(call(IF_QUERY, 6, &[q, 2]), 0);
        }
        prop_assert_eq!(call(IF_QUERY, 6, &[q, 1]), 0);
        assert_eq!(call(IF_DEVICE, 42, &[dev]), 0);
        let b = wait(q, size);
        let k = size.min(8) as usize;
        prop_assert_eq!(le(&b[..k]), want & if k == 8 { u64::MAX } else { (1 << (8 * k)) - 1 }, "{:?}", rects);
        prop_assert!(b[k..].iter().all(|&x| x == 0xff));
        no_model_errors();
    }
}

#[test]
fn occlusion_states_as_windows() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init();
    let dev = device();
    let q = create(dev, 9);
    // END without a BEGIN: 0 samples; BEGIN twice: from the second
    assert_eq!(call(IF_QUERY, 6, &[q, 1]), 0);
    assert_eq!(le(&wait(q, 4)[..4]), 0);
    assert_eq!(call(IF_QUERY, 6, &[q, 2]), 0);
    quad(dev, [0, 0, 8, 8]);
    assert_eq!(call(IF_QUERY, 6, &[q, 2]), 0);
    // building: S_FALSE
    assert_eq!(get(q, 4, 1).0, S_FALSE);
    quad(dev, [0, 0, 4, 2]);
    assert_eq!(call(IF_QUERY, 6, &[q, 1]), 0);
    assert_eq!(le(&wait(q, 8)[..8]), samples([0, 0, 4, 2]));
    // size 0: S_OK, nothing written
    let (hr, b) = get(q, 0, 1);
    assert_eq!(hr, 0);
    assert!(b.iter().all(|&x| x == 0xff));
    // two queries open at once each count their own draws
    let q2 = create(dev, 9);
    assert_eq!(call(IF_QUERY, 6, &[q, 2]), 0);
    quad(dev, [0, 0, 10, 10]);
    assert_eq!(call(IF_QUERY, 6, &[q2, 2]), 0);
    quad(dev, [0, 0, 3, 3]);
    assert_eq!(call(IF_QUERY, 6, &[q, 1]), 0);
    quad(dev, [0, 0, 5, 1]);
    assert_eq!(call(IF_QUERY, 6, &[q2, 1]), 0);
    assert_eq!(le(&wait(q, 8)[..8]), samples([0, 0, 10, 10]) + samples([0, 0, 3, 3]));
    assert_eq!(le(&wait(q2, 8)[..8]), samples([0, 0, 3, 3]) + samples([0, 0, 5, 1]));
    no_model_errors();
}

#[test]
fn events_and_timestamps_as_windows() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init();
    for stamps in [false, true] {
        let dev = device();
        if stamps {
            crate::render::model::with(|m| m.features.push("timestamp-query"));
        }
        let e = create(dev, 8);
        // BEGIN of an event: S_OK, nothing (still never issued)
        assert_eq!(call(IF_QUERY, 6, &[e, 2]), 0);
        assert_eq!(get(e, 4, 0).1[..4], [0xdd; 4]);
        assert_eq!(call(IF_QUERY, 6, &[e, 1]), 0);
        // not yet: the page has not run it (no flush)
        assert_eq!(get(e, 4, 0).0, S_FALSE);
        assert_eq!(le(&wait(e, 4)[..4]), 1);
        let freq = create(dev, 12);
        assert_eq!(call(IF_QUERY, 6, &[freq, 1]), 0);
        assert_eq!(le(&wait(freq, 8)[..8]), query::FREQ);
        let (t, dis) = (create(dev, 10), create(dev, 11));
        assert_eq!(call(IF_QUERY, 6, &[dis, 2]), 0);
        let mut last = 0;
        for k in 0..3u32 {
            quad(dev, [0, 0, 1 + k, 1]);
            assert_eq!(call(IF_QUERY, 6, &[t, 1]), 0);
            let v = le(&wait(t, 8)[..8]);
            if stamps {
                assert!(v > last, "timestamp {k}: {v} after {last}");
            }
            last = v;
        }
        // a 4-byte read of a timestamp: its low half only
        let b = get(t, 4, 1).1;
        assert_eq!(le(&b[..4]), last & 0xffff_ffff);
        assert_eq!(b[4..8], [0xff; 4]);
        assert_eq!(call(IF_QUERY, 6, &[dis, 1]), 0);
        assert_eq!(le(&wait(dis, 4)[..4]), 0);
        // a device lost and made again in between: disjoint
        assert_eq!(call(IF_QUERY, 6, &[dis, 2]), 0);
        crate::render::lose_device();
        assert_eq!(call(IF_QUERY, 6, &[dis, 1]), 0);
        assert_eq!(le(&wait(dis, 4)[..4]), 1);
        let b = get(dis, 2, 1).1;
        assert_eq!(b[..2], [1, 0]);
        assert_eq!(b[2..4], [0xff; 2]);
        no_model_errors();
        // (stamps: one query set of timestamps was made)
        let made = crate::render::model::with(|m| m.calls.get("timestamp").copied().unwrap_or(0));
        assert_eq!(made > 0, stamps);
    }
}

#[test]
fn queries_without_a_page_answer_at_once() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init();
    let dev = device();
    gpu::set_pump(None);
    let q = create(dev, 9);
    assert_eq!(call(IF_QUERY, 6, &[q, 2]), 0);
    quad(dev, [0, 0, 8, 8]);
    assert_eq!(call(IF_QUERY, 6, &[q, 1]), 0);
    // (nobody draws: no samples)
    let (hr, b) = get(q, 4, 0);
    assert_eq!((hr, le(&b[..4])), (0, 0));
    let e = create(dev, 8);
    assert_eq!(call(IF_QUERY, 6, &[e, 1]), 0);
    assert_eq!(le(&get(e, 4, 0).1[..4]), 1);
}

#[test]
fn released_queries_forget_their_tickets() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init();
    let dev = device();
    let before = gpu::tickets_live();
    let qs: Vec<u32> = (0..5).map(|k| create(dev, 8 + k)).collect();
    for &q in &qs {
        assert_eq!(call(IF_QUERY, 6, &[q, 2]), 0);
        assert_eq!(call(IF_QUERY, 6, &[q, 1]), 0);
        // issued again: the old ticket goes
        assert_eq!(call(IF_QUERY, 6, &[q, 1]), 0);
    }
    assert_eq!(gpu::tickets_live(), before + 5);
    for &q in &qs {
        assert_eq!(call(IF_QUERY, 2, &[q]), 0);
    }
    assert_eq!(gpu::tickets_live(), before);
}
