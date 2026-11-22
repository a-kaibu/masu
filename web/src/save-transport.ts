import { same } from "./file-records";
/** The request headers naming a record of this browser (crates/masu/src/saves.rs): a PUT's
 * the record its bytes hold or supersede, a GET's the record asked about; the GET's answer
 * repeats it when an upload named it. */
export const COVERS = "x-masu-covers";
export const ASK = "x-masu-ask";
export const COVERED = "x-masu-covered";
/** Bytes per second an upload is given at least before it is given up (a phone's poor link). */
const SLOW_LINK = 64 * 1024;

/** What a store that met another device's newer save rejects with (every later store of the
 * page too: the player must reload to take the other device's save). */
export class SaveConflict extends Error {
  constructor() {
    super("別の端末でセーブが更新されました。再読み込みが必要です");
    this.name = "SaveConflict";
  }
}

/** Serialize writes per game and keep the last acknowledged revision. Every request has a time
 * limit (`waitMs`, an upload also a second per SLOW_LINK bytes): one that stalls without failing
 * (a phone moving between networks) used to hold every later save of the game behind it. */
export class SaveTransport {
  private revisions = new Map<string, string>();
  private queues = new Map<string, Promise<void>>();
  /** what a game started from when the server could not be reached (this browser's copy) */
  private bases = new Map<string, Uint8Array>();
  // (fetch called as this.request would be an illegal invocation: bound to the window)
  constructor(
    private request: typeof fetch = (input, init) => fetch(input, init),
    private waitMs = 30_000,
  ) {}
  private limit(bytes = 0): AbortSignal {
    return AbortSignal.timeout(this.waitMs + Math.ceil((bytes / SLOW_LINK) * 1000));
  }
  /** The server's copy (null: none), and whether an upload named `asked` (a record of this
   * browser) as one it holds or supersedes (`covered`; saves.ts, review b5dacf4 1.1). */
  async load(
    url: string,
    asked: string | null,
  ): Promise<{ data: Uint8Array<ArrayBuffer> | null; covered: boolean }> {
    const r = await this.request(url, {
      cache: "no-store",
      signal: this.limit(),
      ...(asked ? { headers: { [ASK]: asked } } : {}),
    });
    if (r.status === 404) {
      this.revisions.set(url, '"0"');
      return { data: null, covered: false };
    }
    if (!r.ok) throw new Error(`セーブの取得に失敗しました: HTTP ${r.status}`);
    const revision = r.headers.get("etag");
    if (!revision) throw new Error("セーブのバージョンがありません");
    const data = new Uint8Array(await r.arrayBuffer());
    this.revisions.set(url, revision);
    return { data, covered: !!asked && r.headers.get(COVERED) === asked };
  }
  /**
   * The server was out of reach at the start, so the game started from `data` (this browser's
   * copy): the first store later learns the server's revision, and writes over it only if the
   * server still has these same bytes (or nothing), so another device's newer save is not lost.
   */
  startedOffline(url: string, data: Uint8Array | null, revision?: string | null): void {
    // (the revision this browser's copy was last in step with, kept beside it (saves.ts): the
    // server's If-Match then tells whether another device wrote since)
    if (revision) this.revisions.set(url, revision);
    else this.bases.set(url, data ?? new Uint8Array());
  }
  /** The server's revision this transport last saw for `url` (undefined: none yet). */
  revision(url: string): string | undefined {
    return this.revisions.get(url);
  }
  private async learnRevision(url: string): Promise<void> {
    const base = this.bases.get(url);
    if (!base) throw new Error("セーブの読み込みが未完了です");
    const r = await this.request(url, { cache: "no-store", signal: this.limit() });
    if (r.status === 404) {
      this.revisions.set(url, '"0"');
    } else {
      if (!r.ok) throw new Error(`セーブの取得に失敗しました: HTTP ${r.status}`);
      const revision = r.headers.get("etag");
      const bytes = new Uint8Array(await r.arrayBuffer());
      if (!revision || !same(bytes, base)) throw new SaveConflict();
      this.revisions.set(url, revision);
    }
    this.bases.delete(url);
  }
  /** Writes `data` as the server's copy: the revision it is there as (the one this write made,
   * not whatever a later store left in `revision(url)` by the time the caller looks). */
  /** (`covers`: the record of this browser these bytes hold or supersede, kept by the server
   * beside the revision: a start asks for it when that record is still marked unsent) */
  store(url: string, data: Uint8Array<ArrayBuffer>, covers: string | null): Promise<string> {
    // (the caller's array as it is: callers hand a fresh export and do not change it)
    const snapshot = data;
    const previous = this.queues.get(url) ?? Promise.resolve();
    const pending = previous
      .catch(() => {})
      .then(async () => {
        if (!this.revisions.has(url)) await this.learnRevision(url);
        const r = await this.request(url, {
          method: "PUT",
          headers: { "if-match": this.revisions.get(url)!, ...(covers ? { [COVERS]: covers } : {}) },
          body: snapshot,
          keepalive: snapshot.length < 60000,
          signal: this.limit(snapshot.length),
        });
        if (r.status === 412) {
          // The write may have committed while its acknowledgement was lost. Accept only
          // byte-identical content; a genuinely different save must remain a conflict.
          const current = await this.request(url, { cache: "no-store", signal: this.limit(snapshot.length) });
          if (current.ok && current.headers.get("etag")) {
            const bytes = new Uint8Array(await current.arrayBuffer());
            if (same(bytes, snapshot)) {
              const committed = current.headers.get("etag")!;
              this.revisions.set(url, committed);
              return committed;
            }
          }
          throw new SaveConflict();
        }
        if (!r.ok) throw new Error(`セーブの保存に失敗しました: HTTP ${r.status}`);
        const revision = r.headers.get("etag");
        if (!revision) throw new Error("保存結果のバージョンがありません");
        this.revisions.set(url, revision);
        return revision;
      });
    this.queues.set(
      url,
      pending.then(
        () => undefined,
        () => undefined,
      ),
    );
    return pending;
  }
}
