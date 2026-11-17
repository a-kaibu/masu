//! The architecture rules: what the 2026-11-14..17 rewrites settled about the repository (one
//! home per primitive, removed functions stay removed, the guards stay), each a [`Rule`]. Rust
//! code is read parsed (syn: items, impls, paths, macro bodies' tokens; #[cfg(test)] modules
//! left out), the other files as text. Each rule's doc says which change set it up and what to
//! write instead; arch_tests.rs proves each finds its regression, in a made-up tree and put back
//! into a real file, and lets the right spelling through.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::code::{line_of, line_text};
use crate::syntax::Items;
use crate::tree::{Rel, Tree};

/// Every rule: a variant without its fixtures in arch_tests.rs does not compile (the fixture
/// table is an exhaustive match).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    BuildOutput,
    HresultCopy,
    KomeCopy,
    GoldenEnv,
    DeadApi,
    UntypedWayBack,
    HomeOnly,
    ReprCLayout,
    ReleaseBuildOnly,
    Required,
    DisallowedNamed,
}

impl Rule {
    pub const ALL: [Rule; 11] = [
        Rule::BuildOutput,
        Rule::HresultCopy,
        Rule::KomeCopy,
        Rule::GoldenEnv,
        Rule::DeadApi,
        Rule::UntypedWayBack,
        Rule::HomeOnly,
        Rule::ReprCLayout,
        Rule::ReleaseBuildOnly,
        Rule::Required,
        Rule::DisallowedNamed,
    ];

    /// the name a site list and a message use
    pub fn id(self) -> &'static str {
        match self {
            Rule::BuildOutput => "build-output",
            Rule::HresultCopy => "hresult-copy",
            Rule::KomeCopy => "kome-copy",
            Rule::GoldenEnv => "golden-env",
            Rule::DeadApi => "dead-api",
            Rule::UntypedWayBack => "untyped-way-back",
            Rule::HomeOnly => "home-only",
            Rule::ReprCLayout => "repr-c-layout",
            Rule::ReleaseBuildOnly => "release-build-only",
            Rule::Required => "required",
            Rule::DisallowedNamed => "disallowed-named",
        }
    }

    /// what the rule keeps and what to write instead
    pub fn why(self) -> &'static str {
        match self {
            Rule::BuildOutput => "no build output in git (a target/ directory, a compiled or packed file outside vendor/ and testdata/): build artifacts never go to GitHub; 99c0d6d6 committed three masu stubs under crates/masu/target",
            Rule::HresultCopy => "sake's HRESULTs live in sake::hresult only (DRY rework 2026-11-15): `use crate::hresult::*` instead of a second `const E_FAIL`",
            Rule::KomeCopy => "conversions, little-endian readers and lock traits live in kome only (95e48ed5): use kome::num / kome::le / kome::locks, not a copy of your own",
            Rule::GoldenEnv => "one golden switch, MASU_UPDATE_GOLDEN, read by kome::golden only: another *_UPDATE_GOLDEN/BLESS name is a golden MASU_UPDATE_GOLDEN=1 leaves stale",
            Rule::DeadApi => "removed stays removed: sake::mem's safe borrows (slice, guest_mut, view, view_mut, guest, in_guest, guest_range; mem::bytes / mem::update copy) and api's untyped out-parameter paths (to_callee, a_at, out_at, put64; api::words!, typed Args)",
            Rule::UntypedWayBack => "a guest address is a value (mem::r32(ga(a) + 4), no reader/writer methods on Ga), and api::Args has no Deref to Ctx (it let c.a(i) read an out-parameter as a word)",
            Rule::HomeOnly => "a primitive sound only under a contract has one list of callers (HOMES in arch.rs): view_unchecked, atomics on guest memory, locked_anyway; a new caller goes in HOMES with its reason",
            Rule::ReprCLayout => "a repr(C) type is a layout something else reads: pin it with `const _: () = assert!(size_of::<T>() == N);` (user rule 2026-11-14)",
            Rule::ReleaseBuildOnly => "release.yml builds and releases (jobs version, web, bin, pages, release; no test suite or linter): CI tests every push",
            Rule::Required => "the guards stay: MASU_OUTS_STRICT, outptr_audit, the compile_fail doctests, the CI steps of these lints",
            Rule::DisallowedNamed => "each clippy.toml disallowed-methods entry's reason starts with its masu-* name (`masu-relay: ...`): the ratchet counts it under that name",
        }
    }

    pub fn check(self, tree: &Tree) -> Vec<Finding> {
        let hits = match self {
            Rule::BuildOutput => build_output(tree),
            Rule::HresultCopy => hresult_copy(tree),
            Rule::KomeCopy => kome_copy(tree),
            Rule::GoldenEnv => golden_env(tree),
            Rule::DeadApi => dead_api(tree),
            Rule::UntypedWayBack => untyped_way_back(tree),
            Rule::HomeOnly => home_only(tree),
            Rule::ReprCLayout => repr_c_layout(tree),
            Rule::ReleaseBuildOnly => release_build_only(tree),
            Rule::Required => required(tree),
            Rule::DisallowedNamed => disallowed_named(tree),
        };
        hits.into_iter().map(|(file, line, what)| Finding { rule: self, file, line, what }).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub rule: Rule,
    pub file: Rel,
    pub line: usize,
    pub what: String,
}

type Hits = Vec<(Rel, usize, String)>;

fn rx(s: &str) -> Option<Regex> {
    Regex::new(s).ok()
}

/// the parsed items of every file in `files` that parses
fn items(tree: &Tree, files: &[Rel]) -> Vec<(Rel, std::sync::Arc<Items>)> {
    files.iter().filter_map(|f| tree.items(f).map(|i| (f.clone(), i))).collect()
}

pub const MEM: &str = "crates/sake/src/mem.rs";
pub const API: &str = "crates/sake/src/api.rs";
pub const HRESULT: &str = "crates/sake/src/hresult.rs";
pub const RELEASE: &str = ".github/workflows/release.yml";

static BUILD_EXT: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"(?i)\.(?:wasm|exe|dll|so|dylib|o|a|lib|rlib|pdb|zst|tar|tgz|7z|zip)$"));
// (vendored sources, test inputs, apitrace traces, sake's guest C runtime: inputs, not outputs)
static BUILD_OK: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"^vendor/|(^|/)testdata/|^crates/sake/src/guest_crt(64)?\.bin$"));
static TARGET_DIR: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"(^|/)target/"));

fn is(r: &LazyLock<Option<Regex>>, s: &str) -> bool {
    r.as_ref().is_some_and(|r| r.is_match(s))
}

fn build_output(tree: &Tree) -> Hits {
    tree.paths()
        .iter()
        .filter(|p| !p.is_rust() && (is(&TARGET_DIR, p.as_str()) || (is(&BUILD_EXT, p.as_str()) && !is(&BUILD_OK, p.as_str()))))
        .map(|p| (p.clone(), 1, p.to_string()))
        .collect()
}

fn hresult_copy(tree: &Tree) -> Hits {
    let home = Rel::new(HRESULT);
    let Some(hr) = tree.syntax(&home) else { return vec![] };
    let names: BTreeSet<String> = hr
        .items
        .iter()
        .filter_map(|i| if let syn::Item::Const(c) = i && matches!(c.vis, syn::Visibility::Public(_)) { Some(c.ident.to_string()) } else { None })
        .collect();
    let files: Vec<Rel> = tree.rs(&["crates/sake/"]).into_iter().filter(|f| *f != home).collect();
    items(tree, &files)
        .into_iter()
        .flat_map(|(f, it)| it.consts.iter().filter(|c| names.contains(&c.0)).map(|(n, l, _)| (f.clone(), *l, format!("const {n}"))).collect::<Vec<_>>())
        .collect()
}

pub const KOME_FNS: [&str; 12] = ["lo8", "lo16", "lo16s", "lo32", "lo64", "us", "u32_of", "u64_of", "to_usize", "u16_at", "u32_at", "u64_at"];

fn kome_copy(tree: &Tree) -> Hits {
    let files: Vec<Rel> = tree.rs(&["crates/"]).into_iter().filter(|f| !f.as_str().starts_with("crates/kome/")).collect();
    let mut out = Hits::new();
    for (f, it) in items(tree, &files) {
        out.extend(it.fns.iter().filter(|(n, _)| KOME_FNS.contains(&n.as_str())).map(|(n, l)| (f.clone(), *l, format!("fn {n}"))));
        out.extend(it.traits.iter().filter(|(n, _)| n == "Lock" || n == "Rw").map(|(n, l)| (f.clone(), *l, format!("trait {n}"))));
    }
    out
}

static GOLDEN_ANY: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\b[A-Z0-9_]*(?:UPDATE_GOLDEN|BLESS|UPDATE_SNAPSHOTS?|REGEN(?:ERATE)?_GOLDEN|UPDATE_EXPECT)[A-Z0-9_]*\b"));
static GOLDEN_FILES: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\.(rs|py|sh|mjs|js|ts|yml|yaml|toml)$"));
pub const GOLDEN_HOME: &str = "crates/kome/src/golden.rs";

fn golden_env(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let Some(any) = GOLDEN_ANY.as_ref() else { return out };
    for p in tree.paths() {
        let s = p.as_str();
        let scanned = ["crates/", "tests/", "tools/", ".github/", "host/", "web/src/", "formal/", "fuzz/"].iter().any(|x| s.starts_with(x));
        if !scanned || s.ends_with(".tsv") || !is(&GOLDEN_FILES, s) || s.starts_with("crates/masu-lint/") {
            continue;
        }
        let src = if p.is_rust() { tree.code(p, true).to_string() } else { tree.text(p) };
        for m in any.find_iter(&src) {
            if m.as_str() != "MASU_UPDATE_GOLDEN" {
                out.push((p.clone(), line_of(&src, m.start()), m.as_str().to_owned()));
            }
        }
        if p.is_rust() && s != GOLDEN_HOME {
            for (k, _) in src.match_indices("\"MASU_UPDATE_GOLDEN\"") {
                out.push((p.clone(), line_of(&src, k), "reads MASU_UPDATE_GOLDEN itself (kome::golden::updating)".into()));
            }
        }
    }
    out
}

pub const GONE: [(&str, &[&str]); 2] = [
    // (review 2026-11-15/16: safe borrows of guest memory let another thread's write race the
    // borrow; mem::bytes / mem::update copy, view_unchecked says its contract)
    (MEM, &["slice", "slice_mut", "guest", "guest_mut", "in_guest", "guest_range", "view", "view_mut"]),
    // (out-parameters 2026-11-16: an Out reaches a call word only through api::words!, an argument
    // only through the function's typed Args)
    (API, &["to_callee", "a_at", "out_at", "put64"]),
];

fn dead_api(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for (file, names) in GONE {
        let f = Rel::new(file);
        for (_, it) in items(tree, std::slice::from_ref(&f)) {
            let all = it.fns.iter().map(|(n, l)| (n, *l)).chain(it.methods.iter().map(|(_, _, n, l)| (n, *l)));
            out.extend(all.filter(|(n, _)| names.contains(&n.as_str())).map(|(n, l)| (f.clone(), l, format!("fn {n}"))));
        }
    }
    out
}

/// what Ga may not have: `r`, `w`, `r8`..`w64`, `rp`/`wp`, `rs`/`ws`
fn is_accessor(n: &str) -> bool {
    let rest = n.strip_prefix('r').or_else(|| n.strip_prefix('w'));
    rest.is_some_and(|r| ["", "8", "16", "32", "64", "p", "s"].contains(&r))
}

fn untyped_way_back(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for (f, it) in items(tree, &tree.rs(&["crates/sake/"])) {
        for (ty, _, n, l) in &it.methods {
            if ty.as_deref() == Some("Ga") && is_accessor(n) {
                out.push((f.clone(), *l, format!("Ga::{n}: a reader/writer method on Ga")));
            }
        }
        for (ty, tr, l) in &it.impls {
            if ty.as_deref() == Some("Args") && matches!(tr.as_deref(), Some("Deref" | "DerefMut")) {
                out.push((f.clone(), *l, "Deref for Args".into()));
            }
        }
    }
    out
}

/// what may be used only in its homes, and where
pub enum Use {
    /// an identifier (a call, a method, a path's last segment)
    Ident(&'static [&'static str]),
    /// `A::b` with A one of the first, b the second
    Path(&'static [&'static str], &'static str),
}

pub const HOMES: [(&str, Use, &[&str]); 3] = [
    // (2026-11-16: the zero-copy reads of guest memory, each with its contract: ReadFile-like host
    // file reads and decoded video frames)
    (
        "view-unchecked",
        Use::Ident(&["view_unchecked", "view_mut_unchecked"]),
        &[MEM, "crates/sake/src/k32.rs", "crates/sake/src/files.rs", "crates/sake/src/dshow_mpeg.rs", "crates/sake/src/dshow_mpeg/dmo.rs", "crates/sake/src/mf/plat.rs", "crates/sake/src/amstream.rs"],
    ),
    // (guest memory as atomics: the CPU's locked instructions (kura cpu, cmpxchg16 swap16), sake
    // mem's atomic_rmw32 through mem::range, crashlog's reader with its Own token, review
    // 2026-11-16: anything else reading guest memory goes through sake::mem)
    (
        "guest-atomics",
        Use::Path(&["AtomicU8", "AtomicU16", "AtomicU32", "AtomicU64", "AtomicI8", "AtomicI16", "AtomicI32", "AtomicI64"], "from_ptr"),
        &["crates/kura/src/crashlog.rs", "crates/kura/src/cpu.rs", MEM],
    ),
    // (review 2026-11-15: a poisoned lock panics; only crashlog's whole-per-change lists go on)
    ("locked-anyway", Use::Ident(&["locked_anyway"]), &["crates/kura/src/crashlog.rs", "crates/kome/src/locks.rs"]),
];

fn home_only(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let all = items(tree, &tree.rs(&["crates/"]));
    for (name, what, homes) in &HOMES {
        for (f, it) in &all {
            if homes.contains(&f.as_str()) {
                continue;
            }
            let mut lines: BTreeSet<usize> = BTreeSet::new();
            match what {
                Use::Ident(ids) => lines.extend(it.idents.iter().filter(|(i, _)| ids.contains(&i.as_str())).map(|(_, l)| *l)),
                Use::Path(tys, m) => lines.extend(it.paths2.iter().filter(|(a, b, _)| tys.contains(&a.as_str()) && b == m).map(|(_, _, l)| *l)),
            }
            out.extend(lines.into_iter().map(|l| (f.clone(), l, format!("{name} outside its homes"))));
        }
    }
    out
}

fn repr_c_layout(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for (f, it) in items(tree, &tree.rs(&["crates/"])) {
        for (n, l) in &it.repr_c {
            let Some(pin) = rx(&format!(r"\bsize_of\s*::\s*<\s*{n}\s*>")) else { continue };
            if !it.consts.iter().any(|(c, _, v)| c == "_" && pin.is_match(v)) {
                out.push((f.clone(), *l, format!("repr(C) {n} without a size assert")));
            }
        }
    }
    out
}

pub const RELEASE_JOBS: [&str; 5] = ["version", "web", "bin", "pages", "release"];
static JOB: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"(?m)^  ([A-Za-z0-9_-]+):\s*$"));
static CHECKS: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\b(?:cargo (?:test|clippy|nextest|mutants)|pnpm (?:run )?test|vp (?:test|check)|masu-lint)\b"));

fn release_build_only(tree: &Tree) -> Hits {
    let f = Rel::new(RELEASE);
    let src = tree.text(&f);
    let mut out = Hits::new();
    let jobs_at = if src.starts_with("jobs:\n") { Some(0) } else { src.find("\njobs:\n") };
    if let (Some(at), Some(job)) = (jobs_at, JOB.as_ref()) {
        let body = src.get(at..).unwrap_or_default();
        for c in job.captures_iter(body) {
            if let Some(m) = c.get(1)
                && !RELEASE_JOBS.contains(&m.as_str())
            {
                out.push((f.clone(), line_of(&src, at.saturating_add(m.start())), format!("job {}", m.as_str())));
            }
        }
    }
    if let Some(ch) = CHECKS.as_ref() {
        out.extend(ch.find_iter(&src).map(|m| (f.clone(), line_of(&src, m.start()), m.as_str().to_owned())));
    }
    out
}

/// what must stay in place: (file, text, why)
pub const REQUIRED: [(&str, &str, &str); 8] = [
    ("tests/host-tests.sh", "export MASU_OUTS_STRICT=1", "release-build host tests fail on a 32-bit write into a pointer-sized out-parameter (2026-11-16)"),
    ("crates/sake/src/lib.rs", "mod outptr_audit;", "every handler of a function with pointer-sized out-parameters takes its typed Args"),
    ("crates/kura/src/pool.rs", "```compile_fail", "Pool Lease/brand misuse does not compile (review 2026-11-15/16)"),
    (MEM, "```compile_fail", "Ga has no accessors; a guest borrow does not outlive its closure"),
    (API, "```compile_fail", "an out-parameter is not a word"),
    (".github/workflows/ci.yml", "cargo run -q --release -p masu-lint -- arch", "this lint runs on every push, before the build"),
    (".github/workflows/ci.yml", "cargo run -q --release -p masu-lint -- ratchet", "the lint ratchet runs on every push"),
    (".github/workflows/ci.yml", "cargo test --workspace --release", "masu-lint's own tests (each rule and each ratchet lint fires on its regression) run on every push"),
];

fn required(tree: &Tree) -> Hits {
    REQUIRED
        .iter()
        .filter(|(p, _, _)| !tree.is_made_up() || tree.has(&Rel::new(p)))
        .filter(|(p, t, _)| !tree.text(&Rel::new(p)).contains(t))
        .map(|(p, t, why)| (Rel::new(p), 1, format!("missing `{t}`: {why}")))
        .collect()
}

static DISALLOWED: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r#"\{\s*path\s*=\s*"([^"]+)"\s*,\s*reason\s*=\s*"([^"]*)""#));

/// clippy.toml's disallowed-methods: path -> its masu-* name (None: the reason names none)
pub fn disallowed(clippy_toml: &str) -> BTreeMap<String, Option<String>> {
    let Some(r) = DISALLOWED.as_ref() else { return BTreeMap::new() };
    r.captures_iter(clippy_toml)
        .filter_map(|c| {
            let path = c.get(1)?.as_str().to_owned();
            let reason = c.get(2)?.as_str();
            let name = reason.split(':').next().filter(|n| n.starts_with("masu-") && !n.contains(' ')).map(str::to_owned);
            Some((path, name))
        })
        .collect()
}

fn disallowed_named(tree: &Tree) -> Hits {
    let f = Rel::new("clippy.toml");
    let src = tree.text(&f);
    disallowed(&src).into_iter().filter(|(_, n)| n.is_none()).map(|(p, _)| (f.clone(), 1, format!("{p}: its reason does not start with a masu-* name"))).collect()
}

// --- sites ------------------------------------------------------------------------------------

/// A finding as a site list knows it: (rule, file, the line's code with spaces collapsed; for
/// a file-level finding, what it is).
pub type Site = (String, Rel, String);

pub fn site(tree: &Tree, f: &Finding) -> Site {
    let text = if f.line > 1 || f.file.is_rust() { line_text(&if f.file.is_rust() { tree.code(&f.file, true).to_string() } else { tree.text(&f.file) }, f.line) } else { f.what.clone() };
    (f.rule.id().to_owned(), f.file.clone(), text)
}

pub fn check(tree: &Tree, rules: &[Rule]) -> BTreeMap<Site, Vec<Finding>> {
    let mut out: BTreeMap<Site, Vec<Finding>> = BTreeMap::new();
    for r in rules {
        for f in r.check(tree) {
            out.entry(site(tree, &f)).or_default().push(f);
        }
    }
    out
}

/// the known sites: (site, why it stays)
pub fn load_sites(text: &str) -> Vec<(Site, String)> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let mut c = l.splitn(4, '\t');
            let (r, f, t) = (c.next()?, c.next()?, c.next()?);
            Some(((r.to_owned(), Rel::new(f), t.to_owned()), c.next().unwrap_or_default().to_owned()))
        })
        .collect()
}

pub fn known_counts(known: &[(Site, String)]) -> BTreeMap<Site, usize> {
    let mut m: BTreeMap<Site, usize> = BTreeMap::new();
    for (s, _) in known {
        let e = m.entry(s.clone()).or_default();
        *e = e.saturating_add(1);
    }
    m
}

/// the findings beyond the known sites
pub fn fresh(found: &BTreeMap<Site, Vec<Finding>>, known: &BTreeMap<Site, usize>) -> Vec<Finding> {
    found.iter().flat_map(|(s, v)| v.iter().skip(known.get(s).copied().unwrap_or(0)).cloned()).collect()
}

#[cfg(test)]
#[path = "arch_tests.rs"]
mod tests;
