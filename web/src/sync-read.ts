// NW.js games read their own files synchronously (Node's fs.readFileSync) from the page. A
// synchronous XMLHttpRequest does not go through the frame origin's service worker (game-sw.ts)
// in Chromium, and the server has no game files. So the frame has a worker of its own
// (frame-read.ts, /frame-read.js) ask the app's tree worker for the file over a MessagePort the
// app hands it with the saves (gameport.ts ReadAsk; not through the service worker: one stopped
// and started again asks the frame's main thread for a port, the thread that waits) and hand
// it over in shared memory while the page waits (a window's main thread may not Atomics.wait,
// so it spins). The asks go in shared memory too: a message posted to the worker while the
// page spins was, at times, not delivered until the spin ended. Files are read when asked, not
// sent ahead: a 950 MB RPG Maker MV folder sent ahead took the tab past 8 GB.
//
//   page --{port, ctl, data}--> worker, once (it answers "ready")
//   then each read: the page writes the URL into ctl, bumps ctl[SEQ] (Atomics.notify); the
//   worker (Atomics.waitAsync) reads it, grows `data` to the file's size, copies it in and sets
//   ctl[STATE] = DONE (status, size beside it) or FAILED

/** ctl's Int32 slots */
export const STATE = 0;
export const STATUS = 1;
export const SIZE = 2;
export const SEQ = 3;
export const URL_LEN = 4;
/** Sequence number of the reply currently in data. */
export const REPLY_SEQ = 5;
/** where the URL's UTF-8 starts in ctl, and its room */
export const URL_AT = 32;
export const URL_MAX = 8192;
/** ctl[STATE] */
export const WAIT = 0;
export const DONE = 1;
export const FAILED = 2;
/** How long a read may take (a chunk from the bucket, with its retries). */
export const SYNC_MS = 60_000;
/** The largest file read this way (the data buffer's most). */
export const DATA_MAX = 1 << 30;

/** The memory a page and its worker share. */
export interface Shared {
  ctl: SharedArrayBuffer;
  /** growable to its maxByteLength */
  data: SharedArrayBuffer;
}

interface Growable {
  byteLength: number;
  maxByteLength: number;
  grow(n: number): void;
}

export function makeShared(max = DATA_MAX): Shared {
  const Grow = SharedArrayBuffer as unknown as new (
    n: number,
    o: { maxByteLength: number },
  ) => SharedArrayBuffer;
  return { ctl: new SharedArrayBuffer(URL_AT + URL_MAX), data: new Grow(0, { maxByteLength: max }) };
}

/** Spins until ctl[STATE] leaves WAIT (true) or `ms` pass (false). */
function spin(ctl: Int32Array, seq: number, ms: number, now: () => number): boolean {
  const until = now() + ms;
  for (let n = 0; Atomics.load(ctl, REPLY_SEQ) !== seq || Atomics.load(ctl, STATE) === WAIT; n++)
    if ((n & 1023) === 0 && now() > until) return false;
  return true;
}

/**
 * The file at `url`, read synchronously through `shared` (a worker answering it: readServer);
 * null when there is none (or it cannot be read in `ms`).
 */
export function readSync(
  url: string,
  shared: Shared | null = frameShared(),
  ms = SYNC_MS,
  now: () => number = () => performance.now(),
): Uint8Array | null {
  if (!shared) return null;
  const ctl = new Int32Array(shared.ctl);
  const name = new TextEncoder().encode(url);
  if (name.length > URL_MAX) return null;
  new Uint8Array(shared.ctl, URL_AT, URL_MAX).set(name);
  Atomics.store(ctl, URL_LEN, name.length);
  Atomics.store(ctl, STATE, WAIT);
  const seq = (Atomics.add(ctl, SEQ, 1) + 1) | 0;
  Atomics.notify(ctl, SEQ);
  if (!spin(ctl, seq, ms, now) || Atomics.load(ctl, STATE) !== DONE || Atomics.load(ctl, STATUS) !== 200)
    return null;
  // (a copy of its own: TextDecoder and the game take no shared memory)
  return new Uint8Array(shared.data, 0, Atomics.load(ctl, SIZE)).slice();
}

let shared: Shared | null | undefined;
let ready: Promise<void> = Promise.resolve();

/** How long the frame's worker may take to start. */
const READY_MS = 10_000;

/** The frame's worker's memory (null: none, not cross-origin isolated). */
export function frameShared(): Shared | null {
  return shared ?? null;
}

/**
 * Starts the frame's worker over `reads` (the app's tree worker), once; resolves when it runs
 * (a worker starts with the page's main thread's help: the page must not spin on one that has
 * not yet), at once when there is none.
 */
export function frameReady(reads?: MessagePort): Promise<void> {
  if (shared !== undefined) return ready;
  if (
    typeof crossOriginIsolated === "undefined" ||
    !crossOriginIsolated ||
    typeof Worker === "undefined" ||
    !reads
  ) {
    console.error("masu: the frame is not cross-origin isolated: the game's synchronous reads fail");
    shared = null;
    return ready;
  }
  shared = makeShared();
  const w = new Worker("/frame-read.js");
  ready = new Promise<void>((resolve) => {
    w.onmessage = () => resolve();
    setTimeout(resolve, READY_MS);
  });
  w.postMessage({ port: reads, ...shared }, [reads]);
  return ready;
}

type WaitAsync = (a: Int32Array, i: number, v: number) => { async: boolean; value: Promise<string> | string };

/**
 * The worker's side: waits for the page's asks in `s` and answers each with `get` (the file
 * by URL), until `stop` aborts. Only the newest ask is answered (one the page gave up on is not).
 */
export async function readServer(
  s: Shared,
  get: (url: string) => Promise<Response>,
  stop?: AbortSignal,
): Promise<void> {
  const ctl = new Int32Array(s.ctl);
  const wait = (Atomics as unknown as { waitAsync: WaitAsync }).waitAsync;
  let seen = Atomics.load(ctl, SEQ);
  while (!stop?.aborted) {
    if (Atomics.load(ctl, SEQ) === seen) {
      const r = wait(ctl, SEQ, seen);
      if (r.async) await r.value;
      continue;
    }
    seen = Atomics.load(ctl, SEQ);
    const n = Math.min(Atomics.load(ctl, URL_LEN), URL_MAX);
    const url = new TextDecoder().decode(new Uint8Array(s.ctl, URL_AT, n).slice());
    let state = FAILED;
    try {
      const r = await get(url);
      const bytes = r.ok ? new Uint8Array(await r.arrayBuffer()) : new Uint8Array(0);
      const data = s.data as unknown as Growable;
      if (bytes.length <= data.maxByteLength) {
        if (data.byteLength < bytes.length) data.grow(bytes.length);
        new Uint8Array(s.data, 0, bytes.length).set(bytes);
        Atomics.store(ctl, STATUS, r.status);
        Atomics.store(ctl, SIZE, bytes.length);
        state = DONE;
      }
    } catch {
      // (FAILED)
    }
    // (a newer ask meanwhile: the page gave up on this one)
    if (Atomics.load(ctl, SEQ) === seen) {
      Atomics.store(ctl, STATE, state);
      Atomics.store(ctl, REPLY_SEQ, seen);
    }
  }
}

/** The URL of `path` (in the game's folder, "/"-separated) under the folder's URL `base`. */
export const fileUrl = (base: string, path: string) =>
  base + path.split("/").map(encodeURIComponent).join("/");
