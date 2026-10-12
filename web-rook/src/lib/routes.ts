/**
 * Every route the site answers, in one place.
 *
 * The masthead reads NAV, the shell reads KNOWN to tell a page from a typo, and AppPages dispatches on the same strings. Keeping
 * them here is what makes it possible to check, in a test, that a route added to one of them was added to all of them.
 */

/** The masthead, in order. Launch and the wordmark are buttons rather than nav items, so they are not in this list. */
export const NAV: [string, string][] = [
  ["explore", "Explore"], ["vaults", "Vaults"], ["receipts", "Receipts"], ["draws", "Draws"], ["claims", "Claims"], ["reserves", "Reserves"], ["analytics", "Analytics"], ["docs", "Docs"],
];

/** The landing page is "". Everything else is the first part of the hash route. */
export const PAGES = [
  "", "explore", "receipts", "claims", "analytics", "launch", "coin", "listing", "listings", "vaults", "draws", "docs", "trust", "staging",
  "collection", "reserves", "redeem", "rent", "handins",
  "terms", "risk", "privacy",
] as const;
export type Page = (typeof PAGES)[number];

export const KNOWN: ReadonlySet<string> = new Set<string>(PAGES);
/** Pages that read no chain and need no wallet code: their own chunk. */
export const LEGAL: ReadonlySet<string> = new Set(["terms", "risk", "privacy"]);
/** The section a page belongs to, so its nav item stays lit while you are inside it. */
export const UNDER: Record<string, string> = { coin: "explore", listing: "vaults", listings: "vaults", collection: "reserves", redeem: "reserves", rent: "reserves", handins: "reserves" };

/**
 * What a thing is called on screen, against what the contracts and the indexer call it.
 *
 * On chain a registry holds `Listing` structs and the indexer serves `/v1/listings`; those are their words and this site does
 * not get to change them. What a reader sees is a **vault**, because that is the thing that exists: one per collection or rune,
 * holding ETH that can only buy that one asset. A collection with no vault is a catalogue entry and nothing else, and saying
 * so with a noun is what makes the two impossible to confuse. docs/phase9.md, section 8.
 *
 * `#/vaults` is canonical. `#/listings` still answers, because the deployed site and the docs link to it.
 */
export const CANONICAL: Record<string, string> = { listings: "vaults" };

export const sectionOf = (at: string): string => UNDER[at] ?? at;
