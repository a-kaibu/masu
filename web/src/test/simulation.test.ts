import { expect, test, vi } from "vite-plus/test";
import { deferred, simulate } from "./simulation";

test("simulation: virtual deadlines, same-time FIFO, promise continuations and cancellation replay exactly", async () => {
  for (let replay = 0; replay < 2; replay++) {
    await simulate(async ({ advance }) => {
      const trace: string[] = [];
      const reply = deferred<void>();
      const consumed = reply.promise.then(() => trace.push("ack"));
      setTimeout(() => {
        trace.push("first");
        reply.resolve();
      }, 25);
      setTimeout(() => trace.push("second"), 25);
      clearTimeout(setTimeout(() => trace.push("cancelled"), 25));
      await advance(24);
      expect(Date.now()).toBe(24);
      expect(performance.now()).toBe(24);
      expect(crypto.randomUUID()).toMatch(/^[0-9a-f-]{14}4[0-9a-f]{3}-[89ab]/);
      expect(trace).toEqual([]);
      await advance(1);
      await consumed;
      expect(trace).toEqual(["first", "ack", "second"]);
    });
    expect(vi.isFakeTimers()).toBe(false);
  }
});

test("simulation: a failed scenario restores the host clock, deadlines and ID generator", async () => {
  const descriptors = () =>
    [
      "navigator",
      "document",
      "location",
      "fetch",
      "localStorage",
      "sessionStorage",
      "addEventListener",
      "removeEventListener",
    ].map((name) => Object.getOwnPropertyDescriptor(globalThis, name));
  const originalGlobals = descriptors();
  const random = Object.getOwnPropertyDescriptor(Math, "random");
  const cryptoBytes = Object.getOwnPropertyDescriptor(crypto, "getRandomValues");
  const uuid = Object.getOwnPropertyDescriptor(crypto, "randomUUID");
  const deadline = Object.getOwnPropertyDescriptor(AbortSignal, "timeout");
  await expect(
    simulate(async () => {
      throw new Error("scenario failed");
    }),
  ).rejects.toThrow("scenario failed");
  expect(descriptors()).toEqual(originalGlobals);
  expect(Object.getOwnPropertyDescriptor(Math, "random")).toEqual(random);
  expect(Object.getOwnPropertyDescriptor(crypto, "getRandomValues")).toEqual(cryptoBytes);
  expect(Object.getOwnPropertyDescriptor(crypto, "randomUUID")).toEqual(uuid);
  expect(Object.getOwnPropertyDescriptor(AbortSignal, "timeout")).toEqual(deadline);
  expect(vi.isFakeTimers()).toBe(false);
});

test("simulation: seed replays entropy, independent of the event schedule and previous origin", async () => {
  const replay = (seed: number, extraEntropy = false) =>
    simulate(async ({ randomBelow, advance, browser }) => {
      expect(localStorage.length).toBe(0);
      expect(sessionStorage.length).toBe(0);
      expect(location.origin).toBe("https://masu.test");
      expect(navigator.language).toBe("en-US");
      const bytes = new Uint8Array(6).fill(255);
      crypto.getRandomValues(bytes.subarray(1, 5));
      const entropy = [Math.random(), crypto.randomUUID(), ...bytes];
      expect(bytes[0]).toBe(255);
      expect(bytes[5]).toBe(255);
      if (extraEntropy) crypto.randomUUID();
      localStorage.setItem("saved", "yes");
      expect(sessionStorage.getItem("saved")).toBe(null);
      const trace: string[] = [];
      addEventListener("offline", () => trace.push(`offline ${Date.now()}`));
      addEventListener("visibilitychange", () => trace.push(`${document.visibilityState} ${Date.now()}`));
      setTimeout(() => browser.setOnline(false), randomBelow(100));
      setTimeout(() => browser.setVisibility("hidden"), randomBelow(100));
      await advance(100);
      expect(navigator.onLine).toBe(false);
      return { entropy, trace };
    }, seed);
  const a = await replay(42);
  expect(a.trace).toEqual(["hidden 8", "offline 25"]);
  expect(await replay(42)).toEqual(a);
  expect((await replay(42, true)).trace).toEqual(a.trace);
  expect((await replay(43)).entropy).not.toEqual(a.entropy);
  expect((await replay(43)).trace).not.toEqual(a.trace);
});

test("simulation: same-name locks wait FIFO, other names progress and a rejection releases the next waiter", async () => {
  await simulate(async () => {
    const trace: string[] = [];
    const hold = deferred<void>();
    const first = navigator.locks.request("game", async () => {
      trace.push("first");
      await hold.promise;
      throw new Error("disk full");
    });
    const second = navigator.locks.request("game", () => {
      trace.push("second");
    });
    const third = navigator.locks.request("game", () => {
      trace.push("third");
    });
    await navigator.locks.request("other", () => {
      trace.push("other");
    });
    expect(trace).toEqual(["first", "other"]);
    hold.resolve();
    await expect(first).rejects.toThrow("disk full");
    await Promise.all([second, third]);
    expect(trace).toEqual(["first", "other", "second", "third"]);
  });
});
