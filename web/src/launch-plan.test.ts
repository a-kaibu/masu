import { expect, test } from "vite-plus/test";
import { applyGame, type GameEntry } from "./launcher";
import {
  missingFeature,
  readPlan,
  SAKE_NEEDS,
  sakeArguments,
  SYNC_READS_NEED,
  writeArgs,
  writePlan,
  type LaunchPlan,
} from "./launch-plan";

function rng(seed: number) {
  let s = seed >>> 0;
  return () => (s = (Math.imul(s, 1664525) + 1013904223) >>> 0) / 0x100000000;
}

test("the review's arguments come back as they were (22bb061 §4)", () => {
  const list = ["--title=a,b", "", "two words"];
  const params = new URLSearchParams();
  writeArgs(params, { kind: "list", list });
  expect(readPlan(params).args).toEqual({ kind: "list", list });
  expect(sakeArguments(readPlan(params)).argv.slice(1)).toEqual(list);
  // (through the address bar's text too)
  expect(readPlan(new URLSearchParams(`?${params}`)).args).toEqual({ kind: "list", list });
});

test("a game's list of arguments reaches sake as that list, any characters (seeded property)", () => {
  const r = rng(2026);
  const alphabet = 'a,, "\\\\&=%+?#/:あ\t';
  const word = () =>
    Array.from({ length: Math.floor(r() * 7) }, () => alphabet[Math.floor(r() * alphabet.length)]).join("");
  for (let i = 0; i < 2000; i++) {
    const g: GameEntry = {
      id: "g",
      title: "g",
      run: `Z:\\masu\\game\\${word()}.exe`,
      args: Array.from({ length: Math.floor(r() * 5) }, word),
      mounts: Array.from({ length: Math.floor(r() * 3) }, () => `/masu/${word()}=${word()}`),
      env: Array.from({ length: Math.floor(r() * 3) }, () => `${word()}=${word()}`),
    };
    const params = new URLSearchParams(r() < 0.5 ? "debug" : "");
    applyGame(params, g);
    const plan = readPlan(new URLSearchParams(params.toString()));
    expect(plan).toEqual({ run: g.run, args: { kind: "list", list: g.args }, mounts: g.mounts, env: g.env });
    const { argv, env } = sakeArguments(plan);
    expect(argv).toEqual([g.run.replace(/^z:/i, "").replaceAll("\\", "/"), ...g.args]);
    expect(env).toEqual({});
  }
});

test("a program's command line text is kept as written, and goes to sake as its tail", () => {
  const params = new URLSearchParams("run=a.exe&arg=old");
  writeArgs(params, { kind: "line", line: '  /from="auto run" a\\"b ' });
  const plan = readPlan(params);
  expect(plan.args).toEqual({ kind: "line", line: '  /from="auto run" a\\"b ' });
  expect(params.has("arg")).toBe(false);
  expect(sakeArguments(plan)).toEqual({
    argv: ["a.exe"],
    env: { SAKE_COMMAND_TAIL: '  /from="auto run" a\\"b ' },
  });
});

test("an address of the old comma-joined &args= or of both kinds is refused, not split", () => {
  expect(() => readPlan(new URLSearchParams("run=a.exe&args=x,y"))).toThrow("&args=");
  expect(() => readPlan(new URLSearchParams("run=a.exe&arg=x&cmdline=y"))).toThrow("&cmdline=");
  // (the bench's address for older builds too: tests/bench/suite.mjs argParams)
  expect(readPlan(new URLSearchParams("run=a.exe&args=x,y&arg=x&arg=y")).args).toEqual({
    kind: "list",
    list: ["x", "y"],
  });
});

test("writePlan keeps the address's own &env= after the plan's", () => {
  const params = new URLSearchParams("env=MASU_D3D_TRACE=1&env=A=1");
  const plan: LaunchPlan = {
    run: "r.exe",
    args: { kind: "list", list: [] },
    mounts: [],
    env: ["A=1", "B=2"],
  };
  writePlan(params, plan);
  expect(params.getAll("env")).toEqual(["A=1", "B=2", "MASU_D3D_TRACE=1"]);
});

test("a start is refused for what its backend needs and the browser lacks, nothing else", () => {
  const all = [true, false];
  for (const isolation of all)
    for (const webgpu of all) {
      const has = { isolation, webgpu };
      expect(missingFeature(SAKE_NEEDS, has) === null).toBe(isolation && webgpu);
      expect(missingFeature(SYNC_READS_NEED, has) === null).toBe(isolation);
      // (the list itself needs nothing)
      expect(missingFeature([], has)).toBeNull();
    }
  expect(missingFeature(SAKE_NEEDS, { isolation: true, webgpu: false })).toContain("WebGPU");
});
