//! kikizake's Direct3D 8 suite (github.com/komodaru/kikizake cases/d3d8.txt: defaults, every
//! state set and read back, state blocks of each type, recorded ones, random sequences) run
//! here through the Direct3D 8 entry points the way its runner (runner/kikizake.c op_device8,
//! op_state8, op_sb8) runs them on Windows, every line of output checked against Windows'
//! answer (golden/d3d8-32). The suite is GPU-only in tests/winref (it needs the page there);
//! here it needs nothing but the kikizake checkout at tests/winref/kikizake.rev.

use super::draw_tests::{call, out, rd};
use super::trace_tests::{alloc, attach_page, CALLS, SERIAL};
use super::*;

const V8: u32 = 16;
const BLOCKS: usize = 8;

/// The kikizake checkout (tests/winref/kikizake.sh: $KIKIZAKE or target/kikizake, fetched at
/// the pinned commit if it is not there).
pub(crate) fn kikizake() -> std::path::PathBuf {
    let sh = format!("{}/../../tests/winref/kikizake.sh", env!("CARGO_MANIFEST_DIR"));
    let o = std::process::Command::new("bash").arg(&sh).output().expect("bash");
    assert!(o.status.success(), "{sh}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().into()
}

struct Run {
    d3d: u32,
    dev: u32,
    blocks: [u32; BLOCKS],
    out: Vec<String>,
}

impl Run {
    fn reset(&mut self) {
        if self.dev != 0 {
            for b in self.blocks.iter_mut().filter(|b| **b != 0) {
                call(IF_DEVICE + V8, 56, &[self.dev, *b]);
                *b = 0;
            }
            call(IF_DEVICE + V8, 2, &[self.dev]);
        }
        if self.d3d != 0 {
            call(IF_D3D9 + V8, 2, &[self.d3d]);
        }
        (self.d3d, self.dev) = (0, 0);
    }

    fn device(&mut self, how: &str) {
        self.reset();
        attach_page();
        let o = out();
        assert_eq!(unsafe { vnhost_d3d9_call(super::d3d8::CREATE, [o].as_ptr(), &CALLS) }, 0);
        self.d3d = rd(o);
        // (the runner's: 64x64 X8R8G8B8, one back buffer, DISCARD, windowed, auto D24S8)
        let pp = unsafe { alloc(52) as u32 };
        for (i, x) in [64, 64, 22, 1, 0, 1, 1, 1, 1, 75, 0, 0, 0].into_iter().enumerate() {
            unsafe { *((pp + 4 * i as u32) as *mut u32) = x };
        }
        let flags = match how {
            "swvp" => 0x20,
            "mixed" => 0x80,
            _ => 0x40,
        };
        let o = out();
        let mut r = call(IF_D3D9 + V8, 15, &[self.d3d, 0, 1, 1, flags, pp, o]);
        if r != 0 {
            r = call(IF_D3D9 + V8, 15, &[self.d3d, 0, 1, 1, 0x20, pp, o]);
        }
        if r != 0 {
            self.out.push(format!("device8 failed {r:08x}"));
            return;
        }
        self.dev = rd(o);
        self.out.push("device8 ok".into());
    }

    fn state(&mut self, t: &[&str]) {
        let num = |s: &str| if let Some(h) = s.strip_prefix("0x") { u32::from_str_radix(h, 16).unwrap() } else { s.parse::<i64>().unwrap() as u32 };
        let two = t[0] == "tss";
        let set = t[1] == "set";
        let a = num(t[2]);
        let b = if two { num(t[3]) } else { 0 };
        let key = if two { format!("{} {a} {}", t[0], t[3]) } else { format!("{} {a}", t[0]) };
        let p = out();
        unsafe { *(p as *mut u32) = 0xdead_beef };
        let r = match (two, set) {
            (false, true) => call(IF_DEVICE + V8, 50, &[self.dev, a, num(t[3])]),
            (false, false) => call(IF_DEVICE + V8, 51, &[self.dev, a, p]),
            (true, true) => call(IF_DEVICE + V8, 63, &[self.dev, a, b, num(t[4])]),
            (true, false) => call(IF_DEVICE + V8, 62, &[self.dev, a, b, p]),
        };
        if set {
            if r != 0 {
                self.out.push(format!("{key} set hr {r:08x}"));
            }
        } else if r != 0 {
            self.out.push(format!("{key} hr {r:08x}"));
        } else {
            self.out.push(format!("{key} = {:08x}", rd(p)));
        }
    }

    fn sb(&mut self, t: &[&str]) {
        let r = if t[1] == "begin" {
            call(IF_DEVICE + V8, 52, &[self.dev])
        } else {
            let i: usize = t[2].parse().unwrap();
            match t[1] {
                "end" | "create" => {
                    if self.blocks[i] != 0 {
                        call(IF_DEVICE + V8, 56, &[self.dev, self.blocks[i]]);
                        self.blocks[i] = 0;
                    }
                    let o = out();
                    unsafe { *(o as *mut u32) = 0 };
                    let r = if t[1] == "end" {
                        call(IF_DEVICE + V8, 53, &[self.dev, o])
                    } else {
                        call(IF_DEVICE + V8, 57, &[self.dev, t[3].parse().unwrap(), o])
                    };
                    self.blocks[i] = if (r as i32) < 0 { 0 } else { rd(o) };
                    r
                }
                _ if self.blocks[i] == 0 => {
                    self.out.push(format!("sb {i} none"));
                    return;
                }
                "capture" => call(IF_DEVICE + V8, 55, &[self.dev, self.blocks[i]]),
                "apply" => call(IF_DEVICE + V8, 54, &[self.dev, self.blocks[i]]),
                "release" => {
                    let r = call(IF_DEVICE + V8, 56, &[self.dev, self.blocks[i]]);
                    self.blocks[i] = 0;
                    r
                }
                v => panic!("sb {v}"),
            }
        };
        if r != 0 {
            self.out.push(format!("sb {} hr {r:08x}", t[1]));
        }
    }
}

/// Runs a kikizake d3d8 cases text; its output as the runner prints it, per case.
fn run(cases: &str) -> Vec<(String, Vec<String>)> {
    let mut r = Run { d3d: 0, dev: 0, blocks: [0; BLOCKS], out: Vec::new() };
    let mut done = Vec::new();
    let mut name = None;
    for l in cases.lines() {
        let t: Vec<&str> = l.split_whitespace().collect();
        match t.first().copied() {
            None | Some("#") => {}
            Some("case") => name = Some(t[1].to_string()),
            Some("end") => {
                r.reset();
                done.push((name.take().unwrap(), std::mem::take(&mut r.out)));
            }
            Some("device8") => r.device(t[1]),
            Some("rs" | "tss") => r.state(&t),
            Some("sb") => r.sb(&t),
            Some(w) => panic!("{w}: not a d3d8 line"),
        }
    }
    done
}

fn golden_cases(text: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut m = std::collections::BTreeMap::new();
    let mut cur: Option<String> = None;
    // (the kikizake line first, "done" last)
    for l in text.lines().skip(1).filter(|l| *l != "done") {
        if let Some(n) = l.strip_prefix("case ") {
            cur = Some(n.to_string());
            m.insert(n.to_string(), Vec::new());
        } else if let Some(c) = &cur {
            m.get_mut(c).unwrap().push(l.to_string());
        }
    }
    m
}

/// Every case of the suite, every line as Windows printed it.
#[test]
fn d3d8_suite_is_windows() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    super::trace_tests::on_low_stack(|| {
        unsafe {
            let vt: Vec<u32> = (1..=64).map(|i| 0x1000 * i).collect();
            vnhost_d3d9_init(vt.as_ptr(), vt.len() as u32);
        }
        let k = kikizake();
        let cases = std::fs::read_to_string(k.join("cases/d3d8.txt")).unwrap();
        let golden = golden_cases(&std::fs::read_to_string(k.join("golden/d3d8-32/stdout.txt")).unwrap());
        let got = super::trace_tests::high_heap(|| run(&cases));
        assert!(got.len() > 150, "{} cases", got.len());
        let mut bad = Vec::new();
        for (name, lines) in &got {
            let want = golden.get(name).unwrap_or_else(|| panic!("no golden for case {name}"));
            if lines != want {
                let i = lines.iter().zip(want).position(|(a, b)| a != b).unwrap_or(lines.len().min(want.len()));
                bad.push(format!(
                    "{name} line {i}: want {:?} got {:?}",
                    want.get(i).map_or("(end)", |s| s.as_str()),
                    lines.get(i).map_or("(end)", |s| s.as_str())
                ));
            }
        }
        assert!(bad.is_empty(), "{} of {} cases differ from Windows:\n  {}", bad.len(), got.len(), bad.join("\n  "));
        gpu::set_pump(None);
    });
}
