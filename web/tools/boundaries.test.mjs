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
