"""shirusu.yml: a job that runs past a failed need (`!cancelled()`) still needs a plan.

Without a plan (no good build of main, say) the Windows jobs fail at once and sake waits
for a build that never comes (pick_build.py --wait 240): four hours holding the
concurrency group, every later run queued behind it."""
import os, re, unittest

here = os.path.dirname(os.path.abspath(__file__))
wf = os.path.join(here, '..', '..', '.github', 'workflows', 'shirusu.yml')


def jobs(text):
    """{job: (needs list, if expression or '')} of the jobs: block (two-space job keys)."""
    body = text.split('\njobs:\n', 1)[1]
    out = {}
    for m in re.finditer(r'^  ([a-z][a-z0-9-]*):\n((?:    .*\n|\s*\n)*)', body, re.M):
        name, block = m.group(1), m.group(2)
        n = re.search(r'^    needs: *(.*)$', block, re.M)
        needs = [] if not n else [x.strip() for x in n.group(1).strip('[] ').split(',') if x.strip()]
        i = re.search(r'^    if: *(.*)$', block, re.M)
        out[name] = (needs, i.group(1) if i else '')
    return out


def unguarded(js):
    return sorted(j for j, (needs, cond) in js.items()
                  if 'plan' in needs and '!cancelled()' in cond and "needs.plan.result == 'success'" not in cond)


class Workflow(unittest.TestCase):
    def test_parsed(self):
        js = jobs(open(wf, encoding='utf-8').read())
        for j in ('plan', 'windows', 'sake', 'compare'):
            self.assertIn(j, js)
        self.assertEqual(js['sake'][0], ['plan', 'windows'])

    def test_plan_guards(self):
        self.assertEqual(unguarded(jobs(open(wf, encoding='utf-8').read())), [])

    def test_check_catches(self):
        # every way a job can go wrong: the guard missing, only half of it, no need of plan
        text = ("on: push\njobs:\n  plan:\n    runs-on: x\n"
                "  a:\n    needs: [plan, b]\n    if: ${{ !cancelled() }}\n"
                "  b:\n    needs: plan\n    if: ${{ !cancelled() && needs.plan.result == 'success' }}\n"
                "  c:\n    needs: [b]\n    if: ${{ !cancelled() }}\n"
                "  d:\n    needs: plan\n")
        js = jobs(text)
        self.assertEqual(js['b'][0], ['plan'])
        self.assertEqual(unguarded(js), ['a'])


if __name__ == '__main__':
    unittest.main()
