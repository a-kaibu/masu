// The kernel the host's processes share (crates/sake/src/kern/host.rs): one SharedArrayBuffer
// for all of them, `vnvmKernel` (host/pre.js: the host's, or the first process's own), every
// thread of every process has it. Int32 words:
//   0 lock (0 free, else its holder's process id x 2, + 1 with waiters), 1 generation (changes with every change,
//   what waiting is on), 2 sharing (1 once a process started another), 3 the state's length,
//   4 how many processes ended, 16.. a ring of 64 (id, exit code) of them;
// the state's bytes from byte 1024.
addToLibrary({
  $vnkernH: () => (vnvmKernelI32 ??= new Int32Array(vnvmKernel, 0, 256)),
  vnhost_kern_shared__deps: ['$vnkernH'],
  vnhost_kern_shared: () => (vnvmKernel && Atomics.load(vnkernH(), 2) ? 1 : 0),
  vnhost_kern_can_share: () => (vnvmKernel && vnvmCanSpawn ? 1 : 0),
  vnhost_kern_start_sharing__deps: ['$vnkernH'],
  vnhost_kern_start_sharing: () => Atomics.store(vnkernH(), 2, 1),
  // The lock, word 0: 0 free, else its holder's process id x 2, + 1 while others wait (one
  // word, so that whatever moment a holder is stopped at, the word names it). A holder whose
  // process ended (on the ring) lets it go here: its exit (host/pre.js) frees what its threads
  // hold then, but a thread of it that still ran could take it after that and be stopped
  // holding it (registrymulti: a child's watcher woken by its own end), every process then
  // waiting for good.
  $vnkernFreeDead: (h, c) => {
    const holder = c >>> 1;
    if (!holder) return false;
    const n = Atomics.load(h, 4) >>> 0;
    for (let i = n > 64 ? n - 64 : 0; i < n; i++) {
      if (Atomics.load(h, 16 + 2 * (i % 64)) >>> 0 !== holder) continue;
      if (Atomics.compareExchange(h, 0, c, 0) !== c) return false;
      Atomics.notify(h, 0);
      return true;
    }
    return false;
  },
  vnhost_kern_lock__deps: ['$vnkernH', '$vnkernFreeDead'],
  vnhost_kern_lock: (pid) => {
    const h = vnkernH();
    const me = (pid << 1) | 0;
    let c = Atomics.compareExchange(h, 0, 0, me);
    if (c === 0) return;
    // (a thread that waited takes it marked waited for: others may still wait, as Drepper's)
    for (;;) {
      if (c === 0) {
        c = Atomics.compareExchange(h, 0, 0, me | 1);
        if (c === 0) return;
        continue;
      }
      if (!(c & 1)) {
        const was = Atomics.compareExchange(h, 0, c, c | 1);
        if (was !== c) {
          c = was;
          continue;
        }
        c |= 1;
      }
      if (Atomics.wait(h, 0, c, 100) === 'timed-out') vnkernFreeDead(h, c);
      c = Atomics.load(h, 0);
    }
  },
  vnhost_kern_unlock__deps: ['$vnkernH'],
  vnhost_kern_unlock: () => {
    const h = vnkernH();
    if (Atomics.exchange(h, 0, 0) & 1) Atomics.notify(h, 0, 1);
  },
  vnhost_kern_get__deps: ['$vnkernH'],
  vnhost_kern_get__sig: 'ppp',
  vnhost_kern_get: (dst, cap) => {
    const n = vnkernH()[3];
    if (n && cap >= n) HEAPU8.set(new Uint8Array(vnvmKernel, 1024, n), dst >>> 0);
    return n;
  },
  vnhost_kern_put__deps: ['$vnkernH'],
  vnhost_kern_put__sig: 'ipp',
  vnhost_kern_put: (src, len) => {
    if (1024 + len > vnvmKernel.byteLength) return 0;
    new Uint8Array(vnvmKernel, 1024, len).set(HEAPU8.subarray(src >>> 0, (src >>> 0) + len));
    vnkernH()[3] = len;
    return 1;
  },
  vnhost_kern_gen__deps: ['$vnkernH'],
  vnhost_kern_gen: () => Atomics.load(vnkernH(), 1),
  vnhost_kern_bump__deps: ['$vnkernH'],
  vnhost_kern_bump: () => (Atomics.add(vnkernH(), 1, 1) + 1) | 0,
  vnhost_kern_notify__deps: ['$vnkernH'],
  vnhost_kern_notify: () => Atomics.notify(vnkernH(), 1),
  vnhost_kern_wait__deps: ['$vnkernH'],
  vnhost_kern_wait: (seen, ms) => (Atomics.wait(vnkernH(), 1, seen, ms) === 'timed-out' ? 0 : 1),
  vnhost_kern_dead__deps: ['$vnkernH'],
  vnhost_kern_dead__sig: 'iip',
  // (1: the ith ended process; 0: none yet; -1: that entry was overwritten, more than 64
  // processes ended since: the reader goes past it, not on with another process's pid)
  vnhost_kern_dead: (i, out) => {
    const h = vnkernH();
    const n = Atomics.load(h, 4) >>> 0;
    if ((i >>> 0) >= n) return 0;
    if (n - (i >>> 0) > 64) return -1;
    const at = 16 + 2 * (i % 64);
    HEAPU32[(out >>> 2)] = Atomics.load(h, at);
    HEAPU32[(out >>> 2) + 1] = Atomics.load(h, at + 1);
    return 1;
  },
  // CreateProcess: asks the page or node (Module.vnvmSpawn, on the main thread) to start
  // process `pid` from host file `exe`
  vnhost_spawn__sig: 'iipp',
  vnhost_spawn: (pid, exe, len) => {
    if (!vnvmCanSpawn) return 0;
    const m = { vnvmSpawn: { pid, exe: UTF8ToString(exe >>> 0, len) } };
    if (ENVIRONMENT_IS_NODE) require('worker_threads').parentPort.postMessage(m);
    else self.postMessage(m);
    return 1;
  },
});
