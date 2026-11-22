// Runners side by side for one job (a game's save to the device, reads ahead, a pool of fetches).
// The job is over only once every runner has stopped (review 22bb061 §6): its end, stopped or
// failed, is said after, not while the others still write chunks, hold locks and report
// progress (before, Promise.all said "failed" at the first failure and a save started again ran
// beside the first one's runners).

/** `width` runners, `run(i, signal)` each; `signal` aborts when `outer` does or a runner failed.
 * Resolves, or throws the first failure, once all of them have returned. */
export async function workers(
  width: number,
  run: (i: number, signal: AbortSignal) => Promise<void>,
  outer?: AbortSignal,
): Promise<void> {
  const stop = new AbortController();
  const forward = () => stop.abort(outer?.reason);
  if (outer?.aborted) forward();
  else outer?.addEventListener("abort", forward, { once: true });
  let failure: { e: unknown } | null = null;
  try {
    await Promise.allSettled(
      Array.from({ length: width }, async (_, i) => {
        try {
          await run(i, stop.signal);
        } catch (e) {
          failure ??= { e };
          stop.abort(e);
        }
      }),
    );
  } finally {
    outer?.removeEventListener("abort", forward);
  }
  if (failure) throw (failure as { e: unknown }).e;
}

/** `jobs`, `width` at a time (the next starts as one ends); their results in order. The first
 * failure is thrown once the running ones have ended, and no job starts after it. */
export async function pool<T>(width: number, jobs: (() => Promise<T>)[]): Promise<T[]> {
  const out: T[] = [];
  let next = 0;
  await workers(Math.max(1, Math.min(width, jobs.length)), async (_, signal) => {
    while (next < jobs.length && !signal.aborted) {
      const i = next++;
      out[i] = await jobs[i]!();
    }
  });
  return out;
}
