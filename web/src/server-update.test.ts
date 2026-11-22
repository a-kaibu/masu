import { afterAll, beforeAll, describe, expect, it, test } from "vite-plus/test";
import {
  checkUpdate,
  installUpdate,
  parseUpdate,
  sameVersion,
  UpdateJob,
  waitForVersion,
  type UpdateInfo,
} from "./server-update";
import { PageGame } from "./session";
import { seeds, simulate } from "./test/simulation";

const g = globalThis as unknown as { location?: unknown };
const oldLocation = g.location;
beforeAll(() => {
  g.location = { href: "https://masu.test/sub/index.html" };
});
afterAll(() => {
  g.location = oldLocation;
});

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

function rng(seed: number) {
  return () => (seed = (seed * 1103515245 + 12345) >>> 0) / 2 ** 32;
}

describe("parseUpdate", () => {
  it("reads the server's answer", () => {
    expect(
      parseUpdate({ current: "0.3.3", latest: "0.3.4", newer: true, supported: true, error: null }),
    ).toEqual({
      current: "0.3.3",
      latest: "0.3.4",
      newer: true,
      supported: true,
      error: "",
    });
    expect(
      parseUpdate({ current: "0.3.3", latest: null, newer: false, supported: false, error: "rate limited" }),
    ).toEqual({ current: "0.3.3", latest: "", newer: false, supported: false, error: "rate limited" });
  });
  it("is null for anything that is not one (random values)", () => {
    for (const v of [null, undefined, 1, "x", [], {}, { current: 1, newer: true }, { current: "1" }])
      expect(parseUpdate(v)).toBeNull();
    const r = rng(9);
    const vals = [null, 0, true, false, "", "0.3.4", [], {}];
    for (let n = 0; n < 500; n++) {
      const o: Record<string, unknown> = {};
      for (const k of ["current", "latest", "newer", "supported", "error"])
        if (r() < 0.8) o[k] = vals[Math.floor(r() * vals.length)];
      const p = parseUpdate(o);
      if (p) {
        expect(typeof o.current).toBe("string");
        expect(typeof o.newer).toBe("boolean");
        for (const k of ["current", "latest", "error"] as const) expect(typeof p[k]).toBe("string");
      } else expect(typeof o.current !== "string" || typeof o.newer !== "boolean").toBe(true);
    }
  });
});

describe("sameVersion", () => {
  it("ignores a leading v, and nothing is no version", () => {
    expect(sameVersion("v0.3.4", "0.3.4")).toBe(true);
    expect(sameVersion("0.3.4", "v0.3.4")).toBe(true);
    expect(sameVersion("0.3.4", "0.3.40")).toBe(false);
    expect(sameVersion("", "")).toBe(false);
  });
});

describe("checkUpdate / installUpdate", () => {
  it("asks the page's own server, under its path", async () => {
    const asked: string[] = [];
    const f = (u: URL, init?: RequestInit) => {
      asked.push(`${init?.method ?? "GET"} ${u.href}`);
      return Promise.resolve(
        init?.method === "POST"
          ? json({ installed: "0.3.4" })
          : json({ current: "0.3.3", latest: "0.3.4", newer: true, supported: true }),
      );
    };
    expect((await checkUpdate(f)).newer).toBe(true);
    expect(await installUpdate(f)).toBe("0.3.4");
    expect(asked).toEqual(["GET https://masu.test/sub/api/update", "POST https://masu.test/sub/api/update"]);
  });
  it("says why it did not update", async () => {
    const answer = (r: Response) => () => Promise.resolve(r);
    await expect(installUpdate(answer(new Response("最新です", { status: 409 })))).rejects.toThrow(
      /^最新です$/,
    );
    await expect(installUpdate(answer(new Response("更新中です", { status: 409 })))).rejects.toThrow(
      /^更新中です$/,
    );
    await expect(installUpdate(answer(new Response("", { status: 409 })))).rejects.toThrow(/^最新です$/);
    await expect(installUpdate(answer(new Response("", { status: 501 })))).rejects.toThrow(/Linux/);
    await expect(
      installUpdate(answer(new Response("更新できませんでした: checksum", { status: 502 }))),
    ).rejects.toThrow(/^更新できませんでした: checksum$/);
    await expect(installUpdate(answer(new Response("download failed", { status: 500 })))).rejects.toThrow(
      /500: download failed/,
    );
    await expect(installUpdate(answer(json({})))).rejects.toThrow(/読めません/);
    await expect(checkUpdate(answer(new Response("", { status: 404 })))).rejects.toThrow(/対応していません/);
    // (an older server answers /api/update with the page)
    await expect(checkUpdate(answer(new Response("<!doctype html>")))).rejects.toThrow(/対応していません/);
  });
});

describe("waitForVersion", () => {
  it("waits through the old server and the restart until the new version answers", async () => {
    let t = 0;
    const seq = [json({ version: "0.3.3" }), "down", "down", json({ version: "0.3.4", commit: "abc1234" })];
    let i = 0;
    const f = () => {
      const s = seq[Math.min(i++, seq.length - 1)];
      return s === "down" ? Promise.reject(new TypeError("Failed to fetch")) : Promise.resolve(s as Response);
    };
    const ok = await waitForVersion("v0.3.4", {
      f,
      sleep: async (ms) => void (t += ms),
      now: () => t,
      everyMs: 500,
    });
    expect(ok).toBe(true);
    expect(i).toBe(4);
    expect(t).toBe(1500);
  });
  it("gives up after the timeout (random answers that never match)", async () => {
    const r = rng(4);
    for (let n = 0; n < 50; n++) {
      let t = 0;
      let calls = 0;
      const f = () => {
        calls++;
        const x = r();
        return x < 0.3
          ? Promise.reject(new Error("down"))
          : Promise.resolve(
              x < 0.6 ? json({ version: "0.3.3" }) : x < 0.8 ? new Response("<html>") : json({}, 503),
            );
      };
      const ok = await waitForVersion("0.3.4", {
        f,
        timeoutMs: 10_000,
        everyMs: 700,
        sleep: async (ms) => void (t += ms),
        now: () => t,
      });
      expect(ok).toBe(false);
      expect(calls).toBe(Math.ceil(10_000 / 700));
    }
  });
});

const newer: UpdateInfo = { current: "v1", latest: "v2", newer: true, supported: true, error: "" };

function job(game: PageGame, waits: number[]) {
  const calls = { install: 0, reload: 0, check: 0, inFlight: 0, most: 0, underGame: 0 };
  const j = new UpdateJob({
    check: async () => (calls.check++, newer),
    install: () => {
      calls.most = Math.max(calls.most, ++calls.inFlight);
      return new Promise((ok) => setTimeout(() => (calls.install++, calls.inFlight--, ok("v2")), waits[0]));
    },
    wait: () => new Promise((ok) => setTimeout(() => ok(true), waits[1])),
    refresh: () => new Promise((ok) => setTimeout(ok, waits[2])),
    reload: () => {
      const inGame = game.started;
      const did = game.reload(() => calls.reload++);
      if (did && inGame) calls.underGame++;
      return did;
    },
  });
  return { j, calls };
}

test("a game started while the server restarts is not reloaded under; the job goes on with no screen (review 22bb061 §6)", async () => {
  await simulate(async (sim) => {
    const game = new PageGame();
    const { j, calls } = job(game, [100, 300, 10]);
    j.shown();
    await sim.advance(0);
    expect(j.step).toEqual({ s: "ready", info: newer });
    void j.update(newer);
    // (the screen is closed: nothing listens; the player picks a game meanwhile)
    await sim.advance(200);
    game.begin();
    await sim.advance(1000);
    expect(calls).toMatchObject({ install: 1, reload: 0, check: 1 });
    expect(j.step).toEqual({ s: "restarted", info: newer, version: "v2" });
    // (opened again: the step is kept, not checked again)
    j.shown();
    expect(j.step.s).toBe("restarted");
  });
});

test("property: whatever the order of updates, checks, screens and a game's start, one install at a time and a reload only with no game (seeded)", async () => {
  for (const seed of [...seeds, ...Array.from({ length: 200 }, (_, i) => i + 1)])
    await simulate(async (sim) => {
      const game = new PageGame();
      const waits = [sim.randomBelow(200), sim.randomBelow(400), sim.randomBelow(20)];
      const { j, calls } = job(game, waits);
      let gameAt = Infinity;
      for (let t = 0; t < 40; t++) {
        const what = sim.randomBelow(5);
        if (what === 0) j.shown();
        if (what === 1) void j.check();
        if (what === 2) void j.update(newer);
        if (what === 3 && !game.started) {
          game.begin();
          gameAt = t;
        }
        await sim.advance(sim.randomBelow(100));
      }
      await sim.advance(2000);
      expect(calls.most, `seed ${seed}`).toBeLessThanOrEqual(1);
      expect(calls.underGame, `seed ${seed}`).toBe(0);
      if (j.step.s === "restarted") expect(gameAt, `seed ${seed}`).toBeLessThan(Infinity);
      if (gameAt === Infinity) expect(j.step.s, `seed ${seed}`).not.toBe("restarted");
    }, seed);
});
