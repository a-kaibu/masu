//! dmusic pinned before its code changes shape: CoCreateInstance of the class and others,
//! IIDs right and wrong, outer objects, null outs, then a seeded sweep of every method of the
//! object, each result and what it wrote a line of golden/dmusic.txt.
use super::*;
use crate::mem::{r32, rp, w32};
use crate::test_guest::{call, enter, guest_thread, snapshot, Rng};
use std::fmt::Write;

#[test]
fn dmusic_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0x0d_0505);
    let cls = mem::heap_alloc(16, true);
    let iid = mem::heap_alloc(16, true);
    let po = mem::heap_alloc(64, true);
    let mut other = IIDS[1];
    other[15] ^= 1;
    let iids = [IIDS[0], IIDS[1], IIDS[2], IIDS[3], other, CLSID_DIRECTMUSIC, [0; 16]];
    let mut objs = Vec::new();
    for (ci, c) in [CLSID_DIRECTMUSIC, IIDS[1], [0; 16]].iter().enumerate() {
        mem::write(cls, c);
        for (ii, g) in iids.iter().enumerate() {
            for outer in [0u32, 0x1234] {
                for (iid_p, out_p) in [(iid, po), (0, po), (iid, 0)] {
                    mem::write(iid, g);
                    w32(po, 0xdead_beef);
                    let hr = create_instance(cls, outer, iid_p, crate::api::Out::at(out_p));
                    let o = rp(po);
                    if hr == Some(0) {
                        objs.push(o);
                    }
                    let shown = if o == 0xdead_beef { "untouched".into() } else if o == 0 { "0".into() } else { format!("refs={}", r32(o + mem::ps())) };
                    writeln!(out, "create cls#{ci} iid#{ii} outer={outer:#x} iid={} out={}: {hr:x?} {shown}", iid_p != 0, out_p != 0).unwrap();
                }
            }
        }
    }
    let o = objs.first().copied().unwrap_or(0);
    let slot = |k: u32| -> Handler {
        [qi, add_ref, release, enum_none, not_impl, create_port, enum_none, get_clock, ok, ok, not_found, ok, not_impl][usize::try_from(k).unwrap_or(0)]
    };
    for i in 0..600 {
        let k = u32::try_from(r.below(13)).unwrap_or(0);
        w32(po, 0xdead_beef);
        let p = if r.below(4) == 0 { 0 } else { po };
        mem::write(iid, &iids[usize::try_from(r.below(7)).unwrap_or(0)]);
        let args = [o, if k == 0 { iid } else { r.pick(&[0u32, 1, 5]) }, p, p, 0];
        let hr = call(slot(k), &args);
        writeln!(out, "{i} method {k} p={}: {hr:#x} out={:#x} refs={}", p != 0, if rp(po) == o { 1 } else { rp(po) }, r32(o + mem::ps())).unwrap();
    }
    for p in [cls, iid, po] {
        mem::heap_free(p);
    }
    snapshot("dmusic", &out);
}
