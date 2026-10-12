import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Hex } from "viem";
import type { TransferLog } from "../../tools/snapshot/src/core.js";
import { transferLogsWith, type RawLog } from "../../tools/snapshot/src/rpc.js";
import type { ChainRpc } from "./rpc.js";

/**
 * Audit E-11 and E-04. A coin's Transfer logs, read incrementally: each coin's logs are kept in `<dir>/<coin>.json` up to a cursor,
 * so a draw reads only the blocks since the last one instead of re-scanning from the stack's first block. Only CONFIRMED blocks
 * (the caller's `confirmed`, some minutes behind the head) are written down, so a reorg near the head cannot poison the cache.
 * Ranges are split by tools/snapshot's `transferLogsWith` when a node refuses them as too large (the 10,000-log cap), and any
 * other failure THROWS: no snapshot is ever built over logs that were not all read.
 */
const TRANSFER = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
type Row = [number, number, string, string, string];   // block, log index, from, to, value (hex)
type CacheFile = { version: 1; coin: string; fromBlock: number; cursor: number; logs: Row[] };

export class LogCache {
  constructor(private readonly dir: string, private readonly rpc: ChainRpc, private readonly step = 200_000) { mkdirSync(dir, { recursive: true }); }

  private path(coin: string): string { return join(this.dir, `${coin.toLowerCase()}.json`); }

  private load(coin: string, fromBlock: number): CacheFile {
    const empty: CacheFile = { version: 1, coin: coin.toLowerCase(), fromBlock, cursor: fromBlock - 1, logs: [] };
    if (!existsSync(this.path(coin))) return empty;
    try {
      const j = JSON.parse(readFileSync(this.path(coin), "utf8")) as CacheFile;
      return j.version === 1 && j.coin === coin.toLowerCase() && j.fromBlock === fromBlock && Array.isArray(j.logs) ? j : empty;
    } catch { return empty; }            // a damaged cache is rebuilt from the chain, never trusted
  }

  private save(f: CacheFile): void {
    mkdirSync(this.dir, { recursive: true });
    const tmp = `${this.path(f.coin)}.tmp`;
    writeFileSync(tmp, JSON.stringify(f));
    renameSync(tmp, this.path(f.coin));
  }

  /**
   * R3-check L-D: drop what is kept for `coin`, so its next read comes from the chain again. A rebuilt tree that does not prove
   * against the chain's root may come from logs a provider once returned short, and they must not be served from here for good.
   */
  forget(coin: string): void { rmSync(this.path(coin), { force: true }); }

  /** Every Transfer of `coin` in [fromBlock, toBlock], in chain order. Blocks at or under `confirmed` are cached for next time. */
  async upTo(coin: Hex, fromBlock: number, toBlock: number, confirmed: number): Promise<TransferLog[]> {
    const f = this.load(coin, fromBlock);
    const cached = f.logs.filter((r) => r[0] <= toBlock).map(toLog);
    if (toBlock <= f.cursor) return cached;
    const get = (a: bigint, e: bigint) => this.rpc.call<RawLog[]>("eth_getLogs",
      [{ address: coin, topics: [TRANSFER], fromBlock: `0x${a.toString(16)}`, toBlock: `0x${e.toString(16)}` }]);
    const fresh = await transferLogsWith(get, BigInt(f.cursor + 1), BigInt(toBlock), BigInt(this.step));
    const keepTo = Math.min(toBlock, confirmed);
    if (keepTo > f.cursor) {
      this.save({ ...f, cursor: keepTo, logs: [...f.logs, ...fresh.filter((l) => Number(l.blockNumber) <= keepTo).map(toRow)] });
    }
    return [...cached, ...fresh];
  }
}

const toRow = (l: TransferLog): Row => [Number(l.blockNumber), l.logIndex, l.from.toLowerCase(), l.to.toLowerCase(), `0x${l.value.toString(16)}`];
const toLog = (r: Row): TransferLog => ({ blockNumber: BigInt(r[0]), logIndex: r[1], from: r[2], to: r[3], value: BigInt(r[4]) });
