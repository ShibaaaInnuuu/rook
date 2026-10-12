/** docs/phase4.md §5. A failing node is waited out, with a ceiling, and then this pass is given up — never a tight loop, never a crash. */
export const BACKOFF = { baseMs: 1_000, factor: 2, maxMs: 60_000, tries: 5 } as const;
export const delayFor = (attempt: number, b: { baseMs: number; factor: number; maxMs: number } = BACKOFF): number => Math.min(b.maxMs, b.baseMs * b.factor ** attempt);

export class GaveUp extends Error {
  constructor(readonly attempts: number, readonly last: string) { super(`gave up after ${attempts} attempts: ${last}`); this.name = "GaveUp"; }
}

/** Errors that another try cannot fix (a refusal, a bad argument) are thrown at once; everything else is retried. */
export const retryable = (err: unknown): boolean => !/refused|missing|invalid|not a read|no floor|no price/i.test(String((err as Error)?.message ?? err));

export async function withBackoff<T>(fn: () => Promise<T>, o: { tries?: number; sleep?: (ms: number) => Promise<void>; onRetry?: (attempt: number, waitMs: number, err: unknown) => void;
  baseMs?: number; factor?: number; maxMs?: number } = {}): Promise<T> {
  const tries = o.tries ?? BACKOFF.tries, sleep = o.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
  const b = { baseMs: o.baseMs ?? BACKOFF.baseMs, factor: o.factor ?? BACKOFF.factor, maxMs: o.maxMs ?? BACKOFF.maxMs };
  let last: unknown;
  for (let attempt = 0; attempt < tries; attempt++) {
    try { return await fn(); } catch (err) {
      last = err;
      if (!retryable(err)) throw err;
      if (attempt === tries - 1) break;
      const wait = delayFor(attempt, b);
      o.onRetry?.(attempt + 1, wait, err);
      await sleep(wait);
    }
  }
  throw new GaveUp(tries, String((last as Error)?.message ?? last).slice(0, 200));
}
