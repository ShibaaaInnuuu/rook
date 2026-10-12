/**
 * Dollars per SOL. Live: Pyth's Hermes (the SOL/USD feed) every minute, CoinGecko when Hermes does not answer. History (a backfill
 * reaching back before the service started): Pyth Benchmarks' hourly bars, one call per range. Stored per minute / hour in `sol_usd`;
 * a trade's dollar price uses the rate of its own time (nearest within 2 h), and a time with no rate near it has no dollar price.
 * A rate is never invented: a source that fails leaves a gap, and the chart's `usd` series is then null.
 */
import type { DatabaseSync } from "node:sqlite";
import type { Fetch } from "./rpc.js";

export const SOL_USD_FEED = "ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d";
export const NEAR_S = 7_200;
const ok = (x: unknown): x is number => typeof x === "number" && Number.isFinite(x) && x > 0;

export async function liveSolUsd(f: Fetch = fetch): Promise<{ usd: number; src: string } | null> {
  try {
    const r = await f(`https://hermes.pyth.network/v2/updates/price/latest?ids[]=${SOL_USD_FEED}&parsed=true`, { signal: AbortSignal.timeout(8_000) });
    if (r.ok) {
      const j = (await r.json()) as { parsed?: { price?: { price?: string; expo?: number } }[] };
      const p = j.parsed?.[0]?.price, usd = p ? Number(p.price) * 10 ** Number(p.expo) : NaN;
      if (ok(usd)) return { usd, src: "pyth" };
    }
  } catch { /* next source */ }
  try {
    const r = await f("https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd", { signal: AbortSignal.timeout(8_000) });
    if (r.ok) { const usd = ((await r.json()) as { solana?: { usd?: number } }).solana?.usd; if (ok(usd)) return { usd, src: "coingecko" }; }
  } catch { /* none */ }
  return null;
}

/** Hourly closes for [from, to] (unix s) from Pyth Benchmarks' TradingView shim. */
export async function historySolUsd(from: number, to: number, f: Fetch = fetch): Promise<{ t: number; usd: number }[]> {
  try {
    const u = `https://benchmarks.pyth.network/v1/shims/tradingview/history?symbol=Crypto.SOL%2FUSD&resolution=60&from=${Math.floor(from)}&to=${Math.ceil(to)}`;
    const r = await f(u, { signal: AbortSignal.timeout(15_000) });
    if (!r.ok) return [];
    const j = (await r.json()) as { s?: string; t?: number[]; c?: number[] };
    if (j.s !== "ok" || !j.t || !j.c) return [];
    return j.t.map((t, i) => ({ t, usd: j.c![i] })).filter((x) => ok(x.usd));
  } catch { return []; }
}

export function storeRate(db: DatabaseSync, t: number, usd: number, src: string): void {
  db.prepare(`INSERT OR REPLACE INTO sol_usd (t, usd, src) VALUES (?, ?, ?)`).run(t, usd, src);
}
export function latestRate(db: DatabaseSync, now: number, maxAge = 3_600): { usd: number; at: number } | null {
  const r = db.prepare(`SELECT t, usd FROM sol_usd WHERE src != 'pyth-history' ORDER BY t DESC LIMIT 1`).get() as { t: number; usd: number } | undefined;
  return r && now - r.t <= maxAge ? { usd: r.usd, at: r.t } : null;
}

/** A lookup over [from, to]: the rate nearest each time (within NEAR_S), else null. Loads the window once. */
export function rateLookup(db: DatabaseSync, from: number, to: number): (ts: number) => number | null {
  const rows = db.prepare(`SELECT t, usd FROM sol_usd WHERE t BETWEEN ? AND ? ORDER BY t`).all(from - NEAR_S, to + NEAR_S) as { t: number; usd: number }[];
  return (ts: number) => {
    if (!rows.length) return null;
    let lo = 0, hi = rows.length - 1;
    while (lo < hi) { const mid = (lo + hi + 1) >> 1; if (rows[mid].t <= ts) lo = mid; else hi = mid - 1; }
    const cands = [rows[lo], rows[lo + 1]].filter(Boolean);
    let best: { t: number; usd: number } | null = null;
    for (const c of cands) if (Math.abs(c.t - ts) <= NEAR_S && (!best || Math.abs(c.t - ts) < Math.abs(best.t - ts))) best = c;
    return best ? best.usd : null;
  };
}

/** Fills hourly history for [from, to] where the table has nothing near; one Benchmarks call per missing range. */
export async function ensureHistory(db: DatabaseSync, from: number, to: number, f: Fetch = fetch): Promise<number> {
  const have = (db.prepare(`SELECT count(*) AS n FROM sol_usd WHERE src = 'pyth-history' AND t BETWEEN ? AND ?`).get(from - 3_600, to + 3_600) as { n: number }).n;
  const hours = Math.max(1, Math.ceil((to - from) / 3_600));
  if (have >= hours * 0.9) return 0;
  const bars = await historySolUsd(from - 3_600, to + 3_600, f);
  const ins = db.prepare(`INSERT OR IGNORE INTO sol_usd (t, usd, src) VALUES (?, ?, 'pyth-history')`);
  for (const b of bars) ins.run(b.t, b.usd);
  return bars.length;
}
