/**
 * The site's one vocabulary (design review, 2026-09-27). What a visitor reads, against what the contracts, the routes, the indexer
 * and the design documents call the same things — those keep their words; only the copy on screen changes:
 *
 *   on screen                      in the code and the documents
 *   collection                     nest (a collection with coins on it is a "live collection")
 *   vault                          hoard, custody
 *   NFT (ordinal, on Bitcoin)      item
 *   wFROG, wAPE, … (named once)    unit, wrapped unit
 *   reserves                       ledger
 *   buy the floor                  sweep
 *   the collection list            catalogue
 *
 * Pure, so a test can hold it.
 */
export const GLOSSARY: [string, string][] = [
  ["collection", "nest"],
  ["vault", "hoard, custody"],
  ["NFT (on Bitcoin, an ordinal)", "item"],
  ["wFROG, wAPE, … — the vault's wrapped token", "unit, wrapped unit"],
  ["reserves", "ledger"],
  ["buy the floor", "sweep"],
  ["the collection list", "catalogue"],
];

/** An NFT on an EVM chain; an ordinal (an inscription) on Bitcoin, where "NFT" would be the wrong word. */
export const nftWord = (bitcoin: boolean, n = 2): string => (bitcoin ? (n === 1 ? "ordinal" : "ordinals") : n === 1 ? "NFT" : "NFTs");
/** "1 NFT", "6 ordinals". */
export const nftCount = (n: number, bitcoin: boolean): string => `${n.toLocaleString("en-US")} ${nftWord(bitcoin, n)}`;

/** The one sentence that explains a wrapped token, wherever its name first appears. */
export const wrappedLine = (symbol: string, bitcoin: boolean): string =>
  `${symbol} is this collection's wrapped token: one whole ${symbol} stands for one ${nftWord(bitcoin, 1)} held in the vault, and burning one redeems one.`;
