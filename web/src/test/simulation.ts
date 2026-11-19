// Only the test advances time. Advancing also runs promise continuations between timers.
import { vi } from "vite-plus/test";

import { browser } from "./browser";

declare const process: { env: Record<string, string | undefined> };
const configured = process.env.MASU_SIM_SEED;
if (configured !== undefined && (!/^\d+$/.test(configured) || Number(configured) > 0xffffffff)) {
  throw new Error("MASU_SIM_SEED must be an unsigned 32-bit decimal integer");
}
export const seeds = configured === undefined ? [7, 13, 23, 29, 41, 101, 257, 2026] : [Number(configured)];

interface Simulation {
  advance: (ms: number) => Promise<void>;
  randomBelow: (limit: number) => number;
  browser: ReturnType<typeof browser>;
}

export async function simulate<T>(run: (sim: Simulation) => Promise<T>, seed = seeds[0]!): Promise<T> {
  const origin = browser();
  const restoreGlobals = new Map<string, PropertyDescriptor | undefined>();
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
  // Independent streams: an extra ID or random application value cannot move scheduled events.
  const schedule = randomBelow(seed);
  const entropy = randomBelow(seed ^ 0x9e3779b9);
  const app = randomBelow(seed ^ 0x85ebca6b);
  const random = vi.spyOn(Math, "random").mockImplementation(() => app(0x100000000) / 0x100000000);
  const bytes = vi.spyOn(crypto, "getRandomValues").mockImplementation((array) => {
    if (
      !array ||
      !/^\[object (?:Uint8Clamped|(?:Int|Uint)(?:8|16|32)|Big(?:Uint|Int)64)Array\]$/.test(
        Object.prototype.toString.call(array),
      )
    ) {
      throw new DOMException("Expected an integer typed array", "TypeMismatchError");
    }
    if (array.byteLength > 65536) throw new DOMException("Too many bytes", "QuotaExceededError");
    const view = new Uint8Array(array.buffer, array.byteOffset, array.byteLength);
    for (let i = 0; i < view.length; i++) view[i] = entropy(256);
    return array;
  });
  const ids = vi.spyOn(crypto, "randomUUID").mockImplementation(() => {
    const b = crypto.getRandomValues(new Uint8Array(16));
    b[6] = (b[6]! & 15) | 64;
    b[8] = (b[8]! & 63) | 128;
    const hex = Array.from(b, (n) => n.toString(16).padStart(2, "0")).join("");
    return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
  });
  try {
    for (const [name, value] of Object.entries(origin.globals)) {
      restoreGlobals.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
      Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
    }
    return await run({
      browser: origin,
      randomBelow: schedule,
      advance: async (ms) => {
        await vi.advanceTimersByTimeAsync(ms);
      },
    });
  } catch (cause) {
    throw new Error(`Simulation failed with seed ${seed}: ${String(cause)}`, { cause });
  } finally {
    for (const [name, descriptor] of restoreGlobals) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else Reflect.deleteProperty(globalThis, name);
    }
    ids.mockRestore();
    bytes.mockRestore();
    random.mockRestore();
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
function randomBelow(seed: number) {
  let state = seed >>> 0;
  return (limit: number) => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return Math.floor((state / 0x100000000) * limit);
  };
}
