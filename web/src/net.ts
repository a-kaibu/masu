// The page's requests on a weak or unstable link (mobile): the same policy as the game's own
// reads (host/fetch-bridge.js, the network worker; that runs as a plain script inside sake.js, so
// it has its own copy): retry dropped connections, stalled bodies and 5xx/408/429 with backoff,
// wait while the browser is offline, fail at once for other statuses. Store chunks come only
// from the bucket (direct.ts) and are kept in OPFS (chunkstore.ts, shared with the worker):
// content-addressed, so never stale; only chunks checked against their name (BLAKE3) are kept.

import { drain, gate } from "./budget";
import { deviceChunks, listChunks, type ChunkStore } from "./chunkstore";
import { directFetcher } from "./direct";

const RETRY = new Set([0, 408, 425, 429, 500, 502, 503, 504]);
/** A kept chunk's length (trimChunks). */
const SIZE = "x-masu-size";

export interface RetryOptions {
  /** no byte for this long: abort and try again (ms) */
  stallMs?: number;
  /** give up after this long (ms) */
  giveUpMs?: number;
  /** stop retrying (the request is aborted too) */
  signal?: AbortSignal;
  request?: typeof fetch;
  /** a request of the bucket: its budget class (budget.ts), taken before every try (a retry is
   * one more request), and a 429 stops every class for its wait */
  budget?: number;
}

/** Waits `ms`; while the browser reports no network, until it reports one again (at most 30 s). */
async function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  const wait = (t: number) =>
    new Promise<void>((resolve) => {
      const done = () => {
        clearTimeout(timer);
        signal?.removeEventListener("abort", done);
        resolve();
      };
      // (already aborted: no wait, as its listener would never run)
      if (signal?.aborted) return resolve();
      const timer = setTimeout(done, t);
      signal?.addEventListener("abort", done);
    });
  await wait(ms);
  const offline = () => typeof navigator !== "undefined" && navigator.onLine === false;
  for (let i = 0; i < 60 && offline() && !signal?.aborted; i++) await wait(500);
}

/** The delay before retry `attempt` (from 0): exponential with jitter, at most 8 s. */
export function backoff(attempt: number): number {
  return Math.min(8000, 300 * 2 ** attempt) * (0.5 + Math.random() * 0.5);
}

/** One try with the whole body read; aborted when no byte arrives for `stallMs`. */
async function once(
  url: string,
  init: RequestInit,
  stallMs: number,
  request: typeof fetch,
): Promise<{ response: Response; data?: Uint8Array<ArrayBuffer> }> {
  const abort = new AbortController();
  const outer = init.signal;
  const forward = () => abort.abort(outer?.reason);
  outer?.addEventListener("abort", forward);
  let last = Date.now();
  const watch = setInterval(
    () => Date.now() - last > stallMs && abort.abort(new Error("stalled")),
    Math.min(1000, stallMs / 4),
  );
  try {
    // (an "abort" listener on a signal already aborted never runs: the request would go)
    outer?.throwIfAborted();
    const response = await request(url, { ...init, signal: abort.signal });
    last = Date.now();
    if (!response.ok) {
      void response.body?.cancel().catch(() => undefined);
      return { response };
    }
    if (!response.body) return { response, data: new Uint8Array(await response.arrayBuffer()) };
    const reader = response.body.getReader();
    const parts: Uint8Array[] = [];
    let length = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      last = Date.now();
      parts.push(value);
      length += value.length;
    }
    const data = new Uint8Array(length);
    let at = 0;
    for (const p of parts) {
      data.set(p, at);
      at += p.length;
    }
    return { response, data };
  } finally {
    clearInterval(watch);
    outer?.removeEventListener("abort", forward);
  }
}

/**
 * `f()`, tried again (with backoff) when it fails other than by an HttpError: for requests that
 * must stay a plain fetch (WebAssembly.compileStreaming keeps the compiled code of its URL).
 */
export async function withRetries<T>(f: () => Promise<T>, tries = 5): Promise<T> {
  for (let attempt = 0; ; attempt++) {
    try {
      return await f();
    } catch (e) {
      if (e instanceof HttpError || attempt + 1 >= tries) throw e;
      await sleep(backoff(attempt));
    }
  }
}

/** HTTP failure that retrying will not fix (404 and the like). */
export class HttpError extends Error {
  constructor(
    readonly status: number,
    /** offline, and the file is not on this device (sw.ts) */
    readonly offline = false,
  ) {
    super(offline ? "オフラインで、端末に保存されていません" : `HTTP ${status}`);
  }
}

/** How long a request the start screen waits on is retried (a tree's list): a server that
 * answers 503 showed "loading" for the default 15 minutes before the error. */
export const BOOT_GIVE_UP_MS = 60_000;

/** fetchRetry for what the start screen waits on: given up after BOOT_GIVE_UP_MS. */
export const fetchBoot = (url: string | URL, opt: RetryOptions = {}) =>
  fetchRetry(url, {}, { giveUpMs: BOOT_GIVE_UP_MS, ...opt });

/** `url`'s body, retried as above. Throws HttpError, or the last error after `giveUpMs`. */
export async function fetchRetry(
  url: string | URL,
  init: RequestInit = {},
  opt: RetryOptions = {},
): Promise<Uint8Array<ArrayBuffer>> {
  const request = opt.request ?? ((i, n) => fetch(i, n));
  const stallMs = opt.stallMs ?? 20000;
  const giveUpMs = opt.giveUpMs ?? 15 * 60000;
  const start = Date.now();
  let retrying = false;
  try {
    for (let attempt = 0; ; attempt++) {
      let error: unknown;
      let after = 0;
      try {
        // (an ask cancelled before this try, or while it waited for the budget, sends nothing
        // and takes no token: review 22bb061 §6)
        opt.signal?.throwIfAborted();
        if (opt.budget !== undefined) await gate(opt.budget, opt.signal);
        const { response, data } = await once(String(url), { ...init, signal: opt.signal }, stallMs, request);
        if (data) return data;
        // (sw.ts: offline, and not kept on the device)
        if (response.headers.has("x-masu-offline")) throw new HttpError(response.status, true);
        if (!RETRY.has(response.status)) throw new HttpError(response.status);
        error = new Error(`HTTP ${response.status}`);
        after = Math.min((Number(response.headers.get("retry-after")) || 0) * 1000, 30000);
        if (response.status === 429 && opt.budget !== undefined) drain(after || 5000);
      } catch (e) {
        if (e instanceof HttpError) throw e;
        // (cancelled: the caller's reason, not what the cut try failed with)
        if (opt.signal?.aborted) throw opt.signal.reason;
        error = e;
      }
      if (Date.now() - start > giveUpMs) throw error;
      if (!retrying) {
        retrying = true;
        page.retrying++;
      }
      page.report(String(error instanceof Error ? error.message : error));
      await sleep(after || backoff(attempt), opt.signal);
      if (opt.signal?.aborted) throw opt.signal.reason;
    }
  } finally {
    if (retrying) {
      page.retrying--;
      page.report("");
    }
  }
}

/** The page's own requests being retried, for watchRetries. */
const page = {
  retrying: 0,
  listeners: new Set<(error: string) => void>(),
  report(error: string) {
    for (const l of this.listeners) l(error);
  },
};

/** The device's chunk store (chunkstore.ts: OPFS; null where there is none). */
export function chunkCache(): Promise<ChunkStore | null> {
  return deviceChunks();
}

export const chunkPath = (hex: string) => `chunks/${hex.slice(0, 2)}/${hex}`;

/**
 * Store chunk `hex` (decoded): from the device, else from the bucket (direct.ts; kept on the
 * device). `cached`: whether it came without the network. Throws when the bucket cannot give it.
 */
export async function getChunk(
  store: string,
  hex: string,
  opt: RetryOptions & { cache?: ChunkStore | null } = {},
): Promise<{ data: Uint8Array<ArrayBuffer>; cached: boolean }> {
  const url = new URL(chunkPath(hex), store).href;
  const cache = opt.cache === undefined ? await chunkCache() : opt.cache;
  const hit = await cache?.match(url).catch(() => undefined);
  if (hit) return { data: new Uint8Array(await hit.arrayBuffer()), cached: true };
  const data = await directFetcher(store, {
    request: opt.request,
    ...(opt.budget === undefined ? {} : { cls: opt.budget }),
  }).get(hex, opt);
  if (!data) throw new Error(`chunk ${hex}: the bucket did not give it`);
  if (cache) await keepChunk(cache, url, data);
  return { data, cached: false };
}

/** Stores chunk `data` (checked against its name) at `url` in `cache`; rejects when it could
 * not be (the device full: QuotaExceededError, isFull). */
export function storeChunk(cache: ChunkStore, url: string, data: Uint8Array<ArrayBuffer>): Promise<void> {
  return cache.put(url, new Response(data, { headers: { [SIZE]: String(data.length) } }));
}

/** Keeps chunk `data` at `url` in `cache` if it can, as getChunk does (a cache, not a promise:
 * a chunk not kept is fetched again). */
export function keepChunk(cache: ChunkStore, url: string, data: Uint8Array<ArrayBuffer>): Promise<void> {
  return storeChunk(cache, url, data).catch(() => undefined);
}

/** Whether `e` says the device (the origin's quota) has no room left. */
export function isFull(e: unknown): boolean {
  const name = (e as { name?: unknown } | null)?.name;
  return name === "QuotaExceededError" || (e instanceof Error && /quota/i.test(e.message));
}

/** Bytes of chunks kept at most: the newest are kept. */
// (2 << 30 is negative in JS: until 2026-11-07 every trim emptied the cache of all but the
// saved games' chunks)
const KEEP_BYTES = 2 * 1024 ** 3;

/**
 * Keeps the chunk cache within its share of the origin's storage: half the quota and 2 GB, and
 * with what else the origin keeps (saves and states in OPFS, the app) the whole under SHARE of the
 * quota, those never going. The oldest chunks go first, but not those `saved()` lists (games saved
 * to the device: offline.ts savedChunks; only asked when something must go). Sizes are the
 * chunks' own (x-masu-size; FastCDC chunks are 4 MiB on average, 64 MiB at most). Also asks the
 * browser to keep this origin's data under storage pressure.
 */
export async function trimChunks(
  saved: () => Promise<Set<string>> = async () => new Set(),
  store?: ChunkStore | null,
): Promise<void> {
  void navigator.storage?.persist?.().catch(() => false);
  const cache = store === undefined ? await chunkCache() : store;
  if (!cache) return;
  const { quota = 0, usage = 0 } = (await navigator.storage?.estimate?.().catch(() => undefined)) ?? {};
  // (listed with sizes and times a few files at a time, not every chunk opened at once)
  const kept = await listChunks(cache);
  const chunks = kept.reduce((a, k) => a + k.size, 0);
  let limit = Math.min(KEEP_BYTES, quota ? quota * 0.5 : KEEP_BYTES);
  // (usage counts the chunks too, as the browser last measured)
  if (quota && usage) limit = Math.min(limit, quota * SHARE - Math.max(0, usage - chunks));
  let over = chunks - limit;
  if (over <= 0) return;
  const keep = await saved().catch(() => null);
  // (the saved games' lists out of reach: nothing goes rather than their chunks)
  if (!keep) return;
  for (const k of trimOrder(kept)) {
    if (over <= 0) break;
    if (keep.has(chunkName(k.url))) continue;
    // (one that stayed, open in the network worker for a read: still counted)
    if (await cache.delete(k).catch(() => false)) over -= k.size;
  }
}
/** A kept chunk's name (its hex). */
const chunkName = (url: string) => url.slice(url.lastIndexOf("/") + 1);

/** The order chunks go in at a trim: the oldest written first (OPFS lists by name, not age). */
export function trimOrder<T extends { time: number }>(kept: readonly T[]): T[] {
  return kept
    .map((k, i) => [k, i] as const)
    .sort((a, b) => a[0].time - b[0].time || a[1] - b[1])
    .map(([k]) => k);
}
/** Of the quota, what the origin may fill in all before chunks go (trimChunks). */
const SHARE = 0.8;

/** The store chunk hashes (hex) of a tree's file list (trees/<name>.tsv, crates/masu/src/pack.rs). */
export function treeChunks(tsv: string): string[] {
  const out: string[] = [];
  for (const line of tsv.split("\n").slice(1)) {
    const f = line.split("\t");
    if (f[0] !== "f" || !f[5]) continue;
    // (<hash>:<length>, ":<length>" all zero)
    for (const c of f[5].split(",")) {
      const h = c.split(":")[0]!;
      if (/^[0-9a-f]{32}$/.test(h)) out.push(h);
    }
  }
  return out;
}

/** The store chunks' lengths (hex -> bytes) of a tree's file list (as treeChunks). */
export function treeChunkSizes(tsv: string, into = new Map<string, number>()): Map<string, number> {
  for (const line of tsv.split("\n").slice(1)) {
    const f = line.split("\t");
    if (f[0] !== "f" || !f[5]) continue;
    for (const c of f[5].split(",")) {
      const [h, n] = c.split(":");
      if (h && /^[0-9a-f]{32}$/.test(h) && Number(n) > 0) into.set(h, Number(n));
    }
  }
  return into;
}

/** How many of the page's and the game's requests are being retried now (BroadcastChannel). */
export function watchRetries(on: (retrying: number, error: string) => void): () => void {
  const by = new Map<string, number>();
  const tell = (error: string) => on(page.retrying + [...by.values()].reduce((a, b) => a + b, 0), error);
  page.listeners.add(tell);
  const ch = typeof BroadcastChannel === "undefined" ? null : new BroadcastChannel("masu-net");
  if (ch)
    ch.onmessage = (e: MessageEvent<{ id: string; retrying: number; error: string }>) => {
      by.set(e.data.id, e.data.retrying);
      tell(e.data.error);
    };
  return () => {
    page.listeners.delete(tell);
    ch?.close();
  };
}
