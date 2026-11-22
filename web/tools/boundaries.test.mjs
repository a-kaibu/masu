import ts from "typescript";
import { expect, test } from "vitest";
import { check } from "./boundaries.mjs";

test("save metadata writes and unchecked runtime bytes stay with their callers", () => {
  const files = {
    "/src/saves.ts": `
      async function localWrite() {}
      function persist() { return localWrite(); }
      async function loadSaves() { await localWrite(); }
      function storeSaves() { localWrite(); }
      const escaped = { localWrite };
      const legacyFiles = (name) => ({ bin: name });
      async function takeOver() { legacyFiles("game"); }
      function readSync() { return legacyFiles("game"); }
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
  expect(findings.filter((f) => f.startsWith("/src/saves.ts:"))).toHaveLength(3);
  expect(findings.filter((f) => f.includes("legacyFiles may only"))).toHaveLength(1);
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
