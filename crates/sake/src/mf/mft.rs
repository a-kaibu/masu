//! What a program that decodes into Direct3D 11 textures asks of the Source Reader besides
//! frames, as Unity's VideoPlayer does with a device for video (D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
//! #227): IMFSourceReader::GetServiceForStream(video stream, GUID_NULL, IID_IMFTransform), the
//! stream's decoder, of which it reads the attributes (MF_SA_D3D11_AWARE,
//! MF_SA_MINIMUM_OUTPUT_SAMPLE_COUNT) and the output type (the frame size); and
//! MFCreateVideoSampleAllocatorEx, IMFVideoSampleAllocatorEx samples of a type's frames as
//! textures of the device of an IMFDXGIDeviceManager (memory buffers without one).
//!
//! The decoder is the reader's (vstream): the transform decodes nothing itself, its
//! ProcessInput / ProcessOutput say so (E_NOTIMPL). The allocator keeps no pool: each
//! AllocateSample makes a new texture (at most the maximum InitializeSampleAllocatorEx gave out
//! at once is not counted).

use super::*;

/// IMFTransform {bf94c121-5b05-4e6f-8000-ba598961414d}
pub fn is_transform(g: &Guid) -> bool {
    is(g, 0xbf94_c121, 0x5b05, 0x4e6f)
}
/// MF_SA_D3D11_AWARE {206b4fc8-fcf9-4c51-afe3-9764369e33a0}
fn k_d3d11_aware() -> Guid {
    [0xc8, 0x4f, 0x6b, 0x20, 0xf9, 0xfc, 0x51, 0x4c, 0xaf, 0xe3, 0x97, 0x64, 0x36, 0x9e, 0x33, 0xa0]
}
/// MF_SA_MINIMUM_OUTPUT_SAMPLE_COUNT {851745d5-c3d6-476d-9527-498ef2d10d18}
fn k_min_samples() -> Guid {
    [0xd5, 0x45, 0x17, 0x85, 0xd6, 0xc3, 0x6d, 0x47, 0x95, 0x27, 0x49, 0x8e, 0xf2, 0xd1, 0x0d, 0x18]
}
/// MF_SA_D3D11_BINDFLAGS {eacf97ad-065c-4408-bee3-fdcbfd128be2}
fn k_bindflags() -> Guid {
    [0xad, 0x97, 0xcf, 0xea, 0x5c, 0x06, 0x08, 0x44, 0xbe, 0xe3, 0xfd, 0xcb, 0xfd, 0x12, 0x8b, 0xe2]
}
/// MF_SA_D3D11_USAGE {e85fe442-2ca3-486e-a9c7-109dda609880}
fn k_usage() -> Guid {
    [0x42, 0xe4, 0x5f, 0xe8, 0xa3, 0x2c, 0x6e, 0x48, 0xa9, 0xc7, 0x10, 0x9d, 0xda, 0x60, 0x98, 0x80]
}
/// The samples the decoder asks for (MF_SA_MINIMUM_OUTPUT_SAMPLE_COUNT): Unity's own default.
pub const MIN_SAMPLES: u32 = 6;


// --- the decoder ------------------------------------------------------------------------------

/// A transform: references, its reader (a reference) and stream, its attributes (a store).
struct Mft {
    refs: u32,
    reader: u32,
    stream: usize,
    attrs: u32,
}
static MFTS: Mutex<Option<HashMap<u32, Mft>>> = Mutex::new(None);
fn mft<R>(b: u32, f: impl FnOnce(&mut Mft) -> R) -> Option<R> {
    MFTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get_mut(&b).map(f)
}

fn mt_qi(c: &mut Ctx) -> u32 {
    let (this, out) = (c.a(0), c.a(2));
    if out == 0 {
        return Hr::Pointer.code();
    }
    let g = guid_at(c.a(1));
    if is_unknown(&g) || is_transform(&g) {
        mft(this, |m| m.refs += 1);
        putp(out, this);
        return 0;
    }
    putp(out, 0);
    Hr::NoInterface.code()
}
fn mt_add_ref(c: &mut Ctx) -> u32 {
    mft(c.a(0), |m| {
        m.refs += 1;
        m.refs
    })
    .unwrap_or(1)
}
fn mt_release(c: &mut Ctx) -> u32 {
    let this = c.a(0);
    let n = mft(this, |m| {
        m.refs = m.refs.saturating_sub(1);
        m.refs
    })
    .unwrap_or(0);
    if n == 0 && let Some(m) = MFTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).remove(&this) {
        release(c, m.attrs);
        release(c, m.reader);
        mem::heap_free(this);
    }
    n
}
/// GetStreamLimits(in min, in max, out min, out max): one of each
fn mt_limits(c: &mut Ctx) -> u32 {
    for i in 1..=4 {
        if c.a(i) == 0 {
            return Hr::Pointer.code();
        }
    }
    for i in 1..=4 {
        w32(c.a(i), 1);
    }
    0
}
/// GetStreamCount(in, out): 1, 1
fn mt_count(c: &mut Ctx) -> u32 {
    if c.a(1) == 0 || c.a(2) == 0 {
        return Hr::Pointer.code();
    }
    w32(c.a(1), 1);
    w32(c.a(2), 1);
    0
}
/// GetAttributes(IMFAttributes **)
fn mt_attributes(c: &mut Ctx) -> u32 {
    let (this, out) = (c.a(0), c.a(1));
    if out == 0 {
        return Hr::Pointer.code();
    }
    let a = mft(this, |m| m.attrs).unwrap_or(0);
    add_ref(c, a);
    putp(out, a);
    if a == 0 { Hr::NotImpl.code() } else { 0 }
}
/// GetOutputAvailableType(stream, index, IMFMediaType **): the reader's current type, index 0
fn mt_output_available(c: &mut Ctx) -> u32 {
    let (this, id, i, out) = (c.a(0), c.a(1), c.a(2), c.a(3));
    if out == 0 {
        return Hr::Pointer.code();
    }
    putp(out, 0);
    if id != 0 {
        return Hr::InvalidStreamNumber.code();
    }
    if i != 0 {
        return Hr::NoMoreTypes.code();
    }
    current(this, out)
}
/// GetOutputCurrentType(stream, IMFMediaType **): a copy of the reader's current type
fn mt_output_current(c: &mut Ctx) -> u32 {
    let (this, id, out) = (c.a(0), c.a(1), c.a(2));
    if out == 0 {
        return Hr::Pointer.code();
    }
    putp(out, 0);
    if id != 0 {
        return Hr::InvalidStreamNumber.code();
    }
    current(this, out)
}
fn current(this: u32, out: u32) -> u32 {
    let Some((r, k)) = mft(this, |m| (m.reader, m.stream)) else { return Hr::InvalidArg.code() };
    match super::reader::current_type(r, k) {
        Some(s) => {
            putp(out, super::reader::new_type(s));
            0
        }
        None => Hr::TransformTypeNotSet.code(),
    }
}
/// GetInputStatus / GetOutputStatus: ready (MFT_INPUT_STATUS_ACCEPT_DATA: the reader feeds it)
fn mt_input_status(c: &mut Ctx) -> u32 {
    if c.a(2) == 0 {
        return Hr::Pointer.code();
    }
    w32(c.a(2), 1);
    0
}
fn mt_output_status(c: &mut Ctx) -> u32 {
    if c.a(1) == 0 {
        return Hr::Pointer.code();
    }
    w32(c.a(1), 0);
    0
}
/// GetInputStreamAttributes / GetOutputStreamAttributes(stream, IMFAttributes **): none
fn mt_stream_attributes(c: &mut Ctx) -> u32 {
    if c.a(2) != 0 {
        putp(c.a(2), 0);
    }
    Hr::NotImpl.code()
}
fn mt_vtable() -> u32 {
    static VT: OnceLock<u32> = OnceLock::new();
    *VT.get_or_init(|| {
        vtable(
            "IMFTransform",
            &[
                (mt_qi as Handler, 12),
                (mt_add_ref, 4),
                (mt_release, 4),
                (mt_limits, 20),
                (mt_count, 12),
                (notimpl, 20), // GetStreamIDs: fixed numbers from 0
                (notimpl, 12), // GetInputStreamInfo
                (notimpl, 12), // GetOutputStreamInfo
                (mt_attributes, 8),
                (mt_stream_attributes, 12),
                (mt_stream_attributes, 12),
                (notimpl, 8),  // DeleteInputStream
                (notimpl, 12), // AddInputStreams
                (notimpl, 16), // GetInputAvailableType
                (mt_output_available, 16),
                (notimpl, 16), // SetInputType: the reader's
                (notimpl, 16), // SetOutputType: SetCurrentMediaType on the reader
                (notimpl, 12), // GetInputCurrentType
                (mt_output_current, 12),
                (mt_input_status, 12),
                (mt_output_status, 8),
                (notimpl, 20), // SetOutputBounds
                (notimpl, 12), // ProcessEvent
                (ok, 12),      // ProcessMessage
                (notimpl, 16), // ProcessInput
                (notimpl, 20), // ProcessOutput
            ],
        )
    })
}

/// The decoder of reader `reader`'s stream `stream` (a new transform, one reference; it holds
/// one of the reader).
pub fn transform(c: &mut Ctx, reader: u32, stream: usize) -> u32 {
    let b = object(mt_vtable(), 1);
    let attrs = new_attrs();
    super::attrs(attrs, |a| {
        a.1.set(k_d3d11_aware(), Val::U32(1));
        a.1.set(k_min_samples(), Val::U32(MIN_SAMPLES));
    });
    add_ref(c, reader);
    MFTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(b, Mft { refs: 1, reader, stream, attrs });
    b
}

// --- the sample allocator ---------------------------------------------------------------------

/// An allocator: references, the device manager (a reference), and once initialized the frames'
/// size, DXGI format, usage and bind flags.
struct Alloc {
    refs: u32,
    manager: u32,
    frames: Option<Frames>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frames {
    pub w: u32,
    pub h: u32,
    pub fmt: u32,
    pub usage: u32,
    pub bind: u32,
}
static ALLOCS: Mutex<Option<HashMap<u32, Alloc>>> = Mutex::new(None);
fn alloc<R>(b: u32, f: impl FnOnce(&mut Alloc) -> R) -> Option<R> {
    ALLOCS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get_mut(&b).map(f)
}

/// IMFVideoSampleAllocator {86cbc910-e533-4751-8e3b-f19b5b806a03}, IMFVideoSampleAllocatorEx
/// {545b3a48-3283-4f62-866f-a62d8f598f9f}
fn is_allocator(g: &Guid) -> bool {
    is_unknown(g) || is(g, 0x86cb_c910, 0xe533, 0x4751) || is(g, 0x545b_3a48, 0x3283, 0x4f62)
}

/// The frames a type and attributes describe: MF_MT_FRAME_SIZE, MF_MT_SUBTYPE NV12 / RGB32 /
/// ARGB32, MF_SA_D3D11_USAGE (default D3D11_USAGE_DEFAULT; staging is no texture to draw),
/// MF_SA_D3D11_BINDFLAGS (default D3D11_BIND_RENDER_TARGET, a video processor's output).
pub fn frames_of(ty: &Store, attrs: Option<&Store>) -> Result<Frames, u32> {
    let size = match ty.get(&super::source::k_frame_size()) {
        Some(Val::U64(v)) => *v,
        _ => return Err(Hr::InvalidMediaType.code()),
    };
    let (w, h) = (lo32(size >> 32), lo32(size));
    let fmt = match ty.get(&super::reader::k_subtype()) {
        Some(Val::Guid(g)) => super::dxgi::format_of(data1(g)),
        _ => None,
    };
    let Some(fmt) = fmt else { return Err(Hr::InvalidMediaType.code()) };
    if w == 0 || h == 0 || super::dxgi::frame_bytes(fmt, w, h).is_none() || fmt == 103 && (w | h) & 1 != 0 {
        return Err(Hr::InvalidMediaType.code());
    }
    let get = |k: Guid, d: u32| match attrs.and_then(|a| a.get(&k)) {
        Some(Val::U32(v)) => *v,
        _ => d,
    };
    let usage = get(k_usage(), 0);
    if usage > 2 {
        return Err(Hr::InvalidArg.code());
    }
    Ok(Frames { w, h, fmt, usage, bind: get(k_bindflags(), 0x20) })
}

fn al_qi(c: &mut Ctx) -> u32 {
    let (this, out) = (c.a(0), c.a(2));
    if out == 0 {
        return Hr::Pointer.code();
    }
    if is_allocator(&guid_at(c.a(1))) {
        alloc(this, |a| a.refs += 1);
        putp(out, this);
        return 0;
    }
    putp(out, 0);
    Hr::NoInterface.code()
}
fn al_add_ref(c: &mut Ctx) -> u32 {
    alloc(c.a(0), |a| {
        a.refs += 1;
        a.refs
    })
    .unwrap_or(1)
}
fn al_release(c: &mut Ctx) -> u32 {
    let this = c.a(0);
    let n = alloc(this, |a| {
        a.refs = a.refs.saturating_sub(1);
        a.refs
    })
    .unwrap_or(0);
    if n == 0 && let Some(a) = ALLOCS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).remove(&this) {
        release(c, a.manager);
        mem::heap_free(this);
    }
    n
}
/// SetDirectXManager(IUnknown *): the manager (NULL: none, memory samples)
fn al_set_manager(c: &mut Ctx) -> u32 {
    let (this, m) = (c.a(0), c.a(1));
    add_ref(c, m);
    let old = alloc(this, |a| std::mem::replace(&mut a.manager, m)).unwrap_or(0);
    release(c, old);
    0
}
/// UninitializeSampleAllocator
fn al_uninit(c: &mut Ctx) -> u32 {
    alloc(c.a(0), |a| a.frames = None);
    0
}
fn init_with(this: u32, ty: u32, attrs: u32) -> u32 {
    if ty == 0 {
        return Hr::Pointer.code();
    }
    let Some(t) = store_of(ty) else { return Hr::InvalidArg.code() };
    let a = if attrs != 0 { store_of(attrs) } else { None };
    match frames_of(&t, a.as_ref()) {
        Ok(f) => {
            alloc(this, |a| a.frames = Some(f));
            0
        }
        Err(e) => e,
    }
}
/// InitializeSampleAllocator(count, IMFMediaType *)
fn al_init(c: &mut Ctx) -> u32 {
    if c.a(1) == 0 {
        return Hr::InvalidArg.code();
    }
    init_with(c.a(0), c.a(2), 0)
}
/// InitializeSampleAllocatorEx(initial, maximum, IMFAttributes *, IMFMediaType *)
fn al_init_ex(c: &mut Ctx) -> u32 {
    let (initial, max) = (c.a(1), c.a(2));
    if max == 0 || initial > max {
        return Hr::InvalidArg.code();
    }
    init_with(c.a(0), c.a(4), c.a(3))
}
/// AllocateSample(IMFSample **): a sample of one frame's buffer (a texture's, with a manager)
fn al_allocate(c: &mut Ctx) -> u32 {
    let (this, out) = (c.a(0), c.a(1));
    if out == 0 {
        return Hr::Pointer.code();
    }
    putp(out, 0);
    let Some((manager, frames)) = alloc(this, |a| (a.manager, a.frames)) else { return Hr::InvalidArg.code() };
    let Some(f) = frames else { return Hr::NotInitialized.code() };
    let Some(n) = super::dxgi::frame_bytes(f.fmt, f.w, f.h) else { return Hr::InvalidMediaType.code() };
    let zero = vec![0u8; us(n)];
    if manager == 0 {
        putp(out, super::reader::new_sample(None, None, Some(&zero)));
        return 0;
    }
    let dev = super::dxgi::manager_device(c, manager);
    if dev == 0 {
        return Hr::NotInitialized.code();
    }
    let b = super::dxgi::texture_buffer(c, dev, f.w, f.h, f.fmt, f.usage, f.bind, &zero);
    release(c, dev);
    match b {
        Some(b) => {
            putp(out, super::reader::new_sample_of(None, None, b));
            0
        }
        None => Hr::InvalidArg.code(),
    }
}
fn al_vtable() -> u32 {
    static VT: OnceLock<u32> = OnceLock::new();
    *VT.get_or_init(|| {
        vtable(
            "IMFVideoSampleAllocatorEx",
            &[(al_qi as Handler, 12), (al_add_ref, 4), (al_release, 4), (al_set_manager, 8), (al_uninit, 4), (al_init, 12), (al_allocate, 8), (al_init_ex, 20)],
        )
    })
}

/// MFCreateVideoSampleAllocatorEx(REFIID, void **)
fn MFCreateVideoSampleAllocatorEx(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::MFCreateVideoSampleAllocatorEx>();
    let (riid, out) = (c.a::<0>(), c.out::<1>());
    if riid == 0 || out.is_null() {
        return Hr::Pointer.code();
    }
    if !is_allocator(&guid_at(riid)) {
        out.put(0);
        return Hr::NoInterface.code();
    }
    let b = object(al_vtable(), 1);
    ALLOCS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(b, Alloc { refs: 1, manager: 0, frames: None });
    out.put(b);
    0
}

pub fn init() {
    implement("mfplat", "MFCreateVideoSampleAllocatorEx", MFCreateVideoSampleAllocatorEx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sub(fourcc: u32) -> Val {
        let mut g = [0u8; 16];
        g[0..4].copy_from_slice(&fourcc.to_le_bytes());
        g[4..16].copy_from_slice(&[0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);
        Val::Guid(g)
    }

    proptest! {
        /// The frames of a type: its size and format (NV12 of even sizes only), the usage and
        /// bind flags of the attributes or their defaults; anything else refused.
        #[test]
        fn frames_of_types(w in 0u32..2050, h in 0u32..2050, which in 0usize..5, usage in proptest::option::of(0u32..5), bind in proptest::option::of(any::<u32>())) {
            let fourcc = [super::super::source::NV12, super::super::source::RGB32, super::super::source::ARGB32, u32::from_le_bytes(*b"YUY2"), 0][which];
            let mut t = Store::default();
            t.set(super::super::source::k_frame_size(), Val::U64((lo64(w)) << 32 | lo64(h)));
            t.set(super::super::reader::k_subtype(), sub(fourcc));
            let mut a = Store::default();
            if let Some(u) = usage {
                a.set(k_usage(), Val::U32(u));
            }
            if let Some(b) = bind {
                a.set(k_bindflags(), Val::U32(b));
            }
            let got = frames_of(&t, Some(&a));
            let fmt = super::super::dxgi::format_of(fourcc);
            let ok_size = w > 0 && h > 0 && (fmt != Some(103) || (w | h) & 1 == 0);
            match fmt {
                Some(f) if ok_size && usage.unwrap_or(0) <= 2 => prop_assert_eq!(got, Ok(Frames { w, h, fmt: f, usage: usage.unwrap_or(0), bind: bind.unwrap_or(0x20) })),
                Some(_) if ok_size => prop_assert_eq!(got, Err(Hr::InvalidArg.code())),
                _ => prop_assert_eq!(got, Err(Hr::InvalidMediaType.code())),
            }
        }
    }

    /// A type without a size is no type of frames.
    #[test]
    fn frames_need_a_size() {
        let mut t = Store::default();
        t.set(super::super::reader::k_subtype(), sub(super::super::source::NV12));
        assert_eq!(frames_of(&t, None), Err(Hr::InvalidMediaType.code()));
    }
}
