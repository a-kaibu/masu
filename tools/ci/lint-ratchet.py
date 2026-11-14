#!/usr/bin/env python3
"""The type-safety lint ratchet: clippy's lints for what the compiler could have caught (an `as`
that truncates or changes sign, a `_` arm that swallows a new enum variant, an unwrap), counted per
crate against tools/ci/lint-ratchet.tsv. A crate may only go down: a push that adds one fails CI
(write it with From/TryFrom, an exhaustive match, `?`/a typed error instead).

  tools/ci/lint-ratchet.py            check (CI)
  tools/ci/lint-ratchet.py --update   write the counts down after fixing some (never up: a crate
                                      above its line still fails; --raise only for code moved in
                                      from elsewhere, say why in the commit)

The tsv also has the count per file, which is what a failure prints the growth of. `allow`/`expect`
attributes naming a tracked lint count as one of their own ("allow-attr"), so silencing is not a
way past it. Code under #[cfg(test)] may unwrap (clippy.toml), and test files (tests/, *_tests.rs,
tests.rs) are not counted: the tests that pin behaviour before a rewrite may cast freely.
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


def crate_of(f):
    p = f.split("/")
    return p[1] if p[0] == "crates" and len(p) > 1 else p[0]


def count():
    per_file = collections.Counter()
    for f, _, _, lint in list(clippy()) + list(allow_attrs()):
        if not TEST_FILE.search(f):
            per_file[(f, lint)] += 1
    per_crate = collections.Counter()
    for (f, lint), n in per_file.items():
        per_crate[(crate_of(f), lint)] += n
    return per_crate, per_file


def load():
    crates, files = {}, {}
    if os.path.exists(TSV):
        for line in open(TSV):
            if line.startswith("#") or not line.strip():
                continue
            kind, name, lint, n = line.rstrip("\n").split("\t")
            (crates if kind == "crate" else files)[(name, lint)] = int(n)
    return crates, files


def save(crates, files):
    with open(TSV, "w") as o:
        o.write("# tools/ci/lint-ratchet.py: type-safety lints per crate (enforced: never up) and per file (what a failure shows)\n")
        for (c, l), n in sorted(crates.items()):
            o.write(f"crate\t{c}\t{l}\t{n}\n")
        for (f, l), n in sorted(files.items()):
            if n:
                o.write(f"file\t{f}\t{l}\t{n}\n")


def main():
    update, allow_raise = "--update" in sys.argv, "--raise" in sys.argv
    crates, files = count()
    old_crates, old_files = load()
    up = sorted(k for k, n in crates.items() if n > old_crates.get(k, 0))
    down = sorted(k for k, n in old_crates.items() if crates.get(k, 0) < n)
    for c, l in up:
        print(f"lint-ratchet: {c}: clippy::{l} {old_crates.get((c, l), 0)} -> {crates[(c, l)]}")
        grew = sorted((f, n - old_files.get((f, l2), 0)) for (f, l2), n in files.items() if l2 == l and crate_of(f) == c and n > old_files.get((f, l2), 0))
        for f, d in grew:
            print(f"    {f}: +{d}")
    if update:
        if up and not allow_raise:
            sys.exit("lint-ratchet: counts above went up: fix them (the baseline only goes down)")
        new = dict(old_crates) if not allow_raise else {}
        for k in set(crates) | set(old_crates):
            n = crates.get(k, 0)
            new[k] = n if allow_raise else min(n, old_crates.get(k, n))
        save({k: n for k, n in new.items() if n}, files)
        print(f"lint-ratchet: wrote {os.path.relpath(TSV, ROOT)} ({sum(crates.values())} in all)")
        return
    if down:
        print("lint-ratchet: fewer than the baseline (tools/ci/lint-ratchet.py --update writes it down):")
        for c, l in down:
            print(f"    {c}: clippy::{l} {old_crates[(c, l)]} -> {crates.get((c, l), 0)}")
    if up:
        sys.exit("lint-ratchet: type-safety lints went up (above): From/TryFrom instead of `as`, every variant named instead of `_`, `?` or a typed error instead of unwrap")
    print(f"lint-ratchet: ok ({sum(crates.values())} left)")


main()
