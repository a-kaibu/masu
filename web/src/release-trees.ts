// sake's C:\ (prefix32: the fonts and the RPG Maker RTPs) and the benchmark's programs (bench)
// are made by the release (`masu release-trees`, .github/workflows/release.yml's pages job) and
// put on its GitHub Pages as they are in a store (store/trees/<name>.tsv, store/chunks/ab/<hex>:
// plain zstd, named by their bytes), not in the bucket: the page reads them from there itself,
// near the player whatever the server's line (worker1's uplink kept 0.4.1 putting them in the
// bucket for a quarter of an hour, 2026-11-17). A page built with no CDN (the tests, a local
// build) reads them from the store as any tree (masu serve --dev-bucket puts them there).
import { treeChunks } from "./net";

/** The release's GitHub Pages (the page's build, VITE_MASU_WASM_CDN; wasmsource.ts), "" for none. */
export const CDN: string = (import.meta.env.VITE_MASU_WASM_CDN as string | undefined) ?? "";

/** The trees a release puts there (crates/masu/src/pack.rs release_trees). */
export const RELEASE_TREES = ["prefix32", "bench"] as const;
export type ReleaseTree = (typeof RELEASE_TREES)[number];

export const isReleaseTree = (name: string): name is ReleaseTree =>
  (RELEASE_TREES as readonly string[]).includes(name);

/** Where a chunk is: the bucket (through /store-direct), or a plain URL on the release's Pages. */
export type ChunkHome = { kind: "bucket" } | { kind: "pages"; url: string };

/** The release's trees' chunks (hex -> their URL there), as the page read their lists. */
const onPages = new Map<string, string>();

/** Where tree `name`'s list is read from: the release's Pages for its own trees (with a CDN),
 * else the store (`store`: its /store/ URL). */
export function treeListUrl(name: string, store: string, cdn: string = CDN): string {
  const path = `trees/${encodeURIComponent(name)}.tsv`;
  return cdn && isReleaseTree(name) ? new URL(`store/${path}`, cdn).href : new URL(path, store).href;
}

/** Tree `name`'s list `tsv`, as read from treeListUrl: a release tree's chunks are then read
 * from the same Pages. */
export function noteTree(name: string, tsv: string, cdn: string = CDN): void {
  if (!cdn || !isReleaseTree(name)) return;
  for (const hex of treeChunks(tsv))
    onPages.set(hex, new URL(`store/chunks/${hex.slice(0, 2)}/${hex}`, cdn).href);
}

export function chunkHome(hex: string): ChunkHome {
  const url = onPages.get(hex);
  return url === undefined ? { kind: "bucket" } : { kind: "pages", url };
}

/** What the network worker needs to read them (host/fetch-bridge.js masuPagesInit): hex -> URL. */
export function pagesChunks(): Record<string, string> {
  return Object.fromEntries(onPages);
}

/** (tests) */
export function forgetTrees(): void {
  onPages.clear();
}
