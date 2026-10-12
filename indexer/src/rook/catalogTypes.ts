/**
 * The collections catalogue's API, as types. One file, no imports: web-rook/src/rook/catalogTypes.ts is a byte copy (its
 * copies test checks). docs/v6/collections-catalog.md. Every ETH amount is wei as a decimal string, every Bitcoin amount is
 * sats as a decimal string; times are unix seconds; `null` is "not known", never zero and never a guess.
 *
 * The catalogue is a directory of collections whether or not Rook lists them — every one an open nest: the curated seed (a
 * committed file, built from Nosh's registry on Robinhood Chain and the ordinals collections the old etched site listed)
 * merged with what the public marketplace APIs answered on the last refresh, and each entry's standing against Rook's
 * listing rule.
 */
/** Where the items live. `robinhood` is Robinhood Chain (4663) itself; `other` is a chain no vault here can hold on (Solana, HyperEVM, Zcash), named in `chainName`. */
export type CatalogChain = "bitcoin" | "ethereum" | "base" | "robinhood" | "other";
/** What the items are: inscriptions on Bitcoin; ERC-721s; CryptoPunks (their own market contract, handled by the home-chain
 *  vault's punk path); ERC-1155s and anything else, which no vault here can hold one of. */
export type CatalogStandard = "inscriptions" | "erc721" | "cryptopunks" | "erc1155" | "other";
/** `seed`: the committed file's dated record; `nosh`: Nosh's collection API, relayed OpenSea figures, read once for the seed;
 *  `ord`: the ordinals content gateway (item pictures); `tokenuri`: the collection's own contract through a public RPC, and its
 *  tokens' metadata (item pictures, keyless); `rook`: this indexer's own wrapped-collection reads (the keeper's posted floor). */
export type CatalogSourceName = "ordinalswallet" | "magiceden" | "opensea" | "coinbase" | "seed" | "nosh" | "ord" | "tokenuri" | "rook";
export type CatalogStatusKind = "live" | "listable" | "later" | "unsupported";

/** The cheapest listed item, as a source reported it, and where and when. Never computed from anything but a listing. */
export type CatalogFloor = {
  /** in the chain's own unit: sats on Bitcoin, wei on an EVM chain */
  native: string;
  unit: "sats" | "wei";
  /** the same in ETH — sats through the catalogue's rate — for the rule and the sort; null without a rate */
  eth: number | null;
  source: CatalogSourceName;
  /** the source's own name for what was read (a slug, a symbol), so a reader can repeat the read */
  ref: string;
  /** when the source was read; for a seed record, when the record was written down */
  at: number;
  /** true when the floor is not from this catalogue's last successful refresh: a record kept in the seed, or a cache the
   *  sources failed to renew. A stale floor is shown with its date, never as the price now. */
  stale: boolean;
} | null;

/** Where the entry stands with Rook, and why, in one sentence a page can print. */
export type RookStatus =
  | { kind: "live"; /** the nest's slug on Rook (#/collection/<slug>) */ slug: string; coins: number; why: string }
  | { kind: "listable"; why: string; /** how many items at this floor the current stage's per-collection cap holds */ slotsNow: number }
  | { kind: "later"; why: string; /** the first stage whose caps hold one item at this floor; null when no stage does, or no floor is known */ stage: number | null }
  | { kind: "unsupported"; why: string };

export type CatalogSourceRead = { name: CatalogSourceName; /** the HTTP status, or null when the request never got an answer */ status: number | null; ok: boolean; at: number; note: string };

/** One of a collection's items, a sample of up to twelve: enough for a grid, never the whole collection. */
export type CatalogItem = {
  /** the inscription id, or the token id */
  id: string;
  name: string | null;
  /** the item's picture at its source (the ordinals content gateway, a marketplace's CDN, the token's own metadata through an
   *  IPFS gateway), or the picture itself as a base64 `data:image/…` URI when it lives on chain; live, the site loads it through
   *  the indexer's image route */
  image: string | null;
  /** the item at its explorer or marketplace */
  url: string | null;
};

export type CatalogEntry = {
  slug: string;
  name: string;
  chain: CatalogChain;
  /** the chain by name, for a badge: "Bitcoin", "Ethereum", "Base", "Robinhood Chain", "Solana", … */
  chainName: string;
  /** the EVM chain id, null off the EVM */
  chainId: number | null;
  standard: CatalogStandard;
  /** the contract address (an EVM chain), the collection's id on its Bitcoin marketplace, or its marketplace slug elsewhere */
  id: string;
  description: string | null;
  /** the collection's picture at its source: a marketplace's public logo (OpenSea's `image_url`, Ordinals Wallet's icon), the same
   *  brand image every aggregator shows. Live, the site asks the indexer's image route, which fetches, checks and caches it; in the
   *  labelled fixture it loads the URL itself. A mark cut from the slug stands in wherever a picture cannot be had. */
  image: string | null;
  links: { website: string | null; x: string | null; discord: string | null; marketplace: string | null; explorer: string | null };
  floor: CatalogFloor;
  items: number | null;
  owners: number | null;
  /** the source's count of items listed for sale right now; null when the source does not say */
  listed: number | null;
  volume24hEth: number | null;
  volume7dEth: number | null;
  /** items Rook's custody holds for this collection, from this indexer's wrapped reads; null unless the nest is live */
  heldByRook: number | null;
  sample: CatalogItem[];
  /** which sources were asked for this entry on the last refresh and what each answered; a source not here was not asked */
  sources: CatalogSourceRead[];
  rook: RookStatus;
  /** true when the entry is in the curated seed (docs/v6/collections-catalog.md §2), which every entry is today */
  curated: boolean;
};

/** The listing rule, with the stage's numbers, so a page prints what the service applied and not a paraphrase. */
export type CatalogRule = {
  stage: number;
  stageName: string;
  /** docs/v5/launch-gates.md G7: the stage's per-item maximum and per-collection cap, wei */
  perItemMaxWei: string;
  perCollectionCapWei: string;
  perCollectionItems: number;
  /** the keeper's allowance over the floor a purchase may pay (the per-item maximum from stage 1 is 1.2 × floor) */
  allowanceBps: number;
  /** the highest floor the rule admits at this stage: min(per-item max, per-collection cap) / (1 + allowance), wei */
  maxFloorWei: string;
  text: string;
  doc: string;
};

export type Catalog = {
  source: { kind: "live" | "fixture"; note: string };
  at: number;
  /** the last refresh in which every source that was asked answered; null when none has yet */
  refreshedAt: number | null;
  /** when the sources were last asked, whatever they answered; null before the first attempt */
  attemptedAt: number | null;
  ttlS: number;
  /** true when what is served is not a full, fresh refresh: some or every source failed, or nothing has been fetched yet
   *  and the seed's records are what there is */
  stale: boolean;
  /** what the sources answered on the last attempt, per source, for the page's "sources" line */
  sources: { name: CatalogSourceName; asked: number; answered: number; statuses: number[]; note: string }[];
  satsPerEth: number | null;
  usdPerEth: number | null;
  rate: { source: CatalogSourceName; at: number; stale: boolean } | null;
  rule: CatalogRule;
  entries: CatalogEntry[];
};
export type CatalogOne = { source: Catalog["source"]; at: number; stale: boolean; satsPerEth: number | null; usdPerEth: number | null; rule: CatalogRule; entry: CatalogEntry };
