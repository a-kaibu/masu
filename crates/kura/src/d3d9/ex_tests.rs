//! IDirect3DDevice9Ex's own state calls on a device (sake routes Direct3DCreate9Ex's devices'
//! here): GPU thread priority and maximum frame latency (a model over any sequence of Sets
//! and Gets: Wine's tests of Windows, d3d9ex.c test_frame_latency, and the documented
//! -7..=7), WaitForVBlank (returns at the start of a vertical blank by GetRasterStatus's
//! beam), CheckResourceResidency and CheckDeviceState (windowed and full screen).

use super::draw_tests::{call, out, rd};
use super::trace_tests::{alloc, attach_page, CALLS, SERIAL};
use super::*;
use proptest::prelude::*;

const INVALIDCALL: u32 = 0x8876_086c;
const OCCLUDED: u32 = 0x0876_0868;

/// A 64x64 device of window 1, windowed or full screen.
fn device(windowed: bool) -> u32 {
    attach_page();
    let o = out();
    unsafe { vnhost_d3d9_create(0x20, o as *mut u32, &CALLS) };
    let d3d = rd(o);
    let pp = unsafe { alloc(56) as u32 };
    for (i, x) in [64, 64, 22, 1, 0, 0, 1, 1, u32::from(windowed), 0, 0, 0, 0, 0].into_iter().enumerate() {
        unsafe { *((pp + 4 * i as u32) as *mut u32) = x };
    }
    let o = out();
    assert_eq!(call(IF_D3D9, 16, &[d3d, 0, 1, 1, 0x20, pp, o]), 0);
    rd(o)
}

fn init() {
    unsafe {
        let vt: Vec<u32> = (1..=64).map(|i| 0x1000 * i).collect();
        vnhost_d3d9_init(vt.as_ptr(), vt.len() as u32);
    }
}

#[derive(Clone, Debug)]
enum Op {
    SetPriority(i32),
    GetPriority,
    SetLatency(u32),
    GetLatency,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![(-10i32..=10).prop_map(Op::SetPriority), Just(Op::GetPriority), (0u32..=40).prop_map(Op::SetLatency), Just(Op::GetLatency)]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]
    #[test]
    fn priority_and_latency_as_the_model(ops in proptest::collection::vec(op(), 1..24)) {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        init();
        let dev = device(true);
        let (mut prio, mut lat) = (0i32, 3u32);
        for o in &ops {
            match *o {
                Op::SetPriority(p) => {
                    let ok = (-7..=7).contains(&p);
                    prop_assert_eq!(call(IF_DEVICE, 123, &[dev, p.cast_unsigned()]), if ok { 0 } else { INVALIDCALL }, "SetGPUThreadPriority {}", p);
                    if ok {
                        prio = p;
                    }
                }
                Op::GetPriority => {
                    let q = out();
                    prop_assert_eq!(call(IF_DEVICE, 122, &[dev, q]), 0);
                    prop_assert_eq!(rd(q).cast_signed(), prio);
                }
                Op::SetLatency(n) => {
                    let ok = n <= 30;
                    prop_assert_eq!(call(IF_DEVICE, 126, &[dev, n]), if ok { 0 } else { INVALIDCALL }, "SetMaximumFrameLatency {}", n);
                    if ok {
                        lat = if n == 0 { 3 } else { n };
                    }
                }
                Op::GetLatency => {
                    let q = out();
                    prop_assert_eq!(call(IF_DEVICE, 127, &[dev, q]), 0);
                    prop_assert_eq!(rd(q), lat);
                }
            }
        }
        prop_assert_eq!(call(IF_DEVICE, 122, &[dev, 0]), INVALIDCALL, "no out pointer");
        prop_assert_eq!(call(IF_DEVICE, 127, &[dev, 0]), INVALIDCALL, "no out pointer");
        call(IF_DEVICE, 2, &[dev]);
    }
}

#[test]
fn wait_for_vblank_ends_in_a_vertical_blank() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init();
    let dev = device(true);
    assert_eq!(call(IF_DEVICE, 124, &[dev, 1]), INVALIDCALL, "swap chain 1: none (one implicit)");
    let rs = out();
    for _ in 0..5 {
        let t = std::time::Instant::now();
        assert_eq!(call(IF_DEVICE, 124, &[dev, 0]), 0);
        // (GetRasterStatus right after: in vertical blank, within a frame of waiting)
        assert_eq!(call(IF_DEVICE, 19, &[dev, 0, rs]), 0);
        assert!(t.elapsed().as_secs_f64() <= 1.0 / 60.0 + 0.01, "{:?}", t.elapsed());
        assert_eq!(rd(rs), 1, "InVBlank after WaitForVBlank");
    }
    call(IF_DEVICE, 2, &[dev]);
}

#[test]
fn residency_and_device_state() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init();
    for windowed in [true, false] {
        let dev = device(windowed);
        let o = out();
        assert_eq!(call(IF_DEVICE, 23, &[dev, 4, 4, 1, 0, 21, 0, o, 0]), 0, "CreateTexture");
        let tex = rd(o);
        let list = unsafe { alloc(8) as u32 };
        unsafe { *(list as *mut u32) = tex };
        unsafe { *((list + 4) as *mut u32) = dev };
        assert_eq!(call(IF_DEVICE, 125, &[dev, list, 1]), 0, "a texture: resident");
        assert_eq!(call(IF_DEVICE, 125, &[dev, list, 2]), INVALIDCALL, "the device is no resource");
        assert_eq!(call(IF_DEVICE, 125, &[dev, 0, 0]), 0, "none");
        assert_eq!(call(IF_DEVICE, 125, &[dev, 0, 1]), INVALIDCALL, "no list");
        assert_eq!(call(IF_DEVICE, 125, &[dev, list, 65536]), INVALIDCALL, "past 65535");
        // CheckDeviceState: windowed, never occluded; full screen, every window but its own
        for (hwnd, full) in [(1, 0), (0, OCCLUDED), (7, OCCLUDED)] {
            assert_eq!(call(IF_DEVICE, 128, &[dev, hwnd]), if windowed { 0 } else { full }, "windowed {windowed} hwnd {hwnd}");
        }
        call(IF_TEXTURE, 2, &[tex]);
        call(IF_DEVICE, 2, &[dev]);
    }
}
