/**
 * The receipts feed's ordering, apart from its rendering.
 *
 * A row is "new" only when the feed has been seen before: on a first render nothing is cut in, because everything is new
 * and animating the lot would be noise rather than news. docs/phase9.md, section 4.
 */

export type Settled = { listingId: number; acquisitionId: number; settledAt: number | null };

export const keyOf = (a: Settled): string => `${a.listingId}-${a.acquisitionId}`;

/** Newest settlement first. A row with no time sorts last rather than to the top. */
export function newestFirst<T extends Settled>(rows: T[], limit: number): T[] {
  return [...rows].sort((a, b) => (b.settledAt ?? 0) - (a.settledAt ?? 0)).slice(0, limit);
}

/** Which of these rows were not here last time. Empty on the first pass, by design. */
export function freshKeys(keys: string[], seen: Set<string> | null): Set<string> {
  if (seen === null) return new Set();
  return new Set(keys.filter((k) => !seen.has(k)));
}
