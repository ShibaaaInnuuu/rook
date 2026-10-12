import { decodeEventLog, encodeAbiParameters, encodeEventTopics, keccak256, type Abi, type Hex } from "viem";
import { curvePrice, poolPrice } from "../candles.js";
import { hex, tryRead, type RawLog, type Rpc } from "../rpc.js";
import { CURVE_TRADE_EVENTS, ERC20_EVENTS, LOCK_VIEWS, POOL_SWAP } from "./abis.js";
import type { RookTrade } from "./types.js";

/**
 * A Rook coin's market, read from its own logs and nothing else: the FrogCurve's `Bought` / `Sold` while it is on the curve,
 * the coin's `Transfer`s for who holds it, `Graduated` for when the curve ended, and — once the LiquidityLock names the coin's
 * pool — the PoolManager's `Swap`s for that pool. Reads only; cached per coin and read incrementally. Nothing here is folded
 * into the database: the v2 follower is another agent's file, and a curve coin's history is short enough to read on demand.
 */
export type CurveCoin = { coin: Hex; curve: Hex; fromBlock: number; lock?: Hex | null; poolManager?: Hex | null };
export type CurveRead = {
  /** chain order, oldest first */
  trades: RookTrade[];
  balances: Map<string, bigint>;
  /** when each address first received the coin: the board's holder-growth term */
  firstIn: Map<string, number | null>;
  graduation: { at: number | null; block: number; tx: Hex } | null;
  poolId: Hex | null;
  upTo: number;
};
type State = CurveRead & { at: number; coinIs0: boolean | null };

const ZERO = "0x0000000000000000000000000000000000000000";
const abs = (x: bigint) => (x < 0n ? -x : x);
const topicsOf = (abi: Abi) => abi.filter((x) => x.type === "event").map((e) => encodeEventTopics({ abi, eventName: (e as { name: string }).name } as never)[0] as Hex);

export class CurveReader {
  private readonly states = new Map<string, State>();
  private readonly times = new Map<number, number | null>();
  private readonly now: () => number;
  constructor(private readonly rpc: Rpc, private readonly o: { chunk?: number; confirmations?: number; ttlS?: number; now?: () => number } = {}) {
    this.now = o.now ?? (() => Math.floor(Date.now() / 1000));
  }

  async head(): Promise<number> {
    const h = Number(BigInt(await this.rpc<string>("eth_blockNumber", [])));
    return Math.max(0, h - (this.o.confirmations ?? 20));
  }
  private async timeOf(block: number): Promise<number | null> {
    if (this.times.has(block)) return this.times.get(block)!;
    const b = await this.rpc<{ timestamp: Hex } | null>("eth_getBlockByNumber", [hex(block), false]).catch(() => null);
    const t = b ? Number(BigInt(b.timestamp)) : null;
    if (t !== null) this.times.set(block, t);
    if (this.times.size > 50_000) this.times.clear();
    return t;
  }
  private async logs(address: Hex, abi: Abi, from: number, to: number, extraTopic?: Hex): Promise<RawLog[]> {
    const out: RawLog[] = [], chunk = this.o.chunk ?? 100_000, topics: (Hex[] | Hex | null)[] = [topicsOf(abi)];
    if (extraTopic) topics.push(extraTopic);
    for (let a = from; a <= to; a += chunk) {
      const b = Math.min(to, a + chunk - 1);
      out.push(...(await this.rpc<RawLog[]>("eth_getLogs", [{ address, topics, fromBlock: hex(a), toBlock: hex(b) }])));
    }
    return out.filter((l) => !l.removed).sort((x, y) => Number(BigInt(x.blockNumber) - BigInt(y.blockNumber)) || Number(BigInt(x.logIndex) - BigInt(y.logIndex)));
  }

  /** The coin's state up to the confirmed head, read incrementally from the last read and cached for `ttlS`. */
  async read(c: CurveCoin): Promise<CurveRead> {
    const key = c.coin.toLowerCase();
    const s = this.states.get(key);
    if (s && this.now() - s.at < (this.o.ttlS ?? 20)) return s;
    const head = await this.head();
    const st: State = s ?? { trades: [], balances: new Map(), firstIn: new Map(), graduation: null, poolId: null, coinIs0: null, upTo: c.fromBlock - 1, at: 0 };
    if (head <= st.upTo) { st.at = this.now(); this.states.set(key, st); return st; }
    const from = st.upTo + 1;
    const [curveLogs, transfers] = await Promise.all([this.logs(c.curve, CURVE_TRADE_EVENTS as Abi, from, head), this.logs(c.coin, ERC20_EVENTS as Abi, from, head)]);
    for (const l of curveLogs) {
      const e = decodeEventLog({ abi: CURVE_TRADE_EVENTS, data: l.data, topics: l.topics as [Hex, ...Hex[]] }) as { eventName: string; args: Record<string, bigint | string> };
      const block = Number(BigInt(l.blockNumber)), logIndex = Number(BigInt(l.logIndex)), at = await this.timeOf(block);
      if (e.eventName === "Graduated") { st.graduation = { at, block, tx: l.transactionHash }; continue; }
      const buy = e.eventName === "Bought";
      const quote = BigInt(buy ? e.args.ethIn : e.args.ethOut), coins = BigInt(buy ? e.args.tokensOut : e.args.tokensIn), fee = BigInt(e.args.fee);
      if (coins === 0n) continue;
      st.trades.push({ id: `${block}:${logIndex}`, side: buy ? "buy" : "sell", venue: "curve", quoteWei: String(quote), coinWei: String(coins), feeWei: String(fee), taxWei: null,
        price: curvePrice(buy, quote, coins, fee, 0n), trader: String(buy ? e.args.buyer : e.args.seller) as Hex, tx: l.transactionHash, block, logIndex, at });
    }
    for (const l of transfers) {
      const e = decodeEventLog({ abi: ERC20_EVENTS, data: l.data, topics: l.topics as [Hex, ...Hex[]] }) as { args: { from: string; to: string; value: bigint } };
      const v = e.args.value; if (v === 0n) continue;
      const f = e.args.from.toLowerCase(), t = e.args.to.toLowerCase();
      if (f !== ZERO) st.balances.set(f, (st.balances.get(f) ?? 0n) - v);
      if (t !== ZERO) { st.balances.set(t, (st.balances.get(t) ?? 0n) + v); if (!st.firstIn.has(t)) st.firstIn.set(t, await this.timeOf(Number(BigInt(l.blockNumber)))); }
    }
    // the pool, once there is one: the lock names its key, the id is keccak of it, and the PoolManager's Swaps are read by that id
    if (st.graduation && c.lock && c.poolManager) {
      if (st.poolId === null) {
        const k = await tryRead<{ currency0: Hex; currency1: Hex; fee: number; tickSpacing: number; hooks: Hex }>(this.rpc, c.lock, LOCK_VIEWS as Abi, "coinKey", [c.coin]);
        if (k) {
          st.poolId = keccak256(encodeAbiParameters([{ type: "address" }, { type: "address" }, { type: "uint24" }, { type: "int24" }, { type: "address" }], [k.currency0, k.currency1, k.fee, k.tickSpacing, k.hooks]));
          st.coinIs0 = k.currency0.toLowerCase() === key;
        }
      }
      if (st.poolId) {
        for (const l of await this.logs(c.poolManager, POOL_SWAP as Abi, Math.max(from, st.graduation.block), head, st.poolId)) {
          const e = decodeEventLog({ abi: POOL_SWAP, data: l.data, topics: l.topics as [Hex, ...Hex[]] }) as { args: { id: Hex; sender: string; amount0: bigint; amount1: bigint } };
          if (e.args.id.toLowerCase() !== st.poolId.toLowerCase()) continue;
          const coinAmt = st.coinIs0 ? e.args.amount0 : e.args.amount1, quoteAmt = st.coinIs0 ? e.args.amount1 : e.args.amount0;
          if (coinAmt === 0n) continue;
          const block = Number(BigInt(l.blockNumber)), logIndex = Number(BigInt(l.logIndex));
          st.trades.push({ id: `${block}:${logIndex}`, side: coinAmt < 0n ? "buy" : "sell", venue: "pool", quoteWei: String(abs(quoteAmt)), coinWei: String(abs(coinAmt)), feeWei: null, taxWei: null,
            price: poolPrice(quoteAmt, coinAmt), trader: e.args.sender as Hex, tx: l.transactionHash, block, logIndex, at: await this.timeOf(block) });
        }
        st.trades.sort((x, y) => x.block - y.block || x.logIndex - y.logIndex);
      }
    }
    st.upTo = head; st.at = this.now();
    this.states.set(key, st);
    return st;
  }
}
