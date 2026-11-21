// Restrict references as well as calls. Taking an alias would otherwise bypass the caller check.
import ts from "typescript";
import { pathToFileURL } from "node:url";

/** @type {[string, string, string[]][]} */
const rules = [
  // (this browser's copy is written by a held local lock's write only)
  ["saves.ts", "localWrite", ["persist", "loadSaves"]],
  // (the older format's files: taken over once, never written as this format's; review e18bf44)
  ["saves.ts", "legacyFiles", ["takeOver"]],
  ["wasmsource.ts", "sakeWasmBytes", ["runtimeWasm"]],
];

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
