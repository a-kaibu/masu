import { expect, test } from "vite-plus/test";
import { FrameStart, type Init } from "./frame-protocol";
import { deferred, seeds, simulate } from "./test/simulation";

const init = (launch = "a"): Init => ({ masu: "init", launch, items: [], base: "", names: [] });

for (const seed of seeds)
  test(`simulation: inits sent again while the reads' worker starts release the page's events once (seed ${seed})`, async () => {
    await simulate(async (sim) => {
      let went = 0;
      const start = new FrameStart(() => went++);
      const worker = deferred<"ready">();
      let started = 0;
      const begin = () => (started++, worker.promise);
      // (the app answers each hello sent again: HELLO_AGAIN; the worker comes some seeded time later)
      for (const ms of [0, 500, 1000, 2000, 4000])
        setTimeout(() => start.init(init(), begin), ms + sim.randomBelow(300));
      setTimeout(() => worker.resolve("ready"), sim.randomBelow(6000));
      await sim.advance(10_000);
      expect(started).toBe(1);
      expect(went).toBe(1);
      expect(start.state).toEqual({ kind: "ready", launch: "a" });
      // (and the timeout after changes nothing)
      expect(start.gaveUp()).toBe(false);
      expect(went).toBe(1);
    }, seed);
  });

test("a worker that did not start is told apart from one that did; the game goes on once", async () => {
  let went = 0;
  const start = new FrameStart(() => went++);
  expect(start.init(init(), async () => "timeout")).toBe(true);
  expect(start.init(init(), async () => "ready")).toBe(false);
  await Promise.resolve();
  await Promise.resolve();
  expect(start.state.kind).toBe("failed");
  expect(went).toBe(1);
});

test("no init in time: the game goes on without saves, a late init starts nothing", async () => {
  let went = 0;
  const start = new FrameStart(() => went++);
  expect(start.gaveUp()).toBe(true);
  expect(start.init(init(), async () => "none")).toBe(false);
  expect(went).toBe(1);
  expect(start.state).toMatchObject({ kind: "failed", why: "no saves from the app" });
});
