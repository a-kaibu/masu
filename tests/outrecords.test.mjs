// host/outrecords.js: the page's output records into printChar's bytes, NULs left out (a NUL ended
// a console line in the middle: review e18bf44). Random records against the bytes they hold.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const ctx = {};
vm.runInNewContext(readFileSync(new URL("../host/outrecords.js", import.meta.url), "utf8"), ctx);
const walk = (b, n) => {
  const got = [];
  ctx.vnhostOutputRecords(b, n, (fd, c) => got.push([fd, c]));
  return got;
};

test("records come out byte for byte, without NULs, in order, fd by fd", () => {
  let x = 7;
  const rnd = (k) => ((x = (Math.imul(x, 1103515245) + 12345) >>> 0) >>> 8) % k;
  for (let run = 0; run < 500; run++) {
    const recs = Array.from({ length: rnd(6) }, () => [1 + rnd(2), Array.from({ length: rnd(40) }, () => (rnd(5) ? 32 + rnd(90) : 0))]);
    const bytes = recs.flatMap(([fd, b]) => [fd, b.length & 255, b.length >> 8, ...b]);
    const want = recs.flatMap(([fd, b]) => b.filter((c) => c !== 0).map((c) => [fd, c]));
    assert.deepEqual(walk(Uint8Array.from(bytes), bytes.length), want, `run ${run}`);
    // (cut short: only the whole records before the cut)
    const cut = rnd(bytes.length + 1);
    let at = 0;
    const whole = [];
    for (const [fd, b] of recs) {
      if (at + 3 + b.length > cut) break;
      whole.push(...b.filter((c) => c !== 0).map((c) => [fd, c]));
      at += 3 + b.length;
    }
    assert.deepEqual(walk(Uint8Array.from(bytes), cut), whole, `run ${run} cut ${cut}`);
  }
});

test("a line with a NUL inside stays one line", () => {
  const text = [...new TextEncoder().encode("chain 2: args [/windo"), 0, ...new TextEncoder().encode("w] gamedata yes\n")];
  const b = Uint8Array.from([2, text.length, 0, ...text]);
  const line = String.fromCharCode(...walk(b, b.length).map(([, c]) => c));
  assert.equal(line, "chain 2: args [/window] gamedata yes\n");
});
