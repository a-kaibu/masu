import { expect, test } from "vite-plus/test";
import { MAGIC, stateHeader } from "./savestate";

// A state file starts with 8 bytes of magic, read back as all 8: a 7-letter magic (the masu
// rename's "MASUSNP") made every state "not a state" under node and on the page.

/** A state file's head: magic, version, header length, the header. */
function head(magic: string, version: number, header: object): Uint8Array {
  const h = new TextEncoder().encode(JSON.stringify(header));
  const f = new Uint8Array(16 + h.length);
  const v = new DataView(f.buffer);
  for (let i = 0; i < 8; i++) f[i] = magic.charCodeAt(i) || 0;
  v.setUint32(8, version, true);
  v.setUint32(12, h.length, true);
  f.set(h, 16);
  return f;
}

test("the magic is 8 bytes, each one a letter", () => {
  expect(MAGIC.length).toBe(8);
  expect(MAGIC).toMatch(/^[A-Z]{8}$/);
});

test("a state's header reads back (property: any build, pages, ctl)", () => {
  let x = 7;
  const rnd = (n: number) => ((x = (Math.imul(x, 1103515245) + 12345) >>> 0) >>> 8) % n;
  for (let k = 0; k < 200; k++) {
    const h = {
      format: 1,
      build: rnd(1 << 24).toString(16),
      wasm: rnd(2) ? "sake" : "sake-grow",
      pages: 1 + rnd(65535),
      ctl: rnd(1 << 30),
    };
    const f = head(MAGIC, 1, h);
    const got = stateHeader(f);
    expect(got).toEqual({ build: h.build, wasm: h.wasm, pages: h.pages, ctl: h.ctl, at: f.length });
  }
});

test("a file with another magic, a shorter one or another version is none", () => {
  const h = { format: 1, build: "b", ctl: 1 };
  for (const m of [MAGIC.slice(0, 7), "VNVMSNAP", MAGIC.toLowerCase()])
    expect(() => stateHeader(head(m, 1, h))).toThrow();
  expect(() => stateHeader(head(MAGIC, 2, h))).toThrow();
});

test("node's states (host/snap.js) have the page's magic", async () => {
  // (node's fs, out of the web types' sight)
  const { readFileSync } = (await import(/* @vite-ignore */ ["node", "fs"].join(":"))) as {
    readFileSync: (u: URL, e: string) => string;
  };
  const js = readFileSync(new URL("../../host/snap.js", import.meta.url), "utf8");
  expect(/MAGIC: '([^']*)'/.exec(js)?.[1]).toBe(MAGIC);
});

// ---- the stop's protocol (crates/kura/src/snap.rs Ctl: req, serving, cancel)

/** A module whose coordinator follows snap.rs's protocol in JS: takes CMD with its REQ, says
 * whose it is (SERVING after PAUSING), stands still only once `ready` says so, ends a request
 * the page cancels wherever it is (FAILED; from READY: RUNNING), RESUME after READY. */
function coordinator() {
  const memory = new WebAssembly.Memory({ initial: 1, maximum: 1, shared: true });
  const i32 = new Int32Array(memory.buffer);
  const CTL = 4096;
  const w = (off: number) => (CTL + off) >>> 2;
  const st = { ready: false, paused: false, served: [] as number[] };
  let serving = 0;
  const step = () => {
    const cmd = Atomics.load(i32, w(8));
    const state = Atomics.load(i32, w(12));
    if (state === 0 || state === 3) {
      if (cmd === 1 || cmd === 3) {
        Atomics.store(i32, w(8), 0);
        serving = Atomics.load(i32, w(96));
        Atomics.store(i32, w(12), 1);
        Atomics.store(i32, w(100), serving);
        st.served.push(serving);
        st.paused = true;
      }
      return;
    }
    const cancelled = serving !== 0 && Atomics.load(i32, w(104)) === serving;
    if (state === 1) {
      if (cancelled) {
        st.paused = false;
        Atomics.store(i32, w(12), 3);
      } else if (st.ready) Atomics.store(i32, w(12), 2);
    } else if (state === 2 && (cancelled || cmd === 2)) {
      if (cmd === 2) Atomics.store(i32, w(8), 0);
      st.paused = false;
      Atomics.store(i32, w(12), 0);
    }
    Atomics.notify(i32, w(12));
  };
  const timer = setInterval(step, 1);
  const m = {
    wasmMemory: memory,
    _malloc: () => 0,
    _free: () => {},
    _vnhost_snap_ctl: () => CTL,
    _vnhost_render_shot: () => 0,
  };
  return { m, st, i32, w, stop: () => clearInterval(timer) };
}

test("a stop the coordinator does not finish in time is cancelled: the threads go again, and the next stop is answered as itself", async () => {
  const { stopAll } = await import("./savestate");
  const c = coordinator();
  try {
    // (the threads never stand still: given up, cancelled, the coordinator lets them go)
    await expect(stopAll(c.m, true, 60)).rejects.toThrow();
    const first = c.st.served[0]!;
    expect(Atomics.load(c.i32, c.w(104))).toBe(first);
    await expect.poll(() => c.st.paused).toBe(false);
    // (the next: its own number; the FAILED of the one given up is not its answer)
    c.st.ready = true;
    const s = await stopAll(c.m, true, 2000);
    expect(c.st.served).toHaveLength(2);
    expect(c.st.served[1]).not.toBe(first);
    expect(Atomics.load(c.i32, c.w(100))).toBe(c.st.served[1]);
    expect(c.st.paused).toBe(true);
    await s.resume();
    expect(c.st.paused).toBe(false);
  } finally {
    c.stop();
  }
});
