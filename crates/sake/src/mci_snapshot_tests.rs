//! MCI's string interface pinned before its code changes shape: scripted sessions on two wave
//! files (open by extension / type / alias, every status item, time formats, seek, play ...
//! wait, setaudio, capability, info, where / put / window, close) and a seeded sweep of
//! command strings made of MCI's own words, each answer (error, returned text) a line of
//! golden/mci.txt. Device ids show as the order they were opened in (the counter is global).
use super::*;
use crate::test_guest::{call, enter, guest_thread, snapshot, Rng};
use std::fmt::Write;

/// A RIFF WAVE of `frames` frames of a ramp
fn wav(ch: u16, rate: u32, bits: u16, frames: u32) -> Vec<u8> {
    let align = ch * bits / 8;
    let data: Vec<u8> = (0..frames * u32::from(align)).map(|i| (i * 7) as u8).collect();
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&ch.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
    b.extend_from_slice(&align.to_le_bytes());
    b.extend_from_slice(&bits.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data.len() as u32).to_le_bytes());
    b.extend_from_slice(&data);
    b
}

struct Session {
    /// the test's folder, shown as DIR
    z: String,
    ret: u32,
    cmd: u32,
    ids: Vec<String>,
    out: String,
}
impl Session {
    fn send(&mut self, s: &str) {
        mem::write(self.cmd, s.as_bytes());
        mem::w8(self.cmd + s.len() as u32, 0);
        mem::fill(self.ret, 256, 0);
        let r = call(mciSendStringA, &[self.cmd, self.ret, 256, 0]);
        let mut text = crate::text::a2s(&mem::cstr(self.ret));
        // (an open's id: the order it was opened in)
        if r == 0 && s.to_ascii_lowercase().starts_with("open") {
            self.ids.push(text.clone());
            text = format!("id#{}", self.ids.len());
        }
        let line = format!("{s:?}: {r} {text:?}").replace(&self.z.replace('\\', "\\\\"), "DIR").replace(&self.z, "DIR");
        writeln!(self.out, "{line}").unwrap();
    }
}

#[test]
fn mci_string_snapshot() {
    let _g = enter();
    guest_thread();
    let dir = std::env::temp_dir().join(format!("masu-mci-snapshot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.wav"), wav(1, 8000, 8, 8000)).unwrap();
    std::fs::write(dir.join("b.WAV"), wav(2, 44100, 16, 4410)).unwrap();
    std::fs::write(dir.join("junk.wav"), b"RIFF0000WAVEjunk").unwrap();
    std::fs::write(dir.join("noext"), wav(1, 22050, 16, 2205)).unwrap();
    let z = format!("Z:{}", dir.to_str().unwrap().replace('/', "\\"));
    let mut s = Session { z: z.clone(), ret: mem::heap_alloc(256, true), cmd: mem::heap_alloc(1024, true), ids: Vec::new(), out: String::new() };
    let script = [
        format!("open {z}\\a.wav alias a"),
        format!("open {z}\\a.wav alias a"),
        format!("open \"{z}\\b.wav\" type waveaudio alias bee wait"),
        format!("open {z}\\noext type mpegvideo alias n"),
        format!("open {z}\\noext alias n2"),
        format!("open {z}\\junk.wav alias j"),
        format!("open {z}\\missing.wav alias m"),
        format!("open {z}\\a.mid alias mid"),
        format!("open {z}\\a.xyz alias x"),
        "open waveaudio".into(),
        "open cdaudio".into(),
        "open nonsense type".into(),
        "open \"unclosed".into(),
        "".into(),
        "status".into(),
    ];
    for c in &script {
        s.send(c);
    }
    for dev in ["a", "bee", "n", "nothere"] {
        for item in ["length", "position", "mode", "ready", "media present", "time format", "number of tracks", "current track", "volume", "bogus", ""] {
            s.send(&format!("status {dev} {item}"));
        }
        for fmt in ["milliseconds", "ms", "bytes", "samples", "frames", "smpte", ""] {
            s.send(&format!("set {dev} time format {fmt}"));
            s.send(&format!("status {dev} length"));
            s.send(&format!("status {dev} time format"));
        }
        s.send(&format!("set {dev} time format ms"));
        for to in ["0", "250", "999", "1000", "100000", "-5", "x", "start", "end"] {
            s.send(&format!("seek {dev} to {to}"));
            s.send(&format!("status {dev} position"));
        }
        s.send(&format!("seek {dev} to start"));
        s.send(&format!("seek {dev} to end"));
        s.send(&format!("status {dev} position"));
        for (from, to) in [("0", "30"), ("40", "60"), ("90", "10"), ("", "20"), ("70", "")] {
            let mut c = format!("play {dev}");
            if !from.is_empty() {
                c += &format!(" from {from}");
            }
            if !to.is_empty() {
                c += &format!(" to {to}");
            }
            if to.is_empty() {
                // (no end of its own: stopped right away, the position not read)
                s.send(&c);
                s.send(&format!("status {dev} mode"));
                s.send(&format!("stop {dev}"));
            } else {
                s.send(&format!("{c} wait"));
                s.send(&format!("status {dev} position"));
                s.send(&format!("status {dev} mode"));
            }
        }
        for v in ["0", "500", "1000", "1001", "x"] {
            s.send(&format!("setaudio {dev} volume to {v}"));
            s.send(&format!("status {dev} volume"));
        }
        for cap in ["can play", "can record", "can save", "can eject", "uses files", "compound device", "has audio", "has video", "device type", "can freeze", "bogus"] {
            s.send(&format!("capability {dev} {cap}"));
        }
        for info in ["product", "file", "version", ""] {
            s.send(&format!("info {dev} {info}"));
        }
        for c in ["where {d} destination", "put {d} destination at 1 2 3 4", "where {d} destination", "window {d} handle 1234", "pause {d}", "resume {d}", "cue {d}", "record {d}", "stop {d}", "status {d} mode"] {
            s.send(&c.replace("{d}", dev));
        }
    }
    s.send("close a");
    s.send("close a");
    s.send("status a mode");
    // a seeded sweep of commands made of MCI's words
    let words = ["open", "close", "play", "seek", "stop", "pause", "resume", "status", "set", "setaudio", "capability", "info", "where", "put", "window", "cue", "bee", "n", "all", "to", "from", "start", "end", "wait", "notify", "time", "format", "ms", "bytes", "samples", "frames", "length", "position", "mode", "ready", "volume", "destination", "at", "handle", "0", "1", "50", "100", "x", "\"", "type", "alias", "waveaudio", "repeat", "default"];
    let mut r = Rng(0x00c1_5eed);
    for _ in 0..1500 {
        let n = 1 + r.below(6) as usize;
        let mut c: Vec<&str> = (0..n).map(|_| r.pick(&words)).collect();
        // (no open of a new device, no play without an end: a sweep's lines stay the same)
        if c[0] == "open" || c.contains(&"play") && !c.contains(&"wait") || c.contains(&"resume") {
            c[0] = "status";
        }
        s.send(&c.join(" "));
        if r.below(10) == 0 {
            s.send("stop bee");
            s.send("stop n");
        }
    }
    s.send("close all");
    s.send("status bee mode");
    for p in [s.ret, s.cmd] {
        mem::heap_free(p);
    }
    let _ = std::fs::remove_dir_all(&dir);
    snapshot("mci", &s.out);
}

/// mciSendCommand with parameter blocks: opens through MCI_OPEN_PARMS (type by name / by id,
/// element, alias, the DGV parent), then a seeded sweep of commands with random flags and
/// blocks on the devices, a bad id and MCI_ALL_DEVICE_ID; each answer and the block after it
/// a line of golden/mci_command.txt.
#[test]
fn mci_command_snapshot() {
    let _g = enter();
    guest_thread();
    let dir = std::env::temp_dir().join(format!("masu-mci-command-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.wav"), wav(1, 11025, 16, 11025)).unwrap();
    let path = format!("Z:{}\\a.wav", dir.to_str().unwrap().replace('/', "\\"));
    let (p, sp, tp, ap, buf) = (mem::heap_alloc(64, true), mem::heap_alloc(512, true), mem::heap_alloc(64, true), mem::heap_alloc(64, true), mem::heap_alloc(256, true));
    mem::write(sp, path.as_bytes());
    mem::write(tp, b"waveaudio\0");
    let mut out = String::new();
    let mut ids = Vec::new();
    // MCI_OPEN_PARMS { dwCallback, wDeviceID, lpstrDeviceType, lpstrElementName, lpstrAlias } + DGV { dwStyle, hWndParent }
    for (k, (flags, ty)) in [(0x200u32, 0u32), (0x200 | 0x400, 0), (0x200 | 0x2000 | 0x400, tp), (0x200 | 0x1000 | 0x400, 522), (0x200 | 0x1000 | 0x400, 523), (0x200 | 0x1000 | 0x400, 9), (0x2000, tp), (0, 0), (0x200 | 0x400 | 0x2_0000 | 0x1_0000, 0)].into_iter().enumerate() {
        mem::fill(p, 64, 0);
        mem::write(ap, format!("dev{k}\0").as_bytes());
        w32(p + 8, ty);
        w32(p + 12, sp);
        w32(p + 16, ap);
        w32(p + 20, 0x4000_0000);
        w32(p + 24, 0x1234);
        let r = call(mciSendCommandA, &[0, MCI_OPEN, flags, p]);
        if r == 0 {
            ids.push(r32(p + 4));
        }
        // (a type string is a heap address: shown as "tp")
        let ty_shown = if ty == tp { "tp".to_string() } else { format!("{ty:#x}") };
        writeln!(out, "open {flags:#x} {ty_shown}: {r}{}", if r == 0 { format!(" id#{}", ids.len()) } else { String::new() }).unwrap();
    }
    let msgs = [MCI_CLOSE, MCI_PLAY, MCI_SEEK, MCI_STOP, MCI_PAUSE, MCI_INFO, MCI_GETDEVCAPS, MCI_SET, MCI_STATUS, MCI_CUE, MCI_WINDOW, MCI_PUT, MCI_WHERE, MCI_RESUME, MCI_SETAUDIO, 0x805, 0x80f, 0x1234];
    let flag_bits = [MCI_WAIT, MCI_FROM, MCI_TO, 0x100, 0x200, 0x400, 0x800, 0x1_0000, 0x2_0000, 0x4_0000, 0x80_0000, 0x100_0000];
    let mut r = Rng(0x00c1_c0de);
    for case in 0..1500 {
        let msg = r.pick(&msgs);
        let mut flags = 0;
        for _ in 0..r.below(4) {
            flags |= r.pick(&flag_bits);
        }
        let which = r.below(ids.len() as u64 + 2) as usize;
        let id = ids.get(which).copied().unwrap_or(if which == ids.len() { 0x7777 } else { MCI_ALL_DEVICE_ID });
        // (a play has its end, waited for; a close only near the end: the sweep's lines stay put)
        if msg == MCI_PLAY {
            flags |= MCI_WAIT | MCI_TO;
            flags &= !0x1_0000;
        }
        if msg == MCI_CLOSE && case < 1450 {
            continue;
        }
        let blk = if r.below(8) == 0 { 0 } else { p };
        mem::fill(p, 64, 0);
        let fields = [r.pick(&[0u32, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 0x4002, 0x4019]), r.pick(&[0u32, 3, 8, 9, 50, 500, 999, 1000, 5000]), r.pick(&[0u32, 3, 8, 9, 50, 500, 999, 1000, 5000, 0x4002]), r.pick(&[0u32, 1, 100])];
        for (k, v) in (0u32..).zip(fields) {
            w32(p + 4 + 4 * k, v);
        }
        // (a play's span short: it is waited for in real time)
        if msg == MCI_PLAY {
            let from = r.pick(&[0u32, 100, 500]);
            w32(p + 4, from);
            w32(p + 8, from + r.pick(&[0u32, 5, 10]));
        }
        if msg == MCI_INFO {
            w32(p + 4, buf);
            w32(p + 8, r.pick(&[0u32, 1, 8, 256]));
            mem::fill(buf, 256, 0xcc);
        }
        let rc = call(mciSendCommandA, &[id, msg, flags, blk]);
        let info = if msg == MCI_INFO {
            // (the buffer only: an unterminated one shows its 0xcc)
            let b: Vec<u8> = mem::array::<256>(buf).iter().copied().take_while(|&x| x != 0).collect();
            crate::text::a2s(&b).replace(&path, "PATH")
        } else {
            String::new()
        };
        // (an info's buffer is a heap address: shown as "buf")
        let first = if r32(p + 4) == buf { "buf".to_string() } else { format!("{:x}", r32(p + 4)) };
        writeln!(out, "{case} dev#{which} {msg:#x} {flags:#x} blk{}: {rc} [{first} {:x} {:x} {:x} {:x}] {info:?}", blk != 0, r32(p + 8), r32(p + 12), r32(p + 16), r32(p + 20)).unwrap();
    }
    let rc = call(mciSendCommandA, &[MCI_ALL_DEVICE_ID, MCI_CLOSE, 0, 0]);
    writeln!(out, "close all: {rc}").unwrap();
    for q in [p, sp, tp, ap, buf] {
        mem::heap_free(q);
    }
    let _ = std::fs::remove_dir_all(&dir);
    snapshot("mci_command", &out);
}
