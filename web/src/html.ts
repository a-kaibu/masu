// Games that are web pages already (NW.js or Electron: TyranoScript, RPG Maker MV/MZ): when the
// program's folder (or its resources/app) has a package.json (masu pack unpacks one appended to
// the exe: crates/masu/src/nwjs.rs), the page opens the game's own start page in an iframe
// instead of running the exe. The frame is on another origin (masu serve's game listener, see
// `GET /game-origin`) and sandboxed, so the game's scripts cannot reach this page, the saves of
// other games or the server's other routes: its pages start with frame.ts (frame.js), which
// asks this page for the game's saves and sends back what the game writes, and nothing else
// (frame-protocol.ts). The game's files come from the frame origin's service worker
// (game-sw.ts), which asks this page for them (gameport.ts): this page reads them from the device
// or straight from the bucket (treeread.ts), not through the server.
import { electronNode, electronPages } from "./electron";
import { fromFrame, type Init } from "./frame-protocol";
import type { GivePort } from "./gameport";
import type { TreeWorkerAsk } from "./tree-worker";
import type { TreeReader } from "./treeread";
import { openTree } from "./treesource";
import { loadSaves, storeSaves } from "./saves";
import { SAVE_SOON_MS, SaveItems } from "./webstore";

/** The program's tree and folder in it, from &run and the &mount that holds it. */
function where(run: string, mounts: string[]): { tree: string; dir: string } | null {
  const host = run.replace(/^z:/i, "").replaceAll("\\", "/");
  for (const m of mounts) {
    const at = m.indexOf("=");
    const [prefix, tree] = [m.slice(0, at).replace(/\/$/, ""), m.slice(at + 1)];
    if (!host.toLowerCase().startsWith(`${prefix.toLowerCase()}/`)) continue;
    const rel = host.slice(prefix.length + 1);
    return { tree, dir: rel.includes("/") ? rel.slice(0, rel.lastIndexOf("/") + 1) : "" };
  }
  return null;
}

const enc = (path: string) => path.split("/").map(encodeURIComponent).join("/");

/** Where the server's game listener is (crates/masu/src/serve.rs). */
async function gameOrigin(): Promise<string> {
  const r = await fetch(new URL("game-origin", location.href));
  if (!r.ok) throw new Error(`game-origin: HTTP ${r.status}`);
  const { port, url } = (await r.json()) as { port: number; url: string | null };
  return new URL(url ?? `${location.protocol}//${location.hostname}:${port}`).origin;
}

/** The start page's path in the tree (package.json's main, beside it in `dir`), or null when
 * the program is no web game. */
async function startPage(tree: TreeReader, dir: string): Promise<string | null> {
  const pkg = await tree.text(`${dir}package.json`);
  if (pkg === null) return null;
  let main = "";
  try {
    const m = (JSON.parse(pkg) as { main?: unknown }).main;
    if (typeof m === "string") main = m.replace(/^app:\/\/[^/]*\//, "").replace(/^\.\//, "");
  } catch (e) {
    console.warn("package.json:", e);
  }
  // (Electron: the main script names the page it opens, SKYNovel's doc/app/index.htm, #234)
  const named: string[] = [];
  if (/\.[cm]?js$/i.test(main)) {
    const code = (await tree.text(dir + main)) ?? "";
    named.push(...electronPages(main, code));
  }
  for (const page of [main, ...named, "www/index.html", "index.html"]) {
    if (!/\.html?$/i.test(page)) continue;
    const f = tree.file(dir + page);
    if (f) return f.path;
  }
  return null;
}

/** Whether the Electron game in `dir` (with resources/app/) runs its page with Node. */
async function electronPageNode(tree: TreeReader, dir: string): Promise<boolean> {
  const text = (path: string) => tree.text(path);
  const app = `${dir}resources/app/`;
  let main = "index.js";
  try {
    const m = (JSON.parse((await text(`${app}package.json`)) ?? "{}") as { main?: unknown }).main;
    if (typeof m === "string" && m) main = m.replace(/^\.\//, "");
  } catch (e) {
    console.warn("package.json:", e);
  }
  if (!/\.[cm]?js$/i.test(main)) main += ".js";
  const code = await text(app + main);
  return code !== null && electronNode(code, await text(`${dir}version`));
}

/**
 * Opens the program as a web game when it is one; false when it is not (sake runs it).
 * `saveName` names its saves (as sake's), `oldSaves` their name before the rename (Z:\vnvm).
 */
export async function startWebGame(
  run: string,
  mounts: string[],
  saveName: string | null,
  log: (line: string) => void,
  oldSaves: string | null = null,
): Promise<boolean> {
  const at = where(run, mounts);
  if (!at) return false;
  const tree = await openTree(at.tree, location.href);
  // (the frame's reads answered by a worker: the frame's synchronous reads (sync-read.ts) hold
  // the main thread, which this page shares with it)
  const reads = new Worker(new URL("./tree-worker.ts", import.meta.url), { type: "module" });
  reads.postMessage({ tree: at.tree, page: location.href } satisfies TreeWorkerAsk);
  // NW.js: package.json beside the exe; Electron: resources/app (its pages run without Node:
  // contextIsolation, and a global `process` stops a PixiJS game's title buttons)
  let page = await startPage(tree, at.dir);
  const node = !!page;
  page ??= await startPage(tree, `${at.dir}resources/app/`);
  if (!page) return false;
  // (older Electron games, as TyranoScript v5's, have Node in the page: saves through fs)
  const electronWithNode = !node && (await electronPageNode(tree, at.dir));
  log("Web ゲームとして開いています");
  const origin = await gameOrigin();

  // saves: the page's localStorage (and fs writes), as one JSON blob
  const saves = new SaveItems();
  const items = saves.map;
  // (none yet: those of before the rename, Z:\vnvm, if any; the next save goes under the new name)
  let saved = saveName ? await loadSaves(`${saveName}-web`) : null;
  if (saveName && !saved?.length && oldSaves) saved = await loadSaves(`${oldSaves}-web`);
  if (saved?.length) {
    try {
      for (const [k, v] of Object.entries(
        JSON.parse(new TextDecoder().decode(saved)) as Record<string, string>,
      ))
        if (typeof v === "string") saves.set(k, v);
    } catch (e) {
      console.warn("saves:", e);
    }
  }
  let generation = 0;
  let savedGeneration = 0;
  let pending: Promise<void> | undefined;
  const save = (): Promise<void> => {
    if (pending) return pending.then(save);
    if (generation === savedGeneration || !saveName) return Promise.resolve();
    const current = generation;
    pending = storeSaves(
      `${saveName}-web`,
      new TextEncoder().encode(JSON.stringify(Object.fromEntries(items))),
    )
      .then(() => {
        savedGeneration = current;
      })
      .catch((e: unknown) => {
        log(`セーブを保存できません: ${String(e)}`);
      })
      .finally(() => {
        pending = undefined;
      });
    return pending;
  };
  setInterval(() => void save(), 3000);
  // (soon after a change too: a phone's page put away may be frozen before the next interval)
  let soon: ReturnType<typeof setTimeout> | undefined;
  let refused = false;
  addEventListener("visibilitychange", () => document.visibilityState === "hidden" && void save());
  addEventListener("pagehide", () => void save());

  // NW.js games read their files synchronously (Node's fs): the frame reads them from the
  // folder when asked (frame.ts, sync-read.ts); it gets the folder's URL and file names
  const base = node ? `${origin}/tree/${encodeURIComponent(at.tree)}/${enc(at.dir)}` : "";
  const names: string[] = [];
  if (node)
    for (const { path } of tree.files.values())
      if (path.startsWith(at.dir)) names.push(path.slice(at.dir.length));

  const frame = document.createElement("iframe");
  frame.id = "frame";
  // (cross-origin-isolated: its synchronous reads wait on a SharedArrayBuffer, sync-read.ts)
  frame.allow = "autoplay; fullscreen; cross-origin-isolated";
  // (another origin already: the sandbox keeps it from navigating this page and the like)
  frame.sandbox.add(
    "allow-scripts",
    "allow-same-origin",
    "allow-forms",
    "allow-pointer-lock",
    "allow-popups",
    "allow-modals",
  );
  // (its first page registers the worker that answers the game's files, then opens the game's)
  const to = `/tree/${encodeURIComponent(at.tree)}/${enc(page)}${electronWithNode ? "#masu-node" : ""}`;
  frame.src = `${origin}/game-boot.html?to=${encodeURIComponent(to)}`;
  addEventListener("message", (e: MessageEvent) => {
    if (e.source !== frame.contentWindow || e.origin !== origin) return;
    const m = fromFrame(e.data);
    if (m?.masu === "want-port") {
      // (only this game's tree)
      if (m.tree !== at.tree) return;
      const ch = new MessageChannel();
      reads.postMessage({ port: ch.port1 } satisfies TreeWorkerAsk, [ch.port1]);
      frame.contentWindow!.postMessage({ masu: "port", tree: at.tree } satisfies GivePort, origin, [
        ch.port2,
      ]);
    } else if (m?.masu === "hello") {
      // (each page of the game asks: a reload gets the saves as they are now)
      const init: Init = { masu: "init", items: [...items], base, names };
      // (NW.js: a port to the tree's worker for the synchronous reads, sync-read.ts)
      const ch = new MessageChannel();
      if (node) reads.postMessage({ port: ch.port1 } satisfies TreeWorkerAsk, [ch.port1]);
      frame.contentWindow!.postMessage(init, origin, node ? [ch.port2] : []);
    } else if (m?.masu === "set") {
      if (!saves.set(m.key, m.value)) {
        if (!refused) log("ゲームのセーブが大きすぎるため、これ以上は保存しません");
        refused = true;
        return;
      }
      generation++;
      soon ??= setTimeout(() => {
        soon = undefined;
        void save();
      }, SAVE_SOON_MS);
    }
  });
  document.querySelector<HTMLCanvasElement>("#screen")!.hidden = true;
  document.body.append(frame);
  frame.addEventListener("load", () => {
    log("");
    frame.focus();
    dispatchEvent(new Event("vnvm-frame"));
  });
  return true;
}
