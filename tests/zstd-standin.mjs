// sake's snapz (web/dist/sake/snapz.mjs) stood in for over fzstd, for the browser tests that run
// without sake built: chunks come from the bucket zstd-compressed (16b08c7), and the page decodes
// them with snapz (web/src/direct.ts snapzUnzstd). FZSTD names fzstd's esm/index.mjs (CI's
// browser job installs it beside playwright). Nothing is written when `web` has sake's own.
import assert from "node:assert/strict";
import { cpSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export function zstdStandIn(web) {
  if (existsSync(join(web, "sake/snapz.mjs"))) return;
  const fzstd = process.env.FZSTD;
  assert.ok(fzstd && existsSync(fzstd), "sake not built: FZSTD names fzstd's esm/index.mjs for the zstd stand-in");
  mkdirSync(join(web, "sake"), { recursive: true });
  cpSync(fzstd, join(web, "sake/fzstd.mjs"));
  // (snapz's calls, over fzstd: what direct.ts snapzUnzstd uses of it)
  writeFileSync(
    join(web, "sake/snapz.mjs"),
    `import { decompress } from "./fzstd.mjs";
export default async () => {
  const HEAPU8 = new Uint8Array(160 << 20);
  let top = 8, live = 0;
  return {
    HEAPU8,
    _malloc(n) { const p = top; top += (n + 7) & ~7; if (top > HEAPU8.length) { top = p; return 0; } live++; return p; },
    _free(p) { if (p && --live === 0) top = 8; },
    _snapz_decompress(dst, cap, src, n) {
      let out;
      try { out = decompress(HEAPU8.slice(src, src + n)); } catch { return 0xffffffff; }
      if (out.length > cap) return 0xffffffff;
      HEAPU8.set(out, dst);
      return out.length;
    },
  };
};
`,
  );
}
