// A web game's files (NW.js / Electron pages, html.ts), read by the app's page itself: the
// tree's list (trees/<name>.tsv) says which store chunks make each file, and the chunks come
// from the device (OPFS, chunkstore.ts) or straight from the bucket (direct.ts: presigned URL,
// AES-GCM, zstd, BLAKE3), never through the server. The game's frame (another origin) gets them
// from its service worker (game-sw.ts), which asks this page over a MessagePort (gameport.ts).
import { workers } from "./jobs";

/** A store chunk's name: 32 lower-case hex digits (the first 16 bytes of its BLAKE3). */
export type ChunkHex = string & { readonly __chunkHex: unique symbol };

const HEX = /^[0-9a-f]{32}$/;

export function chunkHex(s: string): ChunkHex | null {
  return HEX.test(s) ? (s as ChunkHex) : null;
}

/** One chunk of a file: stored (`hex`), or all zeros (null: not stored). */
export interface TreeChunk {
  readonly hex: ChunkHex | null;
  readonly start: number;
  readonly len: number;
}

/** A file of a tree: its size and its chunks in order (lengths adding up to the size). */
export interface TreeFile {
  readonly path: string;
  readonly size: number;
  readonly chunks: readonly TreeChunk[];
}

/** A chunk's length at most (crates/kura/src/tree_list.rs CHUNK_MAX). */
export const CHUNK_MAX = 512 << 20;

/** A path or link target as itself: `\t`, `\n`, `\\` (tree_list.rs esc). */
export function unesc(s: string): string {
  return s.replace(/\\([\\tn])/g, (_, c: string) => (c === "t" ? "\t" : c === "n" ? "\n" : "\\"));
}

/** A file line's chunks field against its size; null when it is not one (as tree_list.rs Chunks::parse). */
function chunksOf(field: string, size: number): TreeChunk[] | null {
  const out: TreeChunk[] = [];
  let at = 0;
  for (const c of field.split(",")) {
    if (!c) continue;
    const colon = c.indexOf(":");
    if (colon < 0) return null;
    const [h, n] = [c.slice(0, colon), c.slice(colon + 1)];
    const hex = h === "" ? null : chunkHex(h);
    if (h !== "" && !hex) return null;
    if (!/^\d+$/.test(n)) return null;
    const len = Number(n);
    if (len < 1 || len > CHUNK_MAX) return null;
    out.push({ hex, start: at, len });
    at += len;
    if (!Number.isSafeInteger(at)) return null;
  }
  return at === size ? out : null;
}

/**
 * The files of a tree's list by path in lower case (paths match as on Windows); lines that are
 * not whole ones are left out, as every reader of tree_list.rs does. Folders and links: not files.
 */
export function parseTree(tsv: string): Map<string, TreeFile> {
  const files = new Map<string, TreeFile>();
  const lines = tsv.split("\n");
  for (const line of lines.slice(1)) {
    const f = line.split("\t");
    if (f[0] !== "f" || f.length !== 6) continue;
    if (!/^[0-7]+$/.test(f[1]!) || !/^-?\d+$/.test(f[2]!) || !/^\d+$/.test(f[4]!)) continue;
    const size = Number(f[4]);
    if (!Number.isSafeInteger(size)) continue;
    const chunks = chunksOf(f[5]!, size);
    if (!chunks) continue;
    const path = unesc(f[3]!);
    files.set(path.toLowerCase(), { path, size, chunks });
  }
  return files;
}

/** The chunk holding byte `at` of `f` (at < size). */
export function chunkAt(f: TreeFile, at: number): number {
  let lo = 0;
  let hi = f.chunks.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (f.chunks[mid]!.start <= at) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

/** A chunk's bytes by name (decoded and checked), `len` of them. */
export type GetChunk = (hex: ChunkHex, len: number) => Promise<Uint8Array>;

/** Decoded chunks kept for the next reads (a media element asking for small ranges one after
 * another: the same chunk each time), this many bytes in all. */
export const DECODED_MAX = 64 << 20;
/** Chunks of one read fetched at once. */
export const AHEAD = 4;

/** A tree's files over `get`: what was decoded last kept (DECODED_MAX bytes, newest last). */
export class TreeReader {
  private readonly kept = new Map<ChunkHex, Promise<Uint8Array>>();
  private keptBytes = 0;
  private readonly sizes = new Map<ChunkHex, number>();

  constructor(
    readonly files: ReadonlyMap<string, TreeFile>,
    private readonly get: GetChunk,
  ) {}

  file(path: string): TreeFile | undefined {
    return this.files.get(path.toLowerCase());
  }

  private chunk(c: TreeChunk): Promise<Uint8Array> {
    if (!c.hex) return Promise.resolve(new Uint8Array(c.len));
    const hex = c.hex;
    const had = this.kept.get(hex);
    if (had) {
      this.kept.delete(hex);
      this.kept.set(hex, had);
      return had;
    }
    const p = this.get(hex, c.len).then((d) => {
      if (d.length !== c.len) throw new Error(`chunk ${hex}: ${d.length} bytes, not ${c.len}`);
      return d;
    });
    if (c.len <= DECODED_MAX / 4) {
      this.kept.set(hex, p);
      this.sizes.set(hex, c.len);
      this.keptBytes += c.len;
      // (a failed one is not kept: asked for again next time)
      p.catch(() => this.drop(hex, p));
      for (const [k, v] of this.kept) {
        if (this.keptBytes <= DECODED_MAX) break;
        this.drop(k, v);
      }
    }
    return p;
  }

  private drop(hex: ChunkHex, p: Promise<Uint8Array>): void {
    if (this.kept.get(hex) !== p) return;
    this.kept.delete(hex);
    this.keptBytes -= this.sizes.get(hex) ?? 0;
    this.sizes.delete(hex);
  }

  /** Bytes [from, to) of `f` (to past its end: its end). */
  async read(f: TreeFile, from: number, to: number): Promise<Uint8Array> {
    const end = Math.min(to, f.size);
    if (from >= end) return new Uint8Array(0);
    const first = chunkAt(f, from);
    const last = chunkAt(f, end - 1);
    const out = new Uint8Array(end - from);
    let next = first;
    const one = async (_: number, signal: AbortSignal) => {
      while (next <= last && !signal.aborted) {
        const c = f.chunks[next++]!;
        const d = await this.chunk(c);
        const a = Math.max(from, c.start);
        const b = Math.min(end, c.start + c.len);
        out.set(d.subarray(a - c.start, b - c.start), a - from);
      }
    };
    await workers(Math.min(AHEAD, last - first + 1), one);
    return out;
  }

  /** The file at `path` whole, or null when the tree has none. */
  async bytes(path: string): Promise<Uint8Array | null> {
    const f = this.file(path);
    return f ? this.read(f, 0, f.size) : null;
  }

  async text(path: string): Promise<string | null> {
    const b = await this.bytes(path);
    return b && new TextDecoder().decode(b);
  }
}

export function mime(path: string): string {
  const ext = /\.([^./]*)$/.exec(path)?.[1]?.toLowerCase() ?? "";
  switch (ext) {
    case "html":
    case "htm":
      return "text/html; charset=utf-8";
    case "js":
    case "mjs":
      return "text/javascript; charset=utf-8";
    case "json":
      return "application/json";
    case "css":
      return "text/css";
    case "txt":
    case "ks":
    case "tjs":
      return "text/plain; charset=utf-8";
    case "png":
      return "image/png";
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    case "gif":
      return "image/gif";
    case "webp":
      return "image/webp";
    case "svg":
      return "image/svg+xml";
    case "ogg":
    case "oga":
      return "audio/ogg";
    case "m4a":
      return "audio/mp4";
    case "mp3":
      return "audio/mpeg";
    case "wav":
      return "audio/wav";
    case "webm":
      return "video/webm";
    case "mp4":
    case "m4v":
      return "video/mp4";
    case "ogv":
      return "video/ogg";
    case "woff":
      return "font/woff";
    case "woff2":
      return "font/woff2";
    case "ttf":
      return "font/ttf";
    case "otf":
      return "font/otf";
    case "wasm":
      return "application/wasm";
    default:
      return "application/octet-stream";
  }
}

/** A u64 as Rust's `str::parse::<u64>` reads one (an optional "+", digits), or null. */
function u64(s: string): bigint | null {
  if (!/^\+?\d+$/.test(s)) return null;
  const v = BigInt(s);
  return v < 1n << 64n ? v : null;
}

/** `bytes=a-b` / `bytes=a-` / `bytes=-n` against `size`: [first, last] byte; "bad" for one that
 * is not satisfiable (or not one range of bytes); null for no Range. As the server read it
 * (crates/masu/src/treefiles.rs before the files moved here: testdata/webgame/oracle.rs). */
export function byteRange(header: string | null, size: number): [number, number] | "bad" | null {
  if (header === null) return null;
  const v = header.trim();
  if (!v.startsWith("bytes=") || size < 1) return "bad";
  const rest = v.slice(6);
  const dash = rest.indexOf("-");
  if (dash < 0) return "bad";
  const [a, b] = [rest.slice(0, dash).trim(), rest.slice(dash + 1).trim()];
  const n = BigInt(size);
  const last = n - 1n;
  let r: [bigint, bigint];
  if (a === "") {
    const k = u64(b);
    if (k === null || k === 0n) return "bad";
    r = [k >= n ? 0n : n - k, last];
  } else {
    const x = u64(a);
    if (x === null) return "bad";
    if (b === "") r = [x, last];
    else {
      const y = u64(b);
      if (y === null) return "bad";
      r = [x, y < last ? y : last];
    }
  }
  return r[0] <= r[1] ? [Number(r[0]), Number(r[1])] : "bad";
}

/** Pages bigger than this go out as they are (no game's start page is). */
export const MAX_PAGE = 16 << 20;

const HOOK = new TextEncoder().encode('<script src="/frame.js"></script>');

/** A page with the frame's script first (frame.ts): after `<head>` when it has one. A ".htm"
 * that is not markup (SKYNovel's encrypted frames: base64 the game decrypts after an XHR, #234)
 * goes out as it is: the script in front of it would make it undecryptable. */
export function withHook(page: Uint8Array): Uint8Array {
  let i = page[0] === 0xef && page[1] === 0xbb && page[2] === 0xbf ? 3 : 0;
  while (
    i < page.length &&
    (page[i] === 0x20 || page[i] === 0x09 || page[i] === 0x0a || page[i] === 0x0c || page[i] === 0x0d)
  )
    i++;
  if (page[i] !== 0x3c) return page;
  const lower = (c: number) => (c >= 0x41 && c <= 0x5a ? c | 0x20 : c);
  const head = [0x3c, 0x68, 0x65, 0x61, 0x64]; // "<head"
  let at = 0;
  for (let p = 0; p + head.length <= page.length; p++) {
    if (!head.every((c, k) => lower(page[p + k]!) === c)) continue;
    const gt = page.indexOf(0x3e, p);
    if (gt >= 0) at = gt + 1;
    break;
  }
  const out = new Uint8Array(page.length + HOOK.length);
  out.set(page.subarray(0, at));
  out.set(HOOK, at);
  out.set(page.subarray(at), at + HOOK.length);
  return out;
}
