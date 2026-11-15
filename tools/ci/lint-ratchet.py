#!/usr/bin/env python3
"""The type-safety lint ratchet: clippy's lints for what the compiler could have caught (an `as`
that truncates or changes sign, a `_` arm that swallows a new enum variant, an unwrap), counted per
file against tools/ci/lint-ratchet.tsv. A file may only go down: a push that adds one fails CI
(write it with From/TryFrom, an exhaustive match, `?`/a typed error instead).

  tools/ci/lint-ratchet.py            check (CI)
  tools/ci/lint-ratchet.py --update   write the counts down after fixing some (never up: a file
                                      above its line still fails)
  tools/ci/lint-ratchet.py --update --raise FILE...
                                      those files' lines to what they have now: only for code
                                      moved in from another file (say so in the commit)

Per file, not per crate: threads that each lower a crate's total on their own merge their tsv
lines without a conflict and without one's total hiding the other's additions (two sake totals
merged by rebase let 2 new unwraps through). `allow`/`expect` attributes naming a tracked lint
count as one of their own ("allow-attr"), so silencing is not a way past it. Code under
#[cfg(test)] may unwrap (clippy.toml), and test files (tests/, *_tests.rs, tests.rs) are not
counted: the tests that pin behaviour before a rewrite may cast freely.

ROBUST: the lints for code that can panic, overflow, misread memory or drop an error (indexing,
arithmetic that can overflow, undocumented unsafe, `let _ =` on a Result, ...). They are counted
in a second clippy run over the libraries and binaries only (no --all-targets): tests may index,
add and panic as they like. GROUPS: clippy's correctness, suspicious and perf groups, every lint
in them (mostly at 0; they stay there). rustc's unsafe_op_in_unsafe_fn (E0133 as a warning) is
counted as "unsafe_op_in_unsafe_fn".

CUSTOM ("masu-*"): what clippy has no lint of its own for. The unsafe functions are clippy's
disallowed_methods (clippy.toml), resolved by type, not by name (review 2026-11-15: Vec::set_len was
"a `.set_len(` with `unsafe` in the 5 lines before"): Vec::set_len, slice::from_raw_parts(_mut),
mem::transmute(_copy), mem::zeroed / MaybeUninit::assume_init, each counted under its masu-* name
(DISALLOWED). What is syntax, not a call, is found in each file's code with its comments and
string literals taken out, across lines: `static mut`, `unsafe impl ... Send/Sync for`; and the
store's tree format ("vnvm-tree", a string, so searched in the raw text) spelled outside
crates/kura/src/tree_list.rs (each new reader of a wire format is a second parser to keep in step).
"""
import collections, json, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TSV = os.path.join(ROOT, "tools", "ci", "lint-ratchet.tsv")
LINTS = [
    "as_conversions",  # every `as`: From/TryFrom/a named helper says what may happen
    "cast_possible_truncation",  # the subset of those that drop bits
    "cast_sign_loss",
    "cast_possible_wrap",
    "wildcard_enum_match_arm",  # `_ =>` on an enum: a new variant compiles silently
    "match_wildcard_for_single_variants",
    "unwrap_used",
    "expect_used",
    "fn_to_numeric_cast_any",
    "ptr_as_ptr",
]
TEST_FILE = re.compile(r"(^|/)(tests|benches)/|(^|/|_)tests?\.rs$")
ROBUST = [
    "indexing_slicing",  # a[i] / a[x..y] that panics: get()/split_at_checked/an iterator
    "arithmetic_side_effects",  # + - * / % << that can overflow or divide by zero: checked_/wrapping_/saturating_
    "string_slice",  # &s[a..b] on a str panics off a char boundary
    "get_unwrap",
    "panic",
    "todo",
    "unimplemented",
    "unreachable",
    "panic_in_result_fn",
    "unwrap_in_result",
    "exit",
    "dbg_macro",
    "undocumented_unsafe_blocks",  # every unsafe block says why it is sound (// SAFETY:)
    "multiple_unsafe_ops_per_block",
    "missing_safety_doc",
    "cast_precision_loss",
    "cast_ptr_alignment",
    "ptr_cast_constness",
    "borrow_as_ptr",
    "as_underscore",
    "char_lit_as_u8",
    "transmute_ptr_to_ptr",
    "transmute_undefined_repr",
    "cast_slice_different_sizes",
    "cast_enum_truncation",
    "fn_to_numeric_cast_with_truncation",
    "invalid_upcast_comparisons",
    "checked_conversions",
    "let_underscore_must_use",  # `let _ = r;` drops an error unread
    "unused_result_ok",
    "map_err_ignore",
    "float_cmp",
    "lossy_float_literal",
    "iter_over_hash_type",  # HashMap order differs run to run: replays and goldens must not see it
    "unchecked_time_subtraction",
    "mem_forget",
    "large_stack_arrays",
    "maybe_infinite_iter",
    "mixed_read_write_in_expression",
    "debug_assert_with_mut_call",
    "fallible_impl_from",
    "empty_drop",
    "same_name_method",
    "rest_pat_in_fully_bound_structs",
    "wildcard_in_or_patterns",
    "suspicious_xor_used_as_pow",
    "unnecessary_safety_comment",
    "try_err",
    "mut_mut",
]
GROUPS = ["correctness", "suspicious", "perf"]
RUSTC = {"E0133": "unsafe_op_in_unsafe_fn"}
TRACKED = re.compile(r"\b(?:clippy::(?:" + "|".join(LINTS + ROBUST + GROUPS) + ")|" + "|".join(RUSTC.values()) + r")\b")
# (code with comments and strings taken out: across lines, `[^{]*` stops at the impl's body)
CUSTOM = {
    "masu-static_mut": re.compile(r"\bstatic\s+mut\b"),
    "masu-unsafe_send_sync": re.compile(r"\bunsafe\s+impl\b[^{;]*?\b(?:Send|Sync)\s+for\b"),
}
# (searched in the code with its strings: it is one)
RAW = {"masu-wire_tree": re.compile(r"vnvm-tree")}
CUSTOM_HOME = {"masu-wire_tree": "crates/kura/src/tree_list.rs"}
# clippy's disallowed_methods (clippy.toml lists the same paths), by the path its message names
DISALLOWED = {
    "alloc::vec::Vec::set_len": "masu-set_len",
    "core::slice::from_raw_parts": "masu-from_raw_parts",
    "core::slice::from_raw_parts_mut": "masu-from_raw_parts",
    "core::intrinsics::transmute": "masu-transmute",
    "core::mem::transmute_copy": "masu-transmute",
    "core::mem::zeroed": "masu-uninit",
    "core::mem::MaybeUninit::assume_init": "masu-uninit",
}
DISALLOWED_PATH = re.compile(r"`([^`]+)`")
ALLOW = re.compile(r"#!?\[\s*(?:allow|expect)\s*\(([^\]]*)\)\s*\]")


def clippy():
    yield from run_clippy(["--all-targets"], LINTS + ["clippy::" + g for g in GROUPS])
    yield from run_clippy(["--lib", "--bins"], ROBUST + ["unsafe_op_in_unsafe_fn", "disallowed_methods"])


seen = set()


def run_clippy(targets, lints):
    args = ["cargo", "clippy", "--workspace", *targets, "--keep-going", "--message-format=json", "--", "--cap-lints", "warn", "-A", "clippy::all"]
    for l in lints:
        args += ["-W", l if "::" in l or l in RUSTC.values() else "clippy::" + l]
    # (the warnings are what it counts: CI's CARGO_BUILD_WARNINGS=deny would fail the run)
    env = dict(os.environ, CARGO_BUILD_WARNINGS="warn")
    p = subprocess.run(args, cwd=ROOT, stdout=subprocess.PIPE, text=True, env=env)
    for line in p.stdout.splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("reason") != "compiler-message":
            continue
        msg = m["message"]
        if msg["level"] == "error":
            sys.stderr.write(msg.get("rendered") or msg["message"])
        code = (msg.get("code") or {}).get("code") or ""
        if msg["level"] == "warning" and code in RUSTC:
            code = "clippy::" + RUSTC[code]
        if not code.startswith("clippy::"):
            continue
        if code == "clippy::disallowed_methods":
            m = DISALLOWED_PATH.search(msg["message"])
            name = DISALLOWED.get(m.group(1) if m else "")
            if not name:
                sys.exit(f"lint-ratchet: a disallowed method this script does not name: {msg['message']}")
            code = "clippy::" + name
        sp = [s for s in msg["spans"] if s["is_primary"]]
        if not sp:
            continue
        # (a lib's code is checked twice with --all-targets: as the lib and as its tests)
        key = (sp[0]["file_name"], sp[0]["line_start"], sp[0]["column_start"], code[8:])
        if key not in seen:
            seen.add(key)
            yield key
    if p.returncode != 0:
        sys.exit("lint-ratchet: clippy failed (above)")


def allow_attrs():
    for top in ("crates",):
        for d, _, fs in os.walk(os.path.join(ROOT, top)):
            for f in fs:
                if not f.endswith(".rs"):
                    continue
                path = os.path.join(d, f)
                rel = os.path.relpath(path, ROOT)
                for i, line in enumerate(open(path, encoding="utf-8", errors="replace"), 1):
                    for m in ALLOW.finditer(line):
                        if TRACKED.search(m.group(1)):
                            yield (rel, i, m.start() + 1, "allow-attr")


def code_only(text, strings=False):
    """`text` with its comments and string/char literals blanked (newlines kept, so that line
    numbers stay; `strings`: the string literals kept): a small Rust lexer, enough for raw
    strings, nested block comments, lifetimes."""
    out, i, n = [], 0, len(text)
    def blank(j):
        out.append("".join(c if c == "\n" else " " for c in text[i:j]))
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(j)
            i = j
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(j)
            i = j
        elif (m := re.match(r'b?r(#*)"', text[i:])) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            end = '"' + m.group(1)
            j = text.find(end, i + m.end())
            j = n if j < 0 else j + len(end)
            out.append(text[i:j]) if strings else blank(j)
            i = j
        elif c == '"' or (c == "b" and text.startswith('b"', i) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_"))):
            j = i + (2 if c == "b" else 1)
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            out.append(text[i:min(j + 1, n)]) if strings else blank(min(j + 1, n))
            i = min(j + 1, n)
        elif c == "'" and (m := re.match(r"'(?:\\(?:[^u]|u\{[0-9a-fA-F]+\})[^']*|[^'\\\n])'", text[i:])):
            blank(i + m.end())
            i += m.end()
        else:
            out.append(c)
            i += 1
    return "".join(out)


def custom():
    for d, _, fs in os.walk(os.path.join(ROOT, "crates")):
        for f in fs:
            if not f.endswith(".rs"):
                continue
            path = os.path.join(d, f)
            rel = os.path.relpath(path, ROOT)
            text = open(path, encoding="utf-8", errors="replace").read()
            for src, rules in ((code_only(text), CUSTOM), (code_only(text, strings=True), RAW)):
                for name, rx in rules.items():
                    if CUSTOM_HOME.get(name) == rel:
                        continue
                    for m in rx.finditer(src):
                        yield (rel, src.count("\n", 0, m.start()) + 1, m.start() + 1, name)


def count():
    per_file = collections.Counter()
    for f, _, _, lint in list(clippy()) + list(allow_attrs()) + list(custom()):
        if not TEST_FILE.search(f):
            per_file[(f, lint)] += 1
    return per_file


def load():
    files = {}
    if os.path.exists(TSV):
        for line in open(TSV):
            if line.startswith("#") or not line.strip():
                continue
            kind, name, lint, n = line.rstrip("\n").split("\t")
            # (crate lines: what the ratchet enforced before 2026-11-14 18:40Z; ignored)
            if kind == "file":
                files[(name, lint)] = int(n)
    return files


def save(files):
    with open(TSV, "w") as o:
        o.write("# tools/ci/lint-ratchet.py: type-safety, robustness and masu-* lints per file (enforced: never up)\n")
        for (f, l), n in sorted(files.items()):
            if n:
                o.write(f"file\t{f}\t{l}\t{n}\n")


def main():
    update = "--update" in sys.argv
    raise_files = set(sys.argv[sys.argv.index("--raise") + 1 :]) if "--raise" in sys.argv else set()
    files = count()
    old = load()
    up = sorted(k for k, n in files.items() if n > old.get(k, 0) and k[0] not in raise_files)
    down = sorted(k for k, n in old.items() if files.get(k, 0) < n)
    for f, l in up:
        print(f"lint-ratchet: {f}: clippy::{l} {old.get((f, l), 0)} -> {files[(f, l)]}")
    if update:
        if up:
            sys.exit("lint-ratchet: counts above went up: fix them (the baseline only goes down)")
        new = {k: (n if k[0] in raise_files else min(n, old.get(k, n))) for k, n in files.items()}
        save(new)
        print(f"lint-ratchet: wrote {os.path.relpath(TSV, ROOT)} ({sum(new.values())} in all, {sum(old.values()) - sum(new.values())} fewer)")
        return
    if down:
        print(f"lint-ratchet: {sum(old[k] - files.get(k, 0) for k in down)} fewer than the baseline in {len(set(f for f, _ in down))} files (tools/ci/lint-ratchet.py --update writes it down)")
    if up:
        sys.exit("lint-ratchet: lints went up (above): From/TryFrom instead of `as`, every variant named instead of `_`, `?` or a typed error instead of unwrap/panic, get()/checked_*/wrapping_* instead of a[i] and bare arithmetic, // SAFETY: on unsafe")
    print(f"lint-ratchet: ok ({sum(files.values())} left)")


main()
