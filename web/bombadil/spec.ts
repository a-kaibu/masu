import { always, eventually, next } from "@antithesishq/bombadil";
import { actions, extract, getFingerprint, type ActionTemplate } from "@antithesishq/bombadil/browser";
export { noUncaughtExceptions, noUnhandledPromiseRejections } from "@antithesishq/bombadil/browser/defaults";

const controls = extract(({ document }) => {
  const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>("button"));
  return buttons.flatMap((button) => {
    if (button.disabled) return [];
    const label = button.getAttribute("aria-label") ?? button.textContent?.trim() ?? "";
    if (
      !label.endsWith("の情報") &&
      !label.endsWith("を起動") &&
      !/^(サイドバー|閉じる|Close|詳細|保存|起動|端末に保存|端末から削除|保存せずに遊ぶ|一覧に戻る|お気に入り|お気に入りに追加|遊ぶ|この回線で保存する|続きから保存|メモを保存|メモ)$/.test(
        label,
      )
    )
      return [];
    const rect = button.getBoundingClientRect();
    const point = { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    if (!button.contains(document.elementFromPoint(point.x, point.y))) return [];
    return [
      {
        point,
        fingerprint: getFingerprint(button),
        launches: label.endsWith("を起動") || /^(起動|遊ぶ|保存せずに遊ぶ)$/.test(label),
      },
    ];
  });
});

const screen = extract(({ document, window }) => {
  const info = document.querySelector('[data-testid="game-info"]');
  const first = document.querySelector('[data-testid="save-first"]');
  const rect = info?.closest('[role="dialog"]')?.getBoundingClientRect();
  const scroll = rect ? { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 } : null;
  const progress = (first ?? info)?.querySelector('[role="progressbar"][aria-label="保存の進み"]');
  const now = progress?.getAttribute("aria-valuenow");
  return {
    game: new URL(window.location.href).searchParams.get("game"),
    cards: document.querySelectorAll("li[data-game]").length,
    scroll,
    returning: Array.from(document.querySelectorAll("button")).some((b) =>
      b.textContent?.includes("一覧に戻る"),
    ),
    saving: now === null || now === undefined ? null : Number(now),
    info: info?.getAttribute("data-game-id") ?? null,
    first: first?.getAttribute("data-game-id") ?? null,
    serverErrors: window.performance
      .getEntriesByType("resource")
      .filter((entry) => entry instanceof PerformanceResourceTiming && entry.responseStatus >= 500).length,
  };
});

// Each sake session reserves gigabytes of address space. Leave exploration time for the
// library and the device store without launching a new Wasm instance on every turn.
let launches = 0;
let lastGame: string | null = null;
export const explore = actions(() => {
  const current = screen.current;
  const steps: ActionTemplate[] = ["Wait"];
  if (current.game !== null) {
    if (lastGame !== current.game) launches += 1;
    steps.push("Back");
  }
  lastGame = current.game;
  if (current.scroll) {
    steps.push(
      { ScrollDown: { origin: current.scroll, distance: 420 } },
      { ScrollUp: { origin: current.scroll, distance: 420 } },
    );
  }
  for (const { point, fingerprint, launches: startsGame } of controls.current) {
    if (startsGame && launches >= 6) continue;
    steps.push({
      Click: {
        point,
        fingerprint,
      },
    });
  }
  return steps;
});

export const libraryLoads = eventually(() => screen.current.cards === 2).within(10, "seconds");
export const noServerErrors = always(() => screen.current.serverErrors === 0);
export const progressesWithinRange = always(() => {
  const { saving } = screen.current;
  return saving === null || (saving >= 0 && saving <= 100);
});

export const saveProgressDoesNotGoBack = always(() => {
  const { saving, info, first } = screen.current;
  if (saving === null) return true;
  return next(() => {
    const after = screen.current;
    return info !== after.info || first !== after.first || after.saving === null || after.saving >= saving;
  });
});

export const libraryHasNoRunningGame = always(() => {
  const { cards, game, returning } = screen.current;
  return cards === 0 || game !== null || !returning;
});
