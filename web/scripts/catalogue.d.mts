/**
 * The types of the ranking in `catalogue.mjs`, so a test can import it and check the order without a network, a key or
 * a write. Only the pure parts are declared: everything that fetches stays private to the script.
 */

export declare const WANT_COLLECTIONS: number;
export declare const WANT_RUNES: number;
export declare const WINDOW: "day7";
export declare const RANK_FIELD: "volume7dSats";
export declare const LIFETIME_FIELD: "volumeAllTimeSats";

export type Rankable = {
  id: string; volume7dSats?: number | null; volumeAllTimeSats?: number | null;
  name?: string; slug?: string; tick?: string;
};

/** Sorted by the seven-day figure and cut to `want`, or thrown if the figure is missing or is not in sats. */
export declare function rankByWeek<T extends Rankable>(rows: T[], want: number, what: string): T[];

/**
 * Sorted by the LIFETIME figure and cut to `want`, with the same three refusals: no rows, no positive figure, or a
 * figure too small to be sats. The collections' basis while no source states their week (docs/phase9.md §11.3).
 */
export declare function rankByLifetime<T extends Rankable>(rows: T[], want: number, what: string): T[];

/** A marketplace timestamp in seconds, whichever of the two units it arrived in. */
export declare function stamp(t: unknown): number | null;

export type Fill = {
  collectionId: string;
  priceSats: number;
  at: number;
  inscriptionId?: string | null;
  itemName?: string | null;
  contentType?: string | null;
};
export type FillGroup = {
  id: string; slug: string; name: string;
  volume7dSats: number; trades7d: number;
  items: { id: string; num: number | null; name: string | null }[];
};

/** The fills, per collection: what was paid, how many times, and the ids that changed hands. */
export declare function groupFills(sold: Fill[]): FillGroup[];

/** The Ordinals Wallet collection index's rows, filtered and shaped into catalogue entries (pure: no fetch). */
export declare function shapeCollections(rows: Record<string, unknown>[]): {
  id: string; slug: string; name: string; kind: "inscriptions"; description: string | null; supply: number; floorSats: number | null;
  volume7dSats: null; volumeAllTimeSats: number | null; volumeOnOrdinalsWalletSats: number | null; listed: number | null; verified: boolean;
  first: number | null; last: number | null; sampleInscription: string | null; iconUrl: string | null; source: string; sourceUrl: string;
}[];
