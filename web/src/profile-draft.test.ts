import { expect, test } from "vite-plus/test";
import { emptyProfile, favoriteBody, noteBody, profileStep, shown, type ProfileModel } from "./profile-draft";

function rng(seed: number) {
  let s = seed >>> 0;
  return (n: number) => Math.floor(((s = (Math.imul(s, 1664525) + 1013904223) >>> 0) / 0x100000000) * n);
}

const loaded = profileStep(emptyProfile, { kind: "loaded", profile: { favorite: false, note: "old" } });

test("the favorite's PUT failing while a note was typed takes back the flag only (review 22bb061 §6)", () => {
  let m = profileStep(loaded, { kind: "favorite-asked", on: true });
  expect(favoriteBody(m, true)).toEqual({ favorite: true, note: "old" });
  m = profileStep(m, { kind: "typed", note: "new text" });
  expect(shown(m)).toEqual({ favorite: true, note: "new text" });
  m = profileStep(m, { kind: "favorite-done", ok: false, on: true });
  expect(shown(m)).toEqual({ favorite: false, note: "new text" });
  expect(noteBody(m)).toEqual({ favorite: false, note: "new text" });
});

test("any order of typing, flags, sends and answers: what was typed last is shown until it is saved (seeded property)", () => {
  for (let seed = 1; seed <= 2000; seed++) {
    const r = rng(seed);
    let m: ProfileModel = loaded;
    let typed: string | null = null;
    const pending: ({ kind: "fav"; on: boolean } | { kind: "note"; note: string })[] = [];
    for (let i = 0; i < 30; i++) {
      const what = r(5);
      if (what === 0) {
        typed = `t${seed}-${i}`;
        m = profileStep(m, { kind: "typed", note: typed });
      } else if (what === 1) {
        const on = !shown(m)!.favorite;
        m = profileStep(m, { kind: "favorite-asked", on });
        pending.push({ kind: "fav", on });
      } else if (what === 2 && noteBody(m)) {
        const note = noteBody(m)!.note;
        m = profileStep(m, { kind: "note-sent", note });
        pending.push({ kind: "note", note });
      } else if (pending.length) {
        const p = pending.splice(r(pending.length), 1)[0]!;
        const ok = r(3) !== 0;
        m =
          p.kind === "fav"
            ? profileStep(m, { kind: "favorite-done", ok, on: p.on })
            : profileStep(m, { kind: "note-done", ok, note: p.note });
        if (p.kind === "note" && ok && p.note === typed) typed = null;
      }
      // (the note shown is the last typed, or the saved one once that was saved)
      const note = shown(m)!.note;
      if (typed !== null) expect(note, `seed ${seed}`).toBe(typed);
      else expect(note).toBe(m.saved!.note);
    }
  }
});

test("a note saved while typing went on keeps what was typed after", () => {
  let m = profileStep(loaded, { kind: "typed", note: "a" });
  m = profileStep(m, { kind: "note-sent", note: "a" });
  m = profileStep(m, { kind: "typed", note: "ab" });
  m = profileStep(m, { kind: "note-done", ok: true, note: "a" });
  expect(m.saved).toEqual({ favorite: false, note: "a" });
  expect(shown(m)!.note).toBe("ab");
  expect(noteBody(m)).toEqual({ favorite: false, note: "ab" });
});
