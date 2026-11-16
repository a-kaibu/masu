//! mmdevapi: the multimedia device enumerator (CLSID_MMDeviceEnumerator) with one render
//! endpoint, the browser's audio output, and no capture endpoints. Games ask it for the default
//! device and register for device-change notifications (AdvHD dereferences the enumerator
//! without checking CoCreateInstance); the device's IAudioClient is wasapi.rs.
//! The enumerator is a process-wide singleton holding a reference of its own (CoCreateInstance
//! gives it a second one, as Windows' does); collections are separate objects freed on their
//! last Release; the device, its endpoint and property store are stateless singletons.

use crate::hresult::S_OK;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::api::{Ctx, Handler};
use crate::mem::{self, ps, putp, r16, r32, w16, w32, wp};

use crate::wasapi::Iid;


/// The failures the objects answer
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hr {
    Pointer,
    NoInterface,
    InvalidArg,
    /// HRESULT_FROM_WIN32(ERROR_NOT_FOUND)
    NotFound,
    /// STG_E_ACCESSDENIED
    AccessDenied,
}
impl Hr {
    const fn code(self) -> u32 {
        match self {
            Hr::Pointer => crate::hresult::E_POINTER,
            Hr::NoInterface => crate::hresult::E_NOINTERFACE,
            Hr::InvalidArg => crate::hresult::E_INVALIDARG,
            Hr::NotFound => crate::hresult::E_PROP_ID_UNSUPPORTED,
            Hr::AccessDenied => 0x8003_0005,
        }
    }
}
fn hr(r: Result<(), Hr>) -> u32 {
    r.err().map_or(S_OK, Hr::code)
}

/// EDataFlow
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    Render,
    Capture,
    All,
}
impl Flow {
    const fn code(self) -> u32 {
        match self {
            Flow::Render => 0,
            Flow::Capture => 1,
            Flow::All => 2,
        }
    }
    fn of(v: u32) -> Option<Flow> {
        [Flow::Render, Flow::Capture, Flow::All].into_iter().find(|f| f.code() == v)
    }
}

/// ERole: eConsole, eMultimedia, eCommunications (each the one device)
fn role_ok(v: u32) -> bool {
    v <= 2
}

const DEVICE_STATE_ACTIVE: u32 = 1;
/// DEVICE_STATE_ACTIVE | DISABLED | NOTPRESENT | UNPLUGGED
const DEVICE_STATEMASK_ALL: u32 = 0xf;
/// The endpoint's id (as Windows forms them: flow.{endpoint guid})
pub(crate) const DEVICE_ID: &str = "{0.0.0.00000000}.{5a2c1e0b-7f3d-4e8a-9c61-3b8d0f4a2e17}";
const FRIENDLY_NAME: &str = "スピーカー (sake Audio)";

/// x86 stdcall: the handler pops this many bytes of arguments (x64 ignores it)
fn b(n: u32) -> u32 {
    4 * n
}

fn singleton(name: &str, methods: &[(Handler, u32)], cell: &OnceLock<u32>) -> u32 {
    *cell.get_or_init(|| {
        let o = mem::heap_alloc(2 * ps(), true);
        wp(o, crate::api::vtable(name, methods));
        o
    })
}
/// IID `g` at `p` (its first three fields: the interfaces sake answers differ there)
fn guid_is(p: u32, g: Iid) -> bool {
    p != 0 && r32(p) == g.0 && r16(p + 4) == g.1 && r16(p + 6) == g.2
}
const IID_UNKNOWN: [u8; 16] = guid(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
fn is_unknown(iid: u32) -> bool {
    iid != 0 && mem::array::<16>(iid) == IID_UNKNOWN
}
const IID_ENUMERATOR: Iid = (0xa956_64d2, 0x9614, 0x4f35);
const IID_COLLECTION: Iid = (0x0bd7_a1be, 0x7a1a, 0x44db);
const IID_DEVICE: Iid = (0xd666_063f, 0x1587, 0x4e43);
const IID_PROPERTY_STORE: Iid = (0x886d_8eeb, 0x8cf2, 0x4446);
const IID_DIRECTSOUND: Iid = (0x279a_fa83, 0x4981, 0x11ce);
const IID_DIRECTSOUND8: Iid = (0xc50a_7e93, 0xf395, 0x4834);
const CLSID_ENUMERATOR: Iid = (0xbcde_0395, 0xe52f, 0x467c);
fn one(_: &mut Ctx) -> u32 {
    1
}
/// QueryInterface answering IUnknown and the object's own interface
fn qi(c: &mut crate::api::Args<'_, '_, crate::api::outs::iface::IUnknown::QueryInterface>, own: Iid) -> u32 {
    let (this, iid, out) = (c.a::<0>(), c.a::<1>(), c.out::<2>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    if is_unknown(iid) || guid_is(iid, own) {
        out.put(this);
        S_OK
    } else {
        out.put(0);
        Hr::NoInterface.code()
    }
}
/// A CoTaskMemAlloc'd copy of `s` (UTF-16, NUL-terminated)
fn task_wstr(s: &str) -> u32 {
    let w: Vec<u8> = s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    let p = mem::heap_alloc(u32::try_from(w.len()).unwrap_or(0), false);
    mem::write(p, &w);
    p
}

// ---------------------------------------------------------------------------------------------
// IMMDeviceEnumerator {a95664d2-9614-4f35-a746-de8db63617e6}

fn en_qi(c: &mut crate::api::Args<'_, '_, crate::api::outs::iface::IUnknown::QueryInterface>) -> u32 {
    qi(c, IID_ENUMERATOR)
}
/// EnumAudioEndpoints(flow, state mask, IMMDeviceCollection**)
fn en_enum(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IMMDeviceEnumerator::EnumAudioEndpoints>();
    let (flow, mask, out) = (c.a::<1>(), c.a::<2>(), c.out::<3>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    let Some(flow) = Flow::of(flow).filter(|_| mask & !DEVICE_STATEMASK_ALL == 0) else {
        out.put(0);
        return Hr::InvalidArg.code();
    };
    let renders = match flow {
        Flow::Render | Flow::All => true,
        Flow::Capture => false,
    };
    out.put(collection(renders && mask & DEVICE_STATE_ACTIVE != 0));
    S_OK
}
/// GetDefaultAudioEndpoint(flow, role, IMMDevice**): the render endpoint; no capture one
fn en_default(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IMMDeviceEnumerator::GetDefaultAudioEndpoint>();
    let (flow, role, out) = (c.a::<1>(), c.a::<2>(), c.out::<3>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    out.put(0);
    let Some(flow) = Flow::of(flow).filter(|_| role_ok(role)) else { return Hr::InvalidArg.code() };
    match flow {
        Flow::Render => {
            out.put(device());
            S_OK
        }
        Flow::Capture | Flow::All => Hr::NotFound.code(),
    }
}
/// GetDevice(id, IMMDevice**): an id that names no endpoint is E_INVALIDARG (Windows, Wine's
/// mmdevenum test)
fn en_get(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IUnknown::QueryInterface>();
    let (id, out) = (c.a::<1>(), c.out::<2>());
    if id == 0 || out.is_null() {
        return Hr::Pointer.code();
    }
    if mem::wstr(id).into_iter().eq(DEVICE_ID.encode_utf16()) {
        out.put(device());
        S_OK
    } else {
        out.put(0);
        Hr::InvalidArg.code()
    }
}
/// The registered IMMNotificationClient pointers (one entry per registration; the one device
/// never changes, so none is called; Windows takes no reference on them)
static CLIENTS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
/// RegisterEndpointNotificationCallback(IMMNotificationClient*)
fn en_register(c: &mut Ctx) -> u32 {
    if c.a(1) == 0 {
        return Hr::Pointer.code();
    }
    CLIENTS.lock().unwrap_or_else(|e| e.into_inner()).push(c.a(1));
    S_OK
}
/// UnregisterEndpointNotificationCallback(IMMNotificationClient*): one registration of it
fn en_unregister(c: &mut Ctx) -> u32 {
    if c.a(1) == 0 {
        return Hr::Pointer.code();
    }
    let mut l = CLIENTS.lock().unwrap_or_else(|e| e.into_inner());
    match l.iter().position(|&p| p == c.a(1)) {
        Some(i) => {
            l.remove(i);
            S_OK
        }
        None => Hr::NotFound.code(),
    }
}
/// The enumerator's references: its own one plus its users'; it is never freed
static ENUM_REFS: AtomicU32 = AtomicU32::new(1);
fn en_addref(_: &mut Ctx) -> u32 {
    ENUM_REFS.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}
// (fetch_update is try_update on newer toolchains, as in mem.rs)
#[allow(deprecated)]
fn en_release(_: &mut Ctx) -> u32 {
    let r = ENUM_REFS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |r| Some(r.saturating_sub(1).max(1)));
    r.unwrap_or(1).saturating_sub(1).max(1)
}
fn en_qi_ref(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IUnknown::QueryInterface>();
    let r = en_qi(c);
    if r == S_OK {
        ENUM_REFS.fetch_add(1, Ordering::Relaxed);
    }
    r
}
fn enumerator_object() -> u32 {
    static O: OnceLock<u32> = OnceLock::new();
    singleton("IMMDeviceEnumerator", &[(en_qi_ref, b(3)), (en_addref, b(1)), (en_release, b(1)), (en_enum, b(4)), (en_default, b(4)), (en_get, b(3)), (en_register, b(2)), (en_unregister, b(2))], &O)
}
/// The enumerator with a new reference for the caller
pub fn enumerator() -> u32 {
    let o = enumerator_object();
    ENUM_REFS.fetch_add(1, Ordering::Relaxed);
    o
}

// ---------------------------------------------------------------------------------------------
// IMMDeviceCollection {0bd7a1be-7a1a-44db-8397-cc5392387b5e}: none or the one device; the
// object is [vtable, has, references] and takes no reference on the enumerator

fn col_qi(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IUnknown::QueryInterface>();
    let r = qi(c, IID_COLLECTION);
    if r == S_OK {
        col_addref(c.ctx());
    }
    r
}
/// A collection's [vtable, has, references]
fn col_refs(this: u32) -> u32 {
    this.wrapping_add(2 * ps())
}
fn col_addref(c: &mut Ctx) -> u32 {
    let p = col_refs(c.a(0));
    w32(p, r32(p).wrapping_add(1));
    r32(p)
}
fn col_release(c: &mut Ctx) -> u32 {
    let p = col_refs(c.a(0));
    let r = r32(p).saturating_sub(1);
    w32(p, r);
    if r == 0 {
        mem::heap_free(c.a(0));
    }
    r
}
fn col_has(this: u32) -> bool {
    r32(this + ps()) != 0
}
fn col_count(c: &mut Ctx) -> u32 {
    if c.a(1) == 0 {
        return Hr::Pointer.code();
    }
    w32(c.a(1), u32::from(col_has(c.a(0))));
    S_OK
}
fn col_item(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IMMDeviceCollection::Item>();
    let (this, i, out) = (c.a::<0>(), c.a::<1>(), c.out::<2>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    if i == 0 && col_has(this) {
        out.put(device());
        S_OK
    } else {
        out.put(0);
        Hr::InvalidArg.code()
    }
}
fn collection(has: bool) -> u32 {
    static V: OnceLock<u32> = OnceLock::new();
    let v = *V.get_or_init(|| crate::api::vtable("IMMDeviceCollection", &[(col_qi, b(3)), (col_addref, b(1)), (col_release, b(1)), (col_count, b(2)), (col_item, b(3))]));
    let o = mem::heap_alloc(3 * ps(), true);
    wp(o, v);
    w32(o + ps(), u32::from(has));
    w32(o + 2 * ps(), 1);
    o
}

// ---------------------------------------------------------------------------------------------
// IMMDevice {d666063f-1587-4e43-81f1-b948e807363f}

/// IMMDevice and, as Windows' device object has, IMMEndpoint (Unity 6's audio setup asks for it
/// and uses the pointer without checking the result)
fn dev_qi(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IUnknown::QueryInterface>();
    let (iid, out) = (c.a::<1>(), c.out::<2>());
    if !out.is_null() && guid_is(iid, ENDPOINT) {
        out.put(endpoint());
        return S_OK;
    }
    qi(c, IID_DEVICE)
}
/// Activate(iid, clsctx, params, void**): WASAPI's IAudioClient, IAudioSessionManager(2) and
/// IAudioEndpointVolume (wasapi.rs), or the property store
fn dev_activate(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IMMDevice::Activate>();
    let (iid, out) = (c.a::<1>(), c.out::<4>());
    if iid == 0 || out.is_null() {
        return Hr::Pointer.code();
    }
    if guid_is(iid, IID_PROPERTY_STORE) {
        out.put(store());
        return S_OK;
    }
    // IDirectSound(8) {279afa83-4981-11ce-a521-0020af0be560}, {c50a7e93-f395-4834-9ef6-7fa99de50966}:
    // a DirectSound device on this endpoint, dsound.dll loaded as Windows' does
    if guid_is(iid, IID_DIRECTSOUND) || guid_is(iid, IID_DIRECTSOUND8) {
        crate::pe::load_library(c.ctx().cpu, "dsound.dll");
        if let Some(o) = crate::sound::device() {
            out.put(o);
            return S_OK;
        }
    }
    // WASAPI (wasapi.rs): a stream, the session manager, the endpoint's volume
    use crate::wasapi::{IID_CLIENT, IID_ENDPOINT_VOLUME, IID_MANAGER, IID_MANAGER2};
    let is = |g: Iid| guid_is(iid, g);
    let o = if is(IID_CLIENT) {
        crate::wasapi::new_client()
    } else if is(IID_MANAGER) || is(IID_MANAGER2) {
        crate::wasapi::manager()
    } else if is(IID_ENDPOINT_VOLUME) {
        crate::wasapi::endpoint_volume()
    } else {
        0
    };
    if o != 0 {
        out.put(o);
        return S_OK;
    }
    crate::com::warn_once(&format!("IMMDevice::Activate {:08x}: not available", r32(iid)));
    out.put(0);
    Hr::NoInterface.code()
}
/// OpenPropertyStore(access, IPropertyStore**): STGM_READ, _WRITE or _READWRITE; another
/// access is E_INVALIDARG with the pointer left as it was (Wine's propstore test)
fn dev_store(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IUnknown::QueryInterface>();
    // STGM_READ 0, STGM_WRITE 1, STGM_READWRITE 2
    if c.a::<1>() > 2 {
        return Hr::InvalidArg.code();
    }
    if c.out::<2>().is_null() {
        return Hr::Pointer.code();
    }
    c.out::<2>().put(store());
    S_OK
}
/// GetId(LPWSTR*)
fn dev_id(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IMMDevice::GetId>();
    if c.out::<1>().is_null() {
        return Hr::Pointer.code();
    }
    c.out::<1>().put(task_wstr(DEVICE_ID));
    S_OK
}
fn dev_state(c: &mut Ctx) -> u32 {
    if c.a(1) == 0 {
        return Hr::Pointer.code();
    }
    w32(c.a(1), DEVICE_STATE_ACTIVE);
    S_OK
}
fn device() -> u32 {
    static O: OnceLock<u32> = OnceLock::new();
    singleton("IMMDevice", &[(dev_qi, b(3)), (one, b(1)), (one, b(1)), (dev_activate, b(5)), (dev_store, b(3)), (dev_id, b(2)), (dev_state, b(2))], &O)
}

// ---------------------------------------------------------------------------------------------
// IMMEndpoint {1be09788-6894-4089-8586-9a2a6c265ac5}: the device's second interface (IUnknown
// and IMMDevice through it are the device, as COM's identity rule wants)

const ENDPOINT: Iid = (0x1be0_9788, 0x6894, 0x4089);
fn ep_qi(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IUnknown::QueryInterface>();
    let (this, iid, out) = (c.a::<0>(), c.a::<1>(), c.out::<2>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    let to = if guid_is(iid, ENDPOINT) {
        this
    } else if is_unknown(iid) || guid_is(iid, IID_DEVICE) {
        device()
    } else {
        out.put(0);
        return Hr::NoInterface.code();
    };
    out.put(to);
    S_OK
}
/// GetDataFlow(EDataFlow*): the one endpoint renders
fn ep_flow(c: &mut Ctx) -> u32 {
    if c.a(1) == 0 {
        return Hr::Pointer.code();
    }
    w32(c.a(1), Flow::Render.code());
    S_OK
}
fn endpoint() -> u32 {
    static O: OnceLock<u32> = OnceLock::new();
    singleton("IMMEndpoint", &[(ep_qi, b(3)), (one, b(1)), (one, b(1)), (ep_flow, b(2))], &O)
}

// ---------------------------------------------------------------------------------------------
// IPropertyStore {886d8eeb-8cf2-4446-8d02-cdba1dbdcf99}: the names, read-only

/// A property's value
enum Val {
    Str(&'static str),
    /// VT_UI4
    U4(u32),
    /// VT_BLOB holding the device format (WAVEFORMATEXTENSIBLE)
    DeviceFormat,
}
/// The keys answered (fmtid, pid, value): PKEY_Device_FriendlyName, PKEY_Device_DeviceDesc,
/// PKEY_DeviceInterface_FriendlyName, PKEY_AudioEndpoint_FormFactor (Speakers),
/// PKEY_AudioEndpoint_GUID, PKEY_AudioEngine_DeviceFormat and the device interface path
/// {233164c8-1b2c-4c7d-bc68-b671687a2567},1
const KEYS: [([u8; 16], u32, Val); 7] = [
    (guid(0xa45c_254e, 0xdf1c, 0x4efd, [0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0]), 14, Val::Str(FRIENDLY_NAME)),
    (guid(0xa45c_254e, 0xdf1c, 0x4efd, [0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0]), 2, Val::Str("スピーカー")),
    (guid(0x026e_516e, 0xb814, 0x414b, [0x83, 0xcd, 0x85, 0x6d, 0x6f, 0xef, 0x48, 0x22]), 2, Val::Str("sake Audio")),
    (guid(0x1da5_d803, 0xd492, 0x4edd, [0x8c, 0x23, 0xe0, 0xc0, 0xff, 0xee, 0x7f, 0x0e]), 0, Val::U4(1)),
    (guid(0x1da5_d803, 0xd492, 0x4edd, [0x8c, 0x23, 0xe0, 0xc0, 0xff, 0xee, 0x7f, 0x0e]), 4, Val::Str(ENDPOINT_GUID)),
    (guid(0xf19f_064d, 0x082c, 0x4e27, [0xbc, 0x73, 0x68, 0x82, 0xa1, 0xbb, 0x8e, 0x4c]), 0, Val::DeviceFormat),
    (guid(0x2331_64c8, 0x1b2c, 0x4c7d, [0xbc, 0x68, 0xb6, 0x71, 0x68, 0x7a, 0x25, 0x67]), 1, Val::Str(INTERFACE_PATH)),
];
/// The endpoint's guid, as DEVICE_ID ends
const ENDPOINT_GUID: &str = "{5a2c1e0b-7f3d-4e8a-9c61-3b8d0f4a2e17}";
const INTERFACE_PATH: &str = "\\\\?\\SWD#MMDEVAPI#{0.0.0.00000000}.{5a2c1e0b-7f3d-4e8a-9c61-3b8d0f4a2e17}#{e6327cad-dcec-4949-ae8a-991e976a79d2}";
const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> [u8; 16] {
    let a = d1.to_le_bytes();
    let b = d2.to_le_bytes();
    let c = d3.to_le_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]]
}
/// The device format Windows reports for a typical stereo output: 16-bit PCM, 44.1 kHz
/// (the engine mixes in float, wasapi.rs's mix format)
fn device_format() -> [u8; 40] {
    // wFormatTag EXTENSIBLE, 2 channels, 44100 Hz, avg, block 4, 16 bits, cbSize 22,
    // wValidBitsPerSample 16, dwChannelMask FL|FR, KSDATAFORMAT_SUBTYPE_PCM
    // {00000001-0000-0010-8000-00aa00389b71}
    let mut f = Vec::with_capacity(40);
    f.extend_from_slice(&0xfffeu16.to_le_bytes());
    f.extend_from_slice(&2u16.to_le_bytes());
    f.extend_from_slice(&44100u32.to_le_bytes());
    f.extend_from_slice(&(44100u32 * 4).to_le_bytes());
    f.extend_from_slice(&4u16.to_le_bytes());
    f.extend_from_slice(&16u16.to_le_bytes());
    f.extend_from_slice(&22u16.to_le_bytes());
    f.extend_from_slice(&16u16.to_le_bytes());
    f.extend_from_slice(&3u32.to_le_bytes());
    f.extend_from_slice(&guid(1, 0, 0x10, [0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]));
    f.try_into().unwrap_or([0; 40])
}
fn ps_qi(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IUnknown::QueryInterface>();
    qi(c, IID_PROPERTY_STORE)
}
const KEY_COUNT: u32 = 7;
const _: () = assert!(KEYS.len() == 7);
fn ps_count(c: &mut Ctx) -> u32 {
    if c.a(1) == 0 {
        return Hr::Pointer.code();
    }
    w32(c.a(1), KEY_COUNT);
    S_OK
}
/// GetAt(i, PROPERTYKEY*)
fn ps_at(c: &mut Ctx) -> u32 {
    let (i, key) = (c.a(1), c.a(2));
    if key == 0 {
        return Hr::Pointer.code();
    }
    let Some((g, pid, _)) = usize::try_from(i).ok().and_then(|i| KEYS.get(i)) else { return Hr::InvalidArg.code() };
    mem::write(key, g);
    w32(key + 16, *pid);
    S_OK
}
/// GetValue(REFPROPERTYKEY, PROPVARIANT*): the value, VT_EMPTY for another key
fn ps_get(c: &mut Ctx) -> u32 {
    let (key, v) = (c.a(1), c.a(2));
    if key == 0 || v == 0 {
        return Hr::Pointer.code();
    }
    mem::fill(v, 8 + 2 * ps(), 0);
    let k = &mem::array::<16>(key);
    let pid = r32(key + 16);
    match KEYS.iter().find(|(g, p, _)| g[..] == *k && *p == pid).map(|e| &e.2) {
        Some(Val::Str(s)) => {
            w16(v, 31);
            putp(v + 8, task_wstr(s));
        }
        Some(Val::U4(n)) => {
            w16(v, 19);
            w32(v + 8, *n);
        }
        Some(Val::DeviceFormat) => {
            const N: u32 = 40;
            let f: [u8; 40] = device_format();
            let p = mem::heap_alloc(N, false);
            mem::write(p, &f);
            w16(v, 65);
            w32(v + 8, N);
            putp(v + 8 + ps(), p);
        }
        None => {}
    }
    S_OK
}
/// SetValue / Commit: STGM_READ stores
fn ps_denied(_: &mut Ctx) -> u32 {
    hr(Err(Hr::AccessDenied))
}
fn store() -> u32 {
    static O: OnceLock<u32> = OnceLock::new();
    singleton("IPropertyStore", &[(ps_qi, b(3)), (one, b(1)), (one, b(1)), (ps_count, b(2)), (ps_at, b(3)), (ps_get, b(3)), (ps_denied, b(3)), (ps_denied, b(1))], &O)
}

/// CoCreateInstance of CLSID_MMDeviceEnumerator {bcde0395-e52f-467c-8e3d-c4579291692e}
pub fn create_instance(clsid: u32) -> Option<u32> {
    guid_is(clsid, CLSID_ENUMERATOR).then(enumerator)
}


#[cfg(test)]
mod tests {
    use super::*;

    /// PROPERTYKEYs are laid out as Windows' headers have them (fmtid's first three fields
    /// little-endian), and each key is answered once
    #[test]
    fn property_keys_are_windows_bytes_and_unique() {
        // PKEY_Device_FriendlyName {a45c254e-df1c-4efd-8020-67d146a850e0},14 as stored in memory
        assert_eq!(KEYS[0].0, [0x4e, 0x25, 0x5c, 0xa4, 0x1c, 0xdf, 0xfd, 0x4e, 0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0]);
        assert_eq!(KEYS[0].1, 14);
        for (i, a) in KEYS.iter().enumerate() {
            for b in &KEYS[i + 1..] {
                assert!(a.0 != b.0 || a.1 != b.1, "key {i} twice");
            }
        }
        // Device_FriendlyName is "DeviceDesc (DeviceInterface_FriendlyName)" (Wine's propstore test)
        let s = |i: usize| match KEYS[i].2 {
            Val::Str(s) => s,
            _ => panic!(),
        };
        assert_eq!(s(0), format!("{} ({})", s(1), s(2)));
        assert!(DEVICE_ID.ends_with(ENDPOINT_GUID) && INTERFACE_PATH.contains(DEVICE_ID));
    }

    /// The device format is a well-formed WAVEFORMATEXTENSIBLE that the engine plays in shared
    /// mode as it is (the same rate and channels as the mix format)
    #[test]
    fn device_format_is_a_shared_mode_format() {
        let f = device_format();
        let u16_ = |o: usize| u16::from_le_bytes([f[o], f[o + 1]]);
        let u32_ = |o: usize| u32::from_le_bytes([f[o], f[o + 1], f[o + 2], f[o + 3]]);
        let w = crate::wasapi::Wave {
            tag: u16_(0),
            channels: u16_(2),
            rate: u32_(4),
            avg: u32_(8),
            block: u16_(12),
            bits: u16_(14),
            cb: u16_(16),
            valid: u16_(18),
            sub: (f[28..40] == [0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]).then(|| u32_(24)),
        };
        assert_eq!((w.tag, w.cb, w.sub, u32_(20)), (0xfffe, 22, Some(1), 3));
        assert_eq!(crate::wasapi::shared_supported(&w), Ok(crate::wasapi::Done::Ok));
    }
}

#[cfg(test)]
#[path = "mmdevapi_snapshot_tests.rs"]
mod snapshot_tests;
