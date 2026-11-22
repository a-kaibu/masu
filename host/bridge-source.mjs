// fetch-bridge.js as sake.js has it: host/net-core.mjs, its `export`s taken off, before it (as
// crates/sake/build.sh puts them). For the tests that run the network worker from source.

/** net-core.mjs as a plain script (crates/sake/build.sh does the same with sed). */
export const plainCore = (core) => core.replace(/^export /gm, "");

/** The two files' text, as one script. */
export const bridgeScript = (core, bridge) => `${plainCore(core)}\n${bridge}`;
