import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { decodeFunctionData, encodeFunctionData, encodeFunctionResult, type Abi, type Hex } from "viem";
import type * as T from "../../types.js";
import { APES, buildWorld, FROGS, NOW, T0, worldConfig } from "../../test/wrappedWorld.js";
import { parseConfig } from "../wrapped/config.js";
import { WrappedService } from "../wrapped/service.js";
import { V4_4663 } from "../wrapped/config.js";
import { CURVE_TRADE_EVENTS, CURVE_VIEWS, ERC20_EVENTS, ERC20_VIEWS, FACTORY_VIEWS, LOCK_VIEWS, POOL_SWAP } from "./abis.js";
import { marketOf, ponsRow } from "./board.js";
import { RookService } from "./service.js";
import type { Board, BoardRow, RookRes, RookSource } from "./types.js";

/**
 * docs/v6/site-design.md §6. Writes indexer/fixtures/rook/: the board and the coin pages' chart, tape and holders, for the
 * site's labelled demo (?demo=sample). Two sources, both already fixtures:
 *   - the pons coins come from indexer/fixtures/demo (a recording of the staging fork: SIMULATED trades), composed here
 *     exactly as the live service composes them, from `/v1/coins` and each coin's day of candles;
 *   - the Rook coins come from test/wrappedWorld.ts (docs/v3/curve.md's worked examples), given a handful of INVENTED trades
 *     walked along the worked curve, and read by the REAL CurveReader over that world.
 * Every file says `source.kind: "fixture"`. Times are on the demo recording's clock, so the site shifts both sets alike.
 *   npm run fixtures:rook            then copy the folder to web-rook/public/fixtures/rook (web-rook/test/api.test.ts checks the copy)
 */
const E = 10n ** 18n;
const NOTE = "FIXTURE. Nothing on this page is deployed. The Rook coins (ALIEN, BETA, APEA, APEB) are docs/v3/curve.md's worked examples with a few invented trades walked along "
  + "the worked curve; the other coins are a recording of our staging fork, where a simulator trades. Every price is invented or simulated; nothing was bought; no wallet here is anybody.";
const here = new URL("./", import.meta.url).pathname;
const demoDir = `${here}../../fixtures/demo/`, outDir = `${here}../../fixtures/rook/`;
const demoIndex = JSON.parse(readFileSync(`${demoDir}index.json`, "utf8")) as { files: Record<string, string> };
const readDemo = <X,>(path: string): X | null => { const f = demoIndex.files[path]; return f && existsSync(demoDir + f) ? (JSON.parse(readFileSync(demoDir + f, "utf8")) as X) : null; };
const health = readDemo<T.Health>("/v1/health")!;
const CLOCK = health.lastPassAt!;                 // the demo's clock; the world's times are moved onto it
const SHIFT = CLOCK - NOW;

// ---- the pons coins, composed from the demo recording ----------------------------------------------------------------------
const coinsPage = readDemo<T.Page<T.CoinSummary>>("/v1/coins?limit=200")!;
const ponsRows: BoardRow[] = coinsPage.items.map((c) => {
  const day = readDemo<T.Chart>(`/v1/coins/${c.address}/candles?range=1d`);
  const trades = (day?.marks ?? []).map((t) => ({ ...t, at: t.at })).sort((a, b) => a.block - b.block || a.logIndex - b.logIndex);
  const last = day?.candles.at(-1), first = day?.candles[0];
  const change = first && last && first.o > 0 && day!.candles.length > 1 ? (last.c - first.o) / first.o : null;
  // the recording keeps a chart's largest trades only, so the buys and the new holders of the day are not known from it: null, never a guess
  return ponsRow(c, { price: last ? last.c : null, change24h: change, trades24h: day ? day.trades : null, buys24h: null, newHolders24h: null, graduatedAt: day?.graduation?.at ?? null });
  void trades; void marketOf;
});

// ---- the Rook coins: the worked world, with trades walked along the worked curve --------------------------------------------
const w = buildWorld();
w.failing.add("http:fixture-provider-b");
const poolManager = V4_4663.poolManager;
const buyers = ["0x4000000000000000000000000000000000000001", "0x4000000000000000000000000000000000000002", "0x4000000000000000000000000000000000000003", "0x4000000000000000000000000000000000000004", "0x4000000000000000000000000000000000000005"];
const ceilDiv = (a: bigint, b: bigint) => (a + b - 1n) / b;
/** FrogCurve's constant product, integer for integer (web/src/lib/quote.ts `curveBuy`/`curveSell`), from the worked start (2.790 ETH virtual, 1.246e9 tokens). */
function walk(curve: string, coin: string, startBlock: number, steps: { buyer: string; ethIn?: bigint; tokensIn?: bigint }[], feeBps = 100n) {
  let v = 2_790_000_000_000_000_000n, t = 1_246_000_000n * E, block = startBlock;
  const held = new Map<string, bigint>();
  for (const s of steps) {
    block += 7;
    if (s.ethIn !== undefined) {
      const fee = (s.ethIn * feeBps) / 10_000n, net = s.ethIn - fee, out = t - ceilDiv(v * t, v + net);
      v += net; t -= out; held.set(s.buyer, (held.get(s.buyer) ?? 0n) + out);
      w.event(curve, CURVE_TRADE_EVENTS as Abi, "Bought", { buyer: s.buyer, ethIn: s.ethIn, fee, tokensOut: out }, block);
      w.event(coin, ERC20_EVENTS as Abi, "Transfer", { from: curve, to: s.buyer, value: out }, block);
    } else if (s.tokensIn !== undefined) {
      const gross = v - ceilDiv(v * t, t + s.tokensIn), fee = (gross * feeBps) / 10_000n;
      v -= gross; t += s.tokensIn; held.set(s.buyer, (held.get(s.buyer) ?? 0n) - s.tokensIn);
      w.event(curve, CURVE_TRADE_EVENTS as Abi, "Sold", { seller: s.buyer, tokensIn: s.tokensIn, ethOut: gross - fee, fee }, block);
      w.event(coin, ERC20_EVENTS as Abi, "Transfer", { from: s.buyer, to: curve, value: s.tokensIn }, block);
    }
  }
  return { v, t, block, held };
}
// BETA: on its curve, 1.2 ETH in over a few buys and one sell (the worked example's state)
const beta = walk(FROGS.betaCurve, FROGS.beta, 310, [
  { buyer: buyers[0], ethIn: 300_000_000_000_000_000n }, { buyer: buyers[1], ethIn: 250_000_000_000_000_000n }, { buyer: buyers[2], ethIn: 400_000_000_000_000_000n },
  { buyer: buyers[1], tokensIn: 60_000_000n * E }, { buyer: buyers[3], ethIn: 200_000_000_000_000_000n }, { buyer: buyers[4], ethIn: 120_000_000_000_000_000n },
]);
// ALIEN: the whole 5 ETH raise on the curve (graduated at block 120), then a few swaps in its pool
const alien = walk(FROGS.alienCurve, FROGS.alien, 12, Array.from({ length: 10 }, (_, i) => ({ buyer: buyers[i % 5], ethIn: 505_000_000_000_000_000n })));
w.event(FROGS.alienCurve, CURVE_TRADE_EVENTS as Abi, "Graduated", { unitsToCoinPool: 4n * E, unitsToShared: 2n * E, ethToShared: E }, 120);
const alienKey = { currency0: FROGS.alien as Hex, currency1: FROGS.wrapper as Hex, fee: 10_000, tickSpacing: 200, hooks: "0x0000000000000000000000000000000000000000" as Hex };
const { keccak256, encodeAbiParameters } = await import("viem");
const alienPool = keccak256(encodeAbiParameters([{ type: "address" }, { type: "address" }, { type: "uint24" }, { type: "int24" }, { type: "address" }], [alienKey.currency0, alienKey.currency1, alienKey.fee, alienKey.tickSpacing, alienKey.hooks]));
// a swap: amount0 is the coin's leg (negative = the pool paid coin out = a buy), amount1 the wX leg; prices near the worked 200M coins / 5.453 wX pool
const swaps: [bigint, bigint, number][] = [[-3_000_000n * E, 82_000_000_000_000_000n, 700], [2_000_000n * E, -53_000_000_000_000_000n, 760], [-5_000_000n * E, 140_000_000_000_000_000n, 850], [-1_500_000n * E, 43_000_000_000_000_000n, 940], [900_000n * E, -25_000_000_000_000_000n, 985]];
for (const [a0, a1, block] of swaps) {
  w.event(poolManager, POOL_SWAP as Abi, "Swap", { id: alienPool, sender: FROGS.router, amount0: a0, amount1: a1, sqrtPriceX96: 0n, liquidity: 0n, tick: 0, fee: 10_000 }, block);
  const buyer = buyers[block % 5];
  if (a0 < 0n) w.event(FROGS.alien, ERC20_EVENTS as Abi, "Transfer", { from: poolManager, to: buyer, value: -a0 }, block);
  else w.event(FROGS.alien, ERC20_EVENTS as Abi, "Transfer", { from: buyer, to: poolManager, value: a0 }, block);
}
// APEA graduated with no pool trades yet; APEB is graduating: their curves' raises, walked
walk(APES.coinACurve, APES.coinA, 22, Array.from({ length: 8 }, (_, i) => ({ buyer: buyers[(i + 1) % 5], ethIn: 630_000_000_000_000_000n })));
w.event(APES.coinACurve, CURVE_TRADE_EVENTS as Abi, "Graduated", { unitsToCoinPool: E, unitsToShared: 0n, ethToShared: 0n }, 90);
walk(APES.coinBCurve, APES.coinB, 32, Array.from({ length: 8 }, (_, i) => ({ buyer: buyers[(i + 2) % 5], ethIn: 630_000_000_000_000_000n })));
void beta; void alien;

/** The world answers the wrapped service's reads; the lock's `coinKey` is ours, answered here before the world refuses it. */
const worldRpc = w.rpc;
const curves = new Set([FROGS.alienCurve, FROGS.betaCurve, APES.coinACurve, APES.coinBCurve].map((x) => x.toLowerCase()));
const coins = new Set([FROGS.alien, FROGS.beta, APES.coinA, APES.coinB].map((x) => x.toLowerCase()));
const factories = new Set([FROGS.factory, APES.factory].map((x) => x.toLowerCase()));
const selector = (abi: Abi, fn: string) => encodeFunctionData({ abi, functionName: fn, args: fn === "feeOf" ? [FROGS.alien] : [] }).slice(0, 10);
const rpc: typeof worldRpc = async <X,>(method: string, params: unknown[]): Promise<X> => {
  if (method === "eth_call") {
    const { to, data } = params[0] as { to: string; data: Hex }, at = to.toLowerCase();
    if (at === FROGS.lock.toLowerCase() && decodeFunctionData({ abi: LOCK_VIEWS, data }).functionName === "coinKey") return encodeFunctionResult({ abi: LOCK_VIEWS, functionName: "coinKey", result: alienKey }) as X;
    // the worked coins' fixed terms: a 1% tier, a supply of one billion (docs/v3/curve.md's template)
    if (curves.has(at) && data.startsWith(selector(CURVE_VIEWS as Abi, "feeBps"))) return encodeFunctionResult({ abi: CURVE_VIEWS, functionName: "feeBps", result: 100n }) as X;
    if (factories.has(at) && data.startsWith(selector(FACTORY_VIEWS as Abi, "feeOf"))) return encodeFunctionResult({ abi: FACTORY_VIEWS, functionName: "feeOf", result: 100n }) as X;
    if (coins.has(at) && data.startsWith(selector(ERC20_VIEWS as Abi, "totalSupply"))) return encodeFunctionResult({ abi: ERC20_VIEWS, functionName: "totalSupply", result: 1_000_000_000n * E }) as X;
  }
  return worldRpc<X>(method, params);
};

const market = JSON.parse(readFileSync(`${here}../../fixtures/wrapped/wrapped-frogs.json`, "utf8")).market as T.WrappedMarket;
const apesMarket = JSON.parse(readFileSync(`${here}../../fixtures/wrapped/wrapped-apes.json`, "utf8")).market as T.WrappedMarket;
const source: RookSource = { kind: "fixture", note: NOTE };
const wrapped = new WrappedService({ config: parseConfig({ ...worldConfig(), source: { kind: "fixture", note: NOTE }, market: { frogs: market, apes: apesMarket } }), rpc, fetch: w.fetch, now: () => NOW });
const wrappedConfig = parseConfig(worldConfig());
const svc = new RookService({ wrapped, wrappedConfig, rpc, now: () => NOW, source });

/** Every time in the world's answers moved onto the demo's clock. */
const TIME_KEYS = new Set(["at", "launchedAt", "graduatedAt", "t"]);
const shift = <X,>(x: X): X => {
  if (x === null || typeof x !== "object") return x;
  if (Array.isArray(x)) return x.map(shift) as X;
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(x as Record<string, unknown>)) out[k] = TIME_KEYS.has(k) && typeof v === "number" ? v + SHIFT : shift(v);
  return out as X;
};

mkdirSync(outDir, { recursive: true });
for (const f of readdirSync(outDir)) rmSync(outDir + f);
const files: Record<string, string> = {};
const put = (path: string, file: string, body: unknown) => { files[path] = file; writeFileSync(outDir + file, JSON.stringify(body, null, 2) + "\n"); };

const curveBoard = shift(await svc.board());
const board: Board = { source, at: CLOCK, usdPerEth: readDemo<T.Chart>(`/v1/coins/${coinsPage.items[0].address}/candles?range=1d`)?.usdPerEth ?? null, rows: [...curveBoard.rows, ...ponsRows] };
put("/v1/rook/board", "board.json", board);
const activity = shift(await svc.activity(80));
// the pons coins' trades and launches, from the recording, into the same ticker
for (const c of coinsPage.items.slice(0, 12)) {
  const day = readDemo<T.Chart>(`/v1/coins/${c.address}/candles?range=1d`);
  for (const t of (day?.marks ?? []).slice(-6)) activity.items.push({ at: t.at, kind: t.side, coin: { address: c.address, symbol: c.symbol ?? "?", kind: "pons" }, nest: c.listingName, text: `${t.side === "buy" ? "bought" : "sold"} ${c.symbol} for ${(Number(BigInt(t.quoteWei)) / 1e18).toFixed(4)} ETH`, amountWei: t.quoteWei, tx: t.tx });
  if (c.launchedAt !== null) activity.items.push({ at: c.launchedAt, kind: "launch", coin: { address: c.address, symbol: c.symbol ?? "?", kind: "pons" }, nest: c.listingName, text: `${c.symbol} launched on ${c.listingName ?? "its vault"}`, amountWei: null, tx: null });
}
activity.items.sort((a, b) => (b.at ?? 0) - (a.at ?? 0)); activity.items = activity.items.slice(0, 80); activity.at = CLOCK;
put("/v1/rook/activity?limit=50", "activity.json", activity);
for (const row of curveBoard.rows) {
  const a = row.address;
  for (const res of ["1m", "5m", "15m", "1h", "1d"] as RookRes[]) put(`/v1/rook/coins/${a}/chart?res=${res}`, `chart-${a.toLowerCase()}-${res}.json`, shift(await svc.chart(a, res)));
  put(`/v1/rook/coins/${a}/trades?limit=25`, `trades-${a.toLowerCase()}.json`, shift(await svc.trades(a, 25)));
  put(`/v1/rook/coins/${a}/holders?limit=25`, `holders-${a.toLowerCase()}.json`, shift(await svc.holders(a, 25, 0)));
}
for (const who of buyers) put(`/v1/rook/wallets/${who}`, `wallet-${who.toLowerCase()}.json`, shift(await svc.wallet(who)));
writeFileSync(outDir + "index.json", JSON.stringify({ kind: "rook", note: NOTE, generatedAt: CLOCK, shiftedFromWorldBy: SHIFT, worldT0: T0, files }, null, 2) + "\n");
console.log(`wrote ${Object.keys(files).length} answers to ${outDir}`);
