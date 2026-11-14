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
ALLOW = re.compile(r"#!?\[\s*(?:allow|expect)\s*\(([^\]]*)\)\s*\]")


def clippy():
    args = ["cargo", "clippy", "--workspace", "--all-targets", "--keep-going", "--message-format=json", "--", "--cap-lints", "warn", "-A", "clippy::all"]
    for l in LINTS:
        args += ["-W", "clippy::" + l]
    p = subprocess.run(args, cwd=ROOT, stdout=subprocess.PIPE, text=True)
    seen = set()
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
        if not code.startswith("clippy::"):
            continue
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
                        if any("clippy::" + l in m.group(1) for l in LINTS):
                            yield (rel, i, m.start() + 1, "allow-attr")


def count():
    per_file = collections.Counter()
    for f, _, _, lint in list(clippy()) + list(allow_attrs()):
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
        o.write("# tools/ci/lint-ratchet.py: type-safety lints per file (enforced: never up)\n")
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
        sys.exit("lint-ratchet: type-safety lints went up (above): From/TryFrom instead of `as`, every variant named instead of `_`, `?` or a typed error instead of unwrap")
    print(f"lint-ratchet: ok ({sum(files.values())} left)")


main()
