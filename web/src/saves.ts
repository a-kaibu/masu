import { SaveConflict, SaveTransport } from "./save-transport";
import { withRetries } from "./net";
import { snapping } from "./savestate";
import { portablePath, same } from "./file-records";
// Saved games: what the game wrote (in kura's in-memory file system) survives the page.
// The files written under the game's folders and the prefix's user folders go to the browser's
// origin private file system (OPFS) as one blob per game, whenever they change (checked every
// few seconds, and when the page is hidden); the next start puts them back before the program runs (MASU_SAVES, crates/kura/src/vfs.rs).
// Each new blob is also backed up through masu serve to S3 (`/saves/<name>`, when the server
// has a bucket set up); a browser without a blob of its own starts from that backup.

export interface SavesModule {
  wasmMemory: WebAssembly.Memory;
  _malloc(size: number): number;
  _vnhost_free(p: number): void;
  _vnhost_vfs_export(roots: number, len: number): number;
  _vnhost_vfs_list(roots: number, len: number): number;
  /** changes whenever the file system may have changed */
  _vnhost_vfs_changes?(): number;
}

/** Where this browser keeps the blobs (OPFS "saves/"; tests give their own). */
export interface SaveFiles {
  read(file: string): Promise<Uint8Array<ArrayBuffer> | null>;
  write(file: string, data: Uint8Array): Promise<void>;
  /** the files kept, and a removal (stash keeps the newest few copies set aside) */
  list?(): Promise<string[]>;
  remove?(file: string): Promise<void>;
}

const opfs: SaveFiles = {
  async read(file) {
    try {
      const root = await navigator.storage.getDirectory();
      const dir = await root.getDirectoryHandle("saves");
      const f = await dir.getFileHandle(file);
      return new Uint8Array(await (await f.getFile()).arrayBuffer());
    } catch (e) {
      console.warn(`saves: read ${file}: ${String(e)}`);
      return null;
    }
  },
  async write(file, data) {
    const root = await navigator.storage.getDirectory();
    const dir = await root.getDirectoryHandle("saves", { create: true });
    const w = await (await dir.getFileHandle(file, { create: true })).createWritable();
    await w.write(data as Uint8Array<ArrayBuffer>);
    await w.close();
  },
  async list() {
    const dir = await (await navigator.storage.getDirectory()).getDirectoryHandle("saves", { create: true });
    const out: string[] = [];
    for await (const name of (dir as unknown as { keys(): AsyncIterable<string> }).keys()) out.push(name);
    return out;
  },
  async remove(file) {
    const dir = await (await navigator.storage.getDirectory()).getDirectoryHandle("saves");
    await dir.removeEntry(file);
  },
};

let files: SaveFiles = opfs;
let transport = new SaveTransport();
/** Tests: another place for the blobs and another fetch (a fresh transport: a new page). */
export function savesFor(f: SaveFiles, request?: typeof fetch): void {
  files = f;
  transport = new SaveTransport(request);
  locks.clear();
}

const backup = (name: string) => new URL(`saves/${encodeURIComponent(name)}`, location.href).href;

// This browser's copy of a game's saves: two blob slots and a small record naming the current
// one, published by replacing the record after the inactive slot is written (an interrupted
// write leaves the previous slot and record as they were).
//
// The files are versioned (FORMAT): apps kept for older builds (apps.rs) run their own saves.ts
// on the same origin, and read `<name>.bin` / `<name>.sync.json` as their format had them; one
// that read this format's files took an inactive slot for the newest unsent save and sent it
// (review e18bf44). This format's files are its own; the older ones are read once, when this
// format has none yet (taken over), and left marked as sent, so an older app starts from the
// server's copy, which this one keeps up to date.
export const FORMAT = 2;
interface Sync {
  /** the server's revision this copy was last in step with */
  base: string | null;
  /** not known to be on the server */
  dirty: boolean;
  slot: 0 | 1;
  /** which write made the current blob (an acknowledgement applies to that one only) */
  generation: string | null;
  /** counts every change of the record, an acknowledgement's too: a start's GET compares it,
   * since an ACK changes dirty/base and not the generation (review e18bf44) */
  rev: number;
  /** a blob from before there was a record (an old page's): not known to be sent or not */
  adopted?: boolean;
}
const blobName = (name: string, slot: 0 | 1) => `${name}.v${FORMAT}.${slot}.bin`;
const syncName = (name: string) => `${name}.v${FORMAT}.sync.json`;
/** the files of the format before this one (saves.ts up to e18bf44): read to take over only */
export const legacyFiles = (name: string) => ({
  bin: `${name}.bin`,
  next: `${name}.next`,
  sync: `${name}.sync.json`,
});

function parseSync(b: Uint8Array): Partial<Sync> {
  try {
    return JSON.parse(new TextDecoder().decode(b)) as Partial<Sync>;
  } catch {
    // An unreadable pointer cannot identify which slot is current.
    throw new Error("unreadable save generation");
  }
}
function syncOf(v: Partial<Sync>): Sync {
  return {
    base: typeof v.base === "string" ? v.base : null,
    dirty: v.dirty !== false,
    slot: v.slot === 1 ? 1 : 0,
    generation: typeof v.generation === "string" ? v.generation : null,
    rev: typeof v.rev === "number" && Number.isSafeInteger(v.rev) ? v.rev : 0,
    ...(v.adopted === true ? { adopted: true } : {}),
  };
}
const encodeSync = (s: unknown) => new TextEncoder().encode(JSON.stringify(s));

/** The older format's copy, made this format's (once: this format had no record yet), and the
 * older record marked as sent. Under localLock. */
async function takeOver(name: string): Promise<Sync | null> {
  const old = legacyFiles(name);
  const oldRecord = await files.read(old.sync);
  const legacy = oldRecord ? syncOf(parseSync(oldRecord)) : null;
  const blob = await files.read(legacy?.slot === 1 ? old.next : old.bin);
  if (!blob) return null;
  const record: Sync = legacy
    ? { base: legacy.base, dirty: legacy.dirty, slot: 0, generation: crypto.randomUUID(), rev: 1 }
    : { base: null, dirty: false, slot: 0, generation: crypto.randomUUID(), rev: 1, adopted: true };
  await files.write(blobName(name, 0), blob);
  await files.write(syncName(name), encodeSync(record));
  // (the older app now starts from the server's copy: this format sends what was unsent; its
  // record keeps its own fields, only dirty is set)
  await files.write(
    old.sync,
    encodeSync({ ...(oldRecord ? parseSync(oldRecord) : {}), base: legacy?.base ?? null, dirty: false }),
  );
  return record;
}

async function readSync(name: string): Promise<Sync | null> {
  const b = await files.read(syncName(name));
  return b ? syncOf(parseSync(b)) : takeOver(name);
}
const readBlob = (name: string, sync: Sync | null) =>
  sync ? files.read(blobName(name, sync.slot)) : Promise.resolve(null);
/** Replaces the record: `next` over `previous` (the one read under the same lock), counted. */
const publish = (name: string, previous: Sync | null, next: Omit<Sync, "rev">): Promise<Sync> => {
  const record: Sync = { ...next, rev: (previous?.rev ?? 0) + 1 };
  return files.write(syncName(name), encodeSync(record)).then(() => record);
};
/** Keeps `data` beside the blob (`what`: whose copy), when one of two copies must give way. */
async function stash(name: string, what: string, data: Uint8Array): Promise<void> {
  await files
    .write(`${name}.${what}-${Date.now()}.bin`, data)
    .catch((e: unknown) => console.warn("saves: stash", e));
  // (the newest STASH_KEEP of the name's copies stay: before, every conflict left a whole copy
  // for good, counted against the quota the chunk cache is trimmed by)
  const all = await files.list?.().catch(() => []);
  if (!all || !files.remove) return;
  for (const f of stashesToDrop(name, all)) await files.remove(f).catch(() => undefined);
}

/** Copies set aside kept per game (stash). */
export const STASH_KEEP = 3;

/** Of the files `all`, `name`'s copies set aside beyond the newest STASH_KEEP (by their time). */
export function stashesToDrop(name: string, all: readonly string[]): string[] {
  const pre = `${name}.`;
  const mine = all
    .filter((f) => f.startsWith(pre) && /^(server|local)-\d+\.bin$/.test(f.slice(pre.length)))
    .map((f) => [f, Number(/-(\d+)\.bin$/.exec(f)![1])] as const)
    .sort((a, b) => b[1] - a[1]);
  return mine.slice(STASH_KEEP).map(([f]) => f);
}

// (one save of a name at a time: a later one's "dirty" never lands under an earlier one's "clean")
const locks = new Map<string, Promise<unknown>>();
function locked<T>(name: string, f: () => Promise<T>): Promise<T> {
  const run = (locks.get(name) ?? Promise.resolve()).catch(() => {}).then(f);
  locks.set(name, run);
  return run;
}

// Only OPFS decisions use the origin-wide Web Lock. A slow PUT must never
// keep another tab from persisting its newest local save.
async function localLock<T>(name: string, f: () => Promise<T>): Promise<T> {
  return navigator.locks.request(`masu-save:${name}`, f);
}

/** What this browser's copy now holds after a local write: the record that write published. */
export interface Persisted {
  readonly generation: string;
  readonly record: Sync;
}

/** Writes `data` as this browser's copy (`clean`: the server has these bytes, at `base`). */
function persist(name: string, data: Uint8Array, base: string | null, clean: boolean): Promise<Persisted> {
  return localLock(name, () => localWrite(name, data, base, clean));
}

/**
 * After the server took a copy at `revision`: the record of the write `generation` (this
 * page's last) is marked as sent when it holds those bytes (`holds`), or only moved to the new
 * base when it holds a newer, still unsent copy. A record another write replaced meanwhile is
 * left alone: that write's own acknowledgement settles it.
 */
async function acknowledge(
  name: string,
  generation: string | null,
  revision: string,
  holds: "these" | "older" | "newer",
) {
  await localLock(name, async () => {
    const current = await readSync(name);
    if (!current || current.generation !== generation) return;
    // ("older": this copy could not be written here (OPFS failed) and the server has a newer
    // one: the stale copy must not be taken for the newest unsent save at the next start)
    await publish(name, current, { ...current, base: revision, dirty: holds === "newer" });
  });
}

/**
 * The blob an earlier session saved for `name` (null: none). With the server in reach: the
 * server's, unless this browser's copy was saved after it was last sent (offline, or the page
 * closed before the upload): then this browser's, sent now; when the server's copy moved too
 * (another device), this browser's still wins and the server's is kept beside it (and in the
 * server's revisions), `notice` told. Out of reach: this browser's copy, sent later.
 */
export async function loadSaves(
  name: string,
  notice?: (message: string) => void,
): Promise<Uint8Array | null> {
  const url = backup(name);

  const before = await localLock(name, () => readSync(name));
  let remote: Uint8Array | null;
  try {
    // (offline: no tries at all; started from the app kept on the device, sw.ts)
    if (!navigator.onLine) throw new Error("offline");
    remote = await withRetries(() => transport.load(url), 3);
  } catch (e) {
    console.warn(`saves: server out of reach, starting from this browser's copy: ${String(e)}`);
    return localLock(name, async () => {
      const sync = await readSync(name);
      const local = await readBlob(name, sync);
      transport.startedOffline(url, local, sync?.adopted ? null : sync?.base);
      return local;
    });
  }
  const { local, sync, changed } = await localLock(name, async () => {
    const sync = await readSync(name);
    const local = await readBlob(name, sync);
    // (any change of the record while the GET was out, an acknowledgement's too: its answer
    // may be older than what this browser now has, and is not taken)
    if (before?.rev !== sync?.rev || before?.generation !== sync?.generation)
      return { local, sync, changed: true };
    if (remote && !(local && sync?.dirty)) {
      if (local && sync?.adopted && !same(local, remote)) await stash(name, "local", local);
      const revision = transport.revision(url) ?? null;
      await localWrite(name, remote, revision, true).catch((e: unknown) =>
        console.warn("saves: local cache", e),
      );
    }
    return { local, sync, changed: false };
  });
  if (changed) return local;
  const revision = transport.revision(url) ?? '"0"';
  const send = async (data: Uint8Array, generation: string | null) => {
    // (not sent now: still dirty, sent with the next save or the next start)
    await upload(name, data as Uint8Array<ArrayBuffer>, generation, "these").catch((e: unknown) =>
      console.warn(`saves: not sent: ${String(e)}`),
    );
  };
  if (local && sync?.dirty) {
    if (remote && sync.base !== revision && !same(remote, local)) {
      await stash(name, "server", remote);
      console.warn(`saves: ${name}: saved here and on another device since ${sync.base}; this device's kept`);
      notice?.(
        "別の端末のセーブより、この端末で後から保存したセーブを使います（別の端末のものも残してあります）",
      );
    }
    await send(local, sync.generation);
    return local;
  }
  if (remote) return remote;
  if (local?.length) await send(local, sync?.generation ?? null);
  return local;
}

/** persist under the caller's localLock (loadSaves' decision and its write in one hold) */
async function localWrite(
  name: string,
  data: Uint8Array,
  base: string | null,
  clean: boolean,
): Promise<Persisted> {
  const previous = await readSync(name);
  const slot = previous?.slot === 1 ? 0 : 1;
  const generation = crypto.randomUUID();
  await files.write(blobName(name, slot), data);
  const record = await publish(name, previous, { base, dirty: !clean, generation, slot });
  return { generation, record };
}

/** Sends `data` to the server and settles the local record of the write `generation` (`holds`:
 * how that record's bytes stand to `data`). Rejects when the server did not take it. */
async function upload(
  name: string,
  data: Uint8Array<ArrayBuffer>,
  generation: string | null,
  holds: "these" | "older" | "newer",
): Promise<string> {
  const revision = await transport.store(backup(name), data);
  await acknowledge(name, generation, revision, holds).catch((e: unknown) =>
    console.warn("saves: local cache", e),
  );
  return revision;
}

function call(m: SavesModule, f: (s: number, len: number) => number, arg: string): Uint8Array<ArrayBuffer> {
  const s = new TextEncoder().encode(`${arg}\0`);
  const sp = m._malloc(s.length + 8) >>> 0;
  // (no room in the host heap: not the roots written over the guest's memory at address 0)
  if (!sp) throw new Error("saves: no memory for the export");
  try {
    new Uint8Array(m.wasmMemory.buffer).set(s, sp);
    const lenp = sp + Math.ceil(s.length / 4) * 4;
    const p = f(sp, lenp) >>> 0;
    if (!p) throw new Error("saves: no memory for the export");
    const len = new Uint32Array(m.wasmMemory.buffer, lenp, 1)[0]!;
    const out = new Uint8Array(len);
    out.set(new Uint8Array(m.wasmMemory.buffer, p, len));
    m._vnhost_free(p);
    return out;
  } finally {
    m._vnhost_free(sp);
  }
}

/** The paths of the files written under `roots` (vnhost_vfs_list: path length, path, size). */
export function listFiles(m: SavesModule, roots: string[]): string[] {
  const b = call(m, (s, l) => m._vnhost_vfs_list(s, l), roots.join(":"));
  const view = new DataView(b.buffer);
  const out: string[] = [];
  for (let at = 0; at + 4 <= b.length;) {
    const n = view.getUint32(at, true);
    out.push(new TextDecoder().decode(b.subarray(at + 4, at + 4 + n)));
    at += 12 + n;
  }
  return out;
}

/**
 * The records of the files written under `roots`; with `keep`, only those it keeps, picked
 * from their list first so the others (an installer's temp files: hundreds of MB) are never
 * copied out of the module (#234).
 */
export function exportFiles(
  m: SavesModule,
  roots: string[],
  keep?: (path: string) => boolean,
): Uint8Array<ArrayBuffer> {
  if (keep) {
    roots = listFiles(m, roots).filter(keep);
    if (!roots.length) return new Uint8Array(0);
  }
  return call(m, (s, l) => m._vnhost_vfs_export(s, l), roots.join(":"));
}

function exportSaves(m: SavesModule, roots: string[]): Uint8Array<ArrayBuffer> {
  return exportFiles(m, roots, portablePath);
}

/** What a store did: each side said apart (a "not sent" was once read as "sent", and a save
 * never written here was taken for one that was; review e18bf44). */
export type StoreOutcome =
  /** on the server, and this browser's copy holds it as sent */
  | { readonly kind: "sent"; readonly revision: string }
  /** on the server; this browser's copy could not be written (its older one is marked sent) */
  | { readonly kind: "sent-not-kept"; readonly revision: string }
  /** this browser's copy only, marked not sent (sent later, or at the next start) */
  | { readonly kind: "kept"; readonly generation: string }
  /** neither: no upload asked for and this browser's copy could not be written */
  | { readonly kind: "lost" };

/** Writes `data` as this browser's copy (marked not sent), its failure said, never thrown: a
 * denied OPFS quota must not keep the server's copy from being written. */
async function persistOrStale(
  name: string,
  data: Uint8Array,
  base: string | null,
): Promise<
  | { readonly kind: "persisted"; readonly p: Persisted }
  | { readonly kind: "failed"; readonly stale: string | null }
> {
  try {
    return { kind: "persisted", p: await persist(name, data, base, false) };
  } catch (e) {
    console.warn("saves: local cache", e);
    const stale = await localLock(name, () => readSync(name)).then(
      (r) => r?.generation ?? null,
      () => null,
    );
    return { kind: "failed", stale };
  }
}

/** Writes `data` as `name`'s blob: this browser's copy, then (`upload`) the server's: rejects
 * when the server's failed (this browser's copy is kept, marked as not sent). */
export function storeSaves(
  name: string,
  data: Uint8Array<ArrayBuffer>,
  uploadToo = true,
): Promise<StoreOutcome> {
  return locked(name, async (): Promise<StoreOutcome> => {
    const url = backup(name);
    // This browser's copy first: with the link down the save still survives the page (the
    // server's copy is tried again on the next change check, or the next start).
    const base =
      transport.revision(url) ??
      (await localLock(name, () => readSync(name)).catch(() => null))?.base ??
      null;
    const local = await persistOrStale(name, data, base);
    if (!uploadToo)
      return local.kind === "persisted" ? { kind: "kept", generation: local.p.generation } : { kind: "lost" };
    if (local.kind === "persisted")
      return { kind: "sent", revision: await upload(name, data, local.p.generation, "these") };
    return { kind: "sent-not-kept", revision: await upload(name, data, local.stale, "older") };
  });
}

/** The least time between two uploads of a game's saves while it plays (keepSaving; one forced
 * by the page being hidden or the game quitting goes at once). */
export const UPLOAD_EVERY = 60_000;

/**
 * Keeps saving what the game writes under `roots` for `name`. Returns a function that saves now
 * (`force`: the upload too, even in the pause after a failed one: when the game quits or the
 * page is hidden).
 *
 * Two lines of work, each one at a time: this browser's copy (export, OPFS write) and the
 * upload. A save waits for the previous local write only, never for an upload: a slow PUT held
 * the next save's export and OPFS write behind it, and a page closed meanwhile lost it (review
 * e18bf44). What each side holds is known by what it confirmed: `local` is what this browser's
 * copy was written with (its write's generation), `last` what the server took; neither is
 * inferred from what was asked.
 */
export function keepSaving(
  m: SavesModule,
  name: string,
  roots: string[],
  last: Uint8Array | null,
  // (tests: the clock and the export; `notice`: what the player is told, once, when the server's
  // copy stopped taking this page's saves: another device saved since)
  opt: { now?: () => number; read?: () => Uint8Array<ArrayBuffer>; notice?: (message: string) => void } = {},
): (force?: boolean) => Promise<void> {
  const now = opt.now ?? Date.now;
  const read = opt.read ?? (() => exportSaves(m, roots));
  const url = backup(name);
  let seen = -1;
  // (the newest export, numbered: `seq` orders what the two sides hold; 0 is `last`, what this
  // page started from, which both this browser's copy and the server have)
  let seq = 0;
  let target: { readonly data: Uint8Array<ArrayBuffer>; readonly seq: number } | null = last
    ? { data: last as Uint8Array<ArrayBuffer>, seq: 0 }
    : null;
  // (what this browser's copy holds, as this page wrote it: the export's number and the
  // write's generation; null generation: not written by this page yet)
  let localSeq = 0;
  let localGeneration: string | null = null;
  // (the record current before this page's first write: what a sent copy supersedes when this
  // page could write none; undefined: not read yet)
  let before: string | null | undefined;
  // (what the server has: `last`'s number)
  let sentSeq = 0;
  let failures = 0;
  let retryAt = 0;
  let told = false;
  const due = (force: boolean) => force || now() >= retryAt;

  const localStep = async () => {
    // (a save state is being taken: the memory stays as it is; the next look saves)
    if (snapping()) return;
    // (nothing written since the last look: no export)
    const changes = m._vnhost_vfs_changes?.() ?? -2;
    if (changes !== seen || changes === -2) {
      const data = read();
      seen = changes;
      if (!target || !same(target.data, data)) target = { data, seq: ++seq };
    }
    const t = target;
    if (!t || localSeq >= t.seq) return;
    before ??= await localLock(name, () => readSync(name)).then(
      (r) => r?.generation ?? null,
      () => null,
    );
    try {
      const p = await persist(name, t.data, transport.revision(url) ?? null, same(last, t.data));
      if (t.seq >= localSeq) {
        localSeq = t.seq;
        localGeneration = p.generation;
      }
    } catch (e) {
      // (tried again at the next look: localSeq still names the older copy)
      console.warn("saves: local cache", e);
    }
  };

  /** this browser's record after the server took export `t` at `revision` */
  const settle = (t: number, revision: string) => {
    const holds = localSeq === t ? "these" : localSeq < t ? "older" : "newer";
    return acknowledge(name, localGeneration ?? before ?? null, revision, holds).catch((e: unknown) =>
      console.warn("saves: local cache", e),
    );
  };

  const uploadStep = async (force: boolean) => {
    const t = target;
    if (!t || sentSeq >= t.seq) return;
    // (the server has these bytes already: A, B not sent yet, then A again)
    if (same(last, t.data)) {
      sentSeq = t.seq;
      const revision = transport.revision(url);
      if (revision) await settle(t.seq, revision);
      return;
    }
    if (!due(force)) return;
    try {
      const revision = await transport.store(url, t.data);
      last = t.data;
      sentSeq = Math.max(sentSeq, t.seq);
      failures = 0;
      retryAt = now() + UPLOAD_EVERY;
      // (sent when it holds these bytes; an older one (its write failed) marked sent too, so a
      // start does not take it for the newest; a newer one moved to the new base, still unsent)
      await settle(t.seq, revision);
    } catch (e) {
      if (e instanceof SaveConflict && !told) {
        told = true;
        opt.notice?.(
          "別の端末でこのゲームのセーブが更新されたため、サーバーへの保存を止めました（この端末には保存しています）。再読み込みすると別の端末のセーブを使います",
        );
      }
      failures++;
      // (a forced one that failed does not bring the next plain upload forward)
      retryAt = Math.max(retryAt, now() + Math.min(5000 * 2 ** failures, 5 * 60_000));
      console.warn(`saves: ${String(e)}`);
    }
  };

  // (each line: one at a time, and at most one more waiting: a stuck upload used to have each
  // 5 s tick chain another look behind it)
  const line = <A>(step: (a: A) => Promise<void>, merge: (a: A, b: A) => A) => {
    let pending: Promise<void> | undefined;
    let queued: Promise<void> | undefined;
    let arg: A | undefined;
    const run = (a: A): Promise<void> => {
      if (!pending) {
        pending = step(a).finally(() => {
          pending = undefined;
        });
        return pending;
      }
      arg = arg === undefined ? a : merge(arg, a);
      return (queued ??= pending.then(() => {
        const next = arg!;
        arg = undefined;
        queued = undefined;
        return run(next);
      }));
    };
    return run;
  };
  const saveLocal = line<true>(
    () => localStep(),
    () => true,
  );
  const sendUp = line<boolean>(uploadStep, (a, b) => a || b);
  // (the upload after the local write it follows, never before it: what the server gets is in
  // this browser's copy first, or its write failed)
  // (saves asked for while both lines wait are one: the same local write, then the same upload)
  let chained:
    | { readonly local: Promise<void>; readonly force: boolean; readonly done: Promise<void> }
    | undefined;
  const save = (force = false): Promise<void> => {
    const local = saveLocal(true);
    if (chained?.local === local && (chained.force || !force)) return chained.done;
    const done = local.then(() => sendUp(force));
    chained = { local, force, done };
    return done;
  };
  setInterval(() => void save(), 5000);
  addEventListener("visibilitychange", () => document.visibilityState === "hidden" && void save(true));
  addEventListener("pagehide", () => void save(true));
  return save;
}
