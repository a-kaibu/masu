import { expect, test } from "vite-plus/test";
import { Video, type VideoWasm } from "./video";
import type { Gate } from "./memgate";

/** kura's side of the slot loan (crates/kura/src/video.rs vnhost_video_begin / _ready). */
class FakeKura implements VideoWasm {
  wasmMemory = new WebAssembly.Memory({ initial: 4 });
  private top = 1024;
  ready: { id: number; buf: number; ok: number }[] = [];
  _malloc(n: number): number {
    const p = this.top;
    this.top += (n + 15) & ~15;
    return p;
  }
  _vnhost_video_doorbell(): number {
    return 512;
  }
  _vnhost_video_list(): number {
    return 0;
  }
  _vnhost_video_has(): number {
    return 0;
  }
  _vnhost_video_chunk(): number {
    return 0;
  }
  _vnhost_video_begin(): number {
    return this._malloc(64);
  }
  _vnhost_video_ready(id: number, buf: number, _layout: number, ok: number): void {
    this.ready.push({ id, buf, ok });
  }
  _vnhost_video_fail(): void {}
}

const open: Gate = { closed: false, enter() {}, leave() {}, run: (f: () => void) => f() } as unknown as Gate;

/** A decoded frame whose copy ends when `done` is called (or throws). */
function frame(fail = false) {
  let done = () => {};
  const closed = { n: 0 };
  const f = {
    format: "I420",
    allocationSize: () => 16,
    copyTo: () =>
      new Promise<PlaneLayout[]>((ok, no) => {
        done = () => (fail ? no(new Error("copy failed")) : ok([{ offset: 0, stride: 4 }]));
      }),
    close: () => void closed.n++,
  };
  return { f: f as unknown as VideoFrame, end: () => done(), closed };
}

function stream(id: number) {
  return {
    id,
    w: 4,
    h: 4,
    frames: 10,
    tracks: [{ out: new Map() }],
    want: 0,
    copying: new Set([0]),
    broken: false,
    held: { serve: {}, copy: {} },
  };
}

const how = [{ fmt: 0, opts: {}, need: 16 }];

type Copying = { streams: Map<number, unknown>; copy(...a: unknown[]): Promise<void> };

test("a copy's slot goes back to kura however it ends: copied, failed, or the stream closed meanwhile", async () => {
  for (const ending of ["copied", "failed", "closed", "broken"] as const) {
    const k = new FakeKura();
    const v = new Video(k, () => {}, open, false) as unknown as Copying;
    const s = stream(7);
    v.streams.set(7, s);
    const fr = frame(ending === "failed");
    const buf = k._vnhost_video_begin();
    const copying = v.copy(s, 0, [fr.f], how, buf);
    // (the guest closes the stream, or the decoder breaks, while the copy runs)
    if (ending === "closed") v.streams.delete(7);
    if (ending === "broken") s.broken = true;
    fr.end();
    await copying;
    // (exactly one answer for the slot, ok only for a frame copied into a stream still there)
    expect(k.ready, ending).toEqual([{ id: 7, buf, ok: ending === "copied" ? 1 : 0 }]);
    expect(fr.closed.n, ending).toBe(1);
    expect(s.copying.size, ending).toBe(0);
  }
});

test("property: any order of copies ending and closes gives each slot back once", async () => {
  for (let seed = 1; seed < 200; seed++) {
    let r = seed;
    const rnd = (n: number) => ((r = (r * 1103515245 + 12345) >>> 0) >>> 8) % n;
    const k = new FakeKura();
    const v = new Video(k, () => {}, open, false) as unknown as Copying;
    const s = stream(3);
    v.streams.set(3, s);
    const copies: { end: () => void; p: Promise<void>; buf: number }[] = [];
    for (let n = 0; n < 1 + rnd(4); n++) {
      const fr = frame(rnd(5) === 0);
      const buf = k._vnhost_video_begin();
      s.copying.add(n);
      copies.push({ end: fr.end, p: v.copy(s, n, [fr.f], how, buf), buf });
    }
    const closeAt = rnd(copies.length + 1);
    for (const [i, c] of copies.entries()) {
      if (i === closeAt) v.streams.delete(3);
      c.end();
      await c.p;
    }
    expect(
      k.ready.map((x) => x.buf).sort((a, b) => a - b),
      `seed ${seed}`,
    ).toEqual(copies.map((c) => c.buf).sort((a, b) => a - b));
  }
});
