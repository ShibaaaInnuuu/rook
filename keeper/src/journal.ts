import { closeSync, existsSync, fsyncSync, mkdirSync, openSync, readFileSync, truncateSync, writeSync } from "node:fs";
import { dirname } from "node:path";

/**
 * docs/phase4.md §5. The journal is what the keeper WOULD have done, one JSON object per line. A line is written whole and
 * fsync'd before the pass goes on, so a crash leaves at worst one torn final line — and `repair` removes exactly that, on start,
 * so that a reader never meets half an object and nothing good is ever dropped.
 */
export function repair(path: string): { removedBytes: number } {
  if (!existsSync(path)) return { removedBytes: 0 };
  const buf = readFileSync(path);
  if (buf.length === 0 || buf[buf.length - 1] === 0x0a) return { removedBytes: 0 };
  const lastNewline = buf.lastIndexOf(0x0a);
  truncateSync(path, lastNewline + 1);
  return { removedBytes: buf.length - (lastNewline + 1) };
}

export class Journal {
  constructor(private readonly pathOf: (day: string) => string, private readonly now: () => Date = () => new Date()) {}
  private repaired = new Set<string>();
  append(entry: Record<string, unknown>): string {
    const path = this.pathOf(this.now().toISOString().slice(0, 10));
    mkdirSync(dirname(path), { recursive: true });
    if (!this.repaired.has(path)) { repair(path); this.repaired.add(path); }
    const line = JSON.stringify(entry, (_k, v) => (typeof v === "bigint" ? String(v) : v)) + "\n";
    const fd = openSync(path, "a");
    try { writeSync(fd, line); fsyncSync(fd); } finally { closeSync(fd); }
    return path;
  }
}
