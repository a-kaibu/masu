import { expect, test, vi } from "vite-plus/test";
import { deferred, simulate } from "./simulation";

test("simulation: virtual deadlines, same-time FIFO, promise continuations and cancellation replay exactly", async () => {
  for (let replay = 0; replay < 2; replay++) {
    await simulate(async (advance) => {
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
      expect(crypto.randomUUID()).toBe("00000000-0000-4000-8000-000000000001");
      expect(trace).toEqual([]);
      await advance(1);
      await consumed;
      expect(trace).toEqual(["first", "ack", "second"]);
    });
    expect(vi.isFakeTimers()).toBe(false);
  }
});

test("simulation: a failed scenario restores the host clock, deadlines and ID generator", async () => {
  const uuid = Object.getOwnPropertyDescriptor(crypto, "randomUUID");
  const deadline = Object.getOwnPropertyDescriptor(AbortSignal, "timeout");
  await expect(
    simulate(async () => {
      throw new Error("scenario failed");
    }),
  ).rejects.toThrow("scenario failed");
  expect(Object.getOwnPropertyDescriptor(crypto, "randomUUID")).toEqual(uuid);
  expect(Object.getOwnPropertyDescriptor(AbortSignal, "timeout")).toEqual(deadline);
  expect(vi.isFakeTimers()).toBe(false);
});
