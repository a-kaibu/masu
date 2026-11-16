declare const process: { env: Record<string, string | undefined> }; // (node runs this file; no @types/node here)
import { build, defineConfig, type Plugin } from "vite-plus";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// The game listener's scripts (masu serve), each a classic script: frame.js, the script of web
// games' frames (src/frame.ts: it must run before the game's own scripts); game-boot.js, the
// frame's first page (public/game-boot.html), which registers game-sw.js, the frame origin's
// service worker that answers the game's files (src/game-sw.ts); frame-read.js, the frame's
// worker for its synchronous reads (src/frame-read.ts)
const GAME_SCRIPTS = [
  ["src/frame.ts", "masuFrame", "frame.js"],
  ["src/game-boot.ts", "masuGameBoot", "game-boot.js"],
  ["src/game-sw.ts", "masuGameSw", "game-sw.js"],
  ["src/frame-read.ts", "masuFrameRead", "frame-read.js"],
] as const;
const frame = (): Plugin => ({
  name: "masu-frame",
  apply: "build",
  async closeBundle() {
    for (const [entry, name, file] of GAME_SCRIPTS)
      await build({
        configFile: false,
        logLevel: "warn",
        build: {
          outDir: "dist",
          emptyOutDir: false,
          target: "es2022",
          lib: { entry, formats: ["iife"], name, fileName: () => file },
        },
      });
  },
});

// sw.js: the service worker (src/sw.ts, registered by src/pwa.ts), with the build's shell files
// (the pages, their assets, the manifest and icons; not sake/: that comes versioned, on first use
// or with 端末に保存) and an id from their contents
const PUBLIC_SHELL = [
  "manifest.webmanifest",
  "icons/favicon-48.png",
  "icons/icon-192.png",
  "icons/icon-512.png",
  "icons/maskable-512.png",
  "icons/apple-touch-icon.png",
];
const serviceWorker = (): Plugin => {
  const shell = new Map<string, string | Uint8Array>();
  return {
    name: "masu-sw",
    apply: "build",
    generateBundle(_, bundle) {
      for (const [name, out] of Object.entries(bundle))
        if (!/^(bench|game)\.html$/.test(name)) shell.set(name, out.type === "chunk" ? out.code : out.source);
    },
    async closeBundle() {
      const files = [...PUBLIC_SHELL, ...[...shell.keys()].sort()];
      const enc = new TextEncoder();
      const all = [...shell.keys()].sort().flatMap((k) => {
        const v = shell.get(k)!;
        return [enc.encode(k), typeof v === "string" ? enc.encode(v) : v];
      });
      const bytes = new Uint8Array(all.reduce((n, a) => n + a.length, 0));
      all.reduce((at, a) => (bytes.set(a, at), at + a.length), 0);
      const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
      const id = Array.from(digest.slice(0, 8), (b) => b.toString(16).padStart(2, "0")).join("");
      await build({
        configFile: false,
        logLevel: "warn",
        define: { MASU_BUILD: JSON.stringify(id), MASU_SHELL: JSON.stringify(["./", ...files]) },
        build: {
          outDir: "dist",
          emptyOutDir: false,
          target: "es2022",
          lib: { entry: "src/sw.ts", formats: ["iife"], name: "masuSw", fileName: () => "sw.js" },
        },
      });
    },
  };
};

// Built into web/dist and served by masu serve next to the store. The UI is React with
// Tailwind CSS and shadcn/ui (src/components/ui); sake's files (public/sake/) are copied as-is.
export default defineConfig({
  base: "./",
  plugins: [react(), tailwindcss(), frame(), serviceWorker()],
  resolve: { alias: { "@": new URL("./src", import.meta.url).pathname } },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    // index.html = play.html: sake (crates/sake, files in public/sake/), the game list
    // without ?run=;
    // bench.html: bench.exe on play.html with the native numbers next to it
    rollupOptions: {
      input: {
        main: "index.html",
        play: "play.html",
        bench: "bench.html",
        game: "game.html",
      },
    },
  },
  server: {
    headers: {
      "cross-origin-opener-policy": "same-origin",
      "cross-origin-embedder-policy": "require-corp",
    },
    proxy: { "/store": process.env.MASU_SERVER ?? "http://127.0.0.1:8080" },
  },
  // (oxlint's defaults plus the rules below, all at 0 today: a promise nobody awaits, a switch
  // that misses a case, `+` on mixed types, == and the like fail the check from now on)
  lint: {
    ignorePatterns: ["public/**"],
    options: { typeAware: true, typeCheck: true },
    rules: {
      "typescript/no-floating-promises": "error",
      "typescript/await-thenable": "error",
      "typescript/switch-exhaustiveness-check": "error",
      "typescript/restrict-plus-operands": "error",
      "typescript/restrict-template-expressions": "error",
      "typescript/no-base-to-string": "error",
      "typescript/only-throw-error": "error",
      "typescript/no-unsafe-enum-comparison": "error",
      "typescript/no-redundant-type-constituents": "error",
      "typescript/no-duplicate-type-constituents": "error",
      "typescript/no-explicit-any": "error",
      "typescript/require-array-sort-compare": "error",
      eqeqeq: "error",
      radix: "error",
      "no-fallthrough": "error",
      "default-case-last": "error",
      "no-self-compare": "error",
      "array-callback-return": "error",
      "no-constructor-return": "error",
      "no-useless-catch": "error",
    },
  },
  fmt: { printWidth: 110, ignorePatterns: ["public/**"] },
});
