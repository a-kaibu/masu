import { expect, test } from "vite-plus/test";
import { getState, putState, stateHash, type StateInfo } from "./savestate";
import { cache, localIndex, publish } from "./snapshot-store";
import { memoryOpfs } from "./test/opfs";
import { deferred, seeds, simulate } from "./test/simulation";

const GAME = "g";
const body = (n: number) => new Uint8Array(Array.from({ length: 64 }, (_, i) => (i * 31 + n) & 255));
const entry = (slot: number, created: string): StateInfo => ({
  slot,
  created,
  size: 64,
  build: "b",
  thumb: "",
});

type Sim = Parameters<Parameters<typeof simulate>[0]>[0];

/** A fresh origin's file system, each of its calls taking a seeded 0..`spread` ms. */
function origin(sim: Sim, spread = 6) {
  const fs = memoryOpfs(() => new Promise((ok) => setTimeout(ok, sim.randomBelow(spread))));
  (sim.browser.globals.navigator as unknown as { storage: unknown }).storage = fs.storage;
  return fs;
}

/** Runs the clock until every one of `work` has settled. */
async function settle<T>(sim: Sim, work: Promise<T>[]): Promise<PromiseSettledResult<T>[]> {
  let done = false;
  const all = Promise.allSettled(work).then((r) => ((done = true), r));
  for (let i = 0; i < 100_000 && !done; i++) await sim.advance(1);
  expect(done).toBe(true);
  return all;
}

/** Every slot the index lists has its body, the bytes the hash names. */
async function consistent(sim: Sim, fs: ReturnType<typeof memoryOpfs>) {
  const files = fs.files(`states/${GAME}`);
  const [read] = await settle(sim, [localIndex(GAME)]);
  if (read!.status !== "fulfilled") throw read!.reason;
  const index = read!.value;
  for (const s of index) {
    const b = files.get(`${s.hash}.snap`);
    expect(b, `slot ${s.slot}'s body ${s.hash}`).toBeDefined();
    expect(await stateHash(b!)).toBe(s.hash);
  }
  return index;
}

for (const seed of seeds) {
  test(`simulation: two tabs saving two slots at once keep both states and their bodies (seed ${seed})`, async () => {
    await simulate(async (sim) => {
      const fs = origin(sim);
      const results = await settle(sim, [
        putState(GAME, entry(1, "2026-11-22T00:00:01Z"), body(1), false),
        putState(GAME, entry(2, "2026-11-22T00:00:02Z"), body(2), false),
      ]);
      expect(results.map((r) => r.status)).toEqual(["fulfilled", "fulfilled"]);
      const index = await consistent(sim, fs);
      expect(index.map((s) => s.slot)).toEqual([1, 2]);
      expect([...fs.files(`states/${GAME}`).keys()].sort()).toHaveLength(3);
    }, seed);
  });

  test(`simulation: an older state's download finishing after a newer save leaves the slot new (seed ${seed})`, async () => {
    await simulate(async (sim) => {
      const fs = origin(sim);
      const old = body(10);
      const oldInfo = { ...entry(1, "2026-11-21T00:00:00Z"), hash: await stateHash(old) };
      const served = deferred<void>();
      globalThis.fetch = async () => {
        await served.promise;
        return new Response(old, { status: 200, headers: { etag: '"1"' } });
      };
      const fetched = getState(GAME, oldInfo);
      const saved = putState(GAME, entry(1, "2026-11-22T00:00:00Z"), body(11), false);
      // (the new save is done before the old body arrives, then the old body some seeded time after)
      await settle(sim, [saved]);
      setTimeout(() => served.resolve(), sim.randomBelow(20));
      const [got] = await settle(sim, [fetched]);
      expect(got!.status).toBe("fulfilled");
      const index = await consistent(sim, fs);
      expect(index).toHaveLength(1);
      expect(index[0]!.hash).toBe((await saved).hash);
      expect(fs.files(`states/${GAME}`).has(`${oldInfo.hash}.snap`)).toBe(false);
    }, seed);
  });

  test(`simulation: any mix of saves and downloads in two tabs keeps every listed body and every saved slot (seed ${seed})`, async () => {
    await simulate(async (sim) => {
      const fs = origin(sim, 4);
      const ops: Promise<unknown>[] = [];
      const published = new Set<number>();
      for (let i = 0; i < 12; i++) {
        const slot = sim.randomBelow(4);
        const created = `2026-11-22T00:00:${String(10 + sim.randomBelow(40)).padStart(2, "0")}Z`;
        const b = body(100 + i);
        const info = { ...entry(slot, created), hash: await stateHash(b) };
        const at = sim.randomBelow(30);
        if (sim.randomBelow(2)) {
          published.add(slot);
          ops.push(new Promise((ok) => setTimeout(ok, at)).then(() => publish(GAME, info, b)));
        } else {
          ops.push(new Promise((ok) => setTimeout(ok, at)).then(() => cache(GAME, info, b)));
        }
      }
      const results = await settle(sim, ops);
      expect(results.every((r) => r.status === "fulfilled")).toBe(true);
      const index = await consistent(sim, fs);
      for (const slot of published) expect(index.some((s) => s.slot === slot)).toBe(true);
      // (nothing kept that no slot names)
      expect(fs.files(`states/${GAME}`).size).toBe(index.length + 1);
    }, seed);
  });
}

test("simulation: a download of the slot's own state keeps its body; a newer one moves the slot", async () => {
  await simulate(async (sim) => {
    const fs = origin(sim, 1);
    const a = body(1);
    const ai = { ...entry(0, "2026-11-22T00:00:00Z"), hash: await stateHash(a) };
    await settle(sim, [publish(GAME, ai, a)]);
    fs.files(`states/${GAME}`).delete(`${ai.hash}.snap`);
    const [kept] = await settle(sim, [cache(GAME, ai, a)]);
    expect(kept).toEqual({ status: "fulfilled", value: "kept" });
    const b = body(2);
    const bi = { ...entry(0, "2026-11-22T00:00:09Z"), hash: await stateHash(b) };
    const [adopted] = await settle(sim, [cache(GAME, bi, b)]);
    expect(adopted).toEqual({ status: "fulfilled", value: "adopted" });
    const [older] = await settle(sim, [cache(GAME, ai, a)]);
    expect(older).toEqual({ status: "fulfilled", value: "older" });
    expect((await consistent(sim, fs)).map((s) => s.hash)).toEqual([bi.hash]);
  });
});
