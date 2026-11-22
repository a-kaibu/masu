// A web game's files come from its frame origin's service worker (web/src/game-sw.ts), which
// asks the app's page for them over a MessagePort (web/src/gameport.ts), not from the server.
// Two plain HTTP servers stand in for masu serve's listeners: the game one serves only
// web/dist's game-boot.html, game-boot.js, game-sw.js, frame.js and frame-read.js (serve.rs GAME_FILES);
// the app one a page that does what html.ts does with the port (answers reads from files in
// memory, counting them). Checks in Chromium:
// - the boot page registers the worker and the game's page opens through it, with frame.js
//   first (Node's require there) and the saves' hello answered;
// - synchronous fs reads (sync XHR, sync-read.ts), fetch, a Range (206, 416), HEAD, 404, a
//   34 MiB file streamed whole and in a range (bytes checked by SHA-256);
// - the worker stopped (CDP ServiceWorker.stopAllWorkers) and started again: it asks for a port
//   anew and the reads go on;
// - nothing of the game's comes from the game listener: it saw no /tree/ request.
//   node tests/webgame-sw-browser.mjs   (web/dist built; PLAYWRIGHT, CHROME as elsewhere)
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { resolve } from "node:path";
const pw = await import(process.env.PLAYWRIGHT ?? "playwright");
const { chromium } = pw.default ?? pw;

const dist = resolve(process.argv[2] ?? "web/dist");
const GAME_FILES = ["game-boot.html", "game-boot.js", "game-sw.js", "frame.js", "frame-read.js"];
const big = Buffer.alloc(34 << 20);
for (let i = 0; i < big.length; i += 4) big.writeUInt32LE((i * 2654435761) >>> 0, i);
const sha = (b) => createHash("sha256").update(b).digest("hex");

const files = {
  "package.json": JSON.stringify({ main: "www/index.html" }),
  "www/data.txt": "from the game's folder",
  "www/bytes.bin": Buffer.from(Uint8Array.from({ length: 512 }, (_, i) => (i * 7) & 255)),
  "www/日本 語.json": JSON.stringify({ ok: true }),
  "www/movie.bin": big,
  "www/index.html": `<!doctype html><html><head><title>fake game</title><script>
  const out = { nodeBefore: typeof require };
  window.addEventListener("load", async () => {
    try {
      out.fs = require("fs").readFileSync("www/data.txt", "utf8");
      const b = require("fs").readFileSync("www/bytes.bin");
      out.bytes = b.length + ":" + Array.from(b).every((x, i) => x === ((i * 7) & 255));
      out.json = (await (await fetch("日本 語.json")).json()).ok;
      const r = await fetch("data.txt", { headers: { range: "bytes=5-8" } });
      out.range = r.status + ":" + (await r.text()) + ":" + r.headers.get("content-range");
      out.bad = (await fetch("data.txt", { headers: { range: "bytes=99-" } })).status;
      const h = await fetch("data.txt", { method: "HEAD" });
      out.head = h.status + ":" + h.headers.get("content-length");
      out.missing = (await fetch("none.txt")).status;
      const hex = async (buf) => Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", buf)), (x) => x.toString(16).padStart(2, "0")).join("");
      out.movie = await hex(await (await fetch("movie.bin")).arrayBuffer());
      const part = await fetch("movie.bin", { headers: { range: "bytes=1000000-30000000" } });
      out.moviePart = part.status + ":" + (await hex(await part.arrayBuffer()));
      out.coep = (await fetch("data.txt")).headers.get("cross-origin-embedder-policy");
    } catch (e) { out.error = String(e); }
    document.title = "done " + JSON.stringify(out);
  });
  </script></head><body>game</body></html>`,
};
const bytes = Object.fromEntries(Object.entries(files).map(([k, v]) => [k, Buffer.from(v)]));
const names = Object.keys(files);

const gameSeen = [];
const game = createServer((req, res) => {
  const path = decodeURIComponent(new URL(req.url, "http://x").pathname.slice(1));
  gameSeen.push(path);
  const h = {
    "cross-origin-embedder-policy": "require-corp",
    "cross-origin-resource-policy": "cross-origin",
    "cache-control": "no-cache",
  };
  if (!GAME_FILES.includes(path)) return res.writeHead(404, h).end();
  const type = path.endsWith(".html") ? "text/html; charset=utf-8" : "text/javascript; charset=utf-8";
  res.writeHead(200, { ...h, "content-type": type }).end(readFileSync(resolve(dist, path)));
});
await new Promise((ok) => game.listen(0, "127.0.0.1", ok));
const gameOrigin = `http://127.0.0.1:${game.address().port}`;

// the app's page: html.ts's part of it (the port, the reads, the saves' init)
const appPage = `<!doctype html><html><head><title>app</title></head><body><script>
// (the reads answered by a worker, as html.ts's tree-worker.ts: the frame's synchronous reads
// hold the main thread this page shares with it)
const code = \`
const files = new Map();
const reads = [];
onmessage = async (e) => {
  const m = e.data;
  if (m.load) {
    for (const [i, name] of m.load.entries())
      files.set(name.toLowerCase(), new Uint8Array(await (await fetch(m.base + "files/" + i)).arrayBuffer()));
    postMessage("loaded");
  } else if (m.reads) postMessage(reads);
  else if (m.port) m.port.onmessage = (q) => {
    const a = q.data;
    reads.push(a.path + "@" + a.from + "-" + a.to);
    const f = files.get(a.path.toLowerCase());
    if (!f) return m.port.postMessage({ id: a.id, kind: "none" });
    const b = f.slice(a.from, Math.min(a.to, f.length));
    m.port.postMessage({ id: a.id, kind: "bytes", size: f.length, bytes: b.buffer }, [b.buffer]);
  };
};\`;
const w = new Worker(URL.createObjectURL(new Blob([code], { type: "text/javascript" })));
const ask = (m) => new Promise((ok) => { w.onmessage = (e) => ok(e.data); w.postMessage(m); });
let ports = 0;
window.masuTest = { reads: () => ask({ reads: true }), get ports() { return ports; } };
window.loadFiles = (all) => ask({ load: all, base: location.href });
const origin = ${JSON.stringify(gameOrigin)};
const frame = document.createElement("iframe");
frame.allow = "cross-origin-isolated";
frame.sandbox.add("allow-scripts", "allow-same-origin");
addEventListener("message", (e) => {
  if (e.source !== frame.contentWindow || e.origin !== origin) return;
  const m = e.data;
  if (m?.masu === "want-port" && m.tree === "g") {
    ports++;
    const ch = new MessageChannel();
    w.postMessage({ port: ch.port1 }, [ch.port1]);
    frame.contentWindow.postMessage({ masu: "port", tree: "g" }, origin, [ch.port2]);
  } else if (m?.masu === "hello") {
    // (a port to the worker for the synchronous reads, as html.ts)
    const ch = new MessageChannel();
    w.postMessage({ port: ch.port1 }, [ch.port1]);
    frame.contentWindow.postMessage({ masu: "init", launch: "t", items: [], base: origin + "/tree/g/", names: ${JSON.stringify(names)} }, origin, [ch.port2]);
  }
});
window.startGame = () => {
  frame.src = origin + "/game-boot.html?to=" + encodeURIComponent("/tree/g/www/index.html");
  document.body.append(frame);
};
</script></body></html>`;
const app = createServer((req, res) => {
  const at = /^\/files\/(\d+)$/.exec(req.url);
  if (at) return res.writeHead(200, { "cross-origin-resource-policy": "same-origin" }).end(bytes[names[Number(at[1])]]);
  res
    .writeHead(200, {
      "content-type": "text/html; charset=utf-8",
      "cross-origin-opener-policy": "same-origin",
      "cross-origin-embedder-policy": "require-corp",
    })
    .end(appPage);
});
await new Promise((ok) => app.listen(0, "127.0.0.1", ok));
const appUrl = `http://127.0.0.1:${app.address().port}/`;

const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROME, chromiumSandbox: false });
try {
  const page = await browser.newPage();
  page.on("console", (m) => console.log("page:", m.text()));
  page.on("pageerror", (e) => console.log("page error:", e.message));
  await page.goto(appUrl);
  await page.evaluate((all) => window.loadFiles(all), names);
  const run = async () => {
    await page.evaluate(() => window.startGame());
    let f;
    for (let i = 0; i < 200 && !f; i++) {
      f = page.frames().find((x) => x.url().includes("/tree/g/www/index.html"));
      if (!f) await page.waitForTimeout(100);
    }
    assert.ok(f, "the game's page opened through the worker");
    await f.waitForFunction(() => document.title.startsWith("done "), null, { timeout: 60000 });
    return JSON.parse((await f.title()).slice(5));
  };
  const t0 = Date.now();
  const out = await run();
  console.log(out, `${Date.now() - t0} ms`);
  assert.equal(out.error, undefined);
  assert.equal(out.nodeBefore, "function", "frame.js ran first (the worker put it in)");
  assert.equal(out.fs, "from the game's folder", "a synchronous fs read through the worker");
  assert.equal(out.bytes, "512:true", "binary bytes through the synchronous fs");
  assert.equal(out.json, true, "a file with a non-ASCII name");
  assert.equal(out.range, "206:the :bytes 5-8/22");
  assert.equal(out.bad, 416);
  assert.equal(out.head, "200:22");
  assert.equal(out.missing, 404);
  assert.equal(out.movie, sha(big), "a 34 MiB file streamed whole");
  assert.equal(out.moviePart, `206:${sha(big.subarray(1000000, 30000001))}`, "and a range of it");
  assert.equal(out.coep, "require-corp");
  const reads = await page.evaluate(() => window.masuTest.reads());
  assert.ok(reads.some((r) => r.startsWith("www/movie.bin@")), "the big file was read from the page");
  // (piece by piece, none past the protocol's limit)
  assert.ok(reads.every((r) => { const [a, b] = r.split("@")[1].split("-").map(Number); return b - a <= 16 << 20; }));

  // the worker stopped: started again, it has no port, asks for one, and the game reads on
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("ServiceWorker.enable");
  await cdp.send("ServiceWorker.stopAllWorkers");
  const before = await page.evaluate(() => window.masuTest.ports);
  const f = page.frames().find((x) => x.url().includes("/tree/g/"));
  const again = await f.evaluate(async () => (await fetch("data.txt")).text());
  assert.equal(again, "from the game's folder", "a read after the worker was stopped");
  assert.ok((await page.evaluate(() => window.masuTest.ports)) > before, "the restarted worker asked for a port");
  // a reload of the game's page in the frame: controlled, its files read as before
  await f.evaluate(() => location.reload());
  let g;
  for (let i = 0; i < 100 && !g; i++) {
    await page.waitForTimeout(100);
    g = page.frames().find((x) => x.url().includes("/tree/g/www/index.html"));
    if (g && !(await g.title().catch(() => "")).startsWith("done ")) g = undefined;
  }
  assert.ok(g, "the game's page after a reload");
  assert.equal(JSON.parse((await g.title()).slice(5)).fs, "from the game's folder");

  assert.deepEqual(
    gameSeen.filter((p) => !GAME_FILES.includes(p)),
    [],
    "the game listener was asked for nothing but the frame's scripts",
  );
  console.log("web game service worker: passed");
} finally {
  await browser.close();
  game.close();
  app.close();
}
