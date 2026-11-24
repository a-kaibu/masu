import assert from 'node:assert/strict';
import test from 'node:test';
import { run, validateComparison } from './core.mjs';

function fakeRuntime(t, { status = [0, 0], sums = [42, 42], translations = 1, loadFail = 0 } = {}) {
  const buffer = new ArrayBuffer(32);
  const name = new TextEncoder().encode('sort');
  new Uint8Array(buffer).set(name);
  let phase = 0;
  let instances = 0;
  const ex = {
    jb_dispatch_module: () => 0,
    jb_buf: () => 0,
    jb_set: () => {},
    jb_programs: () => 1,
    jb_program_name: () => name.length,
    jb_setup: (_i, _scale, keep) => { phase = keep; return 1; },
    jb_run: () => status[phase],
    jb_checksum: () => sums[phase],
    jb_stat: (i) => i === 1 && phase === 0 ? translations : i === 9 && phase === 0 ? loadFail : 0,
  };
  for (const [key, implementation] of Object.entries({
    Memory: class { constructor() { return { buffer }; } },
    Table: class {},
    Module: class {},
    Instance: class { constructor() { return { exports: instances++ === 0 ? ex : { f0: () => {} } }; } },
  })) t.mock.method(WebAssembly, key, implementation);
}

test('rejects equally unsuccessful cold and hot runs', (t) => {
  fakeRuntime(t, { status: [1, 1] });
  assert.throws(() => run(null, { hot: 1 }), /cold run failed with status 1/);
});

test('rejects unsuccessful hot runs and checksum changes', async (t) => {
  await t.test('hot status', (t) => {
    fakeRuntime(t, { status: [0, 1] });
    assert.throws(() => run(null, { hot: 1 }), /hot run 0 failed with status 1/);
  });
  await t.test('hot checksum', (t) => {
    fakeRuntime(t, { sums: [42, 43] });
    assert.throws(() => run(null, { hot: 1 }), /hot run 0 checksum differs/);
  });
});

test('requires translations for JIT runs and permits an explicit interpreter baseline', async (t) => {
  await t.test('JIT', (t) => {
    fakeRuntime(t, { translations: 0 });
    assert.throws(() => run(null, { hot: 1 }), /made no translations/);
  });
  await t.test('interpreter', (t) => {
    fakeRuntime(t, { translations: 0 });
    assert.equal(run(null, { hot: 1, knobs: 'nojit=1' })[0].sum, '2a');
  });
  await t.test('failed load', (t) => {
    fakeRuntime(t, { loadFail: 1 });
    assert.throws(() => run(null, { hot: 1 }), /translation failed to load/);
  });
});

test('checks program coverage and every A/B sample before reporting timings', () => {
  const p = { name: 'sort', status: 0, sum: '2a' };
  const A = [[p], [p]];
  assert.deepEqual(validateComparison({ A, B: [[p], [p]] }), ['sort']);
  assert.throws(() => validateComparison({ A, B: [[p], [{ ...p, sum: '2b' }]] }), /checksums differ/);
  assert.throws(() => validateComparison({ A, B: [[p], [{ ...p, status: 1 }]] }), /run failed/);
  assert.throws(() => validateComparison({ A, B: [[p], []] }), /programs differ/);
});
