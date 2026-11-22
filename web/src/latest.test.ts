import { expect, test } from "vite-plus/test";
import { Latest } from "./latest";
import { deferred, simulate } from "./test/simulation";

test("two searches whose answers come in any order: only the newer one's is shown (review 22bb061 §6)", async () => {
  for (const order of [
    ["a", "b"],
    ["b", "a"],
  ] as const) {
    const asks = new Latest();
    let shown: string | null = null;
    const answers = { a: deferred<string>(), b: deferred<string>() };
    const look = async (q: "a" | "b") => {
      const ask = asks.begin();
      const found = await answers[q].promise;
      if (ask.current()) shown = found;
      return ask;
    };
    const a = look("a");
    const b = look("b");
    for (const q of order) answers[q].resolve(`answer ${q}`);
    const [askA, askB] = await Promise.all([a, b]);
    expect(shown, order.join()).toBe("answer b");
    expect(askA.signal.aborted).toBe(true);
    expect(askB.signal.aborted).toBe(false);
    asks.end();
    expect(askB.current()).toBe(false);
    expect(askB.signal.aborted).toBe(true);
  }
});

test("property: any number of searches answered in any order shows the last begun one's, or none after end", async () => {
  for (let seed = 1; seed <= 300; seed++)
    await simulate(async (sim) => {
      const asks = new Latest();
      const n = 1 + sim.randomBelow(6);
      const ends = sim.randomBelow(4) === 0;
      let shown = -1;
      for (let i = 0; i < n; i++) {
        const ask = asks.begin();
        setTimeout(() => {
          if (ask.current()) shown = i;
        }, sim.randomBelow(50));
      }
      if (ends) asks.end();
      await sim.advance(100);
      expect(shown, `seed ${seed}`).toBe(ends ? -1 : n - 1);
    }, seed);
});
