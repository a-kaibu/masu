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
