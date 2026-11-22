// One run of a game in the page (review 22bb061 §5): its processes, where the player's input
// goes, and the lifetime the asynchronous work of the run belongs to.
//
// - ProcessTable: the root process and its children (a child is counted from its request,
//   before its module exists). The run ends when the last of them has: `ended` once, from
//   whichever process it was, whether or not one of them ever drew (a child with no window has
//   no frame loop to notice it).
// - Lifetime: what a run's async work checks after each await and before it calls the guest:
//   once the run (or the stream, a decoder's generation) it belongs to is over, it makes
//   nothing new and calls nothing.
// - inputTarget: a key from the page's own UI (a field, a dialog) is the page's, not the game's.

/** Where a run stands. */
export type RunPhase = "running" | "ended";

/** A child process from its request on (play.ts spawn), until it ends once. */
export interface ChildTicket {
  /** it ended (exited, aborted, or never started) */
  ended(): void;
}

export class ProcessTable {
  #rootLive = true;
  #children = 0;
  #phase: RunPhase = "running";
  constructor(private readonly onEnded: () => void) {}

  get phase(): RunPhase {
    return this.#phase;
  }
  get ended(): boolean {
    return this.#phase === "ended";
  }
  /** processes still running or starting */
  get live(): number {
    return (this.#rootLive ? 1 : 0) + this.#children;
  }

  /** The root process exited (Module.onExit). */
  rootExited(): void {
    if (!this.#rootLive) return;
    this.#rootLive = false;
    this.#check();
  }

  /** A child was asked for (vnvmSpawn): counted until it ends. */
  child(): ChildTicket {
    if (this.ended) throw new Error("a process started after the run ended");
    this.#children++;
    let done = false;
    return {
      ended: () => {
        if (done) return;
        done = true;
        this.#children--;
        this.#check();
      },
    };
  }

  #check(): void {
    if (this.#phase === "ended" || this.live) return;
    this.#phase = "ended";
    this.onEnded();
  }
}

/** The lifetime of a run or of something in it (a decoder's stream): `over` once it ended. */
export class Lifetime {
  #over = false;
  #ends: (() => void)[] = [];
  get over(): boolean {
    return this.#over;
  }
  /** Ends it: each `onEnd` once, newest first. */
  end(): void {
    if (this.#over) return;
    this.#over = true;
    for (const f of this.#ends.splice(0).reverse()) f();
  }
  /** `f` when it ends (at once when it has). */
  onEnd(f: () => void): void {
    if (this.#over) f();
    else this.#ends.push(f);
  }
  /** A child lifetime: ended with this one, or on its own. */
  child(): Lifetime {
    const c = new Lifetime();
    this.onEnd(() => c.end());
    return c;
  }
  /** `r` (what an await gave) when still live; else it is let go (`free`) and null. */
  keep<T>(r: T, free?: (r: T) => void): T | null {
    if (!this.#over) return r;
    free?.(r);
    return null;
  }
}

/** Where a key or click of the page goes. */
export type InputTarget = "game" | "overlay";

/** The page's own UI: a key there is for it (a field typed in, a dialog, a menu). Not a button
 * of the bar over the game: after a click on one (自動送り, fullscreen) the keys are the game's
 * still, as before. */
const HOST_UI =
  'input, textarea, select, [contenteditable=""], [contenteditable="true"], [role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"]';

/** Where a key whose event started at `target` goes. */
export function inputTarget(target: EventTarget | null): InputTarget {
  const el = target as { closest?: (s: string) => unknown } | null;
  return el && typeof el.closest === "function" && el.closest(HOST_UI) ? "overlay" : "game";
}
