// SAKE_SHIRUSU=1's lines (crates/sake/src/shirusu.rs: "shirusu E ..." / "shirusu L ..."), one
// per guest call: kept apart from the console (play.ts printErr) for the harness to take
// (tests/corpus.mjs, masuShirusuTake) instead of a console message each, which the LineLimit
// cut (calls missing from the Trace) and which slowed the page until a game timed out. At most
// `max` characters wait; past them lines are counted, and the count is one line of its own
// ("shirusu X dropped N", which tools/shirusu does not take for a call) when they are taken.

export class ShirusuLines {
  private lines: string[] = [];
  private chars = 0;
  private dropped = 0;
  constructor(readonly max = 64 << 20) {}
  /** Whether `line` is one of these (and so kept here, not shown). */
  take(line: string): boolean {
    if (!line.startsWith("shirusu ")) return false;
    if (this.chars + line.length > this.max) {
      this.dropped++;
      return true;
    }
    this.lines.push(line);
    this.chars += line.length;
    return true;
  }
  /** The lines kept since the last call, a line each (empty: none). */
  drain(): string {
    if (this.dropped) this.lines.push(`shirusu X dropped ${this.dropped}`);
    const out = this.lines.join("\n");
    this.lines = [];
    this.chars = 0;
    this.dropped = 0;
    return out;
  }
}
