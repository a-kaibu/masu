//! A program's own ACM driver (acmDriverAdd with ACM_DRIVERADDF_FUNCTION) pinned before the
//! driver code changes shape: a host DriverProc logs every message with the structs it is
//! handed, read at msacmdrv.h's packed offsets written out here (x86), and answers; the API
//! calls a game makes around it (add, open, details, suggest, stream open / size / prepare /
//! convert / unprepare / reset / close, remove) and their results are lines of
//! golden/acm_driver.txt.
use super::*;
use crate::test_guest::{call_stacked as call, enter, guest_thread, snapshot};
use std::fmt::Write;

static LOG: Mutex<String> = Mutex::new(String::new());
static REFUSE_LOAD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn log(s: String) {
    let mut l = LOG.lock().unwrap();
    l.push_str(&s);
    l.push('\n');
}

/// DriverProc(dwDriverId, hdrvr, msg, lParam1, lParam2)
fn driver_proc(c: &mut Ctx) -> u32 {
    let (id, hdrvr, msg, l1, l2) = (c.a(0), c.a(1), c.a(2), c.a(3), c.a(4));
    let opened = if hdrvr & 0xffff_0000 == 0x00ae_0000 { "opened" } else { "?" };
    let r = match msg {
        1 => u32::from(!REFUSE_LOAD.load(std::sync::atomic::Ordering::Relaxed)),
        2 | 4 | 5 | 6 => 1,
        3 => {
            log(format!("  open desc cb{} fcc{:#x} ver{:#x}", r32(l2), r32(l2 + 4), r32(l2 + 12)));
            0x77
        }
        0x600a => {
            // (ACMDRIVERDETAILSW: strings after 40 bytes)
            w32(l1 + 12, 0x0042_0001);
            for (k, u) in "Vorbis".encode_utf16().chain([0]).enumerate() {
                w16(l1 + 40 + 2 * k as u32, u);
            }
            for (k, u) in "Ogg Vorbis ACM codec".encode_utf16().chain([0]).enumerate() {
                w16(l1 + 40 + 64 + 2 * k as u32, u);
            }
            log(format!("  details cb{}", r32(l1)));
            0
        }
        0x6019 => {
            log(format!("  tag details {l1:#x} {l2:#x}"));
            0
        }
        0x601b => {
            let (src, dst) = (r32(l1 + 8), r32(l1 + 16));
            log(format!("  suggest cb{} f{:#x} src tag {:#x} cb{} dst? {} cb{}", r32(l1), r32(l1 + 4), r16(src), r32(l1 + 12), dst != 0, r32(l1 + 20)));
            if r16(src) != 0x674f {
                return 512;
            }
            w16(dst, 1);
            w16(dst + 2, 2);
            w32(dst + 4, 44100);
            0
        }
        0x604c => {
            // ACMDRVSTREAMINSTANCE: cbStruct, pwfxSrc, pwfxDst, pwfltr, dwCallback, dwInstance,
            // fdwOpen, fdwDriver, dwDriver, has
            let (src, dst) = (r32(l1 + 4), r32(l1 + 8));
            log(format!("  stream open cb{} src {:#x}/{} dst {:#x}/{} filter? {} cb{:#x} inst{:#x} f{:#x} has? {}", r32(l1), r16(src), r32(src + 4), r16(dst), r16(dst + 14), r32(l1 + 12) != 0, r32(l1 + 16), r32(l1 + 20), r32(l1 + 24), r32(l1 + 36) != 0));
            w32(l1 + 32, 0x5150);
            if r16(src) == 0x674f { 0 } else { 512 }
        }
        0x604d => 0,
        0x604e => {
            log(format!("  size inst? {} cb{} f{:#x} src{} dst{}", r32(l1 + 32) == 0x5150, r32(l2), r32(l2 + 4), r32(l2 + 8), r32(l2 + 12)));
            if r32(l2 + 4) & 0xf == 0 { w32(l2 + 12, 2 * r32(l2 + 8)) } else { w32(l2 + 8, r32(l2 + 12) / 2) }
            0
        }
        0x6051 | 0x6052 | 0x6050 => {
            if l2 != 0 {
                log(format!("  {:#x} hdr cb{} st{:#x} src{} dst{} conv{:#x}", msg, r32(l2), r32(l2 + 4), r32(l2 + 16), r32(l2 + 32), r32(l2 + 44)));
            } else {
                log(format!("  {msg:#x}"));
            }
            8
        }
        0x604f => {
            // ACMDRVSTREAMHEADER: ACMSTREAMHEADER's 44 bytes, fdwConvert, padshNext, fdwDriver,
            // dwDriver, fdwPrepared, dwPrepared, pbPreparedSrc, cbPreparedSrcLength,
            // pbPreparedDst, cbPreparedDstLength
            let (src, slen, dst, dlen) = (r32(l2 + 12), r32(l2 + 16), r32(l2 + 28), r32(l2 + 32));
            log(format!("  convert cb{} st{:#x} used {}/{} conv{:#x} prep {} {} {} {}", r32(l2), r32(l2 + 4), r32(l2 + 20), r32(l2 + 36), r32(l2 + 44), r32(l2 + 68) == src, r32(l2 + 72), r32(l2 + 76) == dst, r32(l2 + 80)));
            let n = slen.min(dlen);
            for k in 0..n {
                mem::w8(dst + k, r8(src + k) ^ 0x5a);
            }
            w32(l2 + 20, n);
            w32(l2 + 36, n);
            0
        }
        m => {
            log(format!("  other {m:#x}"));
            0
        }
    };
    log(format!("msg {msg:#x} id{id:#x} {opened}: {r}"));
    r
}

#[test]
fn acm_added_driver_snapshot() {
    let _g = enter();
    guest_thread();
    LOG.lock().unwrap().clear();
    let mut out = String::new();
    let proc_ = crate::api::host_fn("acm_tests::driver_proc", driver_proc, 20);
    let (ph, n, src, dst, hdr) = (mem::heap_alloc(8, true), mem::heap_alloc(8, true), mem::heap_alloc(64, true), mem::heap_alloc(64, true), mem::heap_alloc(84, true));
    let step = |out: &mut String, what: &str, r: u32| {
        let l = std::mem::take(&mut *LOG.lock().unwrap());
        write!(out, "{l}{what}: {r}\n").unwrap();
    };
    // a driver that refuses DRV_LOAD is not added
    REFUSE_LOAD.store(true, std::sync::atomic::Ordering::Relaxed);
    let r = call(acmDriverAddA, &[ph, 0, proc_, 0, 3]);
    step(&mut out, "add refused", r);
    REFUSE_LOAD.store(false, std::sync::atomic::Ordering::Relaxed);
    for (flags, lp) in [(0u32, proc_), (3, 0), (4, 0), (7, proc_)] {
        w32(ph, 0xdead);
        let r = call(acmDriverAddW, &[ph, 0, lp, 0, flags]);
        step(&mut out, &format!("add f{flags} -> {:#x}", r32(ph)), r);
    }
    w32(ph, 0xdead);
    let r = call(acmDriverAddA, &[ph, 0, proc_, 0, 3]);
    let hadid = r32(ph);
    step(&mut out, "add", u32::from(hadid & 0xffff_0000 == ADDED) + 2 * r);
    let ri = call(acmDriverID, &[hadid, n, 0]);
    step(&mut out, &format!("id of hadid same {}", r32(n) == hadid), ri);
    let ro = call(acmDriverOpen, &[n, hadid, 0]);
    let had = r32(n);
    step(&mut out, &format!("open -> opened {} same slot {}", had & 0xffff_0000 == OPENED, had & 0xffff == hadid & 0xffff), ro);
    let ri = call(acmDriverID, &[had, n, 0]);
    step(&mut out, &format!("id of had is hadid {}", r32(n) == hadid), ri);
    for (wide, size) in [(false, 4u32), (false, 0x398), (true, 0x518), (false, 3)] {
        let p = mem::heap_alloc(0x600, true);
        mem::fill(p, 0x600, 0xcc);
        w32(p, size);
        let rd = call(if wide { acmDriverDetailsW } else { acmDriverDetailsA }, &[had, p, 0]);
        let short = if size > 60 { if wide { (0..32).map(|k| r16(p + 40 + 2 * k)).take_while(|&u| u != 0).map(|u| char::from_u32(u32::from(u)).unwrap_or('?')).collect::<String>() } else { crate::text::a2s(&mem::cstr(p + 40)) } } else { String::new() };
        step(&mut out, &format!("details w{wide} {size:#x}: cb{} pid{:#x} short {short:?} {:016x}", r32(p), r32(p + 12), crate::test_guest::hash(p, 0x600)), rd);
        mem::heap_free(p);
    }
    let rt = call(acmFormatTagDetailsW, &[had, 0x1234, 1]);
    step(&mut out, "tag details W", rt);
    // a Vorbis source (0x674f) the driver takes, another it refuses
    for tag in [0x674fu16, 0x9999] {
        mem::fill(src, 64, 0);
        w16(src, tag);
        w16(src + 2, 2);
        w32(src + 4, 44100);
        w16(src + 16, 4);
        mem::fill(dst, 64, 0);
        for had_ in [had, 0] {
            let rs = call(acmFormatSuggest, &[had_, src, dst, 50, 0x0001_0000]);
            step(&mut out, &format!("suggest {tag:#x} via {} -> {:#x}/{}/{}", if had_ == 0 { "any" } else { "had" }, r16(dst), r16(dst + 2), r32(dst + 4)), rs);
        }
        w16(dst, 1);
        w16(dst + 2, 2);
        w32(dst + 4, 44100);
        w16(dst + 12, 4);
        w16(dst + 14, 16);
        for (had_, flags) in [(had, 1u32), (had, 0), (0, 0)] {
            w32(ph, 0xdead);
            let r = call(acmStreamOpen, &[ph, had_, src, dst, 0, 0x1111, 0x2222, flags]);
            let h = r32(ph);
            step(&mut out, &format!("stream open {tag:#x} via {} f{flags} -> {}", if had_ == 0 { "any" } else { "had" }, if h == 0xdead { "-" } else if h == 0 { "0" } else { "+" }), r);
            if r != 0 || flags != 0 {
                continue;
            }
            let ri = call(acmDriverID, &[h, n, 0]);
            step(&mut out, &format!("id of stream is hadid {}", r32(n) == hadid), ri);
            for (k, sf) in [(100u32, 0u32), (100, 1)] {
                let rs = call(acmStreamSize, &[h, k, n, sf]);
                step(&mut out, &format!("size {k}/{sf} -> {}", r32(n)), rs);
            }
            let (sb, db) = (mem::heap_alloc(64, true), mem::heap_alloc(64, true));
            for k in 0..64 {
                mem::w8(sb + k, k as u8);
            }
            mem::fill(hdr, 84, 0);
            w32(hdr, 84);
            w32(hdr + 12, sb);
            w32(hdr + 16, 40);
            w32(hdr + 28, db);
            w32(hdr + 32, 64);
            let rc = call(acmStreamConvert, &[h, hdr, 0]);
            step(&mut out, "convert unprepared", rc);
            let rp_ = call(acmStreamPrepareHeader, &[h, hdr, 0x10]);
            step(&mut out, &format!("prepare st{:#x}", r32(hdr + 4)), rp_);
            let rp2 = call(acmStreamPrepareHeader, &[h, hdr, 0]);
            step(&mut out, "prepare again", rp2);
            let rc = call(acmStreamConvert, &[h, hdr, 0x30]);
            step(&mut out, &format!("convert st{:#x} used {}/{} {:016x}", r32(hdr + 4), r32(hdr + 20), r32(hdr + 36), crate::test_guest::hash(db, 64)), rc);
            w32(hdr + 32, 65);
            let rc = call(acmStreamConvert, &[h, hdr, 0]);
            step(&mut out, "convert past the prepared length", rc);
            w32(hdr + 32, 64);
            let rr = call(acmStreamReset, &[h, 0]);
            step(&mut out, "reset", rr);
            let ru = call(acmStreamUnprepareHeader, &[h, hdr, 0]);
            step(&mut out, &format!("unprepare st{:#x}", r32(hdr + 4)), ru);
            let ru = call(acmStreamUnprepareHeader, &[h, hdr, 0]);
            step(&mut out, "unprepare again", ru);
            let rc = call(acmStreamClose, &[h, 0]);
            step(&mut out, "close", rc);
            mem::heap_free(sb);
            mem::heap_free(db);
        }
    }
    let rc = call(acmDriverClose, &[had, 0]);
    step(&mut out, "driver close", rc);
    let rc = call(acmDriverClose, &[had, 0]);
    step(&mut out, "driver close again", rc);
    let rr = call(acmDriverRemove, &[hadid, 0]);
    step(&mut out, "remove", rr);
    let rr = call(acmDriverRemove, &[hadid, 0]);
    step(&mut out, "remove again", rr);
    let ro = call(acmDriverOpen, &[n, hadid, 0]);
    step(&mut out, "open removed", ro);
    let ro = call(acmDriverOpen, &[n, OPENED | 0x7777, 0]);
    step(&mut out, &format!("open an opened handle -> {:#x}", r32(n)), ro);
    for p in [ph, n, src, dst, hdr] {
        mem::heap_free(p);
    }
    // (slot numbers depend on what ran first: the lines name none)
    snapshot("acm_driver", &out);
}
