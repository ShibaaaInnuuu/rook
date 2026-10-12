/**
 * The types of the prober in `magiceden.mjs`, so a test can check its pacing, its cache-ghost rule and its verdict
 * without a network call. Only the pure parts are declared: everything that fetches stays private to the script.
 */

/** Magic Eden's own gap between calls, which is deliberately not UniSat's 260 ms. docs/phase9.md §11.2. */
export declare const MAGICEDEN_GAP_MS: number;
export declare const BACKOFF_FROM_MS: number;
export declare const BACKOFF_CEILING_MS: number;
export declare const ATTEMPTS: number;

/** The next backoff after one that did not work: doubling, and capped. */
export declare function backoffAfter(prev: number): number;

/** The `cf-cache-status` values in which the edge, not the origin, decided what the body says. */
export declare const GHOST_STATES: Set<string>;

/** A 200 Cloudflare served from a stale copy because it could not reach the origin. Not a read. */
export declare function isCacheGhost(status: number, cacheStatus: string | null | undefined): boolean;

export type Outcome = "live" | "ghost" | "busy" | "refused";

/** What one route's answer means: a live body, a cache ghost, an unreachable upstream, or a live origin declining. */
export declare function classify(status: number, cacheStatus: string | null | undefined): Outcome;

export type Route = { name: string; windowed: boolean; url: string };
export type Result = Route & { outcome: Outcome; rows: number };

/** The routes asked, and which of them could answer the week. Reads only: no order is placed, changed or cancelled. */
export declare const ROUTES: Route[];

/** How many rows a windowed body carried, whichever shape it arrived in. */
export declare function rowsIn(body: unknown): number;

/** `yes` a week is obtainable, `no` a live origin refused one, `not today` nothing answered and nothing refused. */
export declare function verdictOf(results: Result[]): "yes" | "no" | "not today";
