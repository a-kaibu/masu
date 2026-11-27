// The Emscripten caller's lexical scope for tests with controlled network and storage.
export const plainCore = (core) => core.replace(/^export /gm, "");

export const bridgeScript = (core, bridge) =>
  `${plainCore(core)}
${bridge.replace(/^(?:import |export \{).*\n/gm, "")}`;

/** Routes serving the actual module entry and its imports in the browser tests. */
export async function workerModules(read, bridge = (source) => source) {
  const files = ["net-worker.mjs", "fetch-bridge.js", "net-core.mjs"];
  return new Map(await Promise.all(files.map(async (name) => {
    const source = await read(new URL(name, import.meta.url), "utf8");
    return [`/sake/${name}`, name === "fetch-bridge.js" ? bridge(source) : source];
  })));
}
