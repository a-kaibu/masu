// The game page's status line (ui/Shell.tsx), apart from React: play.ts and html.ts say it.

/** The status line (review 22bb061 §6): what the page is doing, as a phase the view picks its
 * look from (the spinner while working), not words it guesses that from. */
export type Status =
  | { phase: "none" }
  /** a step of starting or leaving a game */
  | { phase: "working"; text: string }
  /** said, with nothing going on (a save taken over, a warning) */
  | { phase: "notice"; text: string }
  /** the run could not start or went wrong */
  | { phase: "error"; error: string };

export const NO_STATUS: Status = { phase: "none" };

/** The line's words, as the view shows them. */
export function statusText(s: Status): string {
  switch (s.phase) {
    case "none":
      return "";
    case "working":
    case "notice":
      return s.text;
    case "error":
      return `エラー: ${s.error}`;
  }
}
