import { afterEach, describe, expect, it } from "vite-plus/test";
import { blake3Hex } from "./blake3";
import { directFetcher } from "./direct";
import {
  chunkHome,
  forgetTrees,
  isReleaseTree,
  noteTree,
  pagesChunks,
  RELEASE_TREES,
  treeListUrl,
} from "./release-trees";

type B = Uint8Array<ArrayBuffer>;
const urlOf = (i: RequestInfo | URL) => (typeof i === "string" ? i : i instanceof URL ? i.href : i.url);
const CDN = "https://a-kaibu.github.io/masu/";
const STORE = "https://server.example/store/";
const bytes = (n: number, seed: number): B => {
  let x = seed >>> 0;
  return Uint8Array.from({ length: n }, () => ((x = (Math.imul(x, 1103515245) + 12345) >>> 0) >>> 16) & 255);
};
// (the "zstd" of these tests, as direct.test.ts's: a marker byte before the bytes)
const frame = (d: Uint8Array): B => {
  const f = new Uint8Array(d.length + 1);
  f[0] = 0x5a;
  f.set(d, 1);
  return f;
};
const unzstd = async (f: Uint8Array) => {
  if (f[0] !== 0x5a) throw new Error("not a frame");
  return f.slice(1);
};
/** A tree list (crates/masu/src/pack.rs) of one file made of `chunks`. */
const tsv = (chunks: Uint8Array[]) =>
  `masu-tree 1\nf\t644\t0\tdrive_c/a.bin\t0\t${chunks.map((c) => `${blake3Hex(c)}:${c.length}`).join(",")}\n`;

afterEach(forgetTrees);

describe("the release's trees (sake's C:\\, the benchmark) on its Pages", () => {
  it("reads their lists from the Pages with a CDN, every other tree's from the store", () => {
    expect([...RELEASE_TREES]).toEqual(["prefix32", "bench"]);
    expect(treeListUrl("prefix32", STORE, CDN)).toBe(`${CDN}store/trees/prefix32.tsv`);
    expect(treeListUrl("bench", STORE, CDN)).toBe(`${CDN}store/trees/bench.tsv`);
    expect(treeListUrl("game", STORE, CDN)).toBe(`${STORE}trees/game.tsv`);
    // (no CDN: the tests' and a local build's page, the store has them: --dev-bucket)
    expect(treeListUrl("prefix32", STORE, "")).toBe(`${STORE}trees/prefix32.tsv`);
    expect(treeListUrl("a b", STORE, CDN)).toBe(`${STORE}trees/a%20b.tsv`);
    for (const t of ["prefix", "prefix32 ", "Prefix32", "bench2", ""])
      expect(isReleaseTree(t), t).toBe(false);
  });

  it("their chunks are on the Pages once the list was read there; others, and with no CDN, in the bucket", () => {
    const [a, b, c] = [bytes(1000, 1), bytes(2000, 2), bytes(300, 3)];
    const [ha, hb, hc] = [a, b, c].map(blake3Hex);
    expect(chunkHome(ha!)).toEqual({ kind: "bucket" });
    noteTree("prefix32", tsv([a, b]), CDN);
    noteTree("game", tsv([c]), CDN);
    noteTree("bench", tsv([c]), "");
    expect(chunkHome(ha!)).toEqual({ kind: "pages", url: `${CDN}store/chunks/${ha!.slice(0, 2)}/${ha}` });
    expect(chunkHome(hb!).kind).toBe("pages");
    expect(chunkHome(hc!)).toEqual({ kind: "bucket" });
    expect(Object.keys(pagesChunks()).sort()).toEqual([ha, hb].sort());
  });

  it("directFetcher takes them from the Pages, checked against their names, and never asks the server", async () => {
    const good = bytes(5000, 7);
    const hex = blake3Hex(good);
    const bad = bytes(64, 8);
    const badHex = "0".repeat(32);
    noteTree("prefix32", tsv([good]), CDN);
    noteTree("bench", `masu-tree 1\nf\t644\t0\tx\t0\t${badHex}:64\n`, CDN);
    const asked: string[] = [];
    let damaged = true;
    const request = (async (i: RequestInfo | URL, init?: RequestInit) => {
      const u = urlOf(i);
      asked.push(`${init?.method ?? "GET"} ${u}`);
      if (u === `${CDN}store/chunks/${hex.slice(0, 2)}/${hex}`) {
        const f = frame(good);
        // (the first copy damaged: fetched again)
        if (damaged) {
          f[10] ^= 1;
          damaged = false;
        }
        return new Response(f);
      }
      if (u === `${CDN}store/chunks/00/${badHex}`) return new Response(frame(bad));
      return new Response(null, { status: 404 });
    }) as typeof fetch;
    const f = directFetcher(STORE, { request, unzstd });
    await f.prime([hex, badHex]);
    expect(asked, "nothing to ask the server for").toEqual([]);
    const got = await f.get(hex);
    expect(got && blake3Hex(got)).toBe(hex);
    expect(await f.get(badHex), "never its name: given up on").toBeNull();
    expect(
      asked.every((a) => a.startsWith(`GET ${CDN}store/chunks/`)),
      asked.join("\n"),
    ).toBe(true);
    expect(asked.filter((a) => a.includes(hex)).length).toBe(2);
  });
});
