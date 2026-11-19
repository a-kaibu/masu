import { describe, expect, it, test } from "vite-plus/test";
import { blake3Hex } from "./blake3";
import { chunkFileName, opfsStore, type ChunkStore } from "./chunkstore";
import { buildIdOf, runtimeWasm, sakeWasmBytes, wasmModuleArg, type WasmFrom, type WasmSourceDeps } from "./wasmsource";

type B = Uint8Array<ArrayBuffer>;
const urlOf = (i: RequestInfo | URL) => (typeof i === "string" ? i : i instanceof URL ? i.href : i.url);
const unhex = (h: string) => Uint8Array.from(h.match(/../g) ?? [], (x) => parseInt(x, 16));
const toHex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
const STORE = "https://w2.example/store/";
const URL_ = "https://w2.example/sake/sake.wasm?v=x";
// the smallest wasm module (magic, version), then a custom section of `n` bytes so that it has a body
const wasmOf = (n: number): B => {
  const body = new Uint8Array(n).map((_, i) => (i * 31) & 0xff);
  const name = new TextEncoder().encode("x");
  const size = 1 + name.length + n;
  const leb = (v: number) => {
    const out: number[] = [];
    do {
      let b = v & 0x7f;
      v >>>= 7;
      if (v) b |= 0x80;
      out.push(b);
    } while (v);
    return out;
  };
  return new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0, 0, ...leb(size), name.length, ...name, ...body]);
};

/** envelope.rs's sealing: nonce + AES-256-GCM, the object's UUID as associated data. */
async function seal(key: B, object: string, plain: B): Promise<B> {
  const nonce = crypto.getRandomValues(new Uint8Array(12));
  const k = await crypto.subtle.importKey("raw", key, "AES-GCM", false, ["encrypt"]);
  const c = new Uint8Array(
    await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: nonce, additionalData: unhex(object.replace(/-/g, "")) },
      k,
      plain,
    ),
  );
  const out = new Uint8Array(12 + c.length);
  out.set(nonce);
  out.set(c, 12);
  return out;
}

function memoryStore(): ChunkStore & { files: Map<string, B> } {
  const files = new Map<string, B>();
  return {
    files,
    async match(k) {
      const b = files.get(urlOf(k as string));
      return b ? new Response(b) : undefined;
    },
    async put(k, r) {
      files.set(urlOf(k as string), new Uint8Array(await r.arrayBuffer()));
    },
    async delete(k) {
      return files.delete(urlOf(k as string));
    },
    async keys() {
      return [...files.keys()].map((url) => ({ url }));
    },
    async list() {
      return [...files].map(([url, b], time) => ({ url, size: b.length, time }));
    },
  };
}

/** A server and bucket: what each has, and the requests they saw. */
function world(o: {
  httpCache?: boolean;
  sakeDirect?: number | "bad" | "slow";
  bucket?: "ok" | "fail" | "tampered";
}) {
  const wasm = wasmOf(5000);
  const hex = blake3Hex(wasm);
  const key = crypto.getRandomValues(new Uint8Array(32));
  const object = crypto.randomUUID();
  const seen: string[] = [];
  // (the "zstd" frame: the bytes themselves, unzstd the identity)
  const sealed = seal(key, object, wasm);
  const request = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const u = urlOf(input);
    seen.push(`${init?.cache ?? ""} ${u}`);
    if (u === URL_) {
      if (init?.cache === "only-if-cached") {
        if (o.httpCache) return new Response(wasm);
        throw new TypeError("not cached");
      }
      return new Response(wasm);
    }
    if (u.startsWith("https://w2.example/sake-direct/")) {
      if (o.sakeDirect === "slow")
        return new Promise<Response>((_, no) =>
          init?.signal?.addEventListener("abort", () => no(new DOMException("t", "TimeoutError"))),
        );
      if (o.sakeDirect === "bad") return Response.json({ url: "x", key: "zz", object, chunk: hex });
      if (typeof o.sakeDirect === "number") return new Response("", { status: o.sakeDirect });
      return Response.json({ build: await buildIdOf(wasm), url: "https://bucket.example/o", key: toHex(key), object, chunk: hex });
    }
    if (u === "https://bucket.example/o") {
      if (o.bucket === "fail") return new Response("", { status: 403 });
      const c = await sealed;
      if (o.bucket === "tampered") {
        const t = new Uint8Array(c);
        t[40] = t[40]! ^ 1;
        return new Response(t);
      }
      return new Response(c);
    }
    return new Response("", { status: 404 });
  }) as typeof fetch;
  return { wasm, hex, request, seen };
}

const identity = async (b: Uint8Array) => b;

describe("sakeWasmBytes", () => {
  it("leaves a file the browser kept to emscripten (its compiled code comes with it)", async () => {
    const w = world({ httpCache: true });
    const r = await sakeWasmBytes("sake.wasm", URL_, STORE, { request: w.request, unzstd: identity });
    expect(r).toEqual({ bytes: null, from: "http-cache" });
    expect(w.seen.some((s) => s.includes("sake-direct"))).toBe(false);
  });

  it("fetches it from the bucket, checks it, keeps it on the device; the next start reads the device", async () => {
    const w = world({});
    const device = memoryStore();
    const lines: string[] = [];
    const r = await sakeWasmBytes("sake.wasm", URL_, STORE, {
      request: w.request,
      unzstd: identity,
      device: async () => device,
      note: (l) => lines.push(l),
    });
    expect(r.from).toBe("bucket");
    expect(r.bytes).toEqual(w.wasm);
    expect(lines.join("\n")).toMatch(/from the bucket/);
    // (the server's file itself was not fetched)
    expect(w.seen.filter((s) => s.endsWith(URL_) && !s.startsWith("only-if-cached"))).toEqual([]);
    await new Promise((r) => setTimeout(r, 0));
    expect(device.files.get(`${STORE}chunks/${w.hex.slice(0, 2)}/${w.hex}`)).toEqual(w.wasm);
    const w2 = world({});
    // (the same chunk name: what the server says now)
    const again = await sakeWasmBytes("sake.wasm", URL_, STORE, {
      request: async (i, n) =>
        urlOf(i).includes("sake-direct")
          ? Response.json({ ...(await (await w.request(i, n)).json()) })
          : w2.request(i, n),
      unzstd: identity,
      device: async () => device,
    });
    expect(again.from).toBe("device");
    expect(again.bytes).toEqual(w.wasm);
    expect(w2.seen.some((s) => s.includes("bucket.example"))).toBe(false);
  });

  it("keeps the checked wasm in OPFS: the next start reads it from there (2026-11-14 review)", async () => {
    const files = new Map<string, B>();
    const dir = {
      async getFileHandle(name: string, o: { create?: boolean } = {}) {
        if (!files.has(name)) {
          if (!o.create) throw new DOMException("no", "NotFoundError");
          files.set(name, new Uint8Array(0));
        }
        return {
          getFile: async () => new File([files.get(name)!.slice()], name),
          createWritable: async () => {
            let d = new Uint8Array(0);
            return {
              write: async (x: B) => void (d = x.slice()),
              close: async () => void files.set(name, d),
              abort: async () => {},
            };
          },
        };
      },
      async removeEntry(name: string) {
        files.delete(name);
      },
      async *keys() {
        yield* files.keys();
      },
    };
    const device = opfsStore(dir as unknown as FileSystemDirectoryHandle);
    const w = world({});
    const r = await sakeWasmBytes("sake.wasm", URL_, STORE, {
      request: w.request,
      unzstd: identity,
      device: async () => device,
    });
    expect(r.from).toBe("bucket");
    await new Promise((r) => setTimeout(r, 10));
    const key = `${STORE}chunks/${w.hex.slice(0, 2)}/${w.hex}`;
    expect(files.get(chunkFileName(key))).toEqual(w.wasm);
    const w2 = world({});
    const again = await sakeWasmBytes("sake.wasm", URL_, STORE, {
      request: async (i, n) => (urlOf(i).includes("sake-direct") ? w.request(i, n) : w2.request(i, n)),
      unzstd: identity,
      device: async () => opfsStore(dir as unknown as FileSystemDirectoryHandle),
    });
    expect(again.from).toBe("device");
    expect(w2.seen.some((s) => s.includes("bucket.example"))).toBe(false);
  });

  it("falls back to the server's file: an older server, no S3, a bad answer, a slow server, a failing or tampered bucket", async () => {
    const cases: Parameters<typeof world>[0][] = [
      { sakeDirect: 404 },
      { sakeDirect: 501 },
      { sakeDirect: 502 },
      { sakeDirect: "bad" },
      { sakeDirect: "slow" },
      { bucket: "fail" },
      { bucket: "tampered" },
    ];
    for (const c of cases) {
      const w = world(c);
      const device = memoryStore();
      const r = await sakeWasmBytes("sake.wasm", URL_, STORE, {
        request: w.request,
        unzstd: identity,
        device: async () => device,
        askMs: 50,
      });
      expect(r, JSON.stringify(c)).toEqual({ bytes: null, from: "server" });
      // (nothing unchecked kept)
      expect(device.files.size, JSON.stringify(c)).toBe(0);
    }
  }, 20000);

  it("picks the first source that has it, in order (seeded property)", async () => {
    let seed = 11;
    const r = () => (seed = (seed * 1103515245 + 12345) >>> 0) / 2 ** 32;
    for (let i = 0; i < 60; i++) {
      const has = { http: r() < 0.3, answer: r() < 0.7, device: r() < 0.4, bucket: r() < 0.6 };
      const w = world({
        httpCache: has.http,
        sakeDirect: has.answer ? undefined : 404,
        bucket: has.bucket ? "ok" : "fail",
      });
      const device = memoryStore();
      if (has.device) device.files.set(`${STORE}chunks/${w.hex.slice(0, 2)}/${w.hex}`, w.wasm);
      const got = await sakeWasmBytes("sake.wasm", URL_, STORE, {
        request: w.request,
        unzstd: identity,
        device: async () => device,
      });
      const want: WasmFrom = has.http
        ? "http-cache"
        : !has.answer
          ? "server"
          : has.device
            ? "device"
            : has.bucket
              ? "bucket"
              : "server";
      expect(got.from, JSON.stringify(has)).toBe(want);
      if (want === "device" || want === "bucket") expect(got.bytes).toEqual(w.wasm);
      else expect(got.bytes).toBeNull();
    }
  }, 30000);
});

describe("wasmModuleArg", () => {
  it("compiles the bytes once; instantiateWasm hands emscripten the instance and module", async () => {
    const wasm = wasmOf(100);
    const lines: string[] = [];
    const arg = await wasmModuleArg(Promise.resolve({ bytes: wasm, from: "bucket" }), (l) => lines.push(l));
    expect(arg.instantiateWasm).toBeTypeOf("function");
    const got = await new Promise<[WebAssembly.Instance, WebAssembly.Module]>((ok) => {
      expect(arg.instantiateWasm!({}, (i, m) => ok([i, m]))).toEqual({});
    });
    expect(got[0]).toBeInstanceOf(WebAssembly.Instance);
    expect(WebAssembly.Module.customSections(got[1], "x")[0]!.byteLength).toBe(100);
    expect(lines).toEqual(["sake wasm: bucket"]);
  });

  it("leaves emscripten to load the URL for no bytes or bytes that do not compile", async () => {
    expect(await wasmModuleArg(Promise.resolve({ bytes: null, from: "http-cache" }))).toEqual({});
    const lines: string[] = [];
    const bad = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);
    expect(
      await wasmModuleArg(Promise.resolve({ bytes: bad, from: "device" }), (l) => lines.push(l)),
    ).toEqual({});
    expect(lines[1]).toMatch(/did not compile/);
  });
});

test("an instance that cannot be made is told (the factory would wait for good otherwise)", async () => {
  // (a module importing a function "m"."f": given none, instantiate rejects)
  const bytes = new Uint8Array([
    0, 0x61, 0x73, 0x6d, 1, 0, 0, 0, 1, 4, 1, 0x60, 0, 0, 2, 7, 1, 1, 0x6d, 1, 0x66, 0, 0,
  ]);
  const notes: string[] = [];
  let failed: unknown = null;
  const arg = await wasmModuleArg(
    Promise.resolve({ bytes, from: "device" as const }),
    (l) => notes.push(l),
    (e) => (failed = e),
  );
  let done = false;
  arg.instantiateWasm!({}, () => (done = true));
  await new Promise((r) => setTimeout(r, 20));
  expect(done).toBe(false);
  expect(failed).toBeInstanceOf(Error);
  expect(notes.some((n) => n.includes("did not start"))).toBe(true);
  // (the imports given: it starts)
  let started = false;
  arg.instantiateWasm!({ m: { f: () => {} } }, () => (started = true));
  await new Promise((r) => setTimeout(r, 20));
  expect(started).toBe(true);
});

describe("sakeWasmBytes from the release's CDN (GitHub Pages)", () => {
  const CDN = "https://owner.github.io/masu/";
  /** world's server and bucket, and a CDN that has `cdnBytes` (none: 404) under build `id`. */
  async function withCdn(
    o: Parameters<typeof world>[0] & { cdn?: "ok" | 404 | "slow" | "other-build" | "bad-zstd" },
  ) {
    const w = world(o);
    const id = await buildIdOf(w.wasm);
    const other = wasmOf(4000);
    const request = (async (input: RequestInfo | URL, init?: RequestInit) => {
      const u = urlOf(input);
      if (u.startsWith(CDN)) {
        w.seen.push(`${init?.cache ?? ""} ${init?.mode ?? ""} ${u}`);
        if (u !== `${CDN}sake/${id}/sake.wasm.zst`) return new Response("", { status: 404 });
        switch (o.cdn ?? "ok") {
          case "ok":
            return new Response(w.wasm);
          case 404:
            return new Response("", { status: 404 });
          case "slow":
            return new Promise<Response>((_, no) =>
              init?.signal?.addEventListener("abort", () => no(new DOMException("t", "TimeoutError"))),
            );
          case "other-build":
            return new Response(other);
          case "bad-zstd":
            return new Response(new Uint8Array([1, 2, 3]));
        }
      }
      return w.request(input, init);
    }) as typeof fetch;
    // (the "zstd" frame: the bytes themselves; [1,2,3] is not a frame)
    const unzstd = async (b: Uint8Array) => {
      if (b.length === 3) throw new Error("not zstd");
      return b;
    };
    return { ...w, id, request, unzstd };
  }

  it("names a build as build.sh does: the first 16 hex digits of the SHA-256", async () => {
    // (sha256("abc") = ba7816bf8f01cfea414140de5dae2223...)
    expect(await buildIdOf(new TextEncoder().encode("abc") as B)).toBe("ba7816bf8f01cfea");
  });

  it("takes the CDN's file of this build before the server or the bucket, cached for good, CORS", async () => {
    const w = await withCdn({});
    const lines: string[] = [];
    const r = await sakeWasmBytes("sake.wasm", URL_, STORE, {
      request: w.request,
      unzstd: w.unzstd,
      cdn: CDN,
      buildId: async () => w.id,
      note: (l) => lines.push(l),
    });
    expect(r.from).toBe("cdn");
    expect(r.bytes).toEqual(w.wasm);
    expect(w.seen.filter((s) => s.includes(CDN))).toEqual([
      `force-cache cors ${CDN}sake/${w.id}/sake.wasm.zst`,
    ]);
    // (no /sake-direct, no bucket, no server file)
    expect(w.seen.some((s) => s.includes("sake-direct") || s.includes("bucket.example"))).toBe(false);
    expect(w.seen.filter((s) => s.endsWith(URL_) && !s.startsWith("only-if-cached"))).toEqual([]);
    expect(lines.join("\n")).toContain(`from ${CDN}`);
  });

  it("still leaves a file the browser kept to emscripten first", async () => {
    const w = await withCdn({ httpCache: true });
    const r = await sakeWasmBytes("sake.wasm", URL_, STORE, {
      request: w.request,
      unzstd: w.unzstd,
      cdn: CDN,
      buildId: async () => w.id,
    });
    expect(r).toEqual({ bytes: w.wasm, from: "http-cache" });
    expect(w.seen.some((s) => s.includes(CDN))).toBe(false);
  });

  it("goes on to the bucket when the CDN lacks it, is slow, has another build or a bad frame", async () => {
    for (const cdn of [404, "slow", "other-build", "bad-zstd"] as const) {
      const w = await withCdn({ cdn });
      const r = await sakeWasmBytes("sake.wasm", URL_, STORE, {
        request: w.request,
        unzstd: w.unzstd,
        cdn: CDN,
        buildId: async () => w.id,
        cdnMs: 50,
      });
      expect(r.from, String(cdn)).toBe("bucket");
      expect(r.bytes, String(cdn)).toEqual(w.wasm);
    }
  });

  it("asks no CDN without one, without a build id, or with a malformed one", async () => {
    const cases: Partial<WasmSourceDeps>[] = [
      { cdn: "", buildId: async () => "0123456789abcdef" },
      { cdn: CDN },
      { cdn: CDN, buildId: async () => undefined },
      { cdn: CDN, buildId: async () => "../../evil" },
      { cdn: CDN, buildId: async () => "0123456789ABCDEF" },
      { cdn: CDN, buildId: () => Promise.reject(new Error("no build-id.txt")) },
      { cdn: "javascript:alert(1)//", buildId: async () => "0123456789abcdef" },
      { cdn: "https://owner.github.io/masu", buildId: async () => "0123456789abcdef" },
    ];
    for (const c of cases) {
      const w = await withCdn({});
      const r = await sakeWasmBytes("sake.wasm", URL_, STORE, { request: w.request, unzstd: w.unzstd, ...c });
      expect(r.from, JSON.stringify(c)).toBe(c.buildId ? "server" : "bucket");
      expect(
        w.seen.some((s) => s.includes("github.io") || s.includes("javascript:")),
        JSON.stringify(c),
      ).toBe(false);
    }
  });

  it("picks the first source that has it, the CDN second (seeded property)", async () => {
    let seed = 7;
    const r = () => (seed = (seed * 1103515245 + 12345) >>> 0) / 2 ** 32;
    for (let i = 0; i < 80; i++) {
      const has = {
        http: r() < 0.3,
        cdn: r() < 0.5,
        answer: r() < 0.7,
        device: r() < 0.4,
        bucket: r() < 0.6,
      };
      const w = await withCdn({
        httpCache: has.http,
        cdn: has.cdn ? "ok" : 404,
        sakeDirect: has.answer ? undefined : 404,
        bucket: has.bucket ? "ok" : "fail",
      });
      const device = memoryStore();
      if (has.device) device.files.set(`${STORE}chunks/${w.hex.slice(0, 2)}/${w.hex}`, w.wasm);
      const got = await sakeWasmBytes("sake.wasm", URL_, STORE, {
        request: w.request,
        unzstd: w.unzstd,
        device: async () => device,
        cdn: CDN,
        buildId: async () => w.id,
      });
      const want: WasmFrom = has.http
        ? "http-cache"
        : has.cdn
          ? "cdn"
          : !has.answer
            ? "server"
            : has.device
              ? "device"
              : has.bucket
                ? "bucket"
                : "server";
      expect(got.from, JSON.stringify(has)).toBe(want);
      if (want === "server") expect(got.bytes).toBeNull();
      else expect(got.bytes).toEqual(w.wasm);
    }
  }, 20000);
});


it("rejects another build even when HTTP is the only available source", async () => {
  const old = wasmOf(500);
  const current = wasmOf(600);
  const id = await buildIdOf(old);
  for (const serverOnly of [false, true]) {
    const request = async () => new Response(current);
    await expect(runtimeWasm("sake.wasm", "/sake/sake.wasm", "https://local/store/", id, serverOnly, { request }))
      .rejects.toThrow("expected build");
  }
});
