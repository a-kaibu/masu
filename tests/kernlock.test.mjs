// The lock of the kernel the host's processes share (host/vnkern.js vnhost_kern_lock and
// vnhost_kern_unlock, the release in host/pre.js exited; formal/tla/KernLock.tla): worker
// threads stand in for the processes' threads, the real lock code taken from those files runs
// in each, and the test ends processes at any point of their threads (Worker.terminate, as an
// instance's exit or kill does) and then frees what they held as the host does. One holder at a
// time; the threads left keep getting the lock (a waiter woken by an unlock whose process then
// ended took that wakeup with it, and a process that ended between taking the lock and naming
// itself its holder kept it for ever: the kernel stood still for every process).
//   node --test tests/kernlock.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { Worker } from 'node:worker_threads';

const lib = readFileSync(new URL('../host/vnkern.js', import.meta.url), 'utf8');
const pre = readFileSync(new URL('../host/pre.js', import.meta.url), 'utf8');

// host/pre.js exited()'s part for the lock, as a function of (h, pid)
const at = pre.indexOf("// (the kernel's lock, if a thread of this one had it");
assert.ok(at > 0, 'the release in host/pre.js');
const body = pre.slice(pre.indexOf('\n', at) + 1, pre.indexOf('const i = Atomics.load(h, 4)', at));
const release = new Function('h', 'pid', body);

// a thread: the library's lock and unlock (addToLibrary's object), then lock, check the
// holder slot, unlock, for ever; [2 + slot] counts its rounds
const worker = `
const { workerData: { sab, pid, slot, lib } } = require('node:worker_threads');
let o;
const addToLibrary = (x) => { o = x; };
let vnvmKernelI32, vnvmKernel = sab;
function vnkernH() { return o.$vnkernH(); }
eval(lib);
const h = new Int32Array(sab, 0, 256);
const s = new Int32Array(sab, 1024, 64);
const lock = o.vnhost_kern_lock.bind(o), unlock = o.vnhost_kern_unlock.bind(o);
for (;;) {
  lock(pid);
  if (Atomics.exchange(s, 0, pid) !== 0) Atomics.store(s, 1, 1);
  for (let k = (Math.random() * 50) | 0; k > 0; k--) Atomics.load(s, 0);
  if (Atomics.exchange(s, 0, 0) !== pid) Atomics.store(s, 1, 1);
  unlock(pid);
  Atomics.add(s, 2 + slot, 1);
}
`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

test('one holder, and the threads left always get the lock while processes end anywhere', { timeout: 120_000 }, async () => {
  const sab = new SharedArrayBuffer(1024 + 64 * 4);
  const h = new Int32Array(sab, 0, 256);
  const s = new Int32Array(sab, 1024, 64);
  // (process 0x10 lives on with 2 threads; processes of 3 threads come and go)
  const keep = [0, 1].map((slot) => new Worker(worker, { eval: true, workerData: { sab, pid: 0x10, slot, lib } }));
  let next = 1;
  let slot = 2;
  const start = (pid) => [0, 1, 2].map(() => {
    const k = slot++;
    if (slot >= 62) slot = 2;
    return new Worker(worker, { eval: true, workerData: { sab, pid, slot: k, lib } });
  });
  let procs = [];
  try {
    for (let i = 0; i < 3; i++) procs.push({ pid: 0x10 + 0x10000 * next++, threads: null });
    for (const p of procs) p.threads = start(p.pid);
    for (let round = 0; round < 150; round++) {
      await sleep(5 + Math.random() * 20);
      // a process ends: its threads stop wherever they are, then the host frees its lock
      const i = (Math.random() * procs.length) | 0;
      const p = procs[i];
      await Promise.all(p.threads.map((t) => t.terminate()));
      // (what it was doing under the lock goes with it)
      Atomics.compareExchange(s, 0, p.pid, 0);
      release(h, p.pid);
      const q = { pid: 0x10 + 0x10000 * next++, threads: null };
      q.threads = start(q.pid);
      procs[i] = q;
      // the threads of 0x10 still get the lock
      const before = [s[2], s[3]];
      const until = Date.now() + 5000;
      while ((s[2] === before[0] || s[3] === before[1]) && Date.now() < until) await sleep(2);
      assert.ok(s[2] !== before[0] && s[3] !== before[1], `round ${round}: a thread of 0x10 stood still (lock word ${h[0]})`);
    }
    assert.equal(s[1], 0, 'two holders at once');
  } finally {
    for (const p of procs) await Promise.all((p.threads || []).map((t) => t.terminate()));
    await Promise.all(keep.map((t) => t.terminate()));
  }
});
