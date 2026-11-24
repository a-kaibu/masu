// One build's round (see README.md), for node (bench.mjs) and a browser page (browser.mjs):
// run(bytes of jitbench.wasm, options) gives one result per program: the cold run (warm-up,
// translation, V8's compile), the hot runs, the translator's and V8's times, module bytes, the
// interpreter's share, the checksum.
//   options: only (program names), knobs ("fn=1,hoist=0,ssa=1"), hot (runs), scale,
//   module(bytes, index) -> bytes: each translated module as it is loaded (dump, swap)
export function run(rtBytes, { only = '', knobs = '', hot: HOT = 7, scale: SCALE = 1, module = (b) => b, memoryMb = () => 0 } = {}) {
  const memory = new WebAssembly.Memory({ initial: 0x9000_0000 / 65536, maximum: 65535, shared: true });
  const table = new WebAssembly.Table({ element: 'anyfunc', initial: 0 });
  let used = 0;
  let loaded = 0;
  let loadMs = 0;
  let dispatch = null;
  let env = null;
  const failed = [];
  const jb = {
    jb_dispatch: (cpu) => dispatch(cpu),
    jb_now: () => performance.now(),
    jb_load: (ptr, len, n) => {
      const t0 = performance.now();
      let inst;
      try {
        const bytes = module(new Uint8Array(memory.buffer, ptr >>> 0, len).slice(), loaded++);
        inst = new WebAssembly.Instance(new WebAssembly.Module(bytes), { env });
      } catch (e) {
        // (the interpreter runs that code then: a round measuring that is no measure)
        failed.push(String(e));
        return -1;
      }
      if (used + n > table.length) table.grow(Math.max(n, table.length >> 1, 256));
      const base = used;
      used += n;
      for (let i = 0; i < n; i++) table.set(base + i, inst.exports['f' + i]);
      loadMs += performance.now() - t0;
      return base;
    },
  };
  const ex = new WebAssembly.Instance(new WebAssembly.Module(rtBytes), { jb, env: { memory } }).exports;
  // (pages: koji::jit::PAGES, the page marks' address; the checks are off here, so never read)
  env = { memory, table, step: ex.jb_step, eflags: ex.jb_eflags, status: ex.jb_status, x87: ex.jb_x87, callmiss: ex.jb_callmiss, pages: 0 };
  {
    const n = ex.jb_dispatch_module();
    const bytes = new Uint8Array(memory.buffer, ex.jb_buf() >>> 0, n).slice();
    dispatch = new WebAssembly.Instance(new WebAssembly.Module(bytes), { env: { memory, table } }).exports.f0;
  }
  const knob = { fn: 0, hoist: 1, memcount: 2, mxexact: 3, nojit: 4, ssa: 5 };
  ex.jb_set(0, 1); // function mode, as kura
  let nojit = false;
  for (const kv of knobs.split(',').filter(Boolean)) {
    const [k, v] = kv.split('=');
    if (!(k in knob)) throw new Error(`no knob ${k} (${Object.keys(knob).join(', ')})`);
    ex.jb_set(knob[k], Number(v));
    if (k === 'nojit') nojit = (Number(v) >>> 0) !== 0;
  }
  const STATS = ['translate_ms', 'translations', 'wasm_bytes', 'guest_ins', 'interp_steps', 'step_calls', 'exit_interp', 'exit_continue', 'dispatches', 'load_fail', 'cpu_loads', 'cpu_stores'];
  const names = [];
  for (let i = 0; i < ex.jb_programs(); i++) {
    const n = ex.jb_program_name(i);
    names.push(new TextDecoder().decode(new Uint8Array(memory.buffer, ex.jb_buf() >>> 0, n).slice()));
  }
  const want = only ? only.split(',') : names;
  if (!want.length || want.some((name) => !names.includes(name))) throw new Error(`unknown benchmark selection: ${only}`);
  if (!Number.isInteger(HOT) || HOT < 1) throw new Error('hot runs must be a positive integer');
  const stats = () => Object.fromEntries(STATS.map((s, k) => [s, ex.jb_stat(k)]));
  const out = [];
  for (const [i, name] of names.entries()) {
    if (!want.includes(name)) continue;
    loadMs = 0;
    if (!ex.jb_setup(i, SCALE, 0)) throw new Error(`${name}: cold setup failed`);
    let t0 = performance.now();
    const r = ex.jb_run();
    const cold = performance.now() - t0;
    const coldStats = stats();
    if (coldStats.load_fail) throw new Error(`${name}: translation failed to load: ${failed[0] ?? 'unknown error'}`);
    if (r !== 0) throw new Error(`${name}: cold run failed with status ${r}`);
    if (!nojit && coldStats.translations < 1) throw new Error(`${name}: cold run made no translations`);
    const coldLoad = loadMs;
    const sum = ex.jb_checksum() >>> 0;
    const hot = [];
    let hotStats = null;
    for (let k = 0; k < HOT; k++) {
      if (!ex.jb_setup(i, SCALE, 1)) throw new Error(`${name}: hot setup failed`);
      t0 = performance.now();
      const r2 = ex.jb_run();
      hot.push(performance.now() - t0);
      hotStats = stats();
      if (r2 !== 0) throw new Error(`${name}: hot run ${k} failed with status ${r2}`);
      if ((ex.jb_checksum() >>> 0) !== sum) throw new Error(`${name}: hot run ${k} checksum differs`);
    }
    hot.sort((a, b) => a - b);
    out.push({ name, status: r, sum: sum.toString(16), cold, hot_min: hot[0], hot_med: hot[hot.length >> 1], load_ms: coldLoad, cold_stats: coldStats, hot_stats: hotStats, rss_mb: memoryMb() });
  }
  if (failed.length) throw new Error(`${failed.length} translations failed to load: ${failed[0]}`);
  return out;
}


// Compare all samples before reporting any timing ratios.
export function validateComparison({ A, B }) {
  const names = A[0]?.map((p) => p.name) ?? [];
  const rounds = [...A, ...B];
  if (!names.length || !B.length || rounds.some((round) => round.length !== names.length || names.some((name) => !round.some((p) => p.name === name)))) {
    throw new Error('A/B benchmark programs differ');
  }
  for (const name of names) {
    const samples = rounds.map((round) => round.find((p) => p.name === name));
    if (samples.some((p) => p.status !== 0)) throw new Error(`${name}: A/B run failed`);
    const sums = new Set(samples.map((p) => p.sum));
    if (sums.size !== 1) throw new Error(`${name}: A/B checksums differ: ${[...sums].join(', ')}`);
  }
  return names;
}
