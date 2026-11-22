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
