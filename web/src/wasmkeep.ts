// The browser keeps the compiled code of a wasm module only with the HTTP cache's entry of the
// URL it was streamed from (WebAssembly.compileStreaming / instantiateStreaming). sake's wasm
// from the device or the bucket (wasmsource.ts 2 and 3) is bytes, compiled anew on every start:
// in Chromium ~1.5 s of TurboFan and ~0.25 s of Liftoff on the CPU each time (a phone: more),
// where a start from the server's file after the first gets its code back (TurboFan ~0.1 s, the
// title ~0.3 s sooner; docs/bench.md). Not from Cache Storage either (measured: as the bytes).
//
// So after a start from bytes the server's versioned file is fetched once, in the background,
// late and at low priority (zstd-coded, ~6 MB), and read to its end: the next start finds it
// there (wasmsource.ts 1), streams it, and the browser keeps the code with it from then on.
// (tests/wasm-keep-browser.mjs runs keepServerWasm in a page as it is: nothing else of here in it.)

export interface KeepDeps {
  request?: typeof fetch;
  /** waits `ms` (default: a timer) */
  sleep?: (ms: number) => Promise<void>;
  /** the game's reads wait on the network now (sake's _vnhost_net_busy): later */
  busy?: () => boolean;
  /** how long after the start (default KEEP_AFTER_MS) */
  afterMs?: number;
}

/** How long a start from bytes runs before the server's file is fetched (the game first). */
export const KEEP_AFTER_MS = 20000;
/** How often a busy game is asked again. */
export const KEEP_BUSY_MS = 1000;

/** What came of it: kept (fetched to its end), there (the HTTP cache had it), failed. */
export type Kept = "kept" | "there" | "failed";

/** The server's file `url` into the browser's HTTP cache (see above). Never throws. */
export async function keepServerWasm(url: string, deps: KeepDeps = {}): Promise<Kept> {
  const request = deps.request ?? ((i, n) => fetch(i, n));
  const sleep = deps.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
  await sleep(deps.afterMs ?? KEEP_AFTER_MS);
  for (let n = 0; deps.busy?.() && n < 120; n++) await sleep(KEEP_BUSY_MS);
  try {
    const r = await request(url, { cache: "only-if-cached", mode: "same-origin" });
    if (r.ok) {
      await r.body?.cancel().catch(() => {});
      return "there";
    }
  } catch {
    // (not kept, or a browser without only-if-cached)
  }
  try {
    const r = await request(url, { priority: "low" });
    if (!r.ok || !r.body) {
      await r.body?.cancel().catch(() => {});
      return "failed";
    }
    // (read to its end, nothing kept here: an entry the cache has only once it is whole)
    const reader = r.body.getReader();
    for (;;) if ((await reader.read()).done) break;
    return "kept";
  } catch {
    return "failed";
  }
}
