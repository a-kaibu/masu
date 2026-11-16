// Real Chromium: the network worker's reads of saved chunks (host/fetch-bridge.js masuOpfsStore,
// web/src/chunkstore.ts), as masuFetchPart reads them: 256 KiB blocks of 4 MiB chunks at
// random places, from OPFS through read-only synchronous access handles, against one handle
// read straight. Checks the bytes, that a page's delete goes through once the worker's handles
// are closed, and prints the times.
//   node tests/opfs-read-browser.mjs          (PLAYWRIGHT as elsewhere)
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
const pw = await import(process.env.PLAYWRIGHT ?? 'playwright');
const { chromium } = pw.default ?? pw;
const source = await readFile(new URL('../host/fetch-bridge.js', import.meta.url), 'utf8');

const server = createServer((req, res) => {
  res.writeHead(200, { 'content-type': 'text/html' });
  res.end('<!doctype html><title>opfs</title>');
}).listen(0, '127.0.0.1');
await new Promise((r) => server.once('listening', r));
const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  const r = await page.evaluate(async (source) => {
    const worker = new Worker(
      URL.createObjectURL(
        new Blob(
          [
            source,
            `
self.onmessage = async () => {
  const N = 12, SIZE = 4 << 20, BLOCK = 256 << 10, READS = 600;
  const hex = (i) => i.toString(16).padStart(32, '0');
  const url = (i) => location.origin + '/store/chunks/' + hex(i).slice(0, 2) + '/' + hex(i);
  const data = (i) => { const d = new Uint8Array(SIZE); for (let k = 0; k < SIZE; k += 4096) d[k] = (i * 31 + k / 4096) & 255; return d; };
  const root = await navigator.storage.getDirectory();
  await root.removeEntry('masu-chunks', { recursive: true }).catch(() => {});
  const store = masuOpfsStore(await root.getDirectoryHandle('masu-chunks', { create: true }));
  let tBytes = 0;
  for (let i = 0; i < N; i++) {
    const d = data(i), t0 = performance.now();
    await store.putBytes(url(i), d);
    tBytes += performance.now() - t0;
  }
  // (a key that is no chunk's: not kept)
  await store.putBytes(location.origin + '/store/trees/x.tsv', new Uint8Array([1, 2, 3]));
  const otherOk = !(await store.match(location.origin + '/store/trees/x.tsv'));
  let x = 5;
  const rnd = (n) => ((x = (Math.imul(x, 1103515245) + 12345) >>> 0) >>> 8) % n;
  const plan = Array.from({ length: READS }, () => [rnd(N), rnd(SIZE / BLOCK) * BLOCK]);
  const ok = (i, at, b) => b.length === BLOCK && b[0] === (at % 4096 ? 0 : (i * 31 + at / 4096) & 255) && b[4096] === ((i * 31 + (at + 4096) / 4096) & 255);
  let bad = 0;
  let t = performance.now();
  for (const [i, at] of plan) if (!ok(i, at, await store.read(url(i), at, BLOCK))) bad++;
  const opfs = (performance.now() - t) / READS;
  // (straight through one handle, for comparison: what the store adds)
  let raw = -1, mode = '';
  try {
    const d = await root.getDirectoryHandle('masu-chunks');
    const hs = [];
    for (let i = 0; i < N; i++) hs.push(await (await d.getFileHandle(hex(i))).createSyncAccessHandle({ mode: 'read-only' }).catch((e) => { mode = String(e); return null; }));
    if (hs.every(Boolean)) {
      t = performance.now();
      for (const [i, at] of plan) { const b = new Uint8Array(BLOCK); hs[i].read(b, { at }); }
      raw = (performance.now() - t) / READS;
    }
    hs.forEach((h) => h?.close());
  } catch (e) { mode = String(e); }
  // a page's delete of a chunk the worker holds open: refused until the worker closes its handles
  await store.read(url(0), 0, 10);
  let refused = false;
  try { await root.getDirectoryHandle('masu-chunks').then((d) => d.removeEntry(hex(0))); } catch (_) { refused = true; }
  store.closeAll();
  const after = await store.delete(url(0));
  postMessage({ raw, mode, opfs, bad, refused, after, tBytes, otherOk, gone: (await store.read(url(0), 0, 10)) === null });
};`,
          ],
          { type: 'text/javascript' },
        ),
      ),
    );
    return new Promise((resolve, reject) => {
      worker.onmessage = (e) => resolve(e.data);
      worker.onerror = (e) => reject(new Error(e.message));
      worker.postMessage(0);
    });
  }, source);
  console.log(`(one handle straight: ${r.raw.toFixed(3)} ms ${r.mode})`);
  console.log(`opfs-read: 256 KiB block of a 4 MiB chunk: OPFS sync ${r.opfs.toFixed(3)} ms`);
  console.log(`opfs-write: 4 MiB chunk putBytes ${(r.tBytes / 12).toFixed(2)} ms`);
  assert.equal(r.bad, 0, 'every block as kept');
  assert.ok(r.otherOk, 'a key that is no chunk is not kept');
  assert.ok(r.refused, 'a delete of an open file refused (as the store expects)');
  assert.ok(r.after && r.gone, "the worker's own delete closes its handles first");
  console.log('opfs-read-browser: ok');
} finally {
  await browser.close();
  server.close();
}
