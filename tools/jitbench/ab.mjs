// A/B of two builds (see README.md), round by round alternating, each round a new node process
// (JB_BROWSER=1: a new Chromium as an Android phone, browser.mjs):
//   node ab.mjs A.wasm B.wasm [rounds] [programs] [knobs]
// Prints a markdown table: medians over rounds (hot: each round's median of JB_HOT runs; cold
// p95: over the rounds, the first run's tail as at a scene change), B/A. JB_KNOBS_A / JB_KNOBS_B:
// knobs for one side (after the shared ones: one build, a knob off against on).
import { execFileSync } from 'node:child_process';
import { runInBrowser } from './browser.mjs';

const [, , a, b, rounds = '5', only = '', shared = ''] = process.argv;
const node = process.execPath;
const runs = { A: [], B: [] };
for (let r = 0; r < Number(rounds); r++) {
  for (const [k, w] of r % 2 ? [['B', b], ['A', a]] : [['A', a], ['B', b]]) {
    const knobs = [shared, process.env[`JB_KNOBS_${k}`] ?? ''].filter(Boolean).join(',');
    if (process.env.JB_BROWSER) {
      runs[k].push(await runInBrowser(w, { only, knobs, hot: Number(process.env.JB_HOT ?? 7), scale: Number(process.env.JB_SCALE ?? 1), throttle: Number(process.env.JB_THROTTLE ?? 4) }));
    } else {
      const out = execFileSync(node, [new URL('bench.mjs', import.meta.url).pathname, w, only, knobs], { encoding: 'utf8', maxBuffer: 1 << 26 });
      runs[k].push(out.trim().split('\n').map((l) => JSON.parse(l)));
    }
  }
}
const med = (v) => { const s = [...v].sort((x, y) => x - y); return s[s.length >> 1]; };
const p95 = (v) => { const s = [...v].sort((x, y) => x - y); return s[Math.min(s.length - 1, Math.ceil(s.length * 0.95) - 1)]; };
const names = runs.A[0].map((p) => p.name);
const cols = [
  ['hot ms', (p) => p.hot_med],
  ['cold ms', (p) => p.cold],
  ['cold p95 ms', (p) => p.cold, p95],
  ['translate ms', (p) => p.cold_stats.translate_ms],
  ['V8 load ms', (p) => p.load_ms],
  ['wasm B', (p) => p.cold_stats.wasm_bytes],
  ['interp steps', (p) => p.cold_stats.interp_steps],
  ['step calls/hot', (p) => p.hot_stats.step_calls],
  ['exits/hot', (p) => p.hot_stats.exit_interp + p.hot_stats.exit_continue],
];
const f = (x) => (Math.abs(x) >= 100 ? x.toFixed(0) : x.toFixed(2));
console.log(`| program | ${cols.map(([c]) => c + ' A / B (B/A)').join(' | ')} | sums |`);
console.log(`|---|${cols.map(() => '---:').join('|')}|---|`);
for (const name of names) {
  const get = (k) => runs[k].map((round) => round.find((p) => p.name === name));
  const cells = cols.map(([, g, agg = med]) => {
    const x = agg(get('A').map(g));
    const y = agg(get('B').map(g));
    return `${f(x)} / ${f(y)} (${x ? (y / x).toFixed(3) : '-'})`;
  });
  const sums = new Set([...get('A'), ...get('B')].map((p) => p.sum));
  console.log(`| ${name} | ${cells.join(' | ')} | ${sums.size === 1 ? 'same' : 'DIFFER ' + [...sums].join(',')} |`);
}
const rss = (k) => med(runs[k].map((round) => Math.max(...round.map((p) => p.rss_mb))));
console.log(`\npeak ${process.env.JB_BROWSER ? 'JS heap' : 'RSS'} MB A ${rss('A').toFixed(0)} B ${rss('B').toFixed(0)}`);
