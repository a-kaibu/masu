// A game's record on the server (favorite and note) and what the player is doing to it
// (review 22bb061 §6): the server's copy, the note being typed (a draft of its own) and the
// favorite flag on its way. A failed favorite takes back the flag only; a note typed meanwhile
// stays, whatever comes back when.
import type { Profile } from "./library-data";

export interface ProfileModel {
  /** the server's copy (null: not loaded) */
  readonly saved: Profile | null;
  /** the note as typed, when it differs from the saved one's start (null: none typed) */
  readonly draft: string | null;
  /** the favorite flag asked for and not yet answered */
  readonly favorite: boolean | null;
  /** the note sent and not yet answered */
  readonly sending: string | null;
}

export type ProfileAction =
  | { readonly kind: "loaded"; readonly profile: Profile }
  | { readonly kind: "typed"; readonly note: string }
  | { readonly kind: "favorite-asked"; readonly on: boolean }
  | { readonly kind: "favorite-done"; readonly ok: boolean; readonly on: boolean }
  | { readonly kind: "note-sent"; readonly note: string }
  | { readonly kind: "note-done"; readonly ok: boolean; readonly note: string };

export const emptyProfile: ProfileModel = { saved: null, draft: null, favorite: null, sending: null };

export function profileStep(m: ProfileModel, a: ProfileAction): ProfileModel {
  switch (a.kind) {
    case "loaded":
      return { ...m, saved: a.profile };
    case "typed":
      return m.saved ? { ...m, draft: a.note === m.saved.note ? null : a.note } : m;
    case "favorite-asked":
      return { ...m, favorite: a.on };
    case "favorite-done":
      // (only the flag moves: the note saved and the note typed stay as they are)
      if (!m.saved) return m;
      return {
        ...m,
        favorite: m.favorite === a.on ? null : m.favorite,
        saved: a.ok ? { ...m.saved, favorite: a.on } : m.saved,
      };
    case "note-sent":
      return { ...m, sending: a.note };
    case "note-done": {
      if (!m.saved) return m;
      const saved = a.ok ? { ...m.saved, note: a.note } : m.saved;
      // (the draft stays when typed on after what was sent)
      const draft = m.draft === null || m.draft === saved.note ? null : m.draft;
      return { ...m, saved, draft, sending: m.sending === a.note ? null : m.sending };
    }
  }
}

/** What the page shows: the saved record with the flag asked for and the note typed. */
export function shown(m: ProfileModel): Profile | null {
  if (!m.saved) return null;
  return { favorite: m.favorite ?? m.saved.favorite, note: m.draft ?? m.saved.note };
}

/** What a PUT of the favorite sends: the saved record with the flag (never the draft). */
export const favoriteBody = (m: ProfileModel, on: boolean): Profile | null =>
  m.saved ? { ...m.saved, favorite: on } : null;

/** What a PUT of the note sends: the saved record with the draft (the flag as saved). */
export const noteBody = (m: ProfileModel): Profile | null =>
  m.saved && m.draft !== null ? { ...m.saved, note: m.draft } : null;
