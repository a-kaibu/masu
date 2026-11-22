// One build's round in node (see README.md): node bench.mjs <jitbench.wasm> [programs,...] [knob=v,...]
// One JSON line per program (core.mjs). JB_HOT runs (7), JB_SCALE; JB_DUMP=dir writes each
// translated module (NNN.wasm, in the order loaded), JB_SWAP=dir loads dir/NNN.wasm in its
// place where there is one (experiments on the code V8 gets).
import fs from 'node:fs';
import { run } from './core.mjs';

const [, , path, only = '', knobs = ''] = process.argv;
const name = (dir, k) => `${dir}/${String(k).padStart(3, '0')}.wasm`;
const module = (bytes, k) => {
  const swap = process.env.JB_SWAP && name(process.env.JB_SWAP, k);
  const b = swap && fs.existsSync(swap) ? fs.readFileSync(swap) : bytes;
  if (process.env.JB_DUMP) fs.writeFileSync(name(process.env.JB_DUMP, k), b);
  return b;
};
const res = run(fs.readFileSync(path), { only, knobs, hot: Number(process.env.JB_HOT ?? 7), scale: Number(process.env.JB_SCALE ?? 1), module, memoryMb: () => process.memoryUsage().rss / 2 ** 20 });
for (const r of res) console.log(JSON.stringify(r));
