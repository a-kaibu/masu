// web/src/session.ts inputTarget in real Chromium (review 22bb061 §5): a Tab typed in the
// page's own field, or a key in an open dialog, is the page's (not forwarded, its default kept:
// the focus moves on); a key with the focus on the page or on the bar's button is the game's.
// node tests/input-target-browser.mjs (PLAYWRIGHT, CHROME as in fetch-browser.mjs)
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
const pw = await import(process.env.PLAYWRIGHT ?? "playwright");
const { chromium } = pw.default ?? pw;

const ts = createRequire(new URL("../web/package.json", import.meta.url))("typescript");
const src = (f) =>
  ts.transpileModule(readFileSync(`web/src/${f}.ts`, "utf8"), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  }).outputText;
const html = `<!doctype html><title>input</title>
<canvas id="screen" width="64" height="64"></canvas>
<button id="auto">自動送り</button>
<input id="memo"><input id="next">
<div role="dialog" id="dlg"><button id="inside">OK</button></div>`;
const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROME, chromiumSandbox: false });
try {
  const page = await browser.newPage();
  await page.route("http://localhost:9/**", (r) =>
    new URL(r.request().url()).pathname === "/"
      ? r.fulfill({ contentType: "text/html", body: html })
      : r.fulfill({ contentType: "text/javascript", body: src("session") }),
  );
  await page.goto("http://localhost:9/");
  await page.evaluate(async () => {
    const { inputTarget } = await import("/session.js");
    // (as play.ts attachInput: the game's keys taken from the page, the page's left alone)
    window.sent = [];
    addEventListener("keydown", (e) => {
      if (inputTarget(e.target) === "overlay") return;
      window.sent.push(e.code);
      e.preventDefault();
    });
  });
  const sent = () => page.evaluate(() => window.sent.splice(0));
  // a Tab in the field: not the game's, and the focus moves to the next field
  await page.focus("#memo");
  await page.keyboard.press("Tab");
  assert.deepEqual(await sent(), []);
  assert.equal(await page.evaluate(() => document.activeElement?.id), "next");
  await page.keyboard.type("ab");
  assert.equal(await page.inputValue("#next"), "ab");
  assert.deepEqual(await sent(), []);
  // a key in the open dialog: the page's
  await page.focus("#inside");
  await page.keyboard.press("Enter");
  assert.deepEqual(await sent(), []);
  // the bar's button clicked, then a key: the game's, as before
  await page.click("#auto");
  await page.keyboard.press("Space");
  assert.deepEqual(await sent(), ["Space"]);
  // the page itself (the game's screen): the game's
  await page.evaluate(() => document.activeElement?.blur());
  await page.keyboard.press("Tab");
  assert.deepEqual(await sent(), ["Tab"]);
  console.log("input-target: ok");
} finally {
  await browser.close();
}
