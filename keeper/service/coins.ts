import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { decodeFunctionResult, encodeFunctionData, type Abi, type Hex } from "viem";
import { FACTORY } from "./abis.js";
import type { ChainRpc } from "./rpc.js";

/**
 * Audit E-04. The factory's coins, indexed once: a coin's address, policy and curve never change after its launch, so each pass
 * reads only the coins launched since the last one (at most `perPass`, `concurrency` at a time) and remembers them in a file. A
 * flood of free launches is absorbed over several passes and can never hold the floor or the caps back (they run first).
 */
export type CoinFacts = { coin: Hex; policy: number; curve: Hex };
type IndexFile = { version: 1; factory: string; coins: CoinFacts[] };

/** `f` over `xs`, at most `n` at a time, results in order. */
export async function mapLimit<T, R>(xs: readonly T[], n: number, f: (x: T, i: number) => Promise<R>): Promise<R[]> {
  const out: R[] = new Array(xs.length);
  let next = 0;
  await Promise.all(Array.from({ length: Math.max(1, Math.min(n, xs.length)) }, async () => {
    for (;;) { const i = next++; if (i >= xs.length) return; out[i] = await f(xs[i], i); }
  }));
  return out;
}

async function read<T>(rpc: ChainRpc, to: Hex, functionName: string, args: unknown[] = []): Promise<T> {
  const out = await rpc.call<Hex>("eth_call", [{ to, data: encodeFunctionData({ abi: FACTORY as Abi, functionName, args }) }, "latest"]);
  return decodeFunctionResult({ abi: FACTORY as Abi, functionName, data: out }) as T;
}

export class CoinIndex {
  private f: IndexFile;
  constructor(private readonly file: string, private readonly rpc: ChainRpc, private readonly factory: Hex, private readonly perPass = 200, private readonly concurrency = 8) {
    mkdirSync(dirname(file), { recursive: true });
    let f: IndexFile = { version: 1, factory: factory.toLowerCase(), coins: [] };
    if (existsSync(file)) {
      try { const j = JSON.parse(readFileSync(file, "utf8")) as IndexFile; if (j.version === 1 && j.factory === factory.toLowerCase() && Array.isArray(j.coins)) f = j; } catch { /* rebuilt */ }
    }
    this.f = f;
  }

  get coins(): readonly CoinFacts[] { return this.f.coins; }

  /** Index the coins launched since last time, up to `perPass` of them. `behind`: launches still to index (a later pass). */
  async refresh(): Promise<{ added: number; behind: number }> {
    const count = Number(await read<bigint>(this.rpc, this.factory, "coinCount"));
    const from = this.f.coins.length, to = Math.min(count, from + this.perPass);
    if (to > from) {
      const ids = Array.from({ length: to - from }, (_, k) => from + k);
      const added = await mapLimit(ids, this.concurrency, async (i) => {
        const coin = await read<Hex>(this.rpc, this.factory, "coins", [BigInt(i)]);
        const [policy, curve] = await Promise.all([read<number>(this.rpc, this.factory, "policyOf", [coin]), read<Hex>(this.rpc, this.factory, "routerOf", [coin])]);
        return { coin, policy: Number(policy), curve };
      });
      this.f = { ...this.f, coins: [...this.f.coins, ...added] };
      const tmp = `${this.file}.tmp`;
      writeFileSync(tmp, JSON.stringify(this.f));
      renameSync(tmp, this.file);
    }
    return { added: to - from, behind: count - to };
  }
}
