// tests/corpus.mjs on a page whose thread stops answering (a loop that never ends, as a
// page stuck inside sake): it gives up after STUCK_S, says when the page last answered, and
// stuck.txt has where the page's thread was (the loop's function). And on a page that keeps
// answering, the watchdog stays out of it (no pageStuck, the run ends as usual).
//
//   node tests/corpus-stuck-browser.mjs    (env PLAYWRIGHT as tests/corpus.mjs)
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync, existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const dir = mkdtempSync(join(tmpdir(), "corpus-stuck-"));
const run = (html, env) => {
  const page = join(dir, `p${Math.random().toString(36).slice(2)}.html`);
  writeFileSync(page, html);
  const out = `${page}.out`;
  const t = Date.now();
  const r = spawnSync(process.execPath, [join(here, "corpus.mjs"), pathToFileURL(page).href, out], {
    env: { ...process.env, ...env }, encoding: "utf8", timeout: 180_000,
  });
  assert.equal(r.status, 0, `corpus.mjs: ${r.status} ${r.signal}\n${r.stderr}`);
  return { out, secs: (Date.now() - t) / 1000, res: JSON.parse(readFileSync(`${out}/result.json`, "utf8")) };
};

// 1. stuck: the loop starts 1 s in
{
  const { out, secs, res } = run(
    "<script>function spinForeverHere() { for (;;) {} } setTimeout(spinForeverHere, 1000);</script>",
    { STUCK_S: "6", TITLE_MAX: "300", STUCK_STACK: "1" },
  );
  assert.ok(secs < 90, `gave up after ${secs} s`);
  assert.ok(typeof res.pageStuck === "number" && res.pageStuck < 6, `pageStuck ${res.pageStuck}`);
  assert.ok(res.errors.some((e) => e.startsWith("page stopped answering")), JSON.stringify(res.errors));
  const s = readFileSync(`${out}/stuck.txt`, "utf8");
  assert.match(s, /spinForeverHere/, s);
  console.log(`stuck: gave up after ${secs.toFixed(1)} s, last answer ${res.pageStuck} s; ${s.split("\n")[1].trim()}`);
}

// 2. a page that answers (no title ever: the run ends at TITLE_MAX, as before)
{
  const { out, res } = run("<p>nothing</p>", { STUCK_S: "4", TITLE_MAX: "10" });
  assert.equal(res.pageStuck, undefined);
  assert.ok(!existsSync(`${out}/stuck.txt`));
  // (the no-title end: corpus.mjs's own result, d3d set there)
  assert.equal(res.title, null);
  assert.equal(res.d3d, false, JSON.stringify(res));
  assert.deepEqual(res.errors, []);
  console.log("answering: no watchdog");
}
rmSync(dir, { recursive: true, force: true });
console.log("corpus-stuck-browser: ok");
