// Set process-wide defaults before Node starts the test workers.
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const { values } = parseArgs({ options: { seed: { type: "string" } } });
const seed = values.seed ?? process.env.MASU_SIM_SEED;
if (seed !== undefined && (!/^\d+$/.test(seed) || Number(seed) > 0xffffffff)) {
  throw new Error("--seed must be an unsigned 32-bit decimal integer");
}
console.log("Simulation seed:", seed ?? "default corpus");
const result = spawnSync(
  process.execPath,
  [
    fileURLToPath(new URL("bin/vp", import.meta.resolve("vite-plus/package.json"))),
    "test",
    "run",
    "-t",
    "simulation:",
  ],
  {
    stdio: "inherit",
    env: {
      ...process.env,
      TZ: "UTC",
      LANG: "en_US.UTF-8",
      LC_ALL: "en_US.UTF-8",
      ...(seed === undefined ? {} : { MASU_SIM_SEED: seed }),
    },
  },
);
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
