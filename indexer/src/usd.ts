import type { DatabaseSync } from "node:sqlite";
import type { Abi } from "viem";
import { READS } from "./abis.js";
import { getMeta, setMeta } from "./db.js";
import { tryRead, type Rpc } from "./rpc.js";

/**
 * docs/phase12.md §3. Dollars per ETH, read from one Uniswap v3 pool on the chain we are already following.
 *
 * docs/phase10.md said this chain's ETH had no price the indexer could read. That was wrong, and its amendment says so: the
 * fork carries Robinhood Chain's real state, and the WETH/USDG pool is in it. `slot0().sqrtPriceX96` squared is the price of
 * token0 in token1, WETH is token0, USDG has 6 decimals against WETH's 18, so the ratio is scaled by 1e12 to reach whole
 * dollars per whole ETH. Verified against the staging RPC on 2026-09-21: $2,785.14.
 *
 * Three honesty rules hold this file together, and every one of them is a null rather than a number:
 *   - a pool that will not read gives `null`, never a remembered guess and never a default;
 *   - a chain with no such pool gives `null`, and every consumer falls back to the pair asset;
 *   - a rate older than RATE_MAX_AGE_S gives `null`, because a stale spot read is not a rate.
 * It is a spot read of one pool at one block. It is not a market data feed and nothing here may present it as one.
 */

/** The WETH/USDG pool on Robinhood Chain. A chain that does not have it simply has no dollar rate. */
export const USD_POOL = "0x52e65b17fb6e5ba00ed806f37afcd2daa50271ca";
/** How often the pool is read. Once every few minutes is plenty, and it must never become a read per request. */
export const RATE_EVERY_S = 300;
/** Past this, the cached rate stops being served at all: an indexer that lost its node must not quote an old price. */
export const RATE_MAX_AGE_S = 3_600;

const Q96 = 2 ** 96;
/** WETH is token0 and USDG has 6 decimals, so the raw ratio is scaled by 1e18 / 1e6. */
export const USD_SCALE = 1e12;

/**
 * `(sqrtPriceX96 / 2^96)^2 × 1e12`. The double is exact to about fifteen digits, which is more than a dollar figure can show.
 * Anything that is not a positive finite number — a pool that has never been initialised, a word of zeros — is null.
 */
export function usdPerEthFrom(sqrtPriceX96: bigint | null): number | null {
  if (sqrtPriceX96 === null || sqrtPriceX96 <= 0n) return null;
  const r = Number(sqrtPriceX96) / Q96;
  const usd = r * r * USD_SCALE;
  return Number.isFinite(usd) && usd > 0 ? usd : null;
}

export type Rate = { usdPerEth: number | null; at: number | null; pool: string | null };
const KEY = "usd_per_eth", AT = "usd_per_eth_at", POOL = "usd_per_eth_pool", TRIED = "usd_per_eth_tried";

/** What is cached, if it is still young enough to be worth saying. `at` is kept even when the rate has gone stale, so a reader can see how old. */
export function rateOf(db: DatabaseSync, nowS: number): Rate {
  const raw = getMeta(db, KEY), at = getMeta(db, AT);
  const seconds = at === null ? null : Number(at);
  const usd = raw === null ? null : Number(raw);
  const fresh = usd !== null && Number.isFinite(usd) && usd > 0 && seconds !== null && nowS - seconds <= RATE_MAX_AGE_S;
  return { usdPerEth: fresh ? usd : null, at: seconds, pool: fresh ? getMeta(db, POOL) : null };
}

/**
 * Read the pool if it has not been read lately, and cache what came back. The gate is on the ATTEMPT, not on the success, so
 * a pool that is unreadable costs one `eth_call` every RATE_EVERY_S rather than one every pass. Returns what a reader would
 * now get. Never throws: a dollar rate is the least important thing this process does and must not break an indexed answer.
 */
export async function refreshRate(rpc: Rpc, db: DatabaseSync, pool: string | null, nowS: number, everyS = RATE_EVERY_S): Promise<Rate> {
  if (pool === null) return { usdPerEth: null, at: null, pool: null };
  const tried = getMeta(db, TRIED);
  if (tried !== null && nowS - Number(tried) < everyS) return rateOf(db, nowS);
  setMeta(db, TRIED, String(nowS));
  const slot0 = await tryRead<readonly [bigint, number, number, number, number, number, boolean]>(rpc, pool, READS as Abi, "slot0");
  const usd = usdPerEthFrom(slot0 === null ? null : slot0[0]);
  if (usd === null) return rateOf(db, nowS);                  // the previous reading stands until it goes stale; nothing is invented
  setMeta(db, KEY, String(usd)); setMeta(db, AT, String(nowS)); setMeta(db, POOL, pool.toLowerCase());
  return { usdPerEth: usd, at: nowS, pool: pool.toLowerCase() };
}
