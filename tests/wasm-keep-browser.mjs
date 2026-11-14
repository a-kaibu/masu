// Real Chromium, its own HTTP cache (a persistent profile, as a player's): web/src/wasmkeep.ts
// after a start that compiled sake's wasm from bytes. Its fetch must leave the server's
// versioned file whole in the HTTP cache, so that the next start's probe (wasmsource.ts 1,
// only-if-cached) finds it and emscripten's instantiateStreaming streams it from there with no
// request to the server (the browser keeps the compiled code only with such an entry). Checked
// too: a second keep finds it ("there", no request); a server failing or cutting the body off
// leaves nothing cached ("failed"), and the keep after it fetches it whole.
//   node tests/wasm-keep-browser.mjs          (PLAYWRIGHT as elsewhere)
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
const pw = await import(process.env.PLAYWRIGHT ?? 'playwright');
const { chromium } = pw.default ?? pw;
// (node strips the types; the function as it is, with the constants it reads)
const keepTs = await import(new URL('../web/src/wasmkeep.ts', import.meta.url).href);
const source = `const KEEP_AFTER_MS = ${keepTs.KEEP_AFTER_MS}, KEEP_BUSY_MS = ${keepTs.KEEP_BUSY_MS};\n${keepTs.keepServerWasm}`;

// a wasm module of ~2 MiB (a custom section for its size) with one function: f() = 42
const leb = (v) => {
  const out = [];
  do {
    let b = v & 0x7f;
    v >>>= 7;
    if (v) b |= 0x80;
    out.push(b);
  } while (v);
  return out;
};
const pad = Buffer.alloc(2 << 20, 7);
const wasm = Buffer.from([
  0, 0x61, 0x73, 0x6d, 1, 0, 0, 0,
  1, 5, 1, 0x60, 0, 1, 0x7f, // type: () -> i32
  3, 2, 1, 0, // func 0: type 0
  7, 5, 1, 1, 0x66, 0, 0, // export "f"
  10, 6, 1, 4, 0, 0x41, 42, 0x0b, // code: i32.const 42
  0, ...leb(1 + 1 + pad.length), 1, 0x78, ...pad, // custom section "x"
]);

const hits = { wasm: 0 };
let mode = 'ok';
const server = createServer((req, res) => {
  const u = new URL(req.url, 'http://x');
  const iso = { 'cross-origin-opener-policy': 'same-origin', 'cross-origin-embedder-policy': 'require-corp' };
  if (u.pathname === '/sake.wasm') {
    hits.wasm++;
    if (mode === '500') return res.writeHead(500).end('no');
    res.writeHead(200, {
      ...iso,
      'content-type': 'application/wasm',
      'content-length': wasm.length,
      'cache-control': 'public, max-age=31536000, immutable',
    });
    if (mode === 'cut') return res.write(wasm.subarray(0, wasm.length >> 1), () => res.destroy());
    return res.end(wasm);
  }
  res.writeHead(200, { ...iso, 'content-type': 'text/html' });
  res.end('<!doctype html><title>keep</title>');
}).listen(0, '127.0.0.1');
await new Promise((r) => server.once('listening', r));
const origin = `http://127.0.0.1:${server.address().port}`;
const dir = await mkdtemp(join(tmpdir(), 'wasm-keep-'));
const ctx = await chromium.launchPersistentContext(dir);
try {
  // (each step on a page of its own, as each start is)
  const onPage = async (fn, arg) => {
    const page = await ctx.newPage();
    await page.goto(`${origin}/`);
    await page.addScriptTag({ content: `${source}\nwindow.keepServerWasm = keepServerWasm;` });
    try {
      return await page.evaluate(fn, arg);
    } finally {
      await page.close();
    }
  };
  const keep = (url) => onPage((url) => keepServerWasm(url, { afterMs: 0 }), url);
  const probe = (url) =>
    onPage(async (url) => {
      try {
        return (await fetch(url, { cache: 'only-if-cached', mode: 'same-origin' })).ok;
      } catch {
        return false;
      }
    }, url);

  // a server failing, then one cutting the body off: nothing kept
  const url = `${origin}/sake.wasm?v=1`;
  mode = '500';
  assert.equal(await keep(url), 'failed');
  assert.equal(await probe(url), false, 'a failed answer is not cached');
  mode = 'cut';
  assert.equal(await keep(url), 'failed');
  assert.equal(await probe(url), false, 'a body cut off is not cached');
  // a whole one: kept, then found
  mode = 'ok';
  const before = hits.wasm;
  assert.equal(await keep(url), 'kept');
  assert.equal(hits.wasm, before + 1);
  assert.equal(await probe(url), true, 'the next start finds it (wasmsource.ts 1)');
  assert.equal(await keep(url), 'there');
  // the next start's streaming compile, from the cache: no request, the module whole
  const n = hits.wasm;
  const f = await onPage(async (url) => {
    const { instance } = await WebAssembly.instantiateStreaming(fetch(url, { credentials: 'same-origin' }));
    return instance.exports.f();
  }, url);
  assert.equal(f, 42);
  assert.equal(hits.wasm, n, 'streamed from the HTTP cache, not the server');
  console.log(`wasm-keep: ok (${hits.wasm} requests to the server for its file)`);
} finally {
  await ctx.close();
  server.close();
  await rm(dir, { recursive: true, force: true });
}
