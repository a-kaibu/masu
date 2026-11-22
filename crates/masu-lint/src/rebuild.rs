//! Rules from the sake ABI / state redesign (review of 6852be1f, 2026-11-22): each table lists
//! what a fixed file must keep saying (the fix) and what it must not say again (the bug), with
//! the bug it stands for. Rust files are read without comments and without their inline test
//! module; other files (Python, shell, JavaScript, workflows) as they are.
use super::{Hits, Rel, Tree};

/// (file, text that must be there / must not be there, what its absence / presence means)
pub type Needle = (&'static str, &'static str, &'static str);

fn body(tree: &Tree, f: &Rel) -> String {
    if f.is_rust() { super::regress::product_code(tree, f) } else { tree.text(f) }
}

/// Each `want` missing from its file, each `not` in it.
pub(super) fn needles(tree: &Tree, want: &[Needle], not: &[Needle]) -> Hits {
    let mut out = Hits::new();
    for (file, needle, why) in want {
        let f = Rel::new(file);
        if !body(tree, &f).contains(needle) {
            out.push((f, 1, format!("no `{needle}`: {why}")));
        }
    }
    for (file, needle, why) in not {
        let f = Rel::new(file);
        let text = body(tree, &f);
        if let Some(at) = text.find(needle) {
            let line = text.get(..at).map_or(1, |b| b.lines().count().max(1));
            out.push((f, line, format!("`{needle}`: {why}")));
        }
    }
    out
}

// --- run-plan --------------------------------------------------------------------------------

/// The runners write their plan before running, compare.py and run.py fail on a plan problem.
pub const RUN_PLAN: [Needle; 7] = [
    ("tests/winref/run.sh", "> \"$OUT/.plan/$runner\"", "run.sh writes no plan: a run that ran nothing compares 0 probes and passes"),
    ("tests/winref/run-browser.mjs", "writeFileSync(join(outDir, \".plan\", runner)", "run-browser.mjs writes no plan: a lost shard passes"),
    ("tests/winref/compare.py", "sys.exit(1 if failed or plan_errors else 0)", "compare.py does not fail on a plan problem (0 compared, a planned probe without a result, a lost shard)"),
    ("tests/winref/compare.py", "plan.problems(plans, required, got)", "compare.py does not check the run's plan"),
    ("tests/winetest/run.py", "all_jobs, unplanned = plan(", "run.py does not plan its tests before sharding: a missing test program passes with 0 run"),
    ("tests/winetest/run.py", "if unplanned:", "run.py does not fail on what it could not plan"),
    (".github/workflows/winref.yml", "--require-plan browser-0of4", "the final winref comparison does not require each browser shard's plan: a lost shard passes"),
];

/// The two spellings that made a run without results pass.
pub const RUN_PLAN_NOT: [Needle; 2] = [
    ("tests/winref/compare.py", "continue  # (not run: a filtered run.sh)", "a probe without a result skipped as if filtered out: an empty result directory passed"),
    ("tests/winetest/run.py", "print(f\"missing {exe}\", file=sys.stderr)\n            continue", "a missing test program printed and skipped: the run passed with nothing run"),
];
