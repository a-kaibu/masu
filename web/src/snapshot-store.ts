// This browser's save states (OPFS states/<game>/): the bodies (<hash>.snap, <slot>.snap for
// states from before hashes) and index.json, the slots and the body each one holds. This module
// is the only one that writes either: every change of the index, and every removal of a body
// the index no longer names, runs under one lock per game (Web Locks, so other tabs of this
// origin wait too), with the body written first and the index read again inside the lock. Two
// tabs saving to two slots at once then both stay listed, each slot's body kept.
//
// Two ways in, kept apart so a download cannot take a slot back to what it was:
// - publish: a state just taken goes into its slot (whatever the slot held before);
// - cache: a body fetched from the server is kept, its slot moved to it only when that entry is
//   newer than what the slot holds now (a slow download of an older state leaves a newer one be).
import type { StateInfo } from "./savestate";

/** The lock's name for `game` (every tab of the origin). */
export const stateLockName = (game: string) => `masu-states:${game}`;

/** Proof that the caller holds `game`'s lock: made by withStateLock only, dead after it. */
class StateLock {
  #live = true;
  private constructor(readonly game: string) {}
  static async hold<T>(game: string, f: (held: StateLock) => Promise<T>): Promise<T> {
    const held = new StateLock(game);
    try {
      return await f(held);
    } finally {
      held.#live = false;
    }
  }
  check(game: string): void {
    if (!this.#live) throw new Error(`state lock of ${this.game} used after it was released`);
    if (game !== this.game) throw new Error(`state lock of ${this.game} used for ${game}`);
  }
}
export type { StateLock };

// (a page without Web Locks still keeps its own calls in order)
const pageTails = new Map<string, Promise<unknown>>();
function withStateLock<T>(game: string, f: (held: StateLock) => Promise<T>): Promise<T> {
  const locks = (globalThis.navigator as Navigator | undefined)?.locks;
  if (locks) return locks.request(stateLockName(game), () => StateLock.hold(game, f));
  const run = (pageTails.get(game) ?? Promise.resolve()).catch(() => {}).then(() => StateLock.hold(game, f));
  pageTails.set(game, run);
  return run;
}

/** A body's file name: its hash, or its slot for states from before hashes. */
export const localName = (info: Pick<StateInfo, "slot" | "hash">) =>
  info.hash ? `${info.hash}.snap` : `${info.slot}.snap`;

export const parseIndex = (b: Uint8Array | null): StateInfo[] => {
  try {
    return b ? (JSON.parse(new TextDecoder().decode(b)) as { states: StateInfo[] }).states : [];
  } catch {
    return [];
  }
};
export const encodeIndex = (v: StateInfo[]) => new TextEncoder().encode(JSON.stringify({ states: v }));

/** The newer of each slot's two entries. */
export function merge(a: StateInfo[], b: StateInfo[]): StateInfo[] {
  const m = new Map<number, StateInfo>();
  for (const s of [...a, ...b]) {
    const had = m.get(s.slot);
    if (!had || had.created < s.created) m.set(s.slot, s);
  }
  return [...m.values()].sort((x, y) => x.slot - y.slot);
}

async function dir(game: string, create: boolean): Promise<FileSystemDirectoryHandle | null> {
  try {
    const root = await navigator.storage.getDirectory();
    const states = await root.getDirectoryHandle("states", { create });
    return await states.getDirectoryHandle(game, { create });
  } catch {
    return null;
  }
}

/** `name` of `game`'s folder, null when there is none. Reading needs no lock: a body is written
 * whole before any index names it, and an index file is replaced whole. */
export async function readLocal(game: string, name: string): Promise<Uint8Array<ArrayBuffer> | null> {
  try {
    const f = await (await dir(game, false))?.getFileHandle(name);
    return f ? new Uint8Array(await (await f.getFile()).arrayBuffer()) : null;
  } catch {
    return null;
  }
}

/** This browser's index of `game`. */
export const localIndex = async (game: string): Promise<StateInfo[]> =>
  parseIndex(await readLocal(game, "index.json"));

async function writeBody(held: StateLock, game: string, name: string, data: Uint8Array<ArrayBuffer>) {
  held.check(game);
  const d = await dir(game, true);
  if (!d) throw new Error("この端末に保存できません");
  const w = await (await d.getFileHandle(name, { create: true })).createWritable();
  await w.write(data);
  await w.close();
}

/** Writes `index` as `game`'s index, then removes the bodies it does not name. Both under the
 * lock: no other tab's body is waiting for its index entry meanwhile (it is written inside its
 * own hold of the same lock). */
async function writeIndex(held: StateLock, game: string, index: StateInfo[]): Promise<void> {
  await writeBody(held, game, "index.json", encodeIndex(index));
  await pruneBodies(held, game, index).catch(() => {});
}

async function pruneBodies(held: StateLock, game: string, index: StateInfo[]): Promise<void> {
  held.check(game);
  const d = await dir(game, false);
  if (!d) return;
  const keep = new Set(index.map(localName));
  const gone: string[] = [];
  for await (const name of (d as unknown as { keys(): AsyncIterable<string> }).keys())
    if (name.endsWith(".snap") && !keep.has(name)) gone.push(name);
  for (const name of gone) await d.removeEntry(name).catch(() => {});
}

/** Puts `file` into `info.slot` of `game` (in place of whatever the slot held). */
export function publish(game: string, info: StateInfo, file: Uint8Array<ArrayBuffer>): Promise<void> {
  return withStateLock(game, async (held) => {
    await writeBody(held, game, localName(info), file);
    const index = [...(await localIndex(game)).filter((s) => s.slot !== info.slot), info].sort(
      (x, y) => x.slot - y.slot,
    );
    await writeIndex(held, game, index);
  });
}

/** What cache did with a fetched body. */
export type Cached = "kept" | "adopted" | "older";

/** Keeps `file`, the body `info` names, fetched from the server. The slot moves to `info` only
 * when `info` is newer than the slot's entry here (merge's rule); the body of an older entry is
 * not written at all (no index would name it). */
export function cache(game: string, info: StateInfo, file: Uint8Array<ArrayBuffer>): Promise<Cached> {
  return withStateLock(game, async (held) => {
    const index = await localIndex(game);
    const now = index.find((s) => s.slot === info.slot);
    // (a hash names the same bytes whenever; a slot-named body only the entry it was of)
    if (now && localName(now) === localName(info) && (info.hash || now.created === info.created)) {
      await writeBody(held, game, localName(info), file);
      return "kept";
    }
    const merged = merge(index, [info]);
    if (merged.find((s) => s.slot === info.slot) !== info) return "older";
    await writeBody(held, game, localName(info), file);
    await writeIndex(held, game, merged);
    return "adopted";
  });
}
