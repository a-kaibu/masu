//! d3d9.dll: kura's Direct3D 9 (crates/kura/src/d3d9, drawn with the page's WebGPU), the
//! same module Wine's dlls/vnd3d9 hands its calls to. As there, objects are { vtable, id }
//! blocks the host allocates in guest memory; here every vtable entry is a trap whose handler
//! passes the guest's stack arguments to `vnhost_d3d9_call(interface << 8 | index, args)`.
//! x64: the vtables hold 8-byte pointers and the arguments go to the host as 8-byte slots (RCX,
//! RDX, R8, R9, then the stack; a float in XMM1-3 taken from there), as kura reads them.

#![allow(non_snake_case)]

use crate::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::api::{host_fn, implement, Ctx};
use crate::mem::{self, ps, putp, wp};
use kura::d3d9::HostCalls;

/// Per interface (1..=15, Wine's vnd3d9 numbering, generated from its main.c), each method's
/// argument count with `this`.
const METHODS: [&[u32]; 15] = [
    // (+ IDirect3D9Ex's 5)
    &[3, 1, 1, 2, 1, 4, 3, 5, 3, 6, 7, 7, 6, 5, 4, 2, 7, 3, 5, 4, 8, 3],
    &[3, 1, 1, 1, 1, 1, 2, 2, 3, 2, 4, 4, 2, 3, 3, 1, 2, 5, 5, 3, 2, 4, 3, 9, 10, 8, 7, 7, 9, 9, 5, 3, 3, 3, 6, 4, 7, 3, 3, 2, 2, 1, 1, 7, 3, 3, 3, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 1, 2, 2, 2, 3, 3, 4, 4, 4, 4, 2, 3, 3, 2, 2, 2, 2, 2, 1, 2, 1, 4, 7, 5, 9, 7, 3, 2, 2, 2, 2, 3, 2, 2, 4, 4, 4, 4, 4, 4, 5, 5, 3, 3, 2, 2, 3, 2, 2, 4, 4, 4, 4, 4, 4, 4, 4, 2, 3, 5, 9, 6, 2, 2, 2, 3, 2, 2, 2, 10, 8, 10, 3, 4],
    &[3, 1, 1, 6, 2, 4, 2, 2, 2, 2],
    &[3, 1, 1, 2, 5, 4, 2, 2, 1, 1, 1, 3, 2, 4, 1, 2, 2],
    &[3, 1, 1, 2, 5, 4, 2, 2, 1, 1, 1, 2, 1, 1, 2, 1, 1, 3, 3, 5, 2, 2],
    &[3, 1, 1, 2, 5, 4, 2, 2, 1, 1, 1, 2, 1, 1, 2, 1, 1, 3, 4, 6, 3, 3],
    &[3, 1, 1, 2, 5, 4, 2, 2, 1, 1, 1, 2, 1, 1, 2, 1, 1, 3, 3, 5, 2, 2],
    &[3, 1, 1, 2, 5, 4, 2, 2, 1, 1, 1, 5, 1, 2],
    &[3, 1, 1, 2, 5, 4, 2, 2, 1, 1, 1, 5, 1, 2],
    &[3, 1, 1, 2, 3],
    &[3, 1, 1, 2, 3],
    &[3, 1, 1, 2, 3],
    &[3, 1, 1, 2, 1, 1],
    &[3, 1, 1, 2, 1, 1, 2, 4],
    &[3, 1, 1, 2, 5, 4, 2, 3, 2, 4, 1],
];

const NUM_IFACES: u32 = 15;

/// Direct3D 8 (kura's d3d8.rs on the same device): interface 16 + the Direct3D 9 one, each
/// method's argument count with `this` (from Wine's dlls/vnd3d8/main.c).
const METHODS8: [(u8, &[u32]); 10] = [
    (17, &[3, 1, 1, 2, 1, 4, 2, 4, 3, 6, 7, 6, 6, 4, 2, 7]),
    (18, &[3, 1, 1, 1, 1, 2, 2, 2, 2, 2, 4, 4, 2, 3, 2, 5, 4, 2, 3, 2, 8, 9, 7, 6, 6, 7, 6, 5, 6, 3, 2, 3, 2, 2, 1, 1, 7, 3, 3, 3, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 1, 2, 2, 2, 2, 3, 2, 2, 3, 3, 4, 4, 2, 4, 3, 3, 2, 2, 4, 6, 5, 9, 6, 5, 2, 2, 2, 4, 4, 4, 4, 4, 4, 3, 3, 3, 2, 2, 2, 4, 4, 4, 4, 4, 2]),
    (19, &[3, 1, 1, 5, 4]),
    (20, &[3, 1, 1, 2, 5, 4, 2, 3, 2, 4, 1]),
    (21, &[3, 1, 1, 2, 5, 4, 2, 2, 1, 1, 1, 2, 1, 1, 3, 3, 5, 2, 2]),
    (22, &[3, 1, 1, 2, 5, 4, 2, 2, 1, 1, 1, 2, 1, 1, 3, 4, 6, 3, 3]),
    (23, &[3, 1, 1, 2, 5, 4, 2, 2, 1, 1, 1, 2, 1, 1, 3, 3, 5, 2, 2]),
    (24, &[3, 1, 1, 2, 5, 4, 2, 2, 1, 1, 1, 5, 1, 2]),
    (25, &[3, 1, 1, 2, 5, 4, 2, 2, 1, 1, 1, 5, 1, 2]),
    (31, &[3, 1, 1, 2, 5, 4, 2, 3, 2, 4, 1]),
];
const NUM_IFACES8: u32 = 31;

// (every interface's methods have a u8 index, below 255 so that `(0u8..).zip` never steps
// past it, and the interfaces fit `NUM_IFACES` / `NUM_IFACES8`)
const fn short(l: &[&[u32]]) -> bool {
    match l {
        [] => true,
        [h, rest @ ..] => h.len() < 255 && short(rest),
    }
}
const fn short8(l: &[(u8, &[u32])]) -> bool {
    match l {
        [] => true,
        [(i, h), rest @ ..] => h.len() < 255 && *i as u32 <= NUM_IFACES8 && short8(rest),
    }
}
const _: () = assert!(METHODS.len() == NUM_IFACES as usize && short(&METHODS) && short8(&METHODS8));

/// A method of kura's Direct3D 9 / 8: its interface (Wine's vnd3d9 numbering, 16 + the
/// Direct3D 9 one for Direct3D 8) and vtable index; `vnhost_d3d9_call` takes it as
/// `iface << 8 | index` (`id`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Method {
    iface: u8,
    index: u8,
}

impl Method {
    const fn new(iface: u8, index: u8) -> Self {
        Self { iface, index }
    }
    /// the method of id `id` (a trap's tag), None past 16 bits
    fn of(id: u32) -> Option<Self> {
        let [index, iface, 0, 0] = id.to_le_bytes() else { return None };
        Some(Self { iface, index })
    }
    /// `iface << 8 | index`, as the host takes it
    fn id(self) -> u32 {
        u32::from(self.iface) << 8 | u32::from(self.index)
    }
    /// the same interface's method `index` (QueryInterface's AddRef)
    const fn sibling(self, index: u8) -> Self {
        Self { iface: self.iface, index }
    }
}

const GETNPATCHMODE: Method = Method::new(2, 80);
const SURFACE_GETDC: Method = Method::new(4, 15);
const SURFACE_RELEASEDC: Method = Method::new(4, 16);


unsafe extern "C" fn alloc(size: u32) -> *mut u8 {
    mem::alloc(size).unwrap_or(0) as usize as *mut u8
}
unsafe extern "C" fn free(p: *mut u8) {
    mem::release(p as usize as u32);
}
unsafe extern "C" fn client_rect(hwnd: u32, out: *mut i32) -> i32 {
    let Some(r) = crate::user::client_rect_screen(hwnd) else { return 0 };
    std::slice::from_raw_parts_mut(out, 4).copy_from_slice(&[r.l, r.t, r.r, r.b]);
    1
}
static CALLS: HostCalls = HostCalls { alloc, free, client_rect };
pub fn host_calls() -> &'static HostCalls {
    &CALLS
}

const CREATE_DEVICE9: Method = Method::new(1, 16);
const CREATE_DEVICE8: Method = Method::new(17, 15);

/// IDirect3D9Ex's and IDirect3DDevice9Ex's methods past the Direct3D 9 ones (kura has the
/// Direct3D 9 interfaces only): each answered with its Direct3D 9 counterpart.
fn method_ex(c: &mut Ctx, id: Method) -> Option<u32> {
    // (every Direct3D 9 call comes here: the others go on before any argument is read)
    if !is_ex(id) {
        return None;
    }
    let a = |c: &Ctx, i: u32| if mem::x64() { c.a64(i) as u32 } else { c.a(i) };
    let args: [u32; 10] = std::array::from_fn(|i| a(c, i as u32));
    let this = args[0];
    let r = match id {
        // QueryInterface: the Ex interfaces are these objects
        // (only those of Direct3DCreate9Ex: Direct3DCreate9's object and its devices answer
        // E_NOINTERFACE, as Windows; replay_b2011_anim_team_mm asks a plain device)
        D3D9_QI | DEVICE_QI if args[1] != 0 && matches!(mem::r32(args[1]), IID_D3D9EX | IID_DEVICE9EX) && extended(id, this) => {
            d3d_call(id.sibling(1), &[this]);
            putp(args[2], this);
            0
        }
        // GetAdapterModeCountEx(adapter, filter), EnumAdapterModesEx(adapter, filter, mode, out)
        Method { iface: 1, index: 0x11 } => d3d_call(Method::new(1, 6), &[this, args[1], filter_format(args[2])]),
        Method { iface: 1, index: 0x12 } => {
            let mut m = [0u32; 4];
            let r = d3d_call(Method::new(1, 7), &[this, args[1], filter_format(args[2]), args[3], m.as_mut_ptr() as usize as u32]);
            write_mode_ex(args[4], &m, r);
            r
        }
        // GetAdapterDisplayModeEx(adapter, mode, rotation)
        Method { iface: 1, index: 0x13 } => {
            let mut m = [0u32; 4];
            let r = d3d_call(Method::new(1, 8), &[this, args[1], m.as_mut_ptr() as usize as u32]);
            write_mode_ex(args[2], &m, r);
            if args[3] != 0 {
                mem::w32(args[3], 1);
            }
            r
        }
        // CreateDeviceEx(adapter, type, hwnd, flags, params, fullscreen mode, out)
        Method { iface: 1, index: 0x14 } => {
            let r = d3d_call(CREATE_DEVICE9, &[this, args[1], args[2], args[3], args[4], args[5], args[7]]);
            if r == 0 && args[4] & 2 == 0 && !mem::x64() {
                c.cpu.fpu.cw = 0x7f;
            }
            r
        }
        // GetAdapterLUID
        Method { iface: 1, index: 0x15 } => {
            mem::w32(args[2], 1);
            mem::w32(args[2] + 4, 0);
            0
        }
        // PresentEx(src, dst, hwnd, region, flags)
        Method { iface: 2, index: 0x79 } => d3d_call(Method::new(2, 17), &[this, args[1], args[2], args[3], args[4]]),
        // Get{GPUThreadPriority,MaximumFrameLatency}
        Method { iface: 2, index: 0x7a | 0x7f } => {
            mem::w32(args[1], if id.index == 0x7f { 3 } else { 0 });
            0
        }
        // CreateRenderTargetEx, CreateOffscreenPlainSurfaceEx, CreateDepthStencilSurfaceEx: the
        // same without the usage
        Method { iface: 2, index: 0x81 } => d3d_call(Method::new(2, 28), &args[..9]),
        Method { iface: 2, index: 0x82 } => d3d_call(Method::new(2, 36), &args[..7]),
        Method { iface: 2, index: 0x83 } => d3d_call(Method::new(2, 29), &args[..9]),
        // ResetEx(params, fullscreen mode)
        Method { iface: 2, index: 0x84 } => d3d_call(Method::new(2, 16), &[this, args[1]]),
        // GetDisplayModeEx(swapchain, mode, rotation)
        Method { iface: 2, index: 0x85 } => {
            let mut m = [0u32; 4];
            let r = d3d_call(Method::new(2, 8), &[this, args[1], m.as_mut_ptr() as usize as u32]);
            write_mode_ex(args[2], &m, r);
            if args[3] != 0 {
                mem::w32(args[3], 1);
            }
            r
        }
        // SetConvolutionMonoKernel, ComposeRects, SetGPUThreadPriority, WaitForVBlank,
        // CheckResourceResidency, SetMaximumFrameLatency, CheckDeviceState
        Method { iface: 2, index: 0x77 | 0x78 | 0x7b..=0x7e | 0x80 } => 0,
        _ => return None,
    };
    Some(r)
}
/// The method ids `method_ex` answers (or may: the QueryInterfaces).
fn is_ex(id: Method) -> bool {
    matches!(id, D3D9_QI | DEVICE_QI | Method { iface: 1, index: 0x11..=0x15 } | Method { iface: 2, index: 0x77..=0x85 })
}

/// The IDirect3D9 objects Direct3DCreate9Ex made.
static EXTENDED: Mutex<Vec<u32>> = Mutex::new(Vec::new());
/// Whether `this` (an IDirect3D9 for D3D9_QI, a device otherwise) comes from Direct3DCreate9Ex.
fn extended(id: Method, this: u32) -> bool {
    let d3d = if id == D3D9_QI {
        this
    } else {
        // the device's IDirect3D9 (GetDirect3D), released again
        let out = mem::heap_alloc(8, true);
        let r = d3d_call(DEVICE_QI.sibling(6), &[this, out]);
        let o = mem::rp(out);
        mem::heap_free(out);
        if r != 0 || o == 0 {
            return false;
        }
        d3d_call(D3D9_QI.sibling(2), &[o]);
        o
    };
    EXTENDED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains(&d3d)
}
const D3D9_QI: Method = Method::new(1, 0);
const DEVICE_QI: Method = Method::new(2, 0);
const IID_D3D9EX: u32 = 0x0217_7241;
const IID_DEVICE9EX: u32 = 0xb18b_10ce;
/// D3DDISPLAYMODEFILTER { Size, Format, ScanLineOrdering }: the format (any: X8R8G8B8)
fn filter_format(p: u32) -> u32 {
    if p == 0 { 22 } else { mem::r32(p + 4) }
}
/// D3DDISPLAYMODEEX { Size, Width, Height, RefreshRate, Format, ScanLineOrdering } from a
/// D3DDISPLAYMODE
fn write_mode_ex(p: u32, m: &[u32; 4], r: u32) {
    if p == 0 || r != 0 {
        return;
    }
    mem::w32(p, 24);
    for (i, v) in m.iter().enumerate() {
        mem::w32(p + 4 + 4 * i as u32, *v);
    }
    mem::w32(p + 20, 1);
}

fn method(c: &mut Ctx) -> u32 {
    // (the method's id, kept with its trap: init)
    // (every tag was made by init from a Method)
    let Some(id) = Method::of(crate::api::tag_of(c.trap).0) else { return 0 };
    if let Some(r) = method_ex(c, id) {
        return r;
    }
    match id {
        // a float in ST0 (x64: XMM0)
        GETNPATCHMODE => {
            let bits = d3d_call(id, &[c.a(0)]);
            c.st0 = Some(if mem::x64() { f64::from_bits(u64::from(bits)) } else { f64::from(f32::from_bits(bits)) });
            0
        }
        SURFACE_GETDC => surface_get_dc(c),
        SURFACE_RELEASEDC => surface_release_dc(c),
        // IDirect3D9::CreateDevice, IDirect3D8::CreateDevice: without D3DCREATE_FPU_PRESERVE
        // the x87 goes to single precision, exceptions masked (control word 0x7f), and stays so
        CREATE_DEVICE9 | CREATE_DEVICE8 if !mem::x64() => {
            let behavior = c.a(4);
            let r = unsafe { kura::d3d9::vnhost_d3d9_call(id.id(), (c.esp + 4) as usize as *const u32, &CALLS) };
            if r == 0 && behavior & 2 == 0 {
                c.cpu.fpu.cw = 0x7f;
            }
            r
        }
        _ if mem::x64() => {
            let n = args_of(id);
            let mut slots = [0u64; 16];
            for (i, s) in slots.iter_mut().enumerate().take(n) {
                *s = if FLOAT_ARGS.contains(&(id, i as u32)) && i < 4 { c.xmm(i as u32) & 0xffff_ffff } else { c.a64(i as u32) };
            }
            unsafe { kura::d3d9::vnhost_d3d9_call(id.id(), slots.as_ptr().cast::<u32>(), &CALLS) }
        }
        _ => unsafe { kura::d3d9::vnhost_d3d9_call(id.id(), (c.esp + 4) as usize as *const u32, &CALLS) },
    }
}

/// x64: float parameters passed in XMM registers (method id, argument index with `this`):
/// IDirect3DDevice9::SetNPatchMode. (Clear's Z is the 6th argument: on the stack.)
const FLOAT_ARGS: [(Method, u32); 1] = [(Method::new(2, 79), 1)];

/// The method's argument count with `this`.
fn args_of(id: Method) -> usize {
    let (i, m) = (u32::from(id.iface), usize::from(id.index));
    let list = if i >= 1 && i <= NUM_IFACES { Some(METHODS[i as usize - 1]) } else { METHODS8.iter().find(|x| u32::from(x.0) == i).map(|x| x.1) };
    list.and_then(|l| l.get(m).copied()).unwrap_or(16).min(16) as usize
}

/// Open surface DCs: dc -> (surface, bitmap, previous bitmap)
static DCS: Mutex<Option<HashMap<u32, (u32, u32, u32)>>> = Mutex::new(None);

/// Calls the host directly (x64: the arguments as 8-byte slots).
fn d3d_call(m: Method, args: &[u32]) -> u32 {
    host_call(m.id(), args)
}

/// d3d_call by the host's raw id: a Method's, or one of the host's own (d3d8::CREATE).
fn host_call(id: u32, args: &[u32]) -> u32 {
    if mem::x64() {
        let mut wide = [0u64; 16];
        assert!(args.len() <= wide.len());
        for (w, &a) in wide.iter_mut().zip(args) {
            *w = u64::from(a);
        }
        return unsafe { kura::d3d9::vnhost_d3d9_call(id, wide.as_ptr().cast::<u32>(), &CALLS) };
    }
    unsafe { kura::d3d9::vnhost_d3d9_call(id, args.as_ptr(), &CALLS) }
}

/// Level 0 of an IDirect3DTexture9 (or an IDirect3DSurface9 when `surface`) as top-down BGRA
/// rows: the 32-bit formats as they are, 16-bit R5G6B5 / X1R5G5B5 widened; None for others or
/// when it can't be locked.
pub(crate) fn read_bgra(obj: u32, surface: bool) -> Option<(u32, u32, Vec<u8>)> {
    let (desc_m, lock_m, unlock_m) = if surface { (Method::new(4, 12), Method::new(4, 13), Method::new(4, 14)) } else { (Method::new(5, 17), Method::new(5, 19), Method::new(5, 20)) };
    let mut desc = [0u32; 8];
    let lvl: &[u32] = if surface { &[] } else { &[0] };
    let a = |rest: &[u32]| -> Vec<u32> { [&[obj][..], lvl, rest].concat() };
    if d3d_call(desc_m, &a(&[desc.as_mut_ptr() as usize as u32])) != 0 {
        return None;
    }
    let (w, h) = (desc[6], desc[7]);
    let fmt = SurfFmt::of(desc[0])?;
    let bpp = fmt.bytes();
    let mut lr = [0u32; 4];
    if d3d_call(lock_m, &a(&[lr.as_mut_ptr() as usize as u32, 0, 0x10])) != 0 {
        return None;
    }
    let (pitch, bits) = (lr[0], lr[ps() as usize / 4]);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let row = &mem::bytes(bits + y * pitch, w * bpp);
        out.extend(widen_bgra(&row, fmt));
    }
    d3d_call(unlock_m, &a(&[]));
    Some((w, h, out))
}

/// The D3DFORMATs of a surface GetDC and read_bgra take
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SurfFmt {
    A8R8G8B8,
    X8R8G8B8,
    R5G6B5,
    X1R5G5B5,
    A1R5G5B5,
}

impl SurfFmt {
    pub(crate) fn of(fmt: u32) -> Option<Self> {
        match fmt {
            21 => Some(Self::A8R8G8B8),
            22 => Some(Self::X8R8G8B8),
            23 => Some(Self::R5G6B5),
            24 => Some(Self::X1R5G5B5),
            25 => Some(Self::A1R5G5B5),
            _ => None,
        }
    }
    /// bytes per pixel
    fn bytes(self) -> u32 {
        match self {
            Self::A8R8G8B8 | Self::X8R8G8B8 => 4,
            Self::R5G6B5 | Self::X1R5G5B5 | Self::A1R5G5B5 => 2,
        }
    }
    /// a DIB section's bits per pixel over it, and whether its masks are R5G6B5's
    fn dc_bits(self) -> (u16, bool) {
        match self {
            Self::A8R8G8B8 | Self::X8R8G8B8 => (32, false),
            Self::R5G6B5 => (16, true),
            Self::X1R5G5B5 | Self::A1R5G5B5 => (16, false),
        }
    }
}

/// One row of a surface as BGRA (X formats opaque).
pub(crate) fn widen_bgra(row: &[u8], fmt: SurfFmt) -> Vec<u8> {
    let x5 = |v: u16, s: u32| (u32::from(v >> s & 31) * 255 / 31) as u8;
    match fmt {
        SurfFmt::A8R8G8B8 => row.to_vec(),
        SurfFmt::X8R8G8B8 => row.chunks(4).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        SurfFmt::R5G6B5 => row
            .chunks(2)
            .flat_map(|p| {
                let v = u16::from_le_bytes([p[0], p[1]]);
                [x5(v, 0), (u32::from(v >> 5 & 63) * 255 / 63) as u8, x5(v, 11), 255]
            })
            .collect(),
        SurfFmt::X1R5G5B5 | SurfFmt::A1R5G5B5 => row
            .chunks(2)
            .flat_map(|p| {
                let v = u16::from_le_bytes([p[0], p[1]]);
                let a = if fmt == SurfFmt::A1R5G5B5 && v & 0x8000 == 0 { 0 } else { 255 };
                [x5(v, 0), x5(v, 5), x5(v, 10), a]
            })
            .collect(),
    }
}

/// IDirect3DSurface9::GetDC: the surface locked, a DIB section over its pixels in a memory DC.
fn surface_get_dc(c: &mut Ctx) -> u32 {
    let (this, out) = (c.a(0), c.a(1));
    let mut desc = [0u32; 8];
    if d3d_call(Method::new(4, 12), &[this, desc.as_mut_ptr() as usize as u32]) != 0 {
        return 0x8876_086c;
    }
    let (w, h) = (desc[6], desc[7]);
    let Some(fmt) = SurfFmt::of(desc[0]) else { return 0x8876_086c };
    let (bpp, r565) = fmt.dc_bits();
    // D3DLOCKED_RECT { Pitch, pBits } (x64: pBits at 8)
    let mut lr = [0u32; 4];
    if d3d_call(Method::new(4, 13), &[this, lr.as_mut_ptr() as usize as u32, 0, 0]) != 0 {
        return 0x8876_086c;
    }
    let bits = lr[ps() as usize / 4];
    let hbm = crate::gdi::bitmap_over(w as i32, h as i32, bpp, bits, lr[0], r565);
    let f = crate::export_once!("gdi32", "CreateCompatibleDC");
    let dc = c.call(f, &[0]);
    let f = crate::export_once!("gdi32", "SelectObject");
    let old = c.call(f, &[dc, hbm]);
    DCS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_or_insert_with(HashMap::default).insert(dc, (this, hbm, old));
    putp(out, dc);
    0
}

fn surface_release_dc(c: &mut Ctx) -> u32 {
    let (this, dc) = (c.a(0), c.a(1));
    let Some((s, hbm, old)) = DCS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_or_insert_with(HashMap::default).remove(&dc) else { return 0x8876_086c };
    let _ = this;
    for (dll, name, args) in [("gdi32", "SelectObject", vec![dc, old]), ("gdi32", "DeleteDC", vec![dc]), ("gdi32", "DeleteObject", vec![hbm])] {
        let f = crate::api::export(dll, name);
        c.call(f, &args);
    }
    d3d_call(Method::new(4, 14), &[s]);
    0
}

/// The vtables, given to the host once.
fn init() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        let k = ps();
        let table = mem::heap_alloc(k * (NUM_IFACES + 1), true);
        for (i, m) in (1u8..).zip(METHODS) {
            let v = mem::heap_alloc(k * m.len() as u32, true);
            for (n, &args) in (0u8..).zip(m) {
                let a = host_fn(&format!("d3d9:{i}:{n}"), method, 4 * args);
                crate::api::set_tag(a, Method::new(i, n).id(), 0);
                wp(v + k * u32::from(n), a);
            }
            wp(table + k * u32::from(i), v);
        }
        unsafe { kura::d3d9::vnhost_d3d9_init(table as usize as *const u32, NUM_IFACES + 1) };
    });
}

/// d3d8.dll's vtables, merged into the host's after d3d9's.
fn init8() {
    init();
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        let k = ps();
        let table = mem::heap_alloc(k * (NUM_IFACES8 + 1), true);
        for &(i, m) in METHODS8.iter() {
            let v = mem::heap_alloc(k * m.len() as u32, true);
            for (n, &args) in (0u8..).zip(m) {
                let a = host_fn(&format!("d3d8:{i}:{n}"), method, 4 * args);
                crate::api::set_tag(a, Method::new(i, n).id(), 0);
                wp(v + k * u32::from(n), a);
            }
            wp(table + k * u32::from(i), v);
        }
        unsafe { kura::d3d9::vnhost_d3d9_init(table as usize as *const u32, NUM_IFACES8 + 1) };
    });
}

fn Direct3DCreate8(_: &mut Ctx) -> u32 {
    init8();
    let out = mem::heap_alloc(8, true);
    let r = host_call(kura::d3d9::d3d8::CREATE, &[out]);
    let o = mem::r32(out);
    mem::heap_free(out);
    if r != 0 { 0 } else { o }
}
/// Validate{Vertex,Pixel}Shader: any code passes; the error text out-pointer is the last argument.
fn validate8(c: &mut Ctx, out: u32) -> u32 {
    putp(out, 0);
    if c.a(0) != 0 { 0 } else { 0x8000_4005 }
}
fn ValidateVertexShader(c: &mut Ctx) -> u32 {
    validate8(c, c.a(4))
}
fn ValidatePixelShader(c: &mut Ctx) -> u32 {
    validate8(c, c.a(3))
}

fn Direct3DCreate9(c: &mut Ctx) -> u32 {
    let o = create9(c);
    // (an address an Ex object had before it was released)
    EXTENDED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|&e| e != o);
    o
}
fn create9(c: &mut Ctx) -> u32 {
    init();
    let out = mem::heap_alloc(8, true);
    let r = unsafe { kura::d3d9::vnhost_d3d9_create(c.a(0), out as usize as *mut u32, &CALLS) };
    let o = mem::r32(out);
    mem::heap_free(out);
    if r != 0 { 0 } else { o }
}
/// Direct3DCreate9Ex(version, out): the Direct3D 9 object, whose vtable has the Ex methods.
fn Direct3DCreate9Ex(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::Direct3DCreate9Ex>();
    let out = c.out::<1>();
    let o = create9(c);
    if o != 0 {
        let mut ex = EXTENDED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !ex.contains(&o) {
            ex.push(o);
        }
    }
    out.put(o);
    if o == 0 { 0x8876_086a } else { 0 }
}
fn Direct3DCreate9On12(c: &mut Ctx) -> u32 {
    Direct3DCreate9(c)
}
fn zero(_: &mut Ctx) -> u32 {
    0
}

pub fn register() {
    implement("d3d9", "Direct3DCreate9", Direct3DCreate9);
    implement("d3d9", "Direct3DCreate9Ex", Direct3DCreate9Ex);
    implement("d3d9", "Direct3DCreate9On12", Direct3DCreate9On12);
    implement("d3d8", "Direct3DCreate8", Direct3DCreate8);
    implement("d3d8", "ValidateVertexShader", ValidateVertexShader);
    implement("d3d8", "ValidatePixelShader", ValidatePixelShader);
    implement("d3d8", "DebugSetMute", zero);
    for n in ["D3DPERF_BeginEvent", "D3DPERF_EndEvent", "D3DPERF_GetStatus", "D3DPERF_QueryRepeatFrame", "D3DPERF_SetMarker",
        "D3DPERF_SetOptions", "D3DPERF_SetRegion", "DebugSetMute", "Direct3DShaderValidatorCreate9"]
    {
        implement("d3d9", n, zero);
    }
}

/// Every Direct3D 8/9 vtable trap carries its method id (method reads it without a lock:
/// review C-2), and the stack bytes its arguments take; only the Ex ids go to method_ex.
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tag_tests {
    use super::*;

    #[test]
    fn traps_carry_their_ids() {
        let _g = crate::test_guest::enter();
        init8();
        let trap = |name: String| crate::api::export("sake", &name);
        for i in 1..=NUM_IFACES {
            for (n, &args) in METHODS[i as usize - 1].iter().enumerate() {
                let a = trap(format!("d3d9:{i}:{n}"));
                assert_eq!(crate::api::tag_of(a), (i << 8 | n as u32, 0), "d3d9:{i}:{n}");
                assert_eq!(crate::api::args_of(a), (false, 4 * args));
            }
        }
        for &(i, m) in METHODS8.iter() {
            for (n, &args) in m.iter().enumerate() {
                let a = trap(format!("d3d8:{i}:{n}"));
                assert_eq!(crate::api::tag_of(a), (u32::from(i) << 8 | n as u32, 0), "d3d8:{i}:{n}");
                assert_eq!(crate::api::args_of(a), (false, 4 * args));
            }
        }
    }

    #[test]
    fn ex_ids() {
        // IDirect3D9Ex's five past IDirect3D9's 17 (0x111..=0x115), IDirect3DDevice9Ex's 15
        // past the device's 119 (0x277..=0x285), and both QueryInterfaces
        let ex: Vec<u32> = (0..0x2000).filter(|&id| Method::of(id).is_some_and(is_ex)).collect();
        let want: Vec<u32> = [D3D9_QI.id(), DEVICE_QI.id()].into_iter().chain(0x111..=0x115).chain(0x277..=0x285).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        assert_eq!(ex, want);
        assert_eq!(METHODS[0].len(), 0x16);
        assert_eq!(METHODS[1].len(), 0x86);
    }
}

/// Surface rows widened to BGRA against the code before the surface format became a type
#[cfg(test)]
#[path = "d3d9_widen_tests.rs"]
mod widen_tests;

/// Method ids against the code before they became a type
#[cfg(test)]
#[path = "d3d9_method_tests.rs"]
mod method_tests;
