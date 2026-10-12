/**
 * The Solana indexer's API, as types: docs/v9/indexer-solana.md §1–§2, field for field. Base path /v1/rook/*, coins addressed by
 * mint, times in unix seconds, `null` = not known. Quote amounts are exact base units in `*Lamports` strings plus a number in whole
 * quote (the field names date from the SOL-quoted curves; canister-only Rook quotes a collection coin in its wX, 9 decimals as SOL,
 * so a wX-quoted coin's `*Sol` figures read as wX: src/rook/sol/market.ts carries the quote); token amounts are base units in
 * `*Raw` strings plus `decimals`. The site reads these through src/rook/sol/indexer.ts; demo fixtures (src/rook/sol/demoMarket.ts)
 * are built to the same shapes.
 */
export type IxSource = { kind: "live" | "fixture"; note: string };
export type IxPhase = "trading" | "graduating" | "graduated";
export type IxRes = "1m" | "5m" | "15m" | "1h" | "1d";

export type IxCoin = {
  kind: "dbc";
  address: string;
  mint: string;
  pool: string;
  dammPool: string | null;
  config: string;
  collection: { slug: string; listingId: number | null };
  nest: { slug: string; listingId: number | null; name: null; symbol: null };
  name: string; symbol: string;
  logoUrl: string | null;
  description: string | null; links: { website: string | null; twitter: string | null; telegram: string | null };
  metadataUri: string | null;
  creator: string;
  decimals: number;
  supplyRaw: string | null;
  phase: IxPhase;
  graduated: boolean;
  launchedAt: number | null;
  graduatedAt: number | null;
  lastTradeAt: number | null;
  curve: { raisedLamports: string; thresholdLamports: string; progress: number } | null;
  raise: { raisedLamports: string; raiseLamports: string } | null;
  price: number | null;
  priceUsd: number | null;
  change24h: number | null;
  marketCapSol: number | null; marketCapUsd: number | null;
  volume24hLamports: string; volume24hSol: number; volume24hUsd: number | null;
  trades24h: number; buys24h: number; sells24h: number;
  holders: number | null;
  holdersAt: number | null;
};
export type IxSort = "new" | "graduating" | "graduated" | "trending" | "mcap" | "active";
export type IxCoinList = { source: IxSource; at: number; solUsd: number | null; sort: string; collection: string | null; total: number; limit: number; offset: number; items: IxCoin[] };
export type IxBoard = { source: IxSource; at: number; usdPerSol: number | null; rows: IxCoin[] };
export type IxGraduation = { at: number | null; slot: number; tx: string; pool: string | null };
export type IxCoinDetail = IxCoin & { tradesAllTime: number; graduation: IxGraduation | null; solUsd: number | null };

export type IxCandle = { t: number; o: number; h: number; l: number; c: number; v: number; n: number };
export type IxChart = {
  source: IxSource; kind: "dbc"; coin: string; res: IxRes; seconds: number; span: number;
  /** what the candles are priced in: the collection's wX symbol, or "SOL" (ROOK) */
  quote: string;
  candles: IxCandle[];
  usd: IxCandle[] | null;
  trades: number; tradesAllTime: number;
  launchedAt: number | null;
  graduation: IxGraduation | null;
  supplyRaw: string | null; decimals: number;
  usdPerSol: number | null;
};

export type IxTrade = {
  id: string;
  side: "buy" | "sell";
  venue: "curve" | "pool";
  solLamports: string; sol: number;
  tokenRaw: string; tokens: number;
  feeLamports: string | null;
  price: number;
  priceUsd: number | null;
  trader: string;
  signature: string; tx: string;
  slot: number; at: number | null;
};
export type IxTradePage = { source: IxSource; kind: "dbc"; coin: string; items: IxTrade[]; next: string | null; total: number };

export type IxHolder = { rank: number; address: string; tokenAccount: string; amountRaw: string; amount: number; share: number | null; label: "creator" | "curve" | "pool" | null };
export type IxHolderPage = {
  source: IxSource; kind: "dbc"; coin: string; holders: number | null; supplyRaw: string | null; decimals: number;
  complete: boolean; refreshedAt: number | null; items: IxHolder[]; total: number; limit: number; offset: number;
};

export type IxActivityItem = {
  at: number | null; kind: "buy" | "sell" | "launch" | "graduated";
  coin: { address: string; symbol: string; name: string; logoUrl: string | null; collection: string };
  text: string; solLamports: string | null; trader: string | null; tx: string | null;
};
export type IxActivity = { source: IxSource; at: number; items: IxActivityItem[] };

/** The resolution tabs and each one's window (§2.6). */
export const RESES: [IxRes, string, number, number][] = [["1m", "1m", 60, 240], ["5m", "5m", 300, 240], ["15m", "15m", 900, 240], ["1h", "1h", 3_600, 240], ["1d", "1d", 86_400, 365]];
