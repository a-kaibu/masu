// The app's page and the frame agree on which game pages run with Node, and those get what
// their synchronous reads need (review 22bb061 §4: an Electron page with Node got #masu-node but
// no folder, no names and no port).
import { expect, test } from "vite-plus/test";
import { frameTarget, frameWiring, webGamePage, type WebGameMode } from "./html";
import type { TreeReader } from "./treeread";
import { pageHasNode } from "./webstore";

const tree = (files: Record<string, string>) =>
  ({
    text: async (p: string) => files[p] ?? null,
    file: (p: string) => (p in files ? { path: p } : undefined),
    files: new Map(Object.keys(files).map((p) => [p, { path: p }])),
  }) as unknown as TreeReader;

const nwjs = { "g/package.json": '{"main":"index.html"}', "g/index.html": "" };
const electron = (main: string) => ({
  "g/resources/app/package.json": '{"main":"main.js"}',
  "g/resources/app/main.js": main,
  "g/resources/app/index.html": "",
  "g/version": "v11.0.0",
});
const withNode =
  "new BrowserWindow({ webPreferences: { nodeIntegration: true } }); win.loadFile('index.html')";
const isolated =
  "new BrowserWindow({ webPreferences: { contextIsolation: true } }); win.loadFile('index.html')";

test("each kind of web game: the frame's Node is the app's synchronous reads", async () => {
  const cases: [Record<string, string>, WebGameMode][] = [
    [nwjs, { runtime: "nwjs" }],
    [electron(withNode), { runtime: "electron", node: true }],
    [electron(isolated), { runtime: "electron", node: false }],
  ];
  for (const [files, mode] of cases) {
    const found = await webGamePage(tree(files), "g/");
    expect(found?.mode).toEqual(mode);
    const wiring = frameWiring(found!.mode);
    const at = new URL(frameTarget("t", found!.page, wiring), "https://game.test/");
    expect(pageHasNode(at.pathname, at.hash), JSON.stringify(mode)).toBe(wiring.syncReads);
  }
  expect(await webGamePage(tree({ "g/a.exe": "" }), "g/")).toBeNull();
});

test("every mode: synchronous reads exactly when the page has Node", () => {
  const modes: WebGameMode[] = [
    { runtime: "nwjs" },
    { runtime: "electron", node: true },
    { runtime: "electron", node: false },
  ];
  for (const mode of modes) {
    const page = mode.runtime === "nwjs" ? "g/index.html" : "g/resources/app/index.html";
    const w = frameWiring(mode);
    const at = new URL(frameTarget("t", page, w), "https://game.test/");
    expect(w.syncReads).toBe(mode.runtime === "nwjs" || mode.node);
    expect(pageHasNode(at.pathname, at.hash)).toBe(w.syncReads);
  }
});
