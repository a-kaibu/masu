// Asks of which only the newest one's answer counts (review 22bb061 §6: the candidates of an
// earlier search, answered last, took the place of the newer search's): each begin aborts the
// one before and gives the new ask its signal and whether it is still the newest.
export interface Ask {
  readonly signal: AbortSignal;
  /** still the newest ask (its answer is taken) */
  current(): boolean;
}

export class Latest {
  #n = 0;
  #controller: AbortController | null = null;
  begin(): Ask {
    this.#controller?.abort();
    const controller = new AbortController();
    this.#controller = controller;
    const me = ++this.#n;
    return { signal: controller.signal, current: () => me === this.#n };
  }
  /** No ask counts any more (the view went away). */
  end(): void {
    this.#n++;
    this.#controller?.abort();
    this.#controller = null;
  }
}
