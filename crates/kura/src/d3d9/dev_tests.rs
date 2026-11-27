//! Direct3D 9 device state as Windows keeps it (Wine's tests test_cursor, test_vidmem_accounting;
//! winref d3d9dev): the gamma ramp (kept; a full-screen device's on the display, the page's
//! GAMMA, gone with the device or a Reset to windowed), the cursor (hidden at first, shown only
//! with an image; SetCursorProperties' sizes and format), SetDialogBoxMode, Set/GetClipStatus,
//! Set/GetSoftwareVertexProcessing of each kind of device, GetAvailableTextureMem going down
//! with default pool resources.

use super::draw_tests::{call, out, rd, words};
use super::trace_tests::{alloc, attach_page, CALLS, PAGE, SERIAL};
use super::*;
use proptest::prelude::*;

use super::format as f;

const INVALIDCALL: u32 = 0x8876_086c;

fn init() {
    unsafe {
        let vt: Vec<u32> = (1..=64).map(|i| 0x1000 * i).collect();
        vnhost_d3d9_init(vt.as_ptr(), vt.len() as u32);
    }
}

fn pump() {
    gpu::flush();
    gpu::pump();
}

/// D3DPRESENT_PARAMETERS words of a 16x16 device, windowed or not, with `flags` and swap effect.
fn params(windowed: bool, flags: u32, swap: u32) -> u32 {
    let pp = unsafe { alloc(56) as u32 };
    for (i, x) in [16, 16, 22, 1, 0, 0, swap, 1, u32::from(windowed), 0, 0, flags, 0, 0].into_iter().enumerate() {
        unsafe { *((pp + 4 * i as u32) as *mut u32) = x };
    }
    pp
}

/// IDirect3D9 and a device of it made with `behavior` (D3DCREATE_*) and present parameters `pp`.
fn device(behavior: u32, pp: u32) -> (u32, u32) {
    attach_page();
    let o = out();
    unsafe { vnhost_d3d9_create(0x20, o as *mut u32, &CALLS) };
    let d3d = rd(o);
    let o = out();
    assert_eq!(call(IF_D3D9, 16, &[d3d, 0, 1, 1, behavior, pp, o]), 0);
    (d3d, rd(o))
}

fn page_gamma() -> Option<Vec<u16>> {
    pump();
    PAGE.with(|p| p.borrow().as_ref().and_then(|pg| pg.borrow().gamma.as_ref().map(|g| g.to_vec())))
}

fn get_ramp(dev: u32) -> Vec<u16> {
    let r = unsafe { alloc(1536) as u32 };
    assert_eq!(call(IF_DEVICE, 22, &[dev, 0, r]), 0);
    (0..768).map(|i| unsafe { *((r + 2 * i) as *const u16) }).collect()
}

fn ramp_words(r: &[u16]) -> u32 {
    let w: Vec<u32> = r.chunks(2).map(|c| u32::from(c[0]) | u32::from(c.get(1).copied().unwrap_or(0)) << 16).collect();
    words(&w)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, ..ProptestConfig::default() })]
    /// Any ramp: GetGammaRamp answers it (the identity before one); a full-screen device's goes
    /// to the display and leaves it with a Reset to windowed or the device's release; a
    /// windowed device's stays off it.
    #[test]
    fn gamma_ramp_kept_and_shown_full_screen(seed in any::<u64>(), windowed in any::<bool>()) {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        init();
        let (_, dev) = device(0x20, params(windowed, 0, 1));
        // (the identity: entry i is i on a windowed device, i * 257 on a full-screen one,
        // winref d3d9dev's windowed row 0000 0001 0080 00ff)
        let identity: Vec<u16> = (0..768u32).map(|i| (i % 256 * if windowed { 1 } else { 257 }) as u16).collect();
        prop_assert_eq!(get_ramp(dev), identity.clone());
        prop_assert_eq!(page_gamma(), None);
        let ramp: Vec<u16> = (0..768u64).map(|i| (seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(i * 7919) >> 20) as u16).collect();
        prop_assert_eq!(call(IF_DEVICE, 21, &[dev, 0, 0, ramp_words(&ramp)]), 0);
        prop_assert_eq!(get_ramp(dev), ramp.clone());
        prop_assert_eq!(page_gamma(), (!windowed).then(|| ramp.clone()));
        // what the screen is drawn through: each entry's top byte
        let t = crate::render::ramp_texels(Some(&ramp.clone().try_into().unwrap()));
        for c in 0..3 {
            for i in [0usize, 1, 127, 255] {
                prop_assert_eq!(t[i * 4 + c], (ramp[c * 256 + i] >> 8) as u8);
            }
        }
        if !windowed {
            // Reset to windowed: off the display, still kept
            prop_assert_eq!(call(IF_DEVICE, 16, &[dev, params(true, 0, 1)]), 0);
            prop_assert_eq!(page_gamma(), None);
            prop_assert_eq!(get_ramp(dev), ramp.clone());
            prop_assert_eq!(call(IF_DEVICE, 16, &[dev, params(false, 0, 1)]), 0);
            prop_assert_eq!(page_gamma(), Some(ramp.clone()));
        }
        call(IF_DEVICE, 2, &[dev]);
        prop_assert_eq!(page_gamma(), None, "gone with the device");
        gpu::set_pump(None);
    }
}

#[test]
fn identity_ramp_texels() {
    let t = crate::render::ramp_texels(None);
    for i in 0..256 {
        assert_eq!(t[i * 4..i * 4 + 4], [i as u8, i as u8, i as u8, 255]);
    }
}

/// A `w`x`h` offscreen plain surface of `fmt` in `pool`.
fn surface(dev: u32, w: u32, h: u32, fmt: u32, pool: u32) -> u32 {
    let o = out();
    assert_eq!(call(IF_DEVICE, 36, &[dev, w, h, fmt, pool, o, 0]), 0);
    rd(o)
}

#[test]
fn cursor_as_windows() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init();
    let (_, dev) = device(0x20, params(true, 0, 1));
    // hidden at first, and not shown without an image (Wine's test test_cursor)
    assert_eq!(call(IF_DEVICE, 12, &[dev, 1]), 0);
    assert_eq!(call(IF_DEVICE, 12, &[dev, 1]), 0);
    assert_eq!(call(IF_DEVICE, 10, &[dev, 0, 0, 0]), INVALIDCALL, "no surface");
    let s = surface(dev, 32, 32, f::A8R8G8B8, 3);
    assert_eq!(call(IF_DEVICE, 10, &[dev, 3, 4, s]), 0);
    call(IF_SURFACE, 2, &[s]);
    assert_eq!(call(IF_DEVICE, 12, &[dev, 1]), 0, "still hidden");
    assert_eq!(call(IF_DEVICE, 12, &[dev, 1]), 1, "shown now");
    assert_eq!(call(IF_DEVICE, 12, &[dev, 0]), 1);
    assert_eq!(call(IF_DEVICE, 12, &[dev, 0]), 0);
    assert_eq!(call(IF_DEVICE, 11, &[dev, 40, 50, 0]), 0);
    let d = D3D.lock().unwrap_or_else(|e| e.into_inner());
    let c = &d.as_ref().and_then(|d| d.dev.as_ref()).expect("device").cursor;
    assert_eq!(c.at, (40, 50));
    let img = c.image.as_ref().expect("an image");
    assert_eq!((img.w, img.h, img.hot), (32, 32, (3, 4)));
    drop(d);
    call(IF_DEVICE, 2, &[dev]);
    gpu::set_pump(None);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]
    /// SetCursorProperties takes an A8R8G8B8 surface whose sides are powers of two no larger
    /// than the display's, of any pool (Wine's test test_cursor; the format: documented).
    #[test]
    fn cursor_sizes_and_formats(w in 1u32..80, h in 1u32..80, fmt in prop_oneof![Just(f::A8R8G8B8), Just(f::X8R8G8B8), Just(f::R5G6B5)], pool in prop_oneof![Just(0u32), Just(2), Just(3)], hx in 0u32..70, hy in 0u32..70) {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        init();
        let (_, dev) = device(0x20, params(true, 0, 1));
        let s = surface(dev, w, h, fmt, pool);
        let (mut dw, mut dh) = (0, 0);
        unsafe { crate::display::vnhost_display_size(&mut dw, &mut dh) };
        let ok = fmt == f::A8R8G8B8 && w.is_power_of_two() && h.is_power_of_two() && w as i32 <= dw && h as i32 <= dh;
        prop_assert_eq!(call(IF_DEVICE, 10, &[dev, 0, 0, s]), if ok { 0 } else { INVALIDCALL });
        // (the hot spot inside the image: winref d3d9dev)
        prop_assert_eq!(call(IF_DEVICE, 10, &[dev, hx, hy, s]), if ok && hx < w && hy < h { 0 } else { INVALIDCALL });
        call(IF_SURFACE, 2, &[s]);
        call(IF_DEVICE, 2, &[dev]);
        gpu::set_pump(None);
    }
}

#[test]
fn dialog_box_mode_clip_status_and_vertex_processing() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init();
    // SetDialogBoxMode(TRUE) and (FALSE): taken of every device (winref d3d9dev)
    for (flags, swap) in [(0, 1), (1, 1), (1, 3), (1, 2), (0, 3)] {
        let (_, dev) = device(0x20, params(true, flags, swap));
        assert_eq!(call(IF_DEVICE, 20, &[dev, 1]), 0, "flags {flags} swap {swap}");
        assert_eq!(call(IF_DEVICE, 20, &[dev, 0]), 0);
        call(IF_DEVICE, 2, &[dev]);
    }
    let (_, dev) = device(0x20, params(true, 0, 1));
    let c = words(&[0xcdcd_cdcd, 0xcdcd_cdcd]);
    assert_eq!(call(IF_DEVICE, 63, &[dev, c]), 0);
    assert_eq!((rd(c), rd(c + 4)), (0, u32::MAX), "no union, every bit of the intersection at first (winref d3d9dev)");
    assert_eq!(call(IF_DEVICE, 62, &[dev, words(&[0x12, 0x34])]), 0);
    assert_eq!(call(IF_DEVICE, 63, &[dev, c]), 0);
    assert_eq!((rd(c), rd(c + 4)), (0x12, 0x34));
    assert_eq!(call(IF_DEVICE, 62, &[dev, 0]), INVALIDCALL);
    assert_eq!(call(IF_DEVICE, 63, &[dev, 0]), INVALIDCALL);
    call(IF_DEVICE, 2, &[dev]);
    // software vertex processing: a mixed device's to set, the others' what they were made as
    for (behavior, first, settable) in [(0x20, 1, false), (0x40, 0, false), (0x80, 0, true)] {
        let (_, dev) = device(behavior, params(true, 0, 1));
        assert_eq!(call(IF_DEVICE, 78, &[dev]), first, "behavior {behavior:#x}");
        assert_eq!(call(IF_DEVICE, 77, &[dev, 1 - first]), if settable { 0 } else { INVALIDCALL });
        assert_eq!(call(IF_DEVICE, 78, &[dev]), if settable { 1 - first } else { first });
        call(IF_DEVICE, 2, &[dev]);
    }
    gpu::set_pump(None);
}

#[test]
fn available_texture_mem_goes_down_with_default_resources() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init();
    let (_, dev) = device(0x20, params(true, 0, 1));
    let a = call(IF_DEVICE, 4, &[dev]);
    assert_eq!(a % (1 << 20), 0, "whole MB");
    let mut held = Vec::new();
    for (kind, args) in [(23u32, vec![1024, 1024, 1, 1, f::X8R8G8B8, 0]), (23, vec![1024, 1024, 0, 0, f::A8R8G8B8, 1]), (26, vec![1 << 20, 0, 0, 0]), (23, vec![1024, 1024, 1, 0, f::A8R8G8B8, 2])] {
        let before = call(IF_DEVICE, 4, &[dev]);
        let o = out();
        let mut a = vec![dev];
        a.extend(&args);
        a.extend([o, 0]);
        assert_eq!(call(IF_DEVICE, kind, &a), 0);
        held.push((if kind == 26 { IF_VB } else { IF_TEXTURE }, rd(o)));
        let after = call(IF_DEVICE, 4, &[dev]);
        let pool = args[if kind == 26 { 3 } else { 5 }];
        if pool == 0 {
            assert!(before - after >= 256 * 1024, "{kind} {args:?}: down by {}", before - after);
        } else {
            assert_eq!(before, after, "{kind} {args:?}: managed and system memory take no video memory");
        }
    }
    for (i, o) in held {
        call(i, 2, &[o]);
    }
    assert_eq!(call(IF_DEVICE, 4, &[dev]), a, "back after their release");
    call(IF_DEVICE, 2, &[dev]);
    gpu::set_pump(None);
}

/// IDirect3D9's RegisterSoftwareDevice, CheckDeviceFormatConversion and CheckDepthStencilMatch
/// against Windows' own answers, line by line from the golden (winref d3d9dev).
#[test]
fn d3d9_answers_as_the_golden() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init();
    let (d3d, dev) = device(0x20, params(true, 0, 1));
    let golden = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/winref/golden/d3d9dev-32/stdout.txt")).expect("the d3d9dev golden");
    let named = |n: &str| -> u32 {
        match n {
            "X8R8G8B8" => f::X8R8G8B8, "A8R8G8B8" => f::A8R8G8B8, "R5G6B5" => f::R5G6B5, "X1R5G5B5" => f::X1R5G5B5,
            "A2R10G10B10" => f::A2R10G10B10, "A16B16G16R16F" => f::A16B16G16R16F, "YUY2" => u32::from_le_bytes(*b"YUY2"),
            "UYVY" => u32::from_le_bytes(*b"UYVY"), "DXT1" => f::DXT1, "L8" => f::L8, "P8" => f::P8, "NV12" => u32::from_le_bytes(*b"NV12"),
            "D16" => f::D16, "D24S8" => f::D24S8, "D24X8" => f::D24X8, "D32" => f::D32, "D15S1" => f::D15S1,
            "D16_LOCKABLE" => f::D16_LOCKABLE, "D32F_LOCKABLE" => f::D32F_LOCKABLE, "D24X4S4" => f::D24X4S4, "D24FS8" => f::D24FS8,
            "A8" => f::A8, "A8L8" => f::A8L8, "A1R5G5B5" => f::A1R5G5B5, "A8B8G8R8" => f::A8B8G8R8, "G16R16" => f::G16R16, "R32F" => f::R32F,
            "A32B32G32R32F" => f::A32B32G32R32F, "V8U8" => f::V8U8, "X8B8G8R8" => f::X8B8G8R8,
            "A4R4G4B4" => f::A4R4G4B4, "X4R4G4B4" => f::X4R4G4B4, "A2B10G10R10" => f::A2B10G10R10, "A16B16G16R16" => f::A16B16G16R16,
            "R16F" => f::R16F, "G16R16F" => f::G16R16F, "G32R32F" => f::G32R32F,
            other => panic!("format {other}"),
        }
    };
    let hr = |v: &str| u32::from_str_radix(v.trim_start_matches("0x"), 16).expect("an HRESULT");
    let (mut conversions, mut matches) = (0, 0);
    let mut measured_sources = std::collections::HashSet::new();
    for line in golden.lines().map(str::trim) {
        if let Some(v) = line.strip_prefix("RegisterSoftwareDevice(NULL) ") {
            assert_eq!(call(IF_D3D9, 3, &[d3d, 0]), hr(v));
        } else if let Some(rest) = line.strip_prefix("CheckDeviceFormatConversion ") {
            let (src, pairs) = rest.split_once(" ->").expect("a source");
            let source_format = named(src);
            let w: Vec<&str> = pairs.split_whitespace().collect();
            for p in w.chunks(2) {
                assert_eq!(call(IF_D3D9, 13, &[d3d, 0, 1, source_format, named(p[0])]), hr(p[1]), "{src} -> {}", p[0]);
                if p[0] == "X8R8G8B8" {
                    measured_sources.insert(source_format);
                }
                conversions += 1;
            }
        } else if let Some(rest) = line.strip_prefix("CheckDeviceFormat(DEPTHSTENCIL, SURFACE):") {
            let w: Vec<&str> = rest.split_whitespace().collect();
            for p in w.chunks(2) {
                assert_eq!(call(IF_D3D9, 10, &[d3d, 0, 1, f::X8R8G8B8, 2, 1, named(p[0])]), hr(p[1]), "CheckDeviceFormat(DEPTHSTENCIL) {}", p[0]);
            }
        } else if let Some(rest) = line.strip_prefix("CheckDepthStencilMatch target ") {
            let (target, pairs) = rest.split_once(':').expect("a target");
            let w: Vec<&str> = pairs.split_whitespace().collect();
            for p in w.chunks(2) {
                assert_eq!(call(IF_D3D9, 12, &[d3d, 0, 1, f::X8R8G8B8, named(target), named(p[0])]), hr(p[1]), "{target}: {}", p[0]);
                // (CheckDeviceFormat(DEPTHSTENCIL) says the same of the depth formats)
                let depth = call(IF_D3D9, 10, &[d3d, 0, 1, f::X8R8G8B8, 2, 1, named(p[0])]);
                assert_eq!(depth == 0, hr(p[1]) == 0, "CheckDeviceFormat(DEPTHSTENCIL) {}", p[0]);
                matches += 1;
            }
        }
    }
    assert!(conversions >= 72 && matches >= 32, "{conversions} {matches}");
    for &format in f::Format::ALL {
        assert_eq!(
            format.measured_conversion().is_some(),
            measured_sources.contains(&format.raw()),
            "{format:?}: measured conversion cases must match the d3d9dev golden",
        );
    }
    call(IF_DEVICE, 2, &[dev]);
    gpu::set_pump(None);
}
