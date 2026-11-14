//! Every D2D object's vtable, slot by slot (d2d/vtables.txt): which handler each method gets,
//! pinned before the method lists change shape (com::check_vtables).
use super::*;
use crate::test_guest::{call, enter};

fn made(f: Handler, args: &[u32]) -> u32 {
    let o = mem::heap_alloc(8, true);
    let mut a = args.to_vec();
    a.push(o);
    assert_eq!(call(f, &a), 0);
    let p = r32(o);
    mem::heap_free(o);
    p
}
fn floats(v: &[f32]) -> u32 {
    let p = mem::heap_alloc(64, true);
    for (i, x) in (0u32..).zip(v) {
        w32(p + 4 * i, x.to_bits());
    }
    p
}

#[test]
fn every_objects_vtable() {
    let _g = enter();
    let f = factory();
    let d = made(CreateDevice, &[f, 0]);
    let dc = made(CreateDeviceContext, &[d, 0]);
    let color = floats(&[1.0, 0.5, 0.0, 1.0]);
    made(CreateSolidColorBrush, &[dc, color, 0]);
    let stops = floats(&[0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0]);
    let sc = made(CreateGradientStopCollection, &[dc, stops, 2, 0, 0]);
    let lp = floats(&[0.0, 0.0, 10.0, 0.0]);
    made(CreateLinearGradientBrush, &[dc, lp, 0, sc]);
    new_bitmap(0x5000);
    let pg = made(CreatePathGeometry, &[f]);
    made(GeomOpen, &[pg]);
    let m = floats(&[1.0, 0.0, 0.0, 1.0, 5.0, 5.0]);
    made(CreateTransformedGeometry, &[f, pg, m]);
    made(CreateStrokeStyle, &[f, 0, 0, 0]);
    // (these two need a D3D11 device of the guest's: their lists alone)
    com::vtable(com::iface!(ID2D1HwndRenderTarget), &hwnd_methods());
    com::vtable(com::iface!(ID2D1BitmapRenderTarget), &bitmap_rt_methods());
    com::check_vtables(
        "d2d/vtables.txt",
        &[
            FACTORY,
            DEVICE,
            CONTEXT,
            com::iface!(ID2D1HwndRenderTarget),
            com::iface!(ID2D1BitmapRenderTarget),
            com::iface!(ID2D1SolidColorBrush),
            com::iface!(ID2D1GradientStopCollection),
            com::iface!(ID2D1LinearGradientBrush),
            com::iface!(ID2D1Bitmap1),
            com::iface!(ID2D1PathGeometry1),
            com::iface!(ID2D1GeometrySink),
            com::iface!(ID2D1TransformedGeometry),
            com::iface!(ID2D1StrokeStyle1),
        ],
    );
}
