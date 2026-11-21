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
    SafeRawPointerAbi,
    UncheckedJitMode,
    OutEscape,
    MemoryPointer,
    PagedReadCast,
    ShaderLayoutPanic,
    StoreDeletion,
    EscapeComment,
    ConstantHandler,
    ConstantMethod,
    ArcTryUnwrap,
    ArcCount,
    CatchUnwindAbort,
    ShellSpecialEnv,
    DepthGuard,
    BooleanArg,
    TypedRegisters,
}

impl Rule {
    pub const ALL: [Rule; 28] = [
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
        Rule::SafeRawPointerAbi,
        Rule::UncheckedJitMode,
        Rule::OutEscape,
        Rule::MemoryPointer,
        Rule::PagedReadCast,
        Rule::ShaderLayoutPanic,
        Rule::StoreDeletion,
        Rule::EscapeComment,
        Rule::ConstantHandler,
        Rule::ConstantMethod,
        Rule::ArcTryUnwrap,
        Rule::ArcCount,
        Rule::CatchUnwindAbort,
        Rule::ShellSpecialEnv,
        Rule::DepthGuard,
        Rule::BooleanArg,
        Rule::TypedRegisters,
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
            Rule::SafeRawPointerAbi => "safe-raw-pointer-abi",
            Rule::UncheckedJitMode => "unchecked-jit-mode",
            Rule::OutEscape => "out-escape",
            Rule::MemoryPointer => "memory-pointer",
            Rule::PagedReadCast => "paged-read-cast",
            Rule::ShaderLayoutPanic => "shader-layout-panic",
            Rule::StoreDeletion => "store-deletion",
            Rule::EscapeComment => "escape-comment",
            Rule::ConstantHandler => "constant-handler",
            Rule::ConstantMethod => "constant-method",
            Rule::ArcTryUnwrap => "arc-try-unwrap",
            Rule::ArcCount => "arc-count",
            Rule::CatchUnwindAbort => "catch-unwind-abort",
            Rule::ShellSpecialEnv => "shell-special-env",
            Rule::DepthGuard => "depth-guard",
            Rule::BooleanArg => "boolean-arg",
            Rule::TypedRegisters => "typed-registers",
        }
    }

    /// what the rule keeps and what to write instead
    pub fn why(self) -> &'static str {
        match self {
            Rule::BuildOutput => "no build output in git (a target/ directory, a compiled or packed file outside vendor/ and testdata/): build artifacts never go to GitHub; 99c0d6d6 committed three masu stubs under crates/masu/target",
            Rule::HresultCopy => "sake's HRESULTs live in sake::hresult only (DRY rework 2026-11-15): `use crate::hresult::*` instead of a second `const E_FAIL`",
            Rule::KomeCopy => "conversions, little-endian readers, GUIDs and lock traits live in kome only (95e48ed5, kome::guid): use kome::num / kome::le / kome::guid / kome::locks (a GUID in guest memory: mem::array::<16>, mem::write), not a copy of your own",
            Rule::GoldenEnv => "one golden switch, MASU_UPDATE_GOLDEN, read by kome::golden only: another *_UPDATE_GOLDEN/BLESS name is a golden MASU_UPDATE_GOLDEN=1 leaves stale",
            Rule::DeadApi => "removed stays removed: sake::mem's safe borrows (slice, guest_mut, view, view_mut, guest, in_guest, guest_range; mem::bytes / mem::update copy) and api's untyped out-parameter paths (to_callee, a_at, out_at, put64; api::words!, typed Args)",
            Rule::UntypedWayBack => "a guest address is a value (mem::r32(ga(a) + 4), no reader/writer methods on Ga), and api::Args has no Deref to Ctx (it let c.a(i) read an out-parameter as a word)",
            Rule::HomeOnly => "a primitive sound only under a contract has one list of callers (HOMES in arch.rs): view_unchecked, atomics on guest memory, locked_anyway, guest slices, two-word LOCK operations, pool brands, adopting the page's JIT parts; a new caller goes in HOMES with its reason",
            Rule::ReprCLayout => "a repr(C) type is a layout something else reads: pin it with `const _: () = assert!(size_of::<T>() == N);` (user rule 2026-11-14)",
            Rule::ReleaseBuildOnly => "release.yml builds and releases (jobs version, web, bin, pages, release; no test suite or linter): CI tests every push",
            Rule::Required => "the guards stay: MASU_OUTS_STRICT, outptr_audit, the compile_fail doctests, the CI steps of these lints",
            Rule::DisallowedNamed => "each clippy.toml disallowed-methods entry's reason starts with its masu-* name (`masu-relay: ...`): the ratchet counts it under that name",
            Rule::SafeRawPointerAbi => "an extern fn that takes a raw pointer is `unsafe extern` with a # Safety section (review 2026-11-17, vnhost_jit_late): a safe one lets safe Rust hand it any address; better, take an owned or typed value",
            Rule::UncheckedJitMode => "the JIT without range checks (koji::jit::NOCHECK, MASU_JIT_NOCHECK) is for measuring only (review 2026-11-17): the switch starts off, only kura's jit.rs may set it, and no build, CI step, script or page turns it on",
            Rule::MemoryPointer => "koji Memory operations must keep host pointers inside the implementation",
            Rule::PagedReadCast => "use checked conversion for file positions and byte counts in Pages::read",
            Rule::ShaderLayoutPanic => "propagate GLSL array layout failures as compiler errors",
            Rule::StoreDeletion => "retired bucket objects must pass through gc before physical deletion",
            Rule::EscapeComment => "no comment excuses code from Windows' behavior (user rule 2026-11-20): implement what Windows does instead of writing the shortcut down; \"not supported\" / an approximation / ignored flags pass only with Windows' evidence in the same comment (winref, a golden, MEASURED, Wine's tests, the spec)",
            Rule::ConstantHandler => "a sake handler that ignores its arguments and returns a constant is an empty success unless its doc names the Windows answer it is (winref, a golden, MEASURED, Wine's tests): implement the call, or cite the evidence (user rule 2026-11-20)",
            Rule::ConstantMethod => "a Direct3D object's method (kura d3d9/d3d11, sake's d3d9 glue) that does nothing and answers a constant (an empty arm, S_OK, 0, E_NOTIMPL) says beside it which Windows answer that is (winref, a golden, MEASURED, Wine's tests, the spec); otherwise it is a method to implement (user rule 2026-11-20; ClearUnorderedAccessView* and ClearView were silent no-ops)",
            Rule::ArcTryUnwrap => "no Arc::try_unwrap (Arc audit 2026-11-20, mf release_data): two last holders letting go at once both get Err and neither runs the last-one-out step; Arc::into_inner, or better a Drop that does that step (mf::source::Data)",
            Rule::ArcCount => "a decision on Arc::strong_count / weak_count holds only while nobody can clone or drop one meanwhile (Arc audit 2026-11-20): each such site is a known site saying which lock makes it so (saves NameLocks, storage gate: the map lock every clone is made under); else let a Drop or Arc::into_inner decide",
            Rule::CatchUnwindAbort => "sake.wasm is built with panic=abort, so catch_unwind in a crate it links never catches (82cdd0d0: naga's GLSL panic ended the game through a catch_unwind; review 2026-11-14 P1-7): return a Result, or check the input before the call",
            Rule::ShellSpecialEnv => "a workflow's env or a script does not set one of bash's own variables (bench.yml's inputs went through GROUPS: a script's GROUPS=.. is dropped, RANDOM/SECONDS/LINENO are replaced, whether an env value gets in depends on the variable and the bash version): give it a name of its own",
            Rule::DepthGuard => "the depth limits on recursion over game and file input stay (regress::DEPTH_GUARDS; review b00/b04/b07: damaged traces, MSI conditions, shaders, .x files nesting without end blew the stack): keep the const and the check against it; a new limit is added to the list",
            Rule::BooleanArg => "a BOOLEAN parameter is its low byte (2026-11-20, RtlAnsiStringToUnicodeString read c.a(2) != 0): read it with c.boolean(i); win32.rs BOOLEAN_ARGS lists them",
            Rule::TypedRegisters => "koji's registers are named, not numbered (2026-11-20, Farcall comparison): Cpu::regs/hi are Gprs indexed by Gpr (EAX..EDI), sreg/sbase Segs indexed by Seg, Cpu::gpr takes a Reg64; write the register's name (koji::EBX, Seg::Ss, Reg64::R8), not Gpr::ALL[3] or values()[3], and give Gprs/Segs no index, conversion or public field that takes a number",
            Rule::OutEscape => "a LockWord's or an Object's key() names a lock or one of sake's objects, it is no guest address (review 2026-11-17, 06dc4bf2): it is not the address of mem's readers and writers or of ga()/Ga::from (a value written into memory is fine); take the lock through LockWord::atomic, the object through its table",
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
            Rule::SafeRawPointerAbi => safe_raw_pointer_abi(tree),
            Rule::UncheckedJitMode => unchecked_jit_mode(tree),
            Rule::OutEscape => out_escape(tree),
            Rule::MemoryPointer => boundaries::check(self, tree, boundaries::MEMORY),
            Rule::PagedReadCast => boundaries::check(self, tree, boundaries::PAGES),
            Rule::ShaderLayoutPanic => boundaries::check(self, tree, boundaries::GLSL),
            Rule::StoreDeletion => boundaries::check(self, tree, boundaries::STORAGE),
            Rule::EscapeComment => crate::escape::escape_comment(tree),
            Rule::ConstantHandler => crate::escape::constant_handler(tree),
            Rule::ConstantMethod => crate::escape::constant_method(tree),
            Rule::ArcTryUnwrap => arc_use(tree, &["try_unwrap"]),
            Rule::ArcCount => arc_use(tree, &["strong_count", "weak_count"]),
            Rule::CatchUnwindAbort => regress::catch_unwind_abort(tree),
            Rule::ShellSpecialEnv => regress::shell_special_env(tree),
            Rule::DepthGuard => regress::depth_guard(tree),
            Rule::BooleanArg => crate::win32::boolean_arg(tree),
            Rule::TypedRegisters => typed_registers(tree),
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

#[path = "boundaries.rs"]
mod boundaries;
#[path = "regress.rs"]
mod regress;

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

pub const KOJI_LIB: &str = "crates/koji/src/lib.rs";

/// A register named by a number: `Gpr::ALL[3]`, `Reg64::ALL[8]`, `Seg::ALL[2]`, `regs.values()[4]`
static REG_BY_NUMBER: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\b(?:Gpr|Reg64|Seg)\s*::\s*ALL\s*\[\s*\d|\.values\s*\(\s*\)\s*\[\s*\d"));
/// In koji's lib.rs, what would let a number be a register again: the Cpu fields as plain arrays,
/// a public array inside Gprs/Segs, an Index by a number or a From of one
static REG_UNTYPED: LazyLock<Option<Regex>> = LazyLock::new(|| {
    rx(r"pub\s+(?:regs|hi|sreg|sbase)\s*:\s*\[|struct\s+(?:Gprs|Segs\s*<\s*T\s*>)\s*\(\s*pub\b|Index(?:Mut)?\s*<\s*(?:usize|u8|u16|u32|u64|i32|isize)\s*>\s*for\s+(?:Gprs|Segs)|From\s*<\s*(?:usize|u8|u16|u32|u64|i32|isize)\s*>\s*for\s+(?:Gpr|Reg64|Seg)\b")
});

fn typed_registers(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let (Some(num), Some(untyped)) = (REG_BY_NUMBER.as_ref(), REG_UNTYPED.as_ref()) else { return out };
    for p in tree.paths() {
        let s = p.as_str();
        // (tests too: a test that names a register by number tests the wrong one as surely)
        if !p.is_rust() || s.starts_with("crates/masu-lint/") || !["crates/", "tools/", "fuzz/"].iter().any(|x| s.starts_with(x)) {
            continue;
        }
        let src = tree.code(p, false);
        out.extend(num.find_iter(&src).map(|m| (p.clone(), line_of(&src, m.start()), m.as_str().to_owned())));
        if s == KOJI_LIB {
            out.extend(untyped.find_iter(&src).map(|m| (p.clone(), line_of(&src, m.start()), m.as_str().to_owned())));
        }
    }
    out
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

pub const KOME_FNS: [&str; 18] = [
    "lo8", "lo16", "lo16s", "lo32", "lo64", "us", "u32_of", "u64_of", "to_usize", "u16_at", "u32_at", "u64_at",
    // (kome::guid, 2026-11-20: eight GUID builders, six parsers, three printers and eleven guest
    // readers/writers were copies: kome::guid::{guid, parse, of_text, braced, text},
    // mem::array::<16> / mem::write)
    "guid", "guid_string", "fmt_guid", "guid_at", "read_guid", "put_guid",
];

fn kome_copy(tree: &Tree) -> Hits {
    let files: Vec<Rel> = tree.rs(&["crates/"]).into_iter().filter(|f| !f.as_str().starts_with("crates/kome/")).collect();
    let mut out = Hits::new();
    for (f, it) in items(tree, &files) {
        // (sake's one guest GUID reader that may get a null pointer lives in mem.rs)
        let home = |n: &str| f.as_str() == MEM && n == "guid_at";
        out.extend(it.fns.iter().filter(|(n, _)| KOME_FNS.contains(&n.as_str()) && !home(n)).map(|(n, l)| (f.clone(), *l, format!("fn {n}"))));
        out.extend(it.byte_readers.iter().map(|(n, l)| (f.clone(), *l, format!("fn {n}: a number from bytes at an offset (kome::le::u32_at, u16_be_at...)"))));
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

pub const GONE: [(&str, &[&str]); 3] = [
    // (review 2026-11-15/16: safe borrows of guest memory let another thread's write race the
    // borrow; mem::bytes / mem::update copy, view_unchecked says its contract)
    (MEM, &["slice", "slice_mut", "guest", "guest_mut", "in_guest", "guest_range", "view", "view_mut"]),
    // (out-parameters 2026-11-16: an Out reaches a call word only through api::words!, an argument
    // only through the function's typed Args)
    (API, &["to_callee", "a_at", "out_at", "put64"]),
    // (review 2026-11-17: safe mem functions made Rust slices of shared guest memory through these;
    // safe code copies by raw pointer now, a slice exists only under exclusive_slice's contract)
    (MEM, &["raw", "raw_mut", "raw_or_empty"]),
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
    for (f, it) in items(tree, &tree.rs(&["crates/sake/", "crates/kura/src/guest.rs"])) {
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

pub const HOMES: [(&str, Use, &[&str]); 8] = [
    ("bucket-deletion", Use::Ident(&["delete_object"]), &[boundaries::STORAGE]),
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
    // (review 2026-11-17: the one place a Rust slice of guest memory is made, under view_unchecked's
    // contract; everything else in sake copies)
    ("guest-slice", Use::Ident(&["exclusive_slice", "exclusive_slice_mut"]), &[MEM]),
    // (review 2026-11-17: LOCK operations across two aligned words, one rule and one lock for all:
    // the interpreter's (kura cpu) and the Interlocked functions' (sake mem); anyone else would
    // take another lock of their own)
    ("locked-pair", Use::Path(&["locked"], "pair"), &["crates/kura/src/cpu.rs", MEM, "crates/waits/src/locked.rs", "crates/waits/tests/loom.rs"]),
    // (review 2026-11-17: a pool brand has one value ever, made by kura's pool! with its one pool;
    // an `unsafe impl Brand` elsewhere could make two pools of one brand)
    ("pool-brand", Use::Ident(&["Brand"]), &["crates/kura/src/pool.rs"]),
    // (review 2026-11-17: the page's late JIT parts are adopted once, by vnhost_jit_late's contract)
    ("late-part-adopt", Use::Path(&["LatePart"], "adopt"), &["crates/kura/src/jit.rs"]),
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
pub const REQUIRED: [(&str, &str, &str); 9] = [
    ("crates/kura/src/d3d9/device.rs", "    pub sub: Sub,", "a Direct3D 9 surface names its part of the page texture as a Sub (level and cube face together): a bare level drew every face of a render target cube on layer 0 (2026-11-20)"),
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

/// exported safe `extern "ABI" fn`s with a raw pointer among their parameters (#[cfg(test)]
/// modules left out, and the files only a #[cfg(test)] `mod x;` brings in)
#[derive(Default)]
struct SafeAbiPtr(Vec<(String, usize)>);

impl<'a> syn::visit::Visit<'a> for SafeAbiPtr {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !crate::syntax::is_cfg_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if !crate::syntax::is_cfg_test(&f.attrs) {
            self.sig(&f.attrs, &f.sig);
            syn::visit::visit_item_fn(self, f);
        }
    }
    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        if !crate::syntax::is_cfg_test(&f.attrs) {
            self.sig(&f.attrs, &f.sig);
            syn::visit::visit_impl_item_fn(self, f);
        }
    }
}

/// `#[no_mangle]`, `#[unsafe(no_mangle)]`, `#[export_name = ..]`: callable from outside Rust
fn exported(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        let toks = match &a.meta {
            syn::Meta::Path(p) => quote::quote!(#p).to_string(),
            syn::Meta::List(l) => format!("{} {}", l.path.get_ident().map(|i| i.to_string()).unwrap_or_default(), l.tokens),
            syn::Meta::NameValue(n) => n.path.get_ident().map(|i| i.to_string()).unwrap_or_default(),
        };
        toks.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == "no_mangle" || w == "export_name")
    })
}

impl SafeAbiPtr {
    fn sig(&mut self, attrs: &[syn::Attribute], s: &syn::Signature) {
        if s.abi.is_none() || s.unsafety.is_some() || !exported(attrs) {
            return;
        }
        let takes_ptr = s.inputs.iter().any(|a| match a {
            syn::FnArg::Typed(t) => has_ptr(&t.ty),
            syn::FnArg::Receiver(_) => false,
        });
        if takes_ptr {
            self.0.push((s.ident.to_string(), crate::syntax::line(s.fn_token.span)));
        }
    }
}

/// a raw pointer in `ty` (also inside Option<..>, a tuple, an array), not behind a fn pointer type
fn has_ptr(ty: &syn::Type) -> bool {
    #[derive(Default)]
    struct P(bool);
    impl<'a> syn::visit::Visit<'a> for P {
        fn visit_type_ptr(&mut self, _: &'a syn::TypePtr) {
            self.0 = true;
        }
        // (a callback's own parameters are its contract, not this fn's)
        fn visit_type_bare_fn(&mut self, _: &'a syn::TypeBareFn) {}
    }
    let mut p = P::default();
    syn::visit::Visit::visit_type(&mut p, ty);
    p.0
}

/// the files a `#[cfg(test)] mod x;` brings in (x.rs or x/mod.rs beside the declaring file)
fn test_mod_files(tree: &Tree, files: &[Rel]) -> BTreeSet<Rel> {
    let mut out = BTreeSet::new();
    for f in files {
        let Some(file) = tree.syntax(f) else { continue };
        let s = f.as_str();
        let dir = match s.rsplit_once('/') {
            Some((d, "mod.rs" | "lib.rs" | "main.rs")) => d.to_owned(),
            Some(_) => s.trim_end_matches(".rs").to_owned(),
            None => String::new(),
        };
        for it in &file.items {
            if let syn::Item::Mod(m) = it
                && m.content.is_none()
                && crate::syntax::is_cfg_test(&m.attrs)
                && !m.attrs.iter().any(|a| a.path().is_ident("path"))
            {
                out.insert(Rel(format!("{dir}/{}.rs", m.ident)));
                out.insert(Rel(format!("{dir}/{}/mod.rs", m.ident)));
            }
        }
    }
    out
}

fn safe_raw_pointer_abi(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let files = tree.rs(&["crates/"]);
    let tests = test_mod_files(tree, &files);
    for f in files.iter().filter(|f| !tests.contains(*f)) {
        let Some(file) = tree.syntax(f) else { continue };
        let mut v = SafeAbiPtr::default();
        syn::visit::Visit::visit_file(&mut v, &file);
        out.extend(v.0.into_iter().map(|(n, l)| (f.clone(), l, format!("safe extern fn {n} takes a raw pointer"))));
    }
    out
}

/// koji's switch and where it may be set
pub const NOCHECK_HOME: &str = "crates/koji/src/jit/mod.rs";
pub const NOCHECK_SETTER: &str = "crates/kura/src/jit.rs";
static NOCHECK_STATIC: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\bstatic\s+NOCHECK\s*:[^=]*=\s*([^;]*);"));
static NOCHECK_ON: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"MASU_JIT_NOCHECK\s*[=:]\s*['\x22]?[^0'\x22\s]"));

fn unchecked_jit_mode(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let home = Rel::new(NOCHECK_HOME);
    if tree.has(&home) {
        let code = tree.code(&home, true);
        match NOCHECK_STATIC.as_ref().and_then(|r| r.captures(&code)) {
            Some(c) => {
                let init: String = c.get(1).map_or("", |m| m.as_str()).split_whitespace().collect();
                if !init.ends_with("AtomicBool::new(false)") {
                    out.push((home.clone(), c.get(0).map_or(1, |m| line_of(&code, m.start())), format!("NOCHECK starts as {init}, not off")));
                }
            }
            None if !tree.is_made_up() => out.push((home.clone(), 1, "static NOCHECK is gone: move this rule with it".into())),
            None => {}
        }
    }
    for (f, it) in items(tree, &tree.rs(&["crates/"])) {
        if f.as_str() == NOCHECK_HOME || f.as_str() == NOCHECK_SETTER {
            continue;
        }
        let lines: BTreeSet<usize> = it.idents.iter().filter(|(i, _)| i == "NOCHECK").map(|(_, l)| *l).collect();
        out.extend(lines.into_iter().map(|l| (f.clone(), l, "NOCHECK outside koji's jit and kura's jit.rs".into())));
    }
    let Some(on) = NOCHECK_ON.as_ref() else { return out };
    for p in tree.paths() {
        let s = p.as_str();
        let scanned = [".github/", "tests/", "tools/", "scripts/", "web/src/", "host/", ".cargo/", "Cargo.toml", "crates/"].iter().any(|x| s.starts_with(x));
        if !scanned || s.ends_with(".md") || s.starts_with("crates/masu-lint/") || (p.is_rust() && p.is_test_file()) {
            continue;
        }
        if p.is_rust() {
            // (only kura's init_flags reads the variable; a set_var or a second reader elsewhere is a
            // way to turn it on)
            if s != NOCHECK_SETTER {
                let src = tree.code(p, true);
                for (k, _) in src.match_indices("\"MASU_JIT_NOCHECK\"") {
                    out.push((p.clone(), line_of(&src, k), "names MASU_JIT_NOCHECK outside kura's jit.rs".into()));
                }
            }
            continue;
        }
        let src = tree.text(p);
        for m in on.find_iter(&src) {
            out.push((p.clone(), line_of(&src, m.start()), format!("turns the unchecked JIT on: {}", m.as_str())));
        }
    }
    out
}

/// sake::mem's functions whose first argument is a guest address (copy: its first two), and the
/// ways to make a Ga
pub const ADDRESSED: [&str; 34] = [
    "rp", "wp", "putp", "r8", "r16", "r32", "r64", "w8", "w16", "w32", "w64", "put32", "put64", "put_addr", "bytes", "array", "update", "view_unchecked", "view_mut_unchecked", "host_ptr", "host_ptr_mut", "fill", "zero",
    "copy", "write", "cstr", "wstr", "cstrn", "wstrn", "put_cstr", "put_wstr", "range", "range_of", "ga",
];

/// `x.lock_word().key()` / `x.object().key()`, or a name a `let` bound to one in the same fn
#[derive(Default)]
struct KeyEscape {
    bound: BTreeSet<String>,
    hits: Vec<(usize, String)>,
}

fn is_key_chain(e: &syn::Expr) -> bool {
    if let syn::Expr::MethodCall(m) = e {
        m.method == "key" && matches!(&*m.receiver, syn::Expr::MethodCall(r) if r.method == "lock_word" || r.method == "object")
    } else if let syn::Expr::Paren(p) = e {
        is_key_chain(&p.expr)
    } else if let syn::Expr::Try(t) = e {
        is_key_chain(&t.expr)
    } else {
        false
    }
}

/// a key in `e` (anywhere: `k + 4` too)
fn has_key(e: &syn::Expr, bound: &BTreeSet<String>) -> bool {
    struct K<'b>(&'b BTreeSet<String>, bool);
    impl<'a> syn::visit::Visit<'a> for K<'_> {
        fn visit_expr(&mut self, e: &'a syn::Expr) {
            if is_key_chain(e) {
                self.1 = true;
            } else if let syn::Expr::Path(p) = e
                && let Some(i) = p.path.get_ident()
                && self.0.contains(&i.to_string())
            {
                self.1 = true;
            } else {
                syn::visit::visit_expr(self, e);
            }
        }
        // (a closure's or a block's own value is not the argument)
        fn visit_expr_closure(&mut self, _: &'a syn::ExprClosure) {}
    }
    let mut k = K(bound, false);
    syn::visit::Visit::visit_expr(&mut k, e);
    k.1
}

impl<'a> syn::visit::Visit<'a> for KeyEscape {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !crate::syntax::is_cfg_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        let outer = std::mem::take(&mut self.bound);
        syn::visit::visit_item_fn(self, f);
        self.bound = outer;
    }
    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        let outer = std::mem::take(&mut self.bound);
        syn::visit::visit_impl_item_fn(self, f);
        self.bound = outer;
    }
    fn visit_local(&mut self, l: &'a syn::Local) {
        if let (syn::Pat::Ident(i), Some(init)) = (&l.pat, &l.init) {
            if is_key_chain(&init.expr) {
                self.bound.insert(i.ident.to_string());
            } else {
                self.bound.remove(&i.ident.to_string());
            }
        }
        syn::visit::visit_local(self, l);
    }
    fn visit_expr_call(&mut self, c: &'a syn::ExprCall) {
        if let syn::Expr::Path(p) = &*c.func {
            let segs: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
            let name = segs.last().map(String::as_str).unwrap_or_default();
            let parent = segs.iter().rev().nth(1).map(String::as_str);
            let ga_from = parent == Some("Ga") && (name == "from" || name == "new");
            let mem_fn = ADDRESSED.contains(&name) && (segs.len() == 1 || parent == Some("mem"));
            if ga_from || mem_fn {
                let n = if name == "copy" { 2 } else { 1 };
                for a in c.args.iter().take(n) {
                    if has_key(a, &self.bound) {
                        self.hits.push((crate::syntax::line(p.path.segments.first().map_or_else(proc_macro2::Span::call_site, |s| s.ident.span())), format!("a key() as the address of {}", segs.join("::"))));
                    }
                }
            }
        }
        syn::visit::visit_expr_call(self, c);
    }
}

fn out_escape(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for f in tree.rs(&["crates/sake/", "crates/kura/"]) {
        let Some(file) = tree.syntax(&f) else { continue };
        let mut v = KeyEscape::default();
        syn::visit::Visit::visit_file(&mut v, &file);
        out.extend(v.hits.into_iter().map(|(l, w)| (f.clone(), l, w)));
    }
    out
}

pub fn site(tree: &Tree, f: &Finding) -> Site {
    // (a comment is known by its own text: the code line is blank there)
    if f.rule == Rule::EscapeComment {
        return (f.rule.id().to_owned(), f.file.clone(), f.what.split_whitespace().collect::<Vec<_>>().join(" "));
    }
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

/// Arc::try_unwrap and Arc::strong_count / weak_count in code (outside #[cfg(test)]
/// modules and fns, and the files only those bring in)
fn arc_use(tree: &Tree, names: &'static [&'static str]) -> Hits {
    let mut out = Hits::new();
    let files = tree.rs(&["crates/"]);
    let tests = test_mod_files(tree, &files);
    for f in files.iter().filter(|f| !tests.contains(*f) && !f.as_str().ends_with("_tests.rs") && !f.as_str().contains("/tests/")) {
        let Some(file) = tree.syntax(f) else { continue };
        let mut v = ArcUse { names, hits: Vec::new() };
        syn::visit::Visit::visit_file(&mut v, &file);
        out.extend(v.hits.into_iter().map(|(l, what)| (f.clone(), l, what)));
    }
    out
}

struct ArcUse {
    /// the Arc associated functions looked for
    names: &'static [&'static str],
    hits: Vec<(usize, String)>,
}

impl<'a> syn::visit::Visit<'a> for ArcUse {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !crate::syntax::is_cfg_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if !crate::syntax::is_cfg_test(&f.attrs) {
            syn::visit::visit_item_fn(self, f);
        }
    }
    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        if !crate::syntax::is_cfg_test(&f.attrs) {
            syn::visit::visit_impl_item_fn(self, f);
        }
    }
    fn visit_expr_path(&mut self, e: &'a syn::ExprPath) {
        let mut back = e.path.segments.iter().rev();
        if let (Some(f), Some(ty)) = (back.next(), back.next())
            && ty.ident == "Arc"
            && self.names.iter().any(|n| f.ident == n)
        {
            self.hits.push((crate::syntax::line(f.ident.span()), format!("Arc::{}", f.ident)));
        }
        syn::visit::visit_expr_path(self, e);
    }
}

#[cfg(test)]
#[path = "arch_tests.rs"]
mod tests;
