// Messages between the app's page (web/src/html.ts) and a web game's frame (web/src/frame.ts),
// which runs on another origin (masu serve's game listener) and reaches nothing of the app's
// but through these.
import { isWantPort, type WantPort } from "./gameport";

/** frame → page */
export type FromFrame =
  /** the frame's script runs: send the saves */
  | { masu: "hello" }
  /** one item of the saves changed (null: removed) */
  | { masu: "set"; key: string; value: string | null }
  /** the frame's service worker needs the port it reads the tree's files over (gameport.ts) */
  | WantPort;

/** page → frame */
export interface Init {
  masu: "init";
  /** this start of the game (html.ts: one per startWebGame): the hellos a frame sends again
   * are each answered, and only its first init starts the game */
  launch: string;
  /** the saves: localStorage, and files the game wrote (FILE + path, base64) */
  items: [string, string][];
  /** NW.js: the game folder's URL on the frame's own origin, which its synchronous fs reads
   * (sync-read.ts) name, and the paths of the folder's files ("/"-separated). The message
   * carries a MessagePort to the app's tree worker for those reads (gameport.ts ReadAsk). */
  base: string;
  names: string[];
}

/** Keys of the saves that hold files the game wrote with Node's fs (base64). */
export const FILE = "\0file:";

/** Whether `m` is a well-formed message from the frame (anything else is ignored). */
export function fromFrame(m: unknown): FromFrame | null {
  if (typeof m !== "object" || m === null) return null;
  const o = m as Record<string, unknown>;
  if (o.masu === "hello") return { masu: "hello" };
  if (isWantPort(o)) return { masu: "want-port", tree: o.tree };
  if (o.masu === "set" && typeof o.key === "string" && (typeof o.value === "string" || o.value === null))
    return { masu: "set", key: o.key, value: o.value };
  return null;
}

/** Where a frame's start stands (review 22bb061 §5: two inits while the reads' worker started
 * each released the held DOMContentLoaded and load). */
export type FrameStartState =
  | { readonly kind: "waiting" }
  | { readonly kind: "starting"; readonly launch: string }
  | { readonly kind: "ready"; readonly launch: string | null }
  | { readonly kind: "failed"; readonly launch: string | null; readonly why: string };

/**
 * A frame's start: the first init starts it (its saves taken, the reads' worker started when the
 * page has Node), any later one is the same start's answer to a hello sent again and is left
 * alone; `go` runs once, when the reads' worker runs, or when it did not start (failed: the
 * game goes on, its synchronous reads fail) or no init came (`gaveUp`).
 */
export class FrameStart {
  #state: FrameStartState = { kind: "waiting" };
  constructor(private readonly go: () => void) {}
  get state(): FrameStartState {
    return this.#state;
  }
  /** An init; `start` takes the saves and starts the reads' worker when needed. True: this one starts the game. */
  init(m: Init, start: () => Promise<"ready" | "timeout" | "unavailable" | "none">): boolean {
    if (this.#state.kind !== "waiting") return false;
    this.#state = { kind: "starting", launch: m.launch };
    void start().then((r) => {
      if (this.#state.kind !== "starting") return;
      this.#state =
        r === "ready" || r === "none"
          ? { kind: "ready", launch: m.launch }
          : {
              kind: "failed",
              launch: m.launch,
              why: r === "timeout" ? "the reads' worker did not start" : "no reads here",
            };
      this.go();
    });
    return true;
  }
  /** No init in time: the game goes on without the saves. */
  gaveUp(): boolean {
    if (this.#state.kind !== "waiting") return false;
    this.#state = { kind: "failed", launch: null, why: "no saves from the app" };
    this.go();
    return true;
  }
}
