import { expect, test } from "vite-plus/test";
import {
  DONE,
  FAILED,
  fileUrl,
  makeShared,
  readServer,
  readSync,
  REPLY_SEQ,
  SEQ,
  STATE,
  WAIT,
  type Shared,
} from "./sync-read";

/** xorshift32 (seeded: a failure comes back the same) */
function rng(seed: number) {
  let s = seed >>> 0 || 1;
  return (n: number) => {
    s ^= s << 13;
    s >>>= 0;
    s ^= s >>> 17;
    s ^= s << 5;
    s >>>= 0;
    return s % n;
  };
}

const response = (files: Record<string, Uint8Array>, url: string) =>
  files[url] ? new Response(files[url] as Uint8Array<ArrayBuffer>) : new Response("no", { status: 404 });

/** The page's ask as readSync writes it, without its spin (the worker here shares this thread). */
function ask(s: Shared, url: string): void {
  // (readSync with a clock that runs out at once: it writes the ask and gives up)
  let t = 0;
  expect(readSync(url, s, 0, () => (t += 10))).toBeNull();
}

/** Until ctl[STATE] leaves WAIT, letting the worker's side run. */
async function answered(s: Shared): Promise<number> {
  const ctl = new Int32Array(s.ctl);
  for (let i = 0; i < 1000 && Atomics.load(ctl, STATE) === WAIT; i++)
    await new Promise((r) => setTimeout(r, 1));
  return Atomics.load(ctl, STATE);
}

test("the worker answers each ask with the file's bytes in the shared buffer, grown to fit (PBT: random files)", async () => {
  const r = rng(12345);
  const files: Record<string, Uint8Array> = {};
  for (let k = 0; k < 60; k++)
    files[`http://g/tree/x/f${k}`] = Uint8Array.from({ length: k ? r(70000) : 0 }, () => r(256));
  const s = makeShared(1 << 20);
  const stop = new AbortController();
  const done = readServer(s, async (u) => response(files, u), stop.signal);
  const ctl = new Int32Array(s.ctl);
  for (const [url, bytes] of Object.entries(files)) {
    ask(s, url);
    expect(await answered(s)).toBe(DONE);
    expect([ctl[1], ctl[2]]).toEqual([200, bytes.length]);
    // what readSync hands the game once DONE: the bytes, a copy not shared
    const got = new Uint8Array(s.data, 0, bytes.length).slice();
    expect(got).toEqual(bytes);
    expect(got.buffer instanceof ArrayBuffer).toBe(true);
  }
  stop.abort();
  Atomics.add(ctl, SEQ, 1);
  Atomics.notify(ctl, SEQ);
  await done;
}, 60_000);

test("the worker: a missing file is its status, a failed fetch or one too big FAILED, an ask given up on not answered", async () => {
  const s = makeShared(8);
  const ctl = new Int32Array(s.ctl);
  let release: () => void = () => undefined;
  const stop = new AbortController();
  const done = readServer(
    s,
    async (u) => {
      if (u === "boom") throw new TypeError("network");
      if (u === "slow") await new Promise<void>((r) => (release = r));
      return response({ a: Uint8Array.of(1, 2, 3), big: new Uint8Array(9), slow: Uint8Array.of(7) }, u);
    },
    stop.signal,
  );
  ask(s, "none");
  expect(await answered(s)).toBe(DONE);
  expect(ctl[1]).toBe(404);
  ask(s, "boom");
  expect(await answered(s)).toBe(FAILED);
  ask(s, "big");
  expect(await answered(s)).toBe(FAILED);
  // the page gave up on "slow" and asked for "a": "slow" ends unseen, "a" is the answer
  ask(s, "slow");
  await new Promise((r) => setTimeout(r, 5));
  ask(s, "a");
  release();
  expect(await answered(s)).toBe(DONE);
  expect(new Uint8Array(s.data, 0, ctl[2]!)).toEqual(Uint8Array.of(1, 2, 3));
  stop.abort();
  Atomics.add(ctl, SEQ, 1);
  Atomics.notify(ctl, SEQ);
  await done;
});

test("readSync: no worker (not isolated) null; a URL past its room null; one that never answers null", () => {
  const url = fileUrl("http://127.0.0.1:9/tree/g/game/", "www/data/日本 語.json");
  expect(url).toBe("http://127.0.0.1:9/tree/g/game/www/data/%E6%97%A5%E6%9C%AC%20%E8%AA%9E.json");
  expect(readSync(url, null)).toBeNull();
  expect(readSync("x".repeat(9000), makeShared(8))).toBeNull();
  let t = 0;
  expect(readSync(url, makeShared(8), 5000, () => (t += 1000))).toBeNull();
});

test("readSync reads what a finished answer holds (as the worker leaves it)", () => {
  const s = makeShared(16);
  const ctl = new Int32Array(s.ctl);
  // (a worker that answered before the page looked: STATE set from the ask's own write on)
  const real = Atomics.notify;
  Atomics.notify = ((a: Int32Array, i: number) => {
    if (a.buffer === s.ctl && i === SEQ) {
      (s.data as unknown as { grow(n: number): void }).grow(3);
      new Uint8Array(s.data).set([9, 8, 7]);
      Atomics.store(ctl, 1, 200);
      Atomics.store(ctl, 2, 3);
      Atomics.store(ctl, STATE, DONE);
      Atomics.store(ctl, REPLY_SEQ, Atomics.load(ctl, SEQ));
    }
    return 0;
  }) as typeof Atomics.notify;
  try {
    expect(readSync("u", s)).toEqual(Uint8Array.of(9, 8, 7));
  } finally {
    Atomics.notify = real;
  }
});

test("a late completion from the previous read cannot answer the next request", () => {
  const s = makeShared(8);
  const ctl = new Int32Array(s.ctl);
  let clock = 0;
  expect(
    readSync("next", s, 10, () => {
      Atomics.store(ctl, 1, 200);
      Atomics.store(ctl, STATE, DONE);
      Atomics.store(ctl, REPLY_SEQ, Atomics.load(ctl, SEQ) - 1);
      return (clock += 20);
    }),
  ).toBeNull();
});
