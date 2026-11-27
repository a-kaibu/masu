// Written before migrating vnfs.js. Exercise the real worker protocol with reversed replies.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Worker } from 'node:worker_threads';
import { readFile } from 'node:fs/promises';
import { bridgeScript } from '../host/bridge-source.mjs';

// (as sake.js has it: host/net-core.mjs before it)
const source = bridgeScript(
  await readFile(new URL('../host/net-core.mjs', import.meta.url), 'utf8'),
  await readFile(new URL('../host/fetch-bridge.js', import.meta.url), 'utf8'),
);
// chunk names of i % 251 inputs from the blake3 crate (crates/masu/src/pack.rs makes the file)
const vectors = (await readFile(new URL('../testdata/blake3.tsv', import.meta.url), 'utf8')).trim().split('\n').slice(1).map((l) => l.split('\t'));
const names = Object.fromEntries(vectors.filter((v) => v[0] === 'p').map(([, n, h]) => [h, +n]));
/** The inputs testdata/blake3.tsv names: `p` i % 251, `r` an LCG seeded with the length. */
const input = (kind, n) => {
  let x = n >>> 0;
  return Uint8Array.from({length: n}, (_, i) => {
    if (kind === 'p') return i % 251;
    x = (Math.imul(x, 1103515245) + 12345) >>> 0;
    return (x >>> 16) & 255;
  });
};
// Keep the mocked broker on a separate worker, as the page does for its pthreads.
const broker = `
const { workerData } = require('node:worker_threads');
MASU_STALL_MS = 200;
MASU_GIVE_UP_MS = 1500;
const tries = new Map();
const body = (length) => Uint8Array.from({length}, (_, i) => i % 251).buffer;
const NAMES = ${JSON.stringify(names)};
// the device's chunk store (OPFS in the browser): kept bodies, reads of parts counted
MASU_RECENT_BYTES = 1;
const kept = new Map();
let fetches = 0, slices = 0, fromPages = 0;
masuCacheOpen = Promise.resolve({
  match: async (u) => { const d = kept.get(u); return d && {arrayBuffer: async () => d.slice().buffer}; },
  putBytes: async (u, d) => void kept.set(u, d.slice()),
  read: async (u, a, n) => { const d = kept.get(u); return d ? (slices++, d.slice(a, a + n)) : null; },
  delete: async (u) => kept.delete(u),
});
// (the stand-in bucket's objects are the chunks themselves: no key, no zstd)
masuOpen = async (info, c) => c;
masuUnzstd = async (base, f) => f;
globalThis.fetch = async (url, init) => {
  const n = (tries.get(url) ?? 0) + 1;
  tries.set(url, n);
  // (how many bucket fetches and device reads of parts so far, as a body's length)
  if (url === 'count' || url === 'slices' || url === 'pages') return {ok: true, status: 200, arrayBuffer: async () => new ArrayBuffer(url === 'count' ? fetches : url === 'pages' ? fromPages : slices)};
  // the server says where in the bucket; it never has a chunk itself
  const ask = /[/]store-direct[/]chunks[/][0-9a-f]{2}[/]([0-9a-f]{32})$/.exec(url);
  if (ask) return {ok: true, status: 200, json: async () => ({url: 'https://bucket/' + ask[1], key: '0'.repeat(64), object: '00000000-0000-4000-8000-000000000000'})};
  // the release's Pages (release-trees.ts): plain chunks by their names, the first try
  // of the 262144-byte one damaged; asked of the bucket they are not there
  const page = /^https:[/][/]pages[/]store[/]chunks[/][0-9a-f]{2}[/]([0-9a-f]{32})$/.exec(url);
  if (page) {
    fetches++;
    fromPages++;
    const length = NAMES[page[1]] ?? 1000;
    const data = new Uint8Array(body(length));
    if (length === 262144 && n === 1) data[7] ^= 1;
    return {ok: true, status: 200, arrayBuffer: async () => data.buffer};
  }
  if (url.includes('/store/chunks/')) throw new Error('a chunk asked of the server: ' + url);
  // the bucket's objects: the bytes their name says (the 262144-byte one damaged the
  // first time), any other name 1000 bytes that never match
  const chunk = /^https:[/][/]bucket[/]([0-9a-f]{32})$/.exec(url);
  if (chunk) {
    fetches++;
    const length = NAMES[chunk[1]] ?? 1000;
    const data = new Uint8Array(body(length));
    if (length === 262144 && n === 1) data[1000] ^= 1;
    return {ok: true, status: 200, arrayBuffer: async () => data.buffer};
  }
  await new Promise(r => setTimeout(r, url === 'slow' ? 30 : 1));
  // a link that drops twice, a server that is restarting, then both fine
  if (url === 'fail' || (url === 'flaky' && n <= 2)) throw new TypeError('Failed to fetch');
  if (url === 'restart' && n <= 2) return {ok: false, status: 503};
  // a body that stops halfway the first time (the stall watch aborts it)
  if (url === 'stall' && n === 1) return {ok: true, status: 200, body: {getReader: () => ({
    read: () => new Promise((_, reject) => init.signal.addEventListener('abort', () => reject(init.signal.reason))),
  })}};
  return {ok: url !== '404', status: 404, arrayBuffer: async () => body(url === 'empty' ? 0 : url === 'big' ? 700001 : url === 'exact' ? 262144 * 2 : 150000)};
};
if (workerData.pages) masuPagesInit(workerData.pages);
workerData.port.on('message', data => masuFetchWorker({data}));
`;

const client = `
const { parentPort, workerData } = require('node:worker_threads');
masuFetchPort = workerData.port;
try {
  const all = workerData.url.split('|').map((u) => Array.from(masuFetchSync(...masuRange(u))));
  parentPort.postMessage({data: all.length === 1 ? all[0] : all});
} catch (e) {
  parentPort.postMessage({error: e.message});
}
`;

async function request(url, pages) {
  const { port1, port2 } = new MessageChannel();
  const net = new Worker(source + '\n' + broker, { eval: true, workerData: { port: port1, pages }, transferList: [port1] });
  const reader = new Worker(source + '\n' + client, { eval: true, workerData: { port: port2, url }, transferList: [port2] });
  try {
    return await new Promise((resolve, reject) => {
      net.once('error', reject);
      reader.once('error', reject);
      reader.once('message', resolve);
    });
  } finally {
    await Promise.all([net.terminate(), reader.terminate()]);
  }
}

test('parallel reverse-order requests preserve every byte across buffer boundaries', async () => {
  const [slow, fast] = await Promise.all([request('slow'), request('fast')]);
  const expected = Array.from({length:150000}, (_, i) => i % 251);
  assert.deepEqual(slow.data, expected);
  assert.deepEqual(fast.data, expected);
});
test('bodies over several transfer buffers (and exactly two) arrive whole, one after another on a thread', async () => {
  for (const [url, n] of [['big', 700001], ['exact', 524288], ['fast', 150000]]) {
    const r = await request(url);
    assert.equal(r.data.length, n, url);
    assert.ok(r.data.every((b, i) => b === i % 251), url);
  }
  // (several reads on one thread share its transfer buffer)
  const seq = await request('big|empty|fast|exact|big');
  assert.deepEqual(seq.data.map((d) => d.length), [700001, 0, 150000, 524288, 700001]);
  assert.ok(seq.data.every((d) => d.every((b, i) => b === i % 251)));
});
test('empty response completes', async () => assert.deepEqual((await request('empty')).data, []));
test('network and HTTP failures unblock the caller', async () => {
  assert.ok((await request('fail')).error);
  assert.ok((await request('404')).error);
});
test('dropped links, 503s and stalled bodies are retried until the bytes arrive', async () => {
  const expected = Array.from({length:150000}, (_, i) => i % 251);
  for (const url of ['flaky', 'restart', 'stall']) assert.deepEqual((await request(url)).data, expected, url);
});
test('a 404 fails at once, without retries', async () => {
  const t = Date.now();
  assert.ok((await request('404')).error);
  assert.ok(Date.now() - t < 1000);
});

test('BLAKE3 chunk names as the blake3 crate makes them', () => {
  const masuBlake3 = new Function(`${source}; return masuBlake3;`)();
  for (const [kind, n, h] of vectors) assert.equal(masuBlake3(input(kind, +n)), h, `${kind} ${n}`);
});
test('BLAKE3 both ways (wasm SIMD and plain JS) as the crate, and equal for random inputs', () => {
  const { js, simd } = new Function(`${source}; return { js: masuBlake3Js, simd: masuBlake3Simd };`)();
  const fast = simd();
  assert.ok(fast, 'node has wasm SIMD');
  for (const [kind, n, h] of vectors) {
    assert.equal(js(input(kind, +n)), h, `js ${kind} ${n}`);
    assert.equal(fast(input(kind, +n)), h, `simd ${kind} ${n}`);
  }
  // (seeded: lengths around 1 KiB chunks, groups of 4, the 256-chunk window)
  let x = 12345;
  const rnd = () => ((x = (Math.imul(x, 1664525) + 1013904223) >>> 0), x);
  for (let k = 0; k < 60; k++) {
    const n = [rnd() % 9000, 256 * 1024 + (rnd() % 8192) - 4096, 4 * 1024 * 1024 + (rnd() % 3000)][k % 3];
    const d = Uint8Array.from({ length: n }, () => rnd() & 255);
    assert.equal(fast(d), js(d), `length ${n}`);
  }
});
test('the module worker sends bytes through a transferred port', async () => {
  const entry = new URL('../host/net-worker.mjs', import.meta.url).href;
  const n = 200000;
  const source = `
    import { parentPort } from 'node:worker_threads';
    globalThis.self = globalThis;
    globalThis.fetch = async () => new Response(Uint8Array.from({length: ${n}}, (_, i) => i % 251));
    await import(${JSON.stringify(entry)});
    parentPort.on('message', data => self.onmessage({data}));
  `;
  const worker = new Worker(new URL(`data:text/javascript,${encodeURIComponent(source)}`));
  const failure = new Promise((_, reject) => worker.once('error', reject));
  const { port1, port2 } = new MessageChannel();
  try {
    const url = "https://server.test/data";
    worker.postMessage({ port: port1 }, [port1]);
    const shared = new SharedArrayBuffer(32 + 262144);
    const state = new Int32Array(shared, 0, 8);
    port2.postMessage({ url, shared });
    const pending = Atomics.waitAsync(state, 0, 0, 5000);
    if (pending.async) await Promise.race([failure, pending.value]);
    assert.equal(Atomics.load(state, 0), 1);
    assert.equal(Atomics.load(state, 1), +n);
    assert.deepEqual(new Uint8Array(shared, 32, +n), input('p', +n));
    Atomics.store(state, 0, 0);
    Atomics.notify(state, 0);
    const finish = Atomics.waitAsync(state, 0, 0, 5000);
    if (finish.async) await Promise.race([failure, finish.value]);
    assert.equal(Atomics.load(state, 0), 2);
  } finally {
    port2.close();
    await worker.terminate();
  }
});
test('a chunk is checked against its name: fetched again when damaged, a failure when it never matches', async () => {
  const name = (n) => vectors.find((v) => v[0] === 'p' && +v[1] === n)[2];
  const good = await request(`https://s/store/chunks/ab/${name(200000)}`);
  assert.equal(good.data.length, 200000);
  const once = await request(`https://s/store/chunks/ab/${name(262144)}`);
  assert.equal(once.data.length, 262144);
  assert.ok(once.data.every((b, i) => b === i % 251), 'the second, undamaged copy');
  const never = await request('https://s/store/chunks/ab/00000000000000000000000000000000');
  assert.ok(never.error, 'a chunk that never matches its name fails');
});
const nameOf = (n) => vectors.find((v) => v[0] === 'p' && +v[1] === n)[2];
const want = (from, len, n) => Array.from({length: Math.max(0, Math.min(from + len, n) - from)}, (_, i) => (from + i) % 251);
test('blocks of a chunk: the whole chunk from the bucket once, its other blocks from memory', async () => {
  const c = `https://s/store/chunks/ab/${nameOf(262143)}`;
  const B = 65536;
  const r = await request([`${c}#0+100/${B}`, `${c}#65536+70000/${B}`, `${c}#196608+65535/${B}`, 'count', `${c}#300000+5/${B}`, c, 'count'].join('|'));
  assert.ok(!r.error, r.error);
  const [a, b, e, n1, past, whole, n2] = r.data;
  assert.deepEqual(a, want(0, 100, 262143));
  assert.deepEqual(b, want(65536, 70000, 262143));
  // (the chunk's last block, short)
  assert.deepEqual(e, want(196608, 65535, 262143));
  assert.equal(n1.length, 1, 'one fetch for every block');
  assert.deepEqual(past, []);
  assert.deepEqual(whole, want(0, 262143, 262143));
  assert.equal(n2.length, 1);
});
test('a chunk pushed out of memory: its blocks read from the device by parts, not fetched again', async () => {
  const c = `https://s/store/chunks/ab/${nameOf(200000)}`;
  const d = `https://s/store/chunks/ab/${nameOf(262143)}`;
  const r = await request([`${c}#0+100/65536`, `${d}#5+10/65536`, 'count', `${c}#7+1000/65536`, 'count', 'slices'].join('|'));
  assert.ok(!r.error, r.error);
  const [a, f, n1, g, n2, s2] = r.data;
  assert.deepEqual(a, want(0, 100, 200000));
  assert.deepEqual(f, want(5, 10, 262143));
  assert.equal(n1.length, 2);
  assert.deepEqual(g, want(7, 1000, 200000));
  assert.equal(n2.length, 2, 'nothing more from the bucket');
  assert.equal(s2.length, 1, 'one read of a part on the device');
});
test('ranges parse only at the end of a name', () => {
  const masuRange = new Function(`${source}; return masuRange;`)();
  // (the last: the request's class, 0 the game's, 1 a fetcher thread's "!a")
  assert.deepEqual(masuRange('chunks/ab/x#0+5'), ['chunks/ab/x', 0, 5, undefined, 0]);
  assert.deepEqual(masuRange('chunks/ab/x#262144+5/262144'), ['chunks/ab/x', 262144, 5, 262144, 0]);
  assert.deepEqual(masuRange('chunks/ab/x#262144+5/262144!a'), ['chunks/ab/x', 262144, 5, 262144, 1]);
  assert.deepEqual(masuRange('chunks/ab/x!a'), ['chunks/ab/x!a']);
  assert.deepEqual(masuRange('trees/a#b.tsv'), ['trees/a#b.tsv']);
  assert.deepEqual(masuRange('chunks/ab/x#12+'), ['chunks/ab/x#12+']);
});

// ---- review b11 ----

/** The bridge's functions in a scope of their own, with `extra` run after them (it can replace
 * any of them) and `ret` returned. */
const scope = (extra, ret) => new Function(`${source}; ${extra}; return ${ret};`)();

test('a zstd frame header gives its decoded size, every field layout (RFC 8878)', () => {
  const size = scope('', 'masuZstdSize');
  // (single segment, 1-byte FCS; 2-byte FCS (+256); 4- and 8-byte; with a window byte and dict ids)
  const head = (fhd, mid, fcs) => Uint8Array.from([0x28, 0xb5, 0x2f, 0xfd, fhd, ...mid, ...fcs, 0, 0, 0]);
  assert.equal(size(head(0x20, [], [200])), 200);
  assert.equal(size(head(0x60, [], [0x10, 0x01])), 0x0110 + 256);
  assert.equal(size(head(0x80, [0x58], [1, 2, 3, 4])), 0x04030201);
  assert.equal(size(head(0xc0 | 0x02, [0x58, 7, 7], [0, 0, 0, 0, 4, 0, 0, 0])), 2 ** 34);
  assert.equal(size(head(0x00, [0x58], [])), 0, 'no FCS field: unknown');
  assert.equal(size(Uint8Array.from([1, 2, 3, 4, 5, 6, 7])), 0, 'not zstd');
  assert.equal(size(head(0x80, [0x58], [1, 2]).subarray(0, 8)), 0, 'cut short');
  // generated: every FHD with every FCS value width, the size written as the spec says
  let x = 7;
  for (let i = 0; i < 4000; i++) {
    x = (Math.imul(x, 1103515245) + 12345) >>> 0;
    const flag = x & 3, single = (x >> 2) & 1, did = (x >> 3) & 3;
    const n = [single ? 1 : 0, 2, 4, 8][flag];
    if (!n) continue;
    const raw = ((x >>> 5) * 2654435761) % { 1: 256, 2: 65536, 4: 2 ** 32, 8: 2 ** 40 }[n];
    const v = raw + (n === 2 ? 256 : 0);
    const fcs = Array.from({ length: n }, (_, k) => Math.floor(raw / 256 ** k) % 256);
    const mid = [...(single ? [] : [0x50]), ...Array([0, 1, 2, 4][did]).fill(9)];
    assert.equal(size(head((flag << 6) | (single << 5) | did, mid, fcs)), v);
  }
});

test('the zstd CLI\'s frames say the sizes they hold (differential, when zstd is on PATH)', async () => {
  const { execFileSync } = await import('node:child_process');
  try { execFileSync('zstd', ['--version']); } catch { return; }
  const { mkdtempSync, writeFileSync, rmSync } = await import('node:fs');
  const { tmpdir } = await import('node:os');
  const { join } = await import('node:path');
  const size = scope('', 'masuZstdSize');
  // (from a file: from stdin the CLI does not know the size and leaves it out of the frame)
  const dir = mkdtempSync(join(tmpdir(), 'zstd-'));
  try {
    for (const n of [0, 1, 255, 256, 65791, 70000, 1 << 20]) {
      const f = join(dir, `in${n}`);
      writeFileSync(f, Buffer.alloc(n, 3));
      const z = execFileSync('zstd', ['-c', '-q', f]);
      assert.equal(size(new Uint8Array(z)), n);
    }
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('a chunk whose put never settles leaves after MASU_PUT_MS, and the fetches ahead go on', async () => {
  const s = scope('MASU_PUT_MS = 30', '{ keep: masuKeep, held: masuHeld, bytes: () => masuSavingBytes, saving: () => masuSaving.size }');
  const stuck = { putBytes: () => new Promise(() => {}) };
  for (let i = 0; i < 8; i++) s.keep(stuck, `u${i}`, new Uint8Array(1000));
  assert.equal(s.bytes(), 8000);
  assert.ok(s.held('u3'));
  await new Promise((r) => setTimeout(r, 80));
  assert.equal(s.bytes(), 0);
  assert.equal(s.saving(), 0);
  // (a put that settles first still ends it at once)
  s.keep({ putBytes: async () => {} }, 'v', new Uint8Array(10));
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(s.bytes(), 0);
});

test('waiters of one chunk share one download from the bucket; a failed one fails them all, never the server', async () => {
  const s = scope(`
    var direct = 0, fail = false;
    masuCache = async () => undefined;
    masuFetchRobust = async (url) => { throw new Error('the server asked for ' + url); };
    masuDirect = async () => { direct++; await new Promise((r) => setTimeout(r, 20)); if (fail) throw new Error('gone'); return { data: new Uint8Array([1, 2, 3]), wire: 3 }; };
  `, '{ whole: masuWhole, direct: () => direct, fail: () => (fail = true) }');
  const got = await Promise.all([0, 0, 2, 1, 0].map((cls) => s.whole('chunks/ab/x', { cls })));
  assert.equal(s.direct(), 1);
  for (const g of got) assert.deepEqual(Array.from(g.data), [1, 2, 3]);
  await s.whole('chunks/ab/x');
  assert.equal(s.direct(), 2, 'a later read fetches again (nothing is kept here past the download)');
  s.fail();
  const all = await Promise.allSettled([0, 1, 0].map((cls) => s.whole('chunks/ab/x', { cls })));
  assert.deepEqual(all.map((a) => a.status), ['rejected', 'rejected', 'rejected']);
  assert.equal(s.direct(), 3);
});



test("the release's trees' chunks come from its Pages, checked, never asked of the server or the bucket", async () => {
  const pages = (...ns) => Object.fromEntries(ns.map((n) => [nameOf(n), `https://pages/store/chunks/ab/${nameOf(n)}`]));
  const on = pages(200000, 262144, 102400);
  const c = `https://s/store/chunks/ab/${nameOf(200000)}`;
  // (the fake server throws for a chunk asked of it; the bucket's would count too)
  const r = await request([c, 'count', 'pages'].join('|'), on);
  assert.ok(!r.error, r.error);
  assert.deepEqual(r.data[0], want(0, 200000, 200000));
  assert.equal(r.data[1].length, 1, 'one fetch');
  assert.equal(r.data[2].length, 1, 'from the Pages');
  // (damaged the first time: fetched again, the second copy)
  const once = await request(`https://s/store/chunks/ab/${nameOf(262144)}`, on);
  assert.ok(!once.error, once.error);
  assert.equal(once.data.length, 262144);
  assert.ok(once.data.every((b, i) => b === i % 251));
  // (named for bytes the Pages never has: given up on, not taken)
  const bad = '0'.repeat(32);
  const never = await request(`https://s/store/chunks/ab/${bad}`, { [bad]: `https://pages/store/chunks/ab/${bad}` });
  assert.ok(never.error, "a chunk that never matches its name fails");
  // (a chunk not of those trees still goes to the bucket: without the map, the same chunk too)
  const other = await request([`https://s/store/chunks/ab/${nameOf(102400)}`, 'pages'].join('|'), pages(200000));
  assert.ok(!other.error, other.error);
  assert.equal(other.data[0].length, 102400);
  assert.equal(other.data[1].length, 0, 'not from the Pages');
});
