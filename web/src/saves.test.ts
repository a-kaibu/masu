import { simulate, deferred } from "./test/simulation";
import { exclusiveLocks } from "./test/browser";
import { afterAll, beforeAll, expect, test, vi } from "vite-plus/test";
import {
  keepSaving,
  loadSaves,
  savesFor,
  stashesToDrop,
  STASH_KEEP,
  storeSaves,
  UPLOAD_EVERY,
  type SaveFiles,
  type SavesModule,
} from "./saves";

// The saved-game blobs across pages and devices (saves.ts): a model of masu serve's /saves
// (crates/masu/src/saves.rs: a revision per name, If-Match, its revisions kept) and of each
// device's OPFS; a "page" is a fresh transport over the same files.

class Server {
  rev = 0;
  data: Uint8Array | null = null;
  history: Uint8Array[] = [];
  up = true;
  request: typeof fetch = async (_url, init) => {
    if (!this.up || !navigator.onLine) throw new TypeError("Failed to fetch");
    if (init?.method === "PUT") {
      const want = Number(new Headers(init.headers).get("if-match")?.replaceAll('"', ""));
      if (want !== this.rev) return new Response(null, { status: 412 });
      this.data = new Uint8Array(init.body as Uint8Array);
      this.history.push(this.data);
      this.rev++;
      return new Response(null, { status: 204, headers: { etag: `"${this.rev}"` } });
    }
    if (!this.data) return new Response(null, { status: 404 });
    return new Response(this.data.slice(), { headers: { etag: `"${this.rev}"` } });
  };
}

class Device {
  m = new Map<string, Uint8Array>();
  files: SaveFiles = {
    read: async (f) => {
      const v = this.m.get(f);
      return v ? (v.slice() as Uint8Array<ArrayBuffer>) : null;
    },
    write: async (f, d) => void this.m.set(f, d.slice()),
  };
  current() {
    const meta = this.m.get("game.v2.sync.json");
    const slot = meta && JSON.parse(new TextDecoder().decode(meta)).slot;
    return meta && this.m.get(`game.v2.${slot === 1 ? 1 : 0}.bin`);
  }
  /** what this browser keeps of the game: its blob and anything kept aside */
  kept(): Uint8Array[] {
    const current = this.current();
    const stashes = [...this.m.entries()].filter(([key]) => /^game\.(local|server)-\d+\.bin$/.test(key));
    return [...(current ? [current] : []), ...stashes.map(([, value]) => value)];
  }
}

const g = globalThis as unknown as { location?: unknown };
const oldLocation = g.location;
let online = true;
beforeAll(() => {
  g.location = { href: "https://masu.test/play.html" };
  Object.defineProperty(navigator, "onLine", { get: () => online, configurable: true });
  // (the origin's Web Locks, as a browser has them: the tests outside simulate() had none and
  // failed on navigator.locks; the simulation's own browser brings its own)
  if (!("locks" in navigator))
    Object.defineProperty(navigator, "locks", { value: exclusiveLocks(), configurable: true });
});
afterAll(() => {
  g.location = oldLocation;
});

const blob = (n: number) => new Uint8Array([n & 255, n >> 8]);
const eq = (a: Uint8Array | null | undefined, b: Uint8Array | null | undefined) =>
  !!a && !!b && a.length === b.length && a.every((x, i) => x === b[i]);
/** A new page on `d`: the start (loadSaves) and what it returned. */
async function page(d: Device, s: Server, notice?: (m: string) => void) {
  savesFor(d.files, s.request);
  return loadSaves("game", notice);
}
const save = (n: number) =>
  storeSaves("game", blob(n)).then(
    () => true,
    () => false,
  );

test("a save made offline is not lost to the server's older copy at the next online start", async () => {
  const s = new Server();
  const phone = new Device();
  online = true;
  await page(phone, s);
  expect(await save(1)).toBe(true);
  expect(eq(s.data, blob(1))).toBe(true);
  // offline: the game saves again, the page closes
  online = false;
  expect(eq(await page(phone, s), blob(1))).toBe(true);
  expect(await save(2)).toBe(false);
  expect(eq(s.data, blob(1))).toBe(true);
  // a later day, online: this device's save 2, and the server gets it
  online = true;
  expect(eq(await page(phone, s), blob(2))).toBe(true);
  expect(eq(s.data, blob(2))).toBe(true);
  // and the start after that is the server's, as usual
  const pc = new Device();
  expect(eq(await page(pc, s), blob(2))).toBe(true);
});

test("a page that closed before its upload was acknowledged sends its save at the next start", async () => {
  const s = new Server();
  const phone = new Device();
  online = true;
  await page(phone, s);
  await save(1);
  // the server takes nothing more (the page is closing: its PUT never lands)
  s.up = false;
  expect(await save(2)).toBe(false);
  s.up = true;
  expect(eq(await page(phone, s), blob(2))).toBe(true);
  expect(eq(s.data, blob(2))).toBe(true);
});

test("saved here offline and on another device meanwhile: this device's wins, the other's is kept", async () => {
  const s = new Server();
  const phone = new Device();
  const pc = new Device();
  online = true;
  await page(phone, s);
  await save(1);
  online = false;
  await page(phone, s);
  await save(2);
  online = true;
  await page(pc, s);
  await save(3);
  const notices: string[] = [];
  expect(eq(await page(phone, s, (m) => notices.push(m)), blob(2))).toBe(true);
  expect(notices.length).toBe(1);
  expect(eq(s.data, blob(2))).toBe(true);
  // the other device's save 3: on this device beside the blob, and in the server's revisions
  expect(phone.kept().some((b) => eq(b, blob(3)))).toBe(true);
  expect(s.history.some((b) => eq(b, blob(3)))).toBe(true);
});

test("a game started offline whose server copy moved meanwhile does not write over it", async () => {
  const s = new Server();
  const phone = new Device();
  const pc = new Device();
  online = true;
  await page(phone, s);
  await save(1);
  // started offline on the phone; the pc saves; the phone's link comes back mid-game
  online = false;
  await page(phone, s);
  online = true;
  savesFor(pc.files, s.request);
  await loadSaves("game");
  await save(5);
  savesFor(phone.files, s.request);
  online = false;
  await loadSaves("game");
  online = true;
  expect(await save(2)).toBe(false);
  expect(eq(s.data, blob(5))).toBe(true);
  // the next start: the phone's save 2 (made after), the pc's 5 kept on the phone
  expect(eq(await page(phone, s), blob(2))).toBe(true);
  expect(phone.kept().some((b) => eq(b, blob(5)))).toBe(true);
});

// Random histories of two devices (pages started on- or offline, saves, uploads lost, the
// server down): the last save made on each device is never lost (it is on the server, in its
// revisions, or on that device), and a device whose last save was not sent, with nobody else
// saving since, starts from that save and sends it.
test("property: no device's last save is ever lost", async () => {
  let seed = 12345;
  const rnd = (n: number) => {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    return seed % n;
  };
  let value = 0;
  for (let run = 0; run < 400; run++) {
    const s = new Server();
    const ds = [new Device(), new Device()];
    const last: (Uint8Array | null)[] = [null, null];
    /** device that saved last, and whether that save is on the server */
    let newest = -1;
    let sent = true;
    for (let step = 0; step < 12; step++) {
      const i = rnd(2);
      const d = ds[i]!;
      online = rnd(3) !== 0;
      s.up = true;
      const got = await page(d, s);
      if (online && newest === i && !sent) {
        expect(eq(got, last[i]), `run ${run} step ${step}: the unsent save`).toBe(true);
        expect(eq(s.data, last[i]), `run ${run} step ${step}: sent`).toBe(true);
        sent = true;
      }
      for (let k = rnd(3); k > 0; k--) {
        if (rnd(4) === 0) s.up = false;
        const v = blob(++value);
        const ok = await save(value);
        last[i] = v;
        newest = i;
        sent = ok && eq(s.data, v);
      }
      for (const [j, v] of last.entries()) {
        if (!v) continue;
        const where = [...s.history, ...ds[j]!.kept()];
        expect(
          where.some((b) => eq(b, v)),
          `run ${run} step ${step}: device ${j}'s last save`,
        ).toBe(true);
      }
    }
  }
  online = true;
});

test("an older page's copy, not known to be sent, is kept aside when the server's is taken", async () => {
  const s = new Server();
  const pc = new Device();
  online = true;
  await page(pc, s);
  await save(7);
  // (a page before saves.ts kept the sync state: a blob and nothing beside it)
  const phone = new Device();
  phone.m.set("game.bin", blob(6));
  expect(eq(await page(phone, s), blob(7))).toBe(true);
  expect(phone.kept().some((b) => eq(b, blob(6)))).toBe(true);
  expect(eq(phone.current(), blob(7))).toBe(true);
});

/** keepSaving over `d` and `s` with a game whose files are `files()` (the export), a clock the
 * test moves, and the 5 s timer and page events left out (the test calls the save itself). */
async function keeper(d: Device, s: Server, files: () => Uint8Array<ArrayBuffer>, last: Uint8Array | null) {
  // (a page's start first: it learns the server's revision, or that it was out of reach)
  await page(d, s);
  let changes = 0;
  let t = 0;
  let reads = 0;
  const m = { _vnhost_vfs_changes: () => changes } as unknown as SavesModule;
  const g = globalThis as unknown as { addEventListener?: unknown };
  const had = g.addEventListener;
  g.addEventListener ??= () => {};
  vi.useFakeTimers();
  const save = keepSaving(m, "game", [], last, {
    now: () => t,
    read: () => (reads++, files()),
  });
  vi.useRealTimers();
  if (!had) delete g.addEventListener;
  let writes = 0;
  const write = d.files.write.bind(d.files);
  d.files.write = async (f, data) => {
    if (/^game\.v2\.[01]\.bin$/.test(f)) writes++;
    return write(f, data);
  };
  return {
    save,
    write: () => void changes++,
    at: (ms: number) => void (t = ms),
    counts: () => ({ reads, writes, puts: s.history.length }),
  };
}

test("offline, a changed save is written to this browser once, and the upload backs off", async () => {
  const s = new Server();
  const d = new Device();
  online = true;
  let current = blob(1);
  const k = await keeper(d, s, () => current as Uint8Array<ArrayBuffer>, null);
  s.up = false;
  current = blob(2);
  k.write();
  await k.save();
  expect(k.counts()).toEqual({ reads: 1, writes: 1, puts: 0 });
  expect(eq(d.current(), blob(2))).toBe(true);
  // (nothing new written by the game, and the next try not due: no export, no write, no request)
  for (let i = 1; i <= 5; i++) {
    k.at(i * 1000);
    await k.save();
  }
  expect(k.counts()).toEqual({ reads: 1, writes: 1, puts: 0 });
  // (due again: the upload only, not this browser's copy again)
  s.up = true;
  k.at(60_000);
  await k.save();
  expect(k.counts()).toEqual({ reads: 1, writes: 1, puts: 1 });
  expect(eq(s.data, blob(2))).toBe(true);
  // (sent: nothing more to do)
  k.at(120_000);
  await k.save();
  expect(k.counts().puts).toBe(1);
});

test("offline, the game writing the same bytes again makes no new local write", async () => {
  const s = new Server();
  const d = new Device();
  online = true;
  s.up = false;
  const k = await keeper(d, s, () => blob(3) as Uint8Array<ArrayBuffer>, blob(1));
  for (let i = 0; i < 4; i++) {
    k.write();
    k.at(i * 10);
    await k.save();
  }
  expect(k.counts()).toEqual({ reads: 4, writes: 1, puts: 0 });
  s.up = true;
});

test("a forced save (the game quits) uploads at once, even in the pause after a failure", async () => {
  const s = new Server();
  const d = new Device();
  online = true;
  s.up = false;
  const k = await keeper(d, s, () => blob(4) as Uint8Array<ArrayBuffer>, blob(1));
  k.write();
  await k.save();
  s.up = true;
  k.at(1);
  await k.save();
  expect(k.counts().puts).toBe(0);
  await k.save(true);
  expect(k.counts().puts).toBe(1);
  expect(eq(s.data, blob(4))).toBe(true);
});

test("saves asked for while one is stuck wait as one, not one per tick", async () => {
  const s = new Server();
  const d = new Device();
  online = true;
  let release = () => {};
  const stuck = new Promise<void>((r) => (release = r));
  const request = s.request;
  s.request = async (u, init) => {
    if (init?.method === "PUT") await stuck;
    return request(u, init);
  };
  let current = blob(5);
  const k = await keeper(d, s, () => current as Uint8Array<ArrayBuffer>, blob(1));
  k.write();
  const first = k.save();
  const waiting = Array.from({ length: 50 }, () => k.save());
  // (all the waiting ones are the same promise)
  expect(new Set(waiting).size).toBe(1);
  current = blob(6);
  k.write();
  release();
  await Promise.all([first, ...waiting]);
  expect(k.counts().reads).toBe(2);
  // (the second change within a minute of the upload: on this device now, sent when due)
  expect(eq(s.data, blob(5))).toBe(true);
  expect(eq(d.current(), blob(6))).toBe(true);
  k.at(UPLOAD_EVERY);
  await k.save();
  expect(eq(s.data, blob(6))).toBe(true);
});

/**
 * Random plays: the game writes (new bytes or the same), the clock moves, the server goes down
 * and up, the page is hidden (a forced save). At every point this browser's copy holds the
 * newest save; uploads that were not forced are UPLOAD_EVERY apart at least; a forced save with
 * the server up leaves the newest on the server; nothing is uploaded twice in a row
 * (property, seeded).
 */
test("property: uploads at most once a minute unless forced, the newest always on the device", async () => {
  let x = 99;
  const rnd = (n: number) => ((x = (Math.imul(x, 1103515245) + 12345) >>> 0) >>> 8) % n;
  for (let run = 0; run < 150; run++) {
    const s = new Server();
    const d = new Device();
    online = true;
    let v = 1;
    let current = blob(v);
    const k = await keeper(d, s, () => current as Uint8Array<ArrayBuffer>, blob(v));
    let t = 0;
    let lastPlain = -Infinity;
    for (let step = 0; step < 40; step++) {
      t += rnd(30_000);
      k.at(t);
      const op = rnd(6);
      if (op < 3) {
        if (rnd(4)) current = blob(++v);
        k.write();
      } else if (op === 3) s.up = !s.up;
      const force = rnd(8) === 0;
      const before = s.history.length;
      await k.save(force);
      if (s.history.length > before) {
        expect(s.history.length - before, `run ${run} step ${step}`).toBe(1);
        if (!force) {
          expect(t - lastPlain, `run ${run} step ${step}: plain uploads apart`).toBeGreaterThanOrEqual(
            UPLOAD_EVERY,
          );
        }
        lastPlain = t;
        const h = s.history;
        if (h.length > 1)
          expect(eq(h.at(-1), h.at(-2)), `run ${run} step ${step}: the same twice`).toBe(false);
      }
      expect(eq(d.current() ?? blob(1), current), `run ${run} step ${step}: on the device`).toBe(true);
      if (force && s.up) expect(eq(s.data ?? blob(1), current), `run ${run} step ${step}: forced`).toBe(true);
    }
    s.up = true;
  }
});

test("copies set aside: only the newest few of a game stay, other games' and the blob untouched (property, seeded)", async () => {
  let x = 17;
  const rnd = (n: number) => ((x = (Math.imul(x, 1103515245) + 12345) >>> 0) >>> 8) % n;
  for (let run = 0; run < 300; run++) {
    const names = ["game", "game.x", "other"];
    const all: string[] = ["game.bin", "game.sync.json", "game.x.bin", "gamer.server-5.bin"];
    for (let i = rnd(10); i > 0; i--)
      all.push(`${names[rnd(3)]}.${rnd(2) ? "server" : "local"}-${rnd(1e6)}.bin`);
    const drop = stashesToDrop("game", all);
    const mine = all.filter((f) => /^game\.(server|local)-\d+\.bin$/.test(f));
    expect(drop.length).toBe(Math.max(0, mine.length - STASH_KEEP));
    const time = (f: string) => Number(/-(\d+)\.bin$/.exec(f)![1]);
    const kept = mine.filter((f) => !drop.includes(f));
    for (const d of drop) {
      expect(mine).toContain(d);
      for (const k of kept) expect(time(k)).toBeGreaterThanOrEqual(time(d));
    }
  }
  // through stash: a server copy set aside 5 times leaves 3
  const s = new Server();
  const d = new Device();
  d.files.list = async () => [...d.m.keys()];
  d.files.remove = async (f) => void d.m.delete(f);
  online = true;
  const real = Date.now;
  let t = 1000;
  Date.now = () => t;
  try {
    for (let i = 0; i < 5; i++) {
      t += 10;
      // (saved here offline, the server's moved meanwhile: the server's is set aside)
      savesFor(d.files, s.request);
      s.data = blob(100 + i);
      s.rev++;
      d.m.set("game.v2.0.bin", blob(200 + i));
      d.m.set(
        "game.v2.sync.json",
        new TextEncoder().encode(
          JSON.stringify({ base: '"0"', dirty: true, slot: 0, generation: `g${i}`, rev: i }),
        ),
      );
      await loadSaves("game");
    }
  } finally {
    Date.now = real;
  }
  expect([...d.m.keys()].filter((f) => f.includes(".server-")).length).toBe(STASH_KEEP);
});

test("another device's newer save stops the uploads: the player is told once", async () => {
  const s = new Server();
  const d = new Device();
  online = true;
  let current = blob(1);
  await page(d, s);
  const notices: string[] = [];
  let changes = 0;
  let t = 0;
  const m = { _vnhost_vfs_changes: () => changes } as unknown as SavesModule;
  const g = globalThis as unknown as { addEventListener?: unknown };
  const had = g.addEventListener;
  g.addEventListener ??= () => {};
  vi.useFakeTimers();
  const save = keepSaving(m, "game", [], null, {
    now: () => t,
    read: () => current as Uint8Array<ArrayBuffer>,
    notice: (n) => notices.push(n),
  });
  vi.useRealTimers();
  if (!had) delete g.addEventListener;
  // (another device saved meanwhile)
  s.data = blob(50);
  s.rev++;
  for (let i = 0; i < 4; i++) {
    current = blob(2 + i);
    changes++;
    t += UPLOAD_EVERY * 10;
    await save(true);
  }
  expect(notices.length).toBe(1);
  expect(eq(s.data, blob(50))).toBe(true);
  expect(eq(d.current(), blob(5))).toBe(true);
});

test("an upload pause followed by a revert persists the last save across restart", async () => {
  const s = new Server();
  const d = new Device();
  let data = blob(1);
  const k = await keeper(d, s, () => data, null);
  await k.save();
  data = blob(2);
  k.write();
  await k.save();
  data = blob(1);
  k.write();
  await k.save();
  expect(eq(await page(d, s), blob(1))).toBe(true);
  expect(eq(s.data, blob(1))).toBe(true);
});

test.each(["poll", "hide then poll", "poll then hide"])(
  "simulation: slow save acknowledgement with %s",
  async (order) => {
    await simulate(async ({ advance, browser }) => {
      const server = new Server();
      const device = new Device();
      const accepted = deferred<void>();
      const ack = deferred<void>();
      const request = server.request;
      server.request = async (url, init) => {
        const response = await request(url, init);
        if (init?.method === "PUT" && server.rev === 1) {
          accepted.resolve();
          await ack.promise;
        }
        return response;
      };

      await page(device, server);
      let version = 1;
      const game = { _vnhost_vfs_changes: () => version } as unknown as SavesModule;
      const save = keepSaving(game, "game", [], null, { read: () => blob(version) });
      const first = save();
      await accepted.promise;
      version++;
      if (order === "hide then poll") browser.events.dispatchEvent(new Event("pagehide"));
      await advance(5000); // keepSaving's real interval queues the updated version
      if (order === "poll then hide") browser.events.dispatchEvent(new Event("pagehide"));
      const pending = save();
      ack.resolve();
      await Promise.all([first, pending]);
      expect(device.current()).toEqual(blob(2));
      expect(server.data).toEqual(blob(order === "poll" ? 1 : 2));
      expect(await page(device, server)).toEqual(blob(2));
      expect(server.history).toEqual([blob(1), blob(2)]);
    });
  },
);

test.each([false, true])(
  "simulation: lost acknowledgement after commit, other device saves = %s",
  async (conflict) => {
    await simulate(async ({ advance }) => {
      const server = new Server();
      const device = new Device();
      const accepted = deferred<void>();
      const request = server.request;
      server.request = async (url, init) => {
        const response = await request(url, init);
        if (init?.method === "PUT" && server.rev === 1 && response.ok) {
          const signal = init.signal!;
          accepted.resolve();
          await new Promise<void>((_, reject) =>
            signal.addEventListener("abort", () => reject(new Error("lost acknowledgement")), { once: true }),
          );
        }
        return response;
      };

      await page(device, server);
      let failed = false;
      const write = storeSaves("game", blob(1)).catch(() => {
        failed = true;
      });
      await accepted.promise;
      await advance(30_000);
      expect(failed).toBe(false);
      await advance(1); // two bytes add one millisecond to the upload deadline
      await write;
      expect(failed).toBe(true);
      if (conflict) {
        const other = new Device();
        await page(other, server);
        await storeSaves("game", blob(2));
        savesFor(device.files, server.request);
      }
      const notices: string[] = [];
      expect(await page(device, server, (message) => notices.push(message))).toEqual(blob(1));
      expect(notices).toHaveLength(conflict ? 1 : 0);
      expect(server.data).toEqual(blob(1));
      if (conflict) expect(device.kept()).toContainEqual(blob(2));
    });
  },
);

test.each(["ack before conflict", "ack after conflict"])(
  "simulation: two tabs sharing save files: %s",
  async (order) => {
    await simulate(async ({ advance }) => {
      const device = new Device();
      const server = new Server();
      const accepted = deferred<void>();
      const ack = deferred<void>();
      // Independent module state for each tab; both use the same OPFS and origin locks.
      const tab = async (request: typeof fetch) => {
        vi.resetModules();
        const session = await import("./saves");
        session.savesFor(device.files, request);
        await session.loadSaves("game");
        return session;
      };

      const a = await tab(async (url, init) => {
        const response = await server.request(url, init);
        if (init?.method === "PUT") {
          accepted.resolve();
          await ack.promise;
        }
        return response;
      });
      const b = await tab(server.request);
      const sending = a.storeSaves("game", blob(1));
      await accepted.promise;
      await advance(250);
      if (order === "ack before conflict") {
        ack.resolve();
        await sending;
      }
      await expect(b.storeSaves("game", blob(2))).rejects.toMatchObject({ name: "SaveConflict" });
      ack.resolve();
      await sending;
      expect(server.data).toEqual(blob(1));
      expect(device.current()).toEqual(blob(2));
      await tab(server.request);
      expect(server.data).toEqual(blob(2));
      expect(device.kept()).toContainEqual(blob(1));
    });
  },
);

test.each(["before publish", "after publish"])(
  "simulation: restart %s of local save metadata",
  async (crash) => {
    await simulate(async ({ advance, browser }) => {
      const device = new Device();
      const server = new Server();

      await page(device, server);
      await storeSaves("game", blob(1));
      const original = device.files.write.bind(device.files);
      // The inactive blob is already written. Interrupt on either side of the atomic pointer update.
      device.files.write = async (file, bytes) => {
        if (file.endsWith(".sync.json") && crash === "before publish") throw new Error("write interrupted");
        await original(file, bytes);
        if (file.endsWith(".sync.json")) throw new Error("acknowledgement interrupted");
      };
      await storeSaves("game", blob(2), false);
      device.files.write = original;
      const expected = blob(crash === "before publish" ? 1 : 2);
      await advance(5000);
      browser.setOnline(false);
      expect(await page(device, server)).toEqual(expected);
      expect(server.data).toEqual(blob(1));
      browser.setOnline(true);
      expect(await page(device, server)).toEqual(expected);
      expect(server.data).toEqual(expected);
    });
  },
);

test("simulation: replay a seeded history of saves, disconnects and restarts", async () => {
  const replay = () =>
    simulate(async ({ advance, randomBelow, browser }) => {
      const server = new Server();
      const device = new Device();
      const trace = [];
      let latest = blob(0);
      await page(device, server);
      await storeSaves("game", latest);
      for (let step = 0; step < 24; step++) {
        await advance(1 + randomBelow(1000));
        browser.setOnline(randomBelow(3) !== 0);
        expect(await page(device, server)).toEqual(latest);
        latest = blob(1 + randomBelow(65535));
        const saved = await storeSaves("game", latest).then(
          (o) => o.kind === "sent",
          () => false,
        );
        expect(saved).toBe(navigator.onLine);
        expect(device.current()).toEqual(latest);
        if (saved) expect(server.data).toEqual(latest);
        trace.push([
          Date.now(),
          saved,
          server.rev,
          ...latest,
          new TextDecoder().decode(device.m.get("game.v2.sync.json")),
        ]);
      }
      browser.setOnline(true);
      expect(await page(device, server)).toEqual(latest);
      expect(server.data).toEqual(latest);
      return trace;
    });
  expect(await replay()).toEqual(await replay());
});

/** `d`'s blob writes fail while `failing()` says so (an OPFS quota denied, say). */
function failBlobs(d: Device, failing: () => boolean) {
  const write = d.files.write.bind(d.files);
  d.files.write = async (f, data) => {
    if (/^game\.v2\.[01]\.bin$/.test(f) && failing()) throw new Error("QuotaExceededError");
    return write(f, data);
  };
}

test("a local write that failed once is tried again: the save after it is kept (review e18bf44)", async () => {
  const s = new Server();
  const d = new Device();
  online = true;
  let current = blob(1);
  const k = await keeper(d, s, () => current as Uint8Array<ArrayBuffer>, null);
  current = blob(1);
  k.write();
  await k.save();
  let fail = true;
  failBlobs(d, () => fail);
  s.up = false;
  current = blob(2);
  k.write();
  await k.save();
  expect(eq(d.current(), blob(1))).toBe(true);
  // (the game writes nothing more; OPFS works again: the next look writes B, not "already kept")
  fail = false;
  k.at(1000);
  await k.save();
  expect(eq(d.current(), blob(2))).toBe(true);
  // (and B is what the next start has, and sends)
  s.up = true;
  expect(eq(await page(d, s), blob(2))).toBe(true);
  expect(eq(s.data, blob(2))).toBe(true);
});

test("this browser's copy not written, the server's written: the next start does not go back (keepSaving)", async () => {
  const s = new Server();
  const d = new Device();
  online = true;
  let current = blob(1);
  const k = await keeper(d, s, () => current as Uint8Array<ArrayBuffer>, null);
  // (A: kept here only, the server down)
  s.up = false;
  k.write();
  await k.save();
  expect(eq(d.current(), blob(1))).toBe(true);
  // (B: this browser's write fails, the upload goes through)
  s.up = true;
  failBlobs(d, () => true);
  current = blob(2);
  k.write();
  await k.save(true);
  expect(eq(s.data, blob(2))).toBe(true);
  expect(eq(d.current(), blob(1))).toBe(true);
  // (A is no longer taken for the newest unsent save)
  expect(eq(await page(d, s), blob(2))).toBe(true);
  expect(eq(s.data, blob(2))).toBe(true);
});

test("this browser's copy not written, the server's written: the next start does not go back (storeSaves)", async () => {
  const s = new Server();
  const d = new Device();
  online = false;
  await page(d, s);
  expect((await storeSaves("game", blob(1), false)).kind).toBe("kept");
  online = true;
  failBlobs(d, () => true);
  await page(d, s);
  expect(await storeSaves("game", blob(2))).toMatchObject({ kind: "sent-not-kept" });
  expect(eq(s.data, blob(2))).toBe(true);
  expect(eq(await page(d, s), blob(2))).toBe(true);
  expect(eq(s.data, blob(2))).toBe(true);
});

// Random runs of the keeper with OPFS writes failing now and then, the server up and down,
// forced saves (property, seeded): after the last save with everything working, this browser's
// copy and the server hold the newest export, and a restart returns it; never an older one.
test("property: failed local writes and uploads never bring back an older save", async () => {
  let x = 4242;
  const rnd = (n: number) => ((x = (Math.imul(x, 1103515245) + 12345) >>> 0) >>> 8) % n;
  for (let run = 0; run < 200; run++) {
    const s = new Server();
    const d = new Device();
    online = true;
    let v = 1;
    let current = blob(v);
    let k = await keeper(d, s, () => current as Uint8Array<ArrayBuffer>, null);
    let fail = false;
    failBlobs(d, () => fail);
    let t = 0;
    for (let step = 0; step < 20; step++) {
      t += rnd(40_000);
      k.at(t);
      if (rnd(2)) {
        current = blob(++v);
        k.write();
      }
      fail = rnd(3) === 0;
      s.up = rnd(3) !== 0;
      await k.save(rnd(4) === 0);
      // (whatever failed: a restart (a new page, sometimes) never returns a save older than
      // the newest either copy holds)
      if (rnd(4) === 0) {
        fail = false;
        const newest = Math.max(d.current()?.[0] ?? 0, s.data?.[0] ?? 0);
        const up = s.up;
        s.up = true;
        const got = await page(d, s);
        expect(got?.[0] ?? 0, `run ${run} step ${step}: restart`).toBe(newest);
        if (got) current = got as Uint8Array<ArrayBuffer>;
        k = await keeper(d, s, () => current as Uint8Array<ArrayBuffer>, got);
        s.up = up;
        failBlobs(d, () => fail);
      }
    }
    fail = false;
    s.up = true;
    k.at(t + 10 * UPLOAD_EVERY);
    await k.save(true);
    expect(eq(d.current(), current), `run ${run}: here`).toBe(true);
    expect(eq(s.data, current), `run ${run}: server`).toBe(true);
    expect(eq(await page(d, s), current), `run ${run}: restart`).toBe(true);
  }
});

test("simulation: an acknowledgement while a start's GET is out keeps the newer local save (review e18bf44)", async () => {
  await simulate(async () => {
    const device = new Device();
    const server = new Server();
    server.data = blob(1);
    server.rev = 1;
    const putStarted = deferred<void>();
    const putGo = deferred<void>();
    const getStarted = deferred<void>();
    const getGo = deferred<void>();
    const tab = async (request: typeof fetch) => {
      vi.resetModules();
      const session = await import("./saves");
      session.savesFor(device.files, request);
      return session;
    };
    const b = await tab(async (url, init) => {
      if (init?.method === "PUT") {
        putStarted.resolve();
        await putGo.promise;
      }
      return server.request(url, init);
    });
    await b.loadSaves("game");
    // (tab B: its save 2 written here, its upload out)
    const storing = b.storeSaves("game", blob(2));
    await putStarted.promise;
    // (tab A starts: its GET answers with the server's copy before B's upload landed)
    let gets = 0;
    const a = await tab(async (url, init) => {
      if (init?.method === "PUT" || gets++) return server.request(url, init);
      const answer = await server.request(url, init);
      getStarted.resolve();
      await getGo.promise;
      return answer;
    });
    const starting = a.loadSaves("game");
    await getStarted.promise;
    putGo.resolve();
    expect(await storing).toMatchObject({ kind: "sent" });
    getGo.resolve();
    // (B's acknowledgement changed only the record's dirty and base: A's older answer is not taken)
    expect(await starting).toEqual(blob(2));
    expect(device.current()).toEqual(blob(2));
    expect(server.data).toEqual(blob(2));
  });
});

test("simulation: a slow upload does not hold the next save's local write (review e18bf44)", async () => {
  await simulate(async ({ advance, browser }) => {
    const device = new Device();
    const server = new Server();
    const putStarted = deferred<void>();
    const putGo = deferred<void>();
    const request = server.request;
    server.request = async (url, init) => {
      if (init?.method === "PUT" && server.rev === 0) {
        putStarted.resolve();
        await putGo.promise;
      }
      return request(url, init);
    };
    await page(device, server);
    let version = 1;
    const game = { _vnhost_vfs_changes: () => version } as unknown as SavesModule;
    const save = keepSaving(game, "game", [], null, { read: () => blob(version) });
    const first = save();
    await putStarted.promise;
    version = 2;
    // (the page is hidden: save 2 is on this device although save 1's PUT is still out)
    browser.events.dispatchEvent(new Event("pagehide"));
    const forced = save(true);
    await advance(10);
    expect(device.current()).toEqual(blob(2));
    expect(server.data).toBeNull();
    putGo.resolve();
    await Promise.all([first, forced]);
    expect(server.data).toEqual(blob(2));
    expect(await page(device, server)).toEqual(blob(2));
  });
});

test("an older app (e18bf44's saves.ts) beside this one: neither takes the other's files for its own", async () => {
  const s = new Server();
  const d = new Device();
  online = true;
  const old = await import("./test/legacy/saves-e18bf44");
  const older = async () => {
    old.savesFor(d.files, s.request);
    return old.loadSaves("game");
  };
  // (the older app saved A here, not sent)
  await older();
  s.up = false;
  await old.storeSaves("game", blob(1)).catch(() => undefined);
  s.up = true;
  // (this app takes it over, saves B offline)
  online = false;
  expect(eq(await page(d, s), blob(1))).toBe(true);
  expect((await storeSaves("game", blob(2), false)).kind).toBe("kept");
  online = true;
  // (the older app starts: A is not sent again, nor this format's inactive slot taken for its own)
  await older();
  expect(s.data === null || eq(s.data, blob(1))).toBe(true);
  expect(s.history.some((h) => eq(h, blob(2)))).toBe(false);
  // (this app starts: B, its own newest unsent save, goes up)
  expect(eq(await page(d, s), blob(2))).toBe(true);
  expect(eq(s.data, blob(2))).toBe(true);
  // (the older app now starts from the server's copy: B)
  expect(eq(await older(), blob(2))).toBe(true);
});
