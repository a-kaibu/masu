// The worker a web game's frame reads its files through synchronously (sync-read.ts), built as
// frame-read.js and served by masu serve's game listener: it asks the app's tree worker over the
// port the page hands it first (gameport.ts ReadAsk), piece by piece.
import { ASK_MAX, isReadAnswer, treePath, type ReadAnswer, type ReadAsk } from "./gameport";
import { DATA_MAX, SYNC_MS, readServer, type Shared } from "./sync-read";

let port: MessagePort | null = null;
const pending = new Map<number, (a: ReadAnswer) => void>();
let nextId = 1;
const ask = (path: string, from: number, to: number) =>
  new Promise<ReadAnswer>((resolve) => {
    const id = nextId++;
    const timeout = setTimeout(() => {
      pending.delete(id);
      resolve({ id, kind: "error", error: "file read timed out" });
    }, SYNC_MS);
    pending.set(id, (answer) => {
      clearTimeout(timeout);
      resolve(answer);
    });
    port!.postMessage({ id, path, from, to } satisfies ReadAsk);
  });

/** The file at `url` (/tree/<tree>/<path> of this origin) as a response: 404 when there is none. */
export async function fileResponse(url: string): Promise<Response> {
  const at = treePath(new URL(url).pathname);
  if (!at || !port) return new Response(null, { status: 404 });
  const parts: Uint8Array[] = [];
  for (let from = 0, size = 1; from < size;) {
    const a = await ask(at.path, from, from + ASK_MAX);
    if (a.kind === "none") return new Response(null, { status: 404 });
    if (a.kind === "error") return new Response(a.error, { status: 502 });
    if (a.size > DATA_MAX) return new Response(null, { status: 413 });
    if ((from > 0 && a.size !== size) || a.bytes.byteLength !== Math.min(ASK_MAX, a.size - from))
      return new Response(null, { status: 502 });
    size = a.size;
    parts.push(new Uint8Array(a.bytes));
    from += a.bytes.byteLength;
  }
  return new Response(new Blob(parts as Uint8Array<ArrayBuffer>[]));
}

const scope = self as unknown as {
  onmessage: ((e: MessageEvent) => void) | null;
  postMessage(m: unknown): void;
};
scope.onmessage = (e: MessageEvent) => {
  const m = e.data as Partial<Shared & { port: MessagePort }> | null;
  if (
    port ||
    !(m?.port instanceof MessagePort) ||
    !(m.ctl instanceof SharedArrayBuffer) ||
    !(m.data instanceof SharedArrayBuffer)
  )
    return;
  port = m.port;
  port.onmessage = (a: MessageEvent) => {
    const v: unknown = a.data;
    if (!isReadAnswer(v)) return;
    pending.get(v.id)?.(v);
    pending.delete(v.id);
  };
  void readServer({ ctl: m.ctl, data: m.data }, fileResponse);
  // (running: the page may wait on it now, sync-read.ts frameReady)
  scope.postMessage("ready");
};
