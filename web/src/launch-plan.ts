// What one start of a program runs (review 22bb061 §4): the program, its arguments, the trees
// mounted and the environment. The page's address holds it (play.html?run=...), the game list
// fills it from a GameEntry (launcher.ts applyGame), a program starting the next in its place
// (sake's k32::exec) gives its command line; play.ts reads it once, here, and hands sake the
// arguments as a list it quotes (crates/sake/src/k32.rs set_command_line).
//
// The address's keys:
//   &run=Z:\path\to.exe   the program
//   &arg=x                one argument (repeated: each one as it is, commas, spaces, "" too)
//   &cmdline=...          or the text after the program, as a program wrote it (exec)
//   &mount=/path=tree     a store tree (repeated)
//   &env=NAME=value       an environment variable (repeated)
// Only this module reads or writes them (lint: web/tools/boundaries.mjs).

/** A program's arguments: a list (each one argument), or a command line's text after the
 * program as a program wrote it (its quotes as they were: sake keeps the text). */
export type LaunchArgs =
  | { readonly kind: "list"; readonly list: readonly string[] }
  | { readonly kind: "line"; readonly line: string };

export interface LaunchPlan {
  readonly run: string;
  readonly args: LaunchArgs;
  readonly mounts: readonly string[];
  readonly env: readonly string[];
}

export const DEFAULT_RUN = "C:\\wintest.exe";
const RUN = "run",
  ARG = "arg",
  LINE = "cmdline",
  MOUNT = "mount",
  ENV = "env";
/** the comma-joined arguments of before (review 22bb061 §4: they did not come back) */
const OLD_ARGS = "args";
/** every key of a plan in the address (a page that keeps its own parameters apart: bench.ts) */
export const PLAN_KEYS = [RUN, ARG, LINE, MOUNT, ENV, OLD_ARGS] as const;

/** The plan `params` describe. Throws for an address of the old comma-joined `args` alone
 * (with `arg` or `cmdline` beside it, `args` is the older builds' copy: tests/bench argParams). */
export function readPlan(params: URLSearchParams): LaunchPlan {
  const list = params.getAll(ARG);
  const line = params.get(LINE);
  if (params.has(OLD_ARGS) && !list.length && line === null)
    throw new Error("&args= は使えません。引数ごとに &arg= を付けてください（または &cmdline=）");
  if (line !== null && list.length) throw new Error("&arg= と &cmdline= は一緒に使えません");
  return {
    run: params.get(RUN) ?? DEFAULT_RUN,
    args: line !== null ? { kind: "line", line } : { kind: "list", list },
    mounts: params.getAll(MOUNT),
    env: params.getAll(ENV),
  };
}

/** Whether `params` name a program (else the page is the game list, or a game by its id). */
export const hasRun = (params: URLSearchParams) => params.has(RUN);

/** Puts `args` into `params` (in place of the ones there). */
export function writeArgs(params: URLSearchParams, args: LaunchArgs): void {
  params.delete(ARG);
  params.delete(LINE);
  params.delete(OLD_ARGS);
  if (args.kind === "line") params.set(LINE, args.line);
  else for (const a of args.list) params.append(ARG, a);
}

/** Puts `plan` into `params`. The address's own &env= entries the plan lacks stay after its
 * (a trace asked for, &env=MASU_D3D_TRACE=1: later wins, so it can override the plan's). */
export function writePlan(params: URLSearchParams, plan: LaunchPlan): void {
  params.set(RUN, plan.run);
  writeArgs(params, plan.args);
  params.delete(MOUNT);
  for (const m of plan.mounts) params.append(MOUNT, m);
  const asked = params.getAll(ENV).filter((e) => !plan.env.includes(e));
  params.delete(ENV);
  for (const e of [...plan.env, ...asked]) params.append(ENV, e);
}

/** What sake's main is given for `plan`: its argv after the program, and the host environment
 * that goes with it (SAKE_COMMAND_TAIL: the command line's text, kept as written). */
export function sakeArguments(plan: LaunchPlan): { argv: string[]; env: Record<string, string> } {
  const host = plan.run.replace(/^z:/i, "").replaceAll("\\", "/");
  return plan.args.kind === "line"
    ? { argv: [host], env: { SAKE_COMMAND_TAIL: plan.args.line } }
    : { argv: [host, ...plan.args.list], env: {} };
}

/** The arguments for logs: the list as JSON (its empty ones and spaces seen), or the line. */
export const argsText = (args: LaunchArgs): string =>
  args.kind === "line" ? args.line : JSON.stringify(args.list);

/** What a backend needs of the browser (review 22bb061 §4: checked when a game starts, not
 * before the list: the list, its settings and saves work without either). */
export type Feature = "isolation" | "webgpu";
/** sake: its threads share memory (cross-origin isolation) and it draws with WebGPU. */
export const SAKE_NEEDS: readonly Feature[] = ["isolation", "webgpu"];
/** a web game's page with Node: its synchronous reads wait on shared memory (sync-read.ts). */
export const SYNC_READS_NEED: readonly Feature[] = ["isolation"];

const FEATURE_MISSING: Record<Feature, string> = {
  isolation: "SharedArrayBuffer が使えません (COOP/COEP ヘッダが必要)",
  webgpu: "WebGPU に対応していないブラウザです",
};

/** The browser's side of each feature. */
export function browserHas(): Record<Feature, boolean> {
  return {
    isolation: typeof crossOriginIsolated !== "undefined" && crossOriginIsolated,
    webgpu: typeof navigator !== "undefined" && "gpu" in navigator,
  };
}

/** Why a start that needs `need` cannot run here, or null. */
export function missingFeature(need: readonly Feature[], has = browserHas()): string | null {
  const f = need.find((n) => !has[n]);
  return f ? FEATURE_MISSING[f] : null;
}
