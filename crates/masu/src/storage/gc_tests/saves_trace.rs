//! Random save writes (the revision expected current or stale), reads and collections through
//! the real Saves on the fake bucket, with failures injected at random: requests the bucket
//! refuses, forgets the objects table refuses (all, or of revisions ending in 1) and row writes
//! the saves table refuses. Each run is a trace for TLC to check against formal/tla/SaveApi.tla
//! (formal/trace.sh): every call's answer, the row's revision and the revisions the bucket names
//! after it. A failure with nothing injected is written "fault", which SaveApi never allows.
//!
//! `MASU_TRACE_DIR=dir` writes `dir/saves-<seed>/`; `MASU_TRACE_SEEDS=n` (default 20).

use super::*;
use std::fmt::Write as _;

/// (save names, one a prefix of the others' "saves/<name>/" ranges in two ways)
const NAMES: [&str; 3] = ["g", "g.index", "g/x"];

struct Step {
    op: &'static str,
    name: usize,
    e: i64,
    v: i64,
    ret: &'static str,
    rev: i64,
    got: i64,
    row: i64,
    kept: Vec<i64>,
}

fn row(s: &Storage, name: &str) -> i64 {
    let c = s.db.connect().unwrap();
    c.query_row("SELECT revision FROM saves WHERE name=?", [name], |r| r.get(0)).optional().unwrap().unwrap_or(0)
}

/// the revisions of `name` the objects table names
fn kept(s: &Storage, name: &str) -> Vec<i64> {
    let c = s.db.connect().unwrap();
    let mut q = c.prepare("SELECT logical FROM objects WHERE logical LIKE 'saves/%'").unwrap();
    let all: Vec<String> = q.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    let mut v: Vec<i64> = all.iter().filter_map(|l| l.strip_prefix("saves/")?.rsplit_once('/')).filter(|(n, _)| *n == name).map(|(_, r)| r.parse().unwrap()).collect();
    v.sort_unstable();
    v
}

/// The injections a run turns on and off: the objects table refusing forgets, the saves table
/// refusing row writes.
const TRIGGERS: [(&str, &str); 3] = [
    ("no_forget", "CREATE TRIGGER no_forget BEFORE DELETE ON objects BEGIN SELECT RAISE(FAIL, 'refused'); END;"),
    ("no_forget_1", "CREATE TRIGGER no_forget_1 BEFORE DELETE ON objects WHEN old.logical LIKE '%1' BEGIN SELECT RAISE(FAIL, 'refused'); END;"),
    ("no_row", "CREATE TRIGGER no_row BEFORE UPDATE ON saves BEGIN SELECT RAISE(FAIL, 'refused'); END; CREATE TRIGGER no_row_new BEFORE INSERT ON saves BEGIN SELECT RAISE(FAIL, 'refused'); END;"),
];

async fn run(seed: u64) -> Vec<Step> {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let (f, s, dir) = serve(0).await;
    let saves = crate::saves::Saves::open(s.db.clone(), Some(s)).unwrap();
    let s = saves.remote.as_ref().unwrap();
    let mut on = [false; TRIGGERS.len()];
    let mut steps = Vec::new();
    let mut v = 0;
    while steps.len() < 80 {
        let roll = next() % 20;
        if roll == 0 {
            // (an injection on or off between calls)
            let t = (next() % TRIGGERS.len() as u64) as usize;
            let c = s.db.connect().unwrap();
            if on[t] {
                let (name, _) = TRIGGERS[t];
                c.execute_batch(&format!("DROP TRIGGER {name};")).unwrap();
                if name == "no_row" {
                    c.execute_batch("DROP TRIGGER no_row_new;").unwrap();
                }
            } else {
                c.execute_batch(TRIGGERS[t].1).unwrap();
            }
            on[t] = !on[t];
            continue;
        }
        // (a few refusals, which the retries absorb, or more than they try)
        let refused = match next() % 10 {
            0 => (next() % 3 + 1) as usize,
            1 => (RETRIES + 1) as usize + (next() % 3) as usize,
            _ => 0,
        };
        f.fail_next.store(refused, SeqCst);
        let n = (next() % NAMES.len() as u64) as usize;
        let name = NAMES[n];
        let now = row(s, name);
        let mut step = Step { op: "", name: n, e: 0, v: 0, ret: "", rev: 0, got: 0, row: 0, kept: Vec::new() };
        // (a failure with an injection that can cause it is an "err"; with none, a "fault")
        let err = |injected: bool| if injected { "err" } else { "fault" };
        match roll {
            1..=10 => {
                v += 1;
                let e = match next() % 10 {
                    0 => now + 1,
                    1 | 2 => (now - 1).max(0),
                    _ => now,
                };
                step.op = "write";
                (step.e, step.v) = (e, v);
                step.ret = match saves.write(name, e, format!("v{v}").as_bytes()).await {
                    Ok(true) => "yes",
                    Ok(false) => "no",
                    Err(_) => err(refused > 0 || on[2]),
                };
            }
            11..=17 => {
                step.op = "read";
                // (from the bucket, not the plaintext cache: a read sees what the bucket holds)
                *s.shared.cache.locked() = Default::default();
                step.ret = match saves.read(name).await {
                    Ok(None) => "none",
                    Ok(Some((rev, data))) => {
                        step.rev = rev;
                        step.got = std::str::from_utf8(&data).ok().and_then(|d| d.strip_prefix('v')?.parse().ok()).unwrap_or(-1);
                        "got"
                    }
                    Err(_) => err(refused > 0),
                };
            }
            _ => {
                step.op = "gc";
                step.ret = match s.gc(std::time::Duration::ZERO, None).await {
                    Ok(_) => "ok",
                    Err(_) => err(refused > 0 || on[0] || on[1]),
                };
            }
        }
        f.fail_next.store(0, SeqCst);
        (step.row, step.kept) = (row(s, name), kept(s, name));
        steps.push(step);
    }
    let _ = std::fs::remove_dir_all(dir);
    steps
}

/// What the run must show without TLC: the row's revision named, a read back what was written
/// last with a yes, and some of every outcome over the seeds (checked by the caller).
fn check(steps: &[Step], seed: u64) {
    let mut last = [(0, 0); NAMES.len()];
    for (i, t) in steps.iter().enumerate() {
        assert_ne!(t.ret, "fault", "seed {seed} step {i}: {} {} failed with nothing injected", t.op, NAMES[t.name]);
        assert!(t.row == 0 || t.kept.contains(&t.row), "seed {seed} step {i}: revision {} not named: {:?}", t.row, t.kept);
        if t.ret == "yes" {
            last[t.name] = (t.e + 1, t.v);
        }
        if t.ret == "got" {
            assert_eq!((t.rev, t.got), last[t.name], "seed {seed} step {i}: read {}", NAMES[t.name]);
        }
    }
}

fn module(steps: &[Step]) -> String {
    let names = NAMES.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ");
    let mut s = format!("---- MODULE SaveData ----\nSaveRevisions == {SAVE_REVISIONS}\nNames == {{{names}}}\nTrace == <<\n");
    for (i, t) in steps.iter().enumerate() {
        let kept = t.kept.iter().map(i64::to_string).collect::<Vec<_>>().join(", ");
        let _ = writeln!(
            s,
            "  [op |-> \"{}\", name |-> \"{}\", e |-> {}, v |-> {}, ret |-> \"{}\", rev |-> {}, got |-> {}, row |-> {}, kept |-> {{{kept}}}]{}",
            t.op, NAMES[t.name], t.e, t.v, t.ret, t.rev, t.got, t.row, if i + 1 < steps.len() { "," } else { "" }
        );
    }
    s.push_str(">>\n====\n");
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saves_trace() {
    let seeds = std::env::var("MASU_TRACE_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(20u64);
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..seeds {
        let steps = run(seed).await;
        check(&steps, seed);
        seen.extend(steps.iter().map(|t| (t.op, t.ret)));
        // (a forget refused: more than SAVE_REVISIONS revisions named)
        if steps.iter().any(|t| t.kept.len() as i64 > SAVE_REVISIONS) {
            seen.insert(("write", "kept more"));
        }
        let Some(dir) = std::env::var_os("MASU_TRACE_DIR") else { continue };
        let dir = std::path::Path::new(&dir).join(format!("saves-{seed}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SaveData.tla"), module(&steps)).unwrap();
        std::fs::write(dir.join("SPEC"), "SaveApi").unwrap();
    }
    if seeds >= 20 {
        for want in [("write", "yes"), ("write", "no"), ("write", "err"), ("read", "got"), ("read", "none"), ("read", "err"), ("gc", "ok"), ("gc", "err"), ("write", "kept more")] {
            assert!(seen.contains(&want), "no run reached {want:?}: {seen:?}");
        }
    }
}
