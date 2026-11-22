import ts from "typescript";
import { expect, test } from "vitest";
import { check } from "./boundaries.mjs";

test("save metadata writes and unchecked runtime bytes stay with their callers", () => {
  const files = {
    "/src/saves.ts": `
      async function localWrite() {}
      function persist() { readSync(); return localWrite(); }
      async function loadSaves() { await localWrite(); }
      function storeSaves() { localWrite(); }
      const escaped = { localWrite };
      const legacyFiles = (name) => ({ bin: name });
      async function legacyRecord() { legacyFiles("game"); }
      function readSync() { return legacyFiles("game"); }
      async function acknowledge() { await readSync(); }
      async function upload() { await acknowledge(); }
      async function persistOrStale() { await localWrite(); await readSync(); }
      function keepSaving() { const settle = () => acknowledge(); return settle; }
      function storeSavesToo() { return acknowledge(); }
    `,
    "/src/wasmsource.ts": `
      export function sakeWasmBytes() {}
      function runtimeWasm() { sakeWasmBytes(); }
      const alias = sakeWasmBytes;
    `,
    "/src/play.ts": 'import { sakeWasmBytes as raw } from "./wasmsource"; raw();',
    "/src/wasmsource.test.ts": 'import { sakeWasmBytes } from "./wasmsource"; sakeWasmBytes();',
  };
  const options = { noLib: true, module: ts.ModuleKind.ESNext };
  const host = ts.createCompilerHost(options);
  host.fileExists = (f) => f in files;
  host.directoryExists = (dir) => dir === "/src";
  host.getSourceFile = (f) =>
    files[f] === undefined ? undefined : ts.createSourceFile(f, files[f], ts.ScriptTarget.Latest, true);
  const findings = check(ts.createProgram(Object.keys(files), options, host));
  // (storeSaves' and escaped's localWrite, persistOrStale's localWrite and readSync (the read in
  // a hold of its own, review b5dacf4 1.2), readSync's legacyFiles, storeSavesToo's acknowledge)
  expect(findings.filter((f) => f.startsWith("/src/saves.ts:"))).toHaveLength(6);
  expect(findings.filter((f) => f.includes("legacyFiles may only"))).toHaveLength(1);
  expect(findings.filter((f) => f.includes("readSync may only"))).toHaveLength(1);
  expect(findings.filter((f) => f.includes("acknowledge may only"))).toHaveLength(1);
  expect(findings.filter((f) => f.startsWith("/src/wasmsource.ts:"))).toHaveLength(1);
  expect(findings.some((f) => f.startsWith("/src/play.ts:"))).toBe(true);
  expect(findings.some((f) => f.includes(".test.ts:"))).toBe(false);
});

test("save states' index and bodies change only inside SnapshotStore's locked ways in", () => {
  const files = {
    "/src/snapshot-store.ts": `
      export async function writeBody() {}
      async function pruneBodies() {}
      async function writeIndex() { await writeBody(); await pruneBodies(); }
      export function publish() { return lock(async () => { await writeBody(); await writeIndex(); }); }
      export function cache() { return lock(async () => { await writeBody(); await writeIndex(); }); }
      export function sneak() { return writeIndex(); }
      function lock(f) { return f(); }
      async function dir(root) { return root.getDirectoryHandle("states"); }
    `,
    "/src/savestate.ts": `
      import { writeBody as w } from "./snapshot-store";
      export async function put(root) { await w(); await root.getDirectoryHandle("states", { create: true }); }
      export async function other(root) { await root.getDirectoryHandle("saves"); }
    `,
  };
  const options = { noLib: true, module: ts.ModuleKind.ESNext };
  const host = ts.createCompilerHost(options);
  host.fileExists = (f) => f in files;
  host.directoryExists = (dir) => dir === "/src";
  host.getSourceFile = (f) =>
    files[f] === undefined ? undefined : ts.createSourceFile(f, files[f], ts.ScriptTarget.Latest, true);
  const findings = check(ts.createProgram(Object.keys(files), options, host));
  // (sneak's writeIndex, savestate's alias of writeBody and its own states/ folder)
  expect(findings.filter((f) => f.startsWith("/src/snapshot-store.ts:"))).toEqual([
    expect.stringContaining("writeIndex may only be called by publish, cache"),
  ]);
  // (the import's two names, then the call through the alias)
  expect(findings.filter((f) => f.startsWith("/src/savestate.ts:"))).toEqual([
    "/src/savestate.ts:2: writeBody may only be called by publish, cache, writeIndex",
    "/src/savestate.ts:2: writeBody may only be called by publish, cache, writeIndex",
    "/src/savestate.ts:3: writeBody may only be called by publish, cache, writeIndex",
    "/src/savestate.ts:3: OPFS states/ belongs to snapshot-store.ts",
  ]);
});

test("the address's launch plan keys are launch-plan.ts's alone", () => {
  const files = {
    "/src/lib.d.ts": `declare class URLSearchParams { get(k: string): string | null; getAll(k: string): string[]; set(k: string, v: string): void; has(k: string): boolean }`,
    "/src/launch-plan.ts": `export function readPlan(p: URLSearchParams) { return [p.get("run"), p.getAll("arg")]; }`,
    "/src/play.ts": `
      const params = new URLSearchParams();
      params.get("args")?.split(",");
      params.getAll("mount");
      params.set("run", "x.exe");
      params.has("game");
      const m = new Map<string, string>(); m.get("run");
    `,
  };
  const options = { noLib: true, module: ts.ModuleKind.ESNext, strict: true };
  const host = ts.createCompilerHost(options);
  host.fileExists = (f) => f in files;
  host.directoryExists = (dir) => dir === "/src";
  host.getSourceFile = (f) =>
    files[f] === undefined ? undefined : ts.createSourceFile(f, files[f], ts.ScriptTarget.Latest, true);
  const findings = check(ts.createProgram(Object.keys(files), options, host)).filter((f) =>
    f.includes("launch plan"),
  );
  // (the Map's "run" and the address's "game" are not a plan's)
  expect(findings).toEqual([
    "/src/play.ts:3: the address's \"args\" is a launch plan's: launch-plan.ts",
    "/src/play.ts:4: the address's \"mount\" is a launch plan's: launch-plan.ts",
    "/src/play.ts:5: the address's \"run\" is a launch plan's: launch-plan.ts",
  ]);
});

test("the runtime's files are named in runtime.ts alone", () => {
  const files = {
    "/src/runtime.ts": "export const ids = () => `${base}build-id.txt`; const t = 'jit-version.txt';",
    "/src/offline.ts":
      "const a = `${base}build-id.txt`; const b = `x${1}snapz.mjs${2}`; const c = 'sake.wasm';",
    "/src/savestate.ts": 'fetch("sake/jit-version.txt");',
  };
  const options = { noLib: true, module: ts.ModuleKind.ESNext };
  const host = ts.createCompilerHost(options);
  host.fileExists = (f) => f in files;
  host.directoryExists = (dir) => dir === "/src";
  host.getSourceFile = (f) =>
    files[f] === undefined ? undefined : ts.createSourceFile(f, files[f], ts.ScriptTarget.Latest, true);
  const findings = check(ts.createProgram(Object.keys(files), options, host)).filter((f) =>
    f.includes("named by"),
  );
  expect(findings).toEqual([
    "/src/offline.ts:1: build-id.txt is named by runtime.ts only (its manifest)",
    "/src/offline.ts:1: snapz.mjs is named by runtime.ts only (its manifest)",
    "/src/savestate.ts:1: jit-version.txt is named by runtime.ts only (its manifest)",
  ]);
});

test("SaveTransport.load does not keep a revision", () => {
  const files = {
    "/src/save-transport.ts": `
      export class SaveTransport {
        private revisions = new Map<string, string>();
        async load(url: string) { const r = "x"; this.revisions.set(url, r); return r; }
        adopt(url: string, r: string) { this.revisions.set(url, r); }
      }
    `,
  };
  const options = { noLib: true, module: ts.ModuleKind.ESNext };
  const host = ts.createCompilerHost(options);
  host.fileExists = (f) => f in files;
  host.directoryExists = (dir) => dir === "/src";
  host.getSourceFile = (f) =>
    files[f] === undefined ? undefined : ts.createSourceFile(f, files[f], ts.ScriptTarget.Latest, true);
  const findings = check(ts.createProgram(Object.keys(files), options, host)).filter((f) =>
    f.includes("must not"),
  );
  expect(findings).toEqual(["/src/save-transport.ts:4: load must not touch this.revisions"]);
});

test("the page's key handler asks where each key goes", () => {
  const run = (code) => {
    const files = { "/src/play.ts": code };
    const options = { noLib: true, module: ts.ModuleKind.ESNext };
    const host = ts.createCompilerHost(options);
    host.fileExists = (f) => f in files;
    host.directoryExists = (dir) => dir === "/src";
    host.getSourceFile = (f) =>
      files[f] === undefined ? undefined : ts.createSourceFile(f, files[f], ts.ScriptTarget.Latest, true);
    return check(ts.createProgram(Object.keys(files), options, host)).filter((f) => f.includes("key"));
  };
  expect(run("function a() { const key = (k) => (e) => { push(k); }; }")).toEqual([
    "/src/play.ts: key must call inputTarget",
  ]);
  expect(
    run(
      "function a() { const key = (k) => (e) => { if (inputTarget(e.target) === 'overlay') return; push(k); }; }",
    ),
  ).toEqual([]);
  expect(run("const other = 1;")).toEqual(["play.ts: missing declaration key"]);
});

test("the decoders end with a Lifetime, not a flag of their own", () => {
  const files = {
    "/src/video.ts": "export class Video { private stopped = false; private life = 1; }",
    "/src/adec.ts": "export class AudioDecode { private life = 1; }",
  };
  const options = { noLib: true, module: ts.ModuleKind.ESNext };
  const host = ts.createCompilerHost(options);
  host.fileExists = (f) => f in files;
  host.directoryExists = (dir) => dir === "/src";
  host.getSourceFile = (f) =>
    files[f] === undefined ? undefined : ts.createSourceFile(f, files[f], ts.ScriptTarget.Latest, true);
  expect(
    check(ts.createProgram(Object.keys(files), options, host)).filter((f) => f.includes("field")),
  ).toEqual(['/src/video.ts:1: no "stopped" field: a Lifetime']);
});

test("an abort listener comes with a look at whether its signal was aborted already", () => {
  const files = {
    "/src/a.ts": [
      "declare const s: { addEventListener(t: string, f: () => void): void; aborted: boolean; throwIfAborted(): void };",
      "export function bad() { s.addEventListener('abort', () => {}); }",
      "export function good() { if (s.aborted) return; s.addEventListener('abort', () => {}); }",
      "export function thrown(o?: typeof s) { o?.throwIfAborted(); o?.addEventListener('abort', () => {}); }",
      "export function inner() { if (s.aborted) return; return () => s.addEventListener('abort', () => {}); }",
      "export function other() { s.addEventListener('message', () => {}); }",
    ].join("\n"),
  };
  const options = { noLib: true, module: ts.ModuleKind.ESNext };
  const host = ts.createCompilerHost(options);
  host.fileExists = (f) => f in files;
  host.directoryExists = (dir) => dir === "/src";
  host.getSourceFile = (f) =>
    files[f] === undefined ? undefined : ts.createSourceFile(f, files[f], ts.ScriptTarget.Latest, true);
  expect(
    check(ts.createProgram(Object.keys(files), options, host)).filter((f) => f.includes("abort")),
  ).toEqual([
    '/src/a.ts:2: an "abort" listener on s without a look at s.aborted first',
    '/src/a.ts:5: an "abort" listener on s without a look at s.aborted first',
  ]);
});

test("the page reloads only from play.ts, PageGame, or a click", () => {
  const files = {
    "/src/a.tsx": [
      "declare const location: { reload(): void };",
      "declare const window: { location: { reload(): void } };",
      "export function after() { location.reload(); }",
      "export const button = <button onClick={() => location.reload()} />;",
      "export function deep() { window.location.reload(); }",
      "export const other = <div title={String(location.reload)} />;",
    ].join("\n"),
    "/src/play.ts":
      "declare const location: { reload(): void }; export function leave() { location.reload(); }",
  };
  const options = { noLib: true, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.Preserve };
  const host = ts.createCompilerHost(options);
  host.fileExists = (f) => f in files;
  host.directoryExists = (dir) => dir === "/src";
  host.getSourceFile = (f) =>
    files[f] === undefined ? undefined : ts.createSourceFile(f, files[f], ts.ScriptTarget.Latest, true);
  expect(
    check(ts.createProgram(Object.keys(files), options, host)).filter((f) => f.includes("reloads")),
  ).toEqual([
    "/src/a.tsx:3: the page reloads by the player's click or pageGame.reload only",
    "/src/a.tsx:5: the page reloads by the player's click or pageGame.reload only",
    "/src/a.tsx:6: the page reloads by the player's click or pageGame.reload only",
  ]);
});

test("the status line is told apart by its phase, not a regex over its words", () => {
  const files = {
    "/src/ui/a.ts": [
      "declare const ui: { status: string; name: string };",
      "export const a = /^エラー|ません/.test(ui.status);",
      "export const b = ui.status.match(/x/);",
      "export const c = /x/.test(ui.name);",
      "export const d = ui.name.match(/x/);",
    ].join("\n"),
    "/src/b.ts": "declare const status: string; export const e = /x/.test(status);",
  };
  const options = { noLib: true, module: ts.ModuleKind.ESNext };
  const host = ts.createCompilerHost(options);
  host.fileExists = (f) => f in files;
  host.directoryExists = (dir) => dir === "/src" || dir === "/src/ui";
  host.getSourceFile = (f) =>
    files[f] === undefined ? undefined : ts.createSourceFile(f, files[f], ts.ScriptTarget.Latest, true);
  expect(
    check(ts.createProgram(Object.keys(files), options, host)).filter((f) => f.includes("phase")),
  ).toEqual([
    "/src/ui/a.ts:2: a status is told apart by its phase, not by a regex over its words",
    "/src/ui/a.ts:3: a status is told apart by its phase, not by a regex over its words",
  ]);
});

test("runners side by side go through jobs.ts", () => {
  const files = {
    "/src/a.ts": [
      "declare const Promise: { all(x: unknown): unknown };",
      "declare const Array: { from(...x: unknown[]): unknown[] };",
      "declare const one: () => void;",
      "export const a = Promise.all(Array.from({ length: 4 }, one));",
      "export const b = Promise.all(Array.from(new Set([1]).values()));",
      "export const c = Promise.all([one(), one()]);",
    ].join("\n"),
    "/src/jobs.ts": [
      "declare const Promise: { all(x: unknown): unknown };",
      "declare const Array: { from(...x: unknown[]): unknown[] };",
      "export const d = Promise.all(Array.from({ length: 4 }, () => 0));",
    ].join("\n"),
  };
  const options = { noLib: true, module: ts.ModuleKind.ESNext };
  const host = ts.createCompilerHost(options);
  host.fileExists = (f) => f in files;
  host.directoryExists = (dir) => dir === "/src";
  host.getSourceFile = (f) =>
    files[f] === undefined ? undefined : ts.createSourceFile(f, files[f], ts.ScriptTarget.Latest, true);
  expect(
    check(ts.createProgram(Object.keys(files), options, host)).filter((f) => f.includes("runners")),
  ).toEqual(["/src/a.ts:4: runners side by side are jobs.ts workers (all stopped before the end)"]);
});
