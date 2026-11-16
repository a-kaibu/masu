import { expect, test } from "vite-plus/test";
import corpusText from "../../testdata/webgame/tree-rules.jsonl?raw";
import {
  answerReads,
  ASK_MAX,
  isGivePort,
  isReadAnswer,
  isReadAsk,
  isTreeName,
  isWantPort,
  treePath,
  type ReadAnswer,
} from "./gameport";
import {
  byteRange,
  chunkAt,
  chunkHex,
  DECODED_MAX,
  mime,
  parseTree,
  TreeReader,
  unesc,
  withHook,
  type ChunkHex,
} from "./treeread";

/** xorshift64, as the Rust tests' (seeded: a failure comes back the same) */
function rng(seed = 0x853c49e6748fea9bn) {
  let s = seed;
  const M = (1n << 64n) - 1n;
  return (n: number) => {
    s ^= (s << 13n) & M;
    s ^= s >> 7n;
    s ^= (s << 17n) & M;
    return Number(s % BigInt(n));
  };
}

const hexOf = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
const bytesOf = (h: string) => Uint8Array.from(h.match(/../g) ?? [], (x) => parseInt(x, 16));

// Differential: the server's own rules before the files moved to the page (crates/masu/src/
// treefiles.rs range and with_hook, run over seeded inputs by testdata/webgame/oracle.rs).
const corpus = corpusText
  .split("\n")
  .filter(Boolean)
  .map((l: string) => JSON.parse(l) as { range?: string; size?: number; page?: string; out: unknown });

test("byteRange answers as the server did for every Range of the corpus", () => {
  const ranges = corpus.filter((c) => c.range !== undefined);
  expect(ranges.length).toBeGreaterThan(1000);
  expect(ranges.filter((c) => Array.isArray(c.out)).length).toBeGreaterThan(300);
  for (const c of ranges)
    expect([c.range, c.size, byteRange(c.range!, c.size!)]).toEqual([c.range, c.size, c.out]);
  expect(byteRange(null, 10)).toBe(null);
});

test("withHook puts the frame's script where the server did, for every page of the corpus", () => {
  const pages = corpus.filter((c) => c.page !== undefined);
  expect(pages.length).toBeGreaterThan(500);
  for (const c of pages) expect([c.page, hexOf(withHook(bytesOf(c.page!)))]).toEqual([c.page, c.out]);
});

test("withHook: after <head>, or first in markup; base64 and text as they are", () => {
  const t = (s: string) => new TextDecoder().decode(withHook(new TextEncoder().encode(s)));
  const HOOK = '<script src="/frame.js"></script>';
  expect(t("<html><head><title>x</title></head></html>")).toBe(
    `<html><head>${HOOK}<title>x</title></head></html>`,
  );
  expect(t("<p>no head")).toBe(`${HOOK}<p>no head`);
  expect(t("fObKeb37k/BjAR+2h6weLbn19Tm8vwK6PvL4BeSN")).toBe("fObKeb37k/BjAR+2h6weLbn19Tm8vwK6PvL4BeSN");
  expect(t("")).toBe("");
});

test("byteRange: any header against any size is none, bad, or bytes inside the file (PBT)", () => {
  const r = rng(7n);
  const parts = ["bytes=", "-", " ", "0", "1", "9", "42", "x", ",", "999999999999"];
  for (let i = 0; i < 20000; i++) {
    let h = "";
    for (let k = r(7); k > 0; k--) h += parts[r(parts.length)];
    const size = [0, 1, 2, 3, 100, 2 ** 53 - 1][r(6)]!;
    const got = byteRange(h, size);
    if (Array.isArray(got)) {
      expect(got[0]).toBeLessThanOrEqual(got[1]);
      expect(got[1]).toBeLessThan(size);
      expect(got[0]).toBeGreaterThanOrEqual(0);
    } else expect(got).toBe("bad");
  }
});

test("mime as the server's table", () => {
  expect(mime("a/B.HTML")).toBe("text/html; charset=utf-8");
  expect(mime("x.Mp4")).toBe("video/mp4");
  expect(mime("noext")).toBe("application/octet-stream");
  expect(mime("dir.d/file")).toBe("application/octet-stream");
});

/** tree_list.rs esc */
const esc = (s: string) => s.replaceAll("\\", "\\\\").replaceAll("\t", "\\t").replaceAll("\n", "\\n");

test("unesc undoes tree_list.rs esc for any path (PBT)", () => {
  const r = rng(11n);
  const alpha = ["a", "\\", "\t", "\n", "t", "n", "日", "/", " "];
  for (let i = 0; i < 5000; i++) {
    let p = "";
    for (let k = r(10); k > 0; k--) p += alpha[r(alpha.length)];
    expect(unesc(esc(p))).toBe(p);
  }
});

const h = (i: number) => chunkHex(i.toString(16).padStart(32, "0"))!;

test("parseTree takes whole file lines only (as tree_list.rs good_lines)", () => {
  const a = h(0xab);
  const files = parseTree(
    [
      "vnvm-tree\t2\t4",
      `d\t755\t0\tdir`,
      `f\t644\t0\tok\t2\t${a}:2`,
      `f\t644\t0\tWWW/Index.html\t3\t${a}:2,:1`,
      `f\t644\t0\tx\t2\t${a}:2\textra`,
      `f\t644\t0\tshort\t2\tab:2`,
      `f\t644\t0\tnochunks\t2\t:2`,
      `f\t644\t0\twrong\t3\t${a}:2`,
      `f\t644\t0\tnolength\t2\t${a}`,
      `f\t644\t0\tzero\t0\t${a}:0`,
      `f\t644\t0\tempty\t0\t`,
      `f\t9\t0\tbadmode\t2\t${a}:2`,
      `f\t644\t0\ttab\\tname\t2\t${a}:2`,
      `l\t777\t0\tlink\tok`,
    ].join("\n"),
  );
  expect([...files.keys()].sort()).toEqual(["empty", "nochunks", "ok", "tab\tname", "www/index.html"]);
  expect(files.get("www/index.html")).toEqual({
    path: "WWW/Index.html",
    size: 3,
    chunks: [
      { hex: a, start: 0, len: 2 },
      { hex: null, start: 2, len: 1 },
    ],
  });
  expect(chunkHex("AB".repeat(16))).toBe(null);
});

/** A random tree: files of random chunks (some all zero, some shared), and the store behind it. */
function randomTree(r: (n: number) => number) {
  const store = new Map<ChunkHex, Uint8Array>();
  const want = new Map<string, Uint8Array>();
  const lines = ["vnvm-tree\t2\t64"];
  let n = 0;
  for (let f = 0; f < 12; f++) {
    const size = r(5) === 0 ? 0 : 1 + r(3000);
    const data = Uint8Array.from({ length: size }, () => r(256));
    const parts: string[] = [];
    for (let at = 0; at < size;) {
      const len = Math.min(size - at, 1 + r(300));
      const piece = data.subarray(at, at + len);
      if (r(5) === 0) {
        piece.fill(0);
        parts.push(`:${len}`);
      } else {
        const hex = h(++n);
        store.set(hex, piece.slice());
        parts.push(`${hex}:${len}`);
      }
      at += len;
    }
    const path = `Dir ${f}/File\t${f}.bin`;
    want.set(path, data);
    lines.push(`f\t644\t0\t${esc(path)}\t${size}\t${parts.join(",")}`);
  }
  return { tsv: lines.join("\n"), store, want };
}

test("TreeReader: any part of any file is its bytes, in order, whatever the chunks' timing (PBT)", async () => {
  for (let seed = 1n; seed <= 8n; seed++) {
    const r = rng(seed);
    const { tsv, store, want } = randomTree(r);
    const files = parseTree(tsv);
    expect(files.size).toBe(want.size);
    let gets = 0;
    const reader = new TreeReader(files, async (hex, len) => {
      gets++;
      // (answers out of order)
      await new Promise((ok) => setTimeout(ok, r(3)));
      const d = store.get(hex)!;
      expect(d.length).toBe(len);
      return d;
    });
    for (const [path, data] of want) {
      // (paths match whatever their case)
      const f = reader.file(r(2) ? path.toUpperCase() : path)!;
      expect(f.size).toBe(data.length);
      expect(await reader.read(f, 0, f.size)).toEqual(data);
      for (let i = 0; i < 30; i++) {
        const a = r(data.length + 2);
        const b = a + r(data.length + 2);
        expect(await reader.read(f, a, b)).toEqual(data.slice(a, b));
      }
      if (data.length)
        for (let at = 0; at < data.length; at += 1 + r(50)) {
          const c = f.chunks[chunkAt(f, at)]!;
          expect(c.start <= at && at < c.start + c.len).toBe(true);
        }
    }
    // (decoded chunks were kept: far fewer gets than reads of them)
    expect(gets).toBeLessThanOrEqual(store.size);
  }
});

test("TreeReader: a failed chunk is asked for again; a wrong length is an error; kept bytes stay bounded", async () => {
  const a = h(1);
  const files = parseTree(`vnvm-tree\t2\t4\nf\t644\t0\tf\t4\t${a}:4\nf\t644\t0\tg\t4\t${h(2)}:4`);
  let fail = true;
  const reader = new TreeReader(files, async (hex) => {
    if (hex === h(2)) return new Uint8Array(3);
    if (fail) {
      fail = false;
      throw new Error("bucket down");
    }
    return Uint8Array.of(1, 2, 3, 4);
  });
  await expect(reader.bytes("f")).rejects.toThrow("bucket down");
  expect(await reader.bytes("f")).toEqual(Uint8Array.of(1, 2, 3, 4));
  await expect(reader.bytes("g")).rejects.toThrow("3 bytes, not 4");
  expect(await reader.bytes("none")).toBe(null);

  // 100 chunks of 4 MiB read once each: only DECODED_MAX of them kept
  const big = 4 << 20;
  const lines = ["vnvm-tree\t2\t4"];
  for (let i = 0; i < 100; i++) lines.push(`f\t644\t0\tf${i}\t${big}\t${h(1000 + i)}:${big}`);
  const asked = new Map<string, number>();
  const many = new TreeReader(parseTree(lines.join("\n")), async (hex, len) => {
    asked.set(hex, (asked.get(hex) ?? 0) + 1);
    return new Uint8Array(len);
  });
  for (let i = 0; i < 100; i++) await many.read(many.file(`f${i}`)!, 0, 1);
  const kept = (many as unknown as { keptBytes: number }).keptBytes;
  expect(kept).toBeLessThanOrEqual(DECODED_MAX);
  // the newest is kept, the oldest went
  await many.read(many.file("f99")!, 5, 6);
  await many.read(many.file("f0")!, 5, 6);
  expect(asked.get(h(1099))).toBe(1);
  expect(asked.get(h(1000))).toBe(2);
});

test("treePath: /tree/<name>/<path> decoded, names as masu serve's", () => {
  expect(treePath("/tree/g/www/%E6%97%A5%20a.json")).toEqual({ tree: "g", path: "www/日 a.json" });
  expect(treePath("/tree/a_b-c.d/x")).toEqual({ tree: "a_b-c.d", path: "x" });
  for (const bad of ["/tree/g/", "/tree/a%2Fb/x", "/tree/a%20b/x", "/tree/../x", "/other/g/x", "/tree/g/%E6"])
    expect(treePath(bad)).toBe(null);
  expect(isTreeName("..")).toBe(false);
});

test("the port messages' checks take only well-formed ones (fuzz)", () => {
  const r = rng(3n);
  const values: unknown[] = [
    null,
    undefined,
    0,
    -1,
    1.5,
    2 ** 60,
    "",
    "x",
    "port",
    [],
    {},
    new ArrayBuffer(2),
    "..",
    "a b",
  ];
  const pick = () => values[r(values.length)];
  for (let i = 0; i < 20000; i++) {
    const m: Record<string, unknown> = {};
    for (const k of ["masu", "tree", "id", "path", "from", "to", "kind", "size", "bytes", "error"])
      if (r(2)) m[k] = pick();
    if (isReadAsk(m)) {
      expect(Number.isSafeInteger(m.id) && m.from >= 0 && m.to >= m.from && m.to - m.from <= ASK_MAX).toBe(
        true,
      );
      expect(typeof m.path).toBe("string");
    }
    if (isWantPort(m) || isGivePort(m)) expect(isTreeName(m.tree)).toBe(true);
    if (isReadAnswer(m)) expect(["bytes", "none", "error"]).toContain(m.kind);
  }
  expect(isReadAsk({ id: 1, path: "a", from: 0, to: ASK_MAX + 1 })).toBe(false);
});

test("answerReads answers a worker's asks over a real MessagePort, only for its own files", async () => {
  const files = parseTree(`vnvm-tree\t2\t4\nf\t644\t0\tA.txt\t5\t${h(1)}:5`);
  const reader = new TreeReader(files, async () => new TextEncoder().encode("hello"));
  const ch = new MessageChannel();
  answerReads(ch.port1, reader);
  const answers = new Map<number, ReadAnswer>();
  const got = new Promise<void>((done) => {
    ch.port2.onmessage = (e: MessageEvent) => {
      const a: unknown = e.data;
      if (isReadAnswer(a)) answers.set(a.id, a);
      if (answers.size === 4) done();
    };
  });
  ch.port2.postMessage({ id: 1, path: "a.txt", from: 1, to: 3 });
  ch.port2.postMessage({ id: 2, path: "../other/a.txt", from: 0, to: 1 });
  ch.port2.postMessage({ id: 3, path: "A.TXT", from: 0, to: 0 });
  ch.port2.postMessage({ id: 99, path: "a.txt", from: 2, to: 1 }); // (not an ask: no answer)
  ch.port2.postMessage({ id: 4, path: "a.txt", from: 3, to: 100 });
  await got;
  ch.port1.close();
  const one = answers.get(1)!;
  expect(one.kind === "bytes" && [one.size, new TextDecoder().decode(one.bytes)]).toEqual([5, "el"]);
  expect(answers.get(2)!.kind).toBe("none");
  const three = answers.get(3)!;
  expect(three.kind === "bytes" && [three.size, three.bytes.byteLength]).toEqual([5, 0]);
  const four = answers.get(4)!;
  expect(four.kind === "bytes" && new TextDecoder().decode(four.bytes)).toBe("lo");
  expect(answers.has(99)).toBe(false);
});
