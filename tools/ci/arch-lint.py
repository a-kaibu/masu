#!/usr/bin/env python3
"""The architecture lint: the rules the 2026-11-14..17 rewrites settled (types instead of raw
words, one home per primitive, no relay through the server), each checked by text so that the
next change that quietly undoes one fails CI instead of waiting for a review to find it again.
clippy and tools/ci/lint-ratchet.py check what a lint can see in one expression (`as`, `_` arms,
unwrap, unsafe kinds); this checks what is a fact about the repository: where something may be
defined, what may not come back, what must stay in place.

  tools/ci/arch-lint.py              check (CI)
  tools/ci/arch-lint.py --self-test  every rule against its fixtures (a regression it must find,
                                     the right spelling it must let through) and against the real
                                     tree with that regression applied to a real file
  tools/ci/arch-lint.py --update     write tools/ci/arch-lint-sites.tsv down after fixing sites
                                     (it only goes down)

A site (rule, file, its line's code with spaces collapsed) listed in tools/ci/arch-lint-sites.tsv
is a known one, left as it was: each line there says why. Any other site fails. An exception is
added by hand to that file with its reason, never by --update.

Each rule's docstring says which change set it up and what to write instead.
"""
import collections, importlib.util, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SITES = os.path.join(ROOT, "tools", "ci", "arch-lint-sites.tsv")

_spec = importlib.util.spec_from_file_location("lint_ratchet", os.path.join(ROOT, "tools", "ci", "lint-ratchet.py"))
ratchet = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ratchet)
code_only = ratchet.code_only
TEST_FILE = ratchet.TEST_FILE


class Tree:
    """The repository's tracked files, or a made-up one (`files`: path -> text), with `overlay`
    (path -> text, None = deleted) on top: what the self-test mutates."""

    def __init__(self, root=None, files=None, overlay=None):
        self.root, self.base, self.overlay = root, files, dict(overlay or {})
        self._code, self._list = {}, None

    def paths(self):
        if self._list is None:
            if self.base is not None:
                ps = set(self.base)
            else:
                out = subprocess.run(["git", "ls-files", "-z"], cwd=self.root, stdout=subprocess.PIPE, check=True).stdout
                ps = {p for p in out.decode().split("\0") if p}
            ps |= {p for p, t in self.overlay.items() if t is not None}
            ps -= {p for p, t in self.overlay.items() if t is None}
            self._list = sorted(ps)
        return self._list

    def text(self, p):
        if p in self.overlay:
            return self.overlay[p] or ""
        if self.base is not None:
            return self.base.get(p, "")
        try:
            return open(os.path.join(self.root, p), encoding="utf-8", errors="replace").read()
        except FileNotFoundError:
            return ""

    def code(self, p, strings=False):
        k = (p, strings)
        if k not in self._code:
            self._code[k] = code_only(self.text(p), strings=strings)
        return self._code[k]

    def rs(self, *prefixes):
        """the Rust files under `prefixes`, test files left out"""
        return [p for p in self.paths() if p.endswith(".rs") and p.startswith(prefixes) and not TEST_FILE.search(p)]


def line_of(src, at):
    return src.count("\n", 0, at) + 1


def test_spans(src):
    """the (start, end) offsets of every `#[cfg(test)] mod x { .. }` in code `src`"""
    spans = []
    for m in re.finditer(r"#\[cfg\(test\)\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{", src):
        depth, j = 1, m.end()
        while j < len(src) and depth:
            depth += {"{": 1, "}": -1}.get(src[j], 0)
            j += 1
        spans.append((m.start(), j))
    return spans


def in_spans(spans, at):
    return any(a <= at < b for a, b in spans)


def finds(tree, paths, rx, strings=False, skip_tests=True):
    """every match of `rx` in the code of `paths` (comments out; strings out unless `strings`),
    outside #[cfg(test)] modules: (file, line, matched text)"""
    for p in paths:
        src = tree.code(p, strings)
        spans = test_spans(src) if skip_tests else []
        for m in rx.finditer(src):
            if not in_spans(spans, m.start()):
                yield p, line_of(src, m.start()), m.group(0)


RULES = []


def rule(bad, good, mutate):
    """`bad`/`good`: made-up trees (path -> text) the rule must find something in / nothing in;
    `mutate`: (real file, f(text) -> text) that brings the regression back into the real tree"""
    def wrap(f):
        f.id, f.bad, f.good, f.mutate = f.__name__.replace("_", "-"), bad, good, mutate
        RULES.append(f)
        return f
    return wrap


# --- the rules ---------------------------------------------------------------------------------

BUILD_EXT = re.compile(r"\.(?:wasm|exe|dll|so|dylib|o|a|lib|rlib|pdb|zst|tar|tgz|7z|zip)$", re.I)
# (vendored sources, test inputs, apitrace traces, sake's guest C runtime: inputs, not outputs)
BUILD_OK = re.compile(r"^vendor/|(^|/)testdata/|^crates/sake/src/guest_crt(64)?\.bin$")


@rule(bad={"crates/masu/target/x86_64-pc-windows-gnu/release/masu.exe": "x", "web/public/sake/sake.wasm": "x"},
      good={"crates/kura/testdata/idec/planes.bin": "x", "testdata/apitrace/a.trace.zst": "x", "crates/kura/src/target.rs": "x"},
      mutate=("crates/masu/target/release/masu", lambda t: "stub\n"))
def build_output(tree):
    """No build output in git (user rule: build artifacts never go to GitHub; three test stubs of
    masu binaries under crates/masu/target/ were committed with 99c0d6d6). A `target/` directory
    anywhere, or a compiled/packed file outside vendor/ and testdata/."""
    for p in tree.paths():
        if p.endswith(".rs"):
            continue
        if re.search(r"(^|/)target/", p) or (BUILD_EXT.search(p) and not BUILD_OK.search(p)):
            yield p, 1, p


HRESULT = "crates/sake/src/hresult.rs"


@rule(bad={HRESULT: "pub const E_FAIL: u32 = 0x8000_4005;\n", "crates/sake/src/vmr9.rs": "const E_FAIL: u32 = 0x8000_4005;\n"},
      good={HRESULT: "pub const E_FAIL: u32 = 0x8000_4005;\n", "crates/sake/src/vmr9.rs": "use crate::hresult::E_FAIL;\nconst VFW_E_X: u32 = 0x8004_0200;\n",
            "crates/sake/src/x_tests.rs": "const E_FAIL: u32 = 1;\n"},
      mutate=("crates/sake/src/k32.rs", lambda t: t + "\nconst E_FAIL: u32 = 0x8000_4005;\n"))
def hresult_copy(tree):
    """sake's common HRESULTs live in sake::hresult only (DRY rework 2026-11-15, masu-kome-dry):
    a second `const E_FAIL` is a second value to get wrong. `use crate::hresult::*` instead."""
    names = re.findall(r"\bpub\s+const\s+([A-Z][A-Z0-9_]*)\s*:", tree.code(HRESULT))
    if not names:
        return
    rx = re.compile(r"\bconst\s+(?:" + "|".join(names) + r")\s*:")
    yield from finds(tree, [p for p in tree.rs("crates/sake/") if p != HRESULT], rx)


KOME_FNS = "lo8|lo16|lo16s|lo32|lo64|us|u32_of|u64_of|to_usize|u16_at|u32_at|u64_at"


@rule(bad={"crates/kura/src/x.rs": "fn lo16(x: u32) -> u16 {\n    x as u16\n}\npub trait Lock<T> {}\n"},
      good={"crates/kura/src/x.rs": "use kome::num::lo16;\nimpl R {\n    fn u32_at(&self, o: usize) -> u32 { 0 }\n}\n", "crates/kome/src/num.rs": "pub fn lo16(x: u32) -> u16 { 0 }\n"},
      mutate=("crates/kura/src/audio.rs", lambda t: t + "\nfn lo32(x: u64) -> u32 {\n    x as u32\n}\n"))
def kome_copy(tree):
    """The number conversions, little-endian readers and lock helpers exist once, in crate kome
    (95e48ed5, 9d3fdbcd removed sake's copies): a per-crate copy drifts from the checked one.
    A free fn named like kome::num / kome::le's, or a `Lock`/`Rw` trait, outside crates/kome."""
    rx = re.compile(r"^(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?fn\s+(?:" + KOME_FNS + r")\b|\btrait\s+(?:Lock|Rw)\b", re.M)
    yield from finds(tree, [p for p in tree.rs("crates/") if not p.startswith("crates/kome/")], rx)


GOLDEN_ANY = re.compile(r"\b[A-Z0-9_]*(?:UPDATE_GOLDEN|BLESS|UPDATE_SNAPSHOTS?|REGEN(?:ERATE)?_GOLDEN|UPDATE_EXPECT)[A-Z0-9_]*\b")
GOLDEN_HOME = "crates/kome/src/golden.rs"


@rule(bad={"crates/sake/src/x.rs": 'if std::env::var_os("SAKE_UPDATE_GOLDEN").is_some() {}\n', "crates/kura/src/y.rs": 'std::env::var("MASU_UPDATE_GOLDEN")\n',
           "tools/x.sh": "BLESS=1 cargo test\n"},
      good={GOLDEN_HOME: 'pub const UPDATE: &str = "MASU_UPDATE_GOLDEN";\n', "crates/sake/src/x.rs": "if kome::golden::updating() {}\n// MASU_UPDATE_GOLDEN=1 rewrites it\n",
            "tests/x.sh": "MASU_UPDATE_GOLDEN=1 cargo test\n"},
      mutate=("crates/sake/src/gl.rs", lambda t: t + '\nfn regen() -> bool { std::env::var_os("SAKE_UPDATE_GOLDEN").is_some() }\n'))
def golden_env(tree):
    """One switch rewrites every golden: MASU_UPDATE_GOLDEN, read only by kome::golden (single
    golden env var, DRY rework 2026-11-15). Another *_UPDATE_GOLDEN/BLESS name is a golden that
    `MASU_UPDATE_GOLDEN=1` silently leaves stale; reading the variable yourself skips kome's rules."""
    for p in tree.paths():
        if not p.startswith(("crates/", "tests/", "tools/", ".github/", "host/", "web/src/", "formal/", "fuzz/")) or p.endswith(".tsv") or p == "tools/ci/arch-lint.py":
            continue
        if not re.search(r"\.(rs|py|sh|mjs|js|ts|yml|yaml|toml)$", p):
            continue
        src = tree.code(p, strings=True) if p.endswith(".rs") else tree.text(p)
        for m in GOLDEN_ANY.finditer(src):
            if m.group(0) != "MASU_UPDATE_GOLDEN":
                yield p, line_of(src, m.start()), m.group(0)
        if p.endswith(".rs") and p != GOLDEN_HOME:
            for m in re.finditer(r'"MASU_UPDATE_GOLDEN"', src):
                yield p, line_of(src, m.start()), "reads MASU_UPDATE_GOLDEN itself (kome::golden)"


MEM = "crates/sake/src/mem.rs"
API = "crates/sake/src/api.rs"
GONE = {
    # (review 2026-11-15/16: safe borrows of guest memory let another thread's write race the
    # borrow; mem::bytes / mem::update copy, view_unchecked says its contract)
    MEM: "slice|slice_mut|guest|guest_mut|in_guest|guest_range|view|view_mut",
    # (out-parameters 2026-11-16: an Out reaches a call word only through api::words!, an argument
    # only through the function's typed Args)
    API: "to_callee|a_at|out_at|put64",
}


@rule(bad={MEM: "pub fn view<R>(a: u32, n: u32, f: impl FnOnce(&[u8]) -> R) -> Option<R> { None }\n", API: "impl Out {\n    pub fn to_callee(self) -> u32 { 0 }\n}\n"},
      good={MEM: "pub unsafe fn view_unchecked<R>(a: u32) {}\npub fn bytes(a: u32, n: u32) -> Vec<u8> { vec![] }\n", API: "pub fn out(&self) {}\n"},
      mutate=(MEM, lambda t: t + "\npub fn guest_mut(a: u32, n: u32) -> &'static mut [u8] { todo!() }\n"))
def dead_api(tree):
    """The functions the reviews removed stay removed: sake::mem's safe zero-copy borrows (slice,
    guest_mut, view, view_mut, guest, in_guest, guest_range) and api's untyped ways around an
    out-parameter (Out::to_callee, a_at, out_at, put64)."""
    for p, names in GONE.items():
        yield from finds(tree, [p], re.compile(r"\bfn\s+(?:" + names + r")\s*[<(]"))


@rule(bad={MEM: "impl Ga {\n    pub fn r32(self) -> u32 { 0 }\n}\n", API: "impl std::ops::Deref for Args<F> {\n    type Target = Ctx;\n}\n"},
      good={MEM: "impl Ga {\n    pub fn since(self, b: Ga) -> u32 { 0 }\n}\npub fn r32(a: impl Into<Ga>) -> u32 { 0 }\n", API: "impl<F> Args<F> {\n    pub fn ctx(&self) {}\n}\n"},
      mutate=(MEM, lambda t: t + "\nimpl Ga {\n    pub fn w32(self, v: u32) {}\n}\n"))
def untyped_way_back(tree):
    """A guest address is a value: `mem::r32(ga(a) + 4)`, never `ga.r32()` (Ga value-only,
    review 2026-11-15: an accessor on Ga skipped mem::range). api::Args has no Deref to Ctx
    (2026-11-16: Deref let `c.a(i)` read an out-parameter as a plain word)."""
    for p in tree.rs("crates/sake/"):
        src = tree.code(p)
        for m in re.finditer(r"\bimpl\b[^{;]*?\bfor\s+(?:\w+::)*Ga\b[^{]*\{|\bimpl\s+(?:\w+::)*Ga\s*\{", src):
            depth, j = 1, m.end()
            while j < len(src) and depth:
                depth += {"{": 1, "}": -1}.get(src[j], 0)
                j += 1
            for f in re.finditer(r"\bfn\s+[rw](?:8|16|32|64|p|s)?\b\s*[<(]", src[m.end():j]):
                yield p, line_of(src, m.end() + f.start()), "a reader/writer method on Ga"
        for m in re.finditer(r"\bimpl\b[^{;]*?\bDeref(?:Mut)?\b[^{;]*?\bfor\s+(?:\w+::)*Args\b", src):
            yield p, line_of(src, m.start()), "Deref for Args"


HOMES = {
    # (2026-11-16: the zero-copy reads of guest memory, each with its contract: ReadFile-like host
    # file reads and decoded video frames)
    "view-unchecked": (re.compile(r"\bview(?:_mut)?_unchecked\b"), {MEM, "crates/sake/src/k32.rs", "crates/sake/src/files.rs", "crates/sake/src/dshow_mpeg.rs",
                                                                    "crates/sake/src/dshow_mpeg/dmo.rs", "crates/sake/src/mf/plat.rs", "crates/sake/src/amstream.rs"}),
    # (guest memory as atomics: the CPU's locked instructions (kura cpu, cmpxchg16 swap16),
    # sake mem's atomic_rmw32 through mem::range, crashlog's reader with its Own token, review
    # 2026-11-16: anything else reading guest memory goes through sake::mem)
    "guest-atomics": (re.compile(r"\bAtomic(?:U|I)(?:8|16|32|64)::from_ptr\b"), {"crates/kura/src/crashlog.rs", "crates/kura/src/cpu.rs", MEM}),
    # (review 2026-11-15: a poisoned lock panics; only crashlog's whole-per-change lists go on)
    "locked-anyway": (re.compile(r"\.locked_anyway\s*\("), {"crates/kura/src/crashlog.rs", "crates/kome/src/locks.rs"}),
}


@rule(bad={"crates/sake/src/gdi.rs": "let v = unsafe { mem::view_unchecked(a, n, |s| s[0]) };\n", "crates/kura/src/vfs.rs": "unsafe { AtomicU32::from_ptr(p) }.load(Relaxed);\nX.locked_anyway();\n"},
      good={"crates/sake/src/k32.rs": "unsafe { mem::view_mut_unchecked(a, n, |s| ()) };\n", "crates/kura/src/crashlog.rs": "unsafe { AtomicU8::from_ptr(p) }.load(Relaxed);\nX.locked_anyway();\n",
            "crates/kura/src/vfs.rs": "X.locked();\n"},
      mutate=("crates/sake/src/gdi/mod.rs", lambda t: t + "\nfn peek(a: u32) -> u8 { unsafe { crate::mem::view_unchecked(a, 1, |s| s[0]) }.unwrap_or(0) }\n"))
def home_only(tree):
    """The primitives that are sound only under a contract have one list of callers: view_unchecked
    (sake mem), AtomicU*::from_ptr on guest memory (kura crashlog, with its Own token),
    locked_anyway (crashlog). A new caller is a new place the contract has to hold: add it to
    HOMES in this file with the reason, in the same commit."""
    for name, (rx, homes) in HOMES.items():
        yield from finds(tree, [p for p in tree.rs("crates/") if p not in homes], rx)


@rule(bad={"crates/kura/src/x.rs": "#[repr(C)]\npub struct Ctl {\n    pub magic: u32,\n}\n"},
      good={"crates/kura/src/x.rs": "#[repr(C)]\npub struct Ctl {\n    pub magic: u32,\n}\nconst _: () = assert!(std::mem::size_of::<Ctl>() == 4);\n",
            "crates/sake/src/nt.rs": "#[cfg(test)]\nmod tests {\n    #[repr(C)]\n    struct Head { next: u32 }\n}\n"},
      mutate=("crates/kura/src/snap.rs", lambda t: t + "\n#[repr(C)]\npub struct Extra {\n    pub a: u32,\n}\n"))
def repr_c_layout(tree):
    """A `#[repr(C)]` type is a layout something else reads (the page, the JIT's wasm, a C
    side): its size is pinned at compile time, `const _: () = assert!(size_of::<T>() == N);`
    (user rule 2026-11-14: const asserts for sizes and layouts). Test oracles in #[cfg(test)]
    modules are not counted."""
    for p in tree.rs("crates/"):
        src = tree.code(p)
        spans = test_spans(src)
        for m in re.finditer(r"#\[repr\(C\b[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?(?:struct|union)\s+(\w+)", src):
            if in_spans(spans, m.start()):
                continue
            n = m.group(1)
            if not re.search(r"\bconst\s+_\s*:\s*\(\)\s*=\s*\{?[^;]*?size_of::<\s*" + n + r"\s*>\s*\(\)", src):
                yield p, line_of(src, m.start()), f"repr(C) {n} without a size assert"


RELEASE = ".github/workflows/release.yml"
RELEASE_JOBS = {"version", "web", "bin", "pages", "release"}


@rule(bad={RELEASE: "jobs:\n  version:\n    runs-on: x\n  test:\n    steps:\n      - run: cargo test --workspace\n"},
      good={RELEASE: "jobs:\n  version:\n    runs-on: x\n  release:\n    steps:\n      - run: cargo build --release\n"},
      mutate=(RELEASE, lambda t: t + "  lint:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo clippy\n"))
def release_build_only(tree):
    """release.yml builds and releases, nothing else (user rule: CI runs the tests on every push;
    a check in the release only delays it): its jobs are version, web, bin, pages, release, and
    no step runs a test suite or a linter."""
    src = tree.text(RELEASE)
    body = src.split("\njobs:\n", 1)[1] if "\njobs:\n" in src else (src.split("jobs:\n", 1)[1] if src.startswith("jobs:\n") else "")
    off = len(src) - len(body)
    for m in re.finditer(r"^  ([A-Za-z0-9_-]+):\s*$", body, re.M):
        if m.group(1) not in RELEASE_JOBS:
            yield RELEASE, line_of(src, off + m.start()), f"job {m.group(1)}"
    for m in re.finditer(r"\b(?:cargo (?:test|clippy|nextest|mutants)|pnpm (?:run )?test|vp (?:test|check)|lint-ratchet|arch-lint)\b", src):
        yield RELEASE, line_of(src, m.start()), m.group(0)


# what must stay in place: (file, text, why)
REQUIRED = [
    ("tests/host-tests.sh", "export MASU_OUTS_STRICT=1", "release-build host tests fail on a 32-bit write into a pointer-sized out-parameter (2026-11-16)"),
    ("crates/sake/src/lib.rs", "mod outptr_audit;", "every handler of a function with pointer-sized out-parameters takes its typed Args"),
    ("crates/kura/src/pool.rs", "```compile_fail", "Pool Lease/brand misuse does not compile (review 2026-11-15/16)"),
    (MEM, "```compile_fail", "Ga has no accessors; a guest borrow does not outlive its closure"),
    (API, "```compile_fail", "an out-parameter is not a word"),
    (".github/workflows/ci.yml", "python3 tools/ci/lint-ratchet.py", "the lint ratchet runs on every push"),
    (".github/workflows/ci.yml", "python3 tools/ci/arch-lint.py", "this lint runs on every push"),
    (".github/workflows/ci.yml", "python3 tools/ci/lint-ratchet.py --self-test", "each ratchet lint still fires on its fixture"),
    (".github/workflows/ci.yml", "python3 tools/ci/arch-lint.py --self-test", "each rule here still fires on its fixture"),
]


@rule(bad={"tests/host-tests.sh": "#!/bin/sh\ncargo test\n"},
      good={p: "\n".join(t for q, t, _ in REQUIRED if q == p) for p, _, _ in REQUIRED},
      mutate=("tests/host-tests.sh", lambda t: t.replace("export MASU_OUTS_STRICT=1", "")))
def required(tree):
    """The guards themselves stay: the strict out-parameter run, the audits, the compile_fail
    doctests that prove a misuse does not compile, the CI steps that run these lints. Deleting one
    turns a checked rule back into a hope. In a made-up tree only the files it has are checked."""
    for p, t, why in REQUIRED:
        if tree.base is not None and p not in tree.base:
            continue
        if t not in tree.text(p):
            yield p, 1, f"missing `{t}`: {why}"


@rule(bad={"clippy.toml": 'disallowed-methods = [\n    { path = "core::mem::zeroed", reason = "masu-uninit" },\n]\n'},
      good={"clippy.toml": "disallowed-methods = [\n" + "".join(f'    {{ path = "{p}", reason = "x" }},\n' for p in ratchet.DISALLOWED) + "]\n"},
      mutate=("clippy.toml", lambda t: t.replace('"core::mem::zeroed"', '"core::mem::zeroed_gone"')))
def disallowed_in_step(tree):
    """clippy.toml's disallowed-methods and lint-ratchet.py's DISALLOWED name the same paths: one
    dropped from clippy.toml is a masu-* lint that silently stops counting."""
    src = tree.text("clippy.toml")
    have = set(re.findall(r'path\s*=\s*"([^"]+)"', src))
    for p in ratchet.DISALLOWED:
        if p not in have:
            yield "clippy.toml", 1, f"missing disallowed path {p}"
    for p in sorted(have - set(ratchet.DISALLOWED)):
        yield "clippy.toml", 1, f"{p} is not in lint-ratchet.py DISALLOWED"


# --- the run -----------------------------------------------------------------------------------


def norm(tree, f, n):
    src = tree.code(f, strings=True) if f.endswith(".rs") else tree.text(f)
    v = src.split("\n")
    return " ".join(v[n - 1].split()) if 0 < n <= len(v) else ""


def check(tree, rules=None):
    """{(rule, file, line text): [line numbers]}"""
    out = collections.defaultdict(list)
    for r in rules or RULES:
        for f, n, what in r(tree):
            k = (r.id, f, norm(tree, f, n) if n > 1 or f.endswith(".rs") else what)
            out[k].append((n, what))
    return out


def load_sites():
    known = collections.Counter()
    if os.path.exists(SITES):
        for line in open(SITES, encoding="utf-8"):
            if line.startswith("#") or not line.strip():
                continue
            r, f, t = line.rstrip("\n").split("\t")[:3]
            known[(r, f, t)] += 1
    return known


def fresh(found, known):
    return [(k, v) for k, v in sorted(found.items()) if len(v) > known.get(k, 0)]


def self_test():
    bad = 0
    for r in RULES:
        if not list(r(Tree(files=r.bad))):
            print(f"arch-lint self-test: {r.id}: finds nothing in its regression fixture")
            bad += 1
        if hits := list(r(Tree(files=r.good))):
            print(f"arch-lint self-test: {r.id}: flags its good fixture: {hits}")
            bad += 1
    real = Tree(root=ROOT)
    known = load_sites()
    clean = fresh(check(real), known)
    if clean:
        print(f"arch-lint self-test: the tree itself fails ({len(clean)} sites): run the check first")
        bad += 1
    for r in RULES:
        f, mut = r.mutate
        tree = Tree(root=ROOT, overlay={f: mut(real.text(f))})
        if not fresh(check(tree, [r]), known):
            print(f"arch-lint self-test: {r.id}: misses its regression applied to {f}")
            bad += 1
    if bad:
        sys.exit(f"arch-lint self-test: {bad} failures")
    print(f"arch-lint self-test: ok ({len(RULES)} rules: each finds its regression in a fixture and in the real tree, and passes its good fixture)")


def main():
    if "--self-test" in sys.argv:
        return self_test()
    tree = Tree(root=ROOT)
    found = check(tree)
    known = load_sites()
    if "--update" in sys.argv:
        keep = []
        for line in open(SITES, encoding="utf-8") if os.path.exists(SITES) else []:
            if line.startswith("#") or not line.strip():
                keep.append(line)
                continue
            k = tuple(line.rstrip("\n").split("\t")[:3])
            if len(found.get(k, ())) > sum(1 for l in keep if tuple(l.rstrip("\n").split("\t")[:3]) == k):
                keep.append(line)
        open(SITES, "w", encoding="utf-8").writelines(keep)
        print(f"arch-lint: wrote {os.path.relpath(SITES, ROOT)} ({sum(1 for l in keep if not l.startswith('#') and l.strip())} known sites)")
        return
    new = fresh(found, known)
    for (r, f, t), v in new:
        for n, what in v[known.get((r, f, t), 0):]:
            doc = next(x for x in RULES if x.id == r).__doc__.split("\n")[0]
            print(f"arch-lint: {f}:{n}: {r}: {what}\n    ({doc} ... tools/ci/arch-lint.py)")
    gone = sum(n - len(found.get(k, ())) for k, n in known.items() if len(found.get(k, ())) < n)
    if gone:
        print(f"arch-lint: {gone} known sites are gone (tools/ci/arch-lint.py --update writes them down)")
    if new:
        sys.exit(f"arch-lint: {sum(len(v) for _, v in new)} sites break the rules above (each rule's docstring in tools/ci/arch-lint.py says what to write instead)")
    print(f"arch-lint: ok ({len(RULES)} rules, {sum(known.values())} known sites)")


if __name__ == "__main__":
    main()
