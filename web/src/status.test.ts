import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vite-plus/test";
import { StatusLine } from "./ui/Shell";
import { NO_STATUS, statusText, type Status } from "./status";

const shown = (status: Status) => renderToStaticMarkup(createElement(StatusLine, { status }));
const spins = (status: Status) => shown(status).includes("animate-spin");

test("the spinner turns with the page working, whatever the words (review 22bb061 §6)", () => {
  // (each of these read wrong to the old /^エラー|ません/ guess)
  expect(spins({ phase: "notice", text: "以前のセーブを引き継ぎました (old)" })).toBe(false);
  expect(spins({ phase: "working", text: "保存されていません領域を読み込み中" })).toBe(true);
  expect(spins({ phase: "error", error: "HTTP 500" })).toBe(false);
  expect(shown({ phase: "error", error: "HTTP 500" })).toContain("エラー: HTTP 500");
  expect(shown(NO_STATUS)).toBe("");
});

test("property: any words in any phase: spinner exactly while working, the words shown as given (seeded)", () => {
  let r = 7;
  const rnd = (n: number) => ((r = (r * 1103515245 + 12345) >>> 0) >>> 8) % n;
  const pieces = ["エラー", "ません", "中", "読み込み", "起動", " ", "x", "：", "（", "）"];
  for (let i = 0; i < 1000; i++) {
    const text = Array.from({ length: rnd(6) }, () => pieces[rnd(pieces.length)]).join("");
    const phase = (["working", "notice", "error"] as const)[rnd(3)]!;
    const status: Status = phase === "error" ? { phase, error: text } : { phase, text };
    expect(spins(status), text).toBe(phase === "working");
    const span = shown(status).match(/<span class="truncate">(.*)<\/span>/)?.[1] ?? null;
    expect(span, text).toBe(statusText(status).replace(/&/g, "&amp;").replace(/</g, "&lt;"));
  }
});
