// Where store chunks are kept on the device: the origin private file system (OPFS), one file a
// chunk in the folder CHUNK_DIR, named as the last part of its URL (its hex name). The network
// worker has its own copy (host/fetch-bridge.js masuOpfsStore), which reads with synchronous
// access handles (docs/s3-reads.md).
//
// A chunk's file is kept only once checked against its name (as every put of one is), so it
// reads back as checked. A file's writes land whole at close (createWritable writes a copy),
// but the file exists empty from its creation on: an empty one reads as not there (no chunk is
// empty).

import { pool } from "./jobs";
/** What net.ts, offline.ts and play.ts use of the device's chunks. */
export interface ChunkStore {
  match(request: Key): Promise<Response | undefined>;
  put(request: Key, response: Response): Promise<void>;
  delete(request: Key): Promise<boolean>;
  keys(): Promise<readonly { url: string }[]>;
  /** What is kept, with each one's bytes and when it was written (ms), none empty. */
  list(): Promise<Kept[]>;
}
/** A kept chunk: its key, bytes, when written (ms since 1970). */
export interface Kept {
  url: string;
  size: number;
  time: number;
}
/** How many files are opened at once to list a store (not all of them together: tens of
 * thousands of File objects on a phone's main thread). */
const LIST_AT_ONCE = 16;

/** Runs `f` over `xs`, `width` at a time, in order of the results. */
const batched = <T, R>(xs: readonly T[], width: number, f: (x: T) => Promise<R>): Promise<R[]> =>
  pool(
    width,
    xs.map((x) => () => f(x)),
  );

/** What `store` keeps with sizes and times. */
export function listChunks(store: ChunkStore): Promise<Kept[]> {
  return store.list();
}

/** Whether a store's put failed because the file is open elsewhere for a read (the network
 * worker's synchronous handle): then it is there already. */
const busy = (e: unknown) => e instanceof DOMException && e.name === "NoModificationAllowedError";

/** Empty files (writes cut short by a killed tab) older than this go at a listing. */
export const EMPTY_STALE_MS = 10 * 60 * 1000;
/** A URL, or what keys() gave. */
export type Key = string | URL | { url: string };

export const CHUNK_DIR = "masu-chunks";
const SIZE = "x-masu-size";

const urlOf = (r: Key) => (typeof r === "string" ? r : r instanceof URL ? r.href : r.url);

/** A key's file name: its URL's last part ("" for none: then not kept here). */
export function chunkFileName(request: Key): string {
  const u = urlOf(request);
  // (chunks/<ab>/<hex>, its folder its first two digits; this store's own keys opfs:/<dir>/<name>)
  const m = /\/([0-9a-f]{2}|masu-chunks)\/([0-9a-f]{32})$/.exec(u) ?? /^()([0-9a-f]{32})$/.exec(u);
  return m && (m[1] === "" || m[1] === "masu-chunks" || m[2]!.startsWith(m[1]!)) ? m[2]! : "";
}

const notFound = (e: unknown) =>
  e instanceof DOMException && (e.name === "NotFoundError" || e.name === "TypeMismatchError");

/** The store in folder `dir`. */
export function opfsStore(dir: FileSystemDirectoryHandle): ChunkStore {
  return {
    async match(request) {
      const name = chunkFileName(request);
      if (name) {
        try {
          const file = await (await dir.getFileHandle(name)).getFile();
          if (file.size > 0) return new Response(file, { headers: { [SIZE]: String(file.size) } });
        } catch (e) {
          if (!notFound(e)) throw e;
        }
      }
      return undefined;
    },
    // (a chunk checked against its name: net.ts storeChunk)
    async put(request, response) {
      const name = chunkFileName(request);
      if (!name) return;
      // (the body as it comes, not read whole first and copied again)
      if (response.headers.get(SIZE) === "0" || !response.body) return;
      let w: FileSystemWritableFileStream;
      try {
        w = await (await dir.getFileHandle(name, { create: true })).createWritable();
      } catch (e) {
        // (open for a read in the network worker: kept there already, as checked as this)
        if (busy(e)) return;
        throw e;
      }
      try {
        // (QuotaExceededError goes to the caller: net.ts storeChunk, isFull)
        const r = response.body.getReader();
        let n = 0;
        for (;;) {
          const { done, value } = await r.read();
          if (done) break;
          n += value.length;
          await w.write(value);
        }
        if (n) await w.close();
        else await w.abort();
      } catch (e) {
        await w.abort().catch(() => undefined);
        throw e;
      }
    },
    async delete(request) {
      const name = chunkFileName(request);
      let gone = false;
      if (name)
        try {
          await dir.removeEntry(name);
          gone = true;
        } catch {
          // (gone already, or open in the network worker for a read: stays, trimmed another time)
        }
      return gone;
    },
    async keys() {
      const out: { url: string }[] = [];
      for await (const name of (dir as unknown as { keys(): AsyncIterable<string> }).keys())
        if (chunkFileName(name)) out.push({ url: `opfs:/${CHUNK_DIR}/${name}` });
      return out;
    },
    async list() {
      const handles: FileSystemFileHandle[] = [];
      for await (const h of (dir as unknown as { values(): AsyncIterable<FileSystemHandle> }).values())
        if (h.kind === "file" && chunkFileName(h.name)) handles.push(h as FileSystemFileHandle);
      const now = Date.now();
      const files = await batched(handles, LIST_AT_ONCE, async (h) => {
        try {
          const f = await h.getFile();
          // (an empty one: being written now, or a write a killed tab cut short; those go)
          if (!f.size && now - f.lastModified > EMPTY_STALE_MS)
            await dir.removeEntry(h.name).catch(() => undefined);
          return { url: `opfs:/${CHUNK_DIR}/${h.name}`, size: f.size, time: f.lastModified };
        } catch {
          return { url: "", size: 0, time: 0 };
        }
      });
      return files.filter((f) => f.size > 0);
    },
  };
}

let opened: Promise<FileSystemDirectoryHandle | null> | undefined;

/** The device's chunk store: OPFS, else none. */
export async function deviceChunks(): Promise<ChunkStore | null> {
  // (the folder opened once; none: asked again next time)
  const dir = await (opened ??= (async () => {
    try {
      return await (await navigator.storage.getDirectory()).getDirectoryHandle(CHUNK_DIR, { create: true });
    } catch {
      return null;
    }
  })());
  if (!dir) opened = undefined;
  return dir ? opfsStore(dir) : null;
}
