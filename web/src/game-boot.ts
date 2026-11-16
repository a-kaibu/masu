// The first page of a web game's frame (game-boot.html on masu serve's game listener, opened by
// html.ts as game-boot.html?to=/tree/<tree>/<page>): registers the frame origin's service
// worker (game-sw.ts), has the app's page give it the port it reads the tree's files over
// (gameport.ts), then goes on to the game's own page, which the worker answers.
import { isTreeName, relayPorts, treePath, type WantPort } from "./gameport";

/** How long the app's page and the worker may take to set up the reads. */
const READY_MS = 20_000;

async function boot(): Promise<void> {
  const to = new URLSearchParams(location.search).get("to") ?? "";
  const target = new URL(to, location.href);
  const at = treePath(target.pathname);
  if (target.origin !== location.origin || !at || !isTreeName(at.tree))
    throw new Error(`not a game's page: ${to}`);
  const sw = navigator.serviceWorker;
  if (!sw)
    throw new Error("this browser has no service workers here (an http origin that is not this machine?)");
  const ok = new Promise<void>((resolve) => relayPorts((tree) => tree === at.tree && resolve()));
  await sw.register("/game-sw.js", { scope: "/", updateViaCache: "none" });
  await sw.ready;
  parent.postMessage({ masu: "want-port", tree: at.tree } satisfies WantPort, "*");
  let timer = 0;
  await Promise.race([
    ok,
    new Promise(
      (_, reject) => (timer = setTimeout(() => reject(new Error("the app's page gave no port")), READY_MS)),
    ),
  ]);
  clearTimeout(timer);
  location.replace(target.href);
}

boot().catch((e: unknown) => {
  console.error("masu:", e);
  document.body.textContent = `ゲームを開けません: ${String(e)}`;
});
