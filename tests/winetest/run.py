#!/usr/bin/env python3
"""Runs Wine's conformance tests (built by tests/winetest/build.sh) on sake's node build and
compares the results with tests/winetest/expected-<arch>.tsv. --suite rostests does the same
for ReactOS's API tests (tests/rostests/build.sh, tests/rostests/expected-<arch>.tsv).

ReactOS's tests were written against ReactOS as much as Windows, so a rostests run on sake
takes only the tests Windows passes: tests/rostests/windows-<arch>.tsv, written by --native on
a Windows machine (the test programs run there directly, no sake). A test Windows fails is not
run on sake; a test its program lists that Windows' results lack fails the run (not measured).
Each rostests test runs in its own copy of the bin directory (hard links), as ReactOS runs them
from theirs (cwd there): the files they read and the ones they write stay theirs.

Every test program is asked for its tests (--list) and each test runs as its own process, in a
fresh C:\\ (SAKE_C), with a time limit. A test's result is one of
  ok        it finished with no failures
  fail      it finished with N failures (N is kept)
  crash     it ended without its summary line (unhandled exception, abort, sake exit)
  timeout   it ran out of time
The tests expect Windows' behaviour: sake is not Wine (no wine_server_call), so todo_wine
marks do not apply.

Functions sake knows but lacks are found by GetProcAddress (SAKE_PROC_STUBS: their stubs return
0), as on Windows, so a test that does not check for NULL fails instead of crashing. The tests
start their child processes (copies of themselves) as on Windows, each its own sake beside
the test's, on the same C: drive; a test's whole process group ends with it (--no-exec: no
children, SAKE_NO_EXEC, and the tests that need one fail).

A run fails (exit 1) on a regression against the expected file: a test that got worse
(ok -> fail -> crash/timeout) or one with clearly more failures than recorded (over 10%, 5 at
least). Better results are
listed; --update writes the new results as the expected ones.

  tests/winetest/run.py [--suite rostests] [--arch i386] [--update] [-j N] [--shard i/n] [--native] [dll[:test] ...]
  SAKE=target/sake-out/sake.js WINETEST=target/winetest ROSTESTS=target/rostests

Also written: $WINETEST/report-<arch>.md (per-dll totals, the most-called unimplemented
functions, the first failure messages of each test) and results-<arch>.tsv.
"""
import argparse
import collections
import concurrent.futures
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
# per suite: its directory (dlls.txt, expected-<arch>.tsv), its test program's name, the
# variable naming its build directory, what its result files say they hold, and whether its
# tests take only what Windows passes (windows-<arch>.tsv) and run in a copy of their bin dir
SUITES = {
    "winetest": dict(dir=HERE, exe="{}_test.exe", env="WINETEST", title="Wine 11.20 tests", windows=False),
    "rostests": dict(dir=os.path.join(ROOT, "tests", "rostests"), exe="{}_apitest.exe", env="ROSTESTS",
                     title="ReactOS 303d27d6 API tests", windows=True),
}
RANK = {"ok": 0, "fail": 1, "crash": 2, "timeout": 2}
SUMMARY = re.compile(r"^(?:([0-9a-f]{4,}):)?([^\s:]+): (\d+) tests executed \((\d+) marked as todo, (?:(\d+) as flaky, )?(\d+) failures?\), (\d+) skipped\.")
FAILED = re.compile(r"^(?:[0-9a-f]{4}:)?\S+\.(?:c|cpp):\d+:(?:\d+\.\d+ )?(?: Test failed:|Test failed:| unhandled exception)")
CRASH = re.compile(r"panicked|RuntimeError|Aborted|unhandled|sake: .*(exception|fault|unknown|not supported)")
PID = re.compile(r"^([0-9a-f]{4,}):")
UNIMPL = re.compile(r"^sake: unimplemented (\S+?) ")


def command(sake, exe, *args):
    """the command running a test program: on sake (node) or, with no sake (--native), itself"""
    return ([exe] if sake is None else ["node", sake, exe]) + list(args)


def list_tests(sake, exe, env):
    p = subprocess.run(command(sake, exe, "--list"), capture_output=True, text=True, errors="replace", env=env, timeout=120)
    out = p.stdout + p.stderr
    names = [l.strip() for l in out.split("Valid test names:", 1)[-1].splitlines() if l.startswith("    ")]
    return names


HOSTS = "".join(f"127.0.0.1 {n}\n" for n in (
    "winehq.org", "test.winehq.org", "source.winehq.org", "www.kernel.org", "xn--zckzah.winehq.org"))


def link_tree(src, dst):
    """dst: a copy of src's files as hard links (copies where the file system has none)"""
    for d, _, files in os.walk(src):
        o = os.path.join(dst, os.path.relpath(d, src))
        os.makedirs(o, exist_ok=True)
        for f in files:
            try:
                os.link(os.path.join(d, f), os.path.join(o, f))
            except OSError:
                shutil.copy2(os.path.join(d, f), os.path.join(o, f))


def run_one(sake, exe, test, timeout, fonts, no_exec=False, own_bin=None):
    """own_bin: the directory to run the test from, a fresh copy of exe's (rostests)"""
    c = tempfile.mkdtemp(prefix="winetest-c-")
    cwd = c
    if own_bin is not None:
        cwd = tempfile.mkdtemp(prefix="bin-", dir=own_bin)
        link_tree(os.path.dirname(exe), cwd)
        exe = os.path.join(cwd, os.path.basename(exe))
    for d in ("windows/temp", "windows/system32", "users/user/desktop"):
        os.makedirs(os.path.join(c, d), exist_ok=True)
    if fonts:
        os.symlink(fonts, os.path.join(c, "windows/Fonts"))
    # sake has no DNS: the names the ws2_32 / iphlpapi tests look up resolve through the hosts
    # file, to loopback (nonexistent / nxdomain names stay unresolved)
    os.makedirs(os.path.join(c, "windows/system32/drivers/etc"), exist_ok=True)
    with open(os.path.join(c, "windows/system32/drivers/etc/hosts"), "w") as f:
        f.write(HOSTS)
    env = dict(os.environ, SAKE_C=c, SAKE_PROC_STUBS="1")
    if no_exec:
        env["SAKE_NO_EXEC"] = "1"
    t0 = time.time()
    # (its own process group: the children it started end with it, and none outlives the run
    # holding the output pipe open)
    p = subprocess.Popen(command(sake, exe, test), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env, cwd=cwd,
                         **({} if os.name == "nt" else dict(start_new_session=True)))
    try:
        raw, _ = p.communicate(timeout=timeout)
        timed_out = False
    except subprocess.TimeoutExpired:
        killpg(p.pid)
        raw, _ = p.communicate()
        timed_out = True
    finally:
        killpg(p.pid)
        shutil.rmtree(c, ignore_errors=True)
        if cwd != c:
            shutil.rmtree(cwd, ignore_errors=True)
    out = (raw or b"").decode(errors="replace")
    return parse(test, out, timed_out, time.time() - t0)


def killpg(pid):
    """ends the process and every process it started"""
    if os.name == "nt":
        # (Windows: the tree by parent ids; a finished one is already gone, taskkill says so)
        subprocess.run(["taskkill", "/F", "/T", "/PID", str(pid)], capture_output=True, check=False)
        return
    try:
        os.killpg(pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass


def parse(test, out, timed_out, secs):
    lines = out.splitlines()
    executed = failures = 0
    summary = False
    # (the test's own process has the lowest id, its children's come after it: a child's
    # summary alone does not finish the test)
    first = min((m.group(1) for m in map(PID.match, lines) if m), key=lambda p: int(p, 16), default=None)
    for l in lines:
        m = SUMMARY.match(l)
        if m:
            summary |= m.group(1) == first
            executed += int(m.group(3))
            failures += int(m.group(6))
    failed = [l for l in lines if FAILED.match(l)]
    if not summary:
        failures = len(failed)  # (the ones printed before it ended)
    status = "timeout" if timed_out else ("crash" if not summary else ("fail" if failures else "ok"))
    msgs = failed[:5]
    if status == "crash":
        msgs += [l for l in lines if CRASH.search(l)][:3]
    unimpl = collections.Counter(m.group(1) for m in map(UNIMPL.match, lines) if m)
    return dict(test=test, status=status, executed=executed, failures=failures, secs=secs, msgs=msgs, unimpl=unimpl)


def read_tsv(path):
    res = {}
    if os.path.exists(path):
        for l in open(path):
            if l.startswith("#") or not l.strip():
                continue
            k, status, failures, *rest = l.rstrip("\n").split("\t")
            res[k] = (status, int(failures), rest[0] if rest else "")
    return res


def adopt(jobs, windows):
    """rostests: the planned tests Windows passes, and the ones it fails or was not asked.
    windows: {dll:test: (status, failures, executed)} of --native on Windows.
    -> ([(dll, exe, test)] Windows passes, [dll:test] Windows fails, [problem])"""
    keep, dropped, problems = [], [], []
    for d, exe, t in jobs:
        k = f"{d}:{t}"
        if k not in windows:
            problems.append(f"{k}: not measured on Windows (windows file lacks it: run --native on Windows, --update)")
        elif windows[k][0] == "ok":
            keep.append((d, exe, t))
        else:
            dropped.append(k)
    return keep, dropped, problems


def dll_list(path, arch):
    """the dlls of a dlls.txt: the first word of each line; only:<arch> keeps a line to that arch"""
    out = []
    for l in open(path):
        w = l.split()
        if not w or l.startswith("#"):
            continue
        only = [x[5:] for x in w[1:] if x.startswith("only:") and len(x) > 5]
        if not only or arch in only:
            out.append(w[0])
    return out


def plan(listed, sel, expected, explicit):
    """The tests to run, before sharding, and what the plan cannot cover.

    listed: {dll: (exe, its --list names, or None when the exe is missing)}. A missing test
    program, a dll whose --list names nothing, and a test of the expected file that its
    dll no longer lists all go to the second list: each fails the run, so a broken build or a
    selection that matches nothing cannot pass with fewer tests run. explicit (--expected):
    every row of the file must be planned, whatever the dll list says.
    -> ([(dll, exe, test)], [problem])"""
    jobs, problems = [], []
    wanted = lambda d, t: not sel.get(d) or "" in sel[d] or t in sel[d]
    for d, (exe, tests) in listed.items():
        if tests is None:
            problems.append(f"{d}: test program missing ({exe})")
            continue
        if not tests:
            # (still run: its output says why it lists nothing)
            problems.append(f"{d}: --list gave no tests")
            jobs.append((d, exe, "--list"))
        jobs.extend((d, exe, t) for t in tests if wanted(d, t))
    have = {f"{d}:{t}" for d, _, t in jobs}
    for k in sorted(expected):
        d, _, t = k.partition(":")
        if k in have or t == "--list":
            continue
        if explicit or (d in listed and listed[d][1] and wanted(d, t)):
            problems.append(f"{k}: expected test is not planned (not listed by its program, or not selected)")
    for d, ts in sel.items():
        problems.extend(f"{d}:{t}: selected test not listed by its program" for t in sorted(ts)
                        if t and d in listed and listed[d][1] and t not in listed[d][1])
    if not jobs:
        problems.append("nothing planned: no test selected")
    return jobs, problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--suite", choices=sorted(SUITES), default="winetest")
    ap.add_argument("--native", action="store_true",
                    help="run the test programs themselves, not on sake (on Windows): the results are windows-<arch>.tsv's")
    ap.add_argument("--arch", default="i386")
    ap.add_argument("--update", action="store_true")
    ap.add_argument("--expected", help="explicit baseline (all its tests must run)")
    ap.add_argument("-j", type=int, default=os.cpu_count())
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--fonts", default=os.environ.get("WINETEST_FONTS"))
    ap.add_argument("--shard", default="0/1", help="i/n: every n-th test from the i-th (CI matrix)")
    ap.add_argument("--browser", action="store_true", help="run in headless Chromium (run-browser.mjs): the GPU dlls of browser.txt")
    ap.add_argument("--no-exec", action="store_true", help="start no child processes (SAKE_NO_EXEC)")
    ap.add_argument("only", nargs="*")
    a = ap.parse_args()
    suite = SUITES[a.suite]
    sdir = suite["dir"]
    if a.native and not suite["windows"]:
        ap.error("--native: only for a suite measured on Windows (rostests)")
    if a.native and os.name == "nt":
        # (inherited by the tests: a crash ends the test with its exception code, not with
        # Windows' error report dialog waiting out the time limit)
        import ctypes
        ctypes.windll.kernel32.SetErrorMode(0x0001 | 0x0002 | 0x8000)  # FAILCRITICALERRORS NOGPFAULTERRORBOX NOOPENFILEERRORBOX
    sake = None if a.native else os.path.abspath(os.environ.get("SAKE", os.path.join(ROOT, "target/sake-out/sake.js")))
    w = os.path.abspath(os.environ.get(suite["env"], os.path.join(ROOT, "target", a.suite)))
    bindir = os.path.join(w, "bin", a.arch)
    dlls = dll_list(os.path.join(sdir, "dlls.txt"), a.arch)
    browser_txt = os.path.join(sdir, "browser.txt")
    gpu = dll_list(browser_txt, a.arch) if os.path.exists(browser_txt) else []
    dlls = [d for d in dlls if (d in gpu) == a.browser]
    suffix = ("-browser" if a.browser else "") + ("-windows" if a.native else "")
    sel = collections.defaultdict(set)
    for o in a.only:
        d, _, t = o.partition(":")
        sel[d].add(t)
    if sel:
        dlls = [d for d in dlls if d in sel] + [d for d in sel if d not in dlls]

    # (--native measures Windows: its baseline is the Windows file)
    exp_path = a.expected or os.path.join(sdir, f"windows-{a.arch}.tsv" if a.native else f"expected-{a.arch}{suffix}.tsv")
    if a.expected and not os.path.isfile(exp_path):
        raise FileNotFoundError(exp_path)
    expected = read_tsv(exp_path)
    env = dict(os.environ, SAKE_C=tempfile.mkdtemp(prefix="winetest-c-"), SAKE_NO_EXEC="1")  # (listing only)
    listed = {}
    for d in dlls:
        exe = os.path.join(bindir, suite["exe"].format(d.split(".")[0]))
        listed[d] = (exe, list_tests(sake, exe, env) if os.path.exists(exe) else None)
    shutil.rmtree(env["SAKE_C"], ignore_errors=True)
    all_jobs, unplanned = plan(listed, sel, expected, bool(a.expected))
    not_adopted = []
    if suite["windows"] and not a.native:
        win_path = os.path.join(sdir, f"windows-{a.arch}.tsv")
        if not os.path.isfile(win_path):
            raise FileNotFoundError(win_path)
        all_jobs, not_adopted, missing = adopt(all_jobs, read_tsv(win_path))
        unplanned += missing
        # (an expected row of a test Windows fails is not planned, and it is no problem)
        unplanned = [p for p in unplanned if p.split(": ", 1)[0] not in not_adopted]
        print(f"windows: {len(not_adopted)} tests Windows fails are not run", file=sys.stderr)
    si, sn = map(int, a.shard.split("/"))
    jobs = all_jobs[si::sn]
    print(f"plan: {len(jobs)} of {len(all_jobs)} tests (shard {a.shard})", file=sys.stderr)

    results = {}
    if a.browser:
        out = tempfile.mkdtemp(prefix="winetest-browser-")
        jf = os.path.join(out, "jobs")
        with open(jf, "w") as f:
            f.writelines(f"{d} {os.path.basename(exe)} {t}\n" for d, exe, t in jobs)
        subprocess.run(["node", os.path.join(HERE, "run-browser.mjs"), bindir, jf, out, str(a.timeout), str(a.j)], check=True)
        for d, exe, t in jobs:
            p = os.path.join(out, f"{d}:{t}.txt")
            text = open(p, errors="replace").read() if os.path.exists(p) else "timeout\n"
            r = parse(t, text, text.rstrip().endswith("timeout"), 0)
            results[f"{d}:{t}"] = r
            print(f"{r['status']:7} {d}:{t} {r['executed']} tests, {r['failures']} failures", flush=True)
        shutil.rmtree(out, ignore_errors=True)
    own_bin = None
    if suite["windows"]:
        own_bin = os.path.join(w, "run", a.arch)
        os.makedirs(own_bin, exist_ok=True)
    with concurrent.futures.ThreadPoolExecutor(a.j) as pool:
        futs = {pool.submit(run_one, sake, exe, t, a.timeout, a.fonts, a.no_exec, own_bin): (d, t) for d, exe, t in ([] if a.browser else jobs)}
        for f in concurrent.futures.as_completed(futs):
            d, t = futs[f]
            r = f.result()
            results[f"{d}:{t}"] = r
            print(f"{r['status']:7} {d}:{t} {r['executed']} tests, {r['failures']} failures ({r['secs']:.0f}s)", flush=True)

    # tests whose failure count depends on thread timing: only their status is compared
    flaky_txt = os.path.join(sdir, "flaky.txt")
    flaky = {l.split()[0] for l in open(flaky_txt) if l.strip() and not l.startswith("#")} if os.path.exists(flaky_txt) else set()
    worse, better = list(unplanned), []
    # (every planned test answered: a job that left no result is a test that did not run)
    worse.extend(f"{d}:{t}: planned, no result" for d, _, t in jobs if f"{d}:{t}" not in results)
    for k in sorted(results):
        r = results[k]
        if k not in expected:
            continue
        es, ef, _ = expected[k]
        # (failure counts of some tests move a little from run to run: timing, threads)
        if RANK[r["status"]] > RANK[es] or (r["status"] == es == "fail" and k not in flaky and r["failures"] > ef + max(5, ef // 10)):
            worse.append(f"{k}: {es} ({ef}) -> {r['status']} ({r['failures']})")
        elif RANK[r["status"]] < RANK[es] or r["failures"] < ef:
            better.append(f"{k}: {es} ({ef}) -> {r['status']} ({r['failures']})")
    new = sorted(k for k in results if k not in expected)

    def tsv(path):
        with open(path, "w") as f:
            where = "on Windows" if a.native else "on sake"
            f.write(f"# tests/winetest/run.py --suite {a.suite} --arch {a.arch}{' --native' if a.native else ''}: "
                    f"<dll>:<test> status failures executed ({suite['title']} {where})\n")
            keep = {k: v for k, v in expected.items() if k not in results} if path == exp_path else {}
            rows = {k: (r["status"], r["failures"], r["executed"]) for k, r in results.items()}
            # (the rows not run this time stay as they were, their executed count too)
            rows.update(keep)
            for k in sorted(rows):
                s, n, e = rows[k]
                f.write(f"{k}\t{s}\t{n}\t{e}\n")

    tsv(os.path.join(w, f"results-{a.arch}{suffix}.tsv"))
    report(os.path.join(w, f"report-{a.arch}{suffix}.md"), suite["title"] + (" on Windows" if a.native else " on sake"),
           a.arch, results, worse, better, not_adopted)
    if a.update:
        tsv(exp_path)
    counts = collections.Counter(r["status"] for r in results.values())
    print(f"\n{len(results)} tests: " + ", ".join(f"{counts[s]} {s}" for s in RANK)
          + (f" ({len(not_adopted)} Windows fails not run)" if not_adopted else ""))
    for l in better:
        print(f"better  {l}")
    for k in new:
        print(f"new     {k}: {results[k]['status']} ({results[k]['failures']})")
    for l in worse:
        print(f"WORSE   {l}")
    if unplanned:
        # (--update too: a run that could not plan everything must not rewrite the baseline's gaps away)
        print(f"\n{len(unplanned)} tests or programs the run could not plan")
        return 1
    if worse and not a.update:
        print(f"\n{len(worse)} regressions against {os.path.relpath(exp_path, ROOT)}")
        return 1
    return 0


def report(path, title, arch, results, worse, better, not_adopted=()):
    by_dll = collections.defaultdict(list)
    for k, r in results.items():
        by_dll[k.split(":")[0]].append(r)
    unimpl = collections.Counter()
    unimpl_tests = collections.Counter()
    for r in results.values():
        unimpl.update(r["unimpl"])
        unimpl_tests.update(r["unimpl"].keys())
    with open(path, "w") as f:
        f.write(f"# {title} ({arch})\n\n")
        f.write("| dll | tests | ok | fail | crash | timeout | checks run | failed checks |\n|---|---|---|---|---|---|---|---|\n")
        tot = collections.Counter()
        for d in sorted(by_dll):
            rs = by_dll[d]
            c = collections.Counter(r["status"] for r in rs)
            ex, fa = sum(r["executed"] for r in rs), sum(r["failures"] for r in rs)
            tot.update(c)
            tot["executed"] += ex
            tot["failures"] += fa
            tot["tests"] += len(rs)
            f.write(f"| {d} | {len(rs)} | {c['ok']} | {c['fail']} | {c['crash']} | {c['timeout']} | {ex} | {fa} |\n")
        f.write(f"| **all** | {tot['tests']} | {tot['ok']} | {tot['fail']} | {tot['crash']} | {tot['timeout']} | {tot['executed']} | {tot['failures']} |\n\n")
        if worse or better:
            f.write("## Changes against the expected results\n\n" + "".join(f"- worse: {l}\n" for l in worse) + "".join(f"- better: {l}\n" for l in better) + "\n")
        if not_adopted:
            f.write("## Not run: Windows fails them\n\n" + "".join(f"- {k}\n" for k in sorted(not_adopted)) + "\n")
        f.write("## Unimplemented functions, by the number of tests that called them\n\n")
        for fn, n in unimpl_tests.most_common(150):
            f.write(f"- {fn}: {n} tests ({unimpl[fn]} calls)\n")
        f.write("\n## First messages of each test that did not pass\n\n")
        for k in sorted(results):
            r = results[k]
            if r["status"] != "ok":
                f.write(f"### {k}: {r['status']} ({r['failures']} of {r['executed']})\n\n```\n" + "\n".join(m[:300] for m in r["msgs"]) + "\n```\n\n")


if __name__ == "__main__":
    sys.exit(main())
