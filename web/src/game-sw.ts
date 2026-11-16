// The service worker of web games' frames (built as game-sw.js, registered by game-boot.ts on
// masu serve's game listener): answers /tree/<tree>/<path> with the file's bytes, which it asks
// the app's page for (gameport.ts), so a game's files come from the device or the bucket and
// not through the server. Ranges are honoured (one `bytes=a-b`; 416 for one not satisfiable),
// pages get the frame's script first (treeread.ts withHook), and big files go out piece by
// piece.
import {
  ASK_MAX,
  isGivePort,
  isReadAnswer,
  treePath,
  type PortOk,
  type ReadAnswer,
  type ReadAsk,
  type WantPort,
} from "./gameport";
import { byteRange, MAX_PAGE, mime, withHook } from "./treeread";

interface ExtendableEvent extends Event {
  waitUntil(p: Promise<unknown>): void;
}
interface FetchEvent extends ExtendableEvent {
  request: Request;
  respondWith(r: Response | Promise<Response>): void;
}
interface WindowClient {
  postMessage(m: unknown): void;
}
interface ExtendableMessageEvent extends ExtendableEvent {
  data: unknown;
  ports: readonly MessagePort[];
  source: WindowClient | null;
}
interface WorkerScope {
  clients: {
    claim(): Promise<void>;
    matchAll(o: { type: "window"; includeUncontrolled: boolean }): Promise<WindowClient[]>;
  };
  skipWaiting(): Promise<void>;
  addEventListener(type: "install" | "activate", f: (e: ExtendableEvent) => void): void;
  addEventListener(type: "fetch", f: (e: FetchEvent) => void): void;
  addEventListener(type: "message", f: (e: ExtendableMessageEvent) => void): void;
}
const sw = self as unknown as WorkerScope;

/** How long the app's page may take to answer one read (a chunk from the bucket: its retries). */
const ANSWER_MS = 90_000;
/** How long to wait for a port (asked for again every ASK_AGAIN_MS meanwhile). */
const PORT_MS = 15_000;
const ASK_AGAIN_MS = 1000;
/** A big file's piece (asked for one or two ahead of the one going out). */
const PIECE = 2 << 20;

sw.addEventListener("install", () => void sw.skipWaiting());
sw.addEventListener("activate", (e) => e.waitUntil(sw.clients.claim()));

// the ports by tree, and the reads waiting on each
const ports = new Map<string, MessagePort>();
const waiting = new Map<string, ((p: MessagePort) => void)[]>();
const pending = new Map<number, (a: ReadAnswer) => void>();
let nextId = 1;

sw.addEventListener("message", (e) => {
  const port = e.ports[0];
  if (!isGivePort(e.data) || !port) return;
  const { tree } = e.data;
  ports.get(tree)?.close();
  ports.set(tree, port);
  port.onmessage = (m: MessageEvent) => {
    const a: unknown = m.data;
    if (!isReadAnswer(a)) return;
    pending.get(a.id)?.(a);
    pending.delete(a.id);
  };
  for (const f of waiting.get(tree) ?? []) f(port);
  waiting.delete(tree);
  e.source?.postMessage({ masu: "port-ok", tree } satisfies PortOk);
});

/** The port for `tree`, asked of every window of the origin when there is none. */
async function portOf(tree: string): Promise<MessagePort> {
  const had = ports.get(tree);
  if (had) return had;
  return new Promise<MessagePort>((resolve, reject) => {
    const list = waiting.get(tree) ?? [];
    waiting.set(tree, list);
    let done = false;
    const ask = async () => {
      for (const c of await sw.clients.matchAll({ type: "window", includeUncontrolled: true }))
        c.postMessage({ masu: "want-port", tree } satisfies WantPort);
    };
    const again = setInterval(() => void ask(), ASK_AGAIN_MS);
    const give = setTimeout(() => {
      if (done) return;
      done = true;
      clearInterval(again);
      const at = list.indexOf(got);
      if (at >= 0) list.splice(at, 1);
      reject(new Error(`no page answers for tree ${tree}`));
    }, PORT_MS);
    const got = (p: MessagePort) => {
      done = true;
      clearInterval(again);
      clearTimeout(give);
      resolve(p);
    };
    list.push(got);
    void ask();
  });
}

/** Bytes [from, to) of `path` in `tree`; the port given up on (and asked for anew) once when the
 * page does not answer in time (its tab closed: another may have the tree). */
async function read(tree: string, path: string, from: number, to: number, again = true): Promise<ReadAnswer> {
  const port = await portOf(tree);
  const id = nextId++;
  const answer = await new Promise<ReadAnswer | null>((resolve) => {
    const t = setTimeout(() => {
      pending.delete(id);
      resolve(null);
    }, ANSWER_MS);
    pending.set(id, (a) => {
      clearTimeout(t);
      resolve(a);
    });
    port.postMessage({ id, path, from, to } satisfies ReadAsk);
  });
  if (answer) return answer;
  if (ports.get(tree) === port) ports.delete(tree);
  if (again) return read(tree, path, from, to, false);
  return { id, kind: "error", error: "the app's page did not answer" };
}

const HEADERS = {
  "cross-origin-embedder-policy": "require-corp",
  "cross-origin-resource-policy": "cross-origin",
  "cache-control": "no-cache",
  "x-content-type-options": "nosniff",
};

const status = (code: number, text: string) =>
  new Response(text, { status: code, headers: { ...HEADERS, "content-type": "text/plain; charset=utf-8" } });

async function serve(req: Request, tree: string, path: string): Promise<Response> {
  // (the size first: an empty read)
  const stat = await read(tree, path, 0, 0);
  switch (stat.kind) {
    case "none":
      return status(404, "not found");
    case "error":
      return status(502, stat.error);
    case "bytes":
      break;
  }
  const size = stat.size;
  const headers: Record<string, string> = {
    ...HEADERS,
    "content-type": mime(path),
    "accept-ranges": "bytes",
  };
  const part = byteRange(req.headers.get("range"), size);
  if (part === "bad")
    return new Response(null, { status: 416, headers: { ...headers, "content-range": `bytes */${size}` } });
  if (req.method === "HEAD") {
    headers["content-length"] = String(size);
    return new Response(null, { status: 200, headers });
  }
  if (size === 0)
    return new Response(new Uint8Array(0), { status: 200, headers: { ...headers, "content-length": "0" } });
  const [a, b] = part ?? [0, size - 1];
  const whole = async (): Promise<Uint8Array> => {
    const out = new Uint8Array(b - a + 1);
    for (let at = a; at <= b; at += ASK_MAX) {
      const r = await read(tree, path, at, Math.min(b + 1, at + ASK_MAX));
      if (r.kind !== "bytes") throw new Error(r.kind === "error" ? r.error : "gone");
      out.set(new Uint8Array(r.bytes), at - a);
    }
    return out;
  };
  if (!part && size <= MAX_PAGE && mime(path).startsWith("text/html")) {
    let page: Uint8Array;
    try {
      page = withHook(await whole());
    } catch (e) {
      return status(502, String(e));
    }
    headers["content-length"] = String(page.length);
    return new Response(page as Uint8Array<ArrayBuffer>, { status: 200, headers });
  }
  headers["content-length"] = String(b - a + 1);
  let at = a;
  const body = new ReadableStream<Uint8Array>(
    {
      async pull(c) {
        if (at > b) return c.close();
        const to = Math.min(b + 1, at + PIECE);
        const r = await read(tree, path, at, to);
        if (r.kind !== "bytes" || r.bytes.byteLength !== to - at)
          return c.error(new Error(r.kind === "error" ? r.error : "short read"));
        at = to;
        c.enqueue(new Uint8Array(r.bytes));
        if (at > b) c.close();
      },
    },
    { highWaterMark: 2 },
  );
  if (part) {
    headers["content-range"] = `bytes ${a}-${b}/${size}`;
    return new Response(body, { status: 206, headers });
  }
  return new Response(body, { status: 200, headers });
}

sw.addEventListener("fetch", (e) => {
  const req = e.request;
  const url = new URL(req.url);
  if (url.origin !== location.origin || (req.method !== "GET" && req.method !== "HEAD")) return;
  const at = treePath(url.pathname);
  if (!at) return;
  e.respondWith(serve(req, at.tree, at.path).catch((err: unknown) => status(502, String(err))));
});
