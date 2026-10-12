import { closeSync, existsSync, fsyncSync, mkdirSync, openSync, readFileSync, renameSync, writeSync } from "node:fs";
import { dirname, join } from "node:path";

/**
 * docs/v6/p1-services.md §1.5. What the keeper must remember across a restart, and only that: the one transaction in flight (its
 * nonce, hash and signed bytes, written BEFORE the bytes are sent), the day's gas spend, and a short history. Everything else is
 * re-derived from the chain every pass. Written whole to a temporary file, fsync'd, then renamed over the old one: a crash leaves
 * the old state or the new, never half of either.
 */
/** One signed transaction for the pending nonce: the first, or a replacement at a higher fee (audit E-10). */
export type Attempt = { hash: string; raw: string; maxFeePerGas: string; tip: string; gas: string; sentAt: number; to: string; data: string; cancel: boolean };
/**
 * The one nonce in flight. `hash` / `raw` are the NEWEST attempt's; `attempts` holds every attempt signed for this nonce (any of them
 * may be the one that is mined). `reservedWei` is the worst case of the newest attempt, counted against its day.
 */
export type Pending = { key: string; what: string; to: string; data: string; nonce: number; hash: string; raw: string; sentAt: number; reservedWei: string; day: string;
  attempts: Attempt[]; firstSentAt: number };
export type Done = { key: string; what: string; hash: string; nonce: number; outcome: "mined" | "reverted" | "dropped"; gasWei: string; at: number; block: number | null };
export type KeeperState = { version: 2; day: string; spentWei: string; pending: Pending | null; history: Done[] };

export const EMPTY: KeeperState = { version: 2, day: "", spentWei: "0", pending: null, history: [] };
const KEEP = 100;

export class StateStore {
  private s: KeeperState;
  readonly path: string;
  constructor(dir: string) {
    mkdirSync(dir, { recursive: true });
    this.path = join(dir, "keeper-state.json");
    this.s = existsSync(this.path) ? StateStore.parse(readFileSync(this.path, "utf8")) : structuredClone(EMPTY);
  }
  /** Version 2, or version 1 upgraded (its pending transaction becomes the first attempt). Anything else is refused. */
  static parse(text: string): KeeperState {
    const j = JSON.parse(text) as { version: number; spentWei: unknown; history: unknown; pending: (Omit<Pending, "attempts" | "firstSentAt"> & Partial<Pending>) | null; day: string };
    if ((j.version !== 1 && j.version !== 2) || typeof j.spentWei !== "string" || !Array.isArray(j.history)) throw new Error("the keeper's state file is not version 1 or 2: refusing to guess");
    const p = j.pending;
    const pending: Pending | null = p ? { ...p, attempts: p.attempts ?? [{ hash: p.hash, raw: p.raw, maxFeePerGas: "0", tip: "0", gas: "0", sentAt: p.sentAt, to: p.to, data: p.data, cancel: false }],
      firstSentAt: p.firstSentAt ?? p.sentAt } : null;
    return { version: 2, day: j.day, spentWei: j.spentWei, pending, history: j.history as Done[] };
  }
  get(): Readonly<KeeperState> { return this.s; }

  /** The spend so far on `day` (UTC yyyy-mm-dd), with the reservation of a pending transaction of that day. */
  committed(day: string): bigint {
    const spent = this.s.day === day ? BigInt(this.s.spentWei) : 0n;
    const reserved = this.s.pending && this.s.pending.day === day ? BigInt(this.s.pending.reservedWei) : 0n;
    return spent + reserved;
  }
  setPending(p: Pending): void { this.s.pending = p; this.save(); }
  /** `day` is the UTC day the transaction was sent on: its gas counts against that day, and a later day starts from zero. */
  finish(d: Done, day: string): void {
    if (day > this.s.day) { this.s.day = day; this.s.spentWei = "0"; }
    if (day === this.s.day) this.s.spentWei = String(BigInt(this.s.spentWei) + BigInt(d.gasWei));
    this.s.pending = null;
    this.s.history = [d, ...this.s.history].slice(0, KEEP);
    this.save();
  }
  /** Nothing was broadcast (the node refused the bytes outright): the reservation is released. */
  abandon(): void { this.s.pending = null; this.save(); }

  private save(): void {
    const tmp = `${this.path}.tmp`;
    const fd = openSync(tmp, "w");
    try { writeSync(fd, JSON.stringify(this.s, null, 1) + "\n"); fsyncSync(fd); } finally { closeSync(fd); }
    renameSync(tmp, this.path);
    try { const d = openSync(dirname(this.path), "r"); try { fsyncSync(d); } finally { closeSync(d); } } catch { /* a directory fsync is best effort */ }
  }
}

export const utcDay = (ms: number): string => new Date(ms).toISOString().slice(0, 10);
