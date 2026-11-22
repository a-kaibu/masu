"""run.py's reading of a test's output: python3 -m unittest tests/winetest/test_run.py"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
sys.dont_write_bytecode = True
import run  # noqa: E402

SUM = "{pid}:virtual: {n} tests executed (0 marked as todo, 0 as flaky, {f} failures), 0 skipped."


class Parse(unittest.TestCase):
    def status(self, *lines):
        return run.parse("virtual", "\n".join(lines) + "\n", False, 0)

    def test_finished_test_is_ok_or_fail(self):
        self.assertEqual(self.status(SUM.format(pid="0010", n=5, f=0))["status"], "ok")
        r = self.status("virtual.c:1: Test failed: x", SUM.format(pid="0010", n=5, f=1))
        self.assertEqual((r["status"], r["failures"], r["executed"]), ("fail", 1, 5))

    def test_a_childs_summary_does_not_finish_a_crashed_test(self):
        r = self.status(
            "virtual.c:174: Test failed: x",
            "0010:virtual: unhandled exception c0000005 at 7FEF01B2",
            SUM.format(pid="10010", n=0, f=0),
        )
        self.assertEqual(r["status"], "crash")
        self.assertEqual(r["failures"], 1)

    def test_childrens_counts_add_to_the_parents(self):
        r = self.status(SUM.format(pid="10010", n=3, f=1), SUM.format(pid="0010", n=5, f=0))
        self.assertEqual((r["status"], r["executed"], r["failures"]), ("fail", 8, 1))

    def test_output_without_pids(self):
        self.assertEqual(self.status("virtual: 2 tests executed (0 marked as todo, 0 failures), 0 skipped.")["status"], "ok")


class Plan(unittest.TestCase):
    """A run that plans fewer tests than it should fails instead of passing with 0 run."""

    EXP = {"kernel32:file": ("ok", 0, ""), "kernel32:path": ("fail", 3, "")}

    def test_full_plan_has_no_problems(self):
        jobs, problems = run.plan({"kernel32": ("k.exe", ["file", "path"])}, {}, self.EXP, False)
        self.assertEqual([t for _, _, t in jobs], ["file", "path"])
        self.assertEqual(problems, [])

    def test_missing_program_fails(self):
        # (the review's reproduction: the selected dll's exe is gone)
        jobs, problems = run.plan({"kernel32": ("k.exe", None)}, {"kernel32": {""}}, self.EXP, False)
        self.assertEqual(jobs, [])
        self.assertTrue(any("test program missing" in p for p in problems), problems)
        self.assertTrue(any("nothing planned" in p for p in problems), problems)

    def test_expected_test_no_longer_listed_fails(self):
        _, problems = run.plan({"kernel32": ("k.exe", ["file"])}, {}, self.EXP, False)
        self.assertEqual(problems, ["kernel32:path: expected test is not planned (not listed by its program, or not selected)"])

    def test_selection_narrows_without_failing(self):
        jobs, problems = run.plan({"kernel32": ("k.exe", ["file", "path"])}, {"kernel32": {"file"}}, self.EXP, False)
        self.assertEqual([t for _, _, t in jobs], ["file"])
        self.assertEqual(problems, [])

    def test_selected_test_that_does_not_exist_fails(self):
        _, problems = run.plan({"kernel32": ("k.exe", ["file", "path"])}, {"kernel32": {"fiel"}}, self.EXP, False)
        self.assertIn("kernel32:fiel: selected test not listed by its program", problems)
        self.assertIn("nothing planned: no test selected", problems)

    def test_explicit_baseline_requires_every_row(self):
        _, problems = run.plan({"gdiplus": ("g.exe", ["font"])}, {"gdiplus": {"font"}},
                               {"gdiplus:font": ("ok", 0, ""), "gdiplus:image": ("ok", 0, "")}, True)
        self.assertEqual(len(problems), 1, problems)

    def test_program_listing_nothing_is_run_and_reported(self):
        jobs, problems = run.plan({"msvcp140": ("m.exe", [])}, {}, {}, False)
        self.assertEqual([t for _, _, t in jobs], ["--list"])
        self.assertEqual(problems, ["msvcp140: --list gave no tests"])

    def test_main_exits_1_when_the_program_is_missing(self):
        import subprocess
        import tempfile
        with tempfile.TemporaryDirectory() as w:
            os.makedirs(os.path.join(w, "bin", "i386"))
            p = subprocess.run([sys.executable, os.path.join(os.path.dirname(__file__), "run.py"), "kernel32"],
                               env=dict(os.environ, WINETEST=w, SAKE="/nonexistent/sake.js"),
                               capture_output=True, text=True, check=False)
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)
        self.assertIn("test program missing", p.stdout)


if __name__ == "__main__":
    unittest.main()
