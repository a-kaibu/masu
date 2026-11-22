// 端末に保存 of a game in a browser that never started one, then its first start with no
// connection (review 22bb061 §4: build-id.txt was not saved, and the start refused "Missing
// build ID for sake.wasm").
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { saveApp } from "./offline";
import { buildIdOf, runtimeWasm } from "./wasmsource";
import { offlineRuntime, runtimeManifest, sakeBuild, setPageHref, type SakeBuild } from "./runtime";
import { builds } from "./savestate";
import type { GameEntry } from "./launcher";

const PAGE = "https://masu.test/play.html";
const bytes = (n: number) => new Uint8Array(Array.from({ length: 256 }, (_, i) => (i * 7 + n) & 255));
const served = new Map<string, Uint8Array<ArrayBuffer>>();
/** what the device keeps: each answer a fetch got (sw.ts keeps the app's files so) */
const kept = new Map<string, { body: Uint8Array<ArrayBuffer>; head: Record<string, string> }>();
let online = true;

const headers = (b: Uint8Array) => ({
  "last-modified": "Sun, 22 Nov 2026 00:00:00 GMT",
  "content-length": String(b.length),
});

async function fake(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = new URL(String(input instanceof Request ? input.url : input), PAGE);
  const key = url.origin + url.pathname + url.search;
  const head = init?.method === "HEAD";
  if (!online) {
    const k = kept.get(key);
    if (!k) return new Response(null, { status: 503, headers: { "x-masu-offline": "1" } });
    return new Response(head ? null : k.body.slice(), { status: 200, headers: k.head });
  }
  const b = served.get(url.origin + url.pathname);
  if (!b) return new Response(null, { status: 404 });
  kept.set(key, { body: b, head: headers(b) });
  return new Response(head ? null : b.slice(), { status: 200, headers: headers(b) });
}

beforeEach(async () => {
  served.clear();
  kept.clear();
  online = true;
  setPageHref(PAGE);
  const wasm = bytes(1);
  const grow = bytes(2);
  const sake = "https://masu.test/sake/";
  served.set(`${sake}sake.js`, bytes(3));
  served.set(`${sake}sake.wasm`, wasm);
  served.set(`${sake}sake-grow.js`, bytes(4));
  served.set(`${sake}sake-grow.wasm`, grow);
  served.set(`${sake}snapz.mjs`, bytes(5));
  served.set(`${sake}jit-version.txt`, new TextEncoder().encode("0123abcd"));
  served.set(
    `${sake}build-id.txt`,
    new TextEncoder().encode(`sake ${await buildIdOf(wasm)}\nsake-grow ${await buildIdOf(grow)}\n`),
  );
  vi.stubGlobal("location", new URL(PAGE));
  vi.stubGlobal("fetch", fake);
  vi.stubGlobal("navigator", { serviceWorker: { controller: {} }, onLine: true });
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
});
afterEach(() => vi.unstubAllGlobals());

const game: GameEntry = { id: "g", title: "g", run: "Z:\\masu\\game\\g.exe", args: [], mounts: [], env: [] };

/** The start's runtime part as play.ts does it: its manifest, the build's id, its wasm. */
async function start(build: SakeBuild) {
  const m = await runtimeManifest(build);
  const name = `${build.name}.wasm`;
  return runtimeWasm(name, m.wasm, "https://masu.test/store/", (await builds()).get(build.name), true, {
    request: fake,
  });
}

test("a game saved to the device before any start starts offline: every runtime file the start reads was saved", async () => {
  await saveApp(game, new AbortController().signal);
  online = false;
  const got = await start({ name: "sake", jit: "" });
  expect(got.verified).toBe(true);
  expect(await buildIdOf(got.bytes)).toBe((await builds()).get("sake"));
  // (and the fallback a device whose 4 GB fails goes on with)
  const grow = await start({ name: "sake-grow", jit: "g" });
  expect(await buildIdOf(grow.bytes)).toBe((await builds()).get("sake-grow"));
});

test("what is saved is the manifest the start reads, for any build the device may run", async () => {
  for (const build of [sakeBuild("grow"), { name: "sake", jit: "" } as SakeBuild]) {
    kept.clear();
    online = true;
    await saveApp(game, new AbortController().signal);
    const { files } = await offlineRuntime(build);
    for (const f of files) expect(kept.has(f), f).toBe(true);
  }
});
