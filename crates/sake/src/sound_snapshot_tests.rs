//! sound pinned before its code changes shape: seeded sweeps of the format rules, gains, G.711,
//! the sample reader, the WAVE image helpers, DirectSound's device and buffer objects (every
//! method, sizes and flags and formats in and out of range, notifications, duplicates, 3D and
//! notify children, buffers mixed by hand), PlaySound of images in memory, and waveOut's devices
//! (opens, headers, writes, resets, positions, volumes, caps, messages), each result and what it
//! wrote a line of golden/sound_*.txt. The mixer thread is held: a buffer is mixed alone by the
//! test (so no other test's buffer is summed in, nor the order of heap addresses). Objects and
//! handles show by the order this test made them in.
use super::*;
use crate::test_guest::{call, enter, guest_thread, hash, snapshot, Rng};
use std::fmt::Write;
use std::sync::atomic::Ordering::SeqCst;

/// made-up event handles for notifications (shown as their index)
const EV0: u32 = 0xee00_0000;

fn hash_f(v: &[f32]) -> u64 {
    v.iter().flat_map(|x| x.to_bits().to_le_bytes()).fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

fn fmt_str(f: &Fmt) -> String {
    format!("{}Hz/{}ch/{}b{}{}", f.rate, f.channels, f.bits, if f.float { "f" } else { "" }, if f.law != 0 { format!("/law{}", f.law) } else { String::new() })
}

/// A WAVEFORMATEX(TENSIBLE) of seeded fields at `p`
fn seeded_wfx(r: &mut Rng, p: u32) {
    mem::fill(p, 40, 0);
    let tag = r.pick(&[1u16, 1, 3, 6, 7, 0xfffe, 0xfffe, 2, 0]);
    let ch = r.pick(&[1u16, 2, 2, 6, 0]);
    let rate = r.pick(&[22050u32, 44100, 44100, 8000, 0, 96000, 11025]);
    let bits = r.pick(&[8u16, 16, 16, 24, 32, 12, 0, 64]);
    let block = ch.wrapping_mul(bits / 8);
    let block = if r.below(10) == 0 { block.wrapping_add(1) } else { block };
    let avg = rate.wrapping_mul(u32::from(block));
    let avg = if r.below(10) == 0 { avg.wrapping_add(3) } else { avg };
    w16(p, tag);
    w16(p + 2, ch);
    w32(p + 4, rate);
    w32(p + 8, avg);
    w16(p + 12, block);
    w16(p + 14, bits);
    w16(p + 16, r.pick(&[0u16, 22, 22, 21]));
    w16(p + 18, r.pick(&[bits, bits, 24, 0, 32]));
    let sub = r.pick(&[1u32, 3, 6, 7, 2]);
    w32(p + 24, sub);
    let ks: [u8; 12] = [0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71];
    mem::write(p + 28, &ks);
    if r.below(12) == 0 {
        w32(p + 28, 0x1234);
    }
}

/// A RIFF WAVE image of seeded chunks (some broken)
fn seeded_riff(r: &mut Rng) -> Vec<u8> {
    let tag = r.pick(&[1u16, 1, 1, 2, 0x11, 3, 7]);
    let ch = r.pick(&[1u16, 2]);
    let rate = r.pick(&[22050u32, 44100, 11025]);
    let bits = if matches!(tag, 2 | 0x11) { 4 } else { r.pick(&[8u16, 16, 16, 32]) };
    let align: u16 = match tag {
        2 => 256 * ch,
        0x11 => 256 * ch,
        _ => ch * bits / 8,
    };
    let spb: u16 = match tag {
        2 => (256 - 7) * 2 + 2,
        0x11 => (256 - 4) * 2 + 1,
        _ => 0,
    };
    let mut fmt = Vec::new();
    for v in [tag, ch] {
        fmt.extend_from_slice(&v.to_le_bytes());
    }
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
    for v in [align, bits] {
        fmt.extend_from_slice(&v.to_le_bytes());
    }
    if matches!(tag, 2 | 0x11) {
        fmt.extend_from_slice(&2u16.to_le_bytes());
        fmt.extend_from_slice(&spb.to_le_bytes());
    }
    let n = usize::from(align) * usize::try_from(r.below(6)).unwrap_or(0) + usize::try_from(r.below(3)).unwrap_or(0);
    let data = r.bytes(n);
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(b"WAVE");
    if r.below(4) == 0 {
        b.extend_from_slice(b"LIST");
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&[1, 2, 3, 0]);
    }
    if r.below(15) != 0 {
        b.extend_from_slice(b"fmt ");
        b.extend_from_slice(&u32::try_from(fmt.len()).unwrap_or(0).to_le_bytes());
        b.extend_from_slice(&fmt);
    }
    if r.below(15) != 0 {
        b.extend_from_slice(b"data");
        let len = u32::try_from(data.len()).unwrap_or(0);
        b.extend_from_slice(&(if r.below(10) == 0 { len.wrapping_add(1000) } else { len }).to_le_bytes());
        b.extend_from_slice(&data);
    }
    let total = u32::try_from(b.len() - 8).unwrap_or(0);
    b[4..8].copy_from_slice(&total.to_le_bytes());
    if r.below(20) == 0 {
        b[0] = b'X';
    }
    b
}

#[test]
fn sound_pure_snapshot() {
    let _g = enter();
    guest_thread();
    let mut out = String::new();
    let mut r = Rng(0x50_0d);
    // G.711, both laws
    for alaw in [false, true] {
        for row in 0..16u8 {
            let v: Vec<i16> = (0..16u8).map(|k| g711(row * 16 + k, alaw)).collect();
            writeln!(out, "g711 alaw={alaw} {:#04x}: {v:?}", row * 16).unwrap();
        }
    }
    // gains
    for vol in [-20000, -10001, -10000, -9999, -5000, -2000, -600, -1, 0, 1, 500] {
        let g: Vec<String> = [-20000, -10001, -10000, -3000, -1, 0, 1, 3000, 10000, 10001].iter().map(|&pan| {
            let (l, r) = gains(vol, pan);
            format!("{:08x}/{:08x}", l.to_bits(), r.to_bits())
        }).collect();
        writeln!(out, "gains {vol}: {}", g.join(" ")).unwrap();
    }
    // the format rules
    for i in 0..3000 {
        let tag = r.pick(&[1u16, 3, 6, 7, 0xfffe, 2, 0x55, 0]);
        let ch = r.pick(&[0u16, 1, 2, 3, 6, 8]);
        let rate = r.pick(&[0u32, 8000, 44100, 48000, 192000, 1]);
        let bits = r.pick(&[0u16, 4, 8, 12, 16, 20, 24, 32, 64]);
        let align = if r.below(6) == 0 { r.pick(&[0u16, 1, 3, 5]) } else { ch.wrapping_mul(bits) / 8 };
        let avg = if r.below(6) == 0 { r.pick(&[0u32, 1, 100]) } else { u32::from(align).wrapping_mul(rate) };
        let ext = (tag == 0xfffe).then(|| {
            let mut g = [0u8; 16];
            g[..4].copy_from_slice(&r.pick(&[1u32, 3, 6, 7, 2, 0xff]).to_le_bytes());
            g[4..].copy_from_slice(&[0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);
            if r.below(10) == 0 {
                g[15] = 0;
            }
            (r.pick(&[22u16, 22, 21, 30]), r.pick(&[bits, bits, 0, 24, 32, 20, 64]), g)
        });
        let direct = r.below(3) == 0;
        let e = wave_format_check(tag, ch, rate, avg, align, bits, ext, direct);
        writeln!(out, "check#{i} tag={tag:#x} ch={ch} rate={rate} avg={avg} align={align} bits={bits} ext={:?} direct={direct}: {e}", ext.map(|(cb, v, g)| (cb, v, g[0], g[15]))).unwrap();
    }
    // Fmt read / write and the sample reader, over seeded formats and bytes
    let p = mem::heap_alloc(64, true);
    let q = mem::heap_alloc(64, true);
    let data = mem::heap_alloc(256 + 4, true);
    for i in 0..600 {
        seeded_wfx(&mut r, p);
        let f = Fmt::read(p);
        mem::fill(q, 64, 0xcc);
        f.write(q);
        let bytes = r.bytes(256);
        mem::write(data, &bytes);
        let s: Vec<String> = (0..4).map(|k| {
            let (l, rr) = sample_at(data, &f, k);
            format!("{:08x}/{:08x}", l.to_bits(), rr.to_bits())
        }).collect();
        // (a block past 64 bytes would read past the data: the first frame only)
        let s = if f.block() > 60 { s[..1].to_vec() } else { s };
        writeln!(out, "fmt#{i} {:x?} -> {} block={} wrote {:x?} samples {}", mem::slice(p, 28), fmt_str(&f), f.block(), mem::slice(q, 20), s.join(" ")).unwrap();
    }
    // the header flags
    for f in 0..64u32 {
        writeln!(out, "hdr flags {f:#x}: prepared {:#x} unprepared {:?}", prepared_flags(f), unprepared_flags(f)).unwrap();
    }
    // WAVE images
    for i in 0..400 {
        let img = seeded_riff(&mut r);
        let ch = wave_chunks(&img);
        let pcm = adpcm_to_pcm(img.clone());
        let h = pcm.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
        writeln!(out, "riff#{i} {} bytes: chunks {ch:?} pcm {} bytes {h:#x} same={}", img.len(), pcm.len(), pcm == img).unwrap();
    }
    // device ids, GUID text, notification lists
    let ids = [DSDEVID_DEFAULT, DEVICE_GUID, [0; 16], [1; 16]];
    let mut ids = ids.to_vec();
    let mut other = DSDEVID_DEFAULT;
    other[15] ^= 1;
    ids.push(other);
    for (k, g) in ids.iter().enumerate() {
        for sound in [false, true] {
            writeln!(out, "device_id #{k} sound={sound}: {:x?} text {}", device_id(*g, sound), guid_text(g)).unwrap();
        }
    }
    let list = mem::heap_alloc(256, true);
    for (at, k) in (list..).step_by(4).zip(0u32..) {
        if at >= list + 256 {
            break;
        }
        w32(at, k * 0x101);
    }
    for (n, wide) in [(0u32, false), (3, false), (3, true), (8, false), (8, true)] {
        writeln!(out, "notify_list {n} wide={wide}: {:x?}", notify_list(n, list, wide, r32)).unwrap();
    }
    for a in [p, q, data, list] {
        mem::heap_free(a);
    }
    snapshot("sound_pure", &out);
}

/// Buffer `key`'s mixing, alone: `steps` passes of it, each one's output and notifications,
/// then where it is
fn mix_alone(key: u32, steps: u64) -> String {
    let Some(b) = with(|d| d.bufs.remove(&key)) else { return "gone".into() };
    let mut tmp = Dev { bufs: HashMap::default(), waves: HashMap::default(), stream: -1 };
    tmp.bufs.insert(key, b);
    let mut s = String::new();
    for _ in 0..steps {
        let mut o = vec![0f32; STEP * 2];
        let ev = mix(&mut tmp, &mut o);
        let ev: Vec<u32> = ev.iter().map(|e| e.wrapping_sub(EV0)).collect();
        write!(s, " [{:#x} ev={ev:?}]", hash_f(&o)).unwrap();
    }
    if let Some(b) = tmp.bufs.remove(&key) {
        write!(s, " pos={} playing={} looping={}", b.pos, b.playing, b.looping).unwrap();
        with(|d| d.bufs.insert(key, b));
    }
    s
}

/// Names objects by the order they were made in; a released one's entry is 0 (its address
/// can come back for a new object)
struct Names(Vec<u32>);
impl Names {
    fn add(&mut self, o: u32) {
        self.0.push(o);
    }
    fn drop_(&mut self, o: u32) {
        self.0.iter_mut().filter(|x| **x == o).for_each(|x| *x = 0);
    }
    fn of(&self, o: u32) -> String {
        if o == 0 {
            return "0".into();
        }
        self.0.iter().position(|&x| x == o).map_or_else(|| "?".into(), |i| format!("#{i}"))
    }
}

struct HeldMixer;
impl HeldMixer {
    fn new() -> HeldMixer {
        MIXER_HELD.store(true, SeqCst);
        HeldMixer
    }
}
impl Drop for HeldMixer {
    fn drop(&mut self) {
        MIXER_HELD.store(false, SeqCst);
    }
}

#[test]
fn sound_dsound_snapshot() {
    let _g = enter();
    guest_thread();
    let _held = HeldMixer::new();
    crate::mixer::SESSION.set(0, 0xffff).ok();
    crate::mixer::SESSION.set(1, 0).ok();
    // (a PlaySound another test left stops first: SND_NOSTOP looks at it)
    play_sound(0, 0, 0, false);
    let mut out = String::new();
    let mut r = Rng(0x0d_5b0f);
    let po = mem::heap_alloc(128, true);
    let desc = mem::heap_alloc(64, true);
    let wfx = mem::heap_alloc(64, true);
    let iid = mem::heap_alloc(16, true);
    let notes = mem::heap_alloc(16 * 8, true);
    let mut names = Names(Vec::new());
    let mut devs: Vec<u32> = Vec::new();
    let mut bufs: Vec<u32> = Vec::new();
    let mut kids: Vec<u32> = Vec::new();
    let mut images: Vec<u32> = Vec::new();
    for _ in 0..2 {
        let hr = call(DirectSoundCreate8, &[0, po, 0]);
        let d = r32(po);
        names.add(d);
        devs.push(d);
        writeln!(out, "create: {hr:#x} {}", names.of(d)).unwrap();
    }
    for i in 0..3000 {
        mem::fill(po, 128, 0);
        w32(po, 0xdead_beef);
        let dev = devs.first().copied().unwrap_or(0);
        let buf = if bufs.is_empty() || r.below(25) == 0 { 0x0bad_0000 } else { bufs[usize::try_from(r.below(u64::try_from(bufs.len()).unwrap_or(1))).unwrap_or(0)] };
        let line = match r.below(24) {
            0 if dev != 0 => {
                let k = r.pick(&[0u32, 0x279a_fa83, 0xc50a_7e93, 0x279a_fa85, 0x1234]);
                mem::fill(iid, 16, 0);
                w32(iid, k);
                let hr = call(DS_QueryInterface, &[dev, iid, po]);
                format!("ds qi {k:#x}: {hr:#x} {}", names.of(r32(po)))
            }
            1 if dev != 0 => {
                let (a, b) = (call(DS_AddRef, &[dev]), call(DS_Release, &[dev]));
                format!("ds addref/release: {a} {b}")
            }
            2 | 3 if dev != 0 && bufs.len() < 12 => {
                let flags = r.pick(&[0u32, 0, 1, 0x80, 0xe0, 0x8000, 0x10]);
                let bytes = r.pick(&[0u32, 4, 100, 4410, 8820, 3, 17640]);
                let with_fmt = r.below(5) != 0;
                if with_fmt {
                    seeded_wfx(&mut r, wfx);
                }
                mem::fill(desc, 64, 0);
                w32(desc, 36);
                w32(desc + 4, flags);
                w32(desc + 8, bytes);
                w32(desc + 16, if with_fmt { wfx } else { 0 });
                let null = r.below(15) == 0;
                let hr = call(DS_CreateSoundBuffer, &[dev, if null { 0 } else { desc }, po, 0]);
                let o = r32(po);
                let mut s = format!("create buffer flags={flags:#x} bytes={bytes} fmt={with_fmt} null={null}: {hr:#x}");
                if hr == 0 {
                    names.add(o);
                    bufs.push(o);
                    if let Some((data, size, f)) = with_buf(o, |b| (b.data, b.size, b.fmt)) {
                        let raw = r.bytes(usize::try_from(size).unwrap_or(0));
                        if data != 0 {
                            mem::write(data, &raw);
                        }
                        write!(s, " {} size={size} {} data={}", names.of(o), fmt_str(&f), data != 0).unwrap();
                    }
                }
                s
            }
            4 if dev != 0 => {
                let n = r.pick(&[0u32, 4, 20, 96, 200]);
                mem::fill(po, 128, 0xcc);
                w32(po, n);
                let hr = call(DS_GetCaps, &[dev, po]);
                format!("ds caps {n}: {hr:#x} {:x?}", (0..25).map(|k| r32(po + 4 * k)).collect::<Vec<_>>())
            }
            5 if dev != 0 && bufs.len() < 12 => {
                let hr = call(DS_DuplicateSoundBuffer, &[dev, buf, po]);
                let o = r32(po);
                if hr == 0 {
                    names.add(o);
                    bufs.push(o);
                }
                format!("duplicate {}: {hr:#x} {}", names.of(buf), if hr == 0 { names.of(o) } else { format!("{o:#x}") })
            }
            6 if dev != 0 => {
                let a = call(DS_SetCooperativeLevel, &[dev, 0x10, 2]);
                let b = call(DS_Compact, &[dev]);
                let c = call(DS_GetSpeakerConfig, &[dev, po]);
                let sc = r32(po);
                let d = call(DS_SetSpeakerConfig, &[dev, 5]);
                let e = call(DS_Initialize, &[dev, 0]);
                let f = call(DS_VerifyCertification, &[dev, po]);
                format!("ds misc: {a:#x} {b:#x} {c:#x} speaker={sc} {d:#x} {e:#x} {f:#x} cert={}", r32(po))
            }
            7 => {
                let k = r.pick(&[0u32, 0x279a_fa85, 0x6825_a449, 0xb021_0783, 0x279a_fa84, 0x279a_fa86, 0x5555]);
                mem::fill(iid, 16, 0);
                w32(iid, k);
                let hr = call(B_QueryInterface, &[buf, iid, po]);
                let o = r32(po);
                let shown = if o == buf { "self".to_string() } else if hr == 0 && o != 0 {
                    names.add(o);
                    kids.push(o);
                    names.of(o)
                } else {
                    format!("{o:#x}")
                };
                format!("qi {} {k:#x}: {hr:#x} {shown} refs={:?}", names.of(buf), with_buf(buf, |b| b.refs))
            }
            8 => format!("addref {}: {}", names.of(buf), call(B_AddRef, &[buf])),
            9 if r.below(3) == 0 => {
                let n = call(B_Release, &[buf]);
                let gone = !with(|d| d.bufs.contains_key(&buf));
                let shown = names.of(buf);
                if gone && buf != 0x0bad_0000 {
                    bufs.retain(|&x| x != buf);
                    names.drop_(buf);
                }
                format!("release {shown}: {n} gone={gone}")
            }
            10 => {
                let p = mem::heap_alloc(20, true);
                w32(p, 20);
                let hr = call(B_GetCaps, &[buf, p]);
                let s = format!("caps {}: {hr:#x} {:x?}", names.of(buf), (0..5).map(|k| r32(p + 4 * k)).collect::<Vec<_>>());
                mem::heap_free(p);
                s
            }
            11 => {
                let (pp, wp) = (if r.below(8) == 0 { 0 } else { po }, if r.below(8) == 0 { 0 } else { po + 4 });
                let hr = call(B_GetCurrentPosition, &[buf, pp, wp]);
                format!("position {} {} {}: {hr:#x} {} {}", names.of(buf), pp != 0, wp != 0, r32(po), r32(po + 4))
            }
            12 => {
                let size = r.pick(&[0u32, 15, 16, 18, 40]);
                let to = if r.below(6) == 0 { 0 } else { po + 8 };
                mem::fill(po, 64, 0xcc);
                let hr = call(B_GetFormat, &[buf, to, size, po]);
                format!("format {} size={size} to={}: {hr:#x} written={} {:x?}", names.of(buf), to != 0, r32(po), mem::slice(po + 8, 18))
            }
            13 => {
                let a = call(B_GetVolume, &[buf, po]);
                let v = r32(po);
                let b = call(B_GetPan, &[buf, po]);
                let p = r32(po);
                let c = call(B_GetFrequency, &[buf, po]);
                let f = r32(po);
                let d = call(B_GetStatus, &[buf, po]);
                format!("get {}: {a:#x} vol={} {b:#x} pan={} {c:#x} freq={f} {d:#x} status={:#x}", names.of(buf), v.cast_signed(), p.cast_signed(), r32(po))
            }
            14 => {
                let off = r.pick(&[0u32, 1, 100, 4409, 4410, 9000, u32::MAX]);
                let bytes = r.pick(&[0u32, 1, 100, 4410, 9000, u32::MAX]);
                let flags = r.pick(&[0u32, 0, 1, 2, 3]);
                let hr = call(B_Lock, &[buf, off, bytes, po, po + 4, po + 8, po + 12, flags]);
                let data = with_buf(buf, |b| b.data).unwrap_or(0);
                let (a1, l1, a2, l2) = (r32(po), r32(po + 4), r32(po + 8), r32(po + 12));
                let a1s = if hr == 0 { format!("+{}", a1.wrapping_sub(data)) } else { format!("{a1:#x}") };
                format!("lock {} off={off} bytes={bytes} flags={flags}: {hr:#x} {a1s} {l1} {} {l2}", names.of(buf), if a2 == 0 { "0" } else if a2 == data { "data" } else { "?" })
            }
            15 => {
                let flags = r.pick(&[0u32, 1, 1, 2]);
                let hr = call(B_Play, &[buf, 0, 0, flags]);
                format!("play {} {flags}: {hr:#x}", names.of(buf))
            }
            16 => {
                let p = r.pick(&[0u32, 2, 100, 4408, 4410, 17640, u32::MAX]);
                let hr = call(B_SetCurrentPosition, &[buf, p]);
                format!("set position {} {p}: {hr:#x} pos={:?}", names.of(buf), with_buf(buf, |b| b.pos))
            }
            17 => {
                let to = if r.below(6) == 0 { 0 } else {
                    seeded_wfx(&mut r, wfx);
                    wfx
                };
                let hr = call(B_SetFormat, &[buf, to]);
                format!("set format {}: {hr:#x} {:?}", names.of(buf), with_buf(buf, |b| (fmt_str(&b.fmt), b.freq)))
            }
            18 => {
                let v = r.pick(&[0i32, -1, -600, -5000, -10000, -10001, 100]);
                let p = r.pick(&[0i32, -10000, -3000, 3000, 10000, 20000]);
                let f = r.pick(&[0u32, 1, 99, 100, 22050, 44100, 96000, 200_000, 200_001]);
                let (a, b, c) = (call(B_SetVolume, &[buf, v.cast_unsigned()]), call(B_SetPan, &[buf, p.cast_unsigned()]), call(B_SetFrequency, &[buf, f]));
                format!("set {} vol={v} pan={p} freq={f}: {a:#x} {b:#x} {c:#x} {:?}", names.of(buf), with_buf(buf, |b| (b.gain.0.to_bits(), b.gain.1.to_bits(), b.freq)))
            }
            19 => {
                let hr = call(B_Stop, &[buf]);
                format!("stop {}: {hr:#x}", names.of(buf))
            }
            20 => {
                let v = [call(B_Unlock, &[buf, 0, 0, 0, 0]), call(B_Restore, &[buf]), call(B_SetFX, &[buf, 0, 0, 0]), call(B_AcquireResources, &[buf, 0, 0, 0]), call(B_GetObjectInPath, &[buf, 0, 0, 0, po]), call(B_Initialize, &[buf, dev, desc])];
                format!("misc {}: {v:x?}", names.of(buf))
            }
            21 if !kids.is_empty() => {
                let k = kids[usize::try_from(r.below(u64::try_from(kids.len()).unwrap_or(1))).unwrap_or(0)];
                match r.below(4) {
                    0 => {
                        let n = r.pick(&[0u32, 1, 3, 8, 0x20_0000]);
                        for j in 0..8u32 {
                            let off = r.pick(&[0u32, 1, 100, 2000, 4409, 8000, u32::MAX]);
                            if crate::mem::x64() {
                                w32(notes + 16 * j, off);
                                w32(notes + 16 * j + 8, EV0 + j);
                            } else {
                                w32(notes + 8 * j, off);
                                w32(notes + 8 * j + 4, EV0 + j);
                            }
                        }
                        let p = if r.below(8) == 0 { 0 } else { notes };
                        let hr = call(N_SetNotificationPositions, &[k, n, p]);
                        let b = r32(k + 4);
                        format!("notify {} n={n} p={}: {hr:#x} {:?}", names.of(k), p != 0, with_buf(b, |x| x.notify.iter().map(|&(o, e)| (o, e.wrapping_sub(EV0))).collect::<Vec<_>>()))
                    }
                    1 => {
                        mem::fill(iid, 16, 0);
                        w32(iid, r.pick(&[0u32, 0xb021_0783, 0x77]));
                        let hr = call(N_QueryInterface, &[k, iid, po]);
                        format!("kid qi {}: {hr:#x} self={} refs={}", names.of(k), r32(po) == k, r32(k + 8))
                    }
                    2 => format!("kid addref {}: {}", names.of(k), call(N_AddRef, &[k])),
                    _ => {
                        let b = r32(k + 4);
                        let n = call(N_Release, &[k]);
                        let shown = names.of(k);
                        if n == 0 {
                            kids.retain(|&x| x != k);
                            names.drop_(k);
                            if !with(|d| d.bufs.contains_key(&b)) {
                                bufs.retain(|&x| x != b);
                                names.drop_(b);
                            }
                        }
                        format!("kid release {shown}: {n} buffer refs={:?}", with_buf(b, |x| x.refs))
                    }
                }
            }
            22 => {
                // PlaySound of an image in memory
                let flags = r.pick(&[SND_MEMORY | SND_ASYNC, SND_MEMORY | SND_ASYNC | SND_LOOP, SND_MEMORY | SND_ASYNC | SND_NOSTOP, SND_PURGE, SND_MEMORY | SND_LOOP]);
                let img = seeded_riff(&mut r);
                let p = mem::heap_alloc(u32::try_from(img.len()).unwrap_or(0) + 4, true);
                mem::write(p, &img);
                images.push(p);
                let name = if r.below(10) == 0 { 0 } else { p };
                let rc = play_sound(name, 0, flags, false);
                let st = with(|d| d.bufs.get(&PLAY_SOUND).map(|b| (fmt_str(&b.fmt), b.size, b.playing, b.looping)));
                format!("playsound {} bytes flags={flags:#x} name={}: {rc} {st:?}", img.len(), name != 0)
            }
            _ => {
                // the mixer, by hand: a buffer alone, or the PlaySound one
                let key = if r.below(5) == 0 { PLAY_SOUND } else { buf };
                let steps = r.below(3) + 1;
                format!("mix {}:{}", if key == PLAY_SOUND { "playsound".into() } else { names.of(key) }, mix_alone(key, steps))
            }
        };
        writeln!(out, "{i} {line}").unwrap();
    }
    play_sound(0, 0, 0, false);
    for k in kids {
        while call(N_Release, &[k]) > 0 {}
    }
    for b in bufs {
        while with(|d| d.bufs.contains_key(&b)) {
            call(B_Release, &[b]);
        }
    }
    for d in devs {
        while call(DS_Release, &[d]) > 0 {}
    }
    for p in images.into_iter().chain([po, desc, wfx, iid, notes]) {
        mem::heap_free(p);
    }
    snapshot("sound_dsound", &out);
}

/// waveOut device `hwo`'s mixing, alone: `steps` passes, each one's output, then where it is
fn mix_wave_alone(hwo: u32, steps: u64, hdrs: &Names) -> String {
    let Some(w) = with(|d| d.waves.remove(&hwo)) else { return "gone".into() };
    let mut tmp = Dev { bufs: HashMap::default(), waves: HashMap::default(), stream: -1 };
    tmp.waves.insert(hwo, w);
    let mut s = String::new();
    for _ in 0..steps {
        let mut o = vec![0f32; STEP * 2];
        mix(&mut tmp, &mut o);
        write!(s, " [{:#x}]", hash_f(&o)).unwrap();
    }
    if let Some(w) = tmp.waves.remove(&hwo) {
        write!(s, " pos={} played={} queue={:?} paused={}", w.pos, w.played, w.queue.iter().map(|&h| hdrs.of(h)).collect::<Vec<_>>(), w.paused).unwrap();
        with(|d| d.waves.insert(hwo, w));
    }
    s
}

#[test]
fn sound_waveout_snapshot() {
    let _g = enter();
    guest_thread();
    let _held = HeldMixer::new();
    crate::mixer::SESSION.set(0, 0xffff).ok();
    crate::mixer::SESSION.set(1, 0).ok();
    let mut out = String::new();
    let mut r = Rng(0x0a_7e00);
    let po = mem::heap_alloc(1024, true);
    let wfx = mem::heap_alloc(64, true);
    let mut handles = Names(Vec::new());
    let mut hdrs = Names(Vec::new());
    let mut open: Vec<u32> = Vec::new();
    let mut headers: Vec<(u32, u32)> = Vec::new();
    let k = ps();
    for i in 0..2500 {
        let hwo = if open.is_empty() || r.below(25) == 0 { r.pick(&[0u32, WAVE_MAPPER, 0x77]) } else { open[usize::try_from(r.below(u64::try_from(open.len()).unwrap_or(1))).unwrap_or(0)] };
        let hname = |h: u32, n: &Names| if h == 0 || h == WAVE_MAPPER || h == 0x77 { format!("{h:#x}") } else { n.of(h) };
        let line = match r.below(18) {
            0 | 1 if open.len() < 4 => {
                if r.below(3) == 0 {
                    // a well-formed PCM format, so that most opens succeed
                    mem::fill(wfx, 64, 0);
                    let (ch, rate, bits) = (r.pick(&[1u16, 2]), r.pick(&[22050u32, 44100, 11025]), r.pick(&[8u16, 16]));
                    w16(wfx, 1);
                    w16(wfx + 2, ch);
                    w32(wfx + 4, rate);
                    w32(wfx + 8, rate * u32::from(ch * bits / 8));
                    w16(wfx + 12, ch * bits / 8);
                    w16(wfx + 14, bits);
                } else {
                    seeded_wfx(&mut r, wfx);
                }
                let dev = r.pick(&[0u32, WAVE_MAPPER, WAVE_MAPPER, 3]);
                let flags = r.pick(&[0u32, 0, 1, 8, 9]);
                let fp = if r.below(15) == 0 { 0 } else { wfx };
                let ph = if r.below(15) == 0 { 0 } else { po };
                w32(po, 0xdead_beef);
                let rc = call(waveOutOpen, &[ph, dev, fp, 0, 0, flags]);
                let h = r32(po);
                let shown = if rc == 0 && flags & 1 == 0 && ph != 0 {
                    handles.add(h);
                    open.push(h);
                    handles.of(h)
                } else {
                    format!("{h:#x}")
                };
                format!("open dev={dev:#x} flags={flags} fmt={:x?} p={}: {rc} {shown}", mem::slice(wfx, 20), ph != 0)
            }
            2 if open.len() > 0 && headers.len() < 16 => {
                let n = r.pick(&[0u32, 1, 4, 100, 882, 4410]);
                let h = mem::heap_alloc(2 * k + 24, true);
                let data = mem::heap_alloc(n + 4, true);
                let raw = r.bytes(usize::try_from(n).unwrap_or(0));
                mem::write(data, &raw);
                mem::wp(h, data);
                w32(h + k, n);
                w32(hflags_at(h), r.pick(&[0u32, 0, 2, 1, 0x10, 0x12]));
                hdrs.add(h);
                headers.push((h, data));
                format!("header {} {n} bytes flags={:#x}", hdrs.of(h), hflags(h))
            }
            3 | 15 | 16 if !headers.is_empty() => {
                let (h, _) = headers[usize::try_from(r.below(u64::try_from(headers.len()).unwrap_or(1))).unwrap_or(0)];
                let which = r.below(3);
                let hp = if r.below(12) == 0 { 0 } else { h };
                let rc = match which {
                    0 => call(waveOutPrepareHeader, &[hwo, hp, 32]),
                    1 => call(waveOutUnprepareHeader, &[hwo, hp, 32]),
                    _ => call(waveOutWrite, &[hwo, hp, 32]),
                };
                format!("{} {} {}: {rc} flags={:#x}", ["prepare", "unprepare", "write"][usize::try_from(which).unwrap_or(0)], hname(hwo, &handles), if hp == 0 { "0".into() } else { hdrs.of(h) }, hflags(h))
            }
            4 => {
                let rc = call(waveOutReset, &[hwo]);
                format!("reset {}: {rc} flags {:x?}", hname(hwo, &handles), headers.iter().map(|&(h, _)| hflags(h)).collect::<Vec<_>>())
            }
            5 => format!("pause {}: {}", hname(hwo, &handles), call(waveOutPause, &[hwo])),
            6 => format!("restart {}: {}", hname(hwo, &handles), call(waveOutRestart, &[hwo])),
            7 => {
                let ty = r.pick(&[1u32, 2, 4, 8, 0x20]);
                let size = r.pick(&[12u32, 12, 8, 16]);
                mem::fill(po, 16, 0);
                w32(po, ty);
                let t = if r.below(10) == 0 { 0 } else { po };
                let rc = call(waveOutGetPosition, &[hwo, t, size]);
                format!("position {} type={ty} size={size}: {rc} {:x?}", hname(hwo, &handles), (0..3).map(|j| r32(po + 4 * j)).collect::<Vec<_>>())
            }
            8 => {
                let v = r.pick(&[0u32, 0xffff_ffff, 0x8000_4000, 0x0000_ffff]);
                // (a device id sets every open device, another test's too: only handles here)
                let target = if open.contains(&hwo) { hwo } else { open.first().copied().unwrap_or(0) };
                let a = if target != 0 { call(waveOutSetVolume, &[target, v]) } else { 0 };
                let p = if r.below(8) == 0 { 0 } else { po };
                let b = call(waveOutGetVolume, &[target, p]);
                format!("volume {} {v:#x}: {a} {b} {:#x}", hname(target, &handles), r32(po))
            }
            9 => {
                let wide = r.below(2) == 0;
                let size = r.pick(&[0u32, 4, 20, 52, 84, 200]);
                mem::fill(po, 256, 0xcc);
                let p = if r.below(10) == 0 { 0 } else { po };
                let rc = call(if wide { waveOutGetDevCapsW } else { waveOutGetDevCapsA }, &[hwo, p, size]);
                format!("caps {} wide={wide} size={size}: {rc} {:#x}", hname(hwo, &handles), hash(po, 100))
            }
            10 => {
                let p = if r.below(8) == 0 { 0 } else { po };
                w32(po, 0xdead_beef);
                let rc = call(waveOutGetID, &[hwo, p]);
                format!("id {}: {rc} {:#x}", hname(hwo, &handles), r32(po))
            }
            11 => {
                let msg = r.pick(&[0x2015u32, 0x2017, 0x805, 0x80c, 0x80d, 0x80d, 0x1234]);
                let p1 = if r.below(8) == 0 { 0 } else { po };
                let p2 = if msg == 0x80d { r.pick(&[0u32, 10, 200, 1000]) } else if r.below(8) == 0 { 0 } else { po + 512 };
                mem::fill(po, 1024, 0);
                let rc = call(waveOutMessage, &[hwo, msg, p1, p2]);
                let p2s = if msg == 0x80d { format!("{p2}") } else { format!("{}", p2 != 0) };
                format!("message {msg:#x} p1={} p2={p2s}: {rc} {:#x}", p1 != 0, hash(po, 300))
            }
            12 => {
                let e = r.pick(&[0u32, 1, 2, 5, 8, 11, 32, 33, 34, 3, 99]);
                let wide = r.below(2) == 0;
                let n = r.pick(&[0u32, 1, 10, 256]);
                mem::fill(po, 600, 0xcc);
                let p = if r.below(10) == 0 { 0 } else { po };
                let rc = call(if wide { |c: &mut Ctx| wave_error_text(c, true) } else { |c: &mut Ctx| wave_error_text(c, false) }, &[e, p, n]);
                format!("error text {e} wide={wide} n={n}: {rc} {:#x}", hash(po, 600))
            }
            13 => format!("num devs: {}", call(waveOutGetNumDevs, &[])),
            14 if r.below(4) == 0 && !open.is_empty() => {
                let rc = call(waveOutClose, &[hwo]);
                let shown = hname(hwo, &handles);
                if rc == 0 {
                    // (the device thread drops it whenever it looks: not used again)
                    open.retain(|&x| x != hwo);
                    handles.drop_(hwo);
                }
                format!("close {shown}: {rc}")
            }
            _ if open.contains(&hwo) => format!("mix {}:{}", handles.of(hwo), mix_wave_alone(hwo, r.below(4) + 1, &hdrs)),
            _ => continue,
        };
        writeln!(out, "{i} {line}").unwrap();
    }
    for h in open {
        call(waveOutReset, &[h]);
        call(waveOutClose, &[h]);
    }
    wave_wake();
    // (the headers stay: the device thread may still be looking at a closed device's done list)
    mem::heap_free(po);
    mem::heap_free(wfx);
    snapshot("sound_waveout", &out);
}
