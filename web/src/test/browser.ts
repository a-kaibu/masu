// A fresh origin per scenario. Model the exclusive, callback-based Web Locks operation
// used by saves.ts; each name has its own FIFO and releases on success and failure.
export function exclusiveLocks() {
  const tails = new Map<string, Promise<void>>();
  return {
    request<T>(name: string, callback: (lock: { name: string; mode: "exclusive" }) => T) {
      const result = (tails.get(name) ?? Promise.resolve()).then(() => callback({ name, mode: "exclusive" }));
      const released = result.then(
        () => {},
        () => {},
      );
      tails.set(name, released);
      return result.finally(() => {
        if (tails.get(name) === released) tails.delete(name);
      });
    },
  };
}

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    key: (i) => [...values.keys()][i] ?? null,
    getItem: (k) => values.get(String(k)) ?? null,
    setItem: (k, v) => {
      values.set(String(k), String(v));
    },
    removeItem: (k) => {
      values.delete(String(k));
    },
    clear: () => values.clear(),
  };
}

export function browser() {
  let online = true;
  let visibility: DocumentVisibilityState = "visible";
  const events = new EventTarget();
  const globals = {
    location: new URL("https://masu.test/play.html"),
    navigator: {
      get onLine() {
        return online;
      },
      locks: exclusiveLocks(),
      language: "en-US",
      languages: ["en-US"],
      hardwareConcurrency: 4,
      userAgent: "masu-simulation",
    },
    document: {
      get visibilityState() {
        return visibility;
      },
    },
    addEventListener: events.addEventListener.bind(events),
    removeEventListener: events.removeEventListener.bind(events),
    localStorage: memoryStorage(),
    sessionStorage: memoryStorage(),
    fetch: async () => {
      throw new Error("simulation requires an injected fetch");
    },
  };
  return {
    globals,
    events,
    setOnline(value: boolean) {
      if (value === online) return;
      online = value;
      events.dispatchEvent(new Event(value ? "online" : "offline"));
    },
    setVisibility(value: DocumentVisibilityState) {
      if (value === visibility) return;
      visibility = value;
      events.dispatchEvent(new Event("visibilitychange"));
    },
  };
}
