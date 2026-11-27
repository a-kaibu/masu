// Run from the repository root after building web/dist and masu.
import { once } from "node:events";
import { execFileSync, spawn } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { setTimeout } from "node:timers/promises";

const bin = resolve(process.argv[2] ?? "target/debug/masu");
const trace = resolve(process.env.BOMBADIL_TRACE ?? "target/bombadil/trace.bin");
const work = mkdtempSync(join(tmpdir(), "masu-bombadil-"));
const port = Number(process.env.BOMBADIL_PORT ?? 9600);
const origin = `http://127.0.0.1:${port}`;
const store = join(work, "store");
const env = {
  ...process.env,
  MASU_RTP: "0",
  XDG_DATA_HOME: join(work, "data"),
  XDG_CONFIG_HOME: join(work, "config"),
  XDG_CACHE_HOME: join(work, "cache"),
  XDG_STATE_HOME: join(work, "state"),
};
const run = (...args) => execFileSync(bin, ["--store", store, ...args], { stdio: "inherit", env });
let server;
let chrome;

try {
  const web = resolve(process.env.BOMBADIL_WEB ?? "web/dist");
  if (!existsSync(join(web, "sake/sake.wasm"))) throw new Error("Build the complete web runtime first");

  const prefix = join(work, "prefix");
  const temp = join(prefix, "drive_c/windows/temp");
  mkdirSync(temp, { recursive: true });
  writeFileSync(join(temp, "empty.txt"), "");
  run("pack", prefix, "prefix32");

  const exe = join(work, "fixture.exe");
  execFileSync("i686-w64-mingw32-gcc", [
    "-O2", "host/tests/autoadvance.c", "-mwindows", "-o", exe,
  ]);
  for (const id of ["sakura", "tsuki"]) {
    const dir = join(work, id);
    mkdirSync(dir);
    copyFileSync(exe, join(dir, "game.exe"));
    writeFileSync(join(dir, "game.dat"), Buffer.alloc(200 << 10, id));
    run("pack", dir, id, "--chunk-kib", "64");
    run("add-game", id, "--title", id, "--tree", id, "--exe", "game.exe", "--no-meta");
  }

  server = spawn(
    bin,
    ["--store", store, "serve", "--foreground", "--web", web, "--jit", join(work, "jit"),
      "--listen", `127.0.0.1:${port}`, "--no-prerun", "--dev-bucket", join(work, "bucket")],
    { stdio: "inherit", env },
  );
  let ready = false;
  for (let i = 0; i < 100 && !ready && server.exitCode === null; i++) {
    ready = await fetch(`${origin}/play.html`).then((response) => response.ok, () => false);
    if (!ready) await setTimeout(300);
  }
  if (!ready) throw new Error("masu serve did not start");

  const mode = ["test", "--headless"];
  if (process.env.BOMBADIL_CHROME) {
    const profile = join(work, "chrome");
    const debug = port + 2_000;
    chrome = spawn(process.env.BOMBADIL_CHROME, [
      "--headless", "--disable-dev-shm-usage", "--enable-unsafe-webgpu", `--user-data-dir=${profile}`,
      `--remote-debugging-port=${debug}`, "about:blank",
    ], { stdio: "ignore" });
    const address = `http://127.0.0.1:${debug}`;
    let chromeReady = false;
    for (let i = 0; i < 100 && !chromeReady && chrome.exitCode === null; i++) {
      chromeReady = await fetch(`${address}/json/version`).then((res) => res.ok, () => false);
      if (!chromeReady) await setTimeout(100);
    }
    if (!chromeReady) throw new Error("Chromium did not start");
    mode.splice(0, mode.length, "test-external", "--remote-debugger", address, "--create-target");
  }

  mkdirSync(dirname(trace), { recursive: true });
  const runOptions = process.env.BOMBADIL_REPRODUCE
    ? ["--reproduce", resolve(process.env.BOMBADIL_REPRODUCE)]
    : ["--exit-on-violation", "--time-limit", process.env.BOMBADIL_LIMIT ?? "3m"];
  const experiment = spawn(
    process.execPath,
    [resolve("web/node_modules/@antithesishq/bombadil/bin/bombadil.js"), "browser", ...mode,
      ...runOptions, "--instrument-javascript", "", "--output-path", trace, `${origin}/play.html?memory=grow`, resolve("web/bombadil/spec.ts")],
    { stdio: "inherit" },
  );
  const [code] = await once(experiment, "exit");
  process.exitCode = code ?? 1;
} finally {
  for (const child of [chrome, server]) {
    if (child && child.exitCode === null && child.signalCode === null) {
      const done = once(child, "exit");
      child.kill();
      await done;
    }
  }
  rmSync(work, { recursive: true, force: true });
}
