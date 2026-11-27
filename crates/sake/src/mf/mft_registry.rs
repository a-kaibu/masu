//! Registry-backed Media Foundation transform metadata and legacy enumeration.

use super::*;
use crate::{misc, rtlstr};

const TRANSFORMS: &str = "hkcr\\mediafoundation\\transforms";
const CATEGORIES: &str = "hkcr\\mediafoundation\\transforms\\categories";
const TYPE_SIZE: u32 = 32;
const REG_SZ: u32 = 1;
const REG_BINARY: u32 = 3;
const REG_DWORD: u32 = 4;
const SYNCMFT: u32 = 0x01;
const MODE_FLAGS: u32 = SYNCMFT | 0x02 | 0x04; // synchronous, asynchronous, hardware
const OPT_IN_FLAGS: u32 = 0x08 | 0x20; // field of use, transcode only

fn registry_name(id: &Guid) -> String {
    rtlstr::guid_text(id).trim_matches(['{', '}']).to_owned()
}

fn transform_key(id: &Guid) -> String {
    format!("{TRANSFORMS}\\{}", registry_name(id))
}

fn reg_data(path: &str, name: &str, ty: u32) -> Option<Vec<u8>> {
    misc::registry_value(path, name).filter(|(kind, _)| *kind == ty).map(|(_, data)| data)
}

fn utf16z(units: &[u16]) -> Vec<u8> {
    units.iter().copied().chain([0]).flat_map(u16::to_le_bytes).collect()
}

fn registered_types(ptr: u32, count: u32) -> Result<Option<Vec<u8>>, u32> {
    if ptr == 0 || count == 0 {
        return Ok(None);
    }
    let bytes = count.checked_mul(TYPE_SIZE).filter(|&n| n <= MAX_BLOB)
        .ok_or(Hr::OutOfMemory.code())?;
    mem::range(ptr, bytes).ok_or(Hr::InvalidArg.code())?;
    Ok(Some(mem::bytes(ptr, bytes)))
}

// IMFA blob: header, fixed-width items, then the GUID, string and blob payloads.
// The item slot contains a scalar or the size and absolute offset of its payload.
fn serialize_attributes(store: &Store) -> Vec<u8> {
    let mut items: Vec<_> = store.items.iter()
        .filter(|(_, value)| !matches!(value, Val::Unk(_)))
        .collect();
    items.sort_by_cached_key(|(key, _)| registry_name(key));
    let mut data = Vec::new();
    data.extend_from_slice(b"AFMI");
    data.extend_from_slice(&(items.len() as u32).to_le_bytes());
    let payload_base = 8 + items.len() * 32;
    let mut payload = Vec::new();
    for (key, value) in items {
        data.extend_from_slice(key);
        data.extend_from_slice(&u64::from(value.vt()).to_le_bytes());
        let scalar = match value {
            Val::U32(n) => Some(u64::from(*n)),
            Val::U64(n) => Some(*n),
            Val::F64(n) => Some(n.to_bits()),
            _ => None,
        };
        if let Some(n) = scalar {
            data.extend_from_slice(&n.to_le_bytes());
            continue;
        }
        let offset = payload_base + payload.len();
        match value {
            Val::Guid(id) => payload.extend_from_slice(id),
            Val::Str(units) => payload.extend(utf16z(units)),
            Val::Blob(bytes) => payload.extend_from_slice(bytes),
            _ => unreachable!(),
        }
        data.extend_from_slice(&((payload_base + payload.len() - offset) as u32).to_le_bytes());
        data.extend_from_slice(&(offset as u32).to_le_bytes());
    }
    data.extend(payload);
    data
}

fn deserialize_attributes(data: &[u8]) -> Option<Store> {
    let [b'A', b'F', b'M', b'I', a, b, c, d, ..] = *data else { return None };
    let count = u32::from_le_bytes([a, b, c, d]) as usize;
    let end = 8usize.checked_add(count.checked_mul(32)?)?;
    let mut store = Store::default();
    for item in data.get(8..end)?.chunks_exact(32) {
        let key = item[..16].try_into().ok()?;
        let ty = u64::from_le_bytes(item[16..24].try_into().ok()?);
        let scalar = u64::from_le_bytes(item[24..32].try_into().ok()?);
        let payload = || {
            let start = (scalar >> 32) as usize;
            if start < end {
                return None;
            }
            data.get(start..start.checked_add(scalar as u32 as usize)?)
        };
        let value = match ty {
            19 => Val::U32(scalar as u32),
            21 => Val::U64(scalar),
            5 => Val::F64(f64::from_bits(scalar)),
            72 => Val::Guid(payload()?.try_into().ok()?),
            31 => {
                let mut units = payload()?.chunks_exact(2);
                let mut text: Vec<_> = units.by_ref()
                    .map(|x| u16::from_le_bytes([x[0], x[1]])).collect();
                if !units.remainder().is_empty() || text.pop() != Some(0) {
                    return None;
                }
                Val::Str(text)
            }
            0x1011 => Val::Blob(payload()?.to_vec()),
            _ => return None,
        };
        store.set(key, value);
    }
    Some(store)
}

fn register_transform(c: &Ctx) -> Result<(), u32> {
    let x64 = mem::x64();
    let clsid = mem::array::<16>(c.byval(0));
    let category = mem::array::<16>(c.byval(if x64 { 1 } else { 4 }));
    let n = if x64 { 2 } else { 8 };
    let name = c.a(n);
    if name == 0 {
        return Err(Hr::InvalidArg.code());
    }
    let mut values = vec![(String::new(), REG_SZ, utf16z(&mem::wstr(name)))];
    let flags = c.a(n + 1);
    if flags != 0 {
        values.push(("MFTFlags".into(), REG_DWORD, flags.to_le_bytes().to_vec()));
    }
    for (key, index) in [("InputTypes", n + 2), ("OutputTypes", n + 4)] {
        if let Some(types) = registered_types(c.a(index + 1), c.a(index))? {
            values.push((key.into(), REG_BINARY, types));
        }
    }
    if c.a(n + 6) != 0 {
        let attributes = store_of(c.a(n + 6)).ok_or(Hr::NotImpl.code())?;
        values.push(("Attributes".into(), REG_BINARY, serialize_attributes(&attributes)));
    }
    let path = transform_key(&clsid);
    let member = format!("{CATEGORIES}\\{}\\{}", registry_name(&category), registry_name(&clsid));
    misc::registry_write_keys([(path.as_str(), values), (member.as_str(), Vec::new())]);
    Ok(())
}

fn MFTRegister(c: &mut Ctx) -> u32 {
    register_transform(c).err().unwrap_or(0)
}

fn MFTUnregister(c: &mut Ctx) -> u32 {
    let clsid = mem::array::<16>(c.byval(0));
    misc::registry_remove_tree_and_memberships(&transform_key(&clsid), CATEGORIES, &registry_name(&clsid));
    0
}

fn task_mem(data: &[u8]) -> Result<u32, u32> {
    if data.is_empty() {
        return Ok(0);
    }
    let p = mem::heap_alloc(lo32(data.len()), false);
    if p == 0 {
        return Err(Hr::OutOfMemory.code());
    }
    mem::write(p, data);
    Ok(p)
}

fn get_types(path: &str, key: &str) -> Vec<u8> {
    reg_data(path, key, REG_BINARY).filter(|data| data.len() % TYPE_SIZE as usize == 0)
        .unwrap_or_default()
}

fn MFTGetInfo(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::MFTGetInfo>();
    let clsid = mem::array::<16>(c.ctx_ref().byval(0));
    let path = transform_key(&clsid);
    let name = c.out_arch::<1, 4>();
    let input = c.out_arch::<2, 5>();
    let output = c.out_arch::<4, 7>();
    let attributes = c.out_arch::<6, 9>();
    let input_count = c.a_arch::<3, 6>();
    let output_count = c.a_arch::<5, 8>();
    for out in [name, input, output, attributes] {
        out.put(0);
    }
    if input_count != 0 {
        w32(input_count, 0);
    }
    if output_count != 0 {
        w32(output_count, 0);
    }
    let Some(friendly_name) = reg_data(&path, "", REG_SZ) else {
        return Hr::FileNotFound.code();
    };
    let store = if attributes.is_null() {
        None
    } else if let Some(data) = reg_data(&path, "Attributes", REG_BINARY) {
        let Some(store) = deserialize_attributes(&data) else { return Hr::InvalidArg.code() };
        Some(store)
    } else {
        Some(Store::default())
    };
    let data = [
        if name.is_null() { Vec::new() } else { friendly_name },
        if input.is_null() { Vec::new() } else { get_types(&path, "InputTypes") },
        if output.is_null() { Vec::new() } else { get_types(&path, "OutputTypes") },
    ];
    let mut ptrs = [0; 3];
    for (ptr, bytes) in ptrs.iter_mut().zip(&data) {
        match task_mem(bytes) {
            Ok(p) => *ptr = p,
            Err(hr) => {
                for p in ptrs.into_iter().filter(|&p| p != 0) {
                    mem::heap_free(p);
                }
                return hr;
            }
        }
    }
    let attrs_ptr = if let Some(store) = store {
        let p = new_attrs();
        attrs(p, |attr| attr.1 = store);
        p
    } else {
        0
    };
    name.put_addr(ptrs[0]);
    input.put_addr(ptrs[1]);
    output.put_addr(ptrs[2]);
    attributes.put_addr(attrs_ptr);
    if input_count != 0 {
        w32(input_count, lo32(data[1].len()) / TYPE_SIZE);
    }
    if output_count != 0 {
        w32(output_count, lo32(data[2].len()) / TYPE_SIZE);
    }
    0
}

fn MFTEnum(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::MFTEnum>();
    let category = mem::array::<16>(c.ctx_ref().byval(0));
    let list = c.out_arch::<5, 8>();
    let count = c.a_arch::<6, 9>();
    if list.is_null() || count == 0 {
        return Hr::InvalidArg.code();
    }
    list.put(0);
    w32(count, 0);
    let input = c.a_arch::<2, 5>();
    let output = c.a_arch::<3, 6>();
    let mut filters = Vec::new();
    for (key, ptr) in [("InputTypes", input), ("OutputTypes", output)] {
        if ptr != 0 {
            if mem::range(ptr, TYPE_SIZE).is_none() {
                return Hr::InvalidArg.code();
            }
            filters.push((key, mem::array::<32>(ptr)));
        }
    }
    let category = format!("{CATEGORIES}\\{}", registry_name(&category));
    let mut ids = Vec::new();
    for member in misc::reg_subkeys(&category) {
        let key = format!("{TRANSFORMS}\\{member}");
        let flags = reg_data(&key, "MFTFlags", REG_DWORD)
            .and_then(|data| data.try_into().ok())
            .map(u32::from_le_bytes)
            .unwrap_or(0);
        let mode = flags & MODE_FLAGS;
        if (mode != 0 && mode & SYNCMFT == 0) || flags & OPT_IN_FLAGS != 0 {
            continue;
        }
        if reg_data(&key, "", REG_SZ).is_none() ||
            !filters.iter().all(|(name, filter)| get_types(&key, name).chunks_exact(TYPE_SIZE as usize)
                .any(|entry| entry == filter))
        {
            continue;
        }
        let text: Vec<_> = format!("{{{member}}}").encode_utf16().collect();
        if let Some(clsid) = rtlstr::guid_from(&text) {
            ids.extend_from_slice(&clsid);
        }
    }
    match task_mem(&ids) {
        Ok(p) => {
            list.put_addr(p);
            w32(count, lo32(ids.len() / 16));
            0
        }
        Err(hr) => hr,
    }
}

pub(super) fn init() {
    implement("mfplat", "MFTRegister", MFTRegister);
    implement("mfplat", "MFTUnregister", MFTUnregister);
    implement("mfplat", "MFTGetInfo", MFTGetInfo);
    implement("mfplat", "MFTEnum", MFTEnum);
}
