/**
 * docs/v5/floor-feed.md §1. A collection's floor that is hard to move: the median of the lowest five valid listings (one per
 * wallet cluster), blended DOWN toward the median of recent valid sales, with every listing and sale that fails a rule reported
 * under the rule's id. Pure: facts in, a number (or none) and its reasons out. Integer arithmetic throughout, in the collection's
 * own unit (sats for inscriptions, wei for EVM NFTs); the caller converts.
 */

export type Contents = { inscriptions: string[]; runes: string[] };

export type Listing = {
  /** The item: an inscription id, or `<collection>:<tokenId>`. */
  id: string;
  chain: "btc" | "evm";
  seller: string;
  price: bigint;
  /** When it was listed (unix s). */
  listedAt: number;
  /** When its fillability was last established (unix s), by the source or a check of our own; null: never. */
  verifiedAt: number | null;
  /** The source or the chain says it can be filled now; false: spent, cancelled, filled, not the owner's. */
  fillable: boolean;
  /** EVM: the order's end time (unix s); null: none. */
  expiresAt?: number | null;
  /** Bitcoin: ord's contents of the listed outpoint; null: ord has not answered. */
  contents?: Contents | null;
  /** EVM: offer items in the order (1 for a single NFT), whether it is priced in native ETH, whether it is a criteria/private order. */
  offerItems?: number;
  nativeCurrency?: boolean;
  special?: boolean;
};

export type Sale = { tx: string; at: number; item: string; seller: string; buyer: string; price: bigint; itemsInTx: number };
export type ListingEvent = { wallet: string; at: number; kind: "list" | "delist" };

export type FloorInput = {
  listings: Listing[];
  sales: Sale[];
  /** List / delist events, from a source's activity feed or the keeper's own diff of successive books. */
  events?: ListingEvent[];
  /** Addresses a source says are one party (a common funder, say). */
  links?: [string, string][];
  /** When the book was read (unix s). */
  snapshotAt: number;
  /**
   * How many independent marketplaces the sales came from (default 1). With fewer than two, sales can pull the floor at most
   * `maxSalePullBps` below the listing floor (audit E-01: one marketplace's sales cost an attacker only fees to write).
   */
  saleSources?: number;
};

export const FLOOR_PARAMS = {
  K: 5,
  minListings: 3,
  seasoningS: 1_800,
  staleS: 900,
  expiryMarginS: 1_800,
  burstWindowS: 3_600,
  burstDelists: 3,
  burstEvents: 6,
  burstCooldownS: 21_600,
  saleWindowS: 604_800,
  linkHistoryS: 30 * 86_400,
  maxSales: 20,
  minSales: 3,
  blendListingTenths: 7n,
  depthBandBps: 1_000n,
  /** E-01: with sales from one marketplace, the sales side moves the floor at most this far below the listing floor (5% = one
   *  hour of the module's move). Listings must be paid to move (each fake-low one can be sniped); wash sales cost only fees. */
  maxSalePullBps: 500n,
} as const;
export type FloorParams = { [K in keyof typeof FLOOR_PARAMS]: (typeof FLOOR_PARAMS)[K] extends bigint ? bigint : number };

export type Rule = "L-unverified" | "L-extras" | "L-currency" | "L-unfillable" | "L-stale" | "L-unseasoned" | "L-burst" | "L-seller-dup"
  | "S-window" | "S-self" | "S-wash" | "S-bundle" | "S-outlier" | "S-repeat" | "S-item-repeat";
export type Excluded = { what: "listing" | "sale"; id: string; rule: Rule; why: string };

export type Floor = {
  /** The floor, or null: fewer than `minListings` valid listings (nothing is posted). */
  price: bigint | null;
  listingFloor: bigint | null;
  saleFloor: bigint | null;
  confidenceBps: number;
  /** Valid listings (every listing rule but the per-cluster one) at or under floor × (1 + depthBand). */
  depthItems: number;
  /** Gross cost to buy listings from the bottom until the listing floor is 10% higher; null: the book runs out first. */
  upCost10: bigint | null;
  /** The listings the median was taken over, cheapest first, and the sales. */
  used: string[];
  salesUsed: string[];
  excluded: Excluded[];
};

// ---- small integer helpers ---------------------------------------------------------------------------------------------------

const lowerMedian = (xs: bigint[]): bigint => {
  const s = [...xs].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  return s[(s.length - 1) >> 1];
};
const abs = (x: bigint): bigint => (x < 0n ? -x : x);
const minB = (a: bigint, b: bigint): bigint => (a < b ? a : b);

// ---- clusters --------------------------------------------------------------------------------------------------------------------

export class Clusters {
  private parent = new Map<string, string>();
  find(a: string): string {
    const k = a.toLowerCase();
    let p = this.parent.get(k) ?? k;
    if (p === k) return k;
    p = this.find(p);
    this.parent.set(k, p);
    return p;
  }
  join(a: string, b: string): void {
    const x = this.find(a), y = this.find(b);
    if (x !== y) this.parent.set(x < y ? y : x, x < y ? x : y);
  }
  same(a: string, b: string): boolean { return this.find(a) === this.find(b); }
}

/** §1.3: both-direction trading, value moved at under a quarter of the floor, and a source's explicit links. */
export function clustersOf(sales: Sale[], links: [string, string][], refFloor: bigint | null): Clusters {
  const c = new Clusters();
  const dir = new Set(sales.map((s) => `${s.seller.toLowerCase()}>${s.buyer.toLowerCase()}`));
  for (const s of sales) {
    const a = s.seller.toLowerCase(), b = s.buyer.toLowerCase();
    if (a === b) continue;
    if (dir.has(`${b}>${a}`)) c.join(a, b);
    if (refFloor !== null && s.itemsInTx === 1 && s.price * 4n < refFloor) c.join(a, b);
  }
  for (const [a, b] of links) c.join(a, b);
  return c;
}

// ---- the listing rules -------------------------------------------------------------------------------------------------------

function listingRule(l: Listing, now: number, p: FloorParams): [Rule, string] | null {
  if (l.chain === "btc") {
    if (l.contents === undefined || l.contents === null) return ["L-unverified", "ord has not said what the listed outpoint carries"];
    if (l.contents.inscriptions.length !== 1 || l.contents.inscriptions[0] !== l.id || l.contents.runes.length !== 0)
      return ["L-extras", `the outpoint carries ${l.contents.inscriptions.length} inscription(s) and ${l.contents.runes.length} rune(s), not exactly ${l.id}`];
  } else {
    if ((l.offerItems ?? 1) !== 1) return ["L-extras", `the order offers ${l.offerItems} items`];
    if (l.nativeCurrency === false || l.special) return ["L-currency", "not a plain order for native ETH"];
  }
  if (!l.fillable) return ["L-unfillable", "the source or the chain says it cannot be filled"];
  if (l.verifiedAt === null || now - l.verifiedAt > p.staleS) return ["L-stale", `its fillability was last established ${l.verifiedAt === null ? "never" : `${now - l.verifiedAt} s ago`}`];
  if (l.expiresAt !== undefined && l.expiresAt !== null && l.expiresAt - now < p.expiryMarginS) return ["L-stale", `it expires in ${l.expiresAt - now} s`];
  if (now - l.listedAt < p.seasoningS) return ["L-unseasoned", `listed ${now - l.listedAt} s ago`];
  return null;
}

/** Clusters (by root) that cycled listings: ≥ burstDelists delists or ≥ burstEvents events in one window, the last within the cooldown. */
export function burstClusters(events: ListingEvent[], c: Clusters, now: number, p: FloorParams): Set<string> {
  const by = new Map<string, ListingEvent[]>();
  for (const e of events) {
    if (e.at > now || e.at < now - p.burstCooldownS - p.burstWindowS) continue;
    const k = c.find(e.wallet);
    by.set(k, [...(by.get(k) ?? []), e]);
  }
  const out = new Set<string>();
  for (const [k, es] of by) {
    for (const e of es) {
      if (e.at < now - p.burstCooldownS) continue;
      const w = es.filter((x) => x.at > e.at - p.burstWindowS && x.at <= e.at);
      if (w.filter((x) => x.kind === "delist").length >= p.burstDelists || w.length >= p.burstEvents) { out.add(k); break; }
    }
  }
  return out;
}

/** The cheapest listing per cluster, cheapest first. */
function perCluster(ls: Listing[], c: Clusters): Listing[] {
  const sorted = [...ls].sort((a, b) => (a.price < b.price ? -1 : a.price > b.price ? 1 : a.id < b.id ? -1 : 1));
  const seen = new Set<string>(), out: Listing[] = [];
  for (const l of sorted) {
    const k = c.find(l.seller);
    if (seen.has(k)) continue;
    seen.add(k);
    out.push(l);
  }
  return out;
}

/** The listing floor of a set of valid listings: the lower median of the lowest K, one per cluster; null under minListings. */
export function listingFloorOf(valid: Listing[], c: Clusters, p: FloorParams = FLOOR_PARAMS): { floor: bigint | null; used: Listing[] } {
  const used = perCluster(valid, c).slice(0, p.K);
  if (used.length < p.minListings) return { floor: null, used };
  return { floor: lowerMedian(used.map((l) => l.price)), used };
}

/**
 * docs/v5/floor-feed.md §4.1. What it costs to raise the listing floor to `target`: buy the cheapest valid listing, recompute,
 * repeat. Gross (the items are kept). Null when the book runs out before the floor gets there.
 */
export function costToRaise(valid: Listing[], c: Clusters, target: bigint, p: FloorParams = FLOOR_PARAMS): { items: number; gross: bigint } | null {
  const book = [...valid].sort((a, b) => (a.price < b.price ? -1 : a.price > b.price ? 1 : 0));
  let gross = 0n;
  for (let n = 0; n <= book.length; n++) {
    const f = listingFloorOf(book.slice(n), c, p).floor;
    if (f === null) return null;
    if (f >= target) return { items: n, gross };
    gross += book[n].price;
  }
  return null;
}

/**
 * docs/v5/floor-feed.md §4.2. What it takes to lower the listing floor to `target`: seasoned listings at `target` from fresh,
 * unlinked wallets until the lower median of the lowest K is at or under it; each one can be sniped for (floor − target).
 */
export function costToLower(valid: Listing[], c: Clusters, target: bigint, p: FloorParams = FLOOR_PARAMS): { listings: number; lossIfSniped: bigint } {
  const { floor } = listingFloorOf(valid, c, p);
  const ladder = perCluster(valid, c);
  const below = ladder.filter((l) => l.price <= target).length;
  const need = Math.max(0, ((p.K - 1) >> 1) + 1 - below);
  return { listings: need, lossIfSniped: floor === null ? 0n : BigInt(need) * (floor > target ? floor - target : 0n) };
}

// ---- the floor -------------------------------------------------------------------------------------------------------------------

export function robustFloor(input: FloorInput, p: FloorParams = FLOOR_PARAMS): Floor {
  const now = input.snapshotAt, excluded: Excluded[] = [];
  // 1. each listing on its own facts
  const survivors: Listing[] = [];
  for (const l of input.listings) {
    const r = listingRule(l, now, p);
    if (r) excluded.push({ what: "listing", id: l.id, rule: r[0], why: r[1] });
    else survivors.push(l);
  }
  // 2. clusters: from the sales of the window and the month before it, with a per-address floor as the reference for rule 2
  const history = input.sales.filter((s) => s.at <= now && now - s.at <= p.saleWindowS + p.linkHistoryS);
  const pre = listingFloorOf(survivors, new Clusters(), p).floor;
  const c = clustersOf(history, input.links ?? [], pre);
  // 3. wallets that cycled listings
  const burst = burstClusters(input.events ?? [], c, now, p);
  const valid: Listing[] = [];
  for (const l of survivors) {
    if (burst.has(c.find(l.seller))) excluded.push({ what: "listing", id: l.id, rule: "L-burst", why: `${l.seller} cycled listings within the last ${p.burstCooldownS} s` });
    else valid.push(l);
  }
  const { floor: L, used } = listingFloorOf(valid, c, p);
  const usedIds = new Set(used.map((l) => l.id)), usedClusters = new Set(used.map((l) => c.find(l.seller)));
  for (const l of valid) {
    if (!usedIds.has(l.id) && usedClusters.has(c.find(l.seller)))
      excluded.push({ what: "listing", id: l.id, rule: "L-seller-dup", why: `its seller's cluster already has a cheaper listing` });
  }
  // 4. sales
  const candidates: Sale[] = [], itemsSeen = new Set<string>();
  for (const s of [...input.sales].sort((a, b) => b.at - a.at)) {
    let r: [Rule, string] | null = null;
    if (s.at > now || now - s.at > p.saleWindowS) r = ["S-window", `${now - s.at} s old`];
    else if (s.seller.toLowerCase() === s.buyer.toLowerCase()) r = ["S-self", "buyer and seller are one address"];
    else if (c.same(s.seller, s.buyer)) r = ["S-wash", "buyer and seller are one cluster"];
    else if (s.itemsInTx !== 1) r = ["S-bundle", `${s.itemsInTx} items in the transaction`];
    else if (L !== null && (s.price * 4n < L || s.price > L * 4n)) r = ["S-outlier", `${s.price} against a listing floor of ${L}`];
    // E-01: one item moved around wallet pairs is one sale, not many (the newest counts)
    else if (itemsSeen.has(s.item)) r = ["S-item-repeat", `${s.item} already sold more recently in the window`];
    if (r) excluded.push({ what: "sale", id: s.tx + ":" + s.item, rule: r[0], why: r[1] });
    else { candidates.push(s); itemsSeen.add(s.item); }
  }
  const buyers = new Set<string>(), sellers = new Set<string>(), sales: Sale[] = [];
  for (const s of candidates) {
    const b = c.find(s.buyer), se = c.find(s.seller);
    if (buyers.has(b) || sellers.has(se) || sales.length >= p.maxSales) {
      excluded.push({ what: "sale", id: s.tx + ":" + s.item, rule: "S-repeat", why: "its buyer or seller already counts once (or the window is full)" });
      continue;
    }
    buyers.add(b); sellers.add(se); sales.push(s);
  }
  const S = sales.length >= p.minSales ? lowerMedian(sales.map((s) => s.price)) : null;
  // 5. the floor
  // the blend, pulled DOWN by sales only; from a single marketplace, never more than maxSalePullBps below the listings (E-01)
  const blend = (l: bigint, s: bigint): bigint => {
    const b = (p.blendListingTenths * l + (10n - p.blendListingTenths) * s) / 10n;
    if ((input.saleSources ?? 1) >= 2) return b;
    const least = l - (l * p.maxSalePullBps) / 10_000n;
    return b < least ? least : b;
  };
  const price = L === null ? null : S === null ? L : minB(L, blend(L, S));
  // 6. confidence and depth
  let confidenceBps = 0, depthItems = 0, upCost10: bigint | null = null;
  if (L !== null && price !== null) {
    const bps = (x: bigint) => Number(x);
    const count = BigInt(Math.min(used.length, p.K)) * 10_000n / BigInt(p.K);
    const lo = used[0].price, hi = used[used.length - 1].price;
    const spreadRaw = 10_000n - ((hi - lo) * 10_000n) / (2n * L);
    const spread = spreadRaw < 2_500n ? 2_500n : spreadRaw;
    const salesF = sales.length >= 5 ? 10_000n : sales.length >= 3 ? 8_000n : 6_000n;
    const agreeRaw = S === null ? 10_000n : 10_000n - (abs(L - S) * 10_000n) / L;
    const agree = agreeRaw < 5_000n ? 5_000n : agreeRaw;
    confidenceBps = bps((((count * spread) / 10_000n) * salesF / 10_000n) * agree / 10_000n);
    const band = price + (price * p.depthBandBps) / 10_000n;
    depthItems = valid.filter((l) => l.price <= band).length;
    upCost10 = costToRaise(valid, c, L + (L * 1_000n + 9_999n) / 10_000n, p)?.gross ?? null;
  }
  return { price, listingFloor: L, saleFloor: S, confidenceBps, depthItems, upCost10, used: used.map((l) => l.id),
    salesUsed: sales.map((s) => s.tx + ":" + s.item), excluded };
}

/** Sats → wei at `satsPerEth`, rounded down (the module is priced in ETH). */
export const satsToWei = (sats: bigint, satsPerEth: bigint): bigint => {
  if (satsPerEth <= 0n) throw new Error("invalid rate: no sats per ETH");
  return (sats * 10n ** 18n) / satsPerEth;
};
