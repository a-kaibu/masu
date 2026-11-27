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

/** A launch plan's keys in the page's address, read and written by launch-plan.ts only (review
 * 22bb061 §4: the arguments joined with commas and split again did not come back). */
const planKeys = new Set(["run", "arg", "args", "cmdline", "mount", "env"]);
const paramMethods = new Set(["get", "getAll", "has", "set", "append", "delete"]);

/** The runtime's files by name, in runtime.ts alone (its RuntimeManifest: what a start reads is
 * what 端末に保存 saves; review 22bb061 §4, build-id.txt left out of the save). */
/** @type {[string, string][]} */
const names = [
  ["build-id.txt", "runtime.ts"],
  ["jit-version.txt", "runtime.ts"],
  ["snapz.mjs", "runtime.ts"],
];

/** Declarations whose code must call a function: [file, declaration, callee]. */
/** @type {[string, string, string][]} */
const mustCall = [
  // (the keys the page gives the game: never one typed in its own UI; review 22bb061 §5)
  ["play.ts", "key", "inputTarget"],
];

/** Fields a module's classes may not declare: [file, field, why]. */
/** @type {[string, string, string][]} */
const noFields = [
  // (a decoder's process ended: a Lifetime (session.ts), checked after each await; review 22bb061 §5)
  ["video.ts", "stopped", "a Lifetime"],
  ["idec.ts", "stopped", "a Lifetime"],
  ["adec.ts", "stopped", "a Lifetime"],
];

/** Methods that must not touch a field of their class: [file, method, field]. */
/** @type {[string, string, string][]} */
const untouched = [
  // (a GET's revision is the base of the next store only once the caller took its bytes:
  // SaveTransport.adopt; review 22bb061 §3, a GET's bytes left aside and its ETag kept)
  ["save-transport.ts", "load", "revisions"],
];

/** OPFS folders each owned by one module: getDirectoryHandle("<folder>") elsewhere is an error. */
/** @type {[string, string][]} */
const folders = [["states", "snapshot-store.ts"]];

/** Files that may reload the page themselves: the run's owner (leaving a game reloads), and
 * PageGame, which reloads only with no game. Anywhere else a reload is the player's click (an
 * `on…` handler of the JSX) or PageGame.reload (review 22bb061 §6: the server's update reloaded
 * under a game started meanwhile). */
const reloaders = ["play.ts", "session.ts"];

function reloads(source, errors) {
  if (reloaders.some((f) => source.fileName.endsWith("/src/" + f))) return;
  const clicked = (n) => {
    for (let p = n.parent; p; p = p.parent)
      if (ts.isJsxAttribute(p) && /^on[A-Z]/.test(p.name.getText(source))) return true;
    return false;
  };
  const look = (n) => {
    if (
      ts.isPropertyAccessExpression(n) &&
      n.name.text === "reload" &&
      /(^|\.)location$/.test(n.expression.getText(source)) &&
      !clicked(n)
    ) {
      const line = source.getLineAndCharacterOfPosition(n.getStart()).line + 1;
      errors.push(
        `${source.fileName}:${line}: the page reloads by the player's click or pageGame.reload only`,
      );
    }
    ts.forEachChild(n, look);
  };
  look(source);
}

/** The view picks the status line's look from its phase, never from its words (review 22bb061
 * §6: a spinner guessed by /^エラー|ません/): a regex run over a status in src/ui is an error. */
function statusGuess(source, errors) {
  if (!source.fileName.includes("/src/ui/")) return;
  const look = (n) => {
    if (
      ts.isCallExpression(n) &&
      ts.isPropertyAccessExpression(n.expression) &&
      ["test", "exec", "match", "search"].includes(n.expression.name.text)
    ) {
      const regex = ts.isRegularExpressionLiteral(n.expression.expression);
      const subject = regex ? n.arguments[0] : n.expression.expression;
      const pattern = regex || (n.arguments[0] && ts.isRegularExpressionLiteral(n.arguments[0]));
      if (pattern && subject && /status/i.test(subject.getText(source))) {
        const line = source.getLineAndCharacterOfPosition(n.getStart()).line + 1;
        errors.push(
          `${source.fileName}:${line}: a status is told apart by its phase, not by a regex over its words`,
        );
      }
    }
    ts.forEachChild(n, look);
  };
  look(source);
}

/** Runners side by side are jobs.ts's workers/pool: `Promise.all(Array.from(...))` of them says
 * the job's end at the first failure while the others still run (review 22bb061 §6). */
function runnerPools(source, errors) {
  if (source.fileName.endsWith("/src/jobs.ts")) return;
  const look = (n) => {
    if (
      ts.isCallExpression(n) &&
      ts.isPropertyAccessExpression(n.expression) &&
      n.expression.expression.getText(source) === "Promise" &&
      ["all", "race", "any"].includes(n.expression.name.text) &&
      n.arguments[0] &&
      ts.isCallExpression(n.arguments[0]) &&
      n.arguments[0].expression.getText(source) === "Array.from" &&
      n.arguments[0].arguments.length === 2
    ) {
      const line = source.getLineAndCharacterOfPosition(n.getStart()).line + 1;
      errors.push(
        `${source.fileName}:${line}: runners side by side are jobs.ts workers (all stopped before the end)`,
      );
    }
    ts.forEachChild(n, look);
  };
  look(source);
}

/** A signal's "abort" listener never runs when it was aborted before: a function that listens
 * must also look at `.aborted` (or throwIfAborted) of the same signal (review 22bb061 §6, a
 * fetch sent for an ask already cancelled). */
function abortListeners(source, errors) {
  const text = (e) => e.getText(source).replace(/\?$/, "");
  const look = (node) => {
    if (ts.isFunctionLike(node)) {
      const listens = [];
      const checks = new Set();
      const walk = (n) => {
        if (n !== node && ts.isFunctionLike(n)) return look(n);
        if (
          ts.isCallExpression(n) &&
          ts.isPropertyAccessExpression(n.expression) &&
          n.expression.name.text === "addEventListener" &&
          n.arguments[0] &&
          ts.isStringLiteralLike(n.arguments[0]) &&
          n.arguments[0].text === "abort"
        )
          listens.push(n);
        if (
          ts.isPropertyAccessExpression(n) &&
          (n.name.text === "aborted" || n.name.text === "throwIfAborted")
        )
          checks.add(text(n.expression));
        ts.forEachChild(n, walk);
      };
      ts.forEachChild(node, walk);
      for (const n of listens) {
        const signal = text(n.expression.expression);
        if (!checks.has(signal)) {
          const line = source.getLineAndCharacterOfPosition(n.getStart()).line + 1;
          errors.push(
            `${source.fileName}:${line}: an "abort" listener on ${signal} without a look at ${signal}.aborted first`,
          );
        }
      }
      return;
    }
    ts.forEachChild(node, look);
  };
  look(source);
}

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
  for (const [file, name, callee] of mustCall) {
    const source = program.getSourceFiles().find((f) => f.fileName.endsWith("/src/" + file));
    if (!source) continue;
    let found = false;
    let calls = false;
    const look = (node, inside) => {
      if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === name) {
        found = true;
        inside = true;
      }
      if (
        inside &&
        ts.isCallExpression(node) &&
        ts.isIdentifier(node.expression) &&
        node.expression.text === callee
      )
        calls = true;
      ts.forEachChild(node, (c) => look(c, inside));
    };
    look(source, false);
    if (!found) errors.push(`${file}: missing declaration ${name}`);
    else if (!calls) errors.push(`${source.fileName}: ${name} must call ${callee}`);
  }
  for (const source of program.getSourceFiles()) {
    if (
      source.isDeclarationFile ||
      source.fileName.includes("/node_modules/") ||
      /\.test\.tsx?$/.test(source.fileName)
    )
      continue;
    function visit(node, owner, method) {
      if (ts.isFunctionDeclaration(node)) owner = node;
      if (ts.isMethodDeclaration(node) && ts.isIdentifier(node.name)) method = node.name.text;
      if (ts.isPropertyDeclaration(node) && ts.isIdentifier(node.name))
        for (const [file, field, why] of noFields)
          if (source.fileName.endsWith("/src/" + file) && node.name.text === field) {
            const line = source.getLineAndCharacterOfPosition(node.getStart()).line + 1;
            errors.push(`${source.fileName}:${line}: no "${field}" field: ${why}`);
          }
      if (method && ts.isPropertyAccessExpression(node) && node.expression.kind === ts.SyntaxKind.ThisKeyword)
        for (const [file, m, field] of untouched)
          if (source.fileName.endsWith("/src/" + file) && m === method && node.name.text === field) {
            const line = source.getLineAndCharacterOfPosition(node.getStart()).line + 1;
            errors.push(`${source.fileName}:${line}: ${m} must not touch this.${field}`);
          }
      if (
        ts.isCallExpression(node) &&
        ts.isPropertyAccessExpression(node.expression) &&
        paramMethods.has(node.expression.name.text) &&
        node.arguments[0] &&
        ts.isStringLiteralLike(node.arguments[0]) &&
        planKeys.has(node.arguments[0].text) &&
        !source.fileName.endsWith("/src/launch-plan.ts") &&
        checker.typeToString(checker.getTypeAtLocation(node.expression.expression)) === "URLSearchParams"
      ) {
        const line = source.getLineAndCharacterOfPosition(node.getStart()).line + 1;
        errors.push(
          `${source.fileName}:${line}: the address's "${node.arguments[0].text}" is a launch plan's: launch-plan.ts`,
        );
      }
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
      if (
        ts.isStringLiteralLike(node) ||
        ts.isTemplateHead(node) ||
        ts.isTemplateMiddle(node) ||
        ts.isTemplateTail(node)
      ) {
        for (const [name, owner] of names)
          if (node.text.includes(name) && !source.fileName.endsWith("/src/" + owner)) {
            const line = source.getLineAndCharacterOfPosition(node.getStart()).line + 1;
            errors.push(`${source.fileName}:${line}: ${name} is named by ${owner} only (its manifest)`);
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
      ts.forEachChild(node, (child) => visit(child, owner, method));
    }
    visit(source);
    abortListeners(source, errors);
    reloads(source, errors);
    statusGuess(source, errors);
    runnerPools(source, errors);
  }
  return errors;
}

/**
 * host/net-core.mjs is the one source of what the page and the network worker do the same way
 * (review 22bb061 §7): fetch-bridge.js declares none of its names again (a second copy that
 * drifts), and the worker's source (masuFetchWorkerSource) takes each of its functions (a
 * worker missing one fails only there, at its first call).
 */
export function checkNetCore(core, bridge) {
  const errors = [];
  const names = [...core.matchAll(/^export (?:var|function|async function) (\w+)/gm)].map((m) => m[1]);
  const fns = [...core.matchAll(/^export (?:async )?function (\w+)/gm)].map((m) => m[1]);
  for (const name of names)
    if (new RegExp(`^(?:var|let|const|function|async function) ${name}\\b`, "m").test(bridge))
      errors.push(`host/fetch-bridge.js: ${name} is host/net-core.mjs's (one source)`);
  const list = /const fns = \[([^\]]*)\]/.exec(bridge)?.[1];
  if (list === undefined) errors.push("host/fetch-bridge.js: masuFetchWorkerSource's fns list not found");
  else {
    const given = new Set(list.split(",").map((x) => x.trim()));
    for (const f of fns)
      if (!given.has(f))
        errors.push(`host/fetch-bridge.js: the network worker's source lacks ${f} (masuFetchWorkerSource)`);
  }
  return errors;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const config = ts.readConfigFile("tsconfig.json", (path) => ts.sys.readFile(path));
  if (config.error) throw new Error(ts.flattenDiagnosticMessageText(config.error.messageText, "\n"));
  const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, process.cwd());
  const errors = check(ts.createProgram(parsed.fileNames, parsed.options));
  const host = (f) => ts.sys.readFile(new URL(`../../host/${f}`, import.meta.url).pathname) ?? "";
  errors.push(...checkNetCore(host("net-core.mjs"), host("fetch-bridge.js")));
  for (const message of errors) console.error(message);
  if (errors.length) process.exitCode = 1;
}
