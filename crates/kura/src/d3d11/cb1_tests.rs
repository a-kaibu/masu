//! Constant buffer ranges (ID3D11DeviceContext1::*SetConstantBuffers1) through the real entry
//! point: random sequences of the plain Set, Set1 with random ranges (NULL arrays too) and
//! Get*ConstantBuffers1 on random stages and slots, against a model of the slots; and the
//! bytes a shader of a random size sees through each range (cb_binding), against the rule
//! constant by constant: the range's constants while inside the buffer, zeros after (D3D11.1
//! functional spec; Windows' answers in winref d3d11cb1).

use kome::locks::Lock as _;
use super::*;
use crate::d3d9::trace_tests::{alloc, attach_page, CALLS, SERIAL};

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
fn m(iface: u32, m: u32) -> u32 {
    iface << 8 | m
}

/// the plain Set*ConstantBuffers' method per stage (VS, PS, GS, HS, DS, CS), and Set1's / Get1's
/// (VS, HS, DS, GS, PS, CS order in the vtable)
const SET: [u32; 6] = [7, 16, 22, 62, 66, 71];
const SET1: [u32; 6] = [119, 123, 122, 120, 121, 124];
const GET1: [u32; 6] = [125, 129, 128, 126, 127, 130];

#[test]
fn ranges_as_the_model_says() {
    use proptest::prelude::*;
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let vt = mem(256);
    put(vt, &(1..=256).map(|i| 0x1000 * i).collect::<Vec<_>>());
    call1(INIT, &[vt, 256]);
    attach_page();
    let desc = mem(16);
    put(desc, &[64, 64, 60, 1, 28, 0, 0, 1, 0, 0x20, 1, 1, 1, 0, 0]);
    let (sc_out, dev_out, level, ctx_out) = (mem(2), mem(2), mem(2), mem(2));
    assert_eq!(call1(CREATE_DEVICE, &[0, 1, 0, 0, 0, 0, 7, desc, sc_out, dev_out, level, ctx_out]), 0);
    let (dev, ctx) = (rd1(dev_out), rd1(ctx_out));
    // (two buffers: 64 constants each, constant k of buffer b holding words b*1000 + k*4 + c)
    let buffer = |b: u32, constants: u32| -> u32 {
        let d = mem(6);
        put(d, &[constants * 16, 0, 4, 0, 0, 0]);
        let data = mem(constants * 4);
        put(data, &(0..constants * 4).map(|w| b * 100_000 + w + 1).collect::<Vec<_>>());
        let init = mem(3);
        put(init, &[data, 0, 0]);
        let o = mem(2);
        assert_eq!(call1(m(IF_DEVICE, 3), &[dev, d, init, o]), 0);
        rd1(o)
    };
    let bufs = [buffer(1, 64), buffer(2, 20)];
    let sizes = [64u32, 20];
    let (arr, firsts, counts, out_b, out_f, out_c) = (mem(2), mem(2), mem(2), mem(2), mem(2), mem(2));
    #[derive(Clone, Debug)]
    enum Op {
        Set { stage: usize, slot: u32, buf: usize },
        Set1 { stage: usize, slot: u32, buf: usize, first: u32, count: u32, null_first: bool, null_count: bool },
    }
    let op = prop_oneof![
        (0usize..6, 0u32..14, 0usize..3).prop_map(|(stage, slot, buf)| Op::Set { stage, slot, buf }),
        (0usize..6, 0u32..14, 0usize..3, prop_oneof![(0u32..8).prop_map(|k| k * 16), 0u32..200], prop_oneof![(0u32..8).prop_map(|k| k * 16), Just(4096), 0u32..200], any::<bool>(), any::<bool>())
            .prop_map(|(stage, slot, buf, first, count, nf, nc)| Op::Set1 { stage, slot, buf, first, count, null_first: nf && count % 3 == 0, null_count: nc && first % 3 == 0 }),
    ];
    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig { cases: 64, ..ProptestConfig::default() });
    runner
        .run(&(proptest::collection::vec(op, 1..24), 1u32..80), |(ops, shader_constants)| {
            // (the model: per stage and slot, the buffer (index, 2 none) and its range)
            let mut model = [[(2usize, CbRange::WHOLE); 14]; 6];
            // (ClearState: every slot empty again)
            call1(m(IF_CONTEXT, 110), &[ctx]);
            for o in &ops {
                match *o {
                    Op::Set { stage, slot, buf } => {
                        put(arr, &[if buf < 2 { bufs[buf] } else { 0 }]);
                        call1(m(IF_CONTEXT, SET[stage]), &[ctx, slot, 1, arr]);
                        model[stage][slot as usize] = (buf, CbRange::WHOLE);
                    }
                    Op::Set1 { stage, slot, buf, first, count, null_first, null_count } => {
                        put(arr, &[if buf < 2 { bufs[buf] } else { 0 }]);
                        put(firsts, &[first]);
                        put(counts, &[count]);
                        call1(m(IF_CONTEXT, SET1[stage]), &[ctx, slot, 1, arr, if null_first { 0 } else { firsts }, if null_count { 0 } else { counts }]);
                        let r = if null_first || null_count { CbRange::WHOLE } else { CbRange { first, count } };
                        model[stage][slot as usize] = (buf, r);
                    }
                }
            }
            for stage in 0..6 {
                for slot in 0..14u32 {
                    put(out_b, &[7]);
                    put(out_f, &[7]);
                    put(out_c, &[7]);
                    call1(m(IF_CONTEXT, GET1[stage]), &[ctx, slot, 1, out_b, out_f, out_c]);
                    let (buf, r) = model[stage][slot as usize];
                    let want_b = if buf < 2 { bufs[buf] } else { 0 };
                    prop_assert_eq!(rd1(out_b), want_b, "stage {} slot {}", stage, slot);
                    prop_assert_eq!((rd1(out_f), rd1(out_c)), (r.first, r.count), "stage {} slot {}", stage, slot);
                    if want_b != 0 {
                        call1(m(IF_BUFFER, 2), &[want_b]);
                    }
                    // (the bytes a shader of `shader_constants` constants sees there)
                    if buf < 2 {
                        let want = shader_constants * 16;
                        let (g, len) = {
                            let mut lock = D11S.locked();
                            let d = lock.as_mut().unwrap();
                            let id = d.id_of(bufs[buf]);
                            cb_binding(d, id, r, want)
                        };
                        prop_assert!(g != 0);
                        let whole = r.first == 0 && r.count.saturating_mul(16) >= sizes[buf] * 16 && sizes[buf] >= shader_constants;
                        if whole {
                            prop_assert_eq!(len, (sizes[buf] * 16).min(65536));
                        } else {
                            prop_assert_eq!(len, want);
                            let lock = D11S.locked();
                            let d = lock.as_ref().unwrap();
                            let id = d.id_of(bufs[buf]);
                            let bytes = &d.cb_copies[&(id, r.first, want)].1;
                            for k in 0..shader_constants {
                                let inside = k < r.count && r.first.saturating_add(k) < sizes[buf];
                                for c in 0..4 {
                                    let w = u32::from_le_bytes(bytes[(k * 16 + c * 4) as usize..][..4].try_into().unwrap());
                                    let model_w = if inside { (buf as u32 + 1) * 100_000 + (r.first + k) * 4 + c + 1 } else { 0 };
                                    prop_assert_eq!(w, model_w, "constant {} of {:?} in buffer {}", k, r, buf);
                                }
                            }
                        }
                    }
                }
            }
            Ok(())
        })
        .unwrap();
}

/// CbRange::bytes stays inside the buffer for any range and size (u32 edges included).
#[test]
fn a_range_stays_inside_its_buffer() {
    use proptest::prelude::*;
    let edge = prop_oneof![4 => 0u32..5000, 1 => (0u32..4).prop_map(|k| u32::MAX - k), 1 => Just(0x1000_0000)];
    proptest!(ProptestConfig { cases: 4096, ..ProptestConfig::default() }, |(first in edge.clone(), count in edge.clone(), size in 0u32..70_000)| {
        let (off, len) = CbRange { first, count }.bytes(size);
        prop_assert!(off <= size && len <= size - off);
        prop_assert_eq!(off, first.saturating_mul(16).min(size));
    });
}
