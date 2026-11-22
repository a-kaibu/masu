import { expect, test } from "vite-plus/test";
import bridgeJs from "../../host/fetch-bridge.js?raw";
import core from "../../host/net-core.mjs?raw";
import { bridgeScript } from "../../host/bridge-source.mjs";
import { isDirectInfo, isPackSpan } from "./direct";

// (the network worker's functions, as its source has them: the same text as the page's import)
const worker = new Function(
  `${bridgeScript(core, bridgeJs)}\nreturn { masuDirectValid, masuPackValid, masuBackoff, MASU_RETRY_STATUS };`,
)() as {
  masuDirectValid(v: unknown): boolean;
  masuPackValid(v: unknown): boolean;
};

const key = "a".repeat(64);
const object = "0123abcd-0123-4567-89ab-0123456789ab";

test("an answer whose key or id is not a string is no answer, for the page and the worker alike (review 22bb061 §7)", () => {
  // (the worker's copy tested the regex only: an array's text matched it)
  for (const bad of [
    { url: "u", key: [key], object },
    { url: "u", key, object: [object] },
  ]) {
    expect(isDirectInfo(bad)).toBe(false);
    expect(worker.masuDirectValid(bad)).toBe(false);
  }
  const pack = { url: "u", id: [object], offset: 0, len: 28 };
  expect(isPackSpan(pack)).toBe(false);
  expect(worker.masuPackValid(pack)).toBe(false);
  expect(worker.masuDirectValid({ url: "u", key, object })).toBe(true);
});

test("property: the page and the worker judge any answer alike (seeded)", () => {
  let r = 11;
  const rnd = (n: number) => ((r = (r * 1103515245 + 12345) >>> 0) >>> 8) % n;
  const values = [key, object, "u", 0, 28, 27, -1, 1.5, null, undefined, [key], [object], {}, "", 2 ** 60];
  const pick = () => values[rnd(values.length)];
  for (let i = 0; i < 5000; i++) {
    const info = { url: pick(), key: pick(), object: pick() };
    const pack = { url: pick(), id: pick(), offset: pick(), len: pick() };
    expect(worker.masuDirectValid(info)).toBe(isDirectInfo(info));
    expect(worker.masuPackValid(pack)).toBe(isPackSpan(pack));
    const strict =
      typeof info.url === "string" &&
      typeof info.key === "string" &&
      /^[0-9a-f]{64}$/.test(info.key) &&
      typeof info.object === "string" &&
      /^[0-9a-f-]{36}$/.test(info.object);
    expect(isDirectInfo(info), JSON.stringify(info)).toBe(strict);
  }
});
