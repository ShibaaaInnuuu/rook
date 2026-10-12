/** Every route the site answers, in one place: the masthead reads NAV, the shell reads KNOWN, SolPages dispatches on the same strings. */
/** The "How it works" page: a top-level route (`#/how-it-works`) that shows the docs page of the same slug (src/content/docs/02-how-it-works.md)
 *  with its flow picture; the masthead and the home page link to it. */
export const HOW_IT_WORKS = "how-it-works";
export const NAV: [string, string][] = [["coins", "Coins"], ["launch", "Launch a coin"], ["collections", "Collections"], [HOW_IT_WORKS, "How it works"], ["docs", "Docs"]];
export const PAGES = ["", "coins", "coin", "launch", "collections", "collection", "tracked", "reserves", "redeem", "draw", "rent", "bots", "wallet", "docs", HOW_IT_WORKS, "trust", "brand", "terms", "risk", "privacy"] as const;
export type Page = (typeof PAGES)[number];
export const KNOWN: ReadonlySet<string> = new Set<string>(PAGES);
/** Pages that read no chain and need no wallet code. */
export const STATIC: ReadonlySet<string> = new Set(["docs", HOW_IT_WORKS, "trust", "brand", "terms", "risk", "privacy"]);
/** The Solana pages (src/SolPages.tsx): `collection/<slug>` is a Rook collection's page for a slug of COLLECTIONS and a tracked
 *  collection's for any other; `tracked` is every collection Rook tracks (the catalogue, no wallet code). */
export const SOLANA: ReadonlySet<string> = new Set(["coins", "coin", "launch", "collections", "collection", "tracked", "reserves"]);
/** Pages of the Robinhood Chain design that the Solana launch does not have (docs/v9/solana-migration.md §2.7): their routes say so;
 *  their code stays in the tree (src/pages/{Reserves,Redeem,Draw,Rent,Bots,Wallet}.tsx), unrouted. `reserves` is the books home's
 *  (src/pages/SolReserves.tsx, docs/v9/canister-only-solana.md §10.2); `redeem` is the collection page's intent desk (§6.1). */
export const ROBINHOOD_ONLY: ReadonlySet<string> = new Set(["redeem", "draw", "rent", "bots", "wallet"]);
/** The section a page belongs to, so its nav item stays lit inside it. */
export const UNDER: Record<string, string> = { coin: "coins", collection: "collections", tracked: "collections", reserves: "collections" };
/** The phone's bottom nav (docs/v6/site-design.md §4): five places, always one tap away. */
export const BOTTOM: [string, string][] = [["", "Home"], ["coins", "Coins"], ["launch", "Launch a coin"], ["collections", "Collections"], ["docs", "Docs"]];
export const sectionOf = (at: string): string => UNDER[at] ?? at;
