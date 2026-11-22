import { expect, test } from "vite-plus/test";
import { pool, workers } from "./jobs";
import { seeds, simulate } from "./test/simulation";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

test("property: a job's end is said only once every runner stopped, with the first failure (review 22bb061 §6, seeded)", async () => {
  for (const seed of [...seeds, ...Array.from({ length: 300 }, (_, i) => i + 1)])
    await simulate(async (sim) => {
      const width = 1 + sim.randomBelow(6);
      const outer = new AbortController();
      const cancelAt = sim.randomBelow(3) === 0 ? sim.randomBelow(300) : -1;
      if (cancelAt >= 0) setTimeout(() => outer.abort(new Error("cancelled")), cancelAt);
      let live = 0;
      let first: string | null = null;
      const plan = Array.from({ length: width }, () => ({
        steps: 1 + sim.randomBelow(5),
        failAt: sim.randomBelow(4) === 0 ? sim.randomBelow(5) : -1,
        ms: sim.randomBelow(80),
      }));
      let liveAtEnd = -1;
      const job = workers(
        width,
        async (i, signal) => {
          live++;
          try {
            for (let step = 0; step < plan[i]!.steps && !signal.aborted; step++) {
              // (a runner busy with something that does not look at the signal)
              await sleep(plan[i]!.ms);
              if (step === plan[i]!.failAt) {
                first ??= `runner ${i}`;
                throw new Error(`runner ${i}`);
              }
            }
          } finally {
            live--;
          }
        },
        outer.signal,
      ).then(
        () => ((liveAtEnd = live), "ok"),
        (e: unknown) => ((liveAtEnd = live), String(e instanceof Error ? e.message : e)),
      );
      await sim.advance(2000);
      const end = await job;
      expect(liveAtEnd, `seed ${seed}`).toBe(0);
      if (first) expect(end, `seed ${seed}`).toBe(first);
      else expect(end, `seed ${seed}`).toBe("ok");
    }, seed);
});

test("an aborted outer signal reaches every runner, also one aborted before the start", async () => {
  const gone = new AbortController();
  gone.abort(new Error("x"));
  const seen: boolean[] = [];
  await workers(3, async (_, s) => void seen.push(s.aborted), gone.signal);
  expect(seen).toEqual([true, true, true]);
});

test("pool: results in order, and no job starts after a failure", async () => {
  expect(
    await pool(
      2,
      [1, 2, 3].map((n) => async () => n),
    ),
  ).toEqual([1, 2, 3]);
  const started: number[] = [];
  await expect(
    pool(
      1,
      [0, 1, 2].map((n) => async () => {
        started.push(n);
        if (n === 0) throw new Error("no");
        return n;
      }),
    ),
  ).rejects.toThrow("no");
  expect(started).toEqual([0]);
});
