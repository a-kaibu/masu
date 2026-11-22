// An origin private file system in memory, as the page's code uses it (navigator.storage):
// folders, files whose writable stream shows its bytes only when closed (as a browser's swap
// file does), keys() and removeEntry. Every operation waits `pause()` first, so a simulation
// interleaves two tabs' calls the way their own event loops would.
type Pause = () => Promise<void>;

class Dir {
  readonly dirs = new Map<string, Dir>();
  readonly files = new Map<string, Uint8Array<ArrayBuffer>>();
}

function fileHandle(parent: Dir, name: string, pause: Pause) {
  return {
    kind: "file" as const,
    name,
    async getFile() {
      await pause();
      const bytes = parent.files.get(name);
      if (!bytes) throw new DOMException(`${name} is gone`, "NotFoundError");
      const copy = bytes.slice();
      return { size: copy.length, arrayBuffer: async () => copy.buffer };
    },
    async createWritable() {
      await pause();
      const parts: Uint8Array[] = [];
      return {
        async write(data: Uint8Array) {
          await pause();
          parts.push(data.slice());
        },
        async close() {
          await pause();
          const whole = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
          let at = 0;
          for (const p of parts) whole.set(p, (at += p.length) - p.length);
          parent.files.set(name, whole);
        },
      };
    },
  };
}

function dirHandle(d: Dir, name: string, pause: Pause): FileSystemDirectoryHandle {
  const handle = {
    kind: "directory" as const,
    name,
    async getDirectoryHandle(child: string, opts?: { create?: boolean }) {
      await pause();
      let c = d.dirs.get(child);
      if (!c) {
        if (!opts?.create) throw new DOMException(`${child} not found`, "NotFoundError");
        d.dirs.set(child, (c = new Dir()));
      }
      return dirHandle(c, child, pause);
    },
    async getFileHandle(child: string, opts?: { create?: boolean }) {
      await pause();
      if (!d.files.has(child)) {
        if (!opts?.create) throw new DOMException(`${child} not found`, "NotFoundError");
        d.files.set(child, new Uint8Array(0));
      }
      return fileHandle(d, child, pause);
    },
    async removeEntry(child: string) {
      await pause();
      if (!d.files.delete(child) && !d.dirs.delete(child))
        throw new DOMException(`${child} not found`, "NotFoundError");
    },
    async *keys() {
      await pause();
      for (const k of [...d.dirs.keys(), ...d.files.keys()]) yield k;
    },
  };
  return handle as unknown as FileSystemDirectoryHandle;
}

/** A file system and a way to look at it without waiting. */
export function memoryOpfs(pause: Pause = async () => {}) {
  const root = new Dir();
  return {
    storage: { getDirectory: async () => dirHandle(root, "", pause) },
    /** `path`'s files ("states/game"), by name */
    files(path: string): Map<string, Uint8Array<ArrayBuffer>> {
      let d: Dir | undefined = root;
      for (const part of path.split("/").filter(Boolean)) d = d?.dirs.get(part);
      return d?.files ?? new Map();
    },
  };
}
