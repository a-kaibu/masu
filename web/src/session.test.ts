import { expect, test } from "vite-plus/test";
import { inputTarget, Lifetime, ProcessTable } from "./session";

function rng(seed: number) {
  let s = seed >>> 0;
  return (n: number) => Math.floor(((s = (Math.imul(s, 1664525) + 1013904223) >>> 0) / 0x100000000) * n);
}

test("the root exits first, then its child that never drew: the run ends once (review 22bb061 §5)", () => {
  let ended = 0;
  const t = new ProcessTable(() => ended++);
  const child = t.child();
  t.rootExited();
  expect(ended).toBe(0);
  expect(t.live).toBe(1);
  child.ended();
  expect(ended).toBe(1);
  expect(t.live).toBe(0);
  // (a second exit callback of either changes nothing)
  child.ended();
  t.rootExited();
  expect(ended).toBe(1);
});

test("any order of starts, exits and failures: ended once, exactly when the last process is gone (seeded property)", () => {
  for (let seed = 1; seed <= 3000; seed++) {
    const r = rng(seed);
    let ended = 0;
    const t = new ProcessTable(() => ended++);
    const open: ReturnType<ProcessTable["child"]>[] = [];
    let root = true;
    for (let step = 0; step < 40 && !t.ended; step++) {
      const what = r(4);
      if (what === 0) open.push(t.child());
      else if (what === 1 && open.length) {
        const i = r(open.length);
        const c = open[i]!;
        // (an end said twice: onExit and onAbort, or finish and failed)
        c.ended();
        if (r(2)) c.ended();
        open.splice(i, 1);
      } else if (what === 2 && root) {
        root = false;
        t.rootExited();
      }
      expect(ended).toBe(!root && !open.length ? 1 : 0);
      expect(t.live).toBe((root ? 1 : 0) + open.length);
    }
    if (!t.ended) {
      if (root) t.rootExited();
      for (const c of open.splice(0)) c.ended();
    }
    expect(ended).toBe(1);
    expect(() => t.child()).toThrow();
  }
});

test("a lifetime's async work: nothing kept after it ended, and what came late is let go", async () => {
  const run = new Lifetime();
  const stream = run.child();
  const freed: string[] = [];
  const late = new Promise<string>((ok) => setTimeout(() => ok("decoder"), 5));
  stream.onEnd(() => freed.push("stream"));
  run.onEnd(() => freed.push("run"));
  run.end();
  expect(stream.over).toBe(true);
  expect(stream.keep(await late, (d) => freed.push(d))).toBeNull();
  expect(freed).toEqual(["run", "stream", "decoder"]);
  const live = new Lifetime();
  expect(live.keep(1)).toBe(1);
  live.end();
  live.end();
  let after = 0;
  live.onEnd(() => after++);
  expect(after).toBe(1);
});

test("a key from a field or a dialog of the page is the page's; one from the game's screen is the game's", () => {
  const el = (inside: string[]) =>
    ({ closest: (s: string) => (inside.some((i) => s.includes(i)) ? {} : null) }) as unknown as EventTarget;
  expect(inputTarget(el(["input"]))).toBe("overlay");
  expect(inputTarget(el(['[role="dialog"]']))).toBe("overlay");
  expect(inputTarget(el(["textarea"]))).toBe("overlay");
  expect(inputTarget(el([]))).toBe("game");
  expect(inputTarget(null)).toBe("game");
  // (the window or the document: no element)
  expect(inputTarget({} as EventTarget)).toBe("game");
});
