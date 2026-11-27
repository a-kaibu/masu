//! IMpegAudioDecoder configuration on the MPEG-1 audio decoder.

use super::*;
use crate::hresult::E_INVALIDARG;

pub(super) const IID: [u8; 16] =
    guid(0xb45d_d570, 0x3c77, 0x11d1, [0xab, 0xe1, 0, 0xa0, 0xc9, 5, 0xf3, 0x75]);

fn query(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IUnknown::QueryInterface>();
    let (this, iid, out) = (c.a::<0>(), c.a::<1>(), c.out::<2>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    let b = Word::AudioConfig.base(this);
    let interface = if is(iid, &IID) {
        this
    } else if filter_iid(iid) {
        b
    } else {
        out.put(0);
        return Hr::NoInterface.code();
    };
    add_ref(b);
    out.put_addr(interface);
    0
}

fn retain(c: &mut Ctx) -> u32 {
    add_ref(Word::AudioConfig.base(c.a(0)))
}

fn drop_ref(c: &mut Ctx) -> u32 {
    let b = Word::AudioConfig.base(c.a(0));
    release(c, b)
}

fn default_value<const DEFAULT: u32>(c: &mut Ctx) -> u32 {
    let output = c.a(1);
    if output == 0 {
        return Hr::Pointer.code();
    }
    w32(output, DEFAULT);
    0
}

// The legacy setters validate these requests. The filter still reports its negotiated
// decoder configuration, so an accepted request does not replace the default value.
fn validate<const ALLOWED: u32>(c: &mut Ctx) -> u32 {
    let value = c.a(1);
    if value >= 32 || ALLOWED & (1u32 << value) == 0 { E_INVALIDARG } else { 0 }
}

fn dual_mode(c: &mut Ctx) -> u32 {
    let b = Word::AudioConfig.base(c.a(0));
    if pin(b, PinNo::P1).peer == 0 { Hr::NotConnected.code() } else { Hr::NotImpl.code() }
}

fn set_dual_mode(c: &mut Ctx) -> u32 {
    if c.a(1) > 2 { E_INVALIDARG } else { dual_mode(c) }
}

fn audio_format(c: &mut Ctx) -> u32 {
    // MPEG1WAVEFORMAT is a packed WAVEFORMATEX followed by 22 bytes of MPEG data.
    const FORMAT_SIZE: u32 = 40;
    let b = Word::AudioConfig.base(c.a(0));
    let output = c.a(1);
    if mem::range(output, FORMAT_SIZE).is_none() {
        return Hr::Pointer.code();
    }
    let Some(format) = fwith(b, |f| {
        let mt = f.pins.at(PinNo::P1).mt;
        let (size, ptr) = mt_fmt_at();
        if mt == 0 || mem::array::<16>(mt + 44) != FORMAT_WAVE_FORMAT_EX || r32(mt + size) < FORMAT_SIZE {
            return None;
        }
        let from = rp(mt + ptr);
        mem::range(from, FORMAT_SIZE)?;
        Some(mem::array::<{ FORMAT_SIZE as usize }>(from))
    }).flatten() else {
        return Hr::NotConnected.code();
    };
    mem::write(output, &format);
    0
}

pub(super) fn vtable() -> u32 {
    static TABLE: OnceLock<u32> = OnceLock::new();
    *TABLE.get_or_init(|| crate::api::vtable("IMpegAudioDecoder", &[
        (query, 12), (retain, 4), (drop_ref, 4),
        (default_value::<1>, 8), (validate::<0x16>, 8), // frequency divisor: 1, 2, 4
        (default_value::<0>, 8), (validate::<0x01>, 8), // decoding accuracy: default only
        (default_value::<2>, 8), (validate::<0x06>, 8), // output channels: 1 or 2
        (default_value::<16>, 8), (validate::<0x10100>, 8), // word size: 8 or 16 bits
        (default_value::<0>, 8), (validate::<0x03>, 8), // integer decoding: off or on
        (dual_mode, 8), (set_dual_mode, 8),
        (audio_format, 8),
    ]))
}
