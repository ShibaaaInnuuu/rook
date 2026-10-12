// The W13 tests' shared world: the canister's own fixtures (the desk's landed DLMM pair, the books_view bytes its tests write), a
// fake read-only RPC answering from a table, a fake canister reader, and the synthetic account bytes a config read needs.
import { readFileSync } from "node:fs";
import type { ParsedTx } from "../src/sol/decode.js";
import type { CanisterReader, CanisterCollection, BooksFloor } from "../src/sol/canister.js";
import { decodeBooksView } from "../src/sol/canister.js";
import { DISC, POOL_CONFIG_LEN } from "../src/sol/dbc.js";
import type { SolRpc } from "../src/sol/rpc.js";

const canisterFixtures = new URL("../../autobuyer/canister-rs/tests/fixtures/", import.meta.url);
export const fx = (name: string) => JSON.parse(readFileSync(new URL(`./fixtures/sol/${name}.json`, import.meta.url), "utf8")) as ParsedTx;

/** autobuyer/canister-rs/tests/fixtures/solix2/dlmm.json: the desk's pair on preset 262 for wX `GyGK…` against wSOL, landed on LiteSVM from DLMM's mainnet bytes. */
export const DLMM = JSON.parse(readFileSync(new URL("solix2/dlmm.json", canisterFixtures), "utf8")) as {
  keys: { wx_mint: string; pair: string; preset: string; k_main: string; k_desk: string; k_coll: string; module: string; floor: number; active0: number; bin_step: number };
  accounts: { name: string; key: string; owner: string; data: string; expect: Record<string, unknown> }[];
};
export const dlmmAccount = (name: string) => { const a = DLMM.accounts.find((x) => x.name === name)!; return { owner: a.owner, data: [a.data, "base64"] as [string, string], expect: a.expect }; };
/** tests/fixtures/views/books-view.hex: a `books_view` reply (coll 0, wX `DyRk…`, accepted 200 SOL per wX), written by tests/desk_intents.rs. */
export const BOOKS_VIEW: BooksFloor = decodeBooksView(Uint8Array.from(Buffer.from(readFileSync(new URL("views/books-view.hex", canisterFixtures), "utf8").trim(), "hex")))!;

export const WX = DLMM.keys.wx_mint, WX2 = BOOKS_VIEW.wxMint, K_MAIN = DLMM.keys.k_main, MODULE = DLMM.keys.module;
export const CONFIG = "9q4yEJsVqUQ4SQp24o8moARNrBFFCm1vZDKLo3KPkqTe", CONFIG2 = "5qvf3QJ5LDo4z2TaYCHmcptSpHtxu4hJ348Np6oCDPUJ";
export const MINT = "8gTju7eND1D8pm52664toFE6gHZKMSGA8jnM9PvQoJkp";

/** A DBC PoolConfig account quoted in `quoteMint` (only the fields decodeConfig reads: discriminator, quote mint, fee mode, decimals, threshold). */
export function configAccount(quoteMint: string, o: { decimals?: number; threshold?: bigint; feeMode?: number } = {}): { owner: string; data: [string, string] } {
  const b = Buffer.alloc(POOL_CONFIG_LEN);
  Buffer.from(DISC.poolConfig).copy(b, 0);
  b58(quoteMint).copy(b, 8);
  b[232] = o.feeMode ?? 0; b[234] = 1; b[235] = o.decimals ?? 6;
  b.writeBigUInt64LE(o.threshold ?? 85_000_000_000n, 264);
  return { owner: "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN", data: [b.toString("base64"), "base64"] };
}
/** An SPL mint account with `decimals`. */
export function mintAccount(decimals = 9, supply = 0n): { owner: string; data: [string, string] } {
  const b = Buffer.alloc(82); b.writeBigUInt64LE(supply, 36); b[44] = decimals;
  return { owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", data: [b.toString("base64"), "base64"] };
}
function b58(s: string): Buffer {
  const A = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
  let n = 0n; for (const c of s) n = n * 58n + BigInt(A.indexOf(c));
  const out: number[] = []; while (n > 0n) { out.unshift(Number(n & 255n)); n >>= 8n; }
  let z = 0; while (s[z] === "1") z++;
  return Buffer.from([...new Array(z).fill(0), ...out]);
}

export type Account = { owner: string; data: [string, string] } | null;
/**
 * A read-only RPC answering from tables: accounts by address (getMultipleAccounts / getAccountInfo), signature pages per address
 * (newest first, as the RPC answers; `memo` as the RPC fills it) and transactions by signature. Counts calls per method.
 */
export function fakeRpc(w: { accounts?: Record<string, Account>; sigs?: Record<string, { tx: ParsedTx; memo?: string | null }[]>; }): SolRpc & { calls: Record<string, number> } {
  const calls: Record<string, number> = {};
  const sigOf = (t: ParsedTx) => t.transaction.signatures[0];
  return {
    calls,
    state: () => [],
    async call<T>(method: string, params: unknown[]): Promise<T> {
      calls[method] = (calls[method] ?? 0) + 1;
      if (method === "getMultipleAccounts") return { value: (params[0] as string[]).map((a) => w.accounts?.[a] ?? null) } as T;
      if (method === "getAccountInfo") return { value: w.accounts?.[params[0] as string] ?? null } as T;
      if (method === "getSignaturesForAddress") {
        const [addr, o] = params as [string, { limit: number; before?: string; until?: string }];
        let xs = w.sigs?.[addr] ?? [];
        if (o.until) xs = xs.slice(0, Math.max(0, xs.findIndex((x) => sigOf(x.tx) === o.until)));
        if (o.before) xs = xs.slice(xs.findIndex((x) => sigOf(x.tx) === o.before) + 1);
        return xs.slice(0, o.limit).map((x, i) => ({ signature: sigOf(x.tx), slot: x.tx.slot, err: null, blockTime: x.tx.blockTime, transactionIndex: i, memo: x.memo === undefined ? null : x.memo })) as T;
      }
      if (method === "getTransaction") {
        for (const xs of Object.values(w.sigs ?? {})) for (const x of xs) if (sigOf(x.tx) === (params as string[])[0]) return x.tx as T;
        return null as T;
      }
      throw new Error(`fake RPC: no ${method}`);
    },
  };
}
/** A canister whose `coins_collections` names the given collections and whose `books_view` answers from a table. */
export function fakeCanister(collections: CanisterCollection[], books: Record<number, BooksFloor | null> = {}): CanisterReader & { asked: number[] } {
  const asked: number[] = [];
  return { id: "aaaaa-aa", asked, collections: async () => collections, books: async (coll) => { asked.push(coll); return books[coll] ?? null; } };
}
/** The RPC memo field as the RPC writes it: `[<len>] <text>`. */
export const rpcMemo = (text: string) => `[${text.length}] ${text}`;
