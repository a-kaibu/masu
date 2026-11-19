// Only the test advances time. Advancing also runs promise continuations between timers.
import { vi } from "vite-plus/test";

export async function simulate(run: (advance: (ms: number) => Promise<void>) => Promise<void>) {
  vi.useFakeTimers({
    now: 0,
    toFake: ["Date", "performance", "setTimeout", "clearTimeout", "setInterval", "clearInterval"],
  });
  // Native AbortSignal.timeout uses its own clock; fetch deadlines must follow the test clock too.
  const deadlines = vi.spyOn(AbortSignal, "timeout").mockImplementation((ms) => {
    const controller = new AbortController();
    setTimeout(() => controller.abort(new DOMException("Deadline expired", "TimeoutError")), ms);
    return controller.signal;
  });
  let generation = 0;
  const ids = vi
    .spyOn(crypto, "randomUUID")
    .mockImplementation(() => `00000000-0000-4000-8000-${(++generation).toString(16).padStart(12, "0")}`);
  try {
    await run(async (ms) => {
      await vi.advanceTimersByTimeAsync(ms);
    });
  } finally {
    ids.mockRestore();
    deadlines.mockRestore();
    vi.useRealTimers();
  }
}

// Separate an I/O request starting from its completion or failure.
export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((ok, fail) => {
    resolve = ok;
    reject = fail;
  });
  return { promise, resolve, reject };
}

// The seed in each test name reproduces its entire event schedule.
export function randomBelow(seed: number) {
  let state = seed >>> 0;
  return (limit: number) => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return Math.floor((state / 0x100000000) * limit);
  };
}
