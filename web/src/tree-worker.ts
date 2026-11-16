// The worker that answers a web game's reads (html.ts starts it): the frame's service worker
// asks over the ports the page hands here (gameport.ts). Not the page itself: the frame's
// synchronous reads (sync-read.ts) hold the main thread, which the page shares with the frame
// (same site, same process), so the page could not answer them.
import { answerReads, isTreeName } from "./gameport";
import { setPageHref } from "./runtime";
import { openTree } from "./treesource";

/** page → worker: first the tree (and the page's URL), then a port for each ask of the frame's */
export type TreeWorkerAsk = { tree: string; page: string } | { port: MessagePort };

const scope = self as unknown as {
  onmessage: ((e: MessageEvent) => void) | null;
};

let tree: ReturnType<typeof openTree> | undefined;
scope.onmessage = (e: MessageEvent) => {
  const m = e.data as Partial<{ tree: unknown; page: unknown; port: unknown }> | null;
  if (!tree && isTreeName(m?.tree) && typeof m?.page === "string") {
    setPageHref(m.page);
    tree = openTree(m.tree, m.page);
    tree.catch((err: unknown) => console.error("masu: the game's file list:", err));
  } else if (tree && m?.port instanceof MessagePort) {
    const port = m.port;
    void tree.then((t) => answerReads(port, t)).catch(() => port.close());
  }
};
