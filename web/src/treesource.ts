// Where a web game's tree (treeread.ts) gets its chunks: the device (OPFS, chunkstore.ts) or
// straight from the bucket (direct.ts), each kept on the device once fetched. Never the server.
import { blake3Hex } from "./blake3";
import { CLASS } from "./budget";
import { directFetcher } from "./direct";
import { chunkCache, chunkPath, fetchBoot, keepChunk } from "./net";
import { parseTree, TreeReader } from "./treeread";

/** Tree `name`'s files; `page`: the app page's URL (the store is store/ beside it). */
export async function openTree(name: string, page: string): Promise<TreeReader> {
  const store = new URL("store/", page).href;
  const tsv = new TextDecoder().decode(
    await fetchBoot(new URL(`trees/${encodeURIComponent(name)}.tsv`, store)),
  );
  const device = await chunkCache().catch(() => null);
  const direct = directFetcher(store, { cls: CLASS.game });
  return new TreeReader(parseTree(tsv), async (hex) => {
    const url = new URL(chunkPath(hex), store).href;
    const hit = await device?.match(url).catch(() => undefined);
    if (hit) {
      const d = new Uint8Array(await hit.arrayBuffer());
      if (hit.headers.get("x-masu-checked") === "1" || blake3Hex(d) === hex) return d;
      await device?.delete(url).catch(() => false);
    }
    const d = await direct.get(hex);
    if (!d) throw new Error(`chunk ${hex}: not to be had from the bucket`);
    if (device) void keepChunk(device, url, d);
    return d;
  });
}
