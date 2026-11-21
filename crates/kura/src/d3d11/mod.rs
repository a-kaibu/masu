//! Direct3D 11 and DXGI for the guest programs, drawn with the page's WebGPU.
//!
//! Same scheme as `d3d9`: d3d11.dll (dlls/vnd3d11) turns every COM method call into a call
//! here with id = interface << 8 | vtable index (interfaces 64+, `names`), through d3d9.dll's
//! dispatcher. Objects are { vtable, id } blocks in guest memory; an object answering several
//! interfaces (the device is also an IDXGIDevice) has a block per interface ("faces").
//!
//! Draws become commands of the d3d9 batch queue (`d3d9::gpu`, run by crates/kura/src/render/d3d.rs):
//! shaders go DXBC -> WGSL (the dxwgsl crate), buffers are guest memory copied into a new page
//! buffer whenever a draw needs them after the guest wrote them.

pub mod d2d;
pub mod d3d10;
mod deferred;
pub mod query;
pub mod check;
mod expand;
mod clear;
pub mod format;
mod hazards;
pub mod names;
pub mod winfmt;
#[cfg(test)]
mod trace_tests;
#[cfg(test)]
mod map_tests;
#[cfg(test)]
mod format_golden_tests;
#[cfg(test)]
mod bind_tests;
#[cfg(test)]
mod cb1_tests;
#[cfg(test)]
mod draw_tests;
#[cfg(test)]
mod query_tests;
#[cfg(test)]
mod deferred_tests;
#[cfg(test)]
mod desc_tests;
#[cfg(test)]
mod copy_tests;
#[cfg(test)]
mod msaa_tests;
#[cfg(test)]
mod bits_tests;
#[cfg(test)]
mod compute_tests;
#[cfg(test)]
mod dxgi_tests;

use kome::num::*;
use crate::d3d9::gpu;
use crate::render::d3d::Factor;
use crate::d3d9::HostCalls;
use std::collections::HashMap;
use std::sync::Mutex;

const S_OK: u32 = 0;
const S_FALSE: u32 = 1;
const E_NOINTERFACE: u32 = 0x8000_4002;
const E_NOTIMPL: u32 = 0x8000_4001;
const E_INVALIDARG: u32 = 0x8007_0057;
const E_OUTOFMEMORY: u32 = 0x8007_000e;
const E_FAIL: u32 = 0x8000_4005;
const DXGI_ERROR_NOT_FOUND: u32 = 0x887a_0002;
const DXGI_ERROR_INVALID_CALL: u32 = 0x887a_0001;
const DXGI_ERROR_UNSUPPORTED: u32 = 0x887a_0004;

/// host ids outside the interfaces (d3d11.dll)
pub const INIT: u32 = 0xffff_0011;
pub const CREATE_DEVICE: u32 = 0xffff_0012;
pub const CREATE_FACTORY: u32 = 0xffff_0013;
/// D3D10CreateDeviceAndSwapChain1(adapter, driver, software, flags, level, sdk, desc, swapchain, device)
pub const CREATE_DEVICE10: u32 = 0xffff_0014;

pub const IF_FACTORY: u32 = 64;
pub const IF_ADAPTER: u32 = 65;
pub const IF_OUTPUT: u32 = 66;
pub const IF_SWAPCHAIN: u32 = 67;
pub const IF_DXGIDEVICE: u32 = 68;
pub const IF_SURFACE: u32 = 69;
pub const IF_DEVICE: u32 = 70;
pub const IF_CONTEXT: u32 = 71;
pub const IF_BUFFER: u32 = 72;
pub const IF_TEX1D: u32 = 73;
pub const IF_TEX2D: u32 = 74;
pub const IF_TEX3D: u32 = 75;
pub const IF_SRV: u32 = 76;
pub const IF_RTV: u32 = 77;
pub const IF_DSV: u32 = 78;
pub const IF_UAV: u32 = 79;
pub const IF_VS: u32 = 80;
pub const IF_PS: u32 = 81;
pub const IF_GS: u32 = 82;
pub const IF_HS: u32 = 83;
pub const IF_DS: u32 = 84;
pub const IF_CS: u32 = 85;
pub const IF_LAYOUT: u32 = 86;
pub const IF_SAMPLER: u32 = 87;
pub const IF_BLEND: u32 = 88;
pub const IF_DEPTH: u32 = 89;
pub const IF_RASTER: u32 = 90;
pub const IF_QUERY: u32 = 91;
pub const IF_LINKAGE: u32 = 92;
pub const IF_CMDLIST: u32 = 93;
/// ID3D10Multithread, on the device and the immediate context (Media Foundation asks for it)
pub const IF_MULTITHREAD: u32 = 94;
/// IDXGIResource, on every buffer and texture (crates/kura/src/d3d11/d3d10.rs has 95..=111)
pub const IF_DXGIRES: u32 = 112;
/// ID3DUserDefinedAnnotation, on every context (PIX events: nobody is listening)
pub const IF_ANNOTATION: u32 = 113;

// IIDs (first dword) and the interfaces that answer them
const IIDS: &[(u32, &[u32])] = &[
    (0x0000_0000, &[]), // IUnknown: any
    (0xaec22fb8, &[IF_FACTORY, IF_ADAPTER, IF_OUTPUT, IF_SWAPCHAIN, IF_DXGIDEVICE, IF_SURFACE]), // IDXGIObject
    (0x7b7166ec, &[IF_FACTORY]),
    (0x770aae78, &[IF_FACTORY]),
    (0x50c83a1c, &[IF_FACTORY]),
    (0x25483823, &[IF_FACTORY]), // IDXGIFactory3
    (0x1bc6ea02, &[IF_FACTORY]), // IDXGIFactory4
    (0x7632e1f5, &[IF_FACTORY]), // IDXGIFactory5
    (0xc1b6694f, &[IF_FACTORY]), // IDXGIFactory6
    (0xa4966eed, &[IF_FACTORY]), // IDXGIFactory7
    (0x2411e7e1, &[IF_ADAPTER]),
    (0x29038f61, &[IF_ADAPTER]),
    (0x0aa1ae0a, &[IF_ADAPTER]), // IDXGIAdapter2
    (0x645967a4, &[IF_ADAPTER]), // IDXGIAdapter3
    (0x3c8d99d1, &[IF_ADAPTER]), // IDXGIAdapter4
    (0xae02eedb, &[IF_OUTPUT]),
    (0x00cddea8, &[IF_OUTPUT]), // IDXGIOutput1
    (0x595e39d1, &[IF_OUTPUT]), // IDXGIOutput2
    (0x8a6bb301, &[IF_OUTPUT]), // IDXGIOutput3
    (0xdc7dca35, &[IF_OUTPUT]), // IDXGIOutput4
    (0x80a07424, &[IF_OUTPUT]), // IDXGIOutput5
    (0x068346e8, &[IF_OUTPUT]), // IDXGIOutput6
    (0x310d36a0, &[IF_SWAPCHAIN]),
    (0x790a45f7, &[IF_SWAPCHAIN]),
    (0x54ec77fa, &[IF_DXGIDEVICE]),
    (0x77db970f, &[IF_DXGIDEVICE]),
    (0x05008617, &[IF_DXGIDEVICE]), // IDXGIDevice2
    (0x6007896c, &[IF_DXGIDEVICE]), // IDXGIDevice3
    (0xcafcb56c, &[IF_SURFACE]),
    (0x4ae63092, &[IF_SURFACE]),
    (0xaba496dd, &[IF_SURFACE]), // IDXGISurface2
    (0x035f3ab4, &[IF_DXGIRES]), // IDXGIResource
    (0xb2daad8b, &[IF_ANNOTATION]), // ID3DUserDefinedAnnotation
    (0x3d3e0379, &[IF_SWAPCHAIN, IF_SURFACE]),
    (0xdb6f6ddb, &[IF_DEVICE]),
    (0xa04bfb29, &[IF_DEVICE]), // ID3D11Device1
    (0x9d06dffa, &[IF_DEVICE]), // ID3D11Device2
    (0xa05c8c37, &[IF_DEVICE]), // ID3D11Device3
    (0x8992ab71, &[IF_DEVICE]), // ID3D11Device4
    (0x8ffde202, &[IF_DEVICE]), // ID3D11Device5
    (0xbb2c6faa, &[IF_CONTEXT]), // ID3D11DeviceContext1
    (0x420d5b32, &[IF_CONTEXT]), // ID3D11DeviceContext2
    (0xb4e3c01d, &[IF_CONTEXT]), // ID3D11DeviceContext3
    (0x917600da, &[IF_CONTEXT]), // ID3D11DeviceContext4
    (0xcc86fabe, &[IF_BLEND]), // ID3D11BlendState1
    (0x1217d7a6, &[IF_RASTER]), // ID3D11RasterizerState1
    (0x6fbd02fb, &[IF_RASTER]), // ID3D11RasterizerState2
    (0x5c1e0d8a, &[IF_LINKAGE]), // ID3DDeviceContextState (a device child like the linkage)
    (0xc0bfa96c, &[IF_CONTEXT]),
    (0x1841e5c8, &[IF_CONTEXT, IF_BUFFER, IF_TEX1D, IF_TEX2D, IF_TEX3D, IF_SRV, IF_RTV, IF_DSV, IF_UAV, IF_VS, IF_PS, IF_GS, IF_HS, IF_DS, IF_CS, IF_LAYOUT, IF_SAMPLER, IF_BLEND, IF_DEPTH, IF_RASTER, IF_QUERY, IF_LINKAGE, IF_CMDLIST]),
    (0xdc8e63f3, &[IF_BUFFER, IF_TEX1D, IF_TEX2D, IF_TEX3D]),
    (0x48570b85, &[IF_BUFFER]),
    (0xf8fb5c27, &[IF_TEX1D]),
    (0x6f15aaf2, &[IF_TEX2D]),
    (0x037e866e, &[IF_TEX3D]),
    (0x839d1216, &[IF_SRV, IF_RTV, IF_DSV, IF_UAV]),
    (0xb0e06fe0, &[IF_SRV]),
    (0xdfdba067, &[IF_RTV]),
    (0x9fdac92a, &[IF_DSV]),
    (0x28acf509, &[IF_UAV]),
    (0x3b301d64, &[IF_VS]),
    (0xea82e40d, &[IF_PS]),
    (0x38325b96, &[IF_GS]),
    (0x8e5c6061, &[IF_HS]),
    (0xf582c508, &[IF_DS]),
    (0x4f5b196e, &[IF_CS]),
    (0xe4819ddc, &[IF_LAYOUT]),
    (0xda6fea51, &[IF_SAMPLER]),
    (0x75b68faa, &[IF_BLEND]),
    (0x03823efb, &[IF_DEPTH]),
    (0x9bb4ab81, &[IF_RASTER]),
    (0x4b35d0cd, &[IF_QUERY]),
    (0xd6c00747, &[IF_QUERY]),
    (0xddf57cba, &[IF_LINKAGE]),
    (0xa24bc4d1, &[IF_CMDLIST]),
    (0x9b7e4e00, &[IF_MULTITHREAD]),
];

/// Interfaces another face of the same object can answer (device <-> IDXGIDevice, 2D textures
/// <-> IDXGISurface)
fn other_face(primary: u32, want: u32) -> bool {
    matches!((primary, want), (IF_DEVICE, IF_DXGIDEVICE) | (IF_TEX2D, IF_SURFACE) | (IF_DEVICE | IF_CONTEXT, IF_MULTITHREAD) | (IF_CONTEXT, IF_ANNOTATION))
        // (a D3D11 device has no ID3D10Device face to ask for: D3D10CreateDevice makes it)
        || (primary != IF_DEVICE && d3d10::ten(primary) == Some(want))
}

pub struct Buffer {
    pub size: u32,
    pub usage: u32,
    pub bind: u32,
    /// what GetDesc gives back
    pub cpu: u32,
    pub misc: u32,
    pub stride: u32,
    pub mem: u32,
    /// page buffer with the current contents (0: none yet)
    pub gpu: u32,
    /// the guest wrote since `gpu` was made
    pub dirty: bool,
    /// every write since the last upload was through a Map NO_OVERWRITE (bytes no draw before
    /// reads: only the changed ones go up, in place)
    pub stream: bool,
    /// a dynamic buffer's bytes as last uploaded (what `stream` uploads are diffed against)
    pub shadow: Vec<u8>,
    /// stream output wrote `gpu` since `mem` was its bytes (expand::pull reads it back)
    pub gpu_written: bool,
}

pub struct Texture {
    /// 1, 2, 3
    pub dim: u32,
    pub w: u32,
    pub h: u32,
    /// array size (depth for 3D)
    pub layers: u32,
    pub mips: u32,
    pub fmt: u32,
    pub page: u32,
    pub usage: u32,
    pub bind: u32,
    pub cube: bool,
    /// what GetDesc gives back: CPU access, misc flags, (samples, quality)
    pub cpu: u32,
    pub misc: u32,
    pub samples: (u32, u32),
    /// a swap chain's buffer
    pub backbuffer: bool,
    pub gpu: u32,
    /// mapped subresources: (subresource, guest memory, pitch, write)
    pub maps: Vec<(u32, u32, u32, bool)>,
    /// a staging texture's subresources whose map memory holds what the page has (since their
    /// last Unmap nothing wrote them): a Map reads no pixels back
    pub clean: Vec<u32>,
    /// the guest's bytes of each subresource written (rows of `format::pitch_rows`, slice after
    /// slice), for formats the page keeps converted (see `keeps_bits`): what Map and copies give
    /// back, as Windows' are the bits written
    pub bits: HashMap<u32, Vec<u8>>,
    /// a staging texture made without initial data that nothing wrote on the page yet: zeros
    /// there (a new WebGPU texture), so a subresource's first Map reads nothing back
    pub pristine: bool,
    /// NV12: its chroma plane's texture (see `Nv12`)
    pub nv12: Option<Nv12>,
}

/// An NV12 texture's chroma plane: a texture of its own (w / 2 x h / 2 R8G8, the texture's
/// layers), held by it; its views of the chroma plane are views of that one, of every kind. The
/// texture's bytes (`bits`) are what the guest has; the page's luma texture and this one follow
/// them (`nv12_push`) and, after a draw or clear into either plane (`drawn`), are read back
/// into them before anything reads or writes the bytes (`nv12_pull`).
pub struct Nv12 {
    pub chroma: u32,
    pub gpu: u32,
    /// the chroma bytes (every layer's) the chroma texture holds
    pub last: Vec<u8>,
    pub drawn: bool,
}

/// The page's texels are the guest's bytes (read back as they are).
fn holds_bits(t: &Texture) -> bool {
    if t.fmt == check::NV12 {
        return false;
    }
    match format::info(t.fmt) {
        Some(f) => f.bpp == format::page_bpp(t.page) && f.block == 1 && f.conv == format::Conv::Copy && !f.no_alpha,
        None => format::page_bpp(t.page) == 4,
    }
}

/// The guest's bytes are kept beside the page's texels: a format the page converts (and
/// 32-bit float RGBA, kept as halves on a device without float32-filterable), of a texture
/// nothing but uploads and copies write (no render target, depth or UAV binding); NV12's (the
/// page has its luma plane, views of the chroma one a texture of their own: `nv12_view_sync`).
fn keeps_bits(t: &Texture) -> bool {
    t.fmt == check::NV12 || (!holds_bits(t) || t.page == format::P_RGBA32F) && t.bind & 0xe0 == 0 && !format::is_depth(t.page)
}

/// A subresource's (level, layer or 0 for 3D, width, height, slices).
fn sub_size(t: &Texture, sub: u32) -> (u32, u32, u32, u32, u32) {
    let level = sub % t.mips.max(1);
    let layer = if t.dim == 3 { 0 } else { sub / t.mips.max(1) };
    let depth = if t.dim == 3 { (t.layers >> level).max(1) } else { 1 };
    (level, layer, (t.w >> level).max(1), (t.h >> level).max(1), depth)
}

#[derive(Clone)]
pub struct View {
    pub res: u32,
    pub fmt: u32,
    /// what GetDesc gives back (format, dimension, a DSV's flags, the union)
    pub desc: Vec<u32>,
    pub mip: u32,
    pub mips: u32,
    pub layer: u32,
    pub layers: u32,
    /// a buffer view (shader resource view of a buffer): `res` is then a one-row texture of
    /// its elements, copied from the buffer when they change (WGSL has no texel buffers; the
    /// shaders fetch from that texture, see wgsl.rs)
    pub buf: Option<BufSrc>,
}

#[derive(Clone)]
pub struct BufSrc {
    pub buf: u32,
    /// first byte, bytes
    pub off: u32,
    pub bytes: u32,
    /// what the texture holds
    pub last: Vec<u8>,
}

pub struct Shader {
    pub code: Vec<u8>,
    pub pixel: bool,
    /// page shader and bindings, per pixel shader for vertex shaders (key 0 otherwise)
    /// by the pixel shader a vertex shader feeds (see shader_for) and the samplers addressing
    /// as WebGPU's don't (dxwgsl::addr)
    pub wgsl: HashMap<(u32, Vec<dxwgsl::addr::Addr>), Option<(u32, Vec<(u32, dxwgsl::Binding)>)>>,
    /// CreateGeometryShaderWithStreamOutput's (`code` a geometry or a vertex shader's)
    pub so: Option<expand::SoDesc>,
}

pub struct Elem {
    pub name: String,
    pub index: u32,
    pub fmt: u32,
    pub slot: u32,
    pub offset: u32,
    pub instance: bool,
    /// InstanceDataStepRate (instance data: instances an element is used for, 0 all of them)
    pub step: u32,
}

pub struct SwapChain {
    pub device: u32,
    pub hwnd: u32,
    pub w: u32,
    pub h: u32,
    pub fmt: u32,
    pub buffer: u32,
    pub presents: u32,
    /// its buffer's DXGI_SAMPLE_DESC (count, quality)
    pub samples: (u32, u32),
}

pub enum Data {
    Factory,
    /// an adapter: WARP's (Microsoft Basic Render Driver) when true
    Adapter(bool),
    Output,
    Device,
    Context,
    SwapChain(SwapChain),
    Buffer(Buffer),
    Texture(Texture),
    View(View),
    Shader(Shader),
    Layout(Vec<Elem>),
    /// state objects: their description's dwords
    State(Vec<u32>),
    /// (type, state: 0 never begun or ended, 1 between Begin and End, 2 ended; timestamp)
    Query(query::Query),
    /// ID3DDeviceContextState: the context state it holds while another one is current
    CtxState(Option<Box<State>>),
    /// a deferred context: its state and the list it records
    Deferred(Box<deferred::Deferred>),
    CmdList(Box<deferred::CmdList>),
    Other,
}

pub struct Obj {
    pub iface: u32,
    pub refs: u32,
    pub binds: u32,
    /// (interface, guest block)
    pub faces: Vec<(u32, u32)>,
    pub data: Data,
    /// (guid, data, the object of SetPrivateDataInterface it holds a reference to)
    pub private: Vec<([u8; 16], Vec<u8>, u32)>,
}

#[derive(Clone)]
pub struct State {
    /// (D3D11_IA_VERTEX_INPUT_RESOURCE_SLOT_COUNT: 32)
    vbs: [(u32, u32, u32); 32],
    ib: (u32, u32, u32),
    layout: u32,
    topology: u32,
    vs: u32,
    ps: u32,
    /// per stage: VS, PS, GS, HS, DS, CS (only VS and PS draw)
    cbs: [[u32; 15]; 6],
    /// each constant buffer slot's range (ID3D11DeviceContext1's *SetConstantBuffers1): its first
    /// constant and how many, in 16-byte constants; *SetConstantBuffers binds (0, 4096)
    cb_ranges: [[CbRange; 15]; 6],
    srvs: [[u32; 128]; 6],
    samplers: [[u32; 16]; 6],
    /// GS, HS, DS and CS shaders (kept for Get*)
    shaders: [u32; 4],
    /// unordered access views: the output merger's and the compute shader's (kept for Get*)
    uavs: [[u32; 8]; 2],
    /// OMSetBlendState's sample mask; SOSetTargets' buffers (kept for Get*)
    sample_mask: u32,
    so: [u32; 4],
    rtvs: [u32; 8],
    dsv: u32,
    /// (all 16: RSGetViewports gives them back; draws use the first)
    viewport: [[f32; 6]; 16],
    scissor: [[i32; 4]; 16],
    blend: u32,
    blend_factor: [f32; 4],
    depth: u32,
    stencil_ref: u32,
    raster: u32,
    /// SetPredication (predicate, value): rendering is off while the predicate's answer is
    /// value (`predicated_off`)
    predicate: (u32, u32),
    /// how many viewports and scissor rects RSSet* gave (RSGet* report them)
    nviewports: u32,
    nscissors: u32,
}

pub struct D11 {
    /// children made by another device object of the device (a D3D11_CREATE_DEVICE_VIDEO_SUPPORT
    /// one: see create_device): child -> that device object, which their GetDevice gives back
    made_by: HashMap<u32, u32>,
    /// CreateDeviceContextState was called: QueryInterface gives the device's D3D10 faces
    ten_faces: bool,
    /// the device was last made by D3D10CreateDevice*: its children's D3D10 faces give it back
    /// (GetDevice), those of a D3D11 device's children give none
    made10: bool,
    /// the feature level the device was last made at (GetFeatureLevel, which shaders it takes)
    level: u32,
    /// PIPELINE_STATISTICS counts of what the device sent the page (d3d11/query.rs)
    stats: query::Stats,
    vtbls: Vec<u32>,
    pub objs: HashMap<u32, Obj>,
    next: u32,
    free_blocks: Vec<u32>,
    calls: *const HostCalls,
    trace: bool,
    /// ID3D10Multithread::SetMultithreadProtected (only reported back)
    mt_protected: bool,
    counts: HashMap<u32, u32>,
    device: u32,
    context: u32,
    factory: u32,
    adapter: u32,
    /// the second adapter, Microsoft Basic Render Driver (WARP: no outputs), last on every
    /// Windows since 8
    warp: u32,
    output: u32,
    st: State,
    /// the ID3DDeviceContextState whose state `st` is (0: the context's own)
    st_owner: u32,
    pipelines: HashMap<Vec<u32>, u32>,
    /// point sprites' viewport uniform (page buffer, the half size written to it)
    sprite_view: (u32, [f32; 2]),
    /// constant buffer ranges a shader reads that are not the whole buffer from its start, or
    /// buffers smaller than the shader's constants: (buffer, first constant, constants the
    /// shader declares) -> the page buffer holding those bytes, zeros past the buffer's end
    /// (as Direct3D reads them), and the bytes last written to it
    cb_copies: HashMap<(u32, u32, u32), (u32, Vec<u8>)>,
    /// page targets set in this batch generation
    targets: Option<(u64, Vec<u32>)>,
    next_gpu: u32,
    /// the page's lost devices seen (render::lost_gen, see `after_lost`)
    lost_seen: u32,
    /// the expanded draws' page buffers and shaders, the queries that count on the page
    expand: expand::Gpu,
}

unsafe impl Send for D11 {}

static D11S: Mutex<Option<D11>> = Mutex::new(None);

/// What a call does after the lock is dropped.
enum After {
    None,
    /// (to window `.0`)
    Present(u32),
    /// readbacks to wait for
    Readback(Vec<u32>),
    /// milliseconds to sleep
    Sleep(u64),
}

/// MASU_D3D_TRACE=2 (the shaders' WGSL, each draw's first vertices): read once, not per draw.
fn trace2() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MASU_D3D_TRACE").as_deref() == Ok("2"))
}

unsafe fn rdf(p: u32, i: usize) -> f32 {
    f32::from_bits(rd(p, i))
}

/// Size of the guest's pointers (and HWNDs, SIZE_Ts): 8 for 64-bit programs.
/// Guest addresses are below 4G either way.
// (the guest-memory words d3d9 reads and writes, the same for d3d11)
use crate::d3d9::device::{ptr, rd, wr, wrp};
use crate::guest::{gc, gp};

/// Pointer `i` of an array of pointers.
unsafe fn rdp(p: u32, i: usize) -> u32 {
    rd(p, i * us(ptr() / 4))
}


/// DXGI_SWAP_CHAIN_DESC as 15 dwords in its 32-bit layout (OutputWindow at 11)
unsafe fn swap_desc(desc: u32) -> [u32; 15] {
    let mut p = [0u32; 15];
    for (i, v) in p.iter_mut().enumerate() {
        // (64-bit: OutputWindow is 8-byte aligned, at dword 12)
        let k = if ptr() == 8 && i >= 11 { i + 1 + usize::from(i > 11) } else { i };
        *v = rd(desc, k);
    }
    p
}

unsafe fn put_swap_desc(desc: u32, p: [u32; 15]) {
    for (i, v) in p.into_iter().enumerate() {
        let k = if ptr() == 8 && i >= 11 { i + 1 + usize::from(i > 11) } else { i };
        wr(desc, k, v);
    }
    if ptr() == 8 {
        wr(desc, 11, 0);
        wr(desc, 13, 0);
    }
}

/// D3D11_MAPPED_SUBRESOURCE { pData, RowPitch, DepthPitch }
unsafe fn put_mapped(out: u32, mem: u32, row: u32, depth: u32, dxgi: bool) {
    if dxgi {
        // DXGI_MAPPED_RECT: Pitch, pBits
        wr(out, 0, row);
        wrp(out + ptr(), 0, mem);
        return;
    }
    wrp(out, 0, mem);
    wr(out + ptr(), 0, row);
    wr(out + ptr(), 1, depth);
}

/// Page ids for textures and buffers (shared with d3d9's: one page)
pub(crate) fn page_id() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x4000_0000);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl D11 {
    fn calls(&self) -> &HostCalls {
        unsafe { &*self.calls }
    }

    fn guest_alloc(&self, size: u32) -> u32 {
        unsafe { lo32((self.calls().alloc)(size.max(16)).expose_provenance()) }
    }

    fn guest_free(&self, p: u32) {
        if p != 0 {
            unsafe { (self.calls().free)(gp::<u8>(p)) }
        }
    }

    fn block(&mut self, iface: u32, id: u32) -> u32 {
        if self.free_blocks.is_empty() {
            let page = self.guest_alloc(65536);
            if page == 0 {
                return 0;
            }
            for i in (0..65536 / 16).rev() {
                self.free_blocks.push(page + i * 16);
            }
        }
        let Some(addr) = self.free_blocks.pop() else { return 0 };
        unsafe {
            wrp(addr, 0, self.vtbls.get(us(iface)).copied().unwrap_or(0));
            *(gp::<u32>(addr + ptr())) = id;
        }
        addr
    }

    /// A new object: returns its guest address.
    fn new_obj(&mut self, iface: u32, data: Data) -> u32 {
        self.next += 1;
        let id = self.next;
        let addr = self.block(iface, id);
        self.objs.insert(id, Obj { iface, refs: 1, binds: 0, faces: vec![(iface, addr)], data, private: Vec::new() });
        if holds_device(iface) && iface != IF_CONTEXT && self.device != 0 {
            self.add_ref(self.device);
        }
        addr
    }

    /// The block of `id` for interface `iface` (made on first use).
    fn face(&mut self, id: u32, iface: u32) -> u32 {
        let Some(o) = self.objs.get(&id) else { return 0 };
        if let Some(&(_, a)) = o.faces.iter().find(|f| f.0 == iface) {
            return a;
        }
        let a = self.block(iface, id);
        if let Some(o) = self.objs.get_mut(&id) {
            o.faces.push((iface, a));
        }
        a
    }

    fn id_of(&self, addr: u32) -> u32 {
        if addr == 0 {
            return 0;
        }
        let id = unsafe { *(gc::<u32>(addr + ptr())) };
        if self.objs.get(&id).is_some_and(|o| o.faces.iter().any(|f| f.1 == addr)) { id } else { 0 }
    }

    fn addr_of(&self, id: u32) -> u32 {
        self.objs.get(&id).map_or(0, |o| o.faces[0].1)
    }

    fn add_ref(&mut self, id: u32) -> u32 {
        match self.objs.get_mut(&id) {
            Some(o) => {
                o.refs += 1;
                let r = o.refs;
                // (a device child holds the device while anyone holds the child)
                if r == 1 && holds_device(o.iface) {
                    self.add_ref(self.device);
                }
                r
            }
            None => 0,
        }
    }

    fn release(&mut self, id: u32) -> u32 {
        let Some(o) = self.objs.get_mut(&id) else { return 0 };
        let was = o.refs;
        o.refs = o.refs.saturating_sub(1);
        let r = o.refs;
        let (iface, binds) = (o.iface, o.binds);
        // (one device per process here: "two" devices, one stored in the other's private data,
        // are one holding itself. Once only those references are left, nobody holds it)
        let selfs = lo32(o.private.iter().filter(|p| p.2 == id).count());
        if r > 0 && r == selfs && id == self.device {
            o.private.retain(|p| p.2 != id);
            o.refs = 0;
            if holds_device(iface) {
                self.release(self.device);
            }
            if binds == 0 {
                self.destroy(id);
            }
            return 0;
        }
        if id == self.device && self.trace && r > 0 {
            // (who still holds the device: device children the application has not let go of)
            let mut left: Vec<String> = self.objs.iter().filter(|(_, o)| o.refs > 0 && holds_device(o.iface) && o.iface != IF_CONTEXT).map(|(&k, o)| format!("{}#{k}x{}", name(o.iface, 0).split(':').next().unwrap_or(""), o.refs)).collect();
            left.sort();
            eprintln!("d3d11: device released, {r} left: {}", left.join(" "));
        }
        if was == 1 && holds_device(iface) {
            self.release(self.device);
        }
        if r == 0 && binds == 0 {
            self.destroy(id);
        }
        r
    }

    fn bind(&mut self, id: u32) {
        if let Some(o) = self.objs.get_mut(&id) {
            o.binds += 1;
        }
    }

    fn unbind(&mut self, id: u32) {
        let Some(o) = self.objs.get_mut(&id) else { return };
        o.binds = o.binds.saturating_sub(1);
        if o.refs == 0 && o.binds == 0 {
            self.destroy(id);
        }
    }

    /// Replaces a bound object (binds the new, unbinds the old).
    fn rebind(&mut self, new: u32, old: u32) {
        if new != 0 {
            self.bind(new);
        }
        if old != 0 {
            self.unbind(old);
        }
    }

    fn destroy(&mut self, id: u32) {
        // (the device, its context and the DXGI objects live as long as the process; but the
        // adapter objects each enumeration makes, Windows' new ones every time)
        let made = id != self.adapter && id != self.warp && matches!(self.objs.get(&id).map(|o| &o.data), Some(Data::Adapter(_)));
        if !made && matches!(self.objs.get(&id).map(|o| o.iface), Some(IF_DEVICE | IF_CONTEXT | IF_FACTORY | IF_ADAPTER | IF_OUTPUT) | None) {
            return;
        }
        let Some(o) = self.objs.remove(&id) else { return };
        self.made_by.remove(&id);
        for &(_, _, held) in &o.private {
            if held != 0 {
                self.release(held);
            }
        }
        for (_, a) in &o.faces {
            unsafe { *(gp::<u32>(a + ptr())) = 0 };
            self.free_blocks.push(*a);
        }
        match o.data {
            Data::Buffer(b) => {
                expand::forget_buffer(self, id);
                self.guest_free(b.mem);
                if b.gpu != 0 {
                    gpu::record(|q| q.cmd(&[gpu::DESTROY_BUFFER, b.gpu]));
                }
                let copies: Vec<(u32, u32, u32)> = self.cb_copies.keys().filter(|k| k.0 == id).copied().collect();
                for k in copies {
                    if let Some((g, _)) = self.cb_copies.remove(&k) {
                        gpu::record(|q| q.cmd(&[gpu::DESTROY_BUFFER, g]));
                    }
                }
            }
            Data::Texture(t) => {
                for (_, mem, _, _) in t.maps {
                    self.guest_free(mem);
                }
                if t.gpu != 0 {
                    gpu::record(|q| q.cmd(&[gpu::DESTROY_TEXTURE, t.gpu]));
                }
                if let Some(n) = t.nv12 {
                    self.unbind(n.chroma);
                }
            }
            Data::View(v) => {
                // (a view holds its resource privately: the resource's own count does not change)
                if let Some(b) = &v.buf {
                    self.unbind(b.buf);
                }
                self.unbind(v.res);
            }
            Data::Deferred(x) => deferred::drop_deferred(self, *x),
            Data::CmdList(l) => deferred::drop_list(self, *l),
            Data::CtxState(Some(s)) => unbind_state(self, &s),
            Data::Shader(s) => {
                // (its translations and the pipelines made of them go from the page: kept for
                // the session before, a game making shaders per scene piled them up)
                let mut mods: Vec<u32> = s.wgsl.values().flatten().map(|r| r.0).collect();
                mods.extend(expand::forget_shader(self, id));
                if !mods.is_empty() {
                    let mut gone = Vec::new();
                    self.pipelines.retain(|k, &mut id| {
                        let keep = !mods.contains(&k[0]) && !mods.contains(&k[1]);
                        if !keep {
                            gone.push(id);
                        }
                        keep
                    });
                    gpu::record(|q| {
                        for id in gone {
                            q.cmd(&[gpu::DESTROY_PIPELINE, id]);
                        }
                        for m in mods {
                            q.cmd(&[gpu::DESTROY_SHADER, m]);
                        }
                    });
                }
            }
            Data::SwapChain(s) => {
                self.release(s.buffer);
                // (the reference new_swapchain took)
                self.release(s.device);
            }
            Data::Factory | Data::Adapter(_) | Data::Output | Data::Device | Data::Context | Data::Layout(_) | Data::State(_) | Data::CtxState(_) | Data::Other => {}
            Data::Query(q) => {
                self.expand.open.retain(|&x| x != id);
                q.forget();
            }
        }
    }
}

/// d3d11.dll's DllMain: its vtables.
pub unsafe fn init(vtbls: *const u32, count: u32) -> u32 {
    let v: Vec<u32> = (0..us(count)).map(|i| rdp(lo32(vtbls.expose_provenance()), i)).collect();
    let mut g = D11S.lock().unwrap_or_else(|e| e.into_inner());
    match g.as_mut() {
        Some(d) => d.vtbls = v,
        None => {
            *g = Some(D11 {
                made_by: HashMap::new(),
                ten_faces: false,
                level: 0xb000,
                made10: false,
                stats: [0; 11],
                vtbls: v,
                objs: HashMap::new(),
                next: 0,
                free_blocks: Vec::new(),
                calls: std::ptr::null(),
                trace: crate::env_on!("MASU_D3D_TRACE"),
                mt_protected: false,
                counts: HashMap::new(),
                device: 0,
                context: 0,
                factory: 0,
                adapter: 0,
                warp: 0,
                output: 0,
                st: default_state(),
                st_owner: 0,
                pipelines: HashMap::new(),
                sprite_view: (0, [0.0; 2]),
                cb_copies: HashMap::new(),
                targets: None,
                next_gpu: 0,
                lost_seen: crate::render::lost_gen(),
                expand: expand::Gpu::default(),
            })
        }
    }
    0
}

/// The arguments of method `m` of interface `iface` (as many as it has: the caller's array is
/// no longer), then zeros up to `n`.
pub(crate) fn args_of(iface: u32, m: u32, a: &dyn Fn(usize) -> u32, n: usize) -> Vec<u32> {
    let k = names::ARGS.get(us(iface.wrapping_sub(64))).and_then(|v| v.get(us(m))).map_or(0, |&k| usize::from(k));
    (0..n).map(|i| if i < k { a(i) } else { 0 }).collect()
}

/// Entry from d3d9's dispatcher: ids INIT.. and interfaces 64+.
pub unsafe fn call(id: u32, args: *const u32, calls: *const HostCalls) -> u32 {
    crate::usage::d3d(id);
    // (64-bit: the arguments are 8-byte slots; their low halves from here on)
    let mut low = [0u32; 16];
    let args = if ptr() == 8 {
        for (i, v) in low.iter_mut().enumerate() {
            *v = *args.add(i * 2);
        }
        low.as_ptr()
    } else {
        args
    };
    if id == INIT {
        return init(gc::<u32>(*args), *args.add(1));
    }
    let (r, after) = {
        let mut g = D11S.lock().unwrap_or_else(|e| e.into_inner());
        let Some(d) = g.as_mut() else { return E_FAIL };
        d.calls = calls;
        after_lost(d);
        let a = |i: usize| *args.add(i);
        let mut after = After::None;
        let r = match id {
            CREATE_DEVICE => create_device(d, args),
            CREATE_DEVICE10 => create_device10(d, args),
            CREATE_FACTORY => {
                let f = dxgi_objects(d);
                let r = query_interface(d, f, IF_FACTORY, a(1), a(2));
                d.release(f);
                r
            }
            _ => {
                let (iface, m) = (id >> 8, id & 0xff);
                let this = d.id_of(a(0));
                let r = if this == 0 {
                    E_INVALIDARG
                } else {
                    match m {
                        0 => query_interface(d, this, iface, a(1), a(2)),
                        1 => d.add_ref(this),
                        2 => d.release(this),
                        _ => method(d, iface, m, this, args, &mut after),
                    }
                };
                if d.trace {
                    *d.counts.entry(id).or_default() += 1;
                    let n = d.counts[&id];
                    if n <= 20 || n.is_power_of_two() {
                        let nargs = (1..6).map(|i| format!("{:#x}", a(i))).collect::<Vec<_>>().join(",");
                        eprintln!("d3d11: {} [{n}] ({nargs}) -> {:#x}", name(iface, m), r);
                    }
                }
                r
            }
        };
        (r, after)
    };
    match after {
        After::None => {}
        After::Present(hwnd) => gpu::present(hwnd),
        After::Sleep(ms) => {
            #[cfg(test)]
            dxgi_tests::check_wait(ms);
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
        After::Readback(reqs) => {
            for req in reqs {
                gpu::wait_readback(req);
            }
        }
    }
    r
}

pub fn name(iface: u32, m: u32) -> String {
    match names::NAMES.get(us(iface.wrapping_sub(64))) {
        Some((i, ms)) => format!("{i}::{}", ms.get(us(m)).copied().unwrap_or("?")),
        None => format!("{iface}::{m}"),
    }
}

/// Shared handles (GetSharedHandle): the texture's id above this
const SHARED_BASE: u32 = 0x4000_0000;
fn shared_handle(id: u32) -> u32 {
    SHARED_BASE + id
}

unsafe fn query_interface(d: &mut D11, id: u32, iface: u32, riid: u32, out: u32) -> u32 {
    if out == 0 || riid == 0 {
        return E_INVALIDARG;
    }
    let first = rd(riid, 0);
    let primary = d.objs.get(&id).map_or(0, |o| o.iface);
    // (a resource's IDXGIObject, IDXGIDeviceSubObject and IDXGIResource are one face, its
    // IDXGISurface another, only for a texture of one level and one layer (1D or 2D))
    if matches!(primary, IF_BUFFER | IF_TEX1D | IF_TEX2D | IF_TEX3D) {
        let surface = matches!(d.objs.get(&id).map(|o| &o.data), Some(Data::Texture(t)) if t.mips == 1 && t.layers == 1 && t.dim != 3);
        let want = match first {
            0xaec22fb8 | 0x3d3e0379 | 0x035f3ab4 => IF_DXGIRES,
            0xcafcb56c | 0x4ae63092 | 0xaba496dd if surface => IF_SURFACE,
            0xcafcb56c | 0x4ae63092 | 0xaba496dd => 0,
            _ => u32::MAX,
        };
        if want != u32::MAX {
            if want == 0 {
                wrp(out, 0, 0);
                return E_NOINTERFACE;
            }
            d.add_ref(id);
            wrp(out, 0, d.face(id, want));
            return S_OK;
        }
    }
    let want = match IIDS.iter().chain(d3d10::IIDS10).find(|x| x.0 == first) {
        Some((0, _)) => primary,
        Some((_, ifs)) => {
            if ifs.contains(&iface) {
                iface
            } else if ifs.contains(&primary) {
                primary
            } else if let Some(&f) = ifs.iter().find(|&&f| other_face(primary, f) || ((d.ten_faces || d3d10::eleven(iface).is_some()) && d3d10::ten(primary) == Some(f))) {
                f
            } else {
                0
            }
        }
        None => 0,
    };
    // (a deferred context is no ID3D11Multithread)
    let want = if want == IF_MULTITHREAD && primary == IF_CONTEXT && id != d.context { 0 } else { want };
    if want == 0 {
        if d.trace {
            eprintln!("d3d11: QueryInterface({first:08x}) on {}: no", name(iface, 0));
        }
        wrp(out, 0, 0);
        return E_NOINTERFACE;
    }
    d.add_ref(id);
    wrp(out, 0, d.face(id, want));
    S_OK
}

/// The factory, adapter and output (made once); returns the factory with a new reference.
fn dxgi_objects(d: &mut D11) -> u32 {
    if d.factory == 0 {
        let f = d.new_obj(IF_FACTORY, Data::Factory);
        d.factory = d.id_of(f);
        let a = d.new_obj(IF_ADAPTER, Data::Adapter(false));
        d.adapter = d.id_of(a);
        let w = d.new_obj(IF_ADAPTER, Data::Adapter(true));
        d.warp = d.id_of(w);
        let o = d.new_obj(IF_OUTPUT, Data::Output);
        d.output = d.id_of(o);
        // (the adapters and the output: only the application's references count, as in Windows'
        // "Adapter has N references left")
        for id in [d.adapter, d.warp, d.output] {
            if let Some(o) = d.objs.get_mut(&id) {
                o.refs = 0;
            }
        }
    } else {
        d.add_ref(d.factory);
    }
    d.factory
}

/// D3D11CreateDeviceAndSwapChain's arguments
unsafe fn create_device(d: &mut D11, args: *const u32) -> u32 {
    let a = |i: usize| *args.add(i);
    let (adapter, driver, levels, nlevels, desc, sc_out, dev_out, level_out, ctx_out) = (a(0), a(1), a(4), a(5), a(7), a(8), a(9), a(10), a(11));
    // (an adapter goes with D3D_DRIVER_TYPE_UNKNOWN, and only with it)
    if (adapter != 0) != (driver == 0) {
        for o in [sc_out, dev_out, ctx_out] {
            wrp(o, 0, 0);
        }
        wr(level_out, 0, 0);
        return E_INVALIDARG;
    }
    // (the swap chain's description is checked before anything is made: no window, or a format
    // a swap chain can't have, and nothing comes back)
    if desc != 0 && sc_out != 0 {
        let p = swap_desc(desc);
        let bad = if p[11] == 0 {
            DXGI_ERROR_INVALID_CALL
        } else if !matches!(p[4], 10 | 24 | 28 | 29 | 87 | 91) {
            E_INVALIDARG
        } else {
            S_OK
        };
        if bad != S_OK {
            for o in [sc_out, dev_out, ctx_out] {
                wrp(o, 0, 0);
            }
            wr(level_out, 0, 0);
            return bad;
        }
    }
    // feature level: the first asked for that is 11_0 or lower
    let mut level = 0xb000;
    if levels != 0 && nlevels > 0 {
        match (0..us(nlevels)).map(|i| rd(levels, i)).find(|&l| l <= 0xb000) {
            Some(l) => level = l,
            None => return E_INVALIDARG,
        }
    }
    dxgi_objects(d);
    d.release(d.factory);
    if d.device == 0 {
        let dev = d.new_obj(IF_DEVICE, Data::Device);
        d.device = d.id_of(dev);
        let ctx = d.new_obj(IF_CONTEXT, Data::Context);
        d.context = d.id_of(ctx);
        // (unreferenced until handed out: see `add_ref`)
        if let Some(o) = d.objs.get_mut(&d.context) {
            o.refs = 0;
        }
        d.st = default_state();
    } else {
        // (a device nobody holds any more: the new one starts from the default state)
        if d.objs.get(&d.device).is_some_and(|o| o.refs == 0) && d.objs.get(&d.context).is_some_and(|o| o.refs == 0) {
            let old = std::mem::replace(&mut d.st, default_state());
            unbind_state(d, &old);
            // (and without what the old one kept in its private data)
            for id in [d.device, d.context] {
                let old = d.objs.get_mut(&id).map_or_default(|o| std::mem::take(&mut o.private));
                for (_, _, held) in old {
                    if held != 0 && held != id {
                        d.release(held);
                    }
                }
            }
        }
        d.add_ref(d.device);
    }
    // D3D11_CREATE_DEVICE_VIDEO_SUPPORT next to a device in use: another device object of the
    // same device (a decoding device of its own, as Unity's VideoPlayer makes one: it tells its
    // textures from its own device's by the device, and opens them through shared handles, #227)
    let mut dev = d.device;
    if a(3) & 0x800 != 0 && dev_out != 0 && d.objs.get(&d.device).is_some_and(|o| o.refs > 1) {
        d.release(d.device);
        let v = d.new_obj(IF_DEVICE, Data::Device);
        dev = d.id_of(v);
    }
    wr(level_out, 0, level);
    d.level = level;
    d.made10 = false;
    if dev_out != 0 {
        wrp(dev_out, 0, d.addr_of(dev));
    } else {
        d.release(d.device);
    }
    if ctx_out != 0 {
        d.add_ref(d.context);
        wrp(ctx_out, 0, d.addr_of(d.context));
    }
    if desc != 0 && sc_out != 0 {
        let p = swap_desc(desc);
        let sc = new_swapchain(d, p[11], p[0], p[1], p[4], (p[7], p[8]));
        wrp(sc_out, 0, sc);
        if sc == 0 {
            return E_FAIL;
        }
    }
    if d.trace {
        eprintln!("d3d11: device, feature level {level:#x}");
    }
    // (nothing asked for back: only a check that the device can be made)
    if dev_out == 0 && ctx_out == 0 && sc_out == 0 {
        return S_FALSE;
    }
    S_OK
}

/// Stored length of a state description: blend states keep D3D11_BLEND_DESC and then
/// LogicOpEnable, LogicOp per target; rasterizer states D3D11_RASTERIZER_DESC2.
fn state_len(iface: u32) -> usize {
    match iface {
        IF_BLEND => 66 + 16,
        IF_DEPTH => 13,
        IF_RASTER => 12,
        _ => 13,
    }
}

/// A state object for `desc`: one object per description while it lives, as Windows gives
/// back; S_FALSE without an out pointer.
unsafe fn new_state(d: &mut D11, iface: u32, desc: Vec<u32>, out: u32) -> u32 {
    if out == 0 {
        return S_FALSE;
    }
    let same = d.objs.iter().find(|(_, o)| o.iface == iface && matches!(&o.data, Data::State(x) if *x == desc)).map(|(&id, _)| id);
    if let Some(id) = same {
        d.add_ref(id);
        wrp(out, 0, d.addr_of(id));
        return S_OK;
    }
    let o = d.new_obj(iface, Data::State(desc));
    wrp(out, 0, o);
    S_OK
}

/// A state description as the state object keeps it (what GetDesc gives and what two
/// descriptions are compared by), or None for one Windows refuses (E_INVALIDARG): fields the
/// state ignores set as Windows sets them, BOOLs as 0 or 1 (tests/winref/golden/d3d11desc-32).
/// Blend: the targets in use (all with IndependentBlendEnable, else the first, which every
/// target then is) have masks of at most 0xf and, blending, blends of 1-11 or 14-19 (no color
/// ones for alpha) and operations of 1-5; a target that does not blend has ONE, ZERO, ADD.
/// Depth stencil: a write mask of 0 or 1 and, testing depth, a function of 1-8 (else LESS and a
/// full write mask); with stencil, operations and functions of 1-8 (else masks 0xff and KEEP,
/// KEEP, KEEP, ALWAYS); the bytes after the two stencil masks are padding. Rasterizer: fill
/// 2-3, cull 1-3, a depth bias clamp that is a number. Sampler: one of the filters, addresses
/// of 1-5, at most 16 anisotropy (0 without an anisotropic filter), a comparison of 1-8 with a
/// comparison filter (else NEVER), a bias of -16 to 15.99 and LODs that are numbers; the
/// border color is 0 without a BORDER address.
fn normalize_state(m: u32, desc: &mut [u32]) -> Option<()> {
    let b = |v: &mut u32| *v = u32::from(*v != 0);
    let ok = |c: bool| c.then_some(());
    let f = |v: u32| f32::from_bits(v);
    match m {
        20 => {
            b(&mut desc[0]);
            b(&mut desc[1]);
            let used = if desc[1] != 0 { 8 } else { 1 };
            let blend = |v: u32| matches!(v, 1..=11 | 14..=19);
            let alpha = |v: u32| blend(v) && !matches!(v, 3 | 4 | 9 | 10 | 16 | 17);
            for t in 0..used {
                let rt = &mut desc[2 + t * 8..2 + t * 8 + 8];
                rt[7] &= 0xff;
                ok(rt[7] <= 0xf)?;
                b(&mut rt[0]);
                if rt[0] != 0 {
                    ok(blend(rt[1]) && blend(rt[2]) && alpha(rt[4]) && alpha(rt[5]) && (1..=5).contains(&rt[3]) && (1..=5).contains(&rt[6]))?;
                } else {
                    rt[1..7].copy_from_slice(&[2, 1, 1, 2, 1, 1]);
                }
            }
            if used == 1 {
                let rt0: Vec<u32> = desc[2..10].to_vec();
                for t in 1..8 {
                    desc[2 + t * 8..10 + t * 8].copy_from_slice(&rt0);
                }
            }
        }
        21 => {
            b(&mut desc[0]);
            ok(desc[1] <= 1)?;
            if desc[0] != 0 {
                ok((1..=8).contains(&desc[2]))?;
            } else {
                (desc[1], desc[2]) = (1, 2);
            }
            b(&mut desc[3]);
            desc[4] &= 0xffff;
            if desc[3] != 0 {
                ok(desc[5..13].iter().all(|v| (1..=8).contains(v)))?;
            } else {
                desc[4] = 0xffff;
                for f in 0..2 {
                    desc[5 + 4 * f..9 + 4 * f].copy_from_slice(&[1, 1, 1, 8]);
                }
            }
        }
        22 => {
            ok((2..=3).contains(&desc[0]) && (1..=3).contains(&desc[1]) && !f(desc[4]).is_nan())?;
            b(&mut desc[2]);
            for v in &mut desc[6..10] {
                b(v);
            }
        }
        23 => {
            let filter = desc[0];
            ok(filter & !0x1ff == 0 && matches!(filter & 0x7f, 0 | 1 | 4 | 5 | 0x10 | 0x11 | 0x14 | 0x15 | 0x55))?;
            ok(desc[1..4].iter().all(|v| (1..=5).contains(v)) && desc[5] <= 16)?;
            if filter & 0x7f != 0x55 {
                desc[5] = 0;
            }
            if filter & 0x180 == 0x80 {
                ok((1..=8).contains(&desc[6]))?;
            } else {
                desc[6] = 1;
            }
            ok((-16.0..=15.99).contains(&f(desc[4])) && !f(desc[11]).is_nan() && !f(desc[12]).is_nan())?;
            if !desc[1..4].contains(&4) {
                desc[7..11].fill(0);
            }
        }
        _ => {}
    }
    Some(())
}

/// D3D10CreateDevice(AndSwapChain)(1): the D3D11 device's D3D10 face
unsafe fn create_device10(d: &mut D11, args: *const u32) -> u32 {
    let a = |i: usize| *args.add(i);
    let dev_out = a(8);
    // (out slots on the heap, not the stack: the host's heap is where 32-bit pointers reach in
    // native tests, a stack frame under ASan is not)
    let mut dev = vec![0u32; 2];
    let mut level = vec![0u32; 1];
    let v = [a(0), a(1), a(2), a(3), 0, 0, 7, a(6), a(7), lo32(dev.as_mut_ptr().expose_provenance()), lo32(level.as_mut_ptr().expose_provenance()), 0];
    let r = create_device(d, v.as_ptr());
    if r != S_OK {
        return r;
    }
    d.made10 = true;
    let id = d.id_of(rdp(lo32(dev.as_mut_ptr().expose_provenance()), 0));
    if dev_out != 0 {
        let f = d.face(id, d3d10::IF10_DEVICE);
        wrp(dev_out, 0, f);
    } else {
        d.release(id);
    }
    S_OK
}

/// Whether an object of this interface is a device child: one that holds a reference to the
/// device while the application holds one to it (as in Windows: "Device has N references left").
fn holds_device(iface: u32) -> bool {
    // (a swap chain holds its device for as long as it lives: new_swapchain)
    !matches!(iface, IF_DEVICE | IF_FACTORY | IF_ADAPTER | IF_OUTPUT | IF_SWAPCHAIN)
}

/// The stage (2 GS, 3 HS, 4 DS, 5 CS) of a context method of the other stages.
fn stage_of(m: u32) -> usize {
    match m {
        22 | 23 | 31 | 32 | 81 | 82 | 87 | 88 => 2,
        59..=62 | 97..=100 => 3,
        63..=66 | 101..=104 => 4,
        _ => 5,
    }
}

/// Lets go of everything a state binds (ClearState, a new device).
fn unbind_state(d: &mut D11, old: &State) {
    for h in deferred::state_objects(old) {
        d.unbind(h);
    }
}

fn default_state() -> State {
    State {
        vbs: [(0, 0, 0); 32],
        ib: (0, 0, 0),
        layout: 0,
        topology: 0,
        vs: 0,
        ps: 0,
        cbs: [[0; 15]; 6],
        cb_ranges: [[CbRange::WHOLE; 15]; 6],
        srvs: [[0; 128]; 6],
        samplers: [[0; 16]; 6],
        shaders: [0; 4],
        uavs: [[0; 8]; 2],
        sample_mask: 0xffff_ffff,
        so: [0; 4],
        rtvs: [0; 8],
        dsv: 0,
        viewport: [[0.0; 6]; 16],
        scissor: [[0; 4]; 16],
        blend: 0,
        blend_factor: [1.0; 4],
        depth: 0,
        stencil_ref: 0,
        raster: 0,
        predicate: (0, 0),
        nviewports: 0,
        nscissors: 0,
    }
}

/// A swap chain (address) whose buffer has `samples` (DXGI_SAMPLE_DESC: count, quality; a
/// multisampled one with BitBlt-model swap effects, resolved when it is presented).
unsafe fn new_swapchain(d: &mut D11, hwnd: u32, w: u32, h: u32, fmt: u32, samples: (u32, u32)) -> u32 {
    let mut rect = [0i32; 4];
    (d.calls().client_rect)(hwnd, rect.as_mut_ptr());
    let w = if w == 0 { (rect[2] - rect[0]).max(1).cast_unsigned() } else { w };
    let h = if h == 0 { (rect[3] - rect[1]).max(1).cast_unsigned() } else { h };
    let fmt = if format::info(fmt).is_some() { fmt } else { 28 };
    let buffer = new_backbuffer(d, w, h, fmt, samples);
    let bid = d.id_of(buffer);
    d.add_ref(d.device);
    let sc = d.new_obj(IF_SWAPCHAIN, Data::SwapChain(SwapChain { device: d.device, hwnd, w, h, fmt, buffer: bid, presents: 0, samples }));
    // (always: in every run's log, a game's picture starts here)
    eprintln!("sake: d3d11 swap chain {w}x{h} format {fmt} samples {} window {hwnd:#x} (client {}x{})", samples.0, rect[2] - rect[0], rect[3] - rect[1]);
    sc
}

/// A swap chain's buffer (address): views of it may take the sRGB twin of its format
/// (DXGI's back buffer cast).
fn new_backbuffer(d: &mut D11, w: u32, h: u32, fmt: u32, samples: (u32, u32)) -> u32 {
    let o = new_texture(d, 2, w, h, 1, 1, fmt, 0, 0x8 | 0x20, false, samples.0);
    let id = d.id_of(o);
    if let Some(Data::Texture(t)) = d.objs.get_mut(&id).map(|o| &mut o.data) {
        t.backbuffer = true;
        t.samples = (samples.0.max(1), samples.1);
    }
    o
}

/// A texture object (address); `samples` above 1: multisampled (on the page 4x where WebGPU
/// can, see render::d3d create_texture2).
#[allow(clippy::too_many_arguments)]
fn new_texture(d: &mut D11, dim: u32, w: u32, h: u32, layers: u32, mips: u32, fmt: u32, usage: u32, bind: u32, cube: bool, samples: u32) -> u32 {
    let page = format::page(fmt, bind).unwrap_or(format::P_BGRA8);
    let max = w.max(h).max(1).bit_width();
    let mips = if mips == 0 { max } else { mips.min(max) };
    let gpu = page_id();
    let layers = layers.max(1);
    // (bit 8: bound for unordered access, a storage texture on the page; bit 9: multisampled;
    // bit 10: 24-bit UNORM depth, which the page's depth24plus may hold as floats)
    let uav = if bind & 0x80 != 0 { 0x100 } else { 0 } | if samples > 1 { 0x200 } else { 0 } | if (44..=47).contains(&fmt) { 0x400 } else { 0 };
    gpu::record(|q| q.cmd(&[gpu::CREATE_TEXTURE2, gpu, w.max(1), h.max(1), layers, mips, page, uav | match dim {
        3 => 1,
        1 => 2,
        _ => 0,
    }]));
    let iface = match dim {
        1 => IF_TEX1D,
        3 => IF_TEX3D,
        _ => IF_TEX2D,
    };
    d.new_obj(iface, Data::Texture(Texture {
        dim,
        w,
        h,
        layers,
        mips,
        fmt,
        page,
        usage,
        bind,
        cube,
        cpu: match usage {
            2 => 0x10000,
            3 => 0x30000,
            _ => 0,
        },
        misc: if cube { 4 } else { 0 },
        samples: (samples.max(1), 0),
        backbuffer: false,
        gpu,
        maps: Vec::new(),
        clean: Vec::new(),
        bits: HashMap::new(),
        pristine: false,
        nv12: None,
    }))
}

/// Uploads a subresource (a box of it) from guest memory (`pitch` bytes per row, `slice` per 3D
/// slice) to the page texture, and keeps the bytes for formats the page converts.
unsafe fn upload_texture(t: &mut Texture, sub: u32, mem: usize, pitch: u32, slice: u32, bx: Option<[u32; 6]>) {
    // (of depth only 16-bit: WebGPU writes into no other; copies into 32-bit float depth are
    // drawn, see render/d3d.rs draw_depth)
    if t.gpu == 0 || matches!(t.page, format::P_D24S8 | format::P_D32F) || mem == 0 {
        return;
    }
    let (level, layer, lw, lh, ld) = sub_size(t, sub);
    let [x0, y0, z0, x1, y1, z1] = bx.unwrap_or([0, 0, 0, lw, lh, ld]);
    // (NV12: a box on even luma texels; `mem` its luma rows, then its chroma rows)
    if t.fmt == check::NV12 {
        let (w, h) = (x1.min(lw).saturating_sub(x0), y1.min(lh).saturating_sub(y0));
        if (x0 | y0 | w | h) % 2 != 0 || w == 0 || h == 0 {
            return;
        }
        nv12_keep(t, sub, mem, pitch, [x0, y0, w, h]);
        let src = std::ptr::with_exposed_provenance::<u8>(mem);
        let mut out = Vec::with_capacity(us(w * h));
        for y in 0..h {
            out.extend_from_slice(std::slice::from_raw_parts(src.add(us(y * pitch)), us(w)));
        }
        gpu::record(|q| {
            let off = q.blob(&out);
            q.cmd(&[gpu::WRITE_TEXTURE2, t.gpu, level, layer, x0, y0, w, h, w, off]);
        });
        return;
    }
    // (2D: one slice, whatever the box's front and back)
    let (z0, z1) = if t.dim == 3 { (z0.min(ld), z1.min(ld)) } else { (0, 1) };
    let Some(f) = format::info(t.fmt) else { return };
    let (x0, y0) = (x0 / f.block * f.block, y0 / f.block * f.block);
    let (w, h) = (x1.min(lw).saturating_sub(x0), y1.min(lh).saturating_sub(y0));
    if w == 0 || h == 0 || z1 <= z0 {
        return;
    }
    keep_bits(t, sub, mem, pitch, slice, [x0, y0, z0, w, h, z1]);
    for z in z0..z1 {
        let src = std::ptr::with_exposed_provenance::<u8>(mem).add(us((z - z0) * slice));
        let (out, bpr) = page_texels(t, &f, src, pitch, w, h);
        let lz = if t.dim == 3 { z } else { layer };
        gpu::record(|q| {
            let off = q.blob(&out);
            q.cmd(&[gpu::WRITE_TEXTURE2, t.gpu, level, lz, x0, y0, w, h, bpr, off]);
        });
    }
}

/// The page's device was lost and everything on it made again empty (render/mod.rs
/// `vnhost_render_lost`): what the guest still has goes up again. Every buffer from its guest
/// memory (the next use uploads it whole), every texture whose bytes are kept (`keeps_bits`)
/// from those. The rest (render targets, textures in formats the page holds as they are) is
/// zeros until the game writes it again.
fn after_lost(d: &mut D11) {
    let g = crate::render::lost_gen();
    if g == d.lost_seen {
        return;
    }
    d.lost_seen = g;
    let mut ids: Vec<u32> = d.objs.keys().copied().collect();
    ids.sort_unstable();
    for id in ids {
        match d.objs.get_mut(&id).map(|o| &mut o.data) {
            Some(Data::Buffer(b)) if b.gpu != 0 => {
                b.dirty = true;
                b.stream = false;
            }
            Some(Data::Texture(t)) if t.gpu != 0 && !t.bits.is_empty() => {
                let mut subs: Vec<(u32, Vec<u8>)> = t.bits.iter().map(|(&k, v)| (k, v.clone())).collect();
                subs.sort_unstable_by_key(|s| s.0);
                for (sub, bytes) in subs {
                    let (_, _, lw, lh, _) = sub_size(t, sub);
                    let (pitch, rows) = format::pitch_rows(t.fmt, lw, lh);
                    unsafe { upload_texture(t, sub, bytes.as_ptr().expose_provenance(), pitch, pitch * rows, None) };
                }
                t.clean.clear();
            }
            _ => {}
        }
    }
}

/// Keeps the bytes of a box (x, y, z, width, height, end z, inside the subresource, x and y on
/// blocks) of guest texels for a format the page converts (see `keeps_bits`).
unsafe fn keep_bits(t: &mut Texture, sub: u32, mem: usize, pitch: u32, slice: u32, b: [u32; 6]) {
    let Some(f) = format::info(t.fmt).filter(|_| keeps_bits(t)) else { return };
    let [x0, y0, z0, w, h, z1] = b;
    let (_, _, lw, lh, ld) = sub_size(t, sub);
    let (gp, rows) = format::pitch_rows(t.fmt, lw, lh);
    let keep = t.bits.entry(sub).or_insert_with(|| vec![0; us(gp * rows * ld)]);
    let (bx0, by0) = (x0 / f.block, y0 / f.block);
    let (row, n) = format::pitch_rows(t.fmt, w, h);
    for z in z0..z1 {
        for r in 0..n {
            let from = std::ptr::with_exposed_provenance::<u8>(mem).add(us((z - z0) * slice + r * pitch));
            let at = us((z * rows + by0 + r) * gp + bx0 * f.bpp);
            keep[at..at + us(row)].copy_from_slice(std::slice::from_raw_parts(from, us(row)));
        }
    }
}

/// A box of guest texels (`pitch` bytes per row) as the page's (and its bytes per row).
unsafe fn page_texels(t: &Texture, f: &format::Format, src: *const u8, pitch: u32, w: u32, h: u32) -> (Vec<u8>, u32) {
    format::to_page(f, t.page, src, pitch, w, h)
}

unsafe fn method(d: &mut D11, iface: u32, m: u32, this: u32, args: *const u32, after: &mut After) -> u32 {
    let a = |i: usize| *args.add(i);
    match iface {
        // IDXGIObject: SetPrivateData, SetPrivateDataInterface, GetPrivateData
        IF_FACTORY | IF_ADAPTER | IF_OUTPUT | IF_SWAPCHAIN | IF_DXGIDEVICE | IF_SURFACE | IF_DXGIRES if (3..=5).contains(&m) => private_data(d, [5, 6, 4][us(m) - 3], this, &a),
        IF_FACTORY => factory_call(d, m, this, &a),
        IF_ADAPTER => adapter_call(d, m, this, &a),
        IF_OUTPUT => output_call(d, m, &a, after),
        IF_SWAPCHAIN => swapchain_call(d, m, this, &a, after),
        // Enter/Leave: every call already runs under the host's lock
        IF_MULTITHREAD => match m {
            5 => u32::from(std::mem::replace(&mut d.mt_protected, a(1) != 0)),
            6 => u32::from(d.mt_protected),
            _ => 0,
        },
        IF_DXGIDEVICE => match m {
            6 | 7 => {
                // GetParent(riid, out) / GetAdapter(out)
                if m == 6 {
                    return parent(d, d.adapter, a(1), a(2));
                }
                d.add_ref(d.adapter);
                wrp(a(1), 0, d.addr_of(d.adapter));
                S_OK
            }
            8 => {
                // CreateSurface(desc, count, usage, shared, out[count]): 2D textures
                if a(1) == 0 || a(2) == 0 || a(5) == 0 {
                    return E_INVALIDARG;
                }
                let (w, h, fmt) = (rd(a(1), 0), rd(a(1), 1), rd(a(1), 2));
                if format::info(fmt).is_none() {
                    return E_INVALIDARG;
                }
                let usage = a(3);
                let bind = if usage & 0x20 != 0 { 0x20 } else { 0 } | if usage & 0x10 != 0 { 8 } else { 0 } | if usage & 0x400 != 0 { 0x80 } else { 0 };
                for i in 0..a(2) {
                    let o = new_texture(d, 2, w, h, 1, 1, fmt, 0, bind, false, 1);
                    let f = d.face(d.id_of(o), IF_SURFACE);
                    wrp(a(5), us(i), f);
                }
                S_OK
            }
            9 => {
                // QueryResourceResidency(resources, status, n): all in memory
                std::ptr::write_bytes(gp::<u8>(a(2)), 0, 4 * us(a(3)));
                S_OK
            }
            10 | 12 => S_OK,
            11 | 13 => {
                wr(a(1), 0, if m == 13 { 3 } else { 0 });
                S_OK
            }
            // IDXGIDevice2/3: OfferResources, ReclaimResources(n, res, discarded), Trim
            14 | 17 => S_OK,
            15 => {
                if a(3) != 0 {
                    std::ptr::write_bytes(gp::<u8>(a(3)), 0, 4 * us(a(1)));
                }
                S_OK
            }
            _ => E_NOTIMPL,
        },
        // BeginEvent / EndEvent: -1 (no tool attached); SetMarker; GetStatus: FALSE
        IF_ANNOTATION => match m {
            3 | 4 => 0xffff_ffff,
            _ => 0,
        },
        IF_DXGIRES => match m {
            6 | 7 => parent(d, d.made_by.get(&this).copied().unwrap_or(d.device), a(1), a(2)),
            8 => {
                // GetSharedHandle: a handle of a texture made with D3D11_RESOURCE_MISC_SHARED (or
                // SHARED_KEYEDMUTEX / SHARED_NTHANDLE), for OpenSharedResource; none otherwise
                let shared = matches!(d.objs.get(&this).map(|o| &o.data), Some(Data::Texture(t)) if t.misc & 0x902 != 0);
                wr(a(1), 0, if shared { shared_handle(this) } else { 0 });
                S_OK
            }
            9 => {
                // GetUsage: DXGI_USAGE_SHADER_INPUT / RENDER_TARGET_OUTPUT / UNORDERED_ACCESS by
                // the bind flags (and BACK_BUFFER for a swap chain's)
                let (bind, back) = match d.objs.get(&this).map(|o| &o.data) {
                    Some(Data::Texture(t)) => (t.bind, t.backbuffer),
                    Some(Data::Buffer(b)) => (b.bind, false),
                    _ => (0, false),
                };
                let mut u = 0;
                for (b, f) in [(0x8, 0x10), (0x20, 0x20), (0x80, 0x400)] {
                    if bind & b != 0 {
                        u |= f;
                    }
                }
                if back {
                    u |= 0x40;
                }
                wr(a(1), 0, u);
                S_OK
            }
            10 | 11 => {
                let primary = d.objs.get(&this).map_or(0, |o| o.iface);
                child_call(d, primary, m - 2, this, &a)
            }
            _ => E_NOTIMPL,
        },
        IF_SURFACE => match m {
            6 | 7 => parent(d, d.made_by.get(&this).copied().unwrap_or(d.device), a(1), a(2)),
            8 => {
                // GetDesc: Width Height Format SampleDesc
                let Some(Data::Texture(t)) = d.objs.get(&this).map(|o| &o.data) else { return E_FAIL };
                for (i, v) in [t.w, t.h, t.fmt, t.samples.0, t.samples.1].into_iter().enumerate() {
                    wr(a(1), i, v);
                }
                S_OK
            }
            // Map(rect, flags): DXGI_MAP_READ 1 / WRITE 2 / DISCARD 4 as D3D11_MAP 1 / 2 / 3 / 4
            9 => {
                let f = a(2);
                let ty = if f & 4 != 0 { 4 } else if f & 3 == 3 { 3 } else if f & 2 != 0 { 2 } else { 1 };
                return map(d, d.addr_of(this), 0, ty, a(1), true, after);
            }
            10 => {
                unmap(d, d.addr_of(this), 0);
                S_OK
            }
            // IDXGISurface2::GetResource(riid, out, subresource): the texture it is (a surface is
            // one level of one layer: subresource 0)
            13 => {
                let r = query_interface(d, this, IF_TEX2D, a(1), a(2));
                if a(3) != 0 {
                    wr(a(3), 0, 0);
                }
                r
            }
            _ => E_NOTIMPL,
        },
        IF_DEVICE if this != d.device => {
            // (another device object: what it makes is its)
            let before = d.next;
            let r = device_call(d, m, this, &a);
            for id in before + 1..=d.next {
                if d.objs.contains_key(&id) {
                    d.made_by.insert(id, this);
                }
            }
            r
        }
        IF_DEVICE => device_call(d, m, this, &a),
        IF_CONTEXT if this != d.context => deferred::call(d, this, m, &a, after),
        IF_CONTEXT => context_call(d, m, &a, after),
        d3d10::IF10_DEVICE => d3d10::device(d, m, this, &a, after),
        d3d10::IF10_BUFFER..=d3d10::IF10_QUERY => d3d10::child(d, iface, m, this, &a, after),
        _ => child_call(d, iface, m, this, &a),
    }
}

unsafe fn parent(d: &mut D11, id: u32, riid: u32, out: u32) -> u32 {
    let iface = d.objs.get(&id).map_or(0, |o| o.iface);
    query_interface(d, id, iface, riid, out)
}

/// IDXGIAdapter..IDXGIAdapter4 of sake's GPU (`d.adapter`) and of WARP (`d.warp`).
unsafe fn adapter_call(d: &mut D11, m: u32, this: u32, a: &dyn Fn(usize) -> u32) -> u32 {
    let warp = is_warp(d, this);
    match m {
        6 => parent(d, d.factory, a(1), a(2)),
        7 => {
            // EnumOutputs (WARP has none)
            if a(2) == 0 {
                return E_INVALIDARG;
            }
            if a(1) != 0 || warp {
                wrp(a(2), 0, 0);
                return DXGI_ERROR_NOT_FOUND;
            }
            d.add_ref(d.output);
            wrp(a(2), 0, d.addr_of(d.output));
            S_OK
        }
        8 | 10 | 11 | 18 if a(1) == 0 => E_INVALIDARG,
        8 | 10 | 11 | 18 => {
            adapter_desc(a(1), warp, adapter_luid(d, this), m);
            S_OK
        }
        9 => {
            // CheckInterfaceSupport(iid, umd version): only the D3D10 devices answer
            if a(1) == 0 || !matches!(rd(a(1), 0), 0x9b7e4c0f | 0x9b7e4c8f) || rd(a(1), 1) != 0x4106_342c {
                return DXGI_ERROR_UNSUPPORTED;
            }
            // (30.0.15.1179)
            wr(a(2), 0, 15 << 16 | 1179);
            wr(a(2), 1, 30 << 16);
            S_OK
        }
        12 | 14 | 16 => {
            // Register*Event(event, cookie)
            if a(1) == 0 || a(2) == 0 {
                return DXGI_ERROR_INVALID_CALL;
            }
            wr(a(2), 0, 1);
            S_OK
        }
        13 | 17 => 0, // Unregister*
        15 => {
            // QueryVideoMemoryInfo(node, segment group local / non-local, info)
            if a(1) != 0 || a(2) > 1 || a(3) == 0 {
                return E_INVALIDARG;
            }
            let budget: u64 = match (warp, a(2)) {
                (true, 0) | (false, 1) => 0,
                (true, _) => 1 << 30,
                (false, _) => 512 << 20,
            };
            for (i, v) in [budget, 0, budget / 2, 0].into_iter().enumerate() {
                wr(a(3), 2 * i, lo32(v));
                wr(a(3), 2 * i + 1, lo32(v >> 32));
            }
            S_OK
        }
        // SetVideoMemoryReservation(node, group, bytes)
        19 => {
            if a(1) != 0 || a(2) > 1 {
                return E_INVALIDARG;
            }
            S_OK
        }
        _ => E_NOTIMPL,
    }
}

/// DXGI_ADAPTER_DESC (m 8), DESC1 (10), DESC2 (11) or DESC3 (18) of sake's GPU or WARP:
/// Description[128] WCHAR, the ids, three SIZE_T memory sizes, the LUID; then Flags
/// (DXGI_ADAPTER_FLAG_SOFTWARE for WARP) and the two preemption granularities.
unsafe fn adapter_desc(p: u32, warp: bool, luid: u32, m: u32) {
    let at = 272 + 3 * ptr();
    let n = match m {
        8 => at + 8,
        10 => (at + 12).next_multiple_of(ptr()),
        _ => (at + 20).next_multiple_of(ptr()),
    };
    std::ptr::write_bytes(gp::<u8>(p), 0, us(n));
    let (name, vendor, device, dedicated, shared) =
        if warp { ("Microsoft Basic Render Driver", 0x1414, 0x8c, 0, 0xffff_0000) } else { ("masu WebGPU", 0x10de, 0x1c82, 512 << 20, 1024 << 20) };
    for (i, c) in name.encode_utf16().enumerate() {
        *(gp::<u16>(p + lo32(i) * 2)) = c;
    }
    wr(p + 256, 0, vendor);
    wr(p + 256, 1, device);
    wrp(p + 272, 0, dedicated); // DedicatedVideoMemory (SIZE_T)
    wrp(p + 272, 2, shared); // SharedSystemMemory
    wr(p + at, 0, luid);
    if m != 8 {
        wr(p + at, 2, if warp { 2 } else { 0 });
    }
    if m == 11 || m == 18 {
        // (DXGI_GRAPHICS_PREEMPTION_PRIMITIVE_BOUNDARY, DXGI_COMPUTE_PREEMPTION_DISPATCH_BOUNDARY)
        wr(p + at, 3, 1);
        wr(p + at, 4, 1);
    }
}

/// The adapters in EnumAdapters' order: sake's GPU, then WARP
fn adapters(d: &D11) -> [u32; 2] {
    [d.adapter, d.warp]
}

/// Whether adapter object `id` is WARP's
fn is_warp(d: &D11, id: u32) -> bool {
    matches!(d.objs.get(&id).map(|o| &o.data), Some(Data::Adapter(true)))
}

/// An adapter's LUID (AdapterLuid): its place in the list, from 0x1000
fn adapter_luid(d: &D11, id: u32) -> u32 {
    0x1000 + u32::from(is_warp(d, id))
}

/// EnumAdapters(1), EnumAdapterByGpuPreference, EnumAdapterByLuid, EnumWarpAdapter: adapter
/// `id` (0: none such) as `riid` (None: IDXGIAdapter itself)
unsafe fn give_adapter(d: &mut D11, id: u32, riid: Option<u32>, out: u32) -> u32 {
    if out == 0 {
        return if id == 0 { DXGI_ERROR_NOT_FOUND } else { E_INVALIDARG };
    }
    if id == 0 {
        wrp(out, 0, 0);
        return DXGI_ERROR_NOT_FOUND;
    }
    // (a new object each time, as Windows': winref dxgienum's "same 0")
    let warp = is_warp(d, id);
    let n = d.new_obj(IF_ADAPTER, Data::Adapter(warp));
    let n = d.id_of(n);
    match riid {
        Some(riid) => {
            let hr = query_interface(d, n, IF_ADAPTER, riid, out);
            d.release(n);
            hr
        }
        None => {
            wrp(out, 0, d.addr_of(n));
            S_OK
        }
    }
}

unsafe fn factory_call(d: &mut D11, m: u32, _this: u32, a: &dyn Fn(usize) -> u32) -> u32 {
    match m {
        3..=5 => S_OK,
        6 => E_NOINTERFACE,
        7 | 12 => {
            // EnumAdapters / EnumAdapters1(index, out)
            let id = adapters(d).get(us(a(1))).copied().unwrap_or(0);
            give_adapter(d, id, None, a(2))
        }
        8 => S_OK,
        9 => {
            wrp(a(1), 0, 0);
            S_OK
        }
        10 => {
            // CreateSwapChain(device, desc, out)
            let desc = a(2);
            if desc == 0 || a(3) == 0 {
                return E_INVALIDARG;
            }
            let p = swap_desc(desc);
            let sc = new_swapchain(d, p[11], p[0], p[1], p[4], (p[7], p[8]));
            wrp(a(3), 0, sc);
            if sc == 0 { E_FAIL } else { S_OK }
        }
        11 => DXGI_ERROR_UNSUPPORTED,
        13 => 1, // IsCurrent
        14 => 0,
        15 => {
            // CreateSwapChainForHwnd(device, hwnd, desc1, fullscreen desc, output, out)
            let desc = a(3);
            if desc == 0 || a(6) == 0 {
                return E_INVALIDARG;
            }
            let sc = new_swapchain(d, a(2), rd(desc, 0), rd(desc, 1), rd(desc, 2), (rd(desc, 4), rd(desc, 5)));
            wrp(a(6), 0, sc);
            if sc == 0 { E_FAIL } else { S_OK }
        }
        16 | 24 => DXGI_ERROR_UNSUPPORTED,
        17..=23 => E_NOTIMPL,
        25 => 0, // GetCreationFlags
        26 => {
            // EnumAdapterByLuid(luid, riid, out): the LUID two stack dwords on x86, one
            // register (its low half here) on x64
            let (high, riid, out) = if ptr() == 8 { (0, a(2), a(3)) } else { (a(2), a(3), a(4)) };
            let id = adapters(d).into_iter().find(|&x| high == 0 && adapter_luid(d, x) == a(1)).unwrap_or(0);
            if riid == 0 {
                return E_INVALIDARG;
            }
            if id == 0 {
                wrp(out, 0, 0);
                return DXGI_ERROR_NOT_FOUND; // (an unknown LUID, winref dxgienum)
            }
            give_adapter(d, id, Some(riid), out)
        }
        27 => {
            // EnumWarpAdapter(riid, out)
            if a(1) == 0 {
                return E_INVALIDARG;
            }
            give_adapter(d, d.warp, Some(a(1)), a(2))
        }
        28 => {
            // CheckFeatureSupport(feature, data, size): DXGI_FEATURE_PRESENT_ALLOW_TEARING, FALSE
            // (another size: DXGI_ERROR_INVALID_CALL, winref dxgienum)
            if a(1) != 0 || a(2) == 0 {
                return E_INVALIDARG;
            }
            if a(3) != 4 {
                return DXGI_ERROR_INVALID_CALL;
            }
            wr(a(2), 0, 0);
            S_OK
        }
        29 => {
            // EnumAdapterByGpuPreference(index, preference, riid, out): one order whatever the
            // preference (UNSPECIFIED, MINIMUM_POWER, HIGH_PERFORMANCE, and others alike as on
            // Windows: winref dxgienum's preference 3)
            if a(3) == 0 {
                return E_INVALIDARG;
            }
            let id = adapters(d).get(us(a(1))).copied().unwrap_or(0);
            give_adapter(d, id, Some(a(3)), a(4))
        }
        30 => {
            // RegisterAdaptersChangedEvent(event, cookie): adapters never change here
            if a(1) == 0 || a(2) == 0 {
                return DXGI_ERROR_INVALID_CALL;
            }
            wr(a(2), 0, 1);
            S_OK
        }
        31 => S_OK, // UnregisterAdaptersChangedNotification
        _ => E_NOTIMPL,
    }
}

const MODES: [(u32, u32); 9] = [(640, 480), (800, 600), (1024, 768), (1280, 720), (1280, 960), (1280, 1024), (1366, 768), (1600, 900), (1920, 1080)];

unsafe fn output_call(d: &mut D11, m: u32, a: &dyn Fn(usize) -> u32, after: &mut After) -> u32 {
    match m {
        6 => parent(d, d.adapter, a(1), a(2)),
        7 => {
            // GetDesc: DeviceName[32] WCHAR, DesktopCoordinates, AttachedToDesktop, Rotation, Monitor
            // (92 bytes; 96 on x64, where the HMONITOR is 8: writing 96 on x86 overwrote what
            // follows, Unity 2019's factory pointer)
            let p = a(1);
            std::ptr::write_bytes(gp::<u8>(p), 0, us(88 + ptr()));
            for (i, c) in "\\\\.\\DISPLAY1".encode_utf16().enumerate() {
                *(gp::<u16>(p + lo32(i) * 2)) = c;
            }
            let (mut w, mut h) = (0, 0);
            crate::display::vnhost_display_size(&mut w, &mut h);
            wr(p + 64, 2, w.cast_unsigned());
            wr(p + 64, 3, h.cast_unsigned());
            wr(p + 64, 4, 1);
            wr(p + 64, 5, 1);
            wr(p + 64, 6, crate::display::HMONITOR);
            S_OK
        }
        8 => {
            // GetDisplayModeList(format, flags, count, modes)
            let (count, modes) = (a(3), a(4));
            if count == 0 {
                return E_INVALIDARG;
            }
            if modes != 0 {
                let n = rd(count, 0).min(lo32(MODES.len()));
                for (i, (w, h)) in MODES.iter().take(us(n)).enumerate() {
                    let p = modes + lo32(i) * 28;
                    for (k, v) in [*w, *h, 60, 1, a(1), 0, 0].into_iter().enumerate() {
                        wr(p, k, v);
                    }
                }
                wr(count, 0, n);
            } else {
                wr(count, 0, lo32(MODES.len()));
            }
            S_OK
        }
        9 => {
            // FindClosestMatchingMode(mode, closest, device)
            let (want, out) = (a(1), a(2));
            let (w, h) = (rd(want, 0), rd(want, 1));
            let best = MODES.into_iter().min_by_key(|(x, y)| x.abs_diff(w) + y.abs_diff(h)).unwrap_or((640, 480));
            let f = if rd(want, 4) != 0 { rd(want, 4) } else { 28 };
            for (k, v) in [best.0, best.1, 60, 1, f, 0, 0].into_iter().enumerate() {
                wr(out, k, v);
            }
            S_OK
        }
        10 => {
            // WaitForVBlank: a frame's wait, after the device's lock is let go (other threads'
            // calls go on meanwhile)
            *after = After::Sleep(16);
            S_OK
        }
        11..=18 => S_OK,
        19 => {
            // GetDisplayModeList1(format, flags, count, modes): DXGI_MODE_DESC1 (Stereo after)
            let (count, modes) = (a(3), a(4));
            if count == 0 {
                return E_INVALIDARG;
            }
            if modes != 0 {
                let n = rd(count, 0).min(lo32(MODES.len()));
                for (i, (w, h)) in MODES.iter().take(us(n)).enumerate() {
                    let p = modes + lo32(i) * 32;
                    for (k, v) in [*w, *h, 60, 1, a(1), 0, 0, 0].into_iter().enumerate() {
                        wr(p, k, v);
                    }
                }
                wr(count, 0, n);
            } else {
                wr(count, 0, lo32(MODES.len()));
            }
            S_OK
        }
        20 => {
            // FindClosestMatchingMode1(mode1, closest1, device)
            let r = output_call(d, 9, a, after);
            wr(a(2), 7, 0);
            r
        }
        21 => E_NOTIMPL,                // GetDisplaySurfaceData1
        22 | 26 => E_NOTIMPL,           // DuplicateOutput(1): no desktop duplication
        23 => 0,                        // SupportsOverlays: FALSE
        24 | 25 => {
            // CheckOverlaySupport / CheckOverlayColorSpaceSupport(.., flags): none
            let out = if m == 24 { a(3) } else { a(4) };
            if out == 0 {
                return E_INVALIDARG;
            }
            wr(out, 0, 0);
            DXGI_ERROR_UNSUPPORTED
        }
        27 => {
            // GetDesc1: GetDesc's, then BitsPerColor, ColorSpace (G22 P709), the sRGB primaries
            // and white point, and luminances
            if a(1) == 0 {
                return E_INVALIDARG;
            }
            output_call(d, 7, a, after);
            let p = a(1) + 88 + ptr();
            let f = |x: f32| x.to_bits();
            for (k, v) in [8, 0, f(0.64), f(0.33), f(0.30), f(0.60), f(0.15), f(0.06), f(0.3127), f(0.3290), f(0.5), f(270.0), f(270.0)].into_iter().enumerate() {
                wr(p, k, v);
            }
            S_OK
        }
        28 => {
            // CheckHardwareCompositionSupport(flags): none
            if a(1) == 0 {
                return E_INVALIDARG;
            }
            wr(a(1), 0, 0);
            S_OK
        }
        _ => E_NOTIMPL,
    }
}

unsafe fn swapchain_call(d: &mut D11, m: u32, this: u32, a: &dyn Fn(usize) -> u32, after: &mut After) -> u32 {
    let Some(Data::SwapChain(sc)) = d.objs.get(&this).map(|o| &o.data) else { return E_FAIL };
    let (device, hwnd, w, h, fmt, buffer, samples) = (sc.device, sc.hwnd, sc.w, sc.h, sc.fmt, sc.buffer, sc.samples);
    match m {
        3..=5 => S_OK,
        6 => parent(d, d.factory, a(1), a(2)),
        7 => parent(d, device, a(1), a(2)),
        8 | 22 => {
            // Present(sync, flags) / Present1(sync, flags, params)
            if a(2) & 1 != 0 {
                return S_OK; // DXGI_PRESENT_TEST
            }
            let mut rect = [0i32; 4];
            (d.calls().client_rect)(hwnd, rect.as_mut_ptr());
            let tex = match d.objs.get(&buffer).map(|o| &o.data) {
                Some(Data::Texture(t)) => t.gpu,
                _ => 0,
            };
            gpu::record(|q| q.cmd(&[gpu::PRESENT, tex, rect[0].cast_unsigned(), rect[1].cast_unsigned(), rect[2].cast_unsigned(), rect[3].cast_unsigned(), w, h, hwnd]));
            d.targets = None;
            if let Some(Data::SwapChain(sc)) = d.objs.get_mut(&this).map(|o| &mut o.data) {
                sc.presents += 1;
                // (always, the 1st, 60th and 600th: where the frames go, in every run's log)
                if matches!(sc.presents, 1 | 60 | 600) {
                    eprintln!("sake: d3d11 present {} of {w}x{h} to window {hwnd:#x} client {:?}", sc.presents, rect);
                }
            }
            *after = After::Present(hwnd);
            S_OK
        }
        9 => {
            // GetBuffer(index, riid, out)
            let iface = IF_TEX2D;
            query_interface(d, buffer, iface, a(2), a(3))
        }
        10 => S_OK,
        11 => {
            wr(a(1), 0, 0);
            wrp(a(2), 0, 0);
            S_OK
        }
        12 => {
            put_swap_desc(a(1), [w, h, 60, 1, fmt, 0, 0, samples.0, samples.1, 0x20, 1, hwnd, 1, 0, 0]);
            S_OK
        }
        13 => {
            // ResizeBuffers(count, w, h, format, flags)
            let mut rect = [0i32; 4];
            (d.calls().client_rect)(hwnd, rect.as_mut_ptr());
            let nw = if a(2) == 0 { (rect[2] - rect[0]).max(1).cast_unsigned() } else { a(2) };
            let nh = if a(3) == 0 { (rect[3] - rect[1]).max(1).cast_unsigned() } else { a(3) };
            let nf = if a(4) == 0 { fmt } else { a(4) };
            let nb = new_backbuffer(d, nw, nh, nf, samples);
            let nb = d.id_of(nb);
            if let Some(Data::SwapChain(sc)) = d.objs.get_mut(&this).map(|o| &mut o.data) {
                sc.w = nw;
                sc.h = nh;
                sc.fmt = nf;
                sc.buffer = nb;
            }
            d.release(buffer);
            S_OK
        }
        14 => S_OK,
        15 => {
            d.add_ref(d.output);
            wrp(a(1), 0, d.addr_of(d.output));
            S_OK
        }
        16 => DXGI_ERROR_UNSUPPORTED,
        17 => {
            let n = match d.objs.get(&this).map(|o| &o.data) {
                Some(Data::SwapChain(sc)) => sc.presents,
                _ => 0,
            };
            wr(a(1), 0, n);
            S_OK
        }
        18 => {
            let p = a(1);
            for (k, v) in [w, h, fmt, 0, samples.0, samples.1, 0x20, 1, 0, 0, 0, 0].into_iter().enumerate() {
                wr(p, k, v);
            }
            S_OK
        }
        19 => {
            for (k, v) in [60, 1, 0, 0, 1].into_iter().enumerate() {
                wr(a(1), k, v);
            }
            S_OK
        }
        20 => {
            wrp(a(1), 0, hwnd);
            S_OK
        }
        23 => 0,
        _ => DXGI_ERROR_INVALID_CALL,
    }
}

// ---------------------------------------------------------------------------------------------
// ID3D11Device

unsafe fn device_call(d: &mut D11, m: u32, this: u32, a: &dyn Fn(usize) -> u32) -> u32 {
    let _ = this;
    match m {
        3 => {
            // CreateBuffer(desc, init, out)
            let desc = a(1);
            if desc == 0 {
                return E_INVALIDARG;
            }
            // D3D11_BUFFER_DESC { ByteWidth, Usage, BindFlags, CPUAccessFlags, MiscFlags, StructureByteStride }
            let (size, usage, bind, cpu, misc) = (rd(desc, 0), rd(desc, 1), rd(desc, 2), rd(desc, 3), rd(desc, 4));
            let stride = match check::buffer(size, usage, bind, cpu, misc, rd(desc, 5), a(2) != 0) {
                Ok(s) => s,
                Err(hr) => return hr,
            };
            if a(2) != 0 && rdp(a(2), 0) == 0 {
                return E_INVALIDARG;
            }
            if a(3) == 0 {
                return S_FALSE;
            }
            let mem = d.guest_alloc(size.next_multiple_of(16));
            if mem == 0 {
                return E_OUTOFMEMORY;
            }
            std::ptr::write_bytes(gp::<u8>(mem), 0, us(size));
            if a(2) != 0 {
                std::ptr::copy_nonoverlapping(gc::<u8>(rdp(a(2), 0)), gp::<u8>(mem), us(size));
            }
            let o = d.new_obj(IF_BUFFER, Data::Buffer(Buffer { size, usage, bind, cpu, misc, stride, mem, gpu: 0, dirty: true, stream: false, shadow: Vec::new(), gpu_written: false }));
            if d.trace && size <= 256 {
                let f: Vec<f32> = (0..size / 4).map(|i| *(gc::<f32>(mem + 4 * i))).collect();
                eprintln!("d3d11: new buffer {o} bind {bind:#x} usage {usage} = {f:?}");
            }
            wrp(a(3), 0, o);
            S_OK
        }
        4..=6 => {
            // CreateTexture1D/2D/3D(desc, init, out)
            let desc = a(1);
            if desc == 0 {
                return E_INVALIDARG;
            }
            // D3D11_TEXTURE1D/2D/3D_DESC
            let r = |i| rd(desc, i);
            let t = match m {
                4 => check::Tex { dim: 1, w: r(0), h: 1, layers: r(2), mips: r(1), fmt: r(3), samples: 1, quality: 0, usage: r(4), bind: r(5), cpu: r(6), misc: r(7) },
                5 => check::Tex { dim: 2, w: r(0), h: r(1), layers: r(3), mips: r(2), fmt: r(4), samples: r(5), quality: r(6), usage: r(7), bind: r(8), cpu: r(9), misc: r(10) },
                _ => check::Tex { dim: 3, w: r(0), h: r(1), layers: r(2), mips: r(3), fmt: r(4), samples: 1, quality: 0, usage: r(5), bind: r(6), cpu: r(7), misc: r(8) },
            };
            let mips = match check::texture(&t, a(2) != 0) {
                Ok(n) => n,
                Err(hr) => {
                    if d.trace {
                        eprintln!("d3d11: texture {t:?} refused");
                    }
                    return hr;
                }
            };
            let check::Tex { dim, w, h, layers, fmt, usage, bind, misc, .. } = t;
            // (sizes its memory cannot hold: before anything multiplies them in u32)
            let (d3, n, s) = if dim == 3 { (layers, 1, 1) } else { (1, layers, t.samples) };
            let bytes = format::texture_bytes(fmt, w, h, d3, n, mips, s).max(format::page_texture_bytes(fmt, w, h, d3, n, mips, s));
            if bytes > format::MAX_TEXTURE_BYTES {
                if d.trace {
                    eprintln!("d3d11: texture {t:?} of {bytes} bytes refused");
                }
                return E_OUTOFMEMORY;
            }
            // (initial data with a missing level)
            if a(2) != 0 {
                let n = mips * if dim == 3 { 1 } else { layers };
                if (0..n).any(|sub| rdp(a(2) + sub * (ptr() + 8), 0) == 0) {
                    return E_INVALIDARG;
                }
            }
            if a(3) == 0 {
                return S_FALSE;
            }
            let o = new_texture(d, dim, w, h, layers, mips, fmt, usage, bind, misc & 4 != 0, t.samples);
            let id = d.id_of(o);
            if fmt == check::NV12 {
                nv12_make(d, id);
            }
            if let Some(Data::Texture(x)) = d.objs.get_mut(&id).map(|o| &mut o.data) {
                (x.cpu, x.misc, x.samples) = (t.cpu, misc, (t.samples, t.quality));
                // (ANGLE makes a staging texture for each upload and maps it once to write)
                x.pristine = usage == 3 && a(2) == 0;
            }
            if a(2) != 0 {
                let Some(Data::Texture(t)) = d.objs.get_mut(&id).map(|o| &mut o.data) else { return E_FAIL };
                for sub in 0..t.mips * if dim == 3 { 1 } else { t.layers } {
                    // D3D11_SUBRESOURCE_DATA { pSysMem, SysMemPitch, SysMemSlicePitch }
                    let init = a(2) + sub * (ptr() + 8);
                    upload_texture(t, sub, us(rdp(init, 0)), rd(init + ptr(), 0), rd(init + ptr(), 1), None);
                }
            }
            wrp(a(3), 0, o);
            S_OK
        }
        7..=10 => {
            let hr = create_view(d, m, a(1), a(2), a(3));
            // (a view refused: the out pointer cleared, as Windows does)
            if hr & 0x8000_0000 != 0 && a(3) != 0 {
                wrp(a(3), 0, 0);
            }
            hr
        }
        11 => {
            // CreateInputLayout(elems, n, code, len, out)
            let mut v = Vec::new();
            let mut raw = Vec::new();
            let mut next = [0u32; 16];
            for i in 0..a(2) {
                // D3D11_INPUT_ELEMENT_DESC { SemanticName, then six UINTs }
                let e = a(1) + i * (ptr() + 24);
                let name = std::ffi::CStr::from_ptr(rdp(e, 0) as *const std::ffi::c_char).to_string_lossy().into_owned();
                let f = e + ptr();
                raw.push((rd(f, 0), rd(f, 1), rd(f, 2), rd(f, 4), rd(f, 5)));
                let (index, fmt, slot, off, class) = (rd(f, 0), rd(f, 1), rd(f, 2) & 15, rd(f, 3), rd(f, 4));
                let off = if off == 0xffff_ffff { next[us(slot)] } else { off };
                let size = format::vertex_format(fmt).map_or(4, vertex_size);
                next[us(slot)] = off + size;
                // SAFETY: rd's own contract: the caller's D3D11_INPUT_ELEMENT_DESC array of a(2) elements
                let step = unsafe { rd(f, 5) };
                v.push(Elem { name, index, fmt, slot, offset: off, instance: class == 1, step });
            }
            let code = if a(3) == 0 { &[][..] } else { std::slice::from_raw_parts(gc::<u8>(a(3)), us(a(4))) };
            let names: Vec<(&str, u32)> = v.iter().map(|e| (e.name.as_str(), e.index)).collect();
            let slots: Vec<(u32, bool)> = v.iter().map(|e| (e.slot, e.instance)).collect();
            let elems: Vec<_> = v.iter().zip(&raw).map(|(e, r)| (e.name.as_str(), r.0, r.1, r.2, r.3, r.4)).collect();
            if let Err(hr) = check::input_elements(&elems).and_then(|()| check::input_layout(a(1) != 0, &names, &slots, code)) {
                wrp(a(5), 0, 0);
                return hr;
            }
            if a(5) == 0 {
                return S_FALSE;
            }
            if d.trace {
                for e in &v {
                    eprintln!("d3d11:   layout {}{} format {} slot {} offset {}{}", e.name, e.index, e.fmt, e.slot, e.offset, if e.instance { " instance" } else { "" });
                }
            }
            let o = d.new_obj(IF_LAYOUT, Data::Layout(v));
            wrp(a(5), 0, o);
            S_OK
        }
        12 | 13 | 15..=18 => {
            // Create*Shader(code, len, linkage, out)
            let code = std::slice::from_raw_parts(gc::<u8>(a(1)), us(a(2))).to_vec();
            let iface = match m {
                12 => IF_VS,
                13 => IF_GS,
                15 => IF_PS,
                16 => IF_HS,
                17 => IF_DS,
                _ => IF_CS,
            };
            if let Err(hr) = check::shader(&code, m, d.level) {
                if a(4) != 0 {
                    wrp(a(4), 0, 0);
                }
                return hr;
            }
            if a(4) == 0 {
                return S_FALSE;
            }
            let o = d.new_obj(iface, Data::Shader(Shader { code, pixel: iface == IF_PS, wgsl: HashMap::new(), so: None }));
            wrp(a(4), 0, o);
            S_OK
        }
        14 => {
            // CreateGeometryShaderWithStreamOutput(code, len, decls, n, strides, nstrides, stream,
            // linkage, out): a geometry or vertex shader's bytecode and what it streams out
            let code = if a(1) == 0 {
                Vec::new()
            } else {
                // SAFETY: byte i of the caller's bytecode of a(2) bytes at a(1) (gc maps the
                // guest address)
                (0..a(2)).map(|i| unsafe { *gc::<u8>(a(1).wrapping_add(i)) }).collect()
            };
            let gs = check::shader(&code, 13, d.level).is_ok();
            let fail = |hr: u32| {
                if a(9) != 0 {
                    wrp(a(9), 0, 0);
                }
                hr
            };
            if a(1) != 0 && !gs && check::shader(&code, 12, d.level).is_err() {
                return fail(E_INVALIDARG);
            }
            // D3D11_SO_DECLARATION_ENTRY { Stream, SemanticName, SemanticIndex, Start, Count, Slot }
            let size = ptr().wrapping_mul(2).wrapping_add(8);
            let entries: Vec<expand::RawEntry> = (0..a(4).min(513))
                .map(|i| {
                    let e = a(3).wrapping_add(i.wrapping_mul(size));
                    // SAFETY: rd's own contract: entry i of the caller's array of a(4)
                    // D3D11_SO_DECLARATION_ENTRY (its Stream)
                    let stream = unsafe { rd(e, 0) };
                    // SAFETY: rdp's own contract: the same entry's SemanticName
                    let np = unsafe { rdp(e.wrapping_add(ptr()), 0) };
                    // SAFETY: rd's own contract: the same entry's SemanticIndex
                    let index = unsafe { rd(e.wrapping_add(ptr().wrapping_mul(2)), 0) };
                    // SAFETY: the entry's SemanticName, a NUL-terminated string of the caller's
                    let name = (np != 0).then(|| unsafe { std::ffi::CStr::from_ptr(gc::<std::ffi::c_char>(np)) }.to_string_lossy().into_owned());
                    let b = e.wrapping_add(ptr().wrapping_mul(2)).wrapping_add(4);
                    // SAFETY: byte k of the same entry's StartComponent, ComponentCount, OutputSlot
                    let byte = |k: u32| u32::from(unsafe { *gc::<u8>(b.wrapping_add(k)) });
                    let (start, count, slot) = (byte(0), byte(1), byte(2));
                    expand::RawEntry { stream, name, index, start, count, slot }
                })
                .collect();
            // SAFETY: rd's own contract: the caller's array of a(6) buffer strides
            let strides: Vec<u32> = (0..a(6).min(5)).map(|i| unsafe { rd(a(5), us(i)) }).collect();
            let osgn = dxwgsl::signature(&code, true);
            let so = match expand::so_desc(&osgn, gs, &entries, &strides, a(7)) {
                Ok(so) => so,
                Err(hr) => return fail(hr),
            };
            if a(9) == 0 {
                return S_FALSE;
            }
            let o = d.new_obj(IF_GS, Data::Shader(Shader { code, pixel: false, wgsl: HashMap::new(), so: Some(so) }));
            wrp(a(9), 0, o);
            S_OK
        }
        19 => {
            let o = d.new_obj(IF_LINKAGE, Data::Other);
            wrp(a(1), 0, o);
            S_OK
        }
        20..=23 => {
            // Create{Blend, DepthStencil, Rasterizer, Sampler}State(desc, out)
            let (iface, n) = match m {
                20 => (IF_BLEND, 66),
                21 => (IF_DEPTH, 13),
                22 => (IF_RASTER, 10),
                _ => (IF_SAMPLER, 13),
            };
            if a(1) == 0 {
                return E_INVALIDARG;
            }
            let mut desc: Vec<u32> = (0..n).map(|i| rd(a(1), i)).collect();
            if normalize_state(m, &mut desc).is_none() {
                return E_INVALIDARG;
            }
            // (the newer descriptions' fields at the end: a blend state's LogicOpEnable and
            // LogicOp per target, a rasterizer state's ForcedSampleCount and ConservativeRaster)
            desc.resize(state_len(iface), 0);
            new_state(d, iface, desc, a(2))
        }
        24 | 25 => {
            // CreateQuery/CreatePredicate(desc { Query, MiscFlags }, out): the predicates are
            // OCCLUSION_PREDICATE and SO_OVERFLOW_PREDICATE, PREDICATEHINT only for them
            if a(1) == 0 {
                return E_INVALIDARG;
            }
            let (t, misc) = (rd(a(1), 0), rd(a(1), 1));
            let predicate = matches!(t, 5 | 7);
            if t > 15 || misc & !1 != 0 || misc != 0 && !predicate || m == 25 && !predicate {
                return E_INVALIDARG;
            }
            if a(2) == 0 {
                return S_FALSE;
            }
            let Some(kind) = query::Kind::of_type(t) else { return E_INVALIDARG };
            let o = d.new_obj(IF_QUERY, Data::Query(query::Query::new(kind)));
            wrp(a(2), 0, o);
            S_OK
        }
        26 => E_INVALIDARG,
        27 => deferred::create(d, a(1), a(2)),
        28 | 48 => {
            // OpenSharedResource(handle, riid, out), OpenSharedResource1: the texture itself
            // (one process, one device)
            if a(3) == 0 {
                return E_INVALIDARG;
            }
            wrp(a(3), 0, 0);
            let id = a(1).wrapping_sub(SHARED_BASE);
            match d.objs.get(&id) {
                Some(o) if a(1) >= SHARED_BASE && matches!(&o.data, Data::Texture(t) if t.misc & 0x902 != 0) => {
                    let iface = o.iface;
                    query_interface(d, id, iface, a(2), a(3))
                }
                _ => E_INVALIDARG,
            }
        }
        29 => {
            // CheckFormatSupport(format, out)
            // (Windows' answers: what the page does with a format is its own business)
            if a(2) == 0 {
                return E_INVALIDARG;
            }
            let bits = check::support(a(1));
            wr(a(2), 0, bits.unwrap_or(0));
            if bits.is_some() { S_OK } else { E_FAIL }
        }
        30 => {
            // CheckMultisampleQualityLevels(format, count, out)
            if a(3) == 0 {
                return E_INVALIDARG;
            }
            wr(a(3), 0, check::quality_levels(a(1), a(2)));
            S_OK
        }
        31 => {
            std::ptr::write_bytes(gp::<u8>(a(1)), 0, 12);
            S_OK
        }
        32 => E_INVALIDARG,
        33 => {
            // CheckFeatureSupport(feature, data, size): nothing optional, but for what WARP
            // answers too: concurrent creates (THREADING) and compute on 10.x (the device is
            // free-threaded here and compute runs at every level)
            // FORMAT_SUPPORT(2): { InFormat, OutFormatSupport(2) }, Windows' answers
            if a(1) == 2 || a(1) == 3 {
                if a(2) == 0 || a(3) != 8 {
                    return E_INVALIDARG;
                }
                let f = rd(a(2), 0);
                let bits = if a(1) == 2 { check::support(f) } else { check::support2(f) };
                wr(a(2), 1, bits.unwrap_or(0));
                return if bits.is_some() { S_OK } else { E_FAIL };
            }
            std::ptr::write_bytes(gp::<u8>(a(2)), 0, us(a(3)));
            if (a(1) == 0 || a(1) == 4) && a(3) >= 4 {
                wr(a(2), 0, 1);
            }
            // D3D11_OPTIONS: ConstantBufferPartialUpdate (6) and ConstantBufferOffsetting (7),
            // which the context does (set_cbs, cb_binding), as WARP answers (winref d3d11cb1)
            if a(1) == 5 && a(3) >= 32 {
                // SAFETY: wr's own contract: the caller's D3D11_FEATURE_DATA_D3D11_OPTIONS of
                // a(3) >= 32 bytes
                unsafe { wr(a(2), 6, 1) };
                // SAFETY: (the same structure's next field)
                unsafe { wr(a(2), 7, 1) };
            }
            S_OK
        }
        34..=36 => private_data(d, m - 30, this, a),
        37 => d.level,
        38 => 0,
        39 => S_OK,
        40 => {
            d.add_ref(d.context);
            wrp(a(1), 0, d.addr_of(d.context));
            S_OK
        }
        41 => S_OK,
        42 => 0,
        // ID3D11Device1/2 (Direct2D uses these)
        43 | 50 | 61 => device_call(d, 40, this, a), // GetImmediateContext1/2/3
        44 | 51 | 62 => deferred::create(d, a(1), a(2)), // CreateDeferredContext1/2/3
        45 => {
            // CreateBlendState1(desc, out): render targets of 10 dwords (LogicOpEnable, LogicOp)
            // to the 8 of D3D11_BLEND_DESC
            if a(1) == 0 {
                return E_INVALIDARG;
            }
            let mut desc = vec![rd(a(1), 0), rd(a(1), 1)];
            for t in 0..8 {
                let b = 2 + t * 10;
                desc.push(rd(a(1), b));
                desc.extend((3..9).map(|i| rd(a(1), b + i - 1)));
                desc.push(rd(a(1), b + 9));
            }
            if normalize_state(20, &mut desc).is_none() {
                return E_INVALIDARG;
            }
            // (winref d3d11copy: a logic op with IndependentBlendEnable is refused)
            let logic = |t: usize| rd(a(1), 2 + t * 10 + 1) != 0;
            if desc[1] != 0 && (0..8).any(logic) {
                return E_INVALIDARG;
            }
            for t in 0..8 {
                let src = if desc[1] != 0 { t } else { 0 };
                let on = rd(a(1), 2 + src * 10 + 1);
                desc.extend([on, if on != 0 { rd(a(1), 2 + src * 10 + 8) } else { 0 }]);
            }
            new_state(d, IF_BLEND, desc, a(2))
        }
        46 | 56 => {
            // CreateRasterizerState1/2: D3D11_RASTERIZER_DESC with ForcedSampleCount (and
            // ConservativeRaster) at its end
            if a(1) == 0 {
                return E_INVALIDARG;
            }
            let mut desc: Vec<u32> = (0..if m == 46 { 11 } else { 12 }).map(|i| rd(a(1), i)).collect();
            // ForcedSampleCount 0, 1, 4 or 8, and not 1 with MultisampleEnable (winref
            // d3d11copy on Windows: 3, 16 and 32 are refused).
            // ConservativeRaster ON: none on the page (WebGPU has none; WARP refuses it too)
            if !matches!(desc[10], 0 | 1 | 4 | 8) || (desc[10] == 1 && desc[8] != 0) || desc.get(11).is_some_and(|&c| c != 0) {
                return E_INVALIDARG;
            }
            if normalize_state(22, &mut desc).is_none() {
                return E_INVALIDARG;
            }
            desc.resize(state_len(IF_RASTER), 0);
            new_state(d, IF_RASTER, desc, a(2))
        }
        47 => {
            // CreateDeviceContextState(flags, levels, count, sdk, iid, level_out, state_out)
            let level = if a(2) != 0 { (0..us(a(3))).map(|i| rd(a(2), i)).find(|&l| l <= 0xb000) } else { None };
            let Some(level) = level else {
                wr(a(6), 0, 0);
                wrp(a(7), 0, 0);
                return E_INVALIDARG;
            };
            wr(a(6), 0, level);
            // (from now on the device answers for its D3D10 interfaces too, as Windows does)
            d.ten_faces = true;
            if a(7) == 0 {
                return S_FALSE;
            }
            let o = d.new_obj(IF_LINKAGE, Data::CtxState(Some(Box::new(default_state()))));
            wrp(a(7), 0, o);
            S_OK
        }
        49 => E_NOTIMPL,
        52 => 0,
        // ID3D11Device3: the *1 / *2 descriptions extend the old ones at their ends (texture
        // layout, conservative rasterization, plane slices of planar formats, a query's
        // context type), which this device does not have: the old creation reads its prefix
        54 => device_call(d, 5, this, a),  // CreateTexture2D1
        55 => device_call(d, 6, this, a),  // CreateTexture3D1
        57 => device_call(d, 7, this, a),  // CreateShaderResourceView1
        58 => device_call(d, 8, this, a),  // CreateUnorderedAccessView1
        59 => device_call(d, 9, this, a),  // CreateRenderTargetView1
        60 => device_call(d, 24, this, a), // CreateQuery1
        // Write / ReadFromSubresource: only for textures mapped with a CPU-visible layout,
        // which none here is
        63 | 64 => 0,
        // ID3D11Device4: the device is never removed (cookie 0 is valid)
        65 => {
            if a(1) == 0 || a(2) == 0 {
                return E_INVALIDARG;
            }
            wr(a(2), 0, 0);
            S_OK
        }
        66 => 0,
        // ID3D11Device5: fences
        67 | 68 => E_NOTIMPL,
        53 => {
            // CheckMultisampleQualityLevels1(format, count, flags, out)
            if a(4) == 0 {
                return E_INVALIDARG;
            }
            wr(a(4), 0, check::quality_levels(a(1), a(2)));
            S_OK
        }
        _ => E_NOTIMPL,
    }
}

fn vertex_size(f: u32) -> u32 {
    match f {
        0 | 4 | 8 => 4,
        1 | 5 | 9 => 8,
        2 | 6 | 10 => 12,
        3 | 7 | 11 => 16,
        17 | 19 | 21 | 23 | 25 => 8,
        26 | 27 | 30 | 31 => 2,
        _ => 4,
    }
}

/// Get/Set/SetInterface PrivateData as methods 4/5/6 of a device child (`m` in that numbering).
unsafe fn private_data(d: &mut D11, m: u32, this: u32, a: &dyn Fn(usize) -> u32) -> u32 {
    // (as Windows: Get/Set/SetInterface on a store per object; an interface stored holds a
    // reference to the object, given out again with one more by Get)
    let mut guid = [0u8; 16];
    if a(1) == 0 {
        return E_INVALIDARG;
    }
    std::ptr::copy_nonoverlapping(gc::<u8>(a(1)), guid.as_mut_ptr(), 16);
    let iface_obj = if m == 6 { d.id_of(a(2)) } else { 0 };
    let Some(o) = d.objs.get_mut(&this) else { return E_FAIL };
    let found = o.private.iter().position(|x| x.0 == guid);
    match m {
        4 => {
            // GetPrivateData(guid, size*, data)
            if a(2) == 0 {
                return E_INVALIDARG;
            }
            let Some(i) = found else {
                wr(a(2), 0, 0);
                return DXGI_ERROR_NOT_FOUND;
            };
            let (v, held) = (o.private[i].1.clone(), o.private[i].2);
            let room = rd(a(2), 0);
            wr(a(2), 0, lo32(v.len()));
            if a(3) == 0 {
                return S_OK;
            }
            if room < lo32(v.len()) {
                return 0x887a_0003; // DXGI_ERROR_MORE_DATA
            }
            if held != 0 {
                d.add_ref(held);
            }
            std::ptr::copy_nonoverlapping(v.as_ptr(), gp::<u8>(a(3)), v.len());
            S_OK
        }
        _ => {
            // SetPrivateData(guid, size, data) / SetPrivateDataInterface(guid, object)
            let (bytes, held) = if m == 5 {
                if a(3) == 0 {
                    // (no data: the entry goes, if there is one)
                    let Some(i) = found else { return S_FALSE };
                    let old = o.private.remove(i).2;
                    if old != 0 {
                        d.release(old);
                    }
                    return S_OK;
                }
                (std::slice::from_raw_parts(gc::<u8>(a(3)), us(a(2))).to_vec(), 0)
            } else {
                let mut b = a(2).to_le_bytes().to_vec();
                b.resize(us(ptr()), 0);
                (b, iface_obj)
            };
            let old = found.map_or(0, |i| o.private.remove(i).2);
            o.private.push((guid, bytes, held));
            if held != 0 {
                d.add_ref(held);
            }
            if old != 0 {
                d.release(old);
            }
            S_OK
        }
    }
}

/// Create{ShaderResource, UnorderedAccess, RenderTarget, DepthStencil}View(resource, desc, out)
unsafe fn create_view(d: &mut D11, m: u32, res: u32, desc: u32, out: u32) -> u32 {
    let rid = d.id_of(res);
    let kind = [check::SRV, check::UAV, check::RTV, check::DSV][us(m) - 7];
    // (the description's words: SRV 6, RTV and UAV 5, DSV 6)
    let words: Option<Vec<u32>> = (desc != 0).then(|| (0..if kind == check::RTV || kind == check::UAV { 5 } else { 6 }).map(|i| rd(desc, i)).collect());
    if let Some(Data::Buffer(b)) = d.objs.get(&rid).map(|o| &o.data) {
        let bv = match check::buffer_view(kind, b.size, b.bind, b.misc, b.stride, words.as_deref()) {
            Ok(v) => v,
            Err(hr) => return hr,
        };
        if out == 0 {
            return S_FALSE;
        }
        if kind == check::SRV {
            return create_buffer_srv(d, rid, bv, out);
        }
        // (buffer UAVs and render targets: kept, not drawn with)
        let o = d.new_obj([IF_SRV, IF_UAV, IF_RTV, IF_DSV][us(m) - 7], Data::Other);
        wrp(out, 0, o);
        return S_OK;
    }
    let Some(Data::Texture(t)) = d.objs.get(&rid).map(|o| &o.data) else { return E_INVALIDARG };
    let r = check::Res { dim: t.dim, fmt: t.fmt, mips: t.mips, layers: t.layers, bind: t.bind, cube: t.cube, samples: t.samples.0, backbuffer: t.backbuffer };
    let c = match check::view(kind, &r, words.as_deref()) {
        Ok(c) => c,
        Err(hr) => {
            if d.trace {
                eprintln!("d3d11: view {kind} of {r:?} as {words:?} refused");
            }
            return hr;
        }
    };
    let v = View { res: rid, fmt: c.desc[0], desc: c.desc, mip: c.mip, mips: c.mips, layer: c.layer, layers: c.layers, buf: None };
    if out == 0 {
        return S_FALSE;
    }
    if d.trace {
        eprintln!("d3d11: view {kind} of texture {rid} ({}x{} format {} bind {:#x}) as {}", t.w, t.h, t.fmt, t.bind, v.fmt);
    }
    // NV12's chroma plane (an R8G8 view of any kind): a view of its texture (see `Nv12`)
    let chroma = match &t.nv12 {
        Some(n) if v.fmt == 49 => Some(n.chroma),
        _ => None,
    };
    if let Some(cid) = chroma {
        d.bind(cid);
        d.bind(rid);
        let buf = Some(BufSrc { buf: rid, off: 0, bytes: 0, last: Vec::new() });
        let o = d.new_obj([IF_SRV, IF_UAV, IF_RTV, IF_DSV][us(m) - 7], Data::View(View { res: cid, buf, ..v }));
        wrp(out, 0, o);
        return S_OK;
    }
    d.bind(rid);
    let o = d.new_obj([IF_SRV, IF_UAV, IF_RTV, IF_DSV][us(m) - 7], Data::View(v));
    wrp(out, 0, o);
    S_OK
}

/// CreateShaderResourceView of a buffer (checked: `bv`): a view of a one-row texture with the
/// elements (at most 8192, WebGPU's width limit; raw and structured views as R32_UINT elements)
unsafe fn create_buffer_srv(d: &mut D11, rid: u32, bv: check::BufView, out: u32) -> u32 {
    let fmt = if matches!(bv.fmt, 0 | 39) { 42 } else { bv.fmt };
    // (the 96-bit formats have no texels on the page: a view of nothing, as Windows takes it)
    let f = format::info(fmt).filter(|f| f.block == 1).unwrap_or(format::Format { page: format::P_R32UI, bpp: 4, block: 1, conv: format::Conv::Copy, no_alpha: false });
    let n = (bv.bytes / f.bpp.max(1)).clamp(1, 8192);
    let tex = new_texture(d, 2, n, 1, 1, 1, fmt, 0, 8, false, 1);
    let tid = d.id_of(tex);
    // (the view's own texture and the buffer: held privately, see `destroy`)
    d.bind(tid);
    d.release(tid);
    d.bind(rid);
    let buf = Some(BufSrc { buf: rid, off: bv.offset, bytes: n * f.bpp, last: Vec::new() });
    let v = View { res: tid, fmt: bv.fmt, desc: bv.desc, mip: 0, mips: 1, layer: 0, layers: 1, buf };
    let o = d.new_obj(IF_SRV, Data::View(v));
    wrp(out, 0, o);
    S_OK
}

/// An NV12 texture's chroma view (see create_view): the planes brought up to date.
fn nv12_view_sync(d: &mut D11, view: u32) {
    let Some(Data::View(v)) = d.objs.get(&view).map(|o| &o.data) else { return };
    let Some(b) = &v.buf else { return };
    let id = b.buf;
    unsafe { nv12_pull(d, id) };
    nv12_push(d, id);
}

/// NV12 texture `id`'s chroma texture (see `Nv12`), made with it.
unsafe fn nv12_make(d: &mut D11, id: u32) {
    let Some(Data::Texture(t)) = d.objs.get(&id).map(|o| &o.data) else { return };
    let (w, h, layers, bind) = (t.w, t.h, t.layers, t.bind & (0x08 | 0x20 | 0x80));
    let tex = new_texture(d, 2, w / 2, h / 2, layers, 1, 49, 0, bind | 0x08, false, 1);
    let cid = d.id_of(tex);
    d.bind(cid);
    d.release(cid);
    let gpu = match d.objs.get(&cid).map(|o| &o.data) {
        Some(Data::Texture(c)) => c.gpu,
        _ => 0,
    };
    if let Some(Data::Texture(t)) = d.objs.get_mut(&id).map(|o| &mut o.data) {
        t.nv12 = Some(Nv12 { chroma: cid, gpu, last: Vec::new(), drawn: false });
    }
}

/// The chroma bytes of every layer of an NV12 texture's bytes (rows of its pitch).
fn nv12_chroma_bytes(t: &Texture) -> Vec<u8> {
    let (pitch, rows) = format::pitch_rows(t.fmt, t.w, t.h);
    let luma = us(pitch * t.h);
    let chroma = us(pitch * (rows - t.h));
    let mut now = vec![0u8; chroma * us(t.layers)];
    for l in 0..t.layers {
        if let Some(bits) = t.bits.get(&l) {
            let n = chroma.min(bits.len().saturating_sub(luma));
            now[us(l) * chroma..us(l) * chroma + n].copy_from_slice(&bits[luma..luma + n]);
        }
    }
    now
}

/// The chroma texture of NV12 texture `id` brought up to date with its bytes.
fn nv12_push(d: &mut D11, id: u32) {
    let Some(Data::Texture(t)) = d.objs.get(&id).map(|o| &o.data) else { return };
    let Some(n) = &t.nv12 else { return };
    let now = nv12_chroma_bytes(t);
    if now == n.last {
        return;
    }
    let (pitch, layers, cid) = (format::pitch_rows(t.fmt, t.w, t.h).0, t.layers, n.chroma);
    let chroma = now.len() / us(layers.max(1));
    if let Some(Data::Texture(x)) = d.objs.get_mut(&cid).map(|o| &mut o.data) {
        for l in 0..layers {
            unsafe { upload_texture(x, l, now.as_ptr().expose_provenance() + us(l) * chroma, pitch, 0, None) };
        }
    }
    if let Some(Data::Texture(t)) = d.objs.get_mut(&id).map(|o| &mut o.data) && let Some(n) = t.nv12.as_mut() {
        n.last = now;
    }
}

/// After a draw or clear into a plane of NV12 texture `id`: both planes read back into its
/// bytes (the page's are what the guest sees).
unsafe fn nv12_pull(d: &mut D11, id: u32) {
    let Some(Data::Texture(t)) = d.objs.get(&id).map(|o| &o.data) else { return };
    let Some(n) = t.nv12.as_ref().filter(|n| n.drawn) else { return };
    let (w, h, layers, luma_gpu, chroma_gpu) = (t.w, t.h, t.layers, t.gpu, n.gpu);
    let (pitch, rows) = format::pitch_rows(t.fmt, w, h);
    let size = pitch * rows;
    let tmp = lo32((((*d.calls).alloc)(size.max(16))).expose_provenance());
    if tmp == 0 {
        return;
    }
    let mut got = Vec::new();
    for l in 0..layers {
        std::ptr::write_bytes(gp::<u8>(tmp), 0, us(size));
        let mut reqs = Vec::new();
        for (g, gw, gh, at) in [(luma_gpu, w, h, tmp), (chroma_gpu, w / 2, h / 2, tmp + pitch * h)] {
            d.next_gpu += 1;
            let req = 0x4000_0000 | d.next_gpu;
            let c = [gpu::READBACK, req, g, l << 16, 0, 0, gw, gh, at, pitch];
            gpu::record(|q| q.cmd(&c));
            reqs.push(req);
        }
        for req in reqs {
            gpu::wait_readback(req);
        }
        got.push(std::slice::from_raw_parts(gc::<u8>(tmp), us(size)).to_vec());
    }
    ((*d.calls).free)(gp::<u8>(tmp));
    if let Some(Data::Texture(t)) = d.objs.get_mut(&id).map(|o| &mut o.data) {
        for (l, b) in got.into_iter().enumerate() {
            t.bits.insert(lo32(l), b);
        }
        let now = nv12_chroma_bytes(t);
        if let Some(n) = t.nv12.as_mut() {
            n.last = now;
            n.drawn = false;
        }
    }
}

/// A view a draw, dispatch or clear writes: if of an NV12 texture's plane, the planes brought
/// up to date with its bytes first, then marked drawn (`nv12_pull`).
fn nv12_target(d: &mut D11, view: u32) {
    let Some(Data::View(v)) = d.objs.get(&view).map(|o| &o.data) else { return };
    let id = match (&v.buf, d.objs.get(&v.res).map(|o| &o.data)) {
        (Some(b), _) => b.buf,
        (None, Some(Data::Texture(t))) if t.nv12.is_some() => v.res,
        _ => return,
    };
    if !matches!(d.objs.get(&id).map(|o| &o.data), Some(Data::Texture(t)) if t.nv12.is_some()) {
        return;
    }
    nv12_push(d, id);
    if let Some(Data::Texture(t)) = d.objs.get_mut(&id).map(|o| &mut o.data) && let Some(n) = t.nv12.as_mut() {
        n.drawn = true;
    }
}

/// Keeps a box (x, y, width, height on luma texels, even) of NV12 bytes into subresource `sub`:
/// `mem` the box's luma rows, then its chroma rows (`pitch` apart).
unsafe fn nv12_keep(t: &mut Texture, sub: u32, mem: usize, pitch: u32, [x0, y0, w, h]: [u32; 4]) {
    let (gp, rows) = format::pitch_rows(t.fmt, t.w, t.h);
    let lh = t.h;
    let keep = t.bits.entry(sub).or_insert_with(|| vec![0; us(gp * rows)]);
    for r in 0..h {
        let at = us((y0 + r) * gp + x0);
        keep[at..at + us(w)].copy_from_slice(std::slice::from_raw_parts(std::ptr::with_exposed_provenance::<u8>(mem + us(r * pitch)), us(w)));
    }
    for r in 0..h / 2 {
        let at = us((lh + y0 / 2 + r) * gp + x0);
        keep[at..at + us(w)].copy_from_slice(std::slice::from_raw_parts(std::ptr::with_exposed_provenance::<u8>(mem + us((h + r) * pitch)), us(w)));
    }
}

/// A box of NV12 bytes of subresource `sub` (see `nv12_keep`), rows `w` apart.
fn nv12_box(t: &Texture, sub: u32, [x0, y0, w, h]: [u32; 4]) -> Vec<u8> {
    let gp = format::pitch_rows(t.fmt, t.w, t.h).0;
    let mut out = vec![0u8; us(w * (h + h / 2))];
    if let Some(b) = t.bits.get(&sub) {
        for r in 0..h {
            let at = us((y0 + r) * gp + x0);
            out[us(r * w)..us((r + 1) * w)].copy_from_slice(&b[at..at + us(w)]);
        }
        for r in 0..h / 2 {
            let at = us((t.h + y0 / 2 + r) * gp + x0);
            out[us((h + r) * w)..us((h + r + 1) * w)].copy_from_slice(&b[at..at + us(w)]);
        }
    }
    out
}

/// A buffer view's texture, brought up to date with its buffer.
fn buffer_view_sync(d: &mut D11, view: u32) {
    let Some(Data::View(v)) = d.objs.get(&view).map(|o| &o.data) else { return };
    let Some(b) = &v.buf else { return };
    if matches!(d.objs.get(&b.buf).map(|o| &o.data), Some(Data::Texture(_))) {
        return nv12_view_sync(d, view);
    }
    let bid = b.buf;
    expand::pull(d, bid);
    let Some(Data::View(v)) = d.objs.get(&view).map(|o| &o.data) else { return };
    let Some(b) = &v.buf else { return };
    let Some(Data::Buffer(buf)) = d.objs.get(&b.buf).map(|o| &o.data) else { return };
    if buf.mem == 0 {
        return;
    }
    let now = unsafe { std::slice::from_raw_parts(gc::<u8>(buf.mem + b.off), us(b.bytes)) };
    if now == b.last.as_slice() {
        return;
    }
    let now = now.to_vec();
    if d.trace {
        eprintln!("d3d11: buffer view {view} <- {} bytes of buffer {}", now.len(), b.buf);
    }
    let (res, bytes) = (v.res, b.bytes);
    if let Some(Data::Texture(t)) = d.objs.get_mut(&res).map(|o| &mut o.data) {
        unsafe { upload_texture(t, 0, now.as_ptr().expose_provenance(), bytes, 0, None) };
    }
    if let Some(Data::View(v)) = d.objs.get_mut(&view).map(|o| &mut o.data) && let Some(b) = v.buf.as_mut() {
        b.last = now;
    }
}

/// Methods of device children: views, shaders, states, resources.
unsafe fn child_call(d: &mut D11, iface: u32, m: u32, this: u32, a: &dyn Fn(usize) -> u32) -> u32 {
    match m {
        3 => {
            let dev = d.made_by.get(&this).copied().unwrap_or(d.device);
            d.add_ref(dev);
            wrp(a(1), 0, d.addr_of(dev));
            return S_OK;
        }
        4..=6 => return private_data(d, m, this, a),
        _ => {}
    }
    let data = d.objs.get(&this).map(|o| &o.data);
    match (iface, m, data) {
        // resources: GetType, SetEvictionPriority, GetEvictionPriority, GetDesc
        (IF_BUFFER | IF_TEX1D | IF_TEX2D | IF_TEX3D, 7, _) => {
            wr(a(1), 0, iface - IF_BUFFER + 1);
            S_OK
        }
        (IF_BUFFER | IF_TEX1D | IF_TEX2D | IF_TEX3D, 8, _) => S_OK,
        (IF_BUFFER | IF_TEX1D | IF_TEX2D | IF_TEX3D, 9, _) => 0,
        (IF_BUFFER, 10, Some(Data::Buffer(b))) => {
            for (i, v) in [b.size, b.usage, b.bind, b.cpu, b.misc, b.stride].into_iter().enumerate() {
                wr(a(1), i, v);
            }
            S_OK
        }
        (IF_TEX1D | IF_TEX2D | IF_TEX3D, 10, Some(Data::Texture(t))) => {
            let (cpu, misc) = (t.cpu, t.misc);
            let v: Vec<u32> = match iface {
                IF_TEX1D => vec![t.w, t.mips, t.layers, t.fmt, t.usage, t.bind, cpu, misc],
                IF_TEX2D => vec![t.w, t.h, t.mips, t.layers, t.fmt, t.samples.0, t.samples.1, t.usage, t.bind, cpu, misc],
                _ => vec![t.w, t.h, t.layers, t.mips, t.fmt, t.usage, t.bind, cpu, misc],
            };
            for (i, x) in v.into_iter().enumerate() {
                wr(a(1), i, x);
            }
            S_OK
        }
        // views: GetResource, GetDesc
        (IF_SRV | IF_RTV | IF_DSV | IF_UAV, 7, Some(Data::View(v))) => {
            let r = v.buf.as_ref().map_or(v.res, |b| b.buf);
            d.add_ref(r);
            wrp(a(1), 0, d.addr_of(r));
            S_OK
        }
        (IF_SRV | IF_RTV | IF_DSV | IF_UAV, 8, Some(Data::View(v))) => {
            let w = v.desc.clone();
            for (i, x) in w.into_iter().enumerate() {
                wr(a(1), i, x);
            }
            S_OK
        }
        // states: GetDesc (GetDesc1/2: the newer descriptions); a NULL one: nothing written (the
        // guest's address 0 is the host's memory's start, not a fault)
        (IF_BLEND | IF_DEPTH | IF_RASTER | IF_SAMPLER, 7..=9, _) if a(1) == 0 => 0,
        (IF_BLEND | IF_DEPTH | IF_RASTER | IF_SAMPLER, 7, Some(Data::State(s))) => {
            let n = match iface {
                IF_BLEND => 66,
                IF_RASTER => 10,
                _ => s.len(),
            };
            std::ptr::copy_nonoverlapping(s.as_ptr(), gp::<u32>(a(1)), n.min(s.len()));
            0
        }
        (IF_RASTER, 8 | 9, Some(Data::State(s))) => {
            std::ptr::copy_nonoverlapping(s.as_ptr(), gp::<u32>(a(1)), if m == 8 { 11 } else { 12 });
            0
        }
        (IF_BLEND, 8, Some(Data::State(s))) => {
            // D3D11_BLEND_DESC1: targets of BlendEnable, LogicOpEnable, the 6 factors and
            // operations, LogicOp, RenderTargetWriteMask
            wr(a(1), 0, s[0]);
            wr(a(1), 1, s[1]);
            for t in 0..8 {
                let (b, o) = (2 + t * 10, 2 + t * 8);
                wr(a(1), b, s[o]);
                wr(a(1), b + 1, s[66 + t * 2]);
                for k in 0..6 {
                    wr(a(1), b + 2 + k, s[o + 1 + k]);
                }
                // (a target without a logic op reads back NOOP, winref d3d11copy)
                wr(a(1), b + 8, if s[66 + t * 2] != 0 { s[67 + t * 2] } else { 4 });
                wr(a(1), b + 9, s[o + 7]);
            }
            0
        }
        // queries: GetDataSize, GetDesc
        (IF_QUERY, 7, Some(Data::Query(q))) => q.kind.data_size(),
        (IF_QUERY, 8, Some(Data::Query(q))) => {
            // SAFETY: wr's own contract: GetDesc's D3D11_QUERY_DESC
            unsafe { wr(a(1), 0, q.kind.type_id()) };
            wr(a(1), 1, 0);
            S_OK
        }
        _ => E_NOTIMPL,
    }
}

// ---------------------------------------------------------------------------------------------
// ID3D11DeviceContext

/// Sets `n` slots from `start` of a slot array to the objects in `p` (guest addresses).
unsafe fn set_slots(d: &mut D11, slots: &mut [u32], start: u32, n: u32, p: u32) {
    for i in 0..n {
        let s = us(start + i);
        if s >= slots.len() {
            break;
        }
        let new = if p == 0 { 0 } else { d.id_of(rdp(p, us(i))) };
        let old = slots[s];
        slots[s] = new;
        d.rebind(new, old);
    }
}

/// the stage (VS PS GS HS DS CS order of `State`) of each of the six *ConstantBuffers1 methods
/// (VS HS DS GS PS CS order of the vtable)
const CB1_STAGE: [usize; 6] = [0, 3, 4, 2, 1, 5];

/// A constant buffer slot's range: its first constant and how many (16-byte constants).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CbRange {
    pub first: u32,
    pub count: u32,
}

impl CbRange {
    /// what *SetConstantBuffers binds: from the start, D3D11_REQ_CONSTANT_BUFFER_ELEMENT_COUNT
    /// (4096) constants
    pub const WHOLE: CbRange = CbRange { first: 0, count: 4096 };
    /// an empty slot's (Get*ConstantBuffers1 past the slots)
    pub const NONE: CbRange = CbRange { first: 0, count: 0 };

    /// the bytes the shader sees of a buffer of `size` bytes: (offset, length); constants past
    /// the buffer's end read as 0 (the binding is cut there)
    pub fn bytes(self, size: u32) -> (u32, u32) {
        let off = self.first.saturating_mul(16).min(size);
        (off, self.count.saturating_mul(16).min(size.saturating_sub(off)))
    }
}

/// *SetConstantBuffers(start, n, buffers) (`firsts` and `counts` 0) and
/// *SetConstantBuffers1(start, n, buffers, firsts, counts): each slot's buffer and range. A
/// range is given in 16-byte constants: the first a multiple of 16, the count a multiple of 16
/// up to 4096 (D3D11.1 functional spec, ConstantBufferOffsetting); either array NULL binds
/// whole buffers (winref d3d11cb1).
unsafe fn set_cbs(d: &mut D11, st: &mut State, stage: usize, start: u32, n: u32, bufs: u32, firsts: u32, counts: u32) {
    let (Some(slots), Some(ranges)) = (st.cbs.get_mut(stage), st.cb_ranges.get_mut(stage)) else { return };
    // SAFETY: set_slots' own contract: `bufs` is the caller's array of `n` pointers
    unsafe { set_slots(d, slots, start, n, bufs) };
    for i in 0..n {
        let Some(slot) = ranges.get_mut(us(start.saturating_add(i))) else { break };
        *slot = if firsts == 0 || counts == 0 {
            CbRange::WHOLE
        } else {
            // SAFETY: rd's own contract: `firsts` and `counts` are the caller's arrays of `n`
            let first = unsafe { rd(firsts, us(i)) };
            // SAFETY: (the same)
            let count = unsafe { rd(counts, us(i)) };
            CbRange { first, count }
        };
    }
}

/// Gets `n` slots from `start` into `p` (with references).
unsafe fn get_slots(d: &mut D11, slots: &[u32], start: u32, n: u32, p: u32) {
    // (no array: nothing asked for, no reference given)
    if p == 0 {
        return;
    }
    for i in 0..n {
        let id = slots.get(us(start + i)).copied().unwrap_or(0);
        if id != 0 {
            d.add_ref(id);
        }
        wrp(p, us(i), d.addr_of(id));
    }
}

unsafe fn set_one(d: &mut D11, slot: &mut u32, addr: u32) {
    let new = d.id_of(addr);
    let old = *slot;
    *slot = new;
    d.rebind(new, old);
}

unsafe fn get_one(d: &mut D11, id: u32, out: u32) {
    // (no out pointer: nothing asked for, no reference given)
    if out == 0 {
        return;
    }
    if id != 0 {
        d.add_ref(id);
    }
    wrp(out, 0, d.addr_of(id));
}

/// Whether SetPredication's predicate turns rendering off now: its answer (waited for: the
/// page measures it) is the value given. A predicate never ended predicates nothing.
fn predicated_off(d: &D11, st: &State) -> bool {
    let (p, value) = st.predicate;
    let Some(Data::Query(q)) = d.objs.get(&p).map(|o| &o.data) else { return false };
    q.predicate_answer().is_some_and(|r| r == (value != 0))
}

unsafe fn context_call(d: &mut D11, m: u32, a: &dyn Fn(usize) -> u32, after: &mut After) -> u32 {
    let mut st = std::mem::replace(&mut d.st, default_state());
    let r = context_call2(d, &mut st, m, a, after);
    d.st = st;
    r
}

unsafe fn context_call2(d: &mut D11, st: &mut State, m: u32, a: &dyn Fn(usize) -> u32, after: &mut After) -> u32 {
    // (what SetPredication's predicate turns off: draws, dispatches, copies, updates, clears,
    // resolves, GenerateMips)
    if matches!(m, 12 | 13 | 20 | 21 | 38..=42 | 46..=48 | 50..=54 | 57 | 132) && predicated_off(d, st) {
        return 0;
    }
    match m {
        3 => {
            d.add_ref(d.device);
            wrp(a(1), 0, d.addr_of(d.device));
        }
        4..=6 => {
            let ctx = d.context;
            return private_data(d, m, ctx, a);
        }
        // SAFETY: set_cbs' own contract: the caller's arguments of *SetConstantBuffers
        7 | 16 => unsafe { set_cbs(d, st, usize::from(m == 16), a(1), a(2), a(3), 0, 0) },
        8 | 25 => {
            set_slots(d, &mut st.srvs[usize::from(m == 8)], a(1), a(2), a(3));
            hazards::after_srvs(d, st, usize::from(m == 8), a(1), a(2));
        }
        9 => set_one(d, &mut st.ps, a(1)),
        10 | 26 => set_slots(d, &mut st.samplers[usize::from(m == 10)], a(1), a(2), a(3)),
        11 => set_one(d, &mut st.vs, a(1)),
        12 => draw(d, st, true, a(1), 1, a(2), a(3).cast_signed(), 0),
        13 => draw(d, st, false, a(1), 1, a(2), 0, 0),
        14 => return map(d, a(1), a(2), a(3), a(5), false, after),
        15 => unmap(d, a(1), a(2)),
        17 => set_one(d, &mut st.layout, a(1)),
        18 => {
            // IASetVertexBuffers(start, n, buffers, strides, offsets)
            for i in 0..a(2) {
                let s = us(a(1) + i);
                if s >= 32 {
                    break;
                }
                let new = if a(3) == 0 { 0 } else { d.id_of(rdp(a(3), us(i))) };
                let old = st.vbs[s].0;
                st.vbs[s] = (new, if a(4) != 0 { rd(a(4), us(i)) } else { 0 }, if a(5) != 0 { rd(a(5), us(i)) } else { 0 });
                d.rebind(new, old);
            }
            hazards::after_buffers(d, st);
        }
        19 => {
            let new = d.id_of(a(1));
            let old = st.ib.0;
            st.ib = (new, a(2), a(3));
            d.rebind(new, old);
        }
        // (no instances: nothing drawn)
        20 => draw(d, st, true, a(1), a(2), a(3), a(4).cast_signed(), a(5)),
        21 => draw(d, st, false, a(1), a(2), a(3), 0, a(4)),
        24 => st.topology = a(1),
        27 | 28 => {
            // Begin, End
            let q = d.id_of(a(1));
            let now = d.stats;
            let on_page = matches!(d.objs.get(&q).map(|o| &o.data), Some(Data::Query(x)) if x.counts_on_page());
            let counters = if on_page { expand::counters(d) } else { 0 };
            // (PIPELINE_STATISTICS counts the clipper's work on the page: the draws while one is
            // open are expanded, expand.rs)
            if matches!(d.objs.get(&q).map(|o| &o.data), Some(Data::Query(x)) if x.kind == query::Kind::PipelineStatistics) {
                d.expand.open.retain(|&x| x != q);
                if m == 27 {
                    d.expand.open.push(q);
                }
            }
            if let Some(Data::Query(x)) = d.objs.get_mut(&q).map(|o| &mut o.data) {
                if m == 28 { x.end(&now) } else { x.begin(&now, counters) }
            }
        }
        29 => {
            // GetData(query, data, size, flags)
            let q = d.id_of(a(1));
            let Some(&Data::Query(x)) = d.objs.get(&q).map(|o| &o.data) else { return E_INVALIDARG };
            let (hr, bytes) = x.get_data(a(2) != 0, a(3), a(4));
            if !bytes.is_empty() {
                // SAFETY: the guest's buffer of a(3) bytes (get_data gives a(3) bytes or none)
                unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), gp::<u8>(a(2)), bytes.len()) };
            }
            return hr;
        }
        33 | 34 => {
            // OMSetRenderTargets(n, rtvs, dsv) (+ UAVs (start, n, uavs): kept, not drawn with)
            if m == 34 && a(5) != 0xffff_ffff {
                let mut u = st.uavs[0];
                set_slots(d, &mut u, 0, 8, 0);
                set_slots(d, &mut u, a(4), a(5).min(8), a(6));
                st.uavs[0] = u;
            }
            let n = if m == 34 && a(1) == 0xffff_ffff { u32::MAX } else { a(1) };
            if n != u32::MAX {
                let mut rt = st.rtvs;
                set_slots(d, &mut rt, 0, 8, 0);
                set_slots(d, &mut rt, 0, n.min(8), a(2));
                st.rtvs = rt;
                set_one(d, &mut st.dsv, a(3));
                if m == 33 {
                    let mut u = st.uavs[0];
                    set_slots(d, &mut u, 0, 8, 0);
                    st.uavs[0] = u;
                }
                hazards::after_outputs(d, st);
                // the viewport follows the first target when the game never set one
                if st.viewport[0][2] == 0.0 && let Some((w, h)) = view_size(d, st.rtvs[0]).or_else(|| view_size(d, st.dsv)) {
                    st.viewport[0] = [0.0, 0.0, w as f32, h as f32, 0.0, 1.0];
                }
            }
        }
        35 => {
            set_one(d, &mut st.blend, a(1));
            st.blend_factor = if a(2) != 0 { [rdf(a(2), 0), rdf(a(2), 1), rdf(a(2), 2), rdf(a(2), 3)] } else { [1.0; 4] };
            st.sample_mask = a(3);
        }
        36 => {
            set_one(d, &mut st.depth, a(1));
            st.stencil_ref = a(2);
        }
        43 => set_one(d, &mut st.raster, a(1)),
        44 => {
            st.nviewports = if a(2) != 0 { a(1).min(16) } else { 0 };
            if a(2) != 0 {
                for k in 0..us(st.nviewports) {
                    for i in 0..6 {
                        st.viewport[k][i] = rdf(a(2), 6 * k + i);
                    }
                }
                if d.trace {
                    eprintln!("d3d11: viewport {:?}", st.viewport[0]);
                }
            }
        }
        45 => {
            st.nscissors = if a(2) != 0 { a(1).min(16) } else { 0 };
            if a(2) != 0 {
                for k in 0..us(st.nscissors) {
                    for i in 0..4 {
                        st.scissor[k][i] = rd(a(2), 4 * k + i).cast_signed();
                    }
                }
            }
        }
        46 => copy_region(d, a(1), a(2), a(3), a(4), a(5), a(6), a(7), a(8)),
        47 => copy_resource(d, a(1), a(2)),
        57 => resolve(d, a(1), a(2), a(3), a(4), a(5)),
        48 => update_subresource(d, a(1), a(2), a(3), a(4), a(5), a(6)),
        50 => {
            // ClearRenderTargetView(view, color)
            let v = d.id_of(a(1));
            nv12_target(d, v);
            if let Some((tex, level, layer)) = view_target(d, v) {
                let layers = view_layers(d, v);
                let mut c = [rd(a(2), 0), rd(a(2), 1), rd(a(2), 2), rd(a(2), 3)];
                // 8-bit UNORM targets: the color as the runtime converts it (each channel
                // clamped, times 255 in floats, rounded to even), not left to the GPU's own
                if let Some(Data::View(View { fmt: 28 | 49 | 61 | 65 | 87 | 88, .. })) = d.objs.get(&v).map(|o| &o.data) {
                    for x in &mut c {
                        let f = f32::from_bits(*x);
                        let f = if f.is_nan() { 0.0 } else { f.clamp(0.0, 1.0) };
                        *x = ((f * 255.0).round_ties_even() / 255.0).to_bits();
                    }
                }
                // (every layer (or 3D slice) the view has)
                for l in layer..layer + layers {
                    gpu::record(|q| q.cmd(&[gpu::CLEAR_VIEW, tex, level, l, 1, c[0], c[1], c[2], c[3], 0, 0]));
                }
                d.targets = None;
            }
        }
        53 => {
            // ClearDepthStencilView(view, flags, depth, stencil)
            let v = d.id_of(a(1));
            if let Some((tex, level, layer)) = view_target(d, v) {
                for l in layer..layer + view_layers(d, v) {
                    gpu::record(|q| q.cmd(&[gpu::CLEAR_VIEW, tex, level, l, (a(2) & 3) << 1, 0, 0, 0, 0, a(3), a(4) & 0xff]));
                }
                d.targets = None;
            }
        }
        54 => {
            let v = d.id_of(a(1));
            if let Some(Data::View(v)) = d.objs.get(&v).map(|o| &o.data) && let Some(Data::Texture(t)) = d.objs.get(&v.res).map(|o| &o.data) {
                let g = t.gpu;
                gpu::record(|q| q.cmd(&[gpu::GEN_MIPS, g]));
            }
        }
        30 => {
            let mut p = st.predicate.0;
            set_one(d, &mut p, a(1));
            st.predicate = (p, a(2));
        }
        86 => {
            get_one(d, st.predicate.0, a(1));
            wr(a(2), 0, st.predicate.1);
        }
        // GS, HS, DS, CS: constant buffers, resources, samplers, shaders (kept, not drawn with)
        // SAFETY: set_cbs' own contract: the caller's arguments of *SetConstantBuffers
        22 | 62 | 66 | 71 => unsafe { set_cbs(d, st, stage_of(m), a(1), a(2), a(3), 0, 0) },
        31 | 59 | 63 | 67 => {
            set_slots(d, &mut st.srvs[stage_of(m)], a(1), a(2), a(3));
            hazards::after_srvs(d, st, stage_of(m), a(1), a(2));
        }
        32 | 61 | 65 | 70 => set_slots(d, &mut st.samplers[stage_of(m)], a(1), a(2), a(3)),
        23 | 60 | 64 | 69 => set_one(d, &mut st.shaders[stage_of(m) - 2], a(1)),
        68 => {
            set_slots(d, &mut st.uavs[1], a(1), a(2), a(3));
            hazards::after_outputs(d, st);
        }
        37 => {
            // SOSetTargets(n, buffers, offsets): the buffers stream output writes (expand.rs),
            // each from its offset (-1: on from what it holds; no offsets: 0)
            let mut so = st.so;
            set_slots(d, &mut so, 0, 4, 0);
            set_slots(d, &mut so, 0, a(1).min(4), a(2));
            st.so = so;
            for k in 0..a(1).min(4) {
                let buf = so.get(us(k)).copied().unwrap_or(0);
                let off = if a(3) == 0 {
                    0
                } else {
                    // SAFETY: rd's own contract: the caller's array of `n` offsets
                    unsafe { rd(a(3), us(k)) }
                };
                if buf != 0 && off != u32::MAX {
                    expand::set_fill(d, buf, off);
                }
            }
            hazards::after_buffers(d, st);
        }
        // ClearUnorderedAccessViewUint(view, values[4]), ClearUnorderedAccessViewFloat(view,
        // values[4]): the whole view
        51 | 52 => {
            let v = d.id_of(a(1));
            // SAFETY: rd's own contract: the caller's array of four values
            let vals: [u32; 4] = std::array::from_fn(|i| unsafe { rd(a(2), i) });
            let vals = if m == 51 { clear::Values::Uint(vals) } else { clear::Values::Float(vals.map(f32::from_bits)) };
            // SAFETY: clear_view_texels' own contract: a view of this device
            unsafe { clear_view_texels(d, v, vals, &[]) };
        }
        49 | 55 => {}
        41 => dispatch(d, st, a(1), a(2), a(3)),
        42 => {
            // DispatchIndirect(args, offset): the group counts from the buffer
            expand::pull(d, d.id_of(a(1)));
            if let Some(Data::Buffer(b)) = d.objs.get(&d.id_of(a(1))).map(|o| &o.data) && u64::from(a(2)) + 12 <= u64::from(b.size) {
                let p = b.mem + a(2);
                let (x, y, z) = (rd(p, 0), rd(p, 1), rd(p, 2));
                dispatch(d, st, x, y, z);
            }
        }
        39 | 40 => {
            // DrawIndexedInstancedIndirect / DrawInstancedIndirect(args, offset): the counts
            // from the buffer
            let n: u32 = if m == 39 { 5 } else { 4 };
            expand::pull(d, d.id_of(a(1)));
            if let Some(Data::Buffer(b)) = d.objs.get(&d.id_of(a(1))).map(|o| &o.data) && a(2).is_multiple_of(4) && u64::from(a(2)).saturating_add(u64::from(n) * 4) <= u64::from(b.size) {
                let p = b.mem.saturating_add(a(2));
                let [c, inst, first, base, first_inst]: [u32; 5] = std::array::from_fn(|i| {
                    if i >= us(n) {
                        return 0;
                    }
                    // SAFETY: rd's own contract: the buffer's guest memory, `n` words inside it (checked above)
                    unsafe { rd(p, i) }
                });
                if m == 39 {
                    // SAFETY: draw's own contract: this context's state
                    unsafe { draw(d, st, true, c, inst, first, base.cast_signed(), first_inst) };
                } else {
                    // SAFETY: draw's own contract: this context's state
                    unsafe { draw(d, st, false, c, inst, first, 0, base) };
                }
            }
        }
        38 => {
            // DrawAuto: the vertices stream output wrote to the buffer in input slot 0 (its
            // fill, from the offset it is bound at, by its stride)
            let (buf, stride, off) = st.vbs[0];
            if buf != 0 && let Some(fill) = expand::fill_of(d, buf) && let Some(n) = fill.saturating_sub(off).checked_div(stride) {
                // SAFETY: draw's own contract: this context's state
                unsafe { draw(d, st, false, n, 1, 0, 0, 0) };
            }
        }
        72 | 77 => get_slots(d, &st.cbs[usize::from(m == 77)], a(1), a(2), a(3)),
        73 | 84 => get_slots(d, &st.srvs[usize::from(m == 73)], a(1), a(2), a(3)),
        74 => {
            get_one(d, st.ps, a(1));
            wr(a(3), 0, 0);
        }
        75 | 85 => get_slots(d, &st.samplers[usize::from(m == 75)], a(1), a(2), a(3)),
        76 => {
            get_one(d, st.vs, a(1));
            wr(a(3), 0, 0);
        }
        78 => get_one(d, st.layout, a(1)),
        79 => {
            for i in 0..a(2) {
                let s = us(a(1) + i);
                let (b, stride, off) = st.vbs.get(s).copied().unwrap_or_default();
                if a(3) != 0 {
                    get_one(d, b, a(3) + i * ptr());
                }
                wr(a(4), us(i), stride);
                wr(a(5), us(i), off);
            }
        }
        80 => {
            get_one(d, st.ib.0, a(1));
            wr(a(2), 0, st.ib.1);
            wr(a(3), 0, st.ib.2);
        }
        82 | 98 | 102 | 107 => {
            get_one(d, st.shaders[stage_of(m) - 2], a(1));
            if a(3) != 0 {
                wr(a(3), 0, 0);
            }
        }
        81 | 100 | 104 | 109 => get_slots(d, &st.cbs[stage_of(m)], a(1), a(2), a(3)),
        87 | 97 | 101 | 105 => get_slots(d, &st.srvs[stage_of(m)], a(1), a(2), a(3)),
        88 | 99 | 103 | 108 => get_slots(d, &st.samplers[stage_of(m)], a(1), a(2), a(3)),
        106 => get_slots(d, &st.uavs[1], a(1), a(2), a(3)),
        83 => wr(a(1), 0, st.topology),
        89 | 90 => {
            if a(2) != 0 {
                get_slots(d, &st.rtvs, 0, a(1), a(2));
            }
            if a(3) != 0 {
                get_one(d, st.dsv, a(3));
            }
            if m == 90 && a(6) != 0 {
                get_slots(d, &st.uavs[0], a(4), a(5), a(6));
            }
        }
        91 => {
            get_one(d, st.blend, a(1));
            if a(2) != 0 {
                for i in 0..4 {
                    wr(a(2), i, st.blend_factor[i].to_bits());
                }
            }
            wr(a(3), 0, st.sample_mask);
        }
        92 => {
            get_one(d, st.depth, a(1));
            wr(a(2), 0, st.stencil_ref);
        }
        93 => get_slots(d, &st.so, 0, a(1), a(2)),
        94 => get_one(d, st.raster, a(1)),
        95 | 96 => {
            // RSGetViewports/RSGetScissorRects(count, out): the count set; the array gets those
            // and zeros after them
            if a(1) != 0 {
                let (have, words) = if m == 95 { (st.nviewports, 6) } else { (st.nscissors, 4) };
                let room = rd(a(1), 0);
                wr(a(1), 0, have);
                if a(2) != 0 {
                    for k in 0..us(room) {
                        for i in 0..words {
                            let v = if (lo32(k)) >= have { 0 } else if m == 95 { st.viewport[k][i].to_bits() } else { (st.scissor[k][i]).cast_unsigned() };
                            wr(a(2), k * words + i, v);
                        }
                    }
                }
            }
        }
        110 => {
            // ClearState
            let old = std::mem::replace(st, default_state());
            unbind_state(d, &old);
        }
        111 => gpu::flush(),
        112 => return 0, // immediate
        113 => return 0,
        114 => return DXGI_ERROR_INVALID_CALL,
        // ID3D11DeviceContext1
        // CopySubresourceRegion1 / UpdateSubresource1: D3D11_COPY_NO_OVERWRITE and DISCARD only
        // promise what the program does with the destination; the copy's bytes are the same
        // (D3D11.1 functional spec, as Windows' WARP answers: winref d3d11cb1)
        115 => return context_call2(d, st, 46, a, after),
        116 => return context_call2(d, st, 48, a, after),
        117 | 118 | 133 => {}                           // Discard*
        // VS, HS, DS, GS, PS, CS SetConstantBuffers1(start, n, buffers, first, count) and
        // Get*ConstantBuffers1(start, n, buffers, first, count): each slot's range of constants
        119..=124 => {
            let stage = CB1_STAGE.get(us(m.saturating_sub(119))).copied().unwrap_or(0);
            // SAFETY: set_cbs' own contract: the caller's arguments of *SetConstantBuffers1
            unsafe { set_cbs(d, st, stage, a(1), a(2), a(3), a(4), a(5)) };
        }
        125..=130 => {
            let stage = CB1_STAGE.get(us(m.saturating_sub(125))).copied().unwrap_or(0);
            let (Some(slots), Some(ranges)) = (st.cbs.get(stage), st.cb_ranges.get(stage)) else { return S_OK };
            // SAFETY: get_slots' own contract: the caller's array of a(2) pointers (or NULL)
            unsafe { get_slots(d, slots, a(1), a(2), a(3)) };
            for i in 0..a(2) {
                let r = ranges.get(us(a(1).saturating_add(i))).copied().unwrap_or(CbRange::NONE);
                if a(4) != 0 {
                    // SAFETY: wr's own contract: the caller's arrays of a(2) dwords
                    unsafe { wr(a(4), us(i), r.first) };
                }
                if a(5) != 0 {
                    // SAFETY: (the same)
                    unsafe { wr(a(5), us(i), r.count) };
                }
            }
        }
        131 => swap_state(d, st, a(1), a(2)),
        58 => deferred::execute(d, st, a(1), a(2), after),
        // ClearView(view, color[4], rects, n): a render target or unordered access view, in
        // rectangles of the view (none or NULL: all of it), the color as a Float clear's
        // (D3D11.1 functional spec, ClearView)
        132 => {
            let v = d.id_of(a(1));
            // SAFETY: rd's own contract: the caller's color of four floats
            let c: [u32; 4] = std::array::from_fn(|i| unsafe { rd(a(2), i) });
            let n = us(a(4).min(4096));
            let rect = |i: usize, k: usize| {
                // SAFETY: rd's own contract: the caller's array of `n` D3D11_RECTs
                unsafe { rd(a(3), i.saturating_mul(4).saturating_add(k)) }.cast_signed()
            };
            let rects: Vec<[i32; 4]> = if a(3) != 0 { (0..n).map(|i| std::array::from_fn(|k| rect(i, k))).collect() } else { Vec::new() };
            if rects.is_empty() && matches!(d.objs.get(&v).map(|o| o.iface), Some(IF_RTV)) {
                // SAFETY: context_call2's own contract: ClearRenderTargetView(view, color)
                return unsafe { context_call2(d, st, 50, &|i| if i == 1 { a(1) } else { a(2) }, after) };
            }
            // SAFETY: clear_view_texels' own contract: a view of this device
            unsafe { clear_view_texels(d, v, clear::Values::Float(c.map(f32::from_bits)), &rects) };
        }
        // ID3D11DeviceContext2: tiled resources and annotations
        134..=139 | 141..=143 => {}
        140 => return 0, // IsAnnotationEnabled
        // ID3D11DeviceContext3: Flush1(type, event); hardware protection (none: off)
        144 => return context_call2(d, st, 111, a, after),
        145 => {}
        146 => {
            if a(1) != 0 {
                wr(a(1), 0, 0);
            }
        }
        // ID3D11DeviceContext4: Signal / Wait on fences (CreateFence gives none)
        147 | 148 => return E_INVALIDARG,
        _ => return E_NOTIMPL,
    }
    S_OK
}

/// SwapDeviceContextState(state, prev_out): `st` (the current state) goes back into the object
/// it came from (or a new one for the context's own state) and `state`'s is loaded.
unsafe fn swap_state(d: &mut D11, st: &mut State, state: u32, prev_out: u32) {
    let new = d.id_of(state);
    let Some(Data::CtxState(slot)) = d.objs.get_mut(&new).map(|o| &mut o.data) else {
        // (no state: nothing changes, and no previous one comes back)
        wrp(prev_out, 0, 0);
        return;
    };
    let Some(loaded) = slot.take() else {
        // (already current: it is the previous one too)
        if prev_out != 0 {
            d.add_ref(new);
            wrp(prev_out, 0, state);
        }
        return;
    };
    let cur = std::mem::replace(st, *loaded);
    let old = d.st_owner;
    d.st_owner = new;
    // (the context holds its current state privately: the application may let go of it)
    d.bind(new);
    if let Some(Data::CtxState(slot)) = d.objs.get_mut(&old).map(|o| &mut o.data) {
        *slot = Some(Box::new(cur));
        if prev_out != 0 {
            d.add_ref(old);
            wrp(prev_out, 0, d.addr_of(old));
        }
        d.unbind(old);
    } else {
        // the context's own state (or one whose object is gone): a new object
        let o = d.new_obj(IF_LINKAGE, Data::CtxState(Some(Box::new(cur))));
        if prev_out != 0 {
            wrp(prev_out, 0, o);
        } else {
            let id = d.id_of(o);
            d.release(id);
        }
    }
}

fn view_size(d: &D11, v: u32) -> Option<(u32, u32)> {
    let Some(Data::View(v)) = d.objs.get(&v).map(|o| &o.data) else { return None };
    let Some(Data::Texture(t)) = d.objs.get(&v.res).map(|o| &o.data) else { return None };
    Some(((t.w >> v.mip).max(1), (t.h >> v.mip).max(1)))
}

/// A render target or depth view's page texture, level and layer.
fn view_target(d: &D11, v: u32) -> Option<(u32, u32, u32)> {
    let Some(Data::View(v)) = d.objs.get(&v).map(|o| &o.data) else { return None };
    let Some(Data::Texture(t)) = d.objs.get(&v.res).map(|o| &o.data) else { return None };
    Some((t.gpu, v.mip, v.layer))
}

/// A clear of a render target or unordered access view of a texture by its texels: each of
/// `rects` (left top right bottom, cut to the view's level; none: all of it) of every layer
/// (3D slice) the view has, as UpdateSubresource of the texel `clear::texel` makes would write
/// them. A view of no texture, a format of no such texel: nothing.
unsafe fn clear_view_texels(d: &mut D11, v: u32, vals: clear::Values, rects: &[[i32; 4]]) {
    let Some(Data::View(view)) = d.objs.get(&v).map(|o| &o.data) else { return };
    let (res, fmt, mip, layer, layers) = (view.res, view.fmt, view.mip, view.layer, view.layers.max(1));
    if view.buf.is_some() {
        return;
    }
    let Some(texel) = clear::texel(fmt, vals) else { return };
    let Some(Data::Texture(t)) = d.objs.get_mut(&res).map(|o| &mut o.data) else { return };
    if mip >= t.mips.max(1) {
        return;
    }
    let (lw, lh) = ((t.w >> mip).max(1), (t.h >> mip).max(1));
    let whole = [[0, 0, lw.cast_signed(), lh.cast_signed()]];
    let rects = if rects.is_empty() { &whole[..] } else { rects };
    for r in rects {
        let (x0, y0) = (r[0].clamp(0, lw.cast_signed()).cast_unsigned(), r[1].clamp(0, lh.cast_signed()).cast_unsigned());
        let (x1, y1) = (r[2].clamp(0, lw.cast_signed()).cast_unsigned(), r[3].clamp(0, lh.cast_signed()).cast_unsigned());
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
        let row = texel.repeat(us(w));
        let bytes = row.repeat(us(h));
        let pitch = lo32(row.len());
        for l in layer..layer.saturating_add(layers) {
            // (3D: slice l of the level; arrays: subresource of the level in layer l)
            let (sub, bx) = if t.dim == 3 { (mip, [x0, y0, l, x1, y1, l.saturating_add(1)]) } else { (mip.saturating_add(l.saturating_mul(t.mips.max(1))), [x0, y0, 0, x1, y1, 1]) };
            t.clean.clear();
            t.pristine = false;
            // SAFETY: upload_texture's own contract: `bytes` holds the box's rows, `pitch` apart
            unsafe { upload_texture(t, sub, bytes.as_ptr().expose_provenance(), pitch, pitch.saturating_mul(h), Some(bx)) };
        }
    }
    d.targets = None;
}

/// How many array layers (3D slices) a view covers (1 at least).
fn view_layers(d: &D11, v: u32) -> u32 {
    match d.objs.get(&v).map(|o| &o.data) {
        Some(Data::View(v)) => v.layers.max(1),
        _ => 1,
    }
}

/// Map(resource, subresource, type, flags, out); `dxgi`: IDXGISurface::Map's DXGI_MAPPED_RECT
unsafe fn map(d: &mut D11, res: u32, sub: u32, ty: u32, out: u32, dxgi: bool, after: &mut After) -> u32 {
    let id = d.id_of(res);
    let mut reqs = Vec::new();
    // (what the runtime allows: staging resources read and write, dynamic ones write discard,
    // and buffers no overwrite too; default and immutable ones are not mapped)
    let usage = match d.objs.get(&id).map(|o| &o.data) {
        Some(Data::Buffer(b)) => Some((b.usage, true)),
        Some(Data::Texture(t)) => Some((t.usage, false)),
        _ => None,
    };
    if let Some((usage, buffer)) = usage.filter(|_| !dxgi) {
        let ok = match usage {
            3 => (1..=3).contains(&ty),
            2 => ty == 4 || (ty == 5 && buffer),
            _ => false,
        };
        if !ok {
            return E_INVALIDARG;
        }
    }
    // (what stream output wrote, back first: but for WRITE_DISCARD, whose bytes are new)
    if ty == 4 {
        if let Some(Data::Buffer(b)) = d.objs.get_mut(&id).map(|o| &mut o.data) {
            b.gpu_written = false;
        }
    } else {
        expand::pull(d, id);
    }
    let r = match d.objs.get_mut(&id).map(|o| &mut o.data) {
        Some(Data::Buffer(b)) => {
            // (D3D11_MAP_WRITE_NO_OVERWRITE: 5)
            b.stream = ty == 5 && (b.stream || !b.dirty);
            b.dirty = true;
            put_mapped(out, b.mem, b.size, b.size, dxgi);
            S_OK
        }
        Some(Data::Texture(t)) => {
            let level = sub % t.mips.max(1);
            let (lw, lh) = ((t.w >> level).max(1), (t.h >> level).max(1));
            let (pitch, rows) = format::pitch_rows(t.fmt, lw, lh);
            let depth = if t.dim == 3 { (t.layers >> level).max(1) } else { 1 };
            // (in 64 bits: a 2048^3 texture's level 0 does not fit in 32)
            let Some(size) = u32::try_from(u64::from(pitch) * u64::from(rows) * u64::from(depth)).ok().filter(|&n| n < 0x8000_0000) else {
                wrp(out, 0, 0);
                return E_OUTOFMEMORY;
            };
            let write = ty != 1;
            let mem = match t.maps.iter().position(|m| m.0 == sub) {
                Some(i) => {
                    t.maps[i].3 = write;
                    t.maps[i].1
                }
                None => 0,
            };
            let fresh = mem == 0;
            let mem = if mem != 0 {
                mem
            } else {
                let p = (*d.calls).alloc;
                let mem = lo32((p(size.max(16))).expose_provenance());
                if mem == 0 {
                    wrp(out, 0, 0);
                    return E_OUTOFMEMORY;
                }
                if t.pristine {
                    std::ptr::write_bytes(gp::<u8>(mem), 0, us(size.max(16)));
                }
                if let Some(Data::Texture(t)) = d.objs.get_mut(&id).map(|o| &mut o.data) {
                    t.maps.push((sub, mem, pitch, write));
                }
                mem
            };
            if let Some(Data::Texture(t)) = d.objs.get(&id).map(|o| &o.data) {
                // reading or writing part: what the texture holds (D3D11_MAP_WRITE keeps what is
                // there too: a part written goes back with the rest), the page's pixels where they
                // are the guest's bytes (one readback a slice), else the bytes kept of the last writes;
                // nothing where the map memory holds them already (since the last Unmap nothing wrote
                // the subresource: ANGLE uploads every texture through a staging one mapped so, and
                // the waits cost Godot seconds)
                let (_, layer, ..) = sub_size(t, sub);
                if ty != 4 && holds_bits(t) && !keeps_bits(t) && !t.clean.contains(&sub) && !(fresh && t.pristine) {
                    for z in 0..depth {
                        d.next_gpu += 1;
                        let req = 0x4000_0000 | d.next_gpu;
                        let (g, l) = (t.gpu, level | (layer + z) << 16);
                        gpu::record(|q| q.cmd(&[gpu::READBACK, req, g, l, 0, 0, lw, lh, mem + z * pitch * rows, pitch]));
                        reqs.push(req);
                    }
                } else if ty != 4 && !t.clean.contains(&sub) {
                    match t.bits.get(&sub) {
                        Some(b) => std::ptr::copy_nonoverlapping(b.as_ptr(), gp::<u8>(mem), b.len().min(us(size))),
                        None => std::ptr::write_bytes(gp::<u8>(mem), 0, us(size)),
                    }
                }
            }
            put_mapped(out, mem, pitch, pitch * rows, dxgi);
            S_OK
        }
        _ => E_INVALIDARG,
    };
    if !reqs.is_empty() {
        *after = After::Readback(reqs);
    }
    r
}

unsafe fn unmap(d: &mut D11, res: u32, sub: u32) {
    let id = d.id_of(res);
    if d.trace && let Some(Data::Buffer(b)) = d.objs.get(&id).map(|o| &o.data) && b.size <= 256 {
        let f: Vec<f32> = (0..b.size / 4).map(|i| *(gc::<f32>(b.mem + 4 * i))).collect();
        eprintln!("d3d11: buffer {id} = {f:?}");
    }
    if let Some(Data::Texture(t)) = d.objs.get_mut(&id).map(|o| &mut o.data) && let Some(&(_, mem, pitch, write)) = t.maps.iter().find(|m| m.0 == sub) {
        if write {
            let (_, _, lw, lh, _) = sub_size(t, sub);
            let rows = format::pitch_rows(t.fmt, lw, lh).1;
            upload_texture(t, sub, us(mem), pitch, pitch * rows, None);
        }
        // (the memory holds what the texture has now: read back or kept at the Map, or sent
        // up here)
        if t.usage == 3 && (holds_bits(t) || keeps_bits(t)) && !t.clean.contains(&sub) {
            t.clean.push(sub);
        }
    }
}

/// The page's copy of texture `id` changed (a copy or update into it): its map memory is stale.
fn texture_written(d: &mut D11, id: u32) {
    if let Some(Data::Texture(t)) = d.objs.get_mut(&id).map(|o| &mut o.data) {
        t.clean.clear();
        t.pristine = false;
    }
}

/// UpdateSubresource(resource, subresource, box, data, row pitch, depth pitch). Of a staging or
/// immutable resource it does nothing (Windows: winref d3d11copy's "update" of a staging texture
/// hashes as before it, every format, 2D and 3D).
unsafe fn update_subresource(d: &mut D11, res: u32, sub: u32, bx: u32, data: u32, pitch: u32, slice: u32) {
    let id = d.id_of(res);
    // (an NV12 texture drawn into: its bytes from the page first, a box changes only itself;
    // a buffer stream output wrote, too)
    nv12_pull(d, id);
    expand::pull(d, id);
    let usage = match d.objs.get(&id).map(|o| &o.data) {
        Some(Data::Buffer(b)) => b.usage,
        Some(Data::Texture(t)) => t.usage,
        _ => 0,
    };
    if matches!(usage, 1 | 3) {
        return;
    }
    let b = if bx != 0 { Some([rd(bx, 0), rd(bx, 1), rd(bx, 2), rd(bx, 3), rd(bx, 4), rd(bx, 5)]) } else { None };
    match d.objs.get_mut(&id).map(|o| &mut o.data) {
        Some(Data::Buffer(buf)) => {
            let (x0, x1) = b.map_or((0, buf.size), |b| (b[0], b[3].min(buf.size)));
            if x1 > x0 {
                std::ptr::copy(gc::<u8>(data), gp::<u8>(buf.mem + x0), us(x1 - x0));
                buf.dirty = true;
                buf.stream = false;
                if d.trace && buf.size <= 256 {
                    let f: Vec<f32> = (0..buf.size / 4).map(|i| *(gc::<f32>(buf.mem + 4 * i))).collect();
                    eprintln!("d3d11: buffer {id} = {f:?}");
                }
            }
        }
        Some(Data::Texture(t)) => {
            // the box's texels start at `data`
            t.clean.clear();
            t.pristine = false;
            match b {
                // (upload_texture reads the box's rows from `data` on: the box's first texel)
                Some(bb) => upload_texture(t, sub, us(data), pitch, slice, Some(bb)),
                None => upload_texture(t, sub, us(data), pitch, slice, None),
            }
        }
        _ => {}
    }
}

/// A 2D texture's width, height and DXGI format, by a guest pointer to it (Media Foundation's
/// Media Engine draws its frames into the game's textures: sake's mf.rs).
pub fn texture_desc(res: u32) -> Option<(u32, u32, u32)> {
    let g = D11S.lock().unwrap_or_else(|e| e.into_inner());
    let d = g.as_ref()?;
    match d.objs.get(&d.id_of(res)).map(|o| &o.data) {
        Some(Data::Texture(t)) if t.dim == 2 => Some((t.w, t.h, t.fmt)),
        _ => None,
    }
}

/// Writes guest texels (rows of `pitch` bytes from `data`) into box `bx` (left top right bottom)
/// of texture `res`'s first subresource, as UpdateSubresource. Whether it is one.
pub unsafe fn write_texture(res: u32, data: u32, pitch: u32, bx: [u32; 4]) -> bool {
    write_texture_from(res, gc::<u8>(data), pitch, bx)
}

/// [`write_texture`] from texels anywhere in memory (sake's own frames: no guest copy).
pub unsafe fn write_texture_from(res: u32, data: *const u8, pitch: u32, bx: [u32; 4]) -> bool {
    let mut g = D11S.lock().unwrap_or_else(|e| e.into_inner());
    let Some(d) = g.as_mut() else { return false };
    let id = d.id_of(res);
    let Some(Data::Texture(t)) = d.objs.get_mut(&id).map(|o| &mut o.data) else { return false };
    t.clean.clear();
    t.pristine = false;
    upload_texture(t, 0, data.expose_provenance(), pitch, pitch * (bx[3] - bx[1]), Some([bx[0], bx[1], 0, bx[2], bx[3], 1]));
    true
}

unsafe fn copy_resource(d: &mut D11, dst: u32, src: u32) {
    let (di, si) = (d.id_of(dst), d.id_of(src));
    texture_written(d, di);
    // (a buffer stream output wrote: its bytes back first; the destination's past the copy kept)
    expand::pull(d, si);
    expand::pull(d, di);
    match (d.objs.get(&di).map(|o| &o.data), d.objs.get(&si).map(|o| &o.data)) {
        (Some(Data::Buffer(a)), Some(Data::Buffer(b))) => {
            std::ptr::copy(gc::<u8>(b.mem), gp::<u8>(a.mem), us(a.size.min(b.size)));
            if let Some(Data::Buffer(a)) = d.objs.get_mut(&di).map(|o| &mut o.data) {
                a.dirty = true;
                a.stream = false;
            }
        }
        (Some(Data::Texture(a)), Some(Data::Texture(b))) => {
            // (every subresource both have: Windows wants them alike)
            let (mips, layers) = (a.mips.min(b.mips), if a.dim == 3 { 1 } else { a.layers.min(b.layers) });
            let (bm, am) = (b.mips, a.mips);
            for layer in 0..layers {
                for l in 0..mips {
                    copy_sub(d, di, layer * am + l, [0; 3], si, layer * bm + l, None);
                }
            }
        }
        _ => {}
    }
}

/// ResolveSubresource(dst, dst sub, src, src sub, format): a multisampled 2D texture's
/// subresource into one of a single-sample one of its size, of formats of one typeless group
/// (Windows does nothing with any other; `format` is a typed one of the group).
unsafe fn resolve(d: &mut D11, dst: u32, dsub: u32, src: u32, ssub: u32, fmt: u32) {
    let (di, si) = (d.id_of(dst), d.id_of(src));
    let (Some(Data::Texture(a)), Some(Data::Texture(b))) = (d.objs.get(&di).map(|o| &o.data), d.objs.get(&si).map(|o| &o.data)) else { return };
    let (dl, dly) = (dsub % a.mips.max(1), dsub / a.mips.max(1));
    let ok = a.dim == 2
        && b.dim == 2
        && a.samples.0 == 1
        && b.samples.0 > 1
        && dly < a.layers
        && ssub < b.layers
        && ((a.w >> dl).max(1), (a.h >> dl).max(1)) == (b.w, b.h)
        && format::copy_compatible(a.fmt, b.fmt)
        && format::copy_compatible(fmt, b.fmt);
    if !ok {
        return;
    }
    let cmd = [gpu::RESOLVE, a.gpu, dl, dly, b.gpu, ssub];
    gpu::record(|q| q.cmd(&cmd));
    d.targets = None;
}

/// CopySubresourceRegion(dst, dst sub, x, y, z, src, src sub, box)
unsafe fn copy_region(d: &mut D11, dst: u32, dsub: u32, x: u32, y: u32, z: u32, src: u32, ssub: u32, bx: u32) {
    let (di, si) = (d.id_of(dst), d.id_of(src));
    texture_written(d, di);
    expand::pull(d, si);
    expand::pull(d, di);
    match (d.objs.get(&di).map(|o| &o.data), d.objs.get(&si).map(|o| &o.data)) {
        (Some(Data::Buffer(a)), Some(Data::Buffer(b))) => {
            let (s0, s1) = if bx != 0 { (rd(bx, 0), rd(bx, 3)) } else { (0, b.size) };
            let n = s1.min(b.size).saturating_sub(s0).min(a.size.saturating_sub(x));
            std::ptr::copy(gc::<u8>(b.mem + s0), gp::<u8>(a.mem + x), us(n));
            if let Some(Data::Buffer(a)) = d.objs.get_mut(&di).map(|o| &mut o.data) {
                a.dirty = true;
                a.stream = false;
            }
        }
        (Some(Data::Texture(_)), Some(Data::Texture(_))) => {
            let b = (bx != 0).then(|| [rd(bx, 0), rd(bx, 1), rd(bx, 2), rd(bx, 3), rd(bx, 4), rd(bx, 5)]);
            copy_sub(d, di, dsub, [x, y, z], si, ssub, b);
        }
        _ => {}
    }
}

/// Copies a box of subresource `ssub` of texture `si` (all of it without one) to `at` in
/// subresource `dsub` of `di`: the bits, between formats of one typeless group (Windows copies
/// nothing between others, nor a box reaching outside either subresource). On the page where both
/// textures' texels are the guest's bytes, or both are the same conversion of them; else through
/// the bytes (those kept, or read back), uploaded as the destination converts them.
/// Whether `n` units from `at` end by `end` (without wrapping: a box at 0xffff_fff0 does not fit).
fn fits(at: u32, n: u32, end: u32) -> bool {
    at.checked_add(n).is_some_and(|e| e <= end)
}

unsafe fn copy_sub(d: &mut D11, di: u32, dsub: u32, at: [u32; 3], si: u32, ssub: u32, bx: Option<[u32; 6]>) {
    let (Some(Data::Texture(a)), Some(Data::Texture(b))) = (d.objs.get(&di).map(|o| &o.data), d.objs.get(&si).map(|o| &o.data)) else { return };
    if !format::copy_compatible(a.fmt, b.fmt) || dsub >= a.mips * if a.dim == 3 { 1 } else { a.layers } || ssub >= b.mips * if b.dim == 3 { 1 } else { b.layers } {
        return;
    }
    let (Some(fa), Some(fb)) = (format::info(a.fmt), format::info(b.fmt)) else { return };
    let (fa_nv12, fb_nv12) = (a.fmt == check::NV12, b.fmt == check::NV12);
    let (sl, slayer, sw, sh, sd) = sub_size(b, ssub);
    let (dl, dlayer, dw, dh, dd) = sub_size(a, dsub);
    let [x0, y0, z0, x1, y1, z1] = bx.unwrap_or([0, 0, 0, sw, sh, sd]);
    let (z0, z1) = if b.dim == 3 { (z0, z1) } else { (0, 1) };
    let dz = if a.dim == 3 { at[2] } else { 0 };
    // (block-compressed boxes: whole blocks, the last ones past a level's edge)
    let pad = |v: u32, f: &format::Format| v.div_ceil(f.block) * f.block;
    let (w, h, depth) = (x1.saturating_sub(x0), y1.saturating_sub(y0), z1.saturating_sub(z0));
    if w == 0 || h == 0 || depth == 0 || x1 > pad(sw, &fb) || y1 > pad(sh, &fb) || z1 > sd || !fits(at[0], w, pad(dw, &fa)) || !fits(at[1], h, pad(dh, &fa)) || !fits(dz, depth, dd) {
        return;
    }
    // a depth-stencil or multisampled texture is copied whole subresources only (Windows: winref
    // d3d11copy's box of R32_UINT into D32_FLOAT leaves it as it was)
    let whole = at == [0, 0, 0] && [x0, y0, z0, x1, y1, z1] == [0, 0, 0, sw, sh, sd] && (sw, sh, sd) == (dw, dh, dd);
    if ((a.bind | b.bind) & 0x40 != 0 || a.samples.0 > 1 || b.samples.0 > 1) && !whole {
        return;
    }
    // NV12 (to NV12: copy_compatible): a box on even luma texels; the luma plane on the page,
    // both in the bytes (the chroma texture follows them)
    if fa_nv12 || fb_nv12 {
        if (x0 | y0 | at[0] | at[1] | w | h) % 2 != 0 {
            return;
        }
        nv12_pull(d, si);
        nv12_pull(d, di);
        texture_written(d, di);
        let Some(Data::Texture(b)) = d.objs.get(&si).map(|o| &o.data) else { return };
        let bytes = nv12_box(b, ssub, [x0, y0, w, h]);
        let (ag, bg) = (match d.objs.get(&di).map(|o| &o.data) {
            Some(Data::Texture(a)) => a.gpu,
            _ => 0,
        }, b.gpu);
        let c = [gpu::COPY, bg, slayer << 16, x0, y0, w, h, ag, dlayer << 16, at[0], at[1], w, h, 0];
        gpu::record(|q| q.cmd(&c));
        if let Some(Data::Texture(a)) = d.objs.get_mut(&di).map(|o| &mut o.data) {
            nv12_keep(a, dsub, bytes.as_ptr().expose_provenance(), w, [at[0], at[1], w, h]);
        }
        d.targets = None;
        return;
    }
    texture_written(d, di);
    let (Some(Data::Texture(a)), Some(Data::Texture(b))) = (d.objs.get(&di).map(|o| &o.data), d.objs.get(&si).map(|o| &o.data)) else { return };
    let bits = if a.fmt != b.fmt { 2 } else { 0 };
    // (the page's texels of both the same function of the guest's bytes; 32-bit float RGBA
    // possibly halves, of another size than the integer ones)
    let f32x4 = |t: &Texture| t.page == format::P_RGBA32F;
    let on_page = (holds_bits(a) && holds_bits(b) && f32x4(a) == f32x4(b)) || (a.page == b.page && fa.conv == fb.conv && fa.bpp == fb.bpp && fa.block == fb.block && fa.no_alpha == fb.no_alpha);
    let (ag, bg, keeps, bfmt) = (a.gpu, b.gpu, keeps_bits(a), b.fmt);
    let gpu_copy = |d: &mut D11| {
        // (the page's boxes inside its levels)
        let (cw, ch) = (w.min(sw.saturating_sub(x0)).min(dw.saturating_sub(at[0])), h.min(sh.saturating_sub(y0)).min(dh.saturating_sub(at[1])));
        let c = [gpu::COPY, bg, sl | (slayer + z0) << 16, x0, y0, cw, ch, ag, dl | (dlayer + dz) << 16, at[0], at[1], cw, ch, bits | depth << 16];
        gpu::record(|q| q.cmd(&c));
        d.targets = None;
    };
    if on_page {
        gpu_copy(d);
        if !keeps {
            return;
        }
    }
    // the source's bytes of the box (kept ones, or the page's read back): the destination's kept
    // ones, and its page texels as it converts them
    let (pitch, rows) = format::pitch_rows(bfmt, w, h);
    let Some(mem) = source_bytes(d, si, ssub, [x0, y0, z0, w, h, depth]) else {
        if !on_page {
            gpu_copy(d);
        }
        return;
    };
    if let Some(Data::Texture(a)) = d.objs.get_mut(&di).map(|o| &mut o.data) {
        if on_page {
            keep_bits(a, dsub, us(mem), pitch, pitch * rows, [at[0], at[1], dz, w, h, dz + depth]);
        } else {
            upload_texture(a, dsub, us(mem), pitch, pitch * rows, Some([at[0], at[1], dz, at[0] + w, at[1] + h, dz + depth]));
        }
    }
    ((*d.calls).free)(gp::<u8>(mem));
    d.targets = None;
}

/// A box (x, y, z, width, height, slices; x and y on blocks) of a texture's subresource as the
/// guest's bytes (rows of `format::pitch_rows`, slice after slice) in memory from the host's
/// allocator: those kept, or the page's read back where they are its texels. None: neither.
unsafe fn source_bytes(d: &mut D11, id: u32, sub: u32, b: [u32; 6]) -> Option<u32> {
    let Some(Data::Texture(t)) = d.objs.get(&id).map(|o| &o.data) else { return None };
    let f = format::info(t.fmt)?;
    let [x0, y0, z0, w, h, depth] = b;
    let (keeps, holds, g) = (keeps_bits(t), holds_bits(t), t.gpu);
    // (a render target of a format the page converts: its texels read back and packed again)
    let packs = !keeps && !holds && format::packs_back(t.fmt);
    if !keeps && !holds && !packs {
        return None;
    }
    let (pitch, rows) = format::pitch_rows(t.fmt, w, h);
    // (in 64 bits; past what the allocator gives, or nothing from it: none)
    let size = u32::try_from(u64::from(pitch) * u64::from(rows) * u64::from(depth)).ok().filter(|&n| n < 0x8000_0000)?;
    let mem = lo32((((*d.calls).alloc)(size.max(16))).expose_provenance());
    if mem == 0 {
        return None;
    }
    // (what nothing wrote, and what a readback leaves alone: zeros)
    std::ptr::write_bytes(gp::<u8>(mem), 0, us(size));
    let (level, layer, lw, lh, _) = sub_size(t, sub);
    if keeps {
        let (spitch, srows) = format::pitch_rows(t.fmt, lw, lh);
        let (bx, by) = (x0 / f.block, y0 / f.block);
        if let Some(kept) = t.bits.get(&sub) {
            for z in 0..depth {
                for r in 0..rows {
                    let s0 = us(((z0 + z) * srows + by + r) * spitch + bx * f.bpp);
                    std::ptr::copy_nonoverlapping(kept.as_ptr().add(s0), gp::<u8>(mem + (z * rows + r) * pitch), us(pitch));
                }
            }
        }
        return Some(mem);
    }
    if packs {
        let pp = w * format::page_bpp(t.page);
        let Some(n) = u32::try_from(u64::from(pp) * u64::from(h)).ok().filter(|&n| n < 0x8000_0000) else {
            ((*d.calls).free)(gp::<u8>(mem));
            return None;
        };
        let tmp = lo32((((*d.calls).alloc)(n.max(16))).expose_provenance());
        if tmp == 0 {
            ((*d.calls).free)(gp::<u8>(mem));
            return None;
        }
        for z in 0..depth {
            std::ptr::write_bytes(gp::<u8>(tmp), 0, us(n));
            d.next_gpu += 1;
            let req = 0x4000_0000 | d.next_gpu;
            let c = [gpu::READBACK, req, g, level | (layer + z0 + z) << 16, x0, y0, w, h, tmp, pp];
            gpu::record(|q| q.cmd(&c));
            gpu::wait_readback(req);
            let page = std::slice::from_raw_parts(gc::<u8>(tmp), us(n));
            if let Some(b) = format::from_page(t.fmt, page, w, h) {
                let row = us(w * f.bpp);
                for r in 0..rows.min(h) {
                    std::ptr::copy_nonoverlapping(b.as_ptr().add(us(r) * row), gp::<u8>(mem + (z * rows + r) * pitch), row.min(us(pitch)));
                }
            }
        }
        ((*d.calls).free)(gp::<u8>(tmp));
        return Some(mem);
    }
    let mut reqs = Vec::new();
    for z in 0..depth {
        d.next_gpu += 1;
        let req = 0x4000_0000 | d.next_gpu;
        let c = [gpu::READBACK, req, g, level | (layer + z0 + z) << 16, x0, y0, w, h, mem + z * pitch * rows, pitch];
        gpu::record(|q| q.cmd(&c));
        reqs.push(req);
    }
    for req in reqs {
        gpu::wait_readback(req);
    }
    Some(mem)
}

// ---------------------------------------------------------------------------------------------
// draws

/// A shader's page shader and bindings (translated on first use; vertex shaders per pixel shader).
/// `samplers`: a pixel shader's (those with BORDER or MIRROR_ONCE addressing: the shader does it)
fn shader_for(d: &mut D11, sh: u32, ps: u32, sprite: Option<f32>, samplers: &[u32]) -> Option<(u32, Vec<(u32, dxwgsl::Binding)>)> {
    let mut addr = Vec::new();
    if matches!(d.objs.get(&sh).map(|o| &o.data), Some(Data::Shader(s)) if s.pixel) {
        for (n, &id) in samplers.iter().enumerate() {
            if let Some(desc) = state_desc(d, id) {
                let flags = dxwgsl::addr::flags(desc[1], desc[2], desc[0] & 4 != 0 || desc[0] & 0x40 != 0);
                if flags != 0 {
                    addr.push(dxwgsl::addr::Addr { sampler: lo32(n), flags, border: [desc[7], desc[8], desc[9], desc[10]] });
                }
            }
        }
    }
    // (point sprite variants of a vertex shader: the pixel shader's id with the top bit set)
    let Some(Data::Shader(s)) = d.objs.get(&sh).map(|o| &o.data) else { return None };
    let key = (if s.pixel { 0 } else if sprite.is_some() { ps | 0x8000_0000 } else { ps }, addr);
    if let Some(r) = s.wgsl.get(&key) {
        return r.clone();
    }
    // (the pixel shader's code copied only for a translation, not on every draw: review b02)
    let ps_code = match d.objs.get(&ps).map(|o| &o.data) {
        Some(Data::Shader(s)) => Some(s.code.clone()),
        _ => None,
    };
    let trace = d.trace;
    let Some(Data::Shader(s)) = d.objs.get_mut(&sh).map(|o| &mut o.data) else { return None };
    let w = match sprite {
        Some(max) if !s.pixel => dxwgsl::translate_point_sprite(&s.code, ps_code.as_deref(), max),
        _ => dxwgsl::translate_addr(&s.code, s.pixel, if s.pixel { None } else { ps_code.as_deref() }, &key.1),
    };
    let r = match w {
        Ok(w) => {
            let id = page_id();
            gpu::record(|q| {
                let off = q.blob(w.code.as_bytes());
                q.cmd(&[gpu::CREATE_SHADER, id, off, lo32(w.code.len())]);
            });
            if trace {
                eprintln!("d3d11: {} shader {sh} -> {} bindings {:?}", if s.pixel { "pixel" } else { "vertex" }, id, w.bindings);
                if trace2() {
                    eprintln!("{}", w.code);
                }
            }
            Some((id, w.bindings))
        }
        Err(e) => {
            eprintln!("d3d11: shader {sh}: {e}");
            None
        }
    };
    s.wgsl.insert(key, r.clone());
    r
}

fn state_desc(d: &D11, id: u32) -> Option<&Vec<u32>> {
    match d.objs.get(&id).map(|o| &o.data) {
        Some(Data::State(s)) => Some(s),
        _ => None,
    }
}

/// D3D11_BLEND to the pipeline key's blend factor (color; `alpha`: as an alpha factor)
fn blend_factor(b: u32, alpha: bool) -> Factor {
    match b {
        1 => Factor::Zero,
        2 => Factor::One,
        3 => if alpha { Factor::SrcAlpha } else { Factor::Src },
        4 => if alpha { Factor::OneMinusSrcAlpha } else { Factor::OneMinusSrc },
        5 => Factor::SrcAlpha,
        6 => Factor::OneMinusSrcAlpha,
        7 => Factor::DstAlpha,
        8 => Factor::OneMinusDstAlpha,
        9 => if alpha { Factor::DstAlpha } else { Factor::Dst },
        10 => if alpha { Factor::OneMinusDstAlpha } else { Factor::OneMinusDst },
        11 => Factor::SrcAlphaSaturated,
        14 => Factor::Constant,
        15 => Factor::OneMinusConstant,
        _ => Factor::One,
    }
}

/// The page buffer with `id`'s current contents (made when the guest wrote it).
fn buffer_gpu(d: &mut D11, id: u32) -> u32 {
    let Some(Data::Buffer(b)) = d.objs.get_mut(&id).map(|o| &mut o.data) else { return 0 };
    if b.dirty || b.gpu == 0 {
        // (written in place: the page moves it to a new GPU buffer when draws before use it)
        let made = b.gpu;
        let g = if made != 0 { made } else { page_id() };
        let (mem, size) = (b.mem, b.size);
        b.gpu = g;
        b.dirty = false;
        let bytes = unsafe { std::slice::from_raw_parts(gc::<u8>(mem), us(size)) };
        if made != 0 && b.stream && b.shadow.len() == bytes.len() {
            // (ANGLE streams each draw's vertices into a big dynamic buffer: uploading all of it
            // per draw cost Godot seconds)
            b.stream = false;
            if let Some((lo, hi)) = changed_range(&b.shadow, bytes) {
                b.shadow[lo..hi].copy_from_slice(&bytes[lo..hi]);
                gpu::record(|q| {
                    let off = q.blob(&bytes[lo..hi]);
                    q.cmd(&[gpu::WRITE_BUFFER, g, lo32(lo) | gpu::WRITE_UNUSED, off, lo32(hi - lo)]);
                });
            }
            return g;
        }
        b.stream = false;
        if b.usage == 2 {
            b.shadow.clear();
            b.shadow.extend_from_slice(bytes);
        }
        gpu::record(|q| {
            let off = q.blob(bytes);
            if made == 0 {
                q.cmd(&[gpu::CREATE_BUFFER, g, size.next_multiple_of(16).max(16)]);
            }
            q.cmd(&[gpu::WRITE_BUFFER, g, 0, off, size]);
        });
    }
    b.gpu
}

/// The `want` bytes a shader sees through a range of `len` bytes whose words `word(i)` reads:
/// the range's bytes, then zeros (a constant buffer's ByteWidth is a multiple of 16: whole
/// words).
fn range_copy(word: impl Fn(u32) -> u32, len: u32, want: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(us(want));
    for i in 0..len.min(want) >> 2 {
        bytes.extend_from_slice(&word(i).to_le_bytes());
    }
    bytes.resize(us(want), 0);
    bytes
}

/// The page buffer a shader reads constant buffer `cb` through, bound with `range`, the shader
/// declaring `size` bytes: (page buffer, bytes bound). The whole buffer from its start when
/// that is what the shader sees; else a copy of the range's bytes, cut at the buffer's end
/// and padded with zeros to `size` (constants past the bound range read as 0: D3D11.1
/// functional spec, ConstantBufferOffsetting; winref d3d11cb1).
fn cb_binding(d: &mut D11, cb: u32, range: CbRange, size: u32) -> (u32, u32) {
    let Some(Data::Buffer(b)) = d.objs.get(&cb).map(|o| &o.data) else { return (0, 0) };
    let full = b.size.next_multiple_of(16).max(16);
    let (off, len) = range.bytes(b.size);
    let want = size.next_multiple_of(16).max(16);
    if off == 0 && len == b.size.min(65536) && full >= want {
        return (buffer_gpu(d, cb), full.min(65536));
    }
    let mem = b.mem;
    // SAFETY: `mem` holds the buffer's `b.size` bytes, and range_copy reads words below
    // off + len <= b.size (CbRange::bytes)
    let bytes = range_copy(|i| unsafe { rd(mem.wrapping_add(off), us(i)) }, len, want);
    let key = (cb, range.first, want);
    let (made, same) = match d.cb_copies.get(&key) {
        Some((g, last)) => (*g, *last == bytes),
        None => (0, false),
    };
    let g = if made != 0 { made } else { page_id() };
    if !same {
        gpu::record(|q| {
            let at = q.blob(&bytes);
            if made == 0 {
                q.cmd(&[gpu::CREATE_BUFFER, g, want]);
            }
            q.cmd(&[gpu::WRITE_BUFFER, g, 0, at, want]);
        });
        d.cb_copies.insert(key, (g, bytes));
    }
    (g, want)
}

/// The bytes where `new` differs from `old` (same length), widened to whole 4-byte words (as
/// writeBuffer writes them; the end may be `new`'s length); None when equal.
pub(crate) fn changed_range(old: &[u8], new: &[u8]) -> Option<(usize, usize)> {
    let n = new.len().min(old.len());
    // (8 bytes at a time from each end, as words: a slice compare is a memcmp call, and this
    // runs over a megabyte a draw)
    let word = |b: &[u8], at: usize| b.get(at..).and_then(<[u8]>::first_chunk::<8>).map_or(0, |w| u64::from_ne_bytes(*w));
    let mut lo = 0;
    while lo + 8 <= n && word(old, lo) == word(new, lo) {
        lo += 8;
    }
    while lo < n && old[lo] == new[lo] {
        lo += 1;
    }
    if lo == n {
        return None;
    }
    let mut hi = n;
    while hi >= lo + 8 && word(old, hi - 8) == word(new, hi - 8) {
        hi -= 8;
    }
    while old[hi - 1] == new[hi - 1] {
        hi -= 1;
    }
    Some((lo & !3, hi.next_multiple_of(4).min(n)))
}

/// WebGPU has 8 vertex buffers, D3D11 16 slots, and ANGLE gives every GL attribute its own
/// slot, often on one buffer. When a draw uses more than 8, slots on the same buffer with the
/// same stride and step become one (offsets moved into the attributes) where the attributes
/// still fit in a vertex. `slots`: (slot, step, attributes [format, offset, location]); the
/// result: (buffer, stride, offset) per slot index used in `slots` afterwards.
fn merge_vertex_buffers(vbs: &[(u32, u32, u32); 32], slots: &mut Vec<(u32, u32, Vec<[u32; 3]>)>) -> Vec<(u32, u32, u32)> {
    let mut vbx = vbs.to_vec();
    if slots.len() <= 8 {
        return vbx;
    }
    let mut out: Vec<(u32, u32, Vec<[u32; 3]>)> = Vec::new();
    for (slot, step, attrs) in std::mem::take(slots) {
        let (buf, stride, off) = vbs[us(slot)];
        let limit = if stride == 0 { 2048 } else { stride };
        let fits = |base: u32, list: &[[u32; 3]], from: u32| list.iter().all(|a| from - base + a[1] + vertex_size(a[0]) <= limit);
        let into = out.iter().position(|(s2, st2, a2)| {
            let (b2, str2, o2) = vbx[us(*s2)];
            if b2 != buf || str2 != stride || *st2 != step || buf == 0 {
                return false;
            }
            let base = o2.min(off);
            fits(base, a2, o2) && fits(base, &attrs, off)
        });
        match into {
            Some(i) => {
                let s2 = us(out[i].0);
                let o2 = vbx[s2].2;
                let base = o2.min(off);
                for a in &mut out[i].2 {
                    a[1] += o2 - base;
                }
                out[i].2.extend(attrs.into_iter().map(|a| [a[0], a[1] + off - base, a[2]]));
                vbx[s2].2 = base;
            }
            None => out.push((slot, step, attrs)),
        }
    }
    *slots = out;
    vbx
}

#[allow(clippy::too_many_arguments)]
/// The edges of a draw's triangles (list, or strip with `strip`) as line-list indices: each
/// triangle's a-b, b-c, c-a, of its index buffer's indices (a strip's restart index cuts it)
/// or of the vertex numbers. None when the indices are out of the buffer.
unsafe fn wire_edges(d: &D11, st: &State, indexed: bool, count: u32, start: u32, strip: bool) -> Option<Vec<u32>> {
    let idx: Vec<u32> = if indexed {
        let isz = if st.ib.1 == 42 { 4u64 } else { 2 };
        let Some(Data::Buffer(b)) = d.objs.get(&st.ib.0).map(|o| &o.data) else { return None };
        if u64::from(st.ib.2) + (u64::from(start) + u64::from(count)) * isz > u64::from(b.size) {
            return None;
        }
        let p = b.mem + st.ib.2 + start * lo32(isz);
        (0..us(count)).map(|k| if isz == 4 { rd(p, k) } else { u32::from(*gc::<u16>(p).add(k)) }).collect()
    } else {
        (start..start.saturating_add(count)).collect()
    };
    let restart = if !indexed { None } else if st.ib.1 == 42 { Some(u32::MAX) } else { Some(0xffff) };
    Some(triangle_edges(&idx, strip, restart))
}

/// See `wire_edges`: the edges of the triangles of `idx`.
fn triangle_edges(idx: &[u32], strip: bool, restart: Option<u32>) -> Vec<u32> {
    let mut e = Vec::new();
    let mut tri = |a: u32, b: u32, c: u32| e.extend_from_slice(&[a, b, b, c, c, a]);
    if strip {
        for run in idx.split(|&v| Some(v) == restart) {
            for t in run.windows(3) {
                tri(t[0], t[1], t[2]);
            }
        }
    } else {
        for t in idx.chunks_exact(3) {
            tri(t[0], t[1], t[2]);
        }
    }
    e
}

unsafe fn draw(d: &mut D11, st: &State, indexed: bool, count: u32, instances: u32, start: u32, base: i32, first_instance: u32) {
    if count == 0 || instances == 0 || st.vs == 0 {
        return;
    }
    // (a geometry shader's invocations: one per input primitive and GS instance)
    let gs_instances = match d.objs.get(&st.shaders[0]).map(|o| &o.data) {
        Some(Data::Shader(s)) => dxwgsl::expand::gs_decl(&s.code).ok().map(|g| g.instances.max(1)),
        _ => None,
    };
    query::count_draw(&mut d.stats, st.topology, count, instances, gs_instances, st.shaders[1] != 0 && st.shaders[2] != 0);
    // GL_POINTS through ANGLE: its point sprite geometry shader, done as an instance per point
    // (non-indexed, non-instanced point lists; see dxwgsl::translate_point_sprite), not
    // expanded
    let sprite = match d.objs.get(&st.shaders[0]).map(|o| &o.data) {
        Some(Data::Shader(s)) if st.topology == 1 && !indexed && instances <= 1 => dxwgsl::point_sprite_gs(&s.code),
        _ => None,
    };
    if sprite.is_none() {
        // SAFETY: draw's own contract (the bound buffers' guest memory is mapped)
        match unsafe { expand::run(d, st, indexed, count, instances, start, base, first_instance) } {
            expand::After::Done => return,
            expand::After::Draw => {}
        }
    }
    // SAFETY: draw's own contract (the bound buffers' guest memory is mapped)
    unsafe { draw_with(d, st, indexed, count, instances, start, base, first_instance, sprite, None) };
}

/// A draw of the render pipeline: the bound vertex shader's, or (`store`) a store's of an
/// expanded draw (expand.rs: its own vertex shader, no vertex buffers).
#[allow(clippy::too_many_arguments)]
unsafe fn draw_with(d: &mut D11, st: &State, indexed: bool, count: u32, instances: u32, start: u32, base: i32, first_instance: u32, sprite: Option<f32>, store: Option<&expand::StoreDraw>) {
    let (count, instances, start, first_instance) = if sprite.is_some() { (6, count, 0, start) } else { (count, instances, start, first_instance) };
    let shaders = match store {
        Some(sd) => Some(sd.vs.clone()),
        None => shader_for(d, st.vs, st.ps, sprite, &[]),
    };
    let Some((vs, vb)) = shaders else { return };
    let ps = if st.ps != 0 { shader_for(d, st.ps, 0, None, &st.samplers[1]) } else { None };
    if trace2() {
        // the first vertices and the vertex shader's constants
        let words = |id: u32, from: u32, n: u32| -> Vec<String> {
            match d.objs.get(&id).map(|o| &o.data) {
                Some(Data::Buffer(b)) => (0..n).filter(|i| from + i * 4 + 4 <= b.size).map(|i| format!("{:08x}", rd(b.mem + from, us(i)))).collect(),
                _ => Vec::new(),
            }
        };
        let (vb0, stride, off) = st.vbs[0];
        eprintln!("d3d11: draw {count} from {start} vb stride {stride} offset {off}: {}", words(vb0, off + start * stride, (stride / 4) * 2).join(" "));
        for (i, &c) in st.cbs[0].iter().enumerate().filter(|c| *c.1 != 0).take(4) {
            eprintln!("d3d11:   vs cb{i}: {}", words(c, 0, 16).join(" "));
        }
    }
    // targets (NV12 planes among them: see nv12_target)
    for &v in st.rtvs.iter().chain(&st.uavs[0]) {
        if v != 0 {
            nv12_target(d, v);
        }
    }
    let mut colors = Vec::new();
    let mut formats = Vec::new();
    for &v in &st.rtvs {
        if let Some((tex, level, layer)) = view_target(d, v) {
            let page = match d.objs.get(&v).map(|o| &o.data) {
                Some(Data::View(vv)) => match d.objs.get(&vv.res).map(|o| &o.data) {
                    Some(Data::Texture(t)) => t.page,
                    _ => 0,
                },
                _ => 0,
            };
            colors.extend_from_slice(&[tex, level, layer]);
            formats.push(page);
        } else {
            break;
        }
    }
    let depth = view_target(d, st.dsv);
    let depth_fmt = depth.and_then(|_| match d.objs.get(&st.dsv).map(|o| &o.data) {
        Some(Data::View(v)) => match d.objs.get(&v.res).map(|o| &o.data) {
            Some(Data::Texture(t)) => Some(t.page),
            _ => None,
        },
        _ => None,
    });
    if colors.is_empty() && depth.is_none() {
        return;
    }
    let mut tw = vec![lo32(colors.len() / 3)];
    tw.extend_from_slice(&colors);
    let (dt, dl, dly) = depth.unwrap_or((0, 0, 0));
    tw.extend_from_slice(&[dt, dl, dly]);
    let generation = gpu::batch_gen();
    if d.targets.as_ref().is_none_or(|(g, t)| *g != generation || *t != tw) {
        let mut c = vec![gpu::SET_TARGETS2];
        c.extend_from_slice(&tw);
        gpu::record(|q| q.cmd(&c));
        d.targets = Some((generation, tw));
    }
    // pipeline
    let layout = match d.objs.get(&st.layout).map(|o| &o.data) {
        Some(Data::Layout(l)) => l.iter().map(|e| (e.name.clone(), e.index, e.fmt, e.slot, e.offset, e.instance)).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let vs_code = match d.objs.get(&st.vs).map(|o| &o.data) {
        Some(Data::Shader(s)) => s.code.clone(),
        _ => return,
    };
    // (a store's vertex shader reads no vertex buffers)
    let inputs = if store.is_some() { Vec::new() } else { dxwgsl::signature(&vs_code, false) };
    // vertex buffers used: slot -> (attributes)
    let mut slots: Vec<(u32, u32, Vec<[u32; 3]>)> = Vec::new(); // (slot, step, attrs: format offset location)
    for inp in &inputs {
        if inp.sysval != 0 {
            continue;
        }
        let Some(e) = layout.iter().find(|e| e.0.eq_ignore_ascii_case(inp.name()) && e.1 == inp.index) else { continue };
        let Some(vf) = format::vertex_format(e.2) else { continue };
        let i = match slots.iter().position(|s| s.0 == e.3) {
            Some(i) => i,
            None => {
                slots.push((e.3, if sprite.is_some() { 1 } else { u32::from(e.5) }, Vec::new()));
                slots.len() - 1
            }
        };
        slots[i].2.push([vf, e.4, inp.reg]);
    }
    slots.sort_by_key(|s| s.0);
    // the vertex buffers the draw reads: (buffer, stride, offset) by slot
    let vbx = merge_vertex_buffers(&st.vbs, &mut slots);
    if slots.len() > 8 {
        // (more than WebGPU's 8: an invalid pipeline would lose the whole frame)
        return;
    }
    let blend = state_desc(d, st.blend).cloned();
    let ds = state_desc(d, st.depth).cloned();
    let rs = state_desc(d, st.raster).cloned();
    let topo = match st.topology {
        _ if sprite.is_some() => 3,
        _ if store.is_some() => store.map_or(3, |sd| sd.topo),
        1 => 0,
        2 => 1,
        3 => 2,
        4 => 3,
        5 => 4,
        _ => 3,
    };
    let r = rs.unwrap_or_else(|| vec![3, 3, 0, 0, 0, 0, 1, 0, 0, 0]);
    // FillMode WIREFRAME (2) of triangles: their edges as a line list of its own indices (#64)
    let wire = if r.first() == Some(&2) && matches!(topo, 3 | 4) && sprite.is_none() && store.is_none() {
        if indexed {
            expand::pull(d, st.ib.0);
        }
        // SAFETY: draw_with's own contract (the index buffer's guest memory is mapped)
        unsafe { wire_edges(d, st, indexed, count, start, topo == 4) }
    } else { None };
    let topo = if wire.is_some() { 1 } else { topo };
    let strip = if wire.is_none() && indexed && matches!(topo, 2 | 4) { if st.ib.1 == 42 { 2 } else { 1 } } else { 0 };
    let mut key = vec![vs, ps.as_ref().map_or(0, |p| p.0), topo, strip];
    // rasterizer: cull, front ccw, depth bias, slope, clamp
    // (front ccw in bit 0, DepthClipEnable off in bit 1)
    key.extend_from_slice(&[r[1], u32::from(r[2] != 0) | u32::from(r[6] == 0) << 1, r[3], r[5], r[4]]);
    // depth stencil
    match (depth_fmt, &ds) {
        (Some(f), Some(s)) => {
            key.extend_from_slice(&[f, s[0], s[1], s[2], s[3], s[4] & 0xffff]);
            key.extend_from_slice(&s[5..13]);
        }
        (Some(f), None) => key.extend_from_slice(&[f, 1, 1, 2, 0, 0xffff, 1, 1, 1, 8, 1, 1, 1, 8]),
        _ => key.push(0xffff),
    }
    // vertex buffers
    key.push(lo32(slots.len()));
    for s in &slots {
        key.extend_from_slice(&[vbx[us(s.0)].1, s.1, lo32(s.2.len())]);
        for at in &s.2 {
            key.extend_from_slice(at);
        }
    }
    // targets
    let b = blend.unwrap_or_else(|| {
        let mut v = vec![0, 0];
        for _ in 0..8 {
            v.extend_from_slice(&[0, 2, 1, 1, 2, 1, 1, 0xf]);
        }
        v
    });
    key.push(lo32(formats.len()));
    for (i, f) in formats.iter().enumerate() {
        let rt = if b[1] != 0 { i } else { 0 };
        let t = &b[2 + rt * 8..2 + rt * 8 + 8];
        key.extend_from_slice(&[*f, t[0], blend_factor(t[1], false).code(), blend_factor(t[2], false).code(), t[3], blend_factor(t[4], true).code(), blend_factor(t[5], true).code(), t[6], t[7] & 0xf]);
    }
    // multisampling: AlphaToCoverageEnable, the sample mask (render::d3d pipeline2)
    key.extend_from_slice(&[b[0], st.sample_mask]);
    let pipe = match d.pipelines.get(&key) {
        Some(&p) => {
            crate::render::gpu::pipeline_hit();
            p
        }
        None => {
            let id = page_id();
            let mut c = vec![gpu::CREATE_PIPELINE2, id, lo32(key.len())];
            c.extend_from_slice(&key);
            gpu::record(|q| q.cmd(&c));
            d.pipelines.insert(key, id);
            id
        }
    };
    // draw
    let mut w = vec![gpu::DRAW2, pipe];
    w.extend(st.viewport[0].iter().map(|f| f.to_bits()));
    let scissor_on = r.get(7).copied().unwrap_or(0) != 0;
    if scissor_on {
        let s = st.scissor[0];
        w.extend_from_slice(&[(s[0].max(0)).cast_unsigned(), (s[1].max(0)).cast_unsigned(), (s[2] - s[0]).max(0).cast_unsigned(), (s[3] - s[1]).max(0).cast_unsigned(), 1]);
    } else {
        w.extend_from_slice(&[0, 0, 0, 0, 0]);
    }
    w.extend(st.blend_factor.iter().map(|f| f.to_bits()));
    w.push(st.stencil_ref);
    w.push(lo32(slots.len()));
    let vbs: Vec<(u32, u32)> = slots.iter().map(|s| (vbx[us(s.0)].0, vbx[us(s.0)].2)).collect();
    for (buf, off) in vbs {
        let g = buffer_gpu(d, buf);
        w.extend_from_slice(&[g, off]);
    }
    let wire_buf = wire.as_ref().map(|e| {
        let g = page_id();
        let bytes: Vec<u8> = e.iter().flat_map(|v| v.to_le_bytes()).collect();
        gpu::record(|q| {
            let off = q.blob(&bytes);
            q.cmd(&[gpu::CREATE_BUFFER, g, lo32(bytes.len())]);
            q.cmd(&[gpu::WRITE_BUFFER, g, 0, off, lo32(bytes.len())]);
        });
        g
    });
    if let Some(g) = wire_buf {
        w.extend_from_slice(&[g, 0, 2]);
    } else if indexed {
        let g = buffer_gpu(d, st.ib.0);
        w.extend_from_slice(&[g, st.ib.2, if st.ib.1 == 42 { 2 } else { 1 }]);
    } else {
        w.extend_from_slice(&[0, 0, 0]);
    }
    // bindings: group 0 vertex, 1 pixel
    let mut cbs_read = Vec::new();
    let mut over: Vec<[u32; 8]> = store.map_or_default(|sd| sd.over.clone());
    if sprite.is_some() {
        over.push(sprite_entry(d, st));
    }
    for (stage, binds) in [(0usize, &vb), (1usize, &ps.as_ref().map_or_default(|p| p.1.clone()))] {
        let entries = bind_entries(d, st, stage, binds, if stage == 0 { &over } else { &[] }, &mut cbs_read);
        w.push(lo32(entries.len()));
        for e in entries {
            w.extend_from_slice(&e);
        }
    }
    match &wire {
        // (the edges hold the vertex numbers of a draw without indices: no base then)
        Some(e) => w.extend_from_slice(&[1, lo32(e.len()), instances, 0, if indexed { base.cast_unsigned() } else { 0 }, first_instance]),
        None => w.extend_from_slice(&[u32::from(indexed), count, instances, start, base.cast_unsigned(), first_instance]),
    }
    // What it reads from its buffers (vertices, indices, constants), hashed: the page tells a
    // frame that draws the same as the last one, though from another place in a ring buffer
    // and with other bytes written around it (0: unknown)
    // (a buffer stream output wrote: the page's bytes are not the guest's, unknown)
    let mem = |id: u32| match d.objs.get(&id).map(|o| &o.data) {
        Some(Data::Buffer(b)) if !b.gpu_written => Some((b.mem, b.size)),
        _ => None,
    };
    let mut known = count <= 4096 && store.is_none();
    let mut h = 0x811c_9dc5u32;
    let mut add = |p: u32, n: u32| {
        for &b in std::slice::from_raw_parts(gc::<u8>(p), us(n)) {
            h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
        }
    };
    let verts: Vec<u32> = if !known {
        Vec::new()
    } else if indexed {
        let isz = if st.ib.1 == 42 { 4 } else { 2 };
        match mem(st.ib.0) {
            Some((m, size)) if u64::from(st.ib.2) + (u64::from(start) + u64::from(count)) * isz <= u64::from(size) => {
                let p = m + st.ib.2 + start * lo32(isz);
                (0..us(count)).map(|k| if isz == 4 { rd(p, k) } else { u32::from(*gc::<u16>(p).add(k)) }.wrapping_add(base.cast_unsigned())).collect()
            }
            _ => {
                known = false;
                Vec::new()
            }
        }
    } else {
        (start..start.saturating_add(count)).collect()
    };
    for s in &slots {
        let (buf, stride, off) = vbx[us(s.0)];
        let Some((m, size)) = mem(buf) else {
            known = false;
            break;
        };
        let at = |v: u32| u64::from(off) + u64::from(v) * u64::from(stride);
        let vs: Vec<u32> = if s.1 != 0 { (first_instance..first_instance.saturating_add(instances)).collect() } else { verts.clone() };
        if stride == 0 || vs.iter().any(|&v| at(v) + u64::from(stride) > u64::from(size)) {
            known = false;
            break;
        }
        for v in vs {
            add(m + lo32(at(v)), stride);
        }
    }
    for cb in cbs_read {
        match mem(cb) {
            Some((m, size)) => add(m, size),
            None => {}
        }
    }
    // (a wireframe draw's edges are a buffer of its own, gone after it: never the same)
    w.push(if known && wire_buf.is_none() { h | 1 } else { 0 });
    gpu::record(|q| q.cmd(&w));
    if let Some(g) = wire_buf {
        gpu::record(|q| q.cmd(&[gpu::DESTROY_BUFFER, g]));
    }
}

/// The entries of bindings `binds` of a shader of stage `stage` (0 VS, 1 PS, 2 GS, ...: the
/// state's slots) for DRAW2 and DISPATCH; `over` the entries of bindings that are not the
/// state's (expanded draws' stores and uniforms, the point sprites' viewport), the constant
/// buffers read into `cbs_read`.
fn bind_entries(d: &mut D11, st: &State, stage: usize, binds: &[(u32, dxwgsl::Binding)], over: &[[u32; 8]], cbs_read: &mut Vec<u32>) -> Vec<[u32; 8]> {
    let mut entries: Vec<[u32; 8]> = Vec::new();
    for (bi, kind) in binds.iter() {
        if let Some(e) = over.iter().find(|e| e[0] == *bi) {
            entries.push(*e);
            continue;
        }
        match kind {
            dxwgsl::Binding::Uniform(size) => {
                let cb = st.cbs.get(stage).and_then(|c| c.get(us(*bi))).copied().unwrap_or(0);
                let range = st.cb_ranges.get(stage).and_then(|c| c.get(us(*bi))).copied().unwrap_or(CbRange::WHOLE);
                let (g, len) = if cb != 0 { cb_binding(d, cb, range, *size) } else { (0, 0) };
                cbs_read.push(cb);
                entries.push([*bi, 0, g, 0, len, *size, 0, 0]);
            }
            dxwgsl::Binding::Texture(vdim, sample) => {
                let t = bi.wrapping_sub(16);
                let sv = st.srvs.get(stage).and_then(|c| c.get(us(t))).copied().unwrap_or(0);
                buffer_view_sync(d, sv);
                let e = match d.objs.get(&sv).map(|o| &o.data) {
                    Some(Data::View(v)) => match d.objs.get(&v.res).map(|o| &o.data) {
                        Some(Data::Texture(tx)) => [*bi, 1, tx.gpu, *vdim, v.mip, v.mips.min(tx.mips.saturating_sub(v.mip.min(tx.mips.saturating_sub(1)))).max(1), v.layer, v.layers.max(1)],
                        _ => [*bi, 1, 0, *vdim, 0, 1, 0, 1],
                    },
                    _ => [*bi, 1, 0, *vdim, 0, 1, 0, 1],
                };
                let mut e = e;
                if *sample != 0 {
                    e[1] = sample.saturating_add(3); // uint/sint/depth textures
                }
                // (a stencil view, X32_TYPELESS_G8X24_UINT or X24_TYPELESS_G8_UINT: not the
                // page's float copy of the depth, which depth views read: unbound)
                if e[1] == 1 && matches!(d.objs.get(&sv).map(|o| &o.data), Some(Data::View(v)) if matches!(v.fmt, 22 | 47)) {
                    e[2] = 0;
                }
                entries.push(e);
            }
            dxwgsl::Binding::Sampler(cmp) => {
                let s = bi.wrapping_sub(160);
                let id = st.samplers.get(stage).and_then(|c| c.get(us(s))).copied().unwrap_or(0);
                let desc = state_desc(d, id).cloned().unwrap_or_else(|| vec![0x15, 3, 3, 3, 0, 1, 8, 0, 0, 0, 0, 0, 0x7f7f_ffff]);
                let key = sampler_key(&desc, *cmp);
                entries.push([*bi, if *cmp { 3 } else { 2 }, key, desc.get(11).copied().unwrap_or(0), desc.get(12).copied().unwrap_or(0x7f7f_ffff), 0, 0, 0]);
            }
            // (compute shaders only)
            dxwgsl::Binding::Storage(..) => {}
            // (a store of an expanded draw that `over` does not name: nothing, the page
            // binds zeros or runs nothing)
            dxwgsl::Binding::StorageBuffer(rw) => entries.push([*bi, if *rw { 10 } else { 9 }, 0, 0, 0, 0, 0, 0]),
        }
    }
    entries
}

/// The point sprites' viewport uniform (b15 of their vertex shader): half the viewport's size.
fn sprite_entry(d: &mut D11, st: &State) -> [u32; 8] {
    let half = [st.viewport[0][2] * 0.5, st.viewport[0][3] * 0.5];
    if d.sprite_view.0 == 0 || d.sprite_view.1 != half {
        let made = d.sprite_view.0;
        let g = if made != 0 { made } else { page_id() };
        d.sprite_view = (g, half);
        let mut bytes = Vec::new();
        for f in [half[0].max(1.0), half[1].max(1.0), 0.0, 0.0] {
            bytes.extend(f.to_le_bytes());
        }
        gpu::record(|q| {
            let off = q.blob(&bytes);
            if made == 0 {
                q.cmd(&[gpu::CREATE_BUFFER, g, 16]);
            }
            q.cmd(&[gpu::WRITE_BUFFER, g, 0, off, 16]);
        });
    }
    [15, 0, d.sprite_view.0, 0, 16, 16, 0, 0]
}

/// A compute shader's page shader and bindings for UAVs of these page formats (slot, format).
fn compute_shader_for(d: &mut D11, cs: u32, fmts: &[(u32, u32)]) -> Option<(u32, Vec<(u32, dxwgsl::Binding)>)> {
    let trace = d.trace;
    let Some(Data::Shader(s)) = d.objs.get_mut(&cs).map(|o| &mut o.data) else { return None };
    let key = fmts.iter().fold(0x811c_9dc5u32, |h, &(n, f)| (h ^ (n << 8 | f)).wrapping_mul(0x0100_0193));
    let key = (key, Vec::new());
    if let Some(r) = s.wgsl.get(&key) {
        return r.clone();
    }
    let names: Vec<(u32, &str)> = fmts.iter().map(|&(n, f)| (n, crate::render::d3d::TEX_FORMATS.get(us(f)).map_or("rgba8unorm", |t| t.name()))).collect();
    let r = match dxwgsl::translate_compute(&s.code, &names) {
        Ok(w) => {
            let id = page_id();
            gpu::record(|q| {
                let off = q.blob(w.code.as_bytes());
                q.cmd(&[gpu::CREATE_SHADER, id, off, lo32(w.code.len())]);
            });
            if trace {
                eprintln!("d3d11: compute shader {cs} -> {id} bindings {:?}", w.bindings);
                if trace2() {
                    eprintln!("{}", w.code);
                }
            }
            Some((id, w.bindings))
        }
        Err(e) => {
            eprintln!("d3d11: compute shader {cs}: {e}");
            None
        }
    };
    s.wgsl.insert(key, r.clone());
    r
}

/// Dispatch(x, y, z): the compute shader on the page (render::d3d DISPATCH), its typed UAVs as
/// storage textures. (D3D11_CS_DISPATCH_MAX_THREAD_GROUPS_PER_DIMENSION: 65535; more is
/// invalid and runs nothing)
unsafe fn dispatch(d: &mut D11, st: &State, x: u32, y: u32, z: u32) {
    let cs = st.shaders[3];
    if cs == 0 || x == 0 || y == 0 || z == 0 || x > 65535 || y > 65535 || z > 65535 {
        return;
    }
    for &v in &st.uavs[1] {
        if v != 0 {
            nv12_target(d, v);
        }
    }
    let code = match d.objs.get(&cs).map(|o| &o.data) {
        Some(Data::Shader(s)) => s.code.clone(),
        _ => return,
    };
    if let Ok([tx, ty, tz]) = dxwgsl::thread_group(&code) {
        let n = |a: u32, b: u32, c: u32| u64::from(a).saturating_mul(u64::from(b)).saturating_mul(u64::from(c));
        query::count_dispatch(&mut d.stats, n(x, y, z), n(tx, ty, tz));
    }
    // the texture behind a view: (page texture, page format)
    fn tex_of(d: &D11, v: u32) -> Option<(u32, u32, &View)> {
        let Some(Data::View(vv)) = d.objs.get(&v).map(|o| &o.data) else { return None };
        match d.objs.get(&vv.res).map(|o| &o.data) {
            // (a buffer's view: its own texture is no storage; an NV12 chroma view's is)
            Some(Data::Texture(t)) if vv.buf.as_ref().is_none_or(|b| matches!(d.objs.get(&b.buf).map(|o| &o.data), Some(Data::Texture(_)))) => Some((t.gpu, t.page, vv)),
            _ => None,
        }
    }
    let mut fmts = Vec::new();
    for (n, _, _) in dxwgsl::uav_slots(&code).unwrap_or_default() {
        let v = st.uavs[1].get(us(n)).copied().unwrap_or(0);
        if let Some((_, page, _)) = tex_of(d, v) {
            fmts.push((n, page));
        }
    }
    let Some((sh, binds)) = compute_shader_for(d, cs, &fmts) else { return };
    let mut w = vec![gpu::DISPATCH, sh, x, y, z, lo32(binds.len())];
    for (bi, kind) in &binds {
        let e = match kind {
            dxwgsl::Binding::Uniform(size) => {
                let cb = st.cbs[5].get(us(*bi)).copied().unwrap_or(0);
                let range = st.cb_ranges[5].get(us(*bi)).copied().unwrap_or(CbRange::WHOLE);
                let (g, len) = if cb != 0 { cb_binding(d, cb, range, *size) } else { (0, 0) };
                [*bi, 0, g, 0, len, *size, 0, 0]
            }
            dxwgsl::Binding::Storage(vdim, _) => {
                let n = bi - 200;
                let v = st.uavs[1].get(us(n)).copied().unwrap_or(0);
                match tex_of(d, v) {
                    Some((g, page, vv)) => [*bi, 7, g, *vdim, vv.mip, page, vv.layer, vv.layers.max(1)],
                    // (no UAV bound: writes go nowhere, as in Direct3D; nothing to run then)
                    None => return,
                }
            }
            dxwgsl::Binding::Texture(vdim, sample) if *bi >= 300 => {
                let n = bi - 300;
                let v = st.uavs[1].get(us(n)).copied().unwrap_or(0);
                match tex_of(d, v) {
                    Some((g, _, vv)) => [*bi, 8, g, *vdim, vv.mip, *sample, vv.layer, vv.layers.max(1)],
                    None => [*bi, 8, 0, *vdim, 0, *sample, 0, 1],
                }
            }
            dxwgsl::Binding::Texture(vdim, sample) => {
                let t = bi - 16;
                let sv = st.srvs[5].get(us(t)).copied().unwrap_or(0);
                buffer_view_sync(d, sv);
                let mut e = match d.objs.get(&sv).map(|o| &o.data) {
                    Some(Data::View(v)) => match d.objs.get(&v.res).map(|o| &o.data) {
                        Some(Data::Texture(tx)) => [*bi, 1, tx.gpu, *vdim, v.mip, v.mips.min(tx.mips - v.mip.min(tx.mips - 1)).max(1), v.layer, v.layers.max(1)],
                        _ => [*bi, 1, 0, *vdim, 0, 1, 0, 1],
                    },
                    _ => [*bi, 1, 0, *vdim, 0, 1, 0, 1],
                };
                if *sample != 0 {
                    e[1] = 3 + *sample;
                }
                e
            }
            dxwgsl::Binding::Sampler(cmp) => {
                let s = bi - 160;
                let id = st.samplers[5].get(us(s)).copied().unwrap_or(0);
                let desc = state_desc(d, id).cloned().unwrap_or_else(|| vec![0x15, 3, 3, 3, 0, 1, 8, 0, 0, 0, 0, 0, 0x7f7f_ffff]);
                [*bi, if *cmp { 3 } else { 2 }, sampler_key(&desc, *cmp), desc[11], desc[12], 0, 0, 0]
            }
            // (a store nothing names: none bound, nothing runs)
            dxwgsl::Binding::StorageBuffer(rw) => [*bi, if *rw { 10 } else { 9 }, 0, 0, 0, 0, 0, 0],
        };
        w.extend_from_slice(&e);
    }
    gpu::record(|q| q.cmd(&w));
    // (what the page drew to is not what kura thinks it last drew: targets set again)
    d.targets = None;
}

/// D3D11_SAMPLER_DESC as the page's sampler key: address u v w (3 bits each), mag, min, mip
/// (linear 1 bit each), anisotropic, compare function (4 bits)
fn sampler_key(s: &[u32], cmp: bool) -> u32 {
    let f = s[0];
    let addr = |a: u32| match a {
        1 => 0, // wrap
        2 => 1, // mirror
        3 => 2, // clamp
        4 | 5 => 2, // border, mirror once: clamped, the pixel shader does the rest (dxwgsl::addr)
        _ => 0,
    };
    let (mip, mag, min) = (f & 1, (f >> 2) & 1, (f >> 4) & 1);
    let aniso = u32::from(f & 0x40 != 0);
    let func = if cmp { s[6] & 15 } else { 0 };
    addr(s[1]) | addr(s[2]) << 3 | addr(s[3]) << 6 | mag << 9 | min << 10 | mip << 11 | aniso << 12 | func << 13 | (s[5].clamp(1, 16)) << 17
}

#[cfg(test)]
mod vb_merge_tests {
    use super::merge_vertex_buffers;
    use kome::num::*;
    use proptest::prelude::*;

    #[test]
    fn nine_slots_on_one_buffer_become_one() {
        // ANGLE: 9 float4 attributes, each its own slot on buffer 7 at offset 16*i, stride 144
        let mut vbs = [(0u32, 0u32, 0u32); 32];
        let mut slots = Vec::new();
        for i in 0..9u32 {
            vbs[us(i)] = (7, 144, 1000 + 16 * i);
            slots.push((i, 0, vec![[3, 0, i]]));
        }
        let vbx = merge_vertex_buffers(&vbs, &mut slots);
        assert_eq!(slots.len(), 1);
        assert_eq!(vbx[us(slots[0].0)], (7, 144, 1000));
        let offs: Vec<u32> = slots[0].2.iter().map(|a| a[1]).collect();
        assert_eq!(offs, (0..9).map(|i| 16 * i).collect::<Vec<_>>());
    }

    #[test]
    fn eight_or_fewer_stay() {
        let mut vbs = [(0u32, 0u32, 0u32); 32];
        vbs[0] = (1, 16, 0);
        vbs[1] = (1, 16, 4);
        let mut slots = vec![(0, 0, vec![[0, 0, 0]]), (1, 0, vec![[0, 0, 1]])];
        merge_vertex_buffers(&vbs, &mut slots);
        assert_eq!(slots.len(), 2);
    }

    proptest! {
        /// every attribute still reads the same bytes of the same buffer, and fits its vertex
        #[test]
        fn merging_keeps_each_attribute_address(n in 9usize..16, bufs in prop::collection::vec(1u32..4, 16),
            strides in prop::collection::vec(prop::sample::select(vec![0u32, 16, 32, 64]), 16),
            offs in prop::collection::vec(0u32..64, 16), fmts in prop::collection::vec(0u32..4, 16)) {
            let mut vbs = [(0u32, 0u32, 0u32); 32];
            let mut slots = Vec::new();
            for i in 0..n {
                vbs[i] = (bufs[i], strides[i], offs[i]);
                slots.push((lo32(i), 0, vec![[fmts[i], 0, lo32(i)]]));
            }
            let want: Vec<(u32, u32, u32, u32)> = (0..n).map(|i| (lo32(i), bufs[i], strides[i], offs[i])).collect();
            let vbx = merge_vertex_buffers(&vbs, &mut slots);
            let mut got = Vec::new();
            for (s, _, attrs) in &slots {
                let (b, st, o) = vbx[us(*s)];
                for a in attrs {
                    let limit = if st == 0 { 2048 } else { st };
                    prop_assert!(a[1] + super::vertex_size(a[0]) <= limit || attrs.len() == 1);
                    got.push((a[2], b, st, o + a[1]));
                }
            }
            got.sort();
            prop_assert_eq!(got, want);
        }
    }
}

#[cfg(test)]
mod changed_range_tests {
    use super::changed_range;
    use proptest::prelude::*;

    #[test]
    fn cases() {
        assert_eq!(changed_range(&[0; 16], &[0; 16]), None);
        assert_eq!(changed_range(&[], &[]), None);
        let mut b = [0u8; 16];
        b[5] = 1;
        assert_eq!(changed_range(&[0; 16], &b), Some((4, 8)));
        b[15] = 1;
        assert_eq!(changed_range(&[0; 16], &b), Some((4, 16)));
        // (a length not a multiple of 4: the end stops at it)
        let mut c = [0u8; 10];
        c[9] = 2;
        assert_eq!(changed_range(&[0; 10], &c), Some((8, 10)));
        c[0] = 2;
        assert_eq!(changed_range(&[0; 10], &c), Some((0, 10)));
    }

    proptest! {
        /// Copying the range from `new` onto `old` gives `new`; the range is word-aligned and
        /// starts and ends within a word of the first and last difference.
        #[test]
        fn covers_every_difference(old in proptest::collection::vec(0u8..3, 0..300), flips in proptest::collection::vec((0usize..300, 1u8..3), 0..6)) {
            let mut new = old.clone();
            for (at, x) in flips {
                if at < new.len() {
                    new[at] ^= x;
                }
            }
            let diffs: Vec<usize> = (0..old.len()).filter(|&i| old[i] != new[i]).collect();
            match changed_range(&old, &new) {
                None => prop_assert!(diffs.is_empty()),
                Some((lo, hi)) => {
                    prop_assert!(!diffs.is_empty());
                    prop_assert!(lo % 4 == 0 && (hi % 4 == 0 || hi == new.len()) && lo < hi && hi <= new.len());
                    let (first, last) = (diffs[0], *diffs.last().unwrap());
                    prop_assert!(lo <= first && first < lo + 4, "lo {} first {}", lo, first);
                    prop_assert!(last < hi && hi <= (last + 4) & !3 || hi == new.len(), "hi {} last {}", hi, last);
                    let mut patched = old.clone();
                    patched[lo..hi].copy_from_slice(&new[lo..hi]);
                    prop_assert_eq!(patched, new);
                }
            }
        }
    }
}

#[cfg(test)]
mod wire_tests {
    use super::triangle_edges;

    #[test]
    fn edges_of_lists_and_strips() {
        assert_eq!(triangle_edges(&[0, 1, 2, 3, 4, 5, 6], false, None), [0, 1, 1, 2, 2, 0, 3, 4, 4, 5, 5, 3]);
        assert_eq!(triangle_edges(&[0, 1, 2, 3], true, None), [0, 1, 1, 2, 2, 0, 1, 2, 2, 3, 3, 1]);
        // a strip cut by the restart index
        assert_eq!(triangle_edges(&[0, 1, 2, 0xffff, 5, 6, 7], true, Some(0xffff)), [0, 1, 1, 2, 2, 0, 5, 6, 6, 7, 7, 5]);
        assert!(triangle_edges(&[0, 1], true, None).is_empty());
    }

    proptest::proptest! {
        /// Every triangle gives three edges that chain back to its first vertex
        #[test]
        fn three_closed_edges_a_triangle(idx in proptest::collection::vec(0u32..50, 0..40), strip: bool) {
            let e = triangle_edges(&idx, strip, None);
            let tris = if strip { idx.len().saturating_sub(2) } else { idx.len() / 3 };
            proptest::prop_assert_eq!(e.len(), tris * 6);
            for t in e.chunks(6) {
                proptest::prop_assert_eq!((t[1], t[3], t[5]), (t[2], t[4], t[0]));
            }
        }
    }
}
