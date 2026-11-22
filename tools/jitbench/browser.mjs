// One build's round in Chromium (see README.md), as on an Android phone: V8's explicit bounds
// checks on every access (--wasm-enforce-bounds-checks: Android's V8 has no guard regions for
// kura's memory, docs/translator.md "Android: V8's bounds checks"), Pixel 7's screen and user
// agent, the CPU slowed by CDP's Emulation.setCPUThrottlingRate (JB_THROTTLE, 4: a mid-range
// phone against this machine), the page cross-origin isolated for the shared memory (a localhost origin, served by page.route: a secure context).
//   node browser.mjs <jitbench.wasm> [programs,...] [knob=v,...]
// One JSON line per program, as bench.mjs (rss_mb: the page's JS heap).
import fs from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);

export async function runInBrowser(path, { only = '', knobs = '', hot = 7, scale = 1, throttle = 4 } = {}) {
  const { chromium, devices } = require('playwright');
  const browser = await chromium.launch({ args: ['--js-flags=--wasm-enforce-bounds-checks'] });
  try {
    const context = await browser.newContext({ ...devices['Pixel 7'] });
    const page = await context.newPage();
    const isolated = { 'Cross-Origin-Opener-Policy': 'same-origin', 'Cross-Origin-Embedder-Policy': 'require-corp' };
    const files = {
      '/index.html': ['text/html', '<!doctype html><title>jitbench</title>'],
      '/core.mjs': ['text/javascript', fs.readFileSync(new URL('core.mjs', import.meta.url))],
      '/jitbench.wasm': ['application/wasm', fs.readFileSync(path)],
    };
    await page.route('http://localhost:8765/**', (route) => {
      const f = files[new URL(route.request().url()).pathname];
      if (!f) return route.fulfill({ status: 404 });
      return route.fulfill({ status: 200, contentType: f[0], body: f[1], headers: isolated });
    });
    await page.goto('http://localhost:8765/index.html');
    const cdp = await context.newCDPSession(page);
    await cdp.send('Emulation.setCPUThrottlingRate', { rate: throttle });
    // (JB_SWAP=dir: as bench.mjs, dir/NNN.wasm loaded in that module's place)
    const swaps = {};
    for (const f of process.env.JB_SWAP ? fs.readdirSync(process.env.JB_SWAP) : []) {
      const m = /^(\d+)\.wasm$/.exec(f);
      if (m) swaps[Number(m[1])] = fs.readFileSync(`${process.env.JB_SWAP}/${f}`).toString('base64');
    }
    return await page.evaluate(
      async ({ only, knobs, hot, scale, swaps }) => {
        if (!crossOriginIsolated) throw new Error('the page is not cross-origin isolated');
        const { run } = await import('/core.mjs');
        const bytes = await (await fetch('/jitbench.wasm')).arrayBuffer();
        const module = (b, k) => (k in swaps ? Uint8Array.from(atob(swaps[k]), (c) => c.charCodeAt(0)) : b);
        return run(bytes, { only, knobs, hot, scale, module, memoryMb: () => (performance.memory?.usedJSHeapSize ?? 0) / 2 ** 20 });
      },
      { only, knobs, hot, scale, swaps },
    );
  } finally {
    await browser.close();
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [, , path, only = '', knobs = ''] = process.argv;
  const res = await runInBrowser(path, { only, knobs, hot: Number(process.env.JB_HOT ?? 7), scale: Number(process.env.JB_SCALE ?? 1), throttle: Number(process.env.JB_THROTTLE ?? 4) });
  for (const r of res) console.log(JSON.stringify(r));
}
