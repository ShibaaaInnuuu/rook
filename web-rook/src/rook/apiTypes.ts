/**
 * The Rook board's API, as types. One file, no imports: web-rook/src/rook/apiTypes.ts is a byte copy (its test checks).
 * docs/v6/site-design.md §6. Every ETH amount is wei as a decimal string; times are unix seconds; `null` is "not known".
 *
 * Two kinds of coin share the board. A `curve` coin is a Rook coin: launched through CurveFactory on a FrogCurve, its fees
 * sweeping a wrapped collection (docs/v3/curve.md, docs/v5). A `pons` coin was launched through the Launcher on Pons's own
 * curve and pays its creator fees into a vault (docs/phase1.md); it is traded on Pons, and the board shows it as such.
 */
export type RookSource = { kind: "live" | "fixture"; note: string };
export type RookKind = "curve" | "pons";
export type RookPhase = "trading" | "graduating" | "graduated" | "refunding";
export type RookPolicy = "raffle" | "burn" | "yoyo";

export type BoardRow = {
  kind: RookKind;
  address: `0x${string}`;
  /** the FrogCurve (curve) or Pons's curve (pons) */
  curve: `0x${string}`;
  name: string;
  symbol: string;
  logoUrl: string | null;
  creator: `0x${string}`;
  /** the collection the fees sweep: a wrapped collection's slug (curve) or a vault's listing (pons) */
  nest: { slug: string | null; listingId: number | null; name: string | null; symbol: string | null };
  /** fixed at launch; null for a pons coin, whose delivery mode is chosen per acquisition */
  policy: RookPolicy | null;
  /** the trade fee, bps: the curve's fee tier (curve) or the creator tax on top of Pons's base (pons) */
  feeBps: number | null;
  phase: RookPhase;
  launchedAt: number | null;
  graduatedAt: number | null;
  /** progress to graduation: what the curve has raised against its raise; null once graduated, or for a pons coin */
  raise: { raisedWei: string; raiseWei: string } | null;
  /** the pair asset per whole coin, the latest trade's curve-side price */
  price: number | null;
  /** the fraction the price moved over the last 24 hours (0.12 is +12%); null with fewer than two prices to compare */
  change24h: number | null;
  /** supply × price, in the pair asset, as a JSON number; null without a supply or a price */
  marketCapEth: number | null;
  supplyWei: string | null;
  /** the pair asset traded in the last 24 hours; null when the source does not know, 0 when nothing traded */
  volume24hWei: string | null;
  trades24h: number | null;
  /** buys in the last 24 hours (the trending score's buys-per-hour term); null when the source does not know */
  buys24h: number | null;
  holders: number | null;
  /** holders whose first inbound transfer was in the last 24 hours and who still hold (the trending score's growth term); null when not known */
  newHolders24h: number | null;
  /** what the coin's fees have put into its vault, or bought for its nest: ETH swept toward the floor */
  sweptWei: string | null;
};
export type Board = { source: RookSource; at: number; usdPerEth: number | null; rows: BoardRow[] };

export type RookRes = "1m" | "5m" | "15m" | "1h" | "1d";
export type RookCandle = { t: number; o: number; h: number; l: number; c: number; v: number; n: number };
export type RookTrade = {
  id: string;
  side: "buy" | "sell";
  venue: "curve" | "pool";
  quoteWei: string;
  coinWei: string;
  feeWei: string | null;
  taxWei: string | null;
  /** the pair asset per whole coin, curve-side (fee-free) */
  price: number;
  trader: `0x${string}`;
  tx: `0x${string}`;
  block: number;
  logIndex: number;
  at: number | null;
};
export type RookChart = {
  source: RookSource;
  kind: RookKind;
  coin: `0x${string}`;
  res: RookRes;
  /** the bucket actually drawn, seconds, and the window it covers */
  seconds: number;
  span: number;
  quote: "ETH";
  candles: RookCandle[];
  trades: number;
  tradesAllTime: number;
  launchedAt: number | null;
  graduation: { at: number | null; block: number | null; tx: `0x${string}` | null } | null;
  supplyWei: string | null;
  usdPerEth: number | null;
};
export type RookTradePage = { source: RookSource; kind: RookKind; items: RookTrade[]; next: string | null; total: number };
export type RookHolder = { rank: number; address: `0x${string}`; balanceWei: string; share: number | null; label: "creator" | "curve" | "vault" | "pool" | "lock" | null };
export type RookHolderPage = { source: RookSource; kind: RookKind; coin: `0x${string}`; holders: number | null; supplyWei: string | null; complete: boolean; items: RookHolder[]; total: number; limit: number; offset: number };

export type ActivityKind = "buy" | "sell" | "launch" | "graduated" | "raffle-won" | "burned" | "relisted" | "arrived";
export type ActivityItem = {
  at: number | null;
  kind: ActivityKind;
  coin: { address: `0x${string}`; symbol: string; kind: RookKind } | null;
  nest: string | null;
  text: string;
  /** the pair asset moved, for a trade */
  amountWei: string | null;
  tx: `0x${string}` | null;
};
export type Activity = { source: RookSource; at: number; items: ActivityItem[] };

/** One wallet, as the board knows it: what it launched, what it holds, what it traded lately, what it won. An address, not a person. */
export type WalletCoin = { address: `0x${string}`; symbol: string; name: string; kind: RookKind; logoUrl: string | null };
export type WalletProfile = {
  source: RookSource;
  address: `0x${string}`;
  at: number;
  launches: BoardRow[];
  /** every coin on the board it holds a positive balance of, largest share first; `valueEth` at the latest price, null without one */
  holdings: { coin: WalletCoin; balanceWei: string; share: number | null; valueEth: number | null }[];
  /** newest first, at most 50 */
  trades: (RookTrade & { coin: WalletCoin })[];
  /** raffle wins the nests' vaults recorded for this address */
  wins: { at: number | null; nest: string; text: string; tx: `0x${string}` }[];
};
