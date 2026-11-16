// How a web game's files reach its frame without the server: the frame's origin (masu serve's
// game listener) has a service worker (game-sw.ts) that answers /tree/<tree>/<path>; it asks
// the app's page that opened the tree (html.ts, which reads the files from the device or the
// bucket: treeread.ts) over a MessagePort. The page hands that port to the frame, which hands
// it to its service worker; the worker asks for one again whenever it has none for a tree (it
// was stopped and started again, the page reloaded):
//
//   worker --want-port--> frame (each window of the origin) --want-port--> app's page
//   app's page --port (MessagePort)--> frame --port--> worker --port-ok--> frame
//
// Over the port the worker asks for bytes of a file and the page answers with them. The page
// answers only for its own tree, so a game reaches no other game's files.

/** worker ⇄ frame, frame ⇄ app's page: the worker needs a port for `tree` */
export interface WantPort {
  masu: "want-port";
  tree: string;
}
/** app's page → frame → worker (the port transferred with it) */
export interface GivePort {
  masu: "port";
  tree: string;
}
/** worker → frame: it has the port for `tree` now */
export interface PortOk {
  masu: "port-ok";
  tree: string;
}

/** A tree's name (as masu serve's trees/<name>.tsv: letters, digits, _ - .). */
export const isTreeName = (s: unknown): s is string =>
  typeof s === "string" &&
  s.length > 0 &&
  s.length <= 200 &&
  /^[A-Za-z0-9_.-]+$/.test(s) &&
  s !== "." &&
  s !== "..";

/** `/tree/<tree>/<path>`: the tree and the path in it (decoded), or null. */
export function treePath(pathname: string): { tree: string; path: string } | null {
  const m = /^\/tree\/([^/]+)\/(.+)$/.exec(pathname);
  if (!m) return null;
  try {
    const tree = decodeURIComponent(m[1]!);
    const path = m[2]!.split("/").map(decodeURIComponent).join("/");
    return isTreeName(tree) ? { tree, path } : null;
  } catch {
    return null;
  }
}

const ofKind = <K extends string>(m: unknown, kind: K): m is { masu: K; tree: string } =>
  typeof m === "object" &&
  m !== null &&
  (m as { masu?: unknown }).masu === kind &&
  isTreeName((m as { tree?: unknown }).tree);

export const isWantPort = (m: unknown): m is WantPort => ofKind(m, "want-port");
export const isGivePort = (m: unknown): m is GivePort => ofKind(m, "port");
export const isPortOk = (m: unknown): m is PortOk => ofKind(m, "port-ok");

/** worker → page over the port: bytes [from, to) of the file at `path` (in the tree, "/"-separated) */
export interface ReadAsk {
  id: number;
  path: string;
  from: number;
  to: number;
}
/** page → worker: the file's size and the bytes asked for (null size: the tree has no such
 * file); or why they could not be read */
export type ReadAnswer =
  | { id: number; kind: "bytes"; size: number; bytes: ArrayBuffer }
  | { id: number; kind: "none" }
  | { id: number; kind: "error"; error: string };

/** The largest read one ask may be (the worker asks big files piece by piece). */
export const ASK_MAX = 16 << 20;

export function isReadAsk(m: unknown): m is ReadAsk {
  const a = m as Partial<ReadAsk> | null;
  return (
    typeof a === "object" &&
    a !== null &&
    Number.isSafeInteger(a.id) &&
    typeof a.path === "string" &&
    Number.isSafeInteger(a.from) &&
    Number.isSafeInteger(a.to) &&
    a.from! >= 0 &&
    a.to! >= a.from! &&
    a.to! - a.from! <= ASK_MAX
  );
}

export function isReadAnswer(m: unknown): m is ReadAnswer {
  const a = m as { id?: unknown; kind?: unknown; size?: unknown; bytes?: unknown; error?: unknown } | null;
  if (typeof a !== "object" || a === null || !Number.isSafeInteger(a.id)) return false;
  switch (a.kind) {
    case "bytes":
      return Number.isSafeInteger(a.size) && (a.size as number) >= 0 && a.bytes instanceof ArrayBuffer;
    case "none":
      return true;
    case "error":
      return typeof a.error === "string";
    default:
      return false;
  }
}

/** What a page needs of a tree to answer reads (treeread.ts TreeReader). */
export interface Files {
  file(path: string): { size: number } | undefined;
  read(f: { size: number }, from: number, to: number): Promise<Uint8Array>;
}

/** Answers the worker's reads on `port` from `files` (the app's page: html.ts). */
export function answerReads(port: MessagePort, files: Files): void {
  port.onmessage = (e: MessageEvent) => {
    const ask: unknown = e.data;
    if (!isReadAsk(ask)) return;
    const f = files.file(ask.path);
    const answer = (a: ReadAnswer, transfer: Transferable[] = []) => port.postMessage(a, transfer);
    if (!f) return answer({ id: ask.id, kind: "none" });
    files.read(f, ask.from, ask.to).then(
      (b) => {
        // (its own buffer, handed over: not a view of a kept chunk)
        const bytes =
          b.byteOffset === 0 && b.byteLength === b.buffer.byteLength && b.buffer instanceof ArrayBuffer
            ? b.buffer
            : b.slice().buffer;
        answer({ id: ask.id, kind: "bytes", size: f.size, bytes }, [bytes]);
      },
      (err: unknown) => answer({ id: ask.id, kind: "error", error: String(err) }),
    );
  };
}

/**
 * In a page of the frame's origin (frame.ts, game-boot.ts): passes the worker's asks for a port
 * to the app's page, and the port it gives to the worker. `onOk(tree)` when the worker has it.
 */
export function relayPorts(onOk: (tree: string) => void = () => {}): void {
  const sw = navigator.serviceWorker;
  if (!sw || parent === window) return;
  sw.addEventListener("message", (e: MessageEvent) => {
    if (isWantPort(e.data))
      parent.postMessage({ masu: "want-port", tree: e.data.tree } satisfies WantPort, "*");
    else if (isPortOk(e.data)) onOk(e.data.tree);
  });
  sw.startMessages();
  addEventListener("message", (e: MessageEvent) => {
    if (e.source !== parent || !isGivePort(e.data) || !e.ports[0]) return;
    const { tree } = e.data;
    const port = e.ports[0];
    void sw.ready.then((r) => r.active?.postMessage({ masu: "port", tree } satisfies GivePort, [port]));
  });
}
