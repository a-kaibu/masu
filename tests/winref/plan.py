#!/usr/bin/env python3
"""A run's plan: which probes a runner meant to run, kept next to its results.

    plan.py write <out dir> <runner id> <description> < names   (one probe per line)

<out dir>/.plan/<runner id> holds "# <description>" and the names. compare.py checks that
every planned probe left a result and that the run planned anything at all, so a missing
artifact, an exe that was never built or a shard that never ran fails instead of passing
with nothing compared. Each runner (run.sh, run-browser.mjs, every shard) has its own id, so
shards merged into one directory keep their plans apart.
"""
import os
import re
import sys

PLAN = ".plan"
ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]*")


def write(out, runner, description, names):
    if not ID.fullmatch(runner):
        raise ValueError(f"runner id {runner!r}: letters, digits, '_', '.', '-'")
    d = os.path.join(out, PLAN)
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, runner), "w", encoding="utf-8", newline="\n") as f:
        f.write(f"# {description}\n")
        f.writelines(n + "\n" for n in names)


def read(out):
    """-> {runner id: set of planned probes}; {} when the directory has no plan."""
    d = os.path.join(out, PLAN)
    if not os.path.isdir(d):
        return {}
    plans = {}
    for runner in sorted(os.listdir(d)):
        lines = open(os.path.join(d, runner), encoding="utf-8").read().splitlines()
        if not lines or not lines[0].startswith("# "):
            raise ValueError(f"{d}/{runner}: a plan starts with '# <runner, filter, shard>'")
        plans[runner] = {l for l in lines[1:] if l}
    return plans


def probes(root):
    """The probe directories of a result or reference directory (not the plan)."""
    return [p for p in os.listdir(root) if not p.startswith(".") and os.path.isdir(os.path.join(root, p))]


def problems(plans, required, out):
    """What makes the run's plan unusable: no plan, a required runner's plan missing, nothing
    planned, or a planned probe no result answers (one without a Windows answer included)."""
    if not plans:
        return [f"{out}: no run plan ({PLAN}/): nothing says which probes were meant to run"]
    errors = [f"plan {r!r} missing: that runner (or shard) did not run or its results were lost"
              for r in required if r not in plans]
    planned = set().union(*plans.values())
    if not planned:
        errors.append("the plans list no probe: the run compared nothing")
    have = set(probes(out))
    errors += [f"{p}: planned but not run (no result directory)" for p in sorted(planned - have)]
    return errors


def main():
    if len(sys.argv) != 5 or sys.argv[1] != "write":
        sys.exit(__doc__)
    write(sys.argv[2], sys.argv[3], sys.argv[4], [l.strip() for l in sys.stdin if l.strip()])


if __name__ == "__main__":
    main()
