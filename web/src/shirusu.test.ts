import { expect, test } from "vite-plus/test";
import { ShirusuLines } from "./shirusu";

test("ShirusuLines: every line kept in order across drains, others left alone, the overflow counted (property, seeded)", () => {
  let x = 7;
  const rnd = (n: number) => ((x = (Math.imul(x, 1103515245) + 12345) >>> 0) >>> 8) % n;
  for (let run = 0; run < 2000; run++) {
    const max = 20 + rnd(400);
    const s = new ShirusuLines(max);
    let want: string[] = [];
    let chars = 0;
    let dropped = 0;
    const got: string[] = [];
    const flush = () => {
      if (dropped) want.push(`shirusu X dropped ${dropped}`);
      const d = s.drain();
      got.push(...(d ? d.split("\n") : []));
      expect(d === "" ? [] : d.split("\n")).toEqual(want);
      want = [];
      chars = 0;
      dropped = 0;
    };
    for (let i = 0; i < 60; i++) {
      if (rnd(10) === 0) flush();
      const ours = rnd(4) !== 0;
      const line = (ours ? "shirusu E " : "sake: ") + "x".repeat(rnd(40));
      expect(s.take(line)).toBe(ours);
      if (!ours) continue;
      if (chars + line.length > max) dropped++;
      else {
        want.push(line);
        chars += line.length;
      }
    }
    flush();
    expect(s.drain()).toBe("");
    expect(got.every((l) => l.startsWith("shirusu "))).toBe(true);
  }
});
