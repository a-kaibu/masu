import { expect, test } from "vite-plus/test";
import { KEEP_AFTER_MS, KEEP_BUSY_MS, keepServerWasm, type Kept } from "./wasmkeep";

const URL_ = "https://w2.example/sake/sake.wasm?v=x";

/** A body of `parts` chunks that says how far it was read; `cut`: fails after that many. */
function body(parts: number, cut?: number) {
  const seen = { read: 0, cancelled: false };
  let i = 0;
  const stream = new ReadableStream<Uint8Array>({
    pull(c) {
      if (cut !== undefined && i === cut) return c.error(new TypeError("network"));
      if (i === parts) return c.close();
      i++;
      seen.read = i;
      c.enqueue(new Uint8Array(1000).fill(i));
    },
    cancel() {
      seen.cancelled = true;
    },
  });
  return { stream, seen };
}

// property (seeded, against a model): whatever the HTTP cache, the game's business, the
// server's answer and its body do, the file is asked for only after the start's wait and the
// game's busy spells (at most 120 s of them), once from the cache (only-if-cached) and at most
// once from the server (at low priority); a whole body is read to its end ("kept"), a cut one
// or a failed answer is "failed", a cached one "there" with nothing fetched; it never throws
test("keeps the server's file as the model says (seeded)", async () => {
  let x = 11;
  const rnd = (n: number) => ((x = (Math.imul(x, 1664525) + 1013904223) >>> 0), x % n);
  for (let run = 0; run < 300; run++) {
    const cached = rnd(3) === 0;
    const cacheThrows = !cached && rnd(4) === 0;
    const busyFor = rnd(4) === 0 ? 200 : rnd(5);
    const answer = (["ok", "ok", "500", "throws", "nobody"] as const)[rnd(5)]!;
    const parts = rnd(6);
    const cut = answer === "ok" && rnd(4) === 0 ? rnd(parts + 1) : undefined;
    let now = 0;
    const asked: { at: number; init?: RequestInit }[] = [];
    let busyAsked = 0;
    const served = body(parts, cut);
    const kept = body(1);
    const request = (async (input: RequestInfo | URL, init?: RequestInit) => {
      expect(typeof input === "string" ? input : input instanceof URL ? input.href : input.url).toBe(URL_);
      asked.push({ at: now, ...(init ? { init } : {}) });
      if (init?.cache === "only-if-cached") {
        if (cacheThrows) throw new TypeError("no only-if-cached");
        return cached ? new Response(kept.stream) : new Response(null, { status: 504 });
      }
      if (answer === "throws") throw new TypeError("offline");
      if (answer === "500") return new Response("no", { status: 500 });
      if (answer === "nobody") return new Response(null, { status: 200 });
      return new Response(served.stream);
    }) as typeof fetch;
    const got: Kept = await keepServerWasm(URL_, {
      request,
      sleep: async (ms) => void (now += ms),
      busy: () => busyAsked++ < busyFor,
    });
    const waited = KEEP_AFTER_MS + Math.min(busyFor, 120) * KEEP_BUSY_MS;
    expect(asked[0]).toMatchObject({ at: waited, init: { cache: "only-if-cached", mode: "same-origin" } });
    if (cached) {
      expect(got).toBe("there");
      expect(asked).toHaveLength(1);
      expect(kept.seen.read).toBeLessThanOrEqual(1);
      continue;
    }
    expect(asked).toHaveLength(2);
    expect(asked[1]).toMatchObject({ at: waited, init: { priority: "low" } });
    const whole = answer === "ok" && cut === undefined;
    expect(got).toBe(whole ? "kept" : "failed");
    if (whole) expect(served.seen.read).toBe(parts);
  }
});

test("waits KEEP_AFTER_MS with no deps given but the request", async () => {
  const at: number[] = [];
  const t0 = Date.now();
  const got = await keepServerWasm(URL_, {
    afterMs: 30,
    request: (async () => {
      at.push(Date.now() - t0);
      return new Response("x");
    }) as typeof fetch,
  });
  expect(got).toBe("there");
  expect(at[0]).toBeGreaterThanOrEqual(25);
});
