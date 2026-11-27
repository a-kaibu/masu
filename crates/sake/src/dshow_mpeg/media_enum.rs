//! IEnumMediaTypes for the MPEG decoders' pins.

use super::*;
use crate::api::Out;
use crate::hresult::{E_INVALIDARG, E_OUTOFMEMORY};

const IID_IENUM_MEDIA_TYPES: [u8; 16] =
    guid(0x89c3_10b8, 0x846b, 0x11ce, [0x97, 0xd3, 0, 0xaa, 0, 0x55, 0x59, 0x5a]);

/// The negotiated output formats are created by out_type and own their format memory.
struct Types(Vec<u32>);

impl Drop for Types {
    fn drop(&mut self) {
        for &ty in &self.0 {
            free_mt(ty);
        }
    }
}

#[derive(Clone)]
struct Enumerator {
    filter: u32,
    types: Arc<Types>,
    at: usize,
    refs: u32,
}

static ENUMERATORS: Mutex<Option<HashMap<u32, Enumerator>>> = Mutex::new(None);

fn with_enums<R>(f: impl FnOnce(&mut HashMap<u32, Enumerator>) -> R) -> R {
    let mut map = ENUMERATORS.lock().unwrap_or_else(|e| e.into_inner());
    f(map.get_or_insert_with(HashMap::new))
}

pub(super) fn enumerate(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IPin::EnumMediaTypes>();
    let out = c.out::<1>();
    if out.is_null() {
        return Hr::Pointer.code();
    }
    out.put(0);
    let Some((filter, pin)) = pin_of(c.a::<0>()) else { return Hr::Unexpected.code() };
    if !matches!(kind(filter), Some(Kind::VDec | Kind::ADec)) {
        return Hr::NotImpl.code();
    }
    let types = if pin == PinNo::P2 {
        out_type(filter, pin).into_iter().collect()
    } else {
        Vec::new()
    };
    create(Enumerator { filter, types: Arc::new(Types(types)), at: 0, refs: 1 }, out)
}

fn create(state: Enumerator, out: Out) -> u32 {
    let object = mem::heap_alloc(ps(), true);
    if object == 0 {
        return E_OUTOFMEMORY;
    }
    wp(object, vtable());
    add_ref(state.filter);
    with_enums(|map| map.insert(object, state));
    out.put_addr(object);
    0
}

fn query(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IUnknown::QueryInterface>();
    let (e, iid, out) = (c.a::<0>(), c.a::<1>(), c.out::<2>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    if is(iid, &IID_IUNKNOWN) || is(iid, &IID_IENUM_MEDIA_TYPES) {
        retain(c.ctx());
        out.put_addr(e);
        0
    } else {
        out.put(0);
        Hr::NoInterface.code()
    }
}

fn retain(c: &mut Ctx) -> u32 {
    with_enums(|map| {
        let Some(state) = map.get_mut(&c.a(0)) else { return 0 };
        state.refs += 1;
        state.refs
    })
}

fn drop_ref(c: &mut Ctx) -> u32 {
    let e = c.a(0);
    let (remaining, released) = with_enums(|map| {
        let Some(state) = map.get_mut(&e) else { return (0, None) };
        state.refs -= 1;
        if state.refs == 0 { (0, map.remove(&e)) } else { (state.refs, None) }
    });
    if let Some(state) = released {
        release(c, state.filter);
        mem::heap_free(e);
    }
    remaining
}

fn next(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IEnumMediaTypes::Next>();
    let (e, n, out, fetched) = (c.a::<0>(), c.a::<1>(), c.out::<2>(), c.a::<3>());
    if out.is_null() {
        return Hr::Pointer.code();
    }
    if fetched == 0 && n != 1 {
        return E_INVALIDARG;
    }
    let count = with_enums(|map| {
        let Some(state) = map.get_mut(&e) else { return 0 };
        let items = &state.types.0[state.at..];
        let count = items.len().min(to_usize(n));
        for (i, &ty) in items.iter().take(count).enumerate() {
            out.nth(lo32(i)).put_addr(dup_mt(ty));
        }
        state.at += count;
        lo32(count)
    });
    put32(fetched, count);
    if count == n { 0 } else { Hr::False.code() }
}

fn skip(c: &mut Ctx) -> u32 {
    with_enums(|map| {
        let Some(state) = map.get_mut(&c.a(0)) else { return Hr::Unexpected.code() };
        let n = to_usize(c.a(1));
        let count = (state.types.0.len() - state.at).min(n);
        state.at += count;
        if count == n { 0 } else { Hr::False.code() }
    })
}

fn reset(c: &mut Ctx) -> u32 {
    with_enums(|map| {
        let Some(state) = map.get_mut(&c.a(0)) else { return Hr::Unexpected.code() };
        state.at = 0;
        0
    })
}

fn clone_enum(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::iface::IEnumMediaTypes::Clone>();
    let out = c.out::<1>();
    if out.is_null() {
        return Hr::Pointer.code();
    }
    out.put(0);
    let Some(state) = with_enums(|map| map.get(&c.a::<0>()).cloned()) else {
        return Hr::Unexpected.code();
    };
    create(Enumerator { refs: 1, ..state }, out)
}

fn vtable() -> u32 {
    static TABLE: OnceLock<u32> = OnceLock::new();
    *TABLE.get_or_init(|| crate::api::vtable("IEnumMediaTypes", &[
        (query, 12), (retain, 4), (drop_ref, 4), (next, 16), (skip, 8), (reset, 4), (clone_enum, 8),
    ]))
}
