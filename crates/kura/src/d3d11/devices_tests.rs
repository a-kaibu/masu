//! Each D3D11CreateDevice makes a device of its own (review of 6852be1, stage 4): its own
//! immediate context and state, children whose GetDevice is the device that made them, and a
//! device that goes once nobody holds it, its context with it. Random calls on two or three
//! devices against a model of one state per device, through the real entry point.

use kome::locks::Lock as _;
use super::*;
use super::deferred_tests::{call1, m, mem, put, rd1, setup};
use proptest::prelude::*;


/// (device, immediate context)
fn device() -> (u32, u32) {
    let (dev_out, level, ctx_out) = (mem(2), mem(2), mem(2));
    assert_eq!(call1(CREATE_DEVICE, &[0, 1, 0, 0, 0, 0, 7, 0, 0, dev_out, level, ctx_out]), 0);
    (rd1(dev_out), rd1(ctx_out))
}

/// A constant buffer of 16 bytes made by `dev`.
fn buffer(dev: u32) -> u32 {
    let (desc, out) = (mem(6), mem(2));
    put(desc, &[16, 0, 4, 0, 0, 0]);
    assert_eq!(call1(m(IF_DEVICE, 3), &[dev, desc, 0, out]), 0);
    rd1(out)
}

/// The object a call's out pointer got, its reference let go of.
fn got(f: impl FnOnce(u32)) -> u32 {
    let out = mem(2);
    put(out, &[0xdead, 0]);
    f(out);
    let v = rd1(out);
    if v != 0 {
        call1(m(IF_DEVICE, 2), &[v]);
    }
    v
}

fn ps_cbs(ctx: u32) -> [u32; 4] {
    let p = mem(4);
    call1(m(IF_CONTEXT, 77), &[ctx, 0, 4, p]);
    let v = [0, 1, 2, 3].map(|i| rd1(p + 4 * i));
    for b in v {
        if b != 0 {
            call1(m(IF_BUFFER, 2), &[b]);
        }
    }
    v
}

#[derive(Clone, Debug)]
enum Op {
    /// a new buffer of device .0
    Make(u8),
    /// device .0's PSSetConstantBuffers(slot .1, buffer .2 of its own, or none)
    Bind(u8, u8, Option<u8>),
    /// device .0's ClearState
    Clear(u8),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0u8..3).prop_map(Op::Make),
        (0u8..3, 0u8..4, proptest::option::of(any::<u8>())).prop_map(|(d, s, b)| Op::Bind(d, s, b)),
        (0u8..3).prop_map(Op::Clear),
    ]
}

/// The objects there are, but for the DXGI ones that live as long as the process.
fn live() -> usize {
    let g = D11S.locked();
    g.as_ref().map_or(0, |d| d.objs.values().filter(|o| !matches!(o.data, Data::Factory | Data::Adapter(_) | Data::Output)).count())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn devices_are_their_own(n in 2usize..4, ops in proptest::collection::vec(op(), 1..40)) {
        let _g = setup();
        let devs: Vec<(u32, u32)> = (0..n).map(|_| device()).collect();
        // (every device and every context its own object)
        let mut all: Vec<u32> = devs.iter().flat_map(|&(d, c)| [d, c]).collect();
        all.sort_unstable();
        all.dedup();
        prop_assert_eq!(all.len(), 2 * n);
        let mut bufs: Vec<Vec<u32>> = vec![Vec::new(); n];
        let mut model: Vec<[u32; 4]> = vec![[0; 4]; n];
        for o in &ops {
            match *o {
                Op::Make(k) => {
                    let k = usize::from(k) % n;
                    bufs[k].push(buffer(devs[k].0));
                }
                Op::Bind(k, s, b) => {
                    let k = usize::from(k) % n;
                    let b = b.and_then(|i| bufs[k].get(usize::from(i) % bufs[k].len().max(1)).copied()).unwrap_or(0);
                    let p = mem(1);
                    put(p, &[b]);
                    call1(m(IF_CONTEXT, 16), &[devs[k].1, u32::from(s), 1, p]);
                    model[k][usize::from(s)] = b;
                }
                Op::Clear(k) => {
                    let k = usize::from(k) % n;
                    call1(m(IF_CONTEXT, 110), &[devs[k].1]);
                    model[k] = [0; 4];
                }
            }
            for (k, &(_, ctx)) in devs.iter().enumerate() {
                prop_assert_eq!(ps_cbs(ctx), model[k], "device {}'s context after {:?}", k, o);
            }
        }
        for (k, &(dev, ctx)) in devs.iter().enumerate() {
            // GetDevice of the device's children and of its context: that device
            for &b in &bufs[k] {
                prop_assert_eq!(got(|out| { call1(m(IF_BUFFER, 3), &[b, out]); }), dev);
            }
            prop_assert_eq!(got(|out| { call1(m(IF_CONTEXT, 3), &[ctx, out]); }), dev);
            prop_assert_eq!(got(|out| { call1(m(IF_DEVICE, 40), &[dev, out]); }), ctx);
            // (the feature level it was made at)
            prop_assert_eq!(call1(m(IF_DEVICE, 37), &[dev]), 0xb000);
        }
        // let go of one device's objects: the others' state is as it was
        let (dev0, ctx0) = devs[0];
        for &b in &bufs[0] {
            call1(m(IF_BUFFER, 2), &[b]);
        }
        call1(m(IF_CONTEXT, 2), &[ctx0]);
        prop_assert_eq!(call1(m(IF_DEVICE, 2), &[dev0]), 0);
        for (k, &(_, ctx)) in devs.iter().enumerate().skip(1) {
            prop_assert_eq!(ps_cbs(ctx), model[k]);
        }
        for (k, &(dev, ctx)) in devs.iter().enumerate().skip(1) {
            for &b in &bufs[k] {
                call1(m(IF_BUFFER, 2), &[b]);
            }
            call1(m(IF_CONTEXT, 2), &[ctx]);
            prop_assert_eq!(call1(m(IF_DEVICE, 2), &[dev]), 0);
        }
        // nothing left: every device took its context, its state and what that bound along
        prop_assert_eq!(live(), 0);
    }
}

/// A device's state, set on its context, is none of another device's: the case of a game that
/// makes a second device to try what the system can do while the first draws.
#[test]
fn a_second_device_starts_from_nothing() {
    let _g = setup();
    let (dev1, ctx1) = device();
    let b = buffer(dev1);
    let p = mem(1);
    put(p, &[b]);
    call1(m(IF_CONTEXT, 16), &[ctx1, 0, 1, p]);
    let (dev2, ctx2) = device();
    assert_ne!(dev1, dev2);
    assert_ne!(ctx1, ctx2);
    assert_eq!(ps_cbs(ctx2), [0; 4]);
    assert_eq!(ps_cbs(ctx1), [b, 0, 0, 0]);
    // (a check that a device can be made: nothing asked for back, nothing left)
    let before = live();
    assert_eq!(call1(CREATE_DEVICE, &[0, 1, 0, 0, 0, 0, 7, 0, 0, 0, mem(1), 0]), 1);
    assert_eq!(live(), before);
}
