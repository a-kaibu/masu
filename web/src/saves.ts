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
  /** the older format's record (legacyFiles) current when this record was written: one with
   * another generation was written later, by an app of the older format still open (review
   * b5dacf4 1.4) */
  legacy?: string | null;
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
    ...(typeof v.legacy === "string" ? { legacy: v.legacy } : {}),
  };
}
const encodeSync = (s: unknown) => new TextEncoder().encode(JSON.stringify(s));

/** The older format's record (null: none, or none readable: nothing to take). */
async function legacyRecord(
  name: string,
): Promise<{ readonly raw: Partial<Sync>; readonly sync: Sync } | null> {
  const b = await files.read(legacyFiles(name).sync);
  if (!b) return null;
  try {
    const raw = parseSync(b);
    return { raw, sync: syncOf(raw) };
  } catch {
    return null;
  }
}
/** The older format's current blob, as its record names it. */
function legacyBlob(name: string, legacy: Sync | null): Promise<Uint8Array<ArrayBuffer> | null> {
  return files.read(legacy?.slot === 1 ? legacyFiles(name).next : legacyFiles(name).bin);
}
/** The older record marked as sent: the older app then starts from the server's copy, which
 * this format keeps up to date (its record keeps its own fields, only dirty is set). */
function legacySent(name: string, raw: Partial<Sync>, base: string | null): Promise<void> {
  return files.write(legacyFiles(name).sync, encodeSync({ ...raw, base, dirty: false }));
}

/** The older format's copy, made this format's (once: this format had no record yet), and the
 * older record marked as sent. Under localLock. */
async function takeOver(name: string): Promise<Sync | null> {
  const old = await legacyRecord(name);
  const blob = await legacyBlob(name, old?.sync ?? null);
  if (!blob) return null;
  const legacy = old?.sync.generation ?? null;
  const record: Sync = old
    ? { base: old.sync.base, dirty: old.sync.dirty, slot: 0, generation: crypto.randomUUID(), rev: 1, legacy }
    : { base: null, dirty: false, slot: 0, generation: crypto.randomUUID(), rev: 1, adopted: true, legacy };
  await files.write(blobName(name, 0), blob);
  await files.write(syncName(name), encodeSync(record));
  await legacySent(name, old?.raw ?? {}, old?.sync.base ?? null);
  return record;
}

/**
 * This format's record, after taking what an older app still open wrote since it was written
 * (its record dirty, of a generation this record did not see: review b5dacf4 1.4): that copy
 * is the newest of this browser and becomes this format's, unsent; this format's own unsent copy
 * it replaces is kept aside. Under localLock; the start's reads use it.
 */
async function readCurrent(name: string): Promise<Sync | null> {
  const sync = await readSync(name);
  if (!sync) return null;
  const old = await legacyRecord(name);
  if (!old?.sync.dirty || (old.sync.generation ?? null) === (sync.legacy ?? null)) return sync;
  const blob = await legacyBlob(name, old.sync);
  if (!blob) return sync;
  if (sync.dirty) {
    const mine = await readBlob(name, sync);
    if (mine && !same(mine, blob)) await stash(name, "local", mine);
  }
  const p = await localWrite(name, sync, blob, old.sync.base, false);
  await legacySent(name, old.raw, old.sync.base).catch((e: unknown) =>
    console.warn("saves: older record", e),
  );
  return p.record;
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

/** How a record of this browser stands to the bytes an upload sent. */
type Holds = "these" | "older" | "newer";

/**
 * After the server took a copy at `revision`: the record of the write `generation` is marked as
 * sent when it holds those bytes or older ones (`holds`), or only moved to the new base when it
 * holds a newer, still unsent copy. A record another write replaced meanwhile is left alone:
 * that write's own acknowledgement settles it. This is the fast path only: the upload named the
 * same generation to the server (`covers`), so a record this could not write (OPFS failing, the
 * page closed first) is still known to be superseded at the next start (review b5dacf4 1.1).
 */
async function acknowledge(name: string, generation: string | null, revision: string, holds: Holds) {
  await localLock(name, async () => {
    const current = await readSync(name);
    if (!current || current.generation !== generation) return;
    await publish(name, current, { ...current, base: revision, dirty: holds === "newer" });
  });
}

/** What a start's load found, each side said apart: the bytes the game starts from, and
 * whether this browser's copy and the server's hold them (review b5dacf4 1.3: the keeper took
 * both for done, and a side whose write failed at the start was never written again). */
export interface LoadOutcome {
  readonly data: Uint8Array<ArrayBuffer> | null;
  /** this browser's copy: the record holding `data`, or the older record (null: none) that a
   * later write of `data` replaces */
  readonly local:
    | { readonly kind: "holds"; readonly generation: string }
    | { readonly kind: "behind"; readonly generation: string | null };
  /** the server: holds `data` at `revision`, or not (to be sent) */
  readonly server: { readonly kind: "holds"; readonly revision: string } | { readonly kind: "behind" };
}

/** `data` loaded from elsewhere (another name's saves, moved): neither side of this name holds it. */
export const movedSaves = (data: Uint8Array<ArrayBuffer> | null): LoadOutcome => ({
  data,
  local: { kind: "behind", generation: null },
  server: { kind: "behind" },
});

/** Both sides hold what the start returned: nothing is left to write until the game changes it. */
export const settled = (o: LoadOutcome): boolean => o.local.kind === "holds" && o.server.kind === "holds";

/**
 * The blob an earlier session saved for `name` (null: none), and how each side stands to it.
 * With the server in reach: the server's, unless this browser's copy was saved after it was
 * last sent (offline, or the page closed before the upload) and no later upload of this browser
 * superseded it (the server's covered answer): then this browser's, sent now; when the server's
 * copy moved too (another device), this browser's still wins and the server's is kept beside it
 * (and in the server's revisions), `notice` told. Out of reach: this browser's copy, sent later.
 */
export async function loadSaves(name: string, notice?: (message: string) => void): Promise<LoadOutcome> {
  const url = backup(name);

  const before = await localLock(name, () => readCurrent(name));
  const holds = (generation: string | null | undefined, local: Uint8Array | null): LoadOutcome["local"] =>
    local && generation ? { kind: "holds", generation } : { kind: "behind", generation: generation ?? null };
  let remote: Uint8Array<ArrayBuffer> | null;
  let covered: boolean;
  try {
    // (offline: no tries at all; started from the app kept on the device, sw.ts)
    if (!navigator.onLine) throw new Error("offline");
    // (an unsent copy here: the server says whether an upload of this browser superseded it)
    const asked = before?.dirty ? before.generation : null;
    ({ data: remote, covered } = await withRetries(() => transport.load(url, asked), 3));
  } catch (e) {
    console.warn(`saves: server out of reach, starting from this browser's copy: ${String(e)}`);
    return localLock(name, async (): Promise<LoadOutcome> => {
      const sync = await readCurrent(name);
      const local = await readBlob(name, sync);
      transport.startedOffline(url, local, sync?.adopted ? null : sync?.base);
      // (a clean copy is what the server had at its base; an unsent one is sent once in reach)
      const server: LoadOutcome["server"] =
        local && sync && !sync.dirty && !sync.adopted && sync.base
          ? { kind: "holds", revision: sync.base }
          : { kind: "behind" };
      return { data: local, local: holds(sync?.generation, local), server };
    });
  }
  const revision = transport.revision(url) ?? '"0"';
  type Decided =
    | { readonly kind: "changed" | "decided"; readonly outcome: LoadOutcome }
    | {
        readonly kind: "send";
        readonly local: Uint8Array<ArrayBuffer> | null;
        readonly sync: Sync | null;
        readonly unsent: { readonly local: Uint8Array<ArrayBuffer>; readonly sync: Sync } | null;
      };
  const decided = await localLock(name, async (): Promise<Decided> => {
    const sync = await readCurrent(name);
    const local = await readBlob(name, sync);
    // (any change of the record while the GET was out, an acknowledgement's too: its answer
    // may be older than what this browser now has, and is not taken)
    if (before?.rev !== sync?.rev || before?.generation !== sync?.generation) {
      const server: LoadOutcome["server"] =
        local && sync && !sync.dirty && sync.base
          ? { kind: "holds", revision: sync.base }
          : { kind: "behind" };
      return { kind: "changed", outcome: { data: local, local: holds(sync?.generation, local), server } };
    }
    // (unsent here and not superseded: this browser's copy is the newest)
    const unsent = local && sync?.dirty && !covered ? { local, sync } : null;
    if (remote && !unsent) {
      if (local && sync?.adopted && !same(local, remote)) await stash(name, "local", local);
      // (this browser's copy not written: the record it would have replaced is behind)
      const kept = await localWrite(name, sync, remote, revision, true).then(
        (p) => holds(p.generation, remote),
        (e: unknown) => (console.warn("saves: local cache", e), holds(sync?.generation, null)),
      );
      return { kind: "decided", outcome: { data: remote, local: kept, server: { kind: "holds", revision } } };
    }
    return { kind: "send", local, sync, unsent: unsent ?? null };
  });
  if (decided.kind !== "send") return decided.outcome;
  const { local, sync, unsent } = decided;
  if (!local?.length && !unsent)
    return { data: local, local: holds(sync?.generation, local), server: { kind: "behind" } };
  if (unsent && remote && unsent.sync.base !== revision && !same(remote, unsent.local)) {
    await stash(name, "server", remote);
    console.warn(
      `saves: ${name}: saved here and on another device since ${unsent.sync.base}; this device's kept`,
    );
    notice?.(
      "別の端末のセーブより、この端末で後から保存したセーブを使います（別の端末のものも残してあります）",
    );
  }
  const generation = sync?.generation ?? null;
  // (not sent now: still dirty, sent by the keeper (the outcome says the server is behind), or
  // at the next start)
  const server = await upload(name, local!, generation, "these").then(
    (r): LoadOutcome["server"] => ({ kind: "holds", revision: r }),
    (e: unknown): LoadOutcome["server"] => (
      console.warn(`saves: not sent: ${String(e)}`),
      { kind: "behind" }
    ),
  );
  return { data: local, local: holds(generation, local), server };
}

/** persist under the caller's localLock, over `previous` (the record read under that same
 * hold: the write and what it replaced are one decision, never a later read; review b5dacf4 1.2) */
async function localWrite(
  name: string,
  previous: Sync | null,
  data: Uint8Array,
  base: string | null,
  clean: boolean,
): Promise<Persisted> {
  const slot = previous?.slot === 1 ? 0 : 1;
  const generation = crypto.randomUUID();
  // (the older format's record as this write saw it: one written after is the newer, 1.4)
  const legacy = (await legacyRecord(name))?.sync.generation ?? null;
  await files.write(blobName(name, slot), data);
  const record = await publish(name, previous, { base, dirty: !clean, generation, slot, legacy });
  return { generation, record };
}

/** Sends `data` to the server, naming the record of this browser it holds or supersedes
 * (`covers`), and settles that record (`holds`: how its bytes stand to `data`). Rejects when the
 * server did not take it. */
async function upload(
  name: string,
  data: Uint8Array<ArrayBuffer>,
  covers: string | null,
  holds: Holds,
): Promise<string> {
  const revision = await transport.store(backup(name), data, holds === "newer" ? null : covers);
  await acknowledge(name, covers, revision, holds).catch((e: unknown) =>
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

/** What a write of this browser's copy did, decided in the hold it was tried in. */
type Persist =
  | { readonly kind: "persisted"; readonly p: Persisted }
  /** not written: `stale` is the record it would have replaced (read in that same hold: a
   * record another tab wrote after it is not this one, review b5dacf4 1.2) */
  | { readonly kind: "failed"; readonly stale: string | null };

/** Writes `data` as this browser's copy (`clean`: the server has it; `base`: from the record it
 * replaces), its failure said, never thrown: a denied OPFS quota must not keep the server's copy
 * from being written. */
function persist(
  name: string,
  data: Uint8Array,
  base: (previous: Sync | null) => string | null,
  clean: boolean,
): Promise<Persist> {
  return localLock(name, async (): Promise<Persist> => {
    let previous: Sync | null;
    try {
      previous = await readSync(name);
    } catch (e) {
      console.warn("saves: local cache", e);
      return { kind: "failed", stale: null };
    }
    try {
      return { kind: "persisted", p: await localWrite(name, previous, data, base(previous), clean) };
    } catch (e) {
      console.warn("saves: local cache", e);
      return { kind: "failed", stale: previous?.generation ?? null };
    }
  }).catch((e: unknown): Persist => (console.warn("saves: local cache", e), { kind: "failed", stale: null }));
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
    const local = await persist(
      name,
      data,
      (previous) => transport.revision(url) ?? previous?.base ?? null,
      false,
    );
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
 * Keeps saving what the game writes under `roots` for `name`, from what the start loaded
 * (`start`: its bytes and what each side held of them). Returns a function that saves now
 * (`force`: the upload too, even in the pause after a failed one: when the game quits or the
 * page is hidden).
 *
 * Two lines of work, each one at a time: this browser's copy (export, OPFS write) and the
 * upload. A save waits for the previous local write only, never for an upload: a slow PUT held
 * the next save's export and OPFS write behind it, and a page closed meanwhile lost it (review
 * e18bf44). What each side holds is known by what it confirmed: `local` is the record this
 * browser's copy was last written as (or the one a failed write would have replaced), `last`
 * what the server took; neither is inferred from what was asked, the start's included (review
 * b5dacf4 1.3: a side the start could not write was taken for written).
 */
export function keepSaving(
  m: SavesModule,
  name: string,
  roots: string[],
  start: LoadOutcome,
  // (tests: the clock and the export; `notice`: what the player is told, once, when the server's
  // copy stopped taking this page's saves: another device saved since)
  opt: { now?: () => number; read?: () => Uint8Array<ArrayBuffer>; notice?: (message: string) => void } = {},
): (force?: boolean) => Promise<void> {
  const now = opt.now ?? Date.now;
  const read = opt.read ?? (() => exportSaves(m, roots));
  const url = backup(name);
  let seen = -1;
  // (the newest export, numbered: `seq` orders what the two sides hold; 0 is what this page
  // started from)
  let seq = 0;
  let target: { readonly data: Uint8Array<ArrayBuffer>; readonly seq: number } | null = start.data
    ? { data: start.data, seq: 0 }
    : null;
  // (this browser's record: `generation` holds export `seq` ("holds"), or is older than export
  // `seq` ("older": written before it, or what a failed write of it would have replaced))
  let local: { readonly kind: "holds" | "older"; readonly seq: number; readonly generation: string | null } =
    start.local.kind === "holds"
      ? { kind: "holds", seq: 0, generation: start.local.generation }
      : { kind: "older", seq: 0, generation: start.local.generation };
  // (what the server has, and its export's number: -1, not what this page started from)
  let last: Uint8Array | null = start.server.kind === "holds" ? start.data : null;
  let sentSeq = start.server.kind === "holds" ? 0 : -1;
  let failures = 0;
  let retryAt = 0;
  let told = false;
  const due = (force: boolean) => force || now() >= retryAt;
  /** how this browser's record stands to export `t` */
  const relation = (t: number): Holds =>
    local.kind === "holds"
      ? local.seq === t
        ? "these"
        : local.seq < t
          ? "older"
          : "newer"
      : local.seq <= t
        ? "older"
        : "newer";

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
    if (!t || (local.kind === "holds" && local.seq >= t.seq)) return;
    const r = await persist(name, t.data, () => transport.revision(url) ?? null, same(last, t.data));
    // (a failed one is tried again at the next look: the record is older than t)
    if (t.seq >= local.seq)
      local =
        r.kind === "persisted"
          ? { kind: "holds", seq: t.seq, generation: r.p.generation }
          : { kind: "older", seq: t.seq, generation: r.stale };
  };

  const uploadStep = async (force: boolean) => {
    const t = target;
    if (!t || sentSeq >= t.seq) return;
    // (the server has these bytes already: A, B not sent yet, then A again)
    if (same(last, t.data)) {
      sentSeq = t.seq;
      const revision = transport.revision(url);
      if (revision)
        await acknowledge(name, local.generation, revision, relation(t.seq)).catch((e: unknown) =>
          console.warn("saves: local cache", e),
        );
      return;
    }
    if (!due(force)) return;
    try {
      // (the record this upload holds or supersedes, named to the server as it stands now)
      const covers = relation(t.seq) === "newer" ? null : local.generation;
      const revision = await transport.store(url, t.data, covers);
      last = t.data;
      sentSeq = Math.max(sentSeq, t.seq);
      failures = 0;
      retryAt = now() + UPLOAD_EVERY;
      // (sent when it holds these bytes; an older one (its write failed) marked sent too, so a
      // start does not take it for the newest; a newer one moved to the new base, still unsent)
      await acknowledge(name, local.generation, revision, relation(t.seq)).catch((e: unknown) =>
        console.warn("saves: local cache", e),
      );
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
