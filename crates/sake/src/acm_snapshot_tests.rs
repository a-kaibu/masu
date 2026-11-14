//! The whole built-in converter pinned before its code changes shape: a seeded sweep of format
//! pairs through acmStreamOpen / acmStreamSize / acmStreamPrepareHeader / acmStreamConvert,
//! acmFormatSuggest, the format tag details, acmMetrics and the driver handles, every answer
//! (results, the header's fields, a hash of each output buffer) a line of golden/acm.txt.
use super::*;
use crate::test_guest::{call, enter, guest_thread, hash, snapshot, Rng};
use std::fmt::Write;

const TAGS_TRIED: [u16; 6] = [PCM, IMA, MSADPCM, MP3, 0x9999, 0];
const RATES: [u32; 8] = [0, 1, 8000, 11025, 22050, 44100, 48000, 0xffff_ffff];

/// A WAVEFORMATEX (with the fields after cbSize the ADPCM and MP3 forms have) from `r`.
fn wfx(r: &mut Rng, p: u32) -> String {
    if r.below(3) != 0 {
        return wfx_valid(r, p);
    }
    let tag = r.pick(&TAGS_TRIED);
    let ch = r.pick(&[0u16, 1, 1, 2, 2, 3]);
    let rate = r.pick(&RATES);
    let bits = r.pick(&[0u16, 4, 8, 16, 16, 24]);
    let align = match r.below(4) {
        0 => ch.wrapping_mul(bits) / 8,
        1 => r.pick(&[0u16, 1, 36, 256, 512, 1024, 2048]),
        2 => (ch.max(1)).wrapping_mul(r.pick(&[36u16, 256, 512, 1024])),
        _ => r.next() as u16,
    };
    let cb = r.pick(&[0u16, 2, 12, 32]);
    let spb = match r.below(3) {
        0 => (align.saturating_sub(4 * ch.max(1))).wrapping_mul(8) / (4 * ch.max(1)) + 1,
        1 => (align.saturating_sub(7 * ch.max(1))).wrapping_mul(8) / (4 * ch.max(1)) + 2,
        _ => r.next() as u16,
    };
    let any = r.next() as u32;
    let avg = r.pick(&[0u32, 16000, 176400, any]);
    let block = r.pick(&[0u16, 104, 417, 1]);
    mem::fill(p, 64, 0);
    w16(p, tag);
    w16(p + 2, ch);
    w32(p + 4, rate);
    w32(p + 8, avg);
    w16(p + 12, align);
    w16(p + 14, bits);
    w16(p + 16, cb);
    w16(p + 18, spb);
    w16(p + 24, block);
    format!("{tag:#x}/{ch}/{rate}/{avg}/{align}/{bits}/cb{cb}/spb{spb}/blk{block}")
}

/// A format a codec takes (PCM 8/16-bit, IMA or MS ADPCM with a block that fits), mostly.
fn wfx_valid(r: &mut Rng, p: u32) -> String {
    let tag = r.pick(&[PCM, PCM, IMA, MSADPCM, MP3]);
    let ch = r.pick(&[1u16, 2]);
    if tag == MP3 {
        let rate = r.pick(&[44100u32, 48000, 22050]);
        mem::fill(p, 64, 0);
        w16(p, MP3);
        w16(p + 2, ch);
        w32(p + 4, rate);
        w32(p + 8, 16000);
        w16(p + 12, 1);
        w16(p + 16, 12);
        w16(p + 18, 1);
        w16(p + 24, r.pick(&[0u16, 417]));
        return format!("ok mp3/{ch}/{rate}/blk{}", r16(p + 24));
    }
    let rate = r.pick(&RATES[2..7]);
    let (bits, align, cb, spb) = match tag {
        PCM => {
            let bits = r.pick(&[8u16, 16]);
            (bits, ch * bits / 8, 0, 0)
        }
        IMA => {
            let align = ch * r.pick(&[256u16, 512, 1024]);
            (4, align, 2, (align - 4 * ch) * 8 / (4 * ch) + 1)
        }
        _ => {
            let align = ch * r.pick(&[256u16, 512, 1024]);
            (4, align, 32, (align - 7 * ch) * 8 / (4 * ch) + 2)
        }
    };
    let avg = u32::from(align) * rate / spb.max(1) as u32;
    mem::fill(p, 64, 0);
    w16(p, tag);
    w16(p + 2, ch);
    w32(p + 4, rate);
    w32(p + 8, avg);
    w16(p + 12, align);
    w16(p + 14, bits);
    w16(p + 16, cb);
    w16(p + 18, spb);
    format!("ok {tag:#x}/{ch}/{rate}/{avg}/{align}/{bits}/cb{cb}/spb{spb}")
}

fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x100_0000_01b3))
}

#[test]
fn acm_behaviour_snapshot() {
    // (MP3 through a test page that decodes a frame with body byte k to 1152 frames of k / 256:
    // the page once on stays on, so it is on for every run of this test)
    let _a = crate::ADEC_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _g = enter();
    guest_thread();
    kura::adec::test_page_pass(&mut |_, _| Vec::new());
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let page = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                kura::adec::test_page_pass(&mut |b, ch| (0..1152).flat_map(|_| (0..ch).map(move |c| b[4] as f32 / 256.0 + c as f32 / 1024.0)).collect());
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        })
    };
    let mut r = Rng(0x00ac_5eed);
    let mut out = String::new();
    let (src, dst, ph, hdr, n) = (mem::heap_alloc(64, true), mem::heap_alloc(64, true), mem::heap_alloc(8, true), mem::heap_alloc(84, true), mem::heap_alloc(8, true));
    let (sbuf, dbuf) = (mem::heap_alloc(0x4000, true), mem::heap_alloc(0x8000, true));
    for case in 0..2500 {
        let fs = wfx(&mut r, src);
        // (half the pairs: a destination that is the suggested one, so most open)
        let fd = if r.below(4) != 0 && call(acmFormatSuggest, &[0, src, dst, 50, 0]) == 0 {
            if r.below(3) == 0 {
                w16(dst + 14, 8);
                w16(dst + 12, r16(dst + 2));
            }
            if r.below(3) == 0 {
                w32(dst + 4, r.pick(&RATES[2..7]));
            }
            if r.below(4) == 0 {
                w16(dst + 2, 3 - r16(dst + 2));
                w16(dst + 12, r16(dst + 2) * r16(dst + 14) / 8);
            }
            format!("suggested {:#x}/{}/{}/{}/{}", r16(dst), r16(dst + 2), r32(dst + 4), r16(dst + 12), r16(dst + 14))
        } else {
            wfx(&mut r, dst)
        };
        let flags = r.pick(&[0u32, 0, 0, 1]);
        putp(ph, 0xdead);
        let ro = call(acmStreamOpen, &[ph, 0, src, dst, 0, 0, 0, flags]);
        let h = r32(ph);
        write!(out, "{case} {fs} -> {fd} f{flags}: open {ro} h{}", if h == 0xdead { "-" } else if h == 0 { "0" } else { "+" }).unwrap();
        if ro != 0 || flags != 0 {
            out.push('\n');
            continue;
        }
        // (the sizes as one hash: a line stays short)
        let mut sizes = String::new();
        for k in [0u32, 1, 2, 3, 4, 100, 512, 1024, 4096, 0x10000, 0xffff_ffff] {
            for sf in [0u32, 1] {
                w32(n, 0xcccc_cccc);
                let rs = call(acmStreamSize, &[h, k, n, sf]);
                write!(sizes, " s{k}/{sf}={rs}:{:#x}", r32(n)).unwrap();
            }
        }
        w32(n, 0xcccc_cccc);
        let rs = call(acmStreamSize, &[h, 1024, n, 0]);
        write!(out, " s1024={rs}:{:#x} sizes {:016x}", r32(n), fnv(sizes.as_bytes())).unwrap();
        for _ in 0..3 {
            let (sl, dl) = (r.pick(&[0u32, 1, 36, 255, 256, 512, 1024, 2048, 0x4000]), r.pick(&[0u32, 2, 100, 1024, 4096, 0x8000]));
            let mut b = r.bytes(sl as usize);
            // (MPEG-1 layer III 128 kbps frames, 417 bytes, body byte k: what the test page decodes)
            if r16(src) == MP3 && r.below(4) != 0 {
                let mono = r16(src + 2) == 1;
                for (k, f) in b.chunks_mut(417).enumerate() {
                    let h = [0xff, 0xfb, 0x90, if mono { 0xc0 } else { 0 }, k as u8];
                    let n = f.len().min(5);
                    f[..n].copy_from_slice(&h[..n]);
                }
            }
            // (mostly block headers the codecs take, so the decoders run)
            let (tag, ch, align) = (r16(src), usize::from(r16(src + 2).max(1)), usize::from(r16(src + 12).max(1)));
            if r.below(4) != 0 && matches!(tag, IMA | MSADPCM) {
                for k in (0..b.len()).step_by(align) {
                    for c in 0..ch {
                        let at = if tag == IMA { k + 4 * c + 2 } else { k + c };
                        if let Some(x) = b.get_mut(at) {
                            *x %= if tag == IMA { 89 } else { 7 };
                        }
                    }
                }
            }
            mem::write(sbuf, &b);
            mem::fill(dbuf, 0x8000, 0xcc);
            mem::fill(hdr, 84, 0);
            w32(hdr, 84);
            w32(hdr + 12, sbuf);
            w32(hdr + 16, sl);
            w32(hdr + 28, dbuf);
            w32(hdr + 32, dl);
            let rp_ = call(acmStreamPrepareHeader, &[h, hdr, 0]);
            let cf = r.pick(&[0u32, 4, 0x10, 0x20, 0x34]);
            let rc = call(acmStreamConvert, &[h, hdr, cf]);
            let st = r32(hdr + 4);
            let ru = call(acmStreamUnprepareHeader, &[h, hdr, 0]);
            write!(out, " | {sl}/{dl} c{cf:#x}: p{rp_} c{rc} st{st:#x}->{:#x} u{ru} used {}/{} {:08x}", r32(hdr + 4), r32(hdr + 20), r32(hdr + 36), hash(dbuf, 0x8000) as u32).unwrap();
        }
        let rcl = call(acmStreamClose, &[h, 0]);
        let again = call(acmStreamSize, &[h, 4, n, 0]);
        writeln!(out, " | close {rcl} then {again}").unwrap();
    }
    // acmFormatSuggest by itself, every cbDst edge
    for case in 0..300 {
        let fs = wfx(&mut r, src);
        mem::fill(dst, 64, 0xcc);
        let cbd = r.pick(&[0u32, 15, 16, 17, 18, 50]);
        let rs = call(acmFormatSuggest, &[0, src, dst, cbd, 0]);
        writeln!(out, "suggest {case} {fs} cb{cbd}: {rs} {:016x}", hash(dst, 64)).unwrap();
    }
    // the format tag details: by index, by tag, the largest, every cbStruct-ish layout
    for wide in [false, true] {
        for (fl, v) in [(0u32, 0u32), (0, 1), (0, 2), (0, 3), (0, 4), (0, 9), (1, 1), (1, 0x11), (1, 2), (1, 0x55), (1, 7), (2, 0), (3, 0)] {
            let p = mem::heap_alloc(0x200, true);
            mem::fill(p, 0x200, 0xcc);
            w32(p, 0x9c);
            w32(p + 4, v);
            w32(p + 8, v);
            let f = if wide { acmFormatTagDetailsW } else { acmFormatTagDetailsA };
            let rt = call(f, &[0, p, fl]);
            writeln!(out, "tagdetails w{wide} f{fl} {v:#x}: {rt} {:016x}", hash(p, 0x200)).unwrap();
            mem::heap_free(p);
        }
    }
    for m in 0..60 {
        w32(n, 0xcccc_cccc);
        let rm = call(acmMetrics, &[0, m, n]);
        writeln!(out, "metric {m}: {rm} {:#x}", r32(n)).unwrap();
    }
    for h in [0u32, DRIVER, DRIVER_MS, 0x1234, ADDED, ADDED + 5, OPENED, OPENED + 1, 0x00ac_1001] {
        w32(n, 0xcccc_cccc);
        let ri = call(acmDriverID, &[h, n, 0]);
        let id = r32(n);
        w32(n, 0xcccc_cccc);
        let ro = call(acmDriverOpen, &[n, h, 0]);
        let opened = r32(n);
        let rc = call(acmDriverClose, &[h, 0]);
        writeln!(out, "driver {h:#x}: id {ri} {id:#x} open {ro} {opened:#x} close {rc}").unwrap();
    }
    for (wide, size) in [(false, 4u32), (false, 40), (false, 0x398), (true, 4), (true, 0x518), (false, 3), (false, 0x10001)] {
        let p = mem::heap_alloc(0x600, true);
        mem::fill(p, 0x600, 0xcc);
        w32(p, size);
        let f = if wide { acmDriverDetailsW } else { acmDriverDetailsA };
        let rd = call(f, &[DRIVER_MS, p, 0]);
        writeln!(out, "details w{wide} {size:#x}: {rd} {:016x}", hash(p, 0x600)).unwrap();
        mem::heap_free(p);
    }
    for p in [src, dst, ph, hdr, n, sbuf, dbuf] {
        mem::heap_free(p);
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    page.join().unwrap();
    snapshot("acm", &out);
}
