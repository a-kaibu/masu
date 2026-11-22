// What the page (web/src) and the game's network worker (host/fetch-bridge.js) do the same way,
// from one source (review 22bb061 §7: two copies of the hash, the retry rule, the budget and the
// answers' checks drifted apart unseen). An ES module for the page; crates/sake/build.sh puts it,
// its `export`s taken off, before fetch-bridge.js in sake.js, whose network worker gets the
// functions as source (masuFetchWorkerSource): so each stays self-contained, naming only what is
// here or in the worker.

export var MASU_RETRY_STATUS = [0, 408, 425, 429, 500, 502, 503, 504];

/** A store chunk: content-addressed, the same bytes forever. */
export function masuIsChunk(url) {
  return /\/chunks\/[0-9a-f]{2}\/[0-9a-f]{32}$/.test(url);
}

/**
 * The first 16 bytes of `data`'s BLAKE3 hash, as hex: a store chunk's name (crates/masu/src/pack.rs).
 * One-shot (a chunk at once: 4 MiB on average, 64 MiB at most), all of it in this function: the
 * network worker gets it as source.
 */
export function masuBlake3(data) {
  // (wasm SIMD where there is: ~6-10x masuBlake3Js's speed; that was the network worker's
  // busiest code while a game loads)
  const fast = data.length > 1024 && masuBlake3Simd();
  return fast ? fast(data) : masuBlake3Js(data);
}

/** masuBlake3 in plain JavaScript (where there is no wasm SIMD, and small inputs). */
export function masuBlake3Js(data) {
  const IV = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
  const PERM = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];
  const S = [];
  let s = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
  for (let r = 0; r < 7; r++) {
    S.push(s);
    s = PERM.map((p) => s[p]);
  }
  const bytes = new Uint8Array(64);
  const m = new Uint32Array(bytes.buffer);
  const out = new Uint32Array(16);
  // the compression function: out = the 16 words for chaining value `h` (8 words at `ho`), block m
  const compress = (h, ho, counter, len, flags) => {
    let v0 = h[ho], v1 = h[ho + 1], v2 = h[ho + 2], v3 = h[ho + 3], v4 = h[ho + 4], v5 = h[ho + 5], v6 = h[ho + 6], v7 = h[ho + 7];
    let v8 = IV[0], v9 = IV[1], v10 = IV[2], v11 = IV[3], v12 = counter | 0, v13 = (counter / 0x100000000) | 0, v14 = len, v15 = flags;
    for (let r = 0; r < 7; r++) {
      const q = S[r];
      // columns
      v0 = (v0 + v4 + m[q[0]]) | 0; v12 ^= v0; v12 = (v12 >>> 16) | (v12 << 16);
      v8 = (v8 + v12) | 0; v4 ^= v8; v4 = (v4 >>> 12) | (v4 << 20);
      v0 = (v0 + v4 + m[q[1]]) | 0; v12 ^= v0; v12 = (v12 >>> 8) | (v12 << 24);
      v8 = (v8 + v12) | 0; v4 ^= v8; v4 = (v4 >>> 7) | (v4 << 25);
      v1 = (v1 + v5 + m[q[2]]) | 0; v13 ^= v1; v13 = (v13 >>> 16) | (v13 << 16);
      v9 = (v9 + v13) | 0; v5 ^= v9; v5 = (v5 >>> 12) | (v5 << 20);
      v1 = (v1 + v5 + m[q[3]]) | 0; v13 ^= v1; v13 = (v13 >>> 8) | (v13 << 24);
      v9 = (v9 + v13) | 0; v5 ^= v9; v5 = (v5 >>> 7) | (v5 << 25);
      v2 = (v2 + v6 + m[q[4]]) | 0; v14 ^= v2; v14 = (v14 >>> 16) | (v14 << 16);
      v10 = (v10 + v14) | 0; v6 ^= v10; v6 = (v6 >>> 12) | (v6 << 20);
      v2 = (v2 + v6 + m[q[5]]) | 0; v14 ^= v2; v14 = (v14 >>> 8) | (v14 << 24);
      v10 = (v10 + v14) | 0; v6 ^= v10; v6 = (v6 >>> 7) | (v6 << 25);
      v3 = (v3 + v7 + m[q[6]]) | 0; v15 ^= v3; v15 = (v15 >>> 16) | (v15 << 16);
      v11 = (v11 + v15) | 0; v7 ^= v11; v7 = (v7 >>> 12) | (v7 << 20);
      v3 = (v3 + v7 + m[q[7]]) | 0; v15 ^= v3; v15 = (v15 >>> 8) | (v15 << 24);
      v11 = (v11 + v15) | 0; v7 ^= v11; v7 = (v7 >>> 7) | (v7 << 25);
      // diagonals
      v0 = (v0 + v5 + m[q[8]]) | 0; v15 ^= v0; v15 = (v15 >>> 16) | (v15 << 16);
      v10 = (v10 + v15) | 0; v5 ^= v10; v5 = (v5 >>> 12) | (v5 << 20);
      v0 = (v0 + v5 + m[q[9]]) | 0; v15 ^= v0; v15 = (v15 >>> 8) | (v15 << 24);
      v10 = (v10 + v15) | 0; v5 ^= v10; v5 = (v5 >>> 7) | (v5 << 25);
      v1 = (v1 + v6 + m[q[10]]) | 0; v12 ^= v1; v12 = (v12 >>> 16) | (v12 << 16);
      v11 = (v11 + v12) | 0; v6 ^= v11; v6 = (v6 >>> 12) | (v6 << 20);
      v1 = (v1 + v6 + m[q[11]]) | 0; v12 ^= v1; v12 = (v12 >>> 8) | (v12 << 24);
      v11 = (v11 + v12) | 0; v6 ^= v11; v6 = (v6 >>> 7) | (v6 << 25);
      v2 = (v2 + v7 + m[q[12]]) | 0; v13 ^= v2; v13 = (v13 >>> 16) | (v13 << 16);
      v8 = (v8 + v13) | 0; v7 ^= v8; v7 = (v7 >>> 12) | (v7 << 20);
      v2 = (v2 + v7 + m[q[13]]) | 0; v13 ^= v2; v13 = (v13 >>> 8) | (v13 << 24);
      v8 = (v8 + v13) | 0; v7 ^= v8; v7 = (v7 >>> 7) | (v7 << 25);
      v3 = (v3 + v4 + m[q[14]]) | 0; v14 ^= v3; v14 = (v14 >>> 16) | (v14 << 16);
      v9 = (v9 + v14) | 0; v4 ^= v9; v4 = (v4 >>> 12) | (v4 << 20);
      v3 = (v3 + v4 + m[q[15]]) | 0; v14 ^= v3; v14 = (v14 >>> 8) | (v14 << 24);
      v9 = (v9 + v14) | 0; v4 ^= v9; v4 = (v4 >>> 7) | (v4 << 25);
    }
    out[0] = v0 ^ v8; out[1] = v1 ^ v9; out[2] = v2 ^ v10; out[3] = v3 ^ v11;
    out[4] = v4 ^ v12; out[5] = v5 ^ v13; out[6] = v6 ^ v14; out[7] = v7 ^ v15;
  };
  // (words are little-endian: m is a Uint32Array over the block's bytes, as every browser is)
  const chunk = (i, root) => {
    const at = i * 1024;
    const end = Math.min(data.length, at + 1024);
    const blocks = Math.max(1, Math.ceil((end - at) / 64));
    const cv = new Uint32Array(IV);
    for (let b = 0; b < blocks; b++) {
      const from = at + b * 64;
      const n = Math.min(64, end - from);
      if (n === 64) bytes.set(data.subarray(from, from + 64));
      else {
        bytes.fill(0);
        if (n > 0) bytes.set(data.subarray(from, end));
      }
      const last = b === blocks - 1;
      compress(cv, 0, i, Math.max(n, 0), (b === 0 ? 1 : 0) | (last ? 2 : 0) | (last && root ? 8 : 0));
      cv.set(out.subarray(0, 8));
    }
    return cv;
  };
  const parent = (l, r, root) => {
    m.set(l);
    m.set(r, 8);
    compress(IV, 0, 0, 64, 4 | (root ? 8 : 0));
    return out.slice(0, 8);
  };
  const n = Math.max(1, Math.ceil(data.length / 1024));
  let cv;
  if (n === 1) cv = chunk(0, true);
  else {
    const stack = [];
    for (let i = 0; i < n - 1; i++) {
      let c = chunk(i, false);
      for (let t = i + 1; (t & 1) === 0; t >>= 1) c = parent(stack.pop(), c, false);
      stack.push(c);
    }
    cv = chunk(n - 1, false);
    while (stack.length > 1) cv = parent(stack.pop(), cv, false);
    cv = parent(stack.pop(), cv, true);
  }
  let hex = '';
  for (let k = 0; k < 16; k++) hex += ((cv[k >> 2] >>> ((k & 3) * 8)) & 255).toString(16).padStart(2, '0');
  return hex;
}

/**
 * BLAKE3 with wasm SIMD (masuBlake3's fast path): a module made here, byte by byte, whose
 * `h(in, stride, n, blocks, counter, step, flags, start, end, lastLen, out)` hashes `n` (1-4)
 * inputs at once, one per i32x4 lane (input l at in + l*stride: `blocks` 64-byte blocks, the
 * last `lastLen` bytes long, zero-padded; counter + l*step; `flags` on every block, `start` on
 * the first, `end` on the last), and stores each one's chaining value (8 words) at out + l*32.
 * Whole chunks go four at a time, then the tree's parents four at a time, level by level.
 * Returns hash(data) -> the first 16 bytes as hex, or null where wasm SIMD is missing.
 */
export function masuBlake3Simd() {
  if (masuBlake3Simd.made) return masuBlake3Simd.hash;
  masuBlake3Simd.made = true;
  const IV = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
  const PERM = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];
  const leb = (v) => {
    const o = [];
    for (;;) {
      const b = v & 0x7f;
      v >>= 7;
      if ((v === 0 && !(b & 0x40)) || (v === -1 && b & 0x40)) return o.push(b), o;
      o.push(b | 0x80);
    }
  };
  const uleb = (v) => {
    const o = [];
    do {
      let b = v & 0x7f;
      v >>>= 7;
      if (v) b |= 0x80;
      o.push(b);
    } while (v);
    return o;
  };
  const c = [];
  // params 0-10 (i32): in stride n blocks counter step flags start end lastLen out
  const [IN, STRIDE, N, BLOCKS, COUNTER, STEP, FLAGS, START, END, LASTLEN, OUT] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
  // i32 locals 11-16: a0-a3 (each lane's address), b (block), f (this block's flags)
  const A = 11, B = 15, F = 16;
  // v128 locals from 17: h0-7, v0-15, m0-15, ctr
  const H = 17, V = 25, M = 41, CTR = 57;
  const lget = (i) => c.push(0x20, ...uleb(i));
  const lset = (i) => c.push(0x21, ...uleb(i));
  const i32 = (v) => c.push(0x41, ...leb(v | 0));
  const simd = (op, ...imm) => c.push(0xfd, ...uleb(op), ...imm);
  const splatConst = (v) => (i32(v), simd(0x11));
  const add = () => simd(0xae), xor = () => simd(0x51), or = () => simd(0x50);
  const shuffle = (lanes) => simd(0x0d, ...lanes);
  const ROT16 = [], ROT8 = [];
  for (let i = 0; i < 4; i++) ROT16.push(4 * i + 2, 4 * i + 3, 4 * i, 4 * i + 1), ROT8.push(4 * i + 1, 4 * i + 2, 4 * i + 3, 4 * i);
  // x = rotr(x, r) on the stack
  const rot = (r, tmp) => {
    if (r === 16 || r === 8) {
      lset(tmp);
      lget(tmp);
      lget(tmp);
      shuffle(r === 16 ? ROT16 : ROT8);
      return;
    }
    lset(tmp);
    lget(tmp);
    i32(r);
    simd(0xad); // shr_u
    lget(tmp);
    i32(32 - r);
    simd(0xab); // shl
    or();
  };
  const T = CTR + 1; // a scratch v128
  const g = (a, b, cc, d, x, y) => {
    const v = (k) => V + k;
    // a = a + b + x; d = rotr(d ^ a, 16); c = c + d; b = rotr(b ^ c, 12)
    lget(v(a)), lget(v(b)), add(), lget(M + x), add(), lset(v(a));
    lget(v(d)), lget(v(a)), xor(), rot(16, T), lset(v(d));
    lget(v(cc)), lget(v(d)), add(), lset(v(cc));
    lget(v(b)), lget(v(cc)), xor(), rot(12, T), lset(v(b));
    lget(v(a)), lget(v(b)), add(), lget(M + y), add(), lset(v(a));
    lget(v(d)), lget(v(a)), xor(), rot(8, T), lset(v(d));
    lget(v(cc)), lget(v(d)), add(), lset(v(cc));
    lget(v(b)), lget(v(cc)), xor(), rot(7, T), lset(v(b));
  };
  // lane addresses: in + min(l, n - 1) * stride
  for (let l = 0; l < 4; l++) {
    lget(IN);
    i32(l);
    lget(N);
    i32(1);
    c.push(0x6b); // i32.sub
    i32(l);
    lget(N);
    i32(1);
    c.push(0x6b);
    c.push(0x48); // i32.lt_s: l < n - 1 ? l : n - 1
    c.push(0x1b); // select
    lget(STRIDE);
    c.push(0x6c); // i32.mul
    c.push(0x6a); // i32.add
    lset(A + l);
  }
  // counters: counter + l * step
  lget(COUNTER), simd(0x11);
  lget(STEP), simd(0x11);
  simd(0x0c, ...[0, 1, 2, 3].flatMap((k) => [k, 0, 0, 0]));
  simd(0xb5); // i32x4.mul
  add();
  lset(CTR);
  for (let k = 0; k < 8; k++) splatConst(IV[k]), lset(H + k);
  i32(0), lset(B);
  // loop over the blocks
  c.push(0x02, 0x40, 0x03, 0x40); // block, loop
  {
    // f = flags | (b == 0 ? start : 0) | (b == blocks - 1 ? end : 0)
    lget(FLAGS);
    lget(START), i32(0), lget(B), c.push(0x45, 0x1b); // b == 0 (i32.eqz) ? start : 0
    c.push(0x72); // i32.or
    lget(END), i32(0), lget(B), lget(BLOCKS), i32(1), c.push(0x6b, 0x46, 0x1b); // b == blocks-1
    c.push(0x72);
    lset(F);
    // m_w: word w of each lane's block (all loaded before anything is stored)
    for (let w = 0; w < 16; w++) {
      simd(0x0c, ...new Array(16).fill(0));
      for (let l = 0; l < 4; l++) {
        lset(T);
        lget(A + l);
        lget(B), i32(6), c.push(0x74), c.push(0x6a); // + (b << 6)
        lget(T);
        simd(0x56, 2, ...uleb(4 * w), l); // v128.load32_lane align=2 offset=4w lane l
      }
      lset(M + w);
    }
    for (let k = 0; k < 8; k++) lget(H + k), lset(V + k);
    for (let k = 0; k < 4; k++) splatConst(IV[k]), lset(V + 8 + k);
    lget(CTR), lset(V + 12);
    simd(0x0c, ...new Array(16).fill(0)), lset(V + 13);
    // len: lastLen on the last block, else 64
    lget(LASTLEN), i32(64), lget(B), lget(BLOCKS), i32(1), c.push(0x6b, 0x46, 0x1b);
    simd(0x11), lset(V + 14);
    lget(F), simd(0x11), lset(V + 15);
    let s = [...Array(16).keys()];
    for (let r = 0; r < 7; r++) {
      g(0, 4, 8, 12, s[0], s[1]);
      g(1, 5, 9, 13, s[2], s[3]);
      g(2, 6, 10, 14, s[4], s[5]);
      g(3, 7, 11, 15, s[6], s[7]);
      g(0, 5, 10, 15, s[8], s[9]);
      g(1, 6, 11, 12, s[10], s[11]);
      g(2, 7, 8, 13, s[12], s[13]);
      g(3, 4, 9, 14, s[14], s[15]);
      s = PERM.map((p) => s[p]);
    }
    for (let k = 0; k < 8; k++) lget(V + k), lget(V + 8 + k), xor(), lset(H + k);
    // b++, again while b < blocks
    lget(B), i32(1), c.push(0x6a), lset(B);
    lget(B), lget(BLOCKS), c.push(0x48, 0x0d, 0x00); // i32.lt_s, br_if 0 (the loop)
  }
  c.push(0x0b, 0x0b); // end loop, end block
  // stores: lane l (l < n) to out + l*32
  for (let l = 0; l < 4; l++) {
    if (l) lget(N), i32(l), c.push(0x4a, 0x04, 0x40); // n > l: if
    for (let k = 0; k < 8; k++) {
      lget(OUT);
      lget(H + k);
      simd(0x5a, 2, ...uleb(l * 32 + 4 * k), l); // v128.store32_lane
    }
    if (l) c.push(0x0b);
  }
  c.push(0x0b); // end function
  const locals = [0x02, 6, 0x7f, 42, 0x7b]; // 6 i32, 42 v128 (h, v, m, ctr, scratch)
  const body = [...locals, ...c];
  const sec = (id, bytes) => [id, ...uleb(bytes.length), ...bytes];
  const name = (s) => [s.length, ...[...s].map((ch) => ch.charCodeAt(0))];
  const bytes = new Uint8Array([
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    ...sec(1, [1, 0x60, 11, ...new Array(11).fill(0x7f), 0]),
    ...sec(3, [1, 0]),
    ...sec(5, [1, 0, 1]),
    ...sec(7, [2, ...name('h'), 0, 0, ...name('m'), 2, 0]),
    ...sec(10, [1, ...uleb(body.length), ...body]),
  ]);
  let inst;
  try {
    inst = new WebAssembly.Instance(new WebAssembly.Module(bytes));
  } catch (_) {
    return (masuBlake3Simd.hash = null);
  }
  const h = inst.exports.h, mem = inst.exports.m;
  masuBlake3Simd.memory = mem;
  const hash = (data) => {
    const len = data.length;
    const n = Math.max(1, Math.ceil(len / 1024));
    // memory: the chunks' chaining values (2 MiB for a 64 MiB chunk), then a window of the
    // input that whole chunks are copied into 256 at a time (not all of it: the memory stays)
    const cvBytes = Math.ceil((n * 32) / 64) * 64;
    const WIN = 256;
    const need = cvBytes + WIN * 1024 + 64;
    if (mem.buffer.byteLength < need) mem.grow(Math.ceil((need - mem.buffer.byteLength) / 65536));
    const u8 = new Uint8Array(mem.buffer);
    const full = n - 1;
    for (let w = 0; w < full; w += WIN) {
      const k = Math.min(WIN, full - w);
      u8.set(data.subarray(w * 1024, (w + k) * 1024), cvBytes);
      for (let i = 0; i < k; i += 4) h(cvBytes + i * 1024, 1024, Math.min(4, k - i), 16, w + i, 1, 0, 1, 2, 64, (w + i) * 32);
    }
    // the last chunk (the root, when it is the only one), zero-padded
    const rest = len - full * 1024;
    const blocks = Math.max(1, Math.ceil(rest / 64));
    u8.set(data.subarray(full * 1024), cvBytes);
    u8.fill(0, cvBytes + rest, cvBytes + blocks * 64);
    h(cvBytes, 1024, 1, blocks, full, 0, 0, 1, n === 1 ? 10 : 2, rest - (blocks - 1) * 64, full * 32);
    // parents, level by level (an odd one out goes up as it is: BLAKE3's tree)
    let count = n;
    while (count > 2) {
      const pairs = count >> 1;
      for (let k = 0; k < pairs; k += 4) h(k * 64, 64, Math.min(4, pairs - k), 1, 0, 0, 4, 0, 0, 64, k * 32);
      if (count & 1) u8.copyWithin(pairs * 32, (count - 1) * 32, count * 32);
      count = pairs + (count & 1);
    }
    if (count === 2) h(0, 64, 1, 1, 0, 0, 12, 0, 0, 64, 0);
    let hex = '';
    for (let k = 0; k < 16; k++) hex += u8[k].toString(16).padStart(2, '0');
    return hex;
  };
  return (masuBlake3Simd.hash = hash);
}

/** The delay before retry `attempt` (from 0): exponential with jitter, at most 8 s. */
export function masuBackoff(attempt) {
  return Math.min(8000, 300 * 2 ** attempt) * (0.5 + Math.random() * 0.5);
}

/** Request classes, most urgent first. */
export var MASU_CLASS = { game: 0, ahead: 1, hint: 2, warm: 3, save: 4 };
/** Requests per second the budget refills by, and holds at most. */
export var MASU_RATE = 9;
export var MASU_BURST = 40;
/** Requests each class leaves to the classes above it; the game's may run the budget into debt
 * (to -MASU_RATE * 5: a game waits for no fetch ahead's budget, and its bursts are short). */
export var MASU_RESERVE = [-MASU_RATE * 5, 0, 8, 16, 16];

/** The budget's share as a number in (0, 1]. */
export function masuBudgetShare(v) {
  return v[3] > 0 && v[3] <= 1 ? v[3] : 1;
}

/** A well-formed answer of /store-direct for one chunk. */
export function masuDirectValid(info) {
  // (strings first: a regex's test of an array or an object tests its text)
  return !!info && typeof info.url === 'string' && typeof info.key === 'string' && /^[0-9a-f]{64}$/.test(info.key) && typeof info.object === 'string' && /^[0-9a-f-]{36}$/.test(info.object);
}

/** A pack run's bytes at most by class (MASU_CLASS), and its chunks at most (fetch-bridge.js masuPackRun). */
export var MASU_PACK_SPAN = [4 << 20, 8 << 20, 16 << 20, 32 << 20, 32 << 20];
export var MASU_PACK_MAX = 64;
/** A well-formed place of a chunk in a delivery pack (store-direct's `pack`). */
export function masuPackValid(p) {
  return !!p && typeof p.url === 'string' && typeof p.id === 'string' && /^[0-9a-f-]{36}$/.test(p.id) && Number.isSafeInteger(p.offset) && p.offset >= 0 && Number.isSafeInteger(p.len) && p.len >= 28;
}

/** Fresh budget values made at `now`: a full burst, refilled from now, nobody blocked, the whole share. */
export function masuBudgetFresh(v, now) {
  v.set([MASU_BURST, now, 0, 1]);
}

/** Takes one request of class `cls` from budget values `v` (refilled to `now`): whether it may
 * go. While the bucket asked to wait (a 429) no class goes, the game's neither. */
export function masuBudgetTakeStep(v, cls, now) {
  if (now < v[2]) return false;
  const s = masuBudgetShare(v);
  const dt = Math.max(0, now - v[1]) / 1000;
  v[0] = Math.min(MASU_BURST * s, v[0] + dt * MASU_RATE * s);
  v[1] = Math.max(v[1], now);
  if (v[0] - 1 < (MASU_RESERVE[cls] ?? 0) * s) return false;
  v[0] -= 1;
  return true;
}

/** The bucket said 429: nobody goes until `until`, and the budget refills only from then on, from
 * none (not a burst of every class at once when the wait ends). */
export function masuBudgetDrainStep(v, until) {
  v[2] = Math.max(v[2], until);
  v[0] = Math.min(v[0], 0);
  v[1] = Math.max(v[1], until);
}

/** The shared budget's bytes: an Int32 lock word, then 4 Float64 values (fetch-bridge.js masuBudgetWith). */
export var MASU_BUDGET_BYTES = 40;
