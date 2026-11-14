//! mmdevapi pinned before its code changes shape: a seeded sweep of calls on the enumerator,
//! collections, the device, its endpoint and property store (every method, IIDs and flows and
//! roles and masks in and out of range, ids right and wrong, every property key and some that
//! are not), each result and what it wrote a line of golden/mmdevapi.txt. Objects show by
//! name (the singletons) or "new" (a collection or a wasapi object).
use super::*;
use crate::mem::rp;
use crate::test_guest::{call, enter, guest_thread, snapshot, Rng};
use std::fmt::Write;

fn name(p: u32) -> String {
    if p == 0 {
        "0".into()
    } else if p == enumerator_object() {
        "enumerator".into()
    } else if p == device() {
        "device".into()
    } else if p == endpoint() {
        "endpoint".into()
    } else if p == store() {
        "store".into()
    } else if p == 0xdead_beef {
        "untouched".into()
    } else {
        "new".into()
    }
}

fn guid_at(p: u32, g: [u8; 16]) -> u32 {
    mem::write(p, &g);
    p
}

fn iids() -> Vec<[u8; 16]> {
    vec![
        guid(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]),
        guid(0xa956_64d2, 0x9614, 0x4f35, [0xa7, 0x46, 0xde, 0x8d, 0xb6, 0x36, 0x17, 0xe6]),
        guid(0x0bd7_a1be, 0x7a1a, 0x44db, [0x83, 0x97, 0xcc, 0x53, 0x92, 0x38, 0x7b, 0x5e]),
        guid(0xd666_063f, 0x1587, 0x4e43, [0x81, 0xf1, 0xb9, 0x48, 0xe8, 0x07, 0x36, 0x3f]),
        guid(0x1be0_9788, 0x6894, 0x4089, [0x85, 0x86, 0x9a, 0x2a, 0x6c, 0x26, 0x5a, 0xc5]),
        guid(0x886d_8eeb, 0x8cf2, 0x4446, [0x8d, 0x02, 0xcd, 0xba, 0x1d, 0xbd, 0xcf, 0x99]),
        // IAudioClient, IAudioSessionManager, IAudioSessionManager2, IAudioEndpointVolume
        guid(0x1cb9_ad4c, 0xdbfa, 0x4c32, [0xb1, 0x78, 0xc2, 0xf5, 0x68, 0xa7, 0x03, 0xb2]),
        guid(0xbfa9_71f1, 0x4d5e, 0x40bb, [0x93, 0x5e, 0x96, 0x70, 0x39, 0xbf, 0xbe, 0xe4]),
        guid(0x77aa_9910, 0x1bd6, 0x484f, [0x8b, 0xc7, 0x2c, 0x65, 0x4c, 0x9a, 0x9b, 0x6f]),
        guid(0x5cdf_2c82, 0x841e, 0x4546, [0x97, 0x22, 0x0c, 0xf7, 0x40, 0x78, 0x22, 0x9a]),
        // the first three fields of an IID right and the rest wrong
        guid(0xd666_063f, 0x1587, 0x4e43, [0; 8]),
        guid(0x1234_5678, 0, 0, [0; 8]),
    ]
}

#[test]
fn mmdevapi_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0x0d_e71c);
    let g = mem::heap_alloc(64, true);
    let po = mem::heap_alloc(64, true);
    let ids = mem::heap_alloc(512, true);
    let mut cols: Vec<u32> = Vec::new();
    let mut keys: Vec<([u8; 16], u32)> = KEYS.iter().map(|k| (k.0, k.1)).collect();
    keys.push((KEYS[0].0, 15));
    keys.push((guid(1, 2, 3, [4; 8]), 14));
    let iid = iids();
    CLIENTS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    writeln!(out, "create_instance: {:?} {:?}", create_instance(guid_at(g, guid(0xbcde_0395, 0xe52f, 0x467c, [0x8e, 0x3d, 0xc4, 0x57, 0x92, 0x91, 0x69, 0x2e]))).map(name), create_instance(guid_at(g, iid[0])).map(name)).unwrap();
    for i in 0..3000 {
        mem::fill(po, 64, 0);
        putp(po, 0xdead_beef);
        let null = r.below(12) == 0;
        let out_p = if null { 0 } else { po };
        let line = match r.below(17) {
            0 => {
                let k = r.below(u64::try_from(iid.len()).unwrap_or(1));
                let this = r.pick(&[enumerator_object(), device(), endpoint(), store()]);
                let p = guid_at(g, iid[usize::try_from(k).unwrap_or(0)]);
                let f: Handler = if this == enumerator_object() { en_qi_ref } else if this == device() { dev_qi } else if this == endpoint() { ep_qi } else { ps_qi };
                let hr = call(f, &[this, p, out_p]);
                format!("qi {} iid#{k}: {hr:#x} {}", name(this), name(rp(po)))
            }
            1 => {
                // (the count is global: shown against what it was)
                let was = ENUM_REFS.load(Ordering::Relaxed);
                let (a, b) = (call(en_addref, &[0]), call(en_release, &[0]));
                format!("en addref/release: +{} +{}", a.wrapping_sub(was), b.wrapping_sub(was))
            }
            2 => {
                let (flow, mask) = (r.pick(&[0u32, 1, 2, 3]), r.pick(&[0u32, 1, 2, 0xf, 0xe, 0x10]));
                let hr = call(en_enum, &[0, flow, mask, out_p]);
                let o = rp(po);
                if hr == 0 && out_p != 0 {
                    cols.push(o);
                }
                format!("enum flow={flow} mask={mask:#x}: {hr:#x} {}", name(o))
            }
            3 => {
                let (flow, role) = (r.pick(&[0u32, 1, 2, 3]), r.pick(&[0u32, 1, 2, 3]));
                let hr = call(en_default, &[0, flow, role, out_p]);
                format!("default flow={flow} role={role}: {hr:#x} {}", name(rp(po)))
            }
            4 => {
                let s = r.pick(&[DEVICE_ID, ENDPOINT_GUID, "", "{0.0.0.00000000}.{5A2C1E0B-7F3D-4E8A-9C61-3B8D0F4A2E17}"]);
                mem::put_wstr(ids, 256, &s.encode_utf16().collect::<Vec<_>>());
                let id = if r.below(10) == 0 { 0 } else { ids };
                let hr = call(en_get, &[0, id, out_p]);
                format!("get {s:?} id={}: {hr:#x} {}", id != 0, name(rp(po)))
            }
            5 | 6 => {
                let reg = r.below(2) == 0;
                let cl = r.pick(&[0u32, 0x1000, 0x2000, 0x3000]);
                let hr = call(if reg { en_register } else { en_unregister }, &[0, cl]);
                format!("{} {cl:#x}: {hr:#x}", if reg { "register" } else { "unregister" })
            }
            7 => {
                let Some(&c) = cols.last() else { continue };
                let which = r.below(5);
                match which {
                    0 => {
                        let k = r.below(u64::try_from(iid.len()).unwrap_or(1));
                        let hr = call(col_qi, &[c, guid_at(g, iid[usize::try_from(k).unwrap_or(0)]), out_p]);
                        format!("col qi iid#{k}: {hr:#x} {} refs={}", if rp(po) == c { "self".into() } else { name(rp(po)) }, r32(c + 2 * ps()))
                    }
                    1 => format!("col addref: {}", call(col_addref, &[c])),
                    2 => {
                        let n = call(col_release, &[c]);
                        if n == 0 {
                            cols.pop();
                        }
                        format!("col release: {n}")
                    }
                    3 => format!("col count: {:#x} {}", call(col_count, &[c, out_p]), r32(po)),
                    _ => {
                        let k = r.pick(&[0u32, 0, 1, 0xffff_ffff]);
                        format!("col item {k}: {:#x} {}", call(col_item, &[c, k, out_p]), name(rp(po)))
                    }
                }
            }
            8 => {
                let k = r.below(u64::try_from(iid.len()).unwrap_or(1));
                let p = if r.below(10) == 0 { 0 } else { guid_at(g, iid[usize::try_from(k).unwrap_or(0)]) };
                let hr = call(dev_activate, &[device(), p, 0x17, 0, out_p]);
                format!("activate iid#{k} p={}: {hr:#x} {}", p != 0, name(rp(po)))
            }
            9 => {
                let a = r.pick(&[0u32, 1, 2, 3, 0x1000]);
                format!("open store {a}: {:#x} {}", call(dev_store, &[device(), a, out_p]), name(rp(po)))
            }
            10 => {
                let hr = call(dev_id, &[device(), out_p]);
                let s = if hr == 0 { String::from_utf16_lossy(&mem::wstr(rp(po))) } else { String::new() };
                format!("dev id: {hr:#x} {s:?}")
            }
            11 => format!("dev state: {:#x} {}", call(dev_state, &[device(), out_p]), r32(po)),
            12 => format!("ep flow: {:#x} {}", call(ep_flow, &[endpoint(), out_p]), r32(po)),
            13 => format!("ps count: {:#x} {}", call(ps_count, &[store(), out_p]), r32(po)),
            14 => {
                let k = r.pick(&[0u32, 3, 6, 7, 0xffff_ffff]);
                let hr = call(ps_at, &[store(), k, out_p]);
                format!("ps at {k}: {hr:#x} {:x?} {}", mem::slice(po, 16), r32(po + 16))
            }
            15 => {
                let (kg, pid) = keys[usize::try_from(r.below(u64::try_from(keys.len()).unwrap_or(1))).unwrap_or(0)];
                mem::write(g, &kg);
                w32(g + 16, pid);
                let kp = if r.below(10) == 0 { 0 } else { g };
                mem::fill(po, 64, 0xcc);
                let hr = call(ps_get, &[store(), kp, out_p]);
                let vt = r16(po);
                let val = match vt {
                    31 => format!("{:?}", String::from_utf16_lossy(&mem::wstr(rp(po + 8)))),
                    19 => format!("{}", r32(po + 8)),
                    65 => format!("{} {:x?}", r32(po + 8), mem::slice(rp(po + 8 + ps()), r32(po + 8).min(64))),
                    _ => format!("{:x?}", mem::slice(po, 16)),
                };
                format!("ps get {:x?},{pid}: {hr:#x} vt={vt} {val}", &kg[..4])
            }
            _ => format!("ps set/commit: {:#x} {:#x} one={}", call(ps_denied, &[store(), 0, 0]), call(ps_denied, &[store()]), call(one, &[0])),
        };
        writeln!(out, "{i} {line}").unwrap();
    }
    for c in cols {
        while call(col_release, &[c]) != 0 {}
    }
    CLIENTS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    for p in [g, po, ids] {
        mem::heap_free(p);
    }
    snapshot("mmdevapi", &out);
}
