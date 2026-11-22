import { expect, test, vi } from "vite-plus/test";
import { blake3Hex } from "./blake3";
import type { ChunkStore } from "./chunkstore";
import {
  BOOT_GIVE_UP_MS,
  fetchBoot,
  fetchRetry,
  getChunk,
  HttpError,
  isFull,
  keepChunk,
  storeChunk,
  treeChunks,
  trimChunks,
  trimOrder,
  watchRetries,
} from "./net";

const bytes = (n: number) => Uint8Array.from({ length: n }, (_, i) => i % 251);

test("dropped connections and 503s are retried; a 404 is final", async () => {
  let n = 0;
  const request: typeof fetch = async () => {
    n++;
    if (n === 1) throw new TypeError("Failed to fetch");
    if (n === 2) return new Response(null, { status: 503 });
    return new Response(bytes(1000));
  };
  expect(Array.from(await fetchRetry("/x", {}, { request }))).toEqual(Array.from(bytes(1000)));
  expect(n).toBe(3);
  let m = 0;
  const missing: typeof fetch = async () => (m++, new Response(null, { status: 404 }));
  await expect(fetchRetry("/x", {}, { request: missing })).rejects.toBeInstanceOf(HttpError);
  expect(m).toBe(1);
});

test("a body that stops arriving is aborted and fetched again", async () => {
  let n = 0;
  const request: typeof fetch = async (_u, init) => {
    n++;
    if (n > 1) return new Response(bytes(70000));
    // half the body, then nothing: the stall watch aborts it
    const body = new ReadableStream<Uint8Array>({
      start(c) {
        c.enqueue(bytes(100));
        init!.signal!.addEventListener("abort", () => c.error(init!.signal!.reason));
      },
    });
    return new Response(body);
  };
  const t = Date.now();
  expect((await fetchRetry("/x", {}, { request, stallMs: 150 })).length).toBe(70000);
  expect(n).toBe(2);
  expect(Date.now() - t).toBeLessThan(3000);
});

test("gives up after giveUpMs, and the page's retries are counted while they last", async () => {
  const seen: number[] = [];
  const stop = watchRetries((r) => seen.push(r));
  const request: typeof fetch = async () => {
    throw new TypeError("Failed to fetch");
  };
  await expect(fetchRetry("/x", {}, { request, giveUpMs: 500 })).rejects.toThrow("Failed to fetch");
  stop();
  expect(seen[0]).toBe(1);
  expect(seen.at(-1)).toBe(0);
});

test("a tree's chunk hashes (holes left out)", () => {
  const h = (c: string) => c.repeat(32);
  const tsv = `vnvm-tree\t2\t16777216\nd\t755\t0\tdir\nf\t644\t0\tdir/a\t30\t${h("a")}:10,:5,${h("b")}:15\nf\t644\t0\tempty\t0\t\nl\t777\t0\tlink\ta\n`;
  expect(treeChunks(tsv)).toEqual([h("a"), h("b")]);
});

/** A device store stand-in (OPFS's shape): `kept` in write order, each [url, bytes]. */
function deviceStore(kept: [string, number][]) {
  const m = new Map<string, number>(kept);
  const store: ChunkStore = {
    match: async () => undefined,
    put: async () => undefined,
    keys: async () => [...m.keys()].map((url) => ({ url })),
    list: async () => [...m].map(([url, size], time) => ({ url, size, time })),
    delete: async (k) => m.delete(typeof k === "string" ? k : "url" in k ? k.url : String(k)),
  };
  return { m, store };
}

test("a chunk on the device is read from it; one the bucket lacks fails, never asked of the server", async () => {
  const good = bytes(5000);
  const hex = blake3Hex(good);
  const url = `https://s/store/chunks/${hex.slice(0, 2)}/${hex}`;
  const asked: string[] = [];
  const request: typeof fetch = async (u) => {
    const s = typeof u === "string" ? u : u instanceof URL ? u.href : u.url;
    asked.push(s);
    return new Response(null, { status: 404 });
  };
  const store: ChunkStore = {
    match: async (k) => (k === url ? new Response(good) : undefined),
    put: async () => undefined,
    delete: async () => false,
    keys: async () => [],
    list: async () => [],
  };
  const r = await getChunk("https://s/store/", hex, { cache: store, request });
  expect(r.cached).toBe(true);
  expect(Array.from(r.data)).toEqual(Array.from(good));
  expect(asked).toEqual([]);
  const other = blake3Hex(bytes(10));
  await expect(getChunk("https://s/store/", other, { cache: store, request })).rejects.toThrow(/bucket/);
  expect(asked.length).toBeGreaterThan(0);
  expect(asked.every((u) => !u.includes("/store/chunks/"))).toBe(true);
});

/** trimChunks over a stand-in store: `sizes` oldest first; the indexes that stay. */
async function trimmed(
  sizes: number[],
  storage: { quota: number; usage?: number },
  saved = new Set<string>(),
): Promise<number[]> {
  const hex = (i: number) => i.toString(16).padStart(32, "0");
  const { m, store } = deviceStore(sizes.map((n, i) => [`opfs:/masu-chunks/${hex(i)}`, n]));
  Object.defineProperty(navigator, "storage", {
    value: { estimate: async () => storage, persist: async () => true },
    configurable: true,
  });
  await trimChunks(async () => saved, store);
  return [...m.keys()].map((u) => parseInt(u.slice(-32), 16));
}

test("the chunks are trimmed by their sizes, the saved games' chunks kept", async () => {
  // limit: half the quota, 2 MiB. 20 chunks of 64 KiB (1.25 MiB): all stay
  expect((await trimmed(Array(20).fill(64 << 10), { quota: 4 << 20 })).length).toBe(20);
  // 12 more of 256 KiB: 4.25 MiB, the oldest go, but not the saved ones (0 and 1)
  const saved = new Set([0, 1].map((i) => i.toString(16).padStart(32, "0")));
  const left = await trimmed(
    [...Array(20).fill(64 << 10), ...Array(12).fill(256 << 10)],
    { quota: 4 << 20 },
    saved,
  );
  expect(left).toContain(0);
  expect(left).toContain(1);
  const kept = left.reduce((a, i) => a + (i < 20 ? 64 << 10 : 256 << 10), 0);
  expect(kept).toBeLessThanOrEqual(2 << 20);
  expect(kept).toBeGreaterThan((2 << 20) - (256 << 10));
});

test("a big quota keeps a small chunk store whole (the 2 GB cap is not negative)", async () => {
  for (const quota of [100e9, 0])
    expect((await trimmed([10, 10, 10], { quota })).length, `quota ${quota}`).toBe(3);
});

test("16 MiB chunks are counted as 16 MiB: 100 of them (1.6 GiB) are trimmed to the limit", async () => {
  const MiB = 1 << 20;
  // quota 1 GiB: the chunks' share is 512 MiB, so 32 of 16 MiB stay (the newest)
  const left = await trimmed(Array(100).fill(16 * MiB), { quota: 1024 * MiB });
  expect(left.length).toBe(32);
  expect(left[0]).toBe(68);
});

test("what else the origin keeps (saves, states) counts: chunks give way so the whole fits", async () => {
  const MiB = 1 << 20;
  // quota 1000 MiB, 600 MiB of states besides 20 chunks of 16 MiB (320 MiB): 800 MiB in all
  // allowed, so 200 MiB of chunks (12) stay
  const left = await trimmed(Array(20).fill(16 * MiB), { quota: 1000 * MiB, usage: 920 * MiB });
  expect(left.length).toBe(12);
});

test("property: after a trim the chunks fit their share, only the oldest unsaved went", async () => {
  let seed = 99;
  const rnd = (n: number) => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff), seed % n);
  for (let run = 0; run < 100; run++) {
    const n = 1 + rnd(40);
    const sizes = Array.from({ length: n }, () => (1 + rnd(64)) << 10);
    const quota = (1 + rnd(4000)) << 10;
    const others = rnd(2) ? rnd(quota) : 0;
    const total = sizes.reduce((a, b) => a + b, 0);
    const saved = new Set(
      Array.from({ length: n }, (_, i) => i)
        .filter(() => rnd(5) === 0)
        .map((i) => i.toString(16).padStart(32, "0")),
    );
    const left = await trimmed(sizes, { quota, usage: others ? others + total : 0 }, saved);
    const limit = Math.min(quota * 0.5, others ? quota * 0.8 - others : Infinity);
    const isSaved = (i: number) => saved.has(i.toString(16).padStart(32, "0"));
    const kept = left.reduce((a, i) => a + sizes[i]!, 0);
    const savedBytes = sizes.reduce((a, s, i) => a + (isSaved(i) ? s : 0), 0);
    // fits (unless the saved games' chunks alone are over)
    expect(kept <= Math.max(limit, savedBytes) || kept === savedBytes, `run ${run}`).toBe(true);
    // every saved chunk stays; what went is a prefix of the unsaved, oldest first
    for (let i = 0; i < n; i++) if (isSaved(i)) expect(left).toContain(i);
    const gone = sizes.map((_, i) => i).filter((i) => !left.includes(i));
    const unsaved = sizes.map((_, i) => i).filter((i) => !isSaved(i));
    expect(gone, `run ${run}`).toEqual(unsaved.slice(0, gone.length));
    // and no more went than needed
    if (gone.length) expect(kept + sizes[gone.at(-1)!]! > limit, `run ${run}`).toBe(true);
  }
});

test("a tree's chunk lengths, for the read-ahead's reservations", async () => {
  const { treeChunkSizes } = await import("./net");
  const h = (c: string) => c.repeat(32);
  const tsv = `vnvm-tree\t2\t16777216\nd\t755\t0\tdir\nf\t644\t0\tdir/a\t16777236\t${h("a")}:16777216,:15,${h("b")}:5\nf\t644\t0\tempty\t0\t\nl\t777\t0\tlink\ta\n`;
  expect([...treeChunkSizes(tsv)]).toEqual([
    [h("a"), 16777216],
    [h("b"), 5],
  ]);
});

test("a chunk the full device could not take: storeChunk says so, keepChunk lets it go", async () => {
  const quota = new DOMException("The quota has been exceeded.", "QuotaExceededError");
  const full = { put: async () => Promise.reject(quota) } as unknown as ChunkStore;
  const data = new Uint8Array([1, 2, 3]);
  await expect(storeChunk(full, "https://s/chunks/ab/abc", data)).rejects.toBe(quota);
  await expect(keepChunk(full, "https://s/chunks/ab/abc", data)).resolves.toBeUndefined();
  expect(isFull(quota)).toBe(true);
  expect(isFull(new Error("QuotaExceededError: no room"))).toBe(true);
  expect(isFull(new TypeError("Failed to fetch"))).toBe(false);
  expect(isFull(null)).toBe(false);
  let kept: Response | undefined;
  const room = { put: async (_k: unknown, r: Response) => void (kept = r) } as unknown as ChunkStore;
  await storeChunk(room, "https://s/chunks/ab/abc", data);
  expect(kept?.headers.get("x-masu-size")).toBe("3");
});

/**
 * trimChunks over a store with list() (OPFS: listed by name, not age), random sizes, times,
 * saved ones and deletes refused (a read holds the file), against a model: the chunks go oldest
 * written first, saved ones never, a refused delete is not counted as freed, and the trim stops
 * as soon as what is left fits (property, seeded).
 */
test("trim: the oldest written go first, counted only when gone (property, seeded)", async () => {
  let x = 7;
  const rnd = (n: number) => ((x = (Math.imul(x, 1103515245) + 12345) >>> 0) >>> 8) % n;
  const hex = (i: number) => i.toString(16).padStart(32, "0");
  for (let run = 0; run < 200; run++) {
    const n = 1 + rnd(40);
    const kept = Array.from({ length: n }, (_, i) => ({
      url: `opfs:/masu-chunks/${hex(i)}`,
      size: 1 + rnd(1000),
      time: rnd(4) ? 1 + rnd(50) : 0,
    }));
    const saved = new Set(kept.filter(() => !rnd(5)).map((k) => k.url.slice(-32)));
    const held = new Set(kept.filter(() => !rnd(6)).map((k) => k.url));
    const quota = 1 + rnd(40 * 1000);
    const gone: string[] = [];
    const store: ChunkStore = {
      match: async () => undefined,
      put: async () => undefined,
      keys: async () => kept,
      list: async () => kept.map((k) => ({ ...k })),
      delete: async (k) => {
        const u = typeof k === "string" ? k : "url" in k ? k.url : String(k);
        if (held.has(u)) return false;
        gone.push(u);
        return true;
      },
    };
    Object.defineProperty(navigator, "storage", {
      value: { estimate: async () => ({ quota }) },
      configurable: true,
    });
    await trimChunks(async () => saved, store);
    const limit = quota * 0.5;
    // model: in trim order, skip saved, delete; stop when what is left fits
    const order = trimOrder(kept);
    let over = kept.reduce((a, k) => a + k.size, 0) - limit;
    const want: string[] = [];
    for (const k of order) {
      if (over <= 0) break;
      if (saved.has(k.url.slice(-32))) continue;
      if (!held.has(k.url)) {
        want.push(k.url);
        over -= k.size;
      }
    }
    expect(gone, `run ${run}`).toEqual(want);
    // the order: by time, then as listed
    for (let i = 1; i < order.length; i++) {
      const a = order[i - 1]!,
        b = order[i]!;
      expect(a.time < b.time || (a.time === b.time && kept.indexOf(a) < kept.indexOf(b))).toBe(true);
    }
  }
});

test("what the start screen waits on is given up after BOOT_GIVE_UP_MS, not 15 minutes, on a server that answers 503", async () => {
  vi.useFakeTimers();
  try {
    let asked = 0;
    const request: typeof fetch = async () => (asked++, new Response("busy", { status: 503 }));
    const t0 = Date.now();
    const got = fetchBoot("/trees/x.tsv", { request }).then(
      () => "ok",
      (e: unknown) => String(e),
    );
    await vi.advanceTimersByTimeAsync(BOOT_GIVE_UP_MS + 60_000);
    expect(await got).toMatch(/HTTP 503/);
    expect(Date.now() - t0).toBeLessThanOrEqual(BOOT_GIVE_UP_MS + 60_000);
    expect(asked).toBeGreaterThan(1);
    expect(BOOT_GIVE_UP_MS).toBeLessThan(15 * 60000);
    // (still retried: one that answers within the time is read)
    let n = 0;
    const later: typeof fetch = async () =>
      n++ < 3 ? new Response("", { status: 503 }) : new Response("tsv");
    const ok = fetchBoot("/trees/y.tsv", { request: later });
    await vi.advanceTimersByTimeAsync(30_000);
    expect(new TextDecoder().decode(await ok)).toBe("tsv");
  } finally {
    vi.useRealTimers();
  }
});

test("a signal already aborted sends no request, with or without a budget (review 22bb061 §6)", async () => {
  for (const budget of [undefined, 3]) {
    let n = 0;
    const request: typeof fetch = async () => (n++, new Response(bytes(10)));
    const gone = new AbortController();
    gone.abort(new Error("left the screen"));
    await expect(fetchRetry("/x", {}, { request, signal: gone.signal, budget })).rejects.toThrow(
      "left the screen",
    );
    expect(n, `budget ${budget}`).toBe(0);
  }
});

test("property: whenever the ask is cancelled, no try starts after it (seeded)", async () => {
  vi.useFakeTimers();
  try {
    for (let seed = 1; seed <= 200; seed++) {
      let r = seed;
      const rnd = (k: number) => ((r = (r * 1103515245 + 12345) >>> 0) >>> 8) % k;
      const gone = new AbortController();
      const cancelAt = rnd(8);
      let n = 0;
      let after = 0;
      const request: typeof fetch = async (_u, init) => {
        if (gone.signal.aborted || init?.signal?.aborted) after++;
        if (n++ === cancelAt) gone.abort(new Error("cancelled"));
        return rnd(2)
          ? new Response(null, { status: 503 })
          : Promise.reject(new TypeError("Failed to fetch"));
      };
      const p = fetchRetry("/x", {}, { request, signal: gone.signal }).catch((e: unknown) => e);
      await vi.advanceTimersByTimeAsync(120_000);
      expect(String(await p), `seed ${seed}`).toContain("cancelled");
      expect(after, `seed ${seed}`).toBe(0);
      expect(n, `seed ${seed}`).toBe(cancelAt + 1);
    }
  } finally {
    vi.useRealTimers();
  }
});
