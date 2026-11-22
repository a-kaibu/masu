// Restrict references as well as calls. Taking an alias would otherwise bypass the caller check.
import ts from "typescript";
import { pathToFileURL } from "node:url";

/** @type {[string, string, string[]][]} */
const rules = [
  // (this browser's copy is written by a held local lock's write only)
  ["saves.ts", "localWrite", ["persist", "loadSaves", "readCurrent"]],
  // (the older format's files: read to take them over, marked sent, never written as this
  // format's; review e18bf44, b5dacf4 1.4)
  ["saves.ts", "legacyFiles", ["legacyRecord", "legacyBlob", "legacySent"]],
  // (a record is read only where the decision it feeds is made, in the same hold: a failed
  // write's record read again in a later hold was another tab's newer one; review b5dacf4 1.2)
  ["saves.ts", "readSync", ["persist", "acknowledge", "readCurrent"]],
  // (a record is settled by the upload that sent its bytes only)
  ["saves.ts", "acknowledge", ["upload", "keepSaving"]],
  ["wasmsource.ts", "sakeWasmBytes", ["runtimeWasm"]],
  // (a save state's index and bodies change under the game's lock only: SnapshotStore's two
  // ways in; review 22bb061 §2, two tabs' saves removing each other's bodies)
  ["snapshot-store.ts", "writeBody", ["publish", "cache", "writeIndex"]],
  ["snapshot-store.ts", "writeIndex", ["publish", "cache"]],
  ["snapshot-store.ts", "pruneBodies", ["writeIndex"]],
];

/** OPFS folders each owned by one module: getDirectoryHandle("<folder>") elsewhere is an error. */
/** @type {[string, string][]} */
const folders = [["states", "snapshot-store.ts"]];

export function check(program) {
  const checker = program.getTypeChecker();
  const allowed = new Map();
  const errors = [];
  for (const [file, name, callers] of rules) {
    const source = program.getSourceFiles().find((f) => f.fileName.endsWith("/src/" + file));
    const declaration = source?.statements
      .flatMap((s) => (ts.isVariableStatement(s) ? [...s.declarationList.declarations] : [s]))
      .find((d) => d.name && ts.isIdentifier(d.name) && d.name.text === name);
    if (!declaration?.name) {
      errors.push(file + ": missing guarded declaration " + name);
      continue;
    }
    allowed.set(checker.getSymbolAtLocation(declaration.name), { source, name, callers, declaration });
  }
  for (const source of program.getSourceFiles()) {
    if (
      source.isDeclarationFile ||
      source.fileName.includes("/node_modules/") ||
      /\.test\.tsx?$/.test(source.fileName)
    )
      continue;
    function visit(node, owner) {
      if (ts.isFunctionDeclaration(node)) owner = node;
      if (
        ts.isCallExpression(node) &&
        ts.isPropertyAccessExpression(node.expression) &&
        node.expression.name.text === "getDirectoryHandle" &&
        node.arguments[0] &&
        ts.isStringLiteralLike(node.arguments[0])
      ) {
        const folder = folders.find(([f]) => f === node.arguments[0].text);
        if (folder && !source.fileName.endsWith("/src/" + folder[1])) {
          const line = source.getLineAndCharacterOfPosition(node.getStart()).line + 1;
          errors.push(`${source.fileName}:${line}: OPFS ${folder[0]}/ belongs to ${folder[1]}`);
        }
      }
      if (ts.isIdentifier(node)) {
        let symbol = ts.isShorthandPropertyAssignment(node.parent)
          ? checker.getShorthandAssignmentValueSymbol(node.parent)
          : checker.getSymbolAtLocation(node);
        if (symbol && symbol.flags & ts.SymbolFlags.Alias) symbol = checker.getAliasedSymbol(symbol);
        const rule = allowed.get(symbol);
        if (rule && node !== rule.declaration.name) {
          const direct = ts.isCallExpression(node.parent) && node.parent.expression === node;
          if (
            !(
              direct &&
              source === rule.source &&
              owner?.parent === source &&
              rule.callers.includes(owner.name?.text)
            )
          ) {
            const line = source.getLineAndCharacterOfPosition(node.getStart()).line + 1;
            errors.push(
              source.fileName +
                ":" +
                line +
                ": " +
                rule.name +
                " may only be called by " +
                rule.callers.join(", "),
            );
          }
        }
      }
      ts.forEachChild(node, (child) => visit(child, owner));
    }
    visit(source);
  }
  return errors;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const config = ts.readConfigFile("tsconfig.json", (path) => ts.sys.readFile(path));
  if (config.error) throw new Error(ts.flattenDiagnosticMessageText(config.error.messageText, "\n"));
  const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, process.cwd());
  const errors = check(ts.createProgram(parsed.fileNames, parsed.options));
  for (const message of errors) console.error(message);
  if (errors.length) process.exitCode = 1;
}
