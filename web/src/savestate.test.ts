import { expect, test } from "vite-plus/test";
import { MAGIC, stateHeader } from "./savestate";

// A state file starts with 8 bytes of magic, read back as all 8: a 7-letter magic (the masu
// rename's "MASUSNP") made every state "not a state" under node and on the page.

/** A state file's head: magic, version, header length, the header. */
function head(magic: string, version: number, header: object): Uint8Array {
  const h = new TextEncoder().encode(JSON.stringify(header));
  const f = new Uint8Array(16 + h.length);
  const v = new DataView(f.buffer);
  for (let i = 0; i < 8; i++) f[i] = magic.charCodeAt(i) || 0;
  v.setUint32(8, version, true);
  v.setUint32(12, h.length, true);
  f.set(h, 16);
  return f;
}

test("the magic is 8 bytes, each one a letter", () => {
  expect(MAGIC.length).toBe(8);
  expect(MAGIC).toMatch(/^[A-Z]{8}$/);
});

test("a state's header reads back (property: any build, pages, ctl)", () => {
  let x = 7;
  const rnd = (n: number) => ((x = (Math.imul(x, 1103515245) + 12345) >>> 0) >>> 8) % n;
  for (let k = 0; k < 200; k++) {
    const h = {
      format: 1,
      build: rnd(1 << 24).toString(16),
      wasm: rnd(2) ? "sake" : "sake-grow",
      pages: 1 + rnd(65535),
      ctl: rnd(1 << 30),
    };
    const f = head(MAGIC, 1, h);
    const got = stateHeader(f);
    expect(got).toEqual({ build: h.build, wasm: h.wasm, pages: h.pages, ctl: h.ctl, at: f.length });
  }
});

test("a file with another magic, a shorter one or another version is none", () => {
  const h = { format: 1, build: "b", ctl: 1 };
  for (const m of [MAGIC.slice(0, 7), "VNVMSNAP", MAGIC.toLowerCase()])
    expect(() => stateHeader(head(m, 1, h))).toThrow();
  expect(() => stateHeader(head(MAGIC, 2, h))).toThrow();
});

test("node's states (host/snap.js) have the page's magic", async () => {
  // (node's fs, out of the web types' sight)
  const { readFileSync } = (await import(/* @vite-ignore */ ["node", "fs"].join(":"))) as {
    readFileSync: (u: URL, e: string) => string;
  };
  const js = readFileSync(new URL("../../host/snap.js", import.meta.url), "utf8");
  expect(/MAGIC: '([^']*)'/.exec(js)?.[1]).toBe(MAGIC);
});
