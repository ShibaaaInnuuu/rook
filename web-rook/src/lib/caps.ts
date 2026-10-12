/**
 * docs/phase14.md §3 and §4. A market cap and a 24-hour volume for many coins at once, from the endpoint that has them.
 *
 * The indexer's list endpoint carries a coin's supply but not its price, and a price lives on
 * `/v1/coins/:address/candles` — one request per coin (docs/phase9.md §10 still asks for a batch endpoint). Two screens need
 * more than one coin's worth of that: the sort by market cap, and the Trending strip.
 *
 * Three things keep the cost honest rather than merely hidden:
 *   - the query key is **exactly** the one `useCandles(address, "1d")` uses, so the coins whose cards are already on screen
 *     are shared rather than fetched twice;
 *   - these observers set no `refetchInterval`, so they add no polling of their own — only the cards that are visible poll;
 *   - `enabled` is false until the reader asks, so nobody pays for a sort they did not pick.
 *
 * Nothing here estimates. A coin whose supply has not been read, or whose candles have not arrived, or which has never
 * traded, gets `null` — which the callers print as not known and sort last, never as zero.
 */
import { useQueries } from "@tanstack/react-query";
import { useSource } from "./catalog";
import type { CoinRow } from "./coinlist";
import { capOf, wholeSupply, type CandleSet } from "./chart";

export type CoinReading = {
  /** the latest close in the window: the pair asset per whole coin, curve-side */
  close: number | null;
  /** supply × close, in the PAIR asset — the number a list is ranked on, because it needs no rate */
  capPair: number | null;
  /** the same cap in dollars, when the indexer read a rate; null is "no rate", never a converted guess */
  capUsd: number | null;
  /** the pair asset traded in the window: the same quantity `volume24hWei` reports, measured the other way */
  vol24h: number | null;
};

export type Readings = {
  /** by lower-cased address */
  by: Map<string, CoinReading>;
  /** the caps a sort ranks on, in the pair asset, by lower-cased address */
  caps: Map<string, number | null>;
  /** the 24-hour volumes measured here, by lower-cased address */
  volumes: Map<string, number | null>;
  /** dollars per ETH as the indexer last read it, from whichever response answered first; null when no rate is known */
  usdPerEth: number | null;
  usdPerEthAt: number | null;
  /** true while any of the requests is still out: a page says the prices are being read rather than showing a wrong order */
  loading: boolean;
  /** how many answered at all, so a caller can say "of the N it could read" rather than implying it read them all */
  answered: number;
};

const readingOf = (set: CandleSet | undefined): CoinReading => {
  if (!set) return { close: null, capPair: null, capUsd: null, vol24h: null };
  const last = set.candles[set.candles.length - 1];
  const close = last ? last.c : null;
  return {
    close,
    capPair: capOf(set, close, "pair"),
    capUsd: wholeSupply(set.supplyWei) === null ? null : capOf(set, close, "usd"),
    // a coin whose candles arrived and are empty traded nothing in the window, which is 0 and not unknown
    vol24h: set.candles.reduce((t, k) => t + k.v, 0),
  };
};

/** Reads a day of candles for each row, as one batch of queries. `enabled: false` makes no request at all. */
export function useReadings(rows: CoinRow[], enabled = true): Readings {
  const src = useSource();
  const on = enabled && !!src;
  const results = useQueries({
    queries: rows.map((r) => ({
      queryKey: ["candles", src?.label, r.address, "1d"] as const,
      enabled: on,
      retry: 0,
      staleTime: 30_000,
      queryFn: () => src!.get<CandleSet>(`/v1/coins/${r.address}/candles?range=1d`),
    })),
  });

  const by = new Map<string, CoinReading>();
  const caps = new Map<string, number | null>();
  const volumes = new Map<string, number | null>();
  let usdPerEth: number | null = null, usdPerEthAt: number | null = null, loading = false, answered = 0;

  results.forEach((q, i) => {
    const key = rows[i].address.toLowerCase();
    if (on && q.isLoading) loading = true;
    const set = q.data as CandleSet | undefined;
    if (set) {
      answered++;
      if (usdPerEth === null && Number.isFinite(set.usdPerEth as number)) { usdPerEth = set.usdPerEth; usdPerEthAt = set.usdPerEthAt; }
    }
    const reading = readingOf(set);
    by.set(key, reading);
    caps.set(key, reading.capPair);
    volumes.set(key, set ? reading.vol24h : null);
  });

  return { by, caps, volumes, usdPerEth, usdPerEthAt, loading, answered };
}

/**
 * Dollars per ETH, for a page that has no coin of its own to show.
 *
 * The rate is a property of the chain and not of a coin (docs/phase12.md §3), but the indexer publishes it on a chart
 * response, so that is where it is read from: one request, against the newest coin, under the key a card already uses. Null
 * when there is no coin, no pool, or a rate that has gone stale — in which case a page draws no dollar figure at all rather
 * than converting at a number it invented.
 */
export function useUsdRate(rows: CoinRow[]): { usdPerEth: number | null; at: number | null; loading: boolean } {
  // three rather than one: a recorded demo keeps only some coins' charts, so the newest coin's may not be among them
  const few = rows.slice(0, 3);
  const r = useReadings(few, few.length > 0);
  return { usdPerEth: r.usdPerEth, at: r.usdPerEthAt, loading: r.loading };
}
