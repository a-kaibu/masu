//! wmvcore.dll, the Windows Media Format SDK's readers: WMCreateSyncReader's IWMSyncReader(2)
//! (with the IWMProfile and IWMHeaderInfo it answers to, IWMStreamConfig / IWMMediaProps /
//! IWMOutputMediaProps for the formats, INSSBuffer for the samples) over an ASF file
//! (crates/asf) opened by name or read through the program's IStream.
//!
//! KiriKiri's krmovie (#60) reads a WMV out of the game's archive this way: OpenStream on its
//! IStream, SetReadStreamSamples(TRUE) for the compressed objects, which its own DirectShow
//! source filter hands to the WMV decoder DMO (dshow_dmo.rs) in buffers of its allocator
//! (IWMSyncReader2::SetAllocateForStream). Uncompressed output (the default) goes through
//! wmdec.rs's decoders.
//!
//! Each reader is one guest block: word 0 IWMSyncReader2, 1 IWMProfile3, 2 IWMHeaderInfo3.
//! The guest's IStream is read on the calling thread (the reader's lock held, not the table's).

#![allow(non_snake_case)]

use kome::guid::IID_IUNKNOWN;
use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::api::{implement, vtable, Ctx, Handler};
use kome::num::*;
use crate::mem::{self, ps, put32, putp, r16, r32, rp, w16, w32, w64, wp};

/// What a method of these readers fails with (HRESULTs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hr {
    NoInterface,
    Pointer,
    InvalidArg,
    Unexpected,
    NotImpl,
    OutOfMemory,
    /// NS_E_NO_MORE_SAMPLES
    NoMoreSamples,
    /// NS_E_INVALID_REQUEST
    InvalidRequest,
    /// NS_E_INVALID_OUTPUT_FORMAT
    InvalidOutputFormat,
    /// NS_E_UNRECOGNIZED_STREAM_TYPE
    UnrecognizedStreamType,
    /// ASF_E_NOTFOUND
    NotFound,
    /// ASF_E_BUFFERTOOSMALL
    BufferTooSmall,
    /// NS_E_FILE_NOT_FOUND
    FileNotFound,
    /// NS_E_INVALID_DATA
    InvalidData,
}

impl Hr {
    pub const fn code(self) -> u32 {
        use crate::hresult::{E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, E_UNEXPECTED};
        match self {
            Hr::NoInterface => E_NOINTERFACE,
            Hr::Pointer => E_POINTER,
            Hr::InvalidArg => E_INVALIDARG,
            Hr::Unexpected => E_UNEXPECTED,
            Hr::NotImpl => E_NOTIMPL,
            Hr::OutOfMemory => E_OUTOFMEMORY,
            Hr::NoMoreSamples => 0xc00d_0bcf,
            Hr::InvalidRequest => 0xc00d_002b,
            Hr::InvalidOutputFormat => 0xc00d_0bc4,
            Hr::UnrecognizedStreamType => 0xc00d_002f,
            Hr::NotFound => 0xc00d_07f0,
            Hr::BufferTooSmall => 0xc00d_07d1,
            Hr::FileNotFound => 0xc00d_001a,
            Hr::InvalidData => 0xc00d_002e,
        }
    }
}

/// WM_SF_CLEANPOINT, WM_SF_DISCONTINUITY
const WM_SF_CLEANPOINT: u32 = 1;
const WM_SF_DISCONTINUITY: u32 = 2;

use kome::guid::Guid;
use kome::guid::guid;

const fn wm(d1: u32) -> Guid {
    // {xxxxxxxx-2b2b-11d3-b36b-00c04f6108ff}
    guid(d1, 0x2b2b, 0x11d3, [0xb3, 0x6b, 0x00, 0xc0, 0x4f, 0x61, 0x08, 0xff])
}
const IID_SYNC_READER: Guid = guid(0x9397_f121, 0x7705, 0x4dc9, [0xb0, 0x49, 0x98, 0xb6, 0x98, 0x18, 0x84, 0x14]);
const IID_SYNC_READER2: Guid = guid(0xfaed_3d21, 0x1b6b, 0x4af7, [0x8c, 0xb6, 0x3e, 0x18, 0x9b, 0xbc, 0x18, 0x7b]);
const IID_PROFILE: Guid = wm(0x9640_6bdb);
const IID_PROFILE2: Guid = guid(0x07e7_2d33, 0xd94e, 0x4be7, [0x88, 0x43, 0x60, 0xae, 0x5f, 0xf7, 0xe5, 0xf5]);
const IID_PROFILE3: Guid = guid(0x00ef_96cc, 0xa461, 0x4546, [0x8b, 0xcd, 0xc9, 0xa2, 0x8f, 0x0e, 0x06, 0xf5]);
const IID_HEADER_INFO: Guid = wm(0x9640_6bda);
const IID_HEADER_INFO2: Guid = guid(0x15cf_9781, 0x454e, 0x482e, [0xb3, 0x93, 0x85, 0xfa, 0xe4, 0x87, 0xa8, 0x10]);
const IID_HEADER_INFO3: Guid = guid(0x15cc_68e3, 0x27cc, 0x4ecd, [0xb2, 0x22, 0x3f, 0x5d, 0x02, 0xd8, 0x0b, 0xd5]);
const IID_STREAM_CONFIG: Guid = wm(0x9640_6bdc);
const IID_MEDIA_PROPS: Guid = wm(0x9640_6bce);
const IID_OUTPUT_MEDIA_PROPS: Guid = wm(0x9640_6bd7);
const IID_NSSBUFFER: Guid = guid(0xe1cd_3524, 0x03d7, 0x11d2, [0x9e, 0xed, 0x00, 0x60, 0x97, 0xd2, 0xd7, 0xcf]);

/// Media types and formats (DirectShow's, which WM_MEDIA_TYPE shares).
pub const fn fourcc(d1: u32) -> Guid {
    guid(d1, 0, 0x10, [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71])
}
const fn ds(d1: u32) -> Guid {
    guid(d1, 0x524f, 0x11ce, [0x9f, 0x53, 0x00, 0x20, 0xaf, 0x0b, 0xa7, 0x70])
}
pub const MEDIATYPE_VIDEO: Guid = fourcc(0x7364_6976);
pub const MEDIATYPE_AUDIO: Guid = fourcc(0x7364_7561);
pub const SUBTYPE_RGB24: Guid = ds(0xe436_eb7d);
pub const SUBTYPE_RGB32: Guid = ds(0xe436_eb7e);
pub const SUBTYPE_PCM: Guid = fourcc(1);
pub const FORMAT_VIDEO_INFO: Guid = guid(0x0558_9f80, 0xc356, 0x11ce, [0xbf, 0x01, 0x00, 0xaa, 0x00, 0x55, 0x59, 0x5a]);
pub const FORMAT_WAVE_FORMAT_EX: Guid = guid(0x0558_9f81, 0xc356, 0x11ce, [0xbf, 0x01, 0x00, 0xaa, 0x00, 0x55, 0x59, 0x5a]);


// ---------------------------------------------------------------------------------------------
// media types

/// A WM_MEDIA_TYPE (AM_MEDIA_TYPE's layout) on the host side.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaType {
    pub major: Guid,
    pub sub: Guid,
    pub fixed: bool,
    pub temporal: bool,
    pub sample_size: u32,
    pub format_type: Guid,
    pub format: Vec<u8>,
}

/// Bytes of a WM_MEDIA_TYPE (the format block not counted).
pub fn mt_size() -> u32 {
    if mem::x64() {
        88
    } else {
        72
    }
}

impl MediaType {
    /// Writes it at `p` with its format block at `fmt` (guest memory).
    pub fn write(&self, p: u32, fmt: u32) {
        mem::write(p, &self.major);
        mem::write(p + 16, &self.sub);
        w32(p + 32, u32::from(self.fixed));
        w32(p + 36, u32::from(self.temporal));
        w32(p + 40, self.sample_size);
        mem::write(p + 44, &self.format_type);
        let k = if mem::x64() { 64 } else { 60 };
        wp(p + k, 0);
        w32(p + k + ps(), lo32(self.format.len()));
        let at = (p + k + ps() + 4).next_multiple_of(ps());
        wp(at, if self.format.is_empty() { 0 } else { fmt });
        if !self.format.is_empty() {
            mem::write(fmt, &self.format);
        }
    }
    pub fn read(p: u32) -> Option<MediaType> {
        if p == 0 {
            return None;
        }
        let k = if mem::x64() { 64 } else { 60 };
        let n = r32(p + k + ps());
        let at = (p + k + ps() + 4).next_multiple_of(ps());
        let fp = rp(at);
        let format = if n != 0 && fp != 0 && n < 1 << 20 { mem::bytes(fp, n) } else { Vec::new() };
        Some(MediaType {
            major: mem::guid_at(p)?,
            sub: mem::guid_at(p + 16)?,
            fixed: r32(p + 32) != 0,
            temporal: r32(p + 36) != 0,
            sample_size: r32(p + 40),
            format_type: mem::guid_at(p + 44)?,
            format,
        })
    }
}

/// The VIDEOINFOHEADER of a picture of `bih` (a BITMAPINFOHEADER and what follows it), at
/// `time_per_frame`.
pub fn video_info(bih: &[u8], width: u32, height: u32, bitrate: u32, time_per_frame: u64) -> Vec<u8> {
    let mut v = vec![0u8; 48];
    v[8..12].copy_from_slice(&width.to_le_bytes());
    v[12..16].copy_from_slice(&height.to_le_bytes());
    v[24..28].copy_from_slice(&width.to_le_bytes());
    v[28..32].copy_from_slice(&height.to_le_bytes());
    v[32..36].copy_from_slice(&bitrate.to_le_bytes());
    v[40..48].copy_from_slice(&time_per_frame.to_le_bytes());
    v.extend_from_slice(bih);
    v
}

/// A stream's compressed media type, as the file describes it.
pub fn stream_type(s: &asf::Stream) -> MediaType {
    match s.kind {
        asf::Kind::Audio => {
            let align = u16::from_le_bytes([s.format.get(12).copied().unwrap_or(1), s.format.get(13).copied().unwrap_or(0)]);
            MediaType { major: MEDIATYPE_AUDIO, sub: fourcc(s.codec()), fixed: true, temporal: false, sample_size: u32::from(align), format_type: FORMAT_WAVE_FORMAT_EX, format: s.format.clone() }
        }
        asf::Kind::Video { width, height } => MediaType {
            major: MEDIATYPE_VIDEO,
            sub: fourcc(s.codec()),
            fixed: false,
            temporal: true,
            sample_size: 0,
            format_type: FORMAT_VIDEO_INFO,
            format: video_info(&s.format, width, height, s.bitrate, s.avg_time_per_frame),
        },
        asf::Kind::Other(g) => MediaType { major: g, sub: [0; 16], fixed: false, temporal: false, sample_size: 0, format_type: [0; 16], format: s.format.clone() },
    }
}

// ---------------------------------------------------------------------------------------------
// the readers

thread_local! {
    /// The handler's context while a reader reads the guest's IStream.
    static CTX: Cell<*mut ()> = const { Cell::new(std::ptr::null_mut()) };
}

/// Runs `f` with `c` reachable to GuestStream::read_at.
fn with_ctx<R>(c: &mut Ctx, f: impl FnOnce() -> R) -> R {
    let old = CTX.with(|p| p.replace(std::ptr::from_mut(c).cast::<()>()));
    let r = f();
    CTX.with(|p| p.set(old));
    r
}

/// A program's IStream (a reference held), read ahead in blocks.
struct GuestStream {
    stream: u32,
    cache: (u64, Vec<u8>),
}

const BLOCK: usize = 64 << 10;

impl GuestStream {
    fn read_guest(&mut self, off: u64, n: usize) -> Vec<u8> {
        let p = CTX.with(|p| p.get());
        if p.is_null() {
            return Vec::new();
        }
        let c = unsafe { &mut *p.cast::<Ctx>() };
        let buf = mem::heap_alloc(lo32(n) + 8, false);
        // (the guest heap is full: nothing read, not a read into address 0)
        if buf == 0 {
            return Vec::new();
        }
        let got = buf + lo32(n);
        // IStream::Seek(off, STREAM_SEEK_SET, NULL), Read(buf, n, &got)
        let hr = if mem::x64() {
            let f = mem::r64(lo32(mem::r64(lo32(mem::widen(self.stream)))) + 5 * 8);
            lo32(crate::call64(c.cpu, f, &[mem::widen(self.stream), off, 0, 0]))
        } else {
            crate::dshow::com(c, self.stream, 5, &[lo32(off), lo32(off >> 32), 0, 0])
        };
        let mut out = Vec::new();
        if hr & 0x8000_0000 == 0 {
            w32(got, 0);
            let hr = crate::dshow::com(c, self.stream, 3, &[buf, lo32(n), got]);
            if hr & 0x8000_0000 == 0 {
                let k = r32(got).min(lo32(n));
                out = mem::bytes(buf, k);
            }
        }
        mem::heap_free(buf);
        out
    }
}

impl asf::Source for GuestStream {
    fn read_at(&mut self, off: u64, buf: &mut [u8]) -> usize {
        let (at, ref data) = self.cache;
        if off < at || off >= at + lo64(data.len()) {
            let n = buf.len().max(BLOCK);
            self.cache = (off, self.read_guest(off, n));
        }
        let (at, ref data) = self.cache;
        let s = us(off - at);
        let n = data.len().saturating_sub(s).min(buf.len());
        buf[..n].copy_from_slice(&data[s..s + n]);
        n
    }
}

/// A file by name.
pub(crate) struct HostFile(pub kura::fs::File);

impl asf::Source for HostFile {
    fn read_at(&mut self, off: u64, buf: &mut [u8]) -> usize {
        use std::io::{Read, Seek, SeekFrom};
        if self.0.seek(SeekFrom::Start(off)).is_err() {
            return 0;
        }
        let mut got = 0;
        while got < buf.len() {
            match self.0.read(&mut buf[got..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n,
            }
        }
        got
    }
}

pub type Src = Box<dyn asf::Source + Send>;

// (the raw guest pointers in it are only used on the guest's threads)
unsafe impl Send for GuestStream {}

/// One output (= one stream of the file, in header order).
pub struct Output {
    pub stream: asf::StreamNumber,
    /// the type the output gives
    pub ty: MediaType,
    /// read compressed (SetReadStreamSamples TRUE)
    pub compressed: bool,
    pub selected: bool,
    /// the program's IWMReaderAllocatorEx for this stream / output (a reference held)
    pub alloc_stream: u32,
    pub alloc_output: u32,
    pub dec: Option<Box<dyn crate::wmdec::Decoder>>,
    /// uncompressed samples decoded, not taken yet: time, duration, bytes, key
    pub ready: VecDeque<(u64, u64, Vec<u8>, bool)>,
    pub discontinuity: bool,
}

pub struct Reader {
    pub r: Option<asf::Reader<Src>>,
    /// the program's IStream (a reference held)
    pub stream: u32,
    pub outputs: Vec<Output>,
    /// samples read from the file, not taken yet
    pub pending: VecDeque<asf::Sample>,
    pub range: (u64, u64),
    pub ended: bool,
    /// the attribute lists asked for, by stream (built once per file, not per call)
    pub attrs: HashMap<u16, Arc<Vec<(String, asf::Value)>>>,
}

impl Reader {
    fn header(&self) -> Option<&asf::Header> {
        self.r.as_ref().map(|r| &r.header)
    }
    fn output_of(&self, stream: u8) -> Option<usize> {
        self.outputs.iter().position(|o| o.stream.get() == stream)
    }
}

struct Obj {
    refs: u32,
    r: Arc<Mutex<Reader>>,
    /// an async reader's (WMCreateReader) delivery state
    a: Option<asyncr::Async>,
}

static READERS: Mutex<Option<HashMap<u32, Obj>>> = Mutex::new(None);

/// interface words of a reader's block
const W_SYNC: u32 = 0;
const W_PROFILE: u32 = 1;
const W_HEADER: u32 = 2;
/// (an async reader's IWMReaderAdvanced2; nothing in a sync reader's)
const W_ADV: u32 = 3;
/// (an async reader's IReferenceClock)
const W_CLOCK: u32 = 4;
const WORDS: u32 = 5;

fn table<R>(f: impl FnOnce(&mut HashMap<u32, Obj>) -> R) -> R {
    f(READERS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

/// The reader block of an interface pointer of one.
fn block(p: u32) -> Option<u32> {
    table(|m| (0..WORDS).find_map(|k| p.checked_sub(ps() * k).filter(|b| m.contains_key(b))))
}

fn reader(p: u32) -> Option<Arc<Mutex<Reader>>> {
    let b = block(p)?;
    table(|m| m.get(&b).map(|o| o.r.clone()))
}

fn lock(r: &Arc<Mutex<Reader>>) -> std::sync::MutexGuard<'_, Reader> {
    r.lock().unwrap_or_else(|e| e.into_inner())
}

struct Vt {
    sync: u32,
    profile: u32,
    header: u32,
    config: u32,
    props: u32,
    buffer: u32,
}

/// IWMSyncReader2's methods and their x86 argument bytes
const M_SYNC: &[(Handler, u32)] = &[
                (qi, 12),
                (add_ref, 4),
                (release, 4),
                (sr_open, 8),
                (sr_close, 4),
                (sr_set_range, 20),
                (sr_set_range_by_frame, 24),
                (sr_get_next_sample, 32),
                (sr_set_streams_selected, 16),
                (sr_get_stream_selected, 12),
                (sr_set_read_stream_samples, 12),
                (sr_get_read_stream_samples, 12),
                (sr_get_output_setting, 24),
                (sr_set_output_setting, 24),
                (sr_get_output_count, 8),
                (sr_get_output_props, 12),
                (sr_set_output_props, 12),
                (sr_get_output_format_count, 12),
                (sr_get_output_format, 16),
                (sr_get_output_number_for_stream, 12),
                (sr_get_stream_number_for_output, 12),
                (sr_get_max_output_sample_size, 12),
                (sr_get_max_stream_sample_size, 12),
                (sr_open_stream, 8),
                // IWMSyncReader2
                (notimpl, 16),
                (notimpl, 28),
                (sr_set_allocate_for_output, 12),
                (sr_get_allocate_for_output, 12),
                (sr_set_allocate_for_stream, 12),
                (sr_get_allocate_for_stream, 12),
            ];
/// IWMProfile3's methods and their x86 argument bytes, as Microsoft's metadata has them
/// (api::vtable_iface tells them apart: the table had four methods too many from slot 30 on,
/// IWMHeaderInfo3 seven sizes wrong and one too many, which popped the wrong stack bytes on x86)
const M_PROFILE: &[(Handler, u32)] = &[
                (qi, 12),
                (add_ref, 4),
                (release, 4),
                (pr_get_version, 8),
                (pr_get_name, 12),
                (notimpl, 8),
                (pr_get_name, 12),
                (notimpl, 8),
                (pr_get_stream_count, 8),
                (pr_get_stream, 12),
                (pr_get_stream_by_number, 12),
                (notimpl, 8),
                (notimpl, 8),
                (notimpl, 8),
                (notimpl, 8),
                (notimpl, 12),
                (pr_zero_count, 8),
                (notimpl, 12),
                (notimpl, 8),
                (notimpl, 8),
                (notimpl, 8),
                // IWMProfile2::GetProfileID, IWMProfile3
                (notimpl, 8),
                (notimpl, 8),
                (notimpl, 8),
                (pr_zero_count, 8),
                (notimpl, 12),
                (notimpl, 8),
                (notimpl, 8),
                (notimpl, 8),
                (pr_no_prioritization, 8),
                (notimpl, 8),
                (notimpl, 4),
                (notimpl, 8),
                (notimpl, 16),
            ];
/// IWMHeaderInfo3's methods and their x86 argument bytes
const M_HEADER: &[(Handler, u32)] = &[
                (qi, 12),
                (add_ref, 4),
                (release, 4),
                (hi_get_attribute_count, 12),
                (hi_get_attribute_by_index, 32),
                (hi_get_attribute_by_name, 24),
                (notimpl, 24),
                (hi_zero_count, 8),
                (notimpl, 20),
                (notimpl, 16),
                (notimpl, 8),
                (hi_zero_count, 8),
                (notimpl, 28),
                (notimpl, 20),
                (notimpl, 8),
                // IWMHeaderInfo2
                (hi_zero_count, 8),
                (notimpl, 36),
                // IWMHeaderInfo3
                (hi_get_attribute_count_ex, 12),
                (notimpl, 24),
                (notimpl, 36),
                (notimpl, 28),
                (notimpl, 32),
                (notimpl, 12),
                (notimpl, 24),
            ];
/// IWMStreamConfig's methods and their x86 argument bytes
const M_CONFIG: &[(Handler, u32)] = &[
                (sc_qi, 12),
                (com_add_ref, 4),
                (sc_release, 4),
                (sc_get_stream_type, 8),
                (sc_get_stream_number, 8),
                (notimpl, 8),
                (sc_get_name, 12),
                (notimpl, 8),
                (sc_get_name, 12),
                (notimpl, 8),
                (sc_get_bitrate, 8),
                (notimpl, 8),
                (sc_get_buffer_window, 8),
                (notimpl, 8),
            ];
/// IWMOutputMediaProps's methods and their x86 argument bytes
const M_PROPS: &[(Handler, u32)] = &[
                (sc_qi, 12),
                (com_add_ref, 4),
                (sc_release, 4),
                (mp_get_type, 8),
                (mp_get_media_type, 12),
                (mp_set_media_type, 8),
                (mp_get_name, 12),
                (mp_get_name, 12),
            ];
/// INSSBuffer's methods and their x86 argument bytes
const M_BUFFER: &[(Handler, u32)] = &[(buf_qi, 12), (buf_add_ref, 4), (buf_release, 4), (buf_get_length, 8), (buf_set_length, 8), (buf_get_max_length, 8), (buf_get_buffer, 8), (buf_get_buffer_and_length, 12)];

fn vts() -> &'static Vt {
    static V: std::sync::OnceLock<Vt> = std::sync::OnceLock::new();
    V.get_or_init(|| Vt {
        sync: vtable("IWMSyncReader2", M_SYNC),
        profile: vtable("IWMProfile3", M_PROFILE),
        header: vtable("IWMHeaderInfo3", M_HEADER),
        config: vtable("IWMStreamConfig", M_CONFIG),
        props: vtable("IWMOutputMediaProps", M_PROPS),
        buffer: vtable("INSSBuffer", M_BUFFER),
    })
}

fn notimpl(_: &mut Ctx) -> u32 {
    Hr::NotImpl.code()
}

/// A new sync reader: its IWMSyncReader2.
pub fn new_sync_reader() -> u32 {
    let v = vts();
    let b = mem::heap_alloc(WORDS * ps(), true);
    if b == 0 {
        return 0;
    }
    wp(b + ps() * W_SYNC, v.sync);
    wp(b + ps() * W_PROFILE, v.profile);
    wp(b + ps() * W_HEADER, v.header);
    let r = Reader { r: None, stream: 0, outputs: Vec::new(), pending: VecDeque::new(), range: (0, 0), ended: false, attrs: HashMap::new() };
    table(|m| m.insert(b, Obj { refs: 1, r: Arc::new(Mutex::new(r)), a: None }));
    b
}

fn qi(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IUnknown::QueryInterface>();
    let (this, riid, out) = (c.a::<0>(), c.a::<1>(), c.out::<2>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    let Some(b) = block(this) else { return Hr::NoInterface.code() };
    let want = mem::guid_at(riid).unwrap_or([0xff; 16]);
    let asy = table(|m| m.get(&b).is_some_and(|o| o.a.is_some()));
    let w = if asy && [IID_IUNKNOWN, asyncr::IID_READER].contains(&want) {
        W_SYNC
    } else if asy && asyncr::IID_ADVANCED.contains(&want) {
        W_ADV
    } else if asy && want == asyncr::IID_REFERENCE_CLOCK {
        W_CLOCK
    } else if !asy && [IID_IUNKNOWN, IID_SYNC_READER, IID_SYNC_READER2].contains(&want) {
        W_SYNC
    } else if [IID_PROFILE, IID_PROFILE2, IID_PROFILE3].contains(&want) {
        W_PROFILE
    } else if [IID_HEADER_INFO, IID_HEADER_INFO2, IID_HEADER_INFO3].contains(&want) {
        W_HEADER
    } else {
        if crate::env_on!("SAKE_TRACE_COM") {
            eprintln!("sake: wmvcore reader QueryInterface {:02x?}: no", want);
        }
        out.put(0);
        return Hr::NoInterface.code();
    };
    table(|m| m.get_mut(&b).map(|o| o.refs += 1));
    out.put(b + ps() * w);
    0
}

fn add_ref(c: &mut Ctx) -> u32 {
    let Some(b) = block(c.a(0)) else { return 1 };
    table(|m| m.get_mut(&b).map(|o| {
        o.refs += 1;
        o.refs
    }))
    .unwrap_or(1)
}

fn release(c: &mut Ctx) -> u32 {
    let Some(b) = block(c.a(0)) else { return 0 };
    let (left, r) = table(|m| {
        let Some(o) = m.get_mut(&b) else { return (0, None) };
        o.refs = o.refs.saturating_sub(1);
        if o.refs > 0 {
            return (o.refs, None);
        }
        (0, m.remove(&b).map(|o| o.r))
    });
    // (the reader's block goes with its last reference; its delivery thread, if any, wakes to
    // find it gone)
    if r.is_some() {
        mem::heap_free(b);
        asyncr::wake();
    }
    if let Some(r) = r {
        close(c, &mut lock(&r));
    }
    left
}

/// Drops what the reader holds of the file and of the program.
fn close(c: &mut Ctx, r: &mut Reader) {
    let held: Vec<u32> = r.outputs.iter().flat_map(|o| [o.alloc_stream, o.alloc_output]).chain([r.stream]).filter(|&p| p != 0).collect();
    r.r = None;
    r.attrs.clear();
    r.stream = 0;
    r.outputs.clear();
    r.pending.clear();
    r.ended = false;
    for p in held {
        crate::dshow::com(c, p, 2, &[]);
    }
}

/// Opens the file in `src`: its outputs, one per stream, uncompressed by default.
fn open_src(r: &mut Reader, src: Src) -> u32 {
    let a = match asf::Reader::new(src) {
        Ok(a) => a,
        Err(asf::Error::NotAsf) => return Hr::InvalidData.code(),
        Err(_) => return Hr::InvalidData.code(),
    };
    r.outputs = a
        .header
        .streams
        .iter()
        .map(|s| {
            let ty = crate::wmdec::default_output(s).unwrap_or_else(|| stream_type(s));
            Output { stream: s.number, ty, compressed: false, selected: true, alloc_stream: 0, alloc_output: 0, dec: None, ready: VecDeque::new(), discontinuity: true }
        })
        .collect();
    r.r = Some(a);
    r.attrs.clear();
    r.pending.clear();
    r.range = (0, 0);
    r.ended = false;
    0
}

/// Open(LPCWSTR)
fn sr_open(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    if c.a(1) == 0 {
        return Hr::InvalidArg.code();
    }
    let name = String::from_utf16_lossy(&mem::wstr(c.a(1)));
    let mut r = lock(&rr);
    if r.r.is_some() {
        return Hr::Unexpected.code();
    }
    let Some(path) = crate::files::resolve(&name) else { return Hr::FileNotFound.code() };
    let Ok(f) = kura::fs::File::open(&path) else { return Hr::FileNotFound.code() };
    open_src(&mut r, Box::new(HostFile(f)))
}

/// OpenStream(IStream *)
fn sr_open_stream(c: &mut Ctx) -> u32 {
    let (this, stream) = (c.a(0), c.a(1));
    let Some(rr) = reader(this) else { return Hr::Unexpected.code() };
    if stream == 0 {
        return Hr::InvalidArg.code();
    }
    let mut r = lock(&rr);
    if r.r.is_some() {
        return Hr::Unexpected.code();
    }
    crate::dshow::com(c, stream, 1, &[]);
    let hr = with_ctx(c, || open_src(&mut r, Box::new(GuestStream { stream, cache: (0, Vec::new()) })));
    if hr != 0 {
        drop(r);
        crate::dshow::com(c, stream, 2, &[]);
        return hr;
    }
    r.stream = stream;
    0
}

fn sr_close(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let mut r = lock(&rr);
    if r.r.is_none() {
        return Hr::InvalidRequest.code();
    }
    close(c, &mut r);
    0
}

/// A QWORD argument at 32-bit slots `i`, `i + 1` (x64: argument `x`).
fn q(c: &Ctx, i: u32, x: u32) -> u64 {
    if mem::x64() {
        c.a64(x)
    } else {
        u64::from(c.a(i)) | u64::from(c.a(i + 1)) << 32
    }
}

/// SetRange(QWORD start, LONGLONG duration)
fn sr_set_range(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let (start, dur) = (q(c, 1, 1), q(c, 3, 2));
    let mut r = lock(&rr);
    let r = &mut *r;
    if r.r.is_none() {
        return Hr::InvalidRequest.code();
    }
    if (lo64(dur).cast_signed()) < 0 {
        return Hr::InvalidArg.code();
    }
    seek(c, r, start, dur);
    0
}

/// Reads from `start` on, `dur` long (0: to the end).
fn seek(c: &mut Ctx, r: &mut Reader, start: u64, dur: u64) {
    if let Some(a) = r.r.as_mut() {
        with_ctx(c, || a.seek(start));
    }
    r.pending.clear();
    r.range = (start, dur);
    r.ended = false;
    for o in &mut r.outputs {
        o.ready.clear();
        o.discontinuity = true;
        if let Some(d) = o.dec.as_mut() {
            d.flush();
        }
    }
}

/// SetRangeByFrame(WORD stream, QWORD frame, LONGLONG frames): by the stream's frame rate
fn sr_set_range_by_frame(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let (n, frame, count) = (lo8(c.a(1)), q(c, 2, 2), q(c, 4, 3));
    let t = {
        let r = lock(&rr);
        let Some(h) = r.header() else { return Hr::InvalidRequest.code() };
        let Some(s) = asf::StreamNumber::new(n).and_then(|n| h.stream(n)) else { return Hr::InvalidArg.code() };
        let per = if s.avg_time_per_frame != 0 { s.avg_time_per_frame } else { 333_667 };
        (frame.saturating_mul(per), lo64((lo64(count).cast_signed()).max(0)) * per)
    };
    let mut r = lock(&rr);
    let r = &mut *r;
    // (closed on another thread between the two locks)
    let Some(a) = r.r.as_mut() else { return Hr::InvalidRequest.code() };
    with_ctx(c, || a.seek(t.0));
    r.pending.clear();
    r.range = t;
    r.ended = false;
    for o in &mut r.outputs {
        o.ready.clear();
        o.discontinuity = true;
        if let Some(d) = o.dec.as_mut() {
            d.flush();
        }
    }
    0
}

/// Writes a new INSSBuffer of `data` (ours, or the program's allocator's) into `*out`.
fn hand_out(c: &mut Ctx, alloc: (u32, bool, u32), data: &[u8], time: u64, dur: u64, flags: u32, out: crate::api::Out) -> u32 {
    let (a, by_stream, num) = alloc;
    if a != 0 {
        // IWMReaderAllocatorEx::AllocateForStreamEx / AllocateForOutputEx(num, cb, &buf, flags,
        // time, duration, context)
        let slot = if by_stream { 3 } else { 4 };
        let pp = mem::heap_alloc(8, true);
        let hr = if mem::x64() {
            let f = rp(rp(a) + ps() * slot);
            lo32(crate::call64(c.cpu, mem::widen(f), &[mem::widen(a), u64::from(num), lo64(data.len()), u64::from(pp), u64::from(flags), time, dur, 0]))
        } else {
            crate::dshow::com(c, a, slot, &[num, lo32(data.len()), pp, flags, lo32(time), lo32(time >> 32), lo32(dur), lo32(dur >> 32), 0])
        };
        let b = rp(pp);
        mem::heap_free(pp);
        if hr & 0x8000_0000 != 0 || b == 0 {
            return if hr & 0x8000_0000 != 0 { hr } else { Hr::OutOfMemory.code() };
        }
        // GetBuffer(&p), SetLength(n)
        let p = mem::heap_alloc(8, true);
        crate::dshow::com(c, b, 6, &[p]);
        let dst = rp(p);
        mem::heap_free(p);
        if dst != 0 {
            mem::write(dst, data);
        }
        crate::dshow::com(c, b, 4, &[lo32(data.len())]);
        out.put(b);
        return 0;
    }
    give(out, new_buffer(data))
}

/// GetNextSample(WORD stream, INSSBuffer **, QWORD *time, QWORD *duration, DWORD *flags,
/// DWORD *output, WORD *stream)
fn sr_get_next_sample(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IWMSyncReader2::GetNextSample>();
    let Some(rr) = reader(c.a::<0>()) else { return Hr::Unexpected.code() };
    let want = lo16(c.a::<1>());
    let (pbuf, ptime, pdur, pflags, pout, pnum) = (c.out::<2>(), c.a::<3>(), c.a::<4>(), c.a::<5>(), c.a::<6>(), c.a::<7>());
    if pbuf.is_null() || ptime == 0 || pdur == 0 || pflags == 0 {
        return Hr::Pointer.code();
    }
    if want == 0 && (pout == 0 && pnum == 0) {
        return Hr::Pointer.code();
    }
    let mut g = lock(&rr);
    let r = &mut *g;
    if r.r.is_none() {
        return Hr::InvalidRequest.code();
    }
    if want != 0 {
        match r.output_of(lo8(want)) {
            None => return Hr::InvalidArg.code(),
            Some(k) if !r.outputs[k].selected => return Hr::InvalidRequest.code(),
            _ => {}
        }
    }
    let got = with_ctx(c.ctx(), || next_sample(r, lo8(want)));
    let Some((k, time, dur, data, key)) = got else {
        return Hr::NoMoreSamples.code();
    };
    let o = &mut r.outputs[k];
    let mut flags = if key { WM_SF_CLEANPOINT } else { 0 };
    if std::mem::replace(&mut o.discontinuity, false) {
        flags |= WM_SF_DISCONTINUITY;
    }
    let alloc = if o.alloc_stream != 0 && o.compressed {
        (o.alloc_stream, true, u32::from(o.stream))
    } else if o.alloc_output != 0 && !o.compressed {
        (o.alloc_output, false, lo32(k))
    } else {
        (0, false, 0)
    };
    let stream = o.stream;
    drop(g);
    let hr = hand_out(c.ctx(), alloc, &data, time, dur, flags, pbuf);
    if hr != 0 {
        return hr;
    }
    w64(ptime, time);
    w64(pdur, dur);
    w32(pflags, flags);
    put32(pout, lo32(k));
    if pnum != 0 {
        w16(pnum, u16::from(stream));
    }
    0
}

/// The next sample of stream `want` (0: of any selected stream): output, time, duration,
/// bytes, key.
/// The most compressed samples held for the streams not being pulled
pub const MAX_PENDING: usize = 4096;

pub fn next_sample(r: &mut Reader, want: u8) -> Option<(usize, u64, u64, Vec<u8>, bool)> {
    let end = if r.range.1 != 0 { r.range.0.saturating_add(r.range.1) } else { u64::MAX };
    loop {
        // uncompressed samples decoded already
        let ready = r.outputs.iter().enumerate().filter(|(_, o)| o.selected && !o.compressed && (want == 0 || o.stream.get() == want)).filter_map(|(k, o)| o.ready.front().map(|s| (k, s.0))).min_by_key(|x| x.1);
        // a sample from the file, of this output, before any later one
        let at = r.pending.iter().position(|s| want == 0 || s.stream.get() == want);
        if let Some((k, t)) = ready && at.is_none_or(|i| r.pending[i].time >= t) && let Some((time, dur, data, key)) = r.outputs[k].ready.pop_front() {
            return Some((k, time, dur, data, key));
        }
        if let Some(s) = at.and_then(|i| r.pending.remove(i)) {
            let k = r.output_of(s.stream.get())?;
            let dur = r.header().and_then(|h| h.stream(s.stream)).map(|st| sample_duration(st, &s)).unwrap_or(0);
            let o = &mut r.outputs[k];
            if o.compressed {
                if s.time >= end {
                    r.ended = true;
                    continue;
                }
                return Some((k, s.time, dur, s.data, s.key));
            }
            // decode it; what it gives comes out by time
            // (the header is only looked at for a new decoder, and borrowed, not copied)
            if r.outputs[k].dec.is_none() {
                let dec = r.r.as_ref()?.header.stream(s.stream).and_then(|st| crate::wmdec::decoder(st, &r.outputs[k].ty));
                r.outputs[k].dec = dec;
            }
            let o = &mut r.outputs[k];
            let Some(d) = o.dec.as_mut() else { continue };
            let start = r.range.0;
            for (t, dur, data) in d.decode(&s) {
                if t + dur <= start && dur != 0 || t >= end {
                    continue;
                }
                o.ready.push_back((t, dur, data, true));
            }
            continue;
        }
        if r.ended {
            return None;
        }
        // more from the file
        let a = r.r.as_mut()?;
        match a.next_sample() {
            Some(s) => {
                let sel = r.outputs.iter().any(|o| o.stream == s.stream && o.selected);
                if sel {
                    // (a program pulling one stream leaves the others' samples here: past
                    // MAX_PENDING the oldest of another stream goes, not the whole track kept)
                    if r.pending.len() >= MAX_PENDING {
                        let i = r.pending.iter().position(|x| want != 0 && x.stream.get() != want).unwrap_or(0);
                        r.pending.remove(i);
                    }
                    r.pending.push_back(s);
                }
            }
            None => {
                // the end: what the decoders hold back comes out
                r.ended = true;
                for o in &mut r.outputs {
                    if let Some(d) = o.dec.as_mut() {
                        for (t, dur, data) in d.drain() {
                            if t < end {
                                o.ready.push_back((t, dur, data, true));
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A compressed sample's duration: a frame's (video), its bytes' time (audio).
pub fn sample_duration(s: &asf::Stream, x: &asf::Sample) -> u64 {
    let bps = u64::from(u32::from_le_bytes(s.format.get(8..12).and_then(|b| b.try_into().ok()).unwrap_or([0; 4])));
    match s.kind {
        asf::Kind::Video { .. } => s.avg_time_per_frame,
        asf::Kind::Audio if bps != 0 => lo64(x.data.len()) * 10_000_000 / bps,
        asf::Kind::Audio | asf::Kind::Other(_) => 0,
    }
}

/// SetStreamsSelected(WORD count, WORD *numbers, WMT_STREAM_SELECTION *selections)
fn sr_set_streams_selected(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let (n, nums, sels) = (c.a(1) & 0xffff, c.a(2), c.a(3));
    if n == 0 || nums == 0 || sels == 0 {
        return Hr::InvalidArg.code();
    }
    let mut r = lock(&rr);
    if r.r.is_none() {
        return Hr::InvalidRequest.code();
    }
    for i in 0..n {
        let s = lo8(r16(nums + 2 * i));
        let Some(k) = r.output_of(s) else { return Hr::InvalidArg.code() };
        // WMT_OFF 0, WMT_CLEANPOINT_ONLY 1, WMT_ON 2
        r.outputs[k].selected = r32(sels + 4 * i) != 0;
    }
    0
}

fn sr_get_stream_selected(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let (s, out) = (lo8(c.a(1)), c.a(2));
    if out == 0 {
        return Hr::Pointer.code();
    }
    let r = lock(&rr);
    let Some(k) = r.output_of(s) else { return Hr::InvalidArg.code() };
    w32(out, if r.outputs[k].selected { 2 } else { 0 });
    0
}

fn sr_set_read_stream_samples(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let (s, on) = (lo8(c.a(1)), c.a(2) != 0);
    let mut r = lock(&rr);
    if r.r.is_none() {
        return Hr::InvalidRequest.code();
    }
    let Some(k) = r.output_of(s) else { return Hr::InvalidArg.code() };
    r.outputs[k].compressed = on;
    0
}

fn sr_get_read_stream_samples(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let (s, out) = (lo8(c.a(1)), c.a(2));
    if out == 0 {
        return Hr::Pointer.code();
    }
    let r = lock(&rr);
    let Some(k) = r.output_of(s) else { return Hr::InvalidArg.code() };
    w32(out, u32::from(r.outputs[k].compressed));
    0
}

/// GetOutputSetting / SetOutputSetting: the settings (e.g. "VideoSampleDurations") are taken
/// and kept to nothing
fn sr_get_output_setting(c: &mut Ctx) -> u32 {
    let _ = c;
    Hr::NotFound.code()
}
fn sr_set_output_setting(c: &mut Ctx) -> u32 {
    let _ = c;
    0
}

fn sr_get_output_count(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    if c.a(1) == 0 {
        return Hr::Pointer.code();
    }
    let r = lock(&rr);
    if r.r.is_none() {
        return Hr::InvalidRequest.code();
    }
    w32(c.a(1), lo32(r.outputs.len()));
    0
}

/// GetOutputProps(DWORD output, IWMOutputMediaProps **): a copy of its type
fn sr_get_output_props(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IWMSyncReader2::GetOutputProps>();
    let Some(rr) = reader(c.a::<0>()) else { return Hr::Unexpected.code() };
    let (k, out) = (us(c.a::<1>()), c.out::<2>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    let r = lock(&rr);
    if r.r.is_none() {
        return Hr::InvalidRequest.code();
    }
    let Some(o) = r.outputs.get(k) else { return Hr::InvalidArg.code() };
    give(out, new_props(Props::Output(o.ty.clone())))
}

/// The offered format with the type's major and sub type
pub(crate) fn chosen_format(offered: Vec<MediaType>, ty: &MediaType) -> Option<MediaType> {
    offered.into_iter().find(|f| f.major == ty.major && f.sub == ty.sub)
}

/// SetOutputProps(DWORD output, IWMOutputMediaProps *): the type it holds, when the output can
/// give it
fn sr_set_output_props(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let (k, p) = (us(c.a(1)), c.a(2));
    if p == 0 {
        return Hr::Pointer.code();
    }
    // (the program's props: ask them for their type)
    let ty = match props_type(p) {
        Some(t) => t,
        None => {
            let n = mem::heap_alloc(4, true);
            if n == 0 {
                return Hr::OutOfMemory.code();
            }
            crate::dshow::com(c, p, 4, &[0, n]);
            let size = r32(n);
            // (a type is a few hundred bytes: a size past 1 MB is no type)
            let buf = if size > 1 << 20 { 0 } else { mem::heap_alloc(size.max(mt_size()), true) };
            let t = if buf == 0 {
                None
            } else {
                w32(n, size);
                let hr = crate::dshow::com(c, p, 4, &[buf, n]);
                if hr == 0 { MediaType::read(buf) } else { None }
            };
            mem::heap_free(buf);
            mem::heap_free(n);
            match t {
                Some(t) => t,
                None => return Hr::InvalidArg.code(),
            }
        }
    };
    let mut g = lock(&rr);
    let r = &mut *g;
    let Some(h) = r.header() else { return Hr::InvalidRequest.code() };
    let Some(n) = r.outputs.get(k).map(|o| o.stream) else { return Hr::InvalidArg.code() };
    let Some(s) = h.stream(n) else { return Hr::InvalidArg.code() };
    // (only the choice among the formats offered is the program's: the format block stays
    // ours, a program's edited or short VIDEOINFOHEADER is not what pictures are cut to)
    let Some(f) = chosen_format(crate::wmdec::formats(s), &ty) else { return Hr::InvalidOutputFormat.code() };
    let o = &mut r.outputs[k];
    o.ty = f;
    o.dec = None;
    o.ready.clear();
    0
}

fn sr_get_output_format_count(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let (k, out) = (us(c.a(1)), c.a(2));
    if out == 0 {
        return Hr::Pointer.code();
    }
    let r = lock(&rr);
    let Some(h) = r.header() else { return Hr::InvalidRequest.code() };
    let Some(s) = r.outputs.get(k).and_then(|o| h.stream(o.stream)) else { return Hr::InvalidArg.code() };
    w32(out, lo32(crate::wmdec::formats(s).len()));
    0
}

fn sr_get_output_format(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IWMReader::GetOutputFormat>();
    let Some(rr) = reader(c.a::<0>()) else { return Hr::Unexpected.code() };
    let (k, i, out) = (us(c.a::<1>()), us(c.a::<2>()), c.out::<3>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    let r = lock(&rr);
    let Some(h) = r.header() else { return Hr::InvalidRequest.code() };
    let Some(s) = r.outputs.get(k).and_then(|o| h.stream(o.stream)) else { return Hr::InvalidArg.code() };
    let Some(f) = crate::wmdec::formats(s).into_iter().nth(i) else { return Hr::InvalidArg.code() };
    give(out, new_props(Props::Output(f)))
}

fn sr_get_output_number_for_stream(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let (s, out) = (lo8(c.a(1)), c.a(2));
    if out == 0 {
        return Hr::Pointer.code();
    }
    let r = lock(&rr);
    let Some(k) = r.output_of(s) else { return Hr::InvalidArg.code() };
    w32(out, lo32(k));
    0
}

fn sr_get_stream_number_for_output(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let (k, out) = (us(c.a(1)), c.a(2));
    if out == 0 {
        return Hr::Pointer.code();
    }
    let r = lock(&rr);
    let Some(o) = r.outputs.get(k) else { return Hr::InvalidArg.code() };
    w16(out, u16::from(o.stream));
    0
}

fn sr_get_max_output_sample_size(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let (k, out) = (us(c.a(1)), c.a(2));
    if out == 0 {
        return Hr::Pointer.code();
    }
    let r = lock(&rr);
    let Some(o) = r.outputs.get(k) else { return Hr::InvalidArg.code() };
    w32(out, crate::wmdec::sample_size(&o.ty).max(1));
    0
}

fn sr_get_max_stream_sample_size(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let (s, out) = (lo8(c.a(1)), c.a(2));
    if out == 0 {
        return Hr::Pointer.code();
    }
    let r = lock(&rr);
    let Some(h) = r.header() else { return Hr::InvalidRequest.code() };
    let Some(st) = asf::StreamNumber::new(s).and_then(|s| h.stream(s)) else { return Hr::InvalidArg.code() };
    // (the largest object the file can hold: its packets', or a picture's when uncompressed)
    let n = match st.kind {
        // (saturating: a header's width and height are any u32)
        asf::Kind::Video { width, height } => width.saturating_mul(height).saturating_mul(4).max(h.packet_size),
        asf::Kind::Audio | asf::Kind::Other(_) => h.packet_size.saturating_mul(4),
    };
    w32(out, n);
    0
}

fn set_alloc(c: &mut Ctx, by_stream: bool) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    let (n, a) = (c.a(1), c.a(2));
    let old = {
        let mut r = lock(&rr);
        let k = if by_stream { r.output_of(lo8(n)) } else { Some(us(n)).filter(|&k| k < r.outputs.len()) };
        let Some(k) = k else { return Hr::InvalidArg.code() };
        let o = &mut r.outputs[k];
        let slot = if by_stream { &mut o.alloc_stream } else { &mut o.alloc_output };
        std::mem::replace(slot, a)
    };
    if a != 0 {
        crate::dshow::com(c, a, 1, &[]);
    }
    if old != 0 {
        crate::dshow::com(c, old, 2, &[]);
    }
    0
}

fn get_alloc<F: crate::api::Outs>(c: &mut crate::api::Args<'_, '_, F>, by_stream: bool) -> u32 {
    let Some(rr) = reader(c.a::<0>()) else { return Hr::Unexpected.code() };
    let (n, out) = (c.a::<1>(), c.out::<2>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    let a = {
        let r = lock(&rr);
        let k = if by_stream { r.output_of(lo8(n)) } else { Some(us(n)).filter(|&k| k < r.outputs.len()) };
        let Some(k) = k else { return Hr::InvalidArg.code() };
        if by_stream { r.outputs[k].alloc_stream } else { r.outputs[k].alloc_output }
    };
    if a != 0 {
        crate::dshow::com(c.ctx(), a, 1, &[]);
    }
    out.put(a);
    0
}

fn sr_set_allocate_for_output(c: &mut Ctx) -> u32 {
    set_alloc(c, false)
}
fn sr_get_allocate_for_output(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IWMSyncReader2::GetAllocateForOutput>();
    get_alloc(c, false)
}
fn sr_set_allocate_for_stream(c: &mut Ctx) -> u32 {
    set_alloc(c, true)
}
fn sr_get_allocate_for_stream(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IWMSyncReader2::GetAllocateForStream>();
    get_alloc(c, true)
}

// ---- IWMProfile

/// WMT_VER_9_0
fn pr_get_version(c: &mut Ctx) -> u32 {
    if c.a(1) == 0 {
        return Hr::Pointer.code();
    }
    w32(c.a(1), 0x0009_0000);
    0
}

/// GetName / GetDescription(WCHAR *, DWORD *chars): empty
fn pr_get_name(c: &mut Ctx) -> u32 {
    let (buf, n) = (c.a(1), c.a(2));
    if n == 0 {
        return Hr::Pointer.code();
    }
    if buf != 0 && r32(n) >= 1 {
        w16(buf, 0);
    }
    w32(n, 1);
    0
}

/// IWMProfile3::GetStreamPrioritization(this, IWMStreamPrioritization **): none, NULL written
/// whole (it is pointer-sized: the table had it as a count, w32, and it took 12 bytes from slot
/// 30 on, against the metadata's)
fn pr_no_prioritization(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IWMProfile3::GetStreamPrioritization>();
    if c.out::<1>().is_null() {
        return Hr::Pointer.code();
    }
    c.out::<1>().put(0);
    0
}

fn pr_zero_count(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IUnknown::QueryInterface>();
    if c.a::<1>() == 0 {
        return Hr::Pointer.code();
    }
    w32(c.a::<1>(), 0);
    0
}

fn pr_get_stream_count(c: &mut Ctx) -> u32 {
    let Some(rr) = reader(c.a(0)) else { return Hr::Unexpected.code() };
    if c.a(1) == 0 {
        return Hr::Pointer.code();
    }
    let r = lock(&rr);
    let Some(h) = r.header() else { return Hr::InvalidRequest.code() };
    w32(c.a(1), lo32(h.streams.len()));
    0
}

fn pr_get_stream(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IWMProfile3::GetStream>();
    let Some(rr) = reader(c.a::<0>()) else { return Hr::Unexpected.code() };
    let (i, out) = (us(c.a::<1>()), c.out::<2>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    let r = lock(&rr);
    let Some(h) = r.header() else { return Hr::InvalidRequest.code() };
    let Some(s) = h.streams.get(i) else { return Hr::InvalidArg.code() };
    give(out, new_props(Props::Stream(s.clone())))
}

fn pr_get_stream_by_number(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IWMProfile3::GetStreamByNumber>();
    let Some(rr) = reader(c.a::<0>()) else { return Hr::Unexpected.code() };
    let (n, out) = (lo8(c.a::<1>()), c.out::<2>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    let r = lock(&rr);
    let Some(h) = r.header() else { return Hr::InvalidRequest.code() };
    let Some(s) = asf::StreamNumber::new(n).and_then(|n| h.stream(n)) else {
        out.put(0);
        return Hr::UnrecognizedStreamType.code();
    };
    give(out, new_props(Props::Stream(s.clone())))
}

// ---- IWMHeaderInfo

/// WMT_ATTR_DATATYPE: DWORD 0, STRING 1, BINARY 2, BOOL 3, QWORD 4, WORD 5, GUID 6
fn attr_bytes(v: &asf::Value) -> (u32, Vec<u8>) {
    match v {
        asf::Value::U32(x) => (0, x.to_le_bytes().to_vec()),
        asf::Value::Str(s) => (1, s.encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()).collect()),
        asf::Value::Bytes(b) => (2, b.clone()),
        asf::Value::Bool(b) => (3, u32::from(*b).to_le_bytes().to_vec()),
        asf::Value::U64(x) => (4, x.to_le_bytes().to_vec()),
        asf::Value::U16(x) => (5, x.to_le_bytes().to_vec()),
        asf::Value::Guid(g) => (6, g.to_vec()),
    }
}

/// The attributes the reader answers for `stream` (0: the file): the file's own, and those
/// Windows makes of the header (Duration, Seekable, Title ...).
pub fn attributes(h: &asf::Header, stream: u16) -> Vec<(String, asf::Value)> {
    let mut v: Vec<(String, asf::Value)> = Vec::new();
    if stream == 0 {
        v.push(("Duration".into(), asf::Value::U64(h.duration)));
        v.push(("Bitrate".into(), asf::Value::U32(h.max_bitrate)));
        v.push(("Seekable".into(), asf::Value::Bool(h.flags & 2 != 0)));
        v.push(("Stridable".into(), asf::Value::Bool(false)));
        v.push(("Broadcast".into(), asf::Value::Bool(h.flags & 1 != 0)));
        v.push(("Is_Protected".into(), asf::Value::Bool(false)));
        v.push(("Is_Trusted".into(), asf::Value::Bool(false)));
        v.push(("Signature_Name".into(), asf::Value::Str(String::new())));
        v.push(("HasAudio".into(), asf::Value::Bool(h.streams.iter().any(|s| s.kind == asf::Kind::Audio))));
        v.push(("HasImage".into(), asf::Value::Bool(false)));
        v.push(("HasScript".into(), asf::Value::Bool(false)));
        v.push(("HasVideo".into(), asf::Value::Bool(h.streams.iter().any(|s| matches!(s.kind, asf::Kind::Video { .. })))));
        v.push(("CurrentBitrate".into(), asf::Value::U32(h.max_bitrate)));
        v.push(("OptimalBitrate".into(), asf::Value::U32(h.max_bitrate)));
        v.push(("Title".into(), asf::Value::Str(h.title.clone())));
        v.push(("Author".into(), asf::Value::Str(h.author.clone())));
        v.push(("Copyright".into(), asf::Value::Str(h.copyright.clone())));
        v.push(("Description".into(), asf::Value::Str(h.description.clone())));
    }
    for a in h.attributes.iter().filter(|a| a.stream == stream) {
        if let Some(e) = v.iter_mut().find(|e| e.0 == a.name) {
            e.1 = a.value.clone();
        } else {
            v.push((a.name.clone(), a.value.clone()));
        }
    }
    v
}

fn hi_attrs(c: &Ctx, stream: u16) -> Result<Arc<Vec<(String, asf::Value)>>, u32> {
    let rr = reader(c.a(0)).ok_or(Hr::Unexpected.code())?;
    let mut g = lock(&rr);
    let r = &mut *g;
    let h = r.r.as_ref().map(|a| &a.header).ok_or(Hr::InvalidRequest.code())?;
    Ok(r.attrs.entry(stream).or_insert_with(|| Arc::new(attributes(h, stream))).clone())
}

/// GetAttributeCount(WORD stream, WORD *count)
fn hi_get_attribute_count(c: &mut Ctx) -> u32 {
    let out = c.a(2);
    if out == 0 {
        return Hr::InvalidArg.code();
    }
    match hi_attrs(c, lo16(c.a(1))) {
        Ok(v) => {
            w16(out, lo16(v.len()));
            0
        }
        Err(e) => e,
    }
}

/// GetAttributeCountEx(WORD stream, WORD *count)
fn hi_get_attribute_count_ex(c: &mut Ctx) -> u32 {
    hi_get_attribute_count(c)
}

fn hi_zero_count(c: &mut Ctx) -> u32 {
    if c.a(1) == 0 {
        return Hr::InvalidArg.code();
    }
    w32(c.a(1), 0);
    0
}

/// Writes a value: type, and the bytes into `value` (`*len` bytes there; NULL: the length only).
fn put_value(ty_p: u32, value: u32, len_p: u32, v: &asf::Value) -> u32 {
    let (ty, b) = attr_bytes(v);
    put32(ty_p, ty);
    if len_p == 0 {
        return Hr::InvalidArg.code();
    }
    let room = us(r16(len_p));
    w16(len_p, lo16(b.len()));
    if value == 0 {
        return 0;
    }
    if room < b.len() {
        return Hr::BufferTooSmall.code();
    }
    mem::write(value, &b);
    0
}

/// GetAttributeByIndex(WORD index, WORD *stream, WCHAR *name, WORD *chars, type, value, *len)
fn hi_get_attribute_by_index(c: &mut Ctx) -> u32 {
    let (i, ps_, name, chars, ty, value, len) = (us(c.a(1)), c.a(2), c.a(3), c.a(4), c.a(5), c.a(6), c.a(7));
    if ps_ == 0 || chars == 0 || len == 0 {
        return Hr::InvalidArg.code();
    }
    let v = match hi_attrs(c, r16(ps_)) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some((n, val)) = v.get(i) else { return Hr::InvalidArg.code() };
    let w: Vec<u16> = n.encode_utf16().chain([0]).collect();
    let room = us(r16(chars));
    w16(chars, lo16(w.len()));
    if name != 0 {
        if room < w.len() {
            return Hr::BufferTooSmall.code();
        }
        for (k, ch) in w.iter().enumerate() {
            w16(name + 2 * lo32(k), *ch);
        }
    }
    put_value(ty, value, len, val)
}

/// GetAttributeByName(WORD *stream, LPCWSTR name, type, BYTE *value, WORD *len)
fn hi_get_attribute_by_name(c: &mut Ctx) -> u32 {
    let (ps_, name, ty, value, len) = (c.a(1), c.a(2), c.a(3), c.a(4), c.a(5));
    if ps_ == 0 || name == 0 || ty == 0 || len == 0 {
        return Hr::InvalidArg.code();
    }
    let want = String::from_utf16_lossy(&mem::wstr(name));
    let v = match hi_attrs(c, r16(ps_)) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some((_, val)) = v.iter().find(|(n, _)| n.eq_ignore_ascii_case(&want)) else { return Hr::NotFound.code() };
    put_value(ty, value, len, val)
}

// ---------------------------------------------------------------------------------------------
// stream configs, media props, buffers

enum Props {
    /// IWMStreamConfig (and IWMMediaProps) of a stream
    Stream(asf::Stream),
    /// IWMOutputMediaProps of a type
    Output(MediaType),
}

struct PropsObj {
    refs: u32,
    p: Props,
}

static PROPS: Mutex<Option<HashMap<u32, PropsObj>>> = Mutex::new(None);

fn props<R>(f: impl FnOnce(&mut HashMap<u32, PropsObj>) -> R) -> R {
    f(PROPS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

/// A stream config's block: word 0 IWMStreamConfig, 1 IWMMediaProps; an output props' word 0.
fn new_props(p: Props) -> u32 {
    let v = vts();
    let b = mem::heap_alloc(2 * ps(), true);
    if b == 0 {
        return 0;
    }
    match p {
        Props::Stream(_) => {
            wp(b, v.config);
            wp(b + ps(), v.props);
        }
        Props::Output(_) => wp(b, v.props),
    }
    props(|m| m.insert(b, PropsObj { refs: 1, p }));
    b
}

fn props_block(p: u32) -> Option<u32> {
    props(|m| [p, p.wrapping_sub(ps())].into_iter().find(|b| m.get(b).is_some_and(|o| *b == p || matches!(o.p, Props::Stream(_)))))
}

/// The type of props `p` when they are ours.
fn props_type(p: u32) -> Option<MediaType> {
    let b = props_block(p)?;
    props(|m| match &m.get(&b)?.p {
        Props::Stream(s) => Some(stream_type(s)),
        Props::Output(t) => Some(t.clone()),
    })
}

fn sc_qi(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IUnknown::QueryInterface>();
    let (this, riid, out) = (c.a::<0>(), c.a::<1>(), c.out::<2>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    let Some(b) = props_block(this) else { return Hr::NoInterface.code() };
    let want = mem::guid_at(riid).unwrap_or([0xff; 16]);
    let stream = props(|m| matches!(m.get(&b).map(|o| &o.p), Some(Props::Stream(_))));
    let w = if want == IID_IUNKNOWN {
        Some(0)
    } else if stream && want == IID_STREAM_CONFIG {
        Some(0)
    } else if stream && want == IID_MEDIA_PROPS {
        Some(1)
    } else if !stream && (want == IID_MEDIA_PROPS || want == IID_OUTPUT_MEDIA_PROPS) {
        Some(0)
    } else {
        None
    };
    let Some(w) = w else {
        out.put(0);
        return Hr::NoInterface.code();
    };
    props(|m| m.get_mut(&b).map(|o| o.refs += 1));
    out.put(b + ps() * w);
    0
}

fn com_add_ref(c: &mut Ctx) -> u32 {
    let Some(b) = props_block(c.a(0)) else { return 1 };
    props(|m| m.get_mut(&b).map(|o| {
        o.refs += 1;
        o.refs
    }))
    .unwrap_or(1)
}

fn sc_release(c: &mut Ctx) -> u32 {
    let Some(b) = props_block(c.a(0)) else { return 0 };
    props(|m| {
        let Some(o) = m.get_mut(&b) else { return 0 };
        o.refs = o.refs.saturating_sub(1);
        if o.refs == 0 {
            m.remove(&b);
            mem::heap_free(b);
            return 0;
        }
        o.refs
    })
}

fn with_props<R>(p: u32, f: impl FnOnce(&mut Props) -> R) -> Option<R> {
    let b = props_block(p)?;
    props(|m| m.get_mut(&b).map(|o| f(&mut o.p)))
}

fn sc_get_stream_type(c: &mut Ctx) -> u32 {
    let out = c.a(1);
    if out == 0 {
        return Hr::Pointer.code();
    }
    match with_props(c.a(0), |p| match p {
        Props::Stream(s) => stream_type(s).major,
        Props::Output(t) => t.major,
    }) {
        Some(g) => {
            mem::write(out, &g);
            0
        }
        None => Hr::Unexpected.code(),
    }
}

fn sc_get_stream_number(c: &mut Ctx) -> u32 {
    let out = c.a(1);
    if out == 0 {
        return Hr::Pointer.code();
    }
    match with_props(c.a(0), |p| match p {
        Props::Stream(s) => u16::from(s.number),
        Props::Output(_) => 0,
    }) {
        Some(n) => {
            w16(out, n);
            0
        }
        None => Hr::Unexpected.code(),
    }
}

/// GetStreamName / GetConnectionName(WCHAR *, WORD *chars): "", as files without names have
fn sc_get_name(c: &mut Ctx) -> u32 {
    let (buf, n) = (c.a(1), c.a(2));
    if n == 0 {
        return Hr::Pointer.code();
    }
    if buf != 0 && r16(n) >= 1 {
        w16(buf, 0);
    }
    w16(n, 1);
    0
}

fn sc_get_bitrate(c: &mut Ctx) -> u32 {
    let out = c.a(1);
    if out == 0 {
        return Hr::Pointer.code();
    }
    let v = with_props(c.a(0), |p| match p {
        Props::Stream(s) => s.bitrate,
        Props::Output(_) => 0,
    })
    .unwrap_or(0);
    w32(out, v);
    0
}

fn sc_get_buffer_window(c: &mut Ctx) -> u32 {
    let out = c.a(1);
    if out == 0 {
        return Hr::Pointer.code();
    }
    w32(out, 3000);
    0
}

fn mp_get_type(c: &mut Ctx) -> u32 {
    sc_get_stream_type(c)
}

/// GetMediaType(WM_MEDIA_TYPE *, DWORD *size): NULL first gives the size
fn mp_get_media_type(c: &mut Ctx) -> u32 {
    let (p, n) = (c.a(1), c.a(2));
    if n == 0 {
        return Hr::Pointer.code();
    }
    let Some(t) = props_type(c.a(0)) else { return Hr::Unexpected.code() };
    let need = mt_size() + lo32(t.format.len());
    if p == 0 {
        w32(n, need);
        return 0;
    }
    if r32(n) < need {
        w32(n, need);
        return Hr::BufferTooSmall.code();
    }
    t.write(p, p + mt_size());
    w32(n, need);
    0
}

/// SetMediaType(WM_MEDIA_TYPE *)
fn mp_set_media_type(c: &mut Ctx) -> u32 {
    let Some(t) = MediaType::read(c.a(1)) else { return Hr::Pointer.code() };
    match with_props(c.a(0), |p| match p {
        Props::Output(o) => {
            *o = t;
            0
        }
        Props::Stream(_) => Hr::NotImpl.code(),
    }) {
        Some(hr) => hr,
        None => Hr::Unexpected.code(),
    }
}

/// GetStreamGroupName / GetConnectionName: ""
fn mp_get_name(c: &mut Ctx) -> u32 {
    sc_get_name(c)
}

// ---- INSSBuffer: our samples (word 0 the vtable; the bytes on the guest heap)

struct Buf {
    refs: u32,
    data: u32,
    len: u32,
    max: u32,
}

static BUFS: Mutex<Option<HashMap<u32, Buf>>> = Mutex::new(None);

fn bufs<R>(f: impl FnOnce(&mut HashMap<u32, Buf>) -> R) -> R {
    f(BUFS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

/// Puts a new object `o` into `out`: S_OK, or Hr::OutOfMemory.code() when there is none (0)
fn give(out: crate::api::Out, o: u32) -> u32 {
    out.put(o);
    if o == 0 { Hr::OutOfMemory.code() } else { 0 }
}

pub fn new_buffer(data: &[u8]) -> u32 {
    let n = lo32(data.len());
    let b = mem::heap_alloc(2 * ps(), true);
    let d = if b == 0 { 0 } else { mem::heap_alloc(n.max(1), false) };
    // (the guest heap is full: no buffer, not a write to address 0)
    if d == 0 {
        mem::heap_free(b);
        return 0;
    }
    wp(b, vts().buffer);
    mem::write(d, data);
    bufs(|m| m.insert(b, Buf { refs: 1, data: d, len: n, max: n }));
    b
}

fn buf_qi(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IUnknown::QueryInterface>();
    let (this, riid, out) = (c.a::<0>(), c.a::<1>(), c.out::<2>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    let want = mem::guid_at(riid).unwrap_or([0xff; 16]);
    if want != IID_IUNKNOWN && want != IID_NSSBUFFER {
        out.put(0);
        return Hr::NoInterface.code();
    }
    bufs(|m| m.get_mut(&this).map(|o| o.refs += 1));
    out.put(this);
    0
}

fn buf_add_ref(c: &mut Ctx) -> u32 {
    bufs(|m| m.get_mut(&c.a(0)).map(|o| {
        o.refs += 1;
        o.refs
    }))
    .unwrap_or(1)
}

fn buf_release(c: &mut Ctx) -> u32 {
    let this = c.a(0);
    let (left, free) = bufs(|m| {
        let Some(o) = m.get_mut(&this) else { return (0, None) };
        o.refs = o.refs.saturating_sub(1);
        if o.refs > 0 {
            return (o.refs, None);
        }
        (0, m.remove(&this).map(|o| o.data))
    });
    if let Some(d) = free {
        mem::heap_free(d);
        mem::heap_free(this);
    }
    left
}

fn buf_get_length(c: &mut Ctx) -> u32 {
    if c.a(1) == 0 {
        return Hr::InvalidArg.code();
    }
    let Some(n) = bufs(|m| m.get(&c.a(0)).map(|o| o.len)) else { return Hr::Unexpected.code() };
    w32(c.a(1), n);
    0
}

fn buf_set_length(c: &mut Ctx) -> u32 {
    let n = c.a(1);
    bufs(|m| match m.get_mut(&c.a(0)) {
        Some(o) if n <= o.max => {
            o.len = n;
            0
        }
        Some(_) => Hr::InvalidArg.code(),
        None => Hr::Unexpected.code(),
    })
}

fn buf_get_max_length(c: &mut Ctx) -> u32 {
    if c.a(1) == 0 {
        return Hr::InvalidArg.code();
    }
    let Some(n) = bufs(|m| m.get(&c.a(0)).map(|o| o.max)) else { return Hr::Unexpected.code() };
    w32(c.a(1), n);
    0
}

fn buf_get_buffer(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::INSSBuffer::GetBuffer>();
    if c.out::<1>().is_null() {
        return Hr::InvalidArg.code();
    }
    let Some(d) = bufs(|m| m.get(&c.a::<0>()).map(|o| o.data)) else { return Hr::Unexpected.code() };
    c.out::<1>().put(d);
    0
}

fn buf_get_buffer_and_length(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::INSSBuffer::GetBufferAndLength>();
    if c.out::<1>().is_null() || c.a::<2>() == 0 {
        return Hr::InvalidArg.code();
    }
    let Some((d, n)) = bufs(|m| m.get(&c.a::<0>()).map(|o| (o.data, o.len))) else { return Hr::Unexpected.code() };
    c.out::<1>().put(d);
    w32(c.a::<2>(), n);
    0
}

// ---------------------------------------------------------------------------------------------

pub fn init() {
    // WMCreateSyncReader(IUnknown *cert, DWORD rights, IWMSyncReader **)
    implement("wmvcore", "WMCreateSyncReader", |c| {
        let c = &mut c.args::<crate::api::outs::WMCreateSyncReader>();
        let out = c.out::<2>();
        if out.is_null() {
            return Hr::InvalidArg.code();
        }
        give(out, new_sync_reader())
    });
    // WMCreateReader(IUnknown *cert, DWORD rights, IWMReader **)
    implement("wmvcore", "WMCreateReader", |c| {
        let c = &mut c.args::<crate::api::outs::WMCreateReader>();
        let out = c.out::<2>();
        if out.is_null() {
            return Hr::InvalidArg.code();
        }
        give(out, asyncr::new_reader())
    });
}

mod asyncr;
pub fn asyncr_snap_wake() {
    asyncr::snap_wake();
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "wmvcore/snapshot_tests.rs"]
mod snapshot_tests;
