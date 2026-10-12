import type { TransferLog } from "./core.js";

/** Raw JSON-RPC over fetch. Reads only: eth_getLogs, eth_call, eth_blockNumber, eth_getBlockByNumber. There is no signer in this package. */
const TRANSFER = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

export class RpcError extends Error { constructor(readonly method: string, message: string, readonly code: number | null) { super(`${method}: ${message}`); this.name = "RpcError"; } }

/** A JSON-RPC answer that is really an outage or a throttle (audit E-04: both public 4663 RPCs answer these in HTTP 200). */
export const isRateLimited = (code: number | null, message: string): boolean =>
  code === 429 || code === -32005 || code === -32016 || /too many requests|rate.?limit|network is busy|try again|capacity/i.test(message);
/** A query the node refuses as too large: split it (audit E-11: "logs matched by query exceeds limit of 10000"). */
export const isTooLarge = (message: string): boolean => /exceeds limit|too many (logs|results)|query returned more than|block range|range (is )?too (large|wide)|limit exceeded|response size/i.test(message);

export type Fetch = typeof fetch;
const sleep = (ms: number) => new Promise<void>((ok) => setTimeout(ok, ms));

export async function rpc<T>(url: string, method: string, params: unknown[], fetchImpl: Fetch = fetch, pause: (ms: number) => Promise<void> = sleep): Promise<T> {
  for (let i = 0; ; i++) {
    const r = await fetchImpl(url, { method: "POST", headers: { "content-type": "application/json", "user-agent": "etch-snapshot/1" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
    if (r.status === 429 && i < 6) { await pause(1500 * (i + 1)); continue; }
    if (!r.ok) throw new RpcError(method, `HTTP ${r.status}`, r.status);
    const j = (await r.json()) as { result?: T; error?: { message?: string; code?: number } };
    if (j.error) {
      const msg = String(j.error.message ?? "error"), code = j.error.code ?? null;
      if (isRateLimited(code, msg) && !isTooLarge(msg) && i < 6) { await pause(1500 * (i + 1)); continue; }
      throw new RpcError(method, msg, code);
    }
    return j.result as T;
  }
}

export type RawLog = { blockNumber: string; logIndex: string; topics: string[]; data: string; removed?: boolean };
export const decodeTransfer = (l: RawLog): TransferLog | null => l.removed || l.topics.length < 3 ? null
  : { blockNumber: BigInt(l.blockNumber), logIndex: Number(BigInt(l.logIndex)), from: `0x${l.topics[1].slice(26)}`, to: `0x${l.topics[2].slice(26)}`,
    value: BigInt(l.data === "0x" ? "0x0" : l.data) };

/**
 * Transfer logs in [fromBlock, toBlock] through `get`. A range the node refuses as too large is halved, down to one block; a
 * failure that is not "too large" throws: a snapshot is never built over logs that were not all read.
 */
export async function transferLogsWith(get: (from: bigint, to: bigint) => Promise<RawLog[]>, fromBlock: bigint, toBlock: bigint, step = 400_000n): Promise<TransferLog[]> {
  const out: TransferLog[] = [];
  for (let b = fromBlock; b <= toBlock; ) {
    let span = step;
    for (;;) {
      const e = b + span - 1n > toBlock ? toBlock : b + span - 1n;
      let logs: RawLog[];
      try { logs = await get(b, e); } catch (err) {
        if (isTooLarge(String((err as Error).message)) && e > b) { span = (e - b + 1n) / 2n; continue; }
        throw err;
      }
      for (const l of logs) { const t = decodeTransfer(l); if (t) out.push(t); }
      b = e + 1n;
      break;
    }
  }
  return out;
}

export const transferLogs = (url: string, coin: string, fromBlock: bigint, toBlock: bigint, step = 400_000n, fetchImpl: Fetch = fetch): Promise<TransferLog[]> =>
  transferLogsWith((a, e) => rpc<RawLog[]>(url, "eth_getLogs", [{ address: coin, topics: [TRANSFER], fromBlock: `0x${a.toString(16)}`, toBlock: `0x${e.toString(16)}` }], fetchImpl),
    fromBlock, toBlock, step);

/**
 * docs/snapshot-v1.md "Version 2": the snapshot block of time `t` is the LAST block whose timestamp is ≤ t (the chain's own numbering). Many
 * blocks share a second on 4663, so it is the block before the first one stamped after `t`. Throws when no block after `t`
 * exists yet (the answer could still change).
 */
export async function lastBlockAtOrBefore(ts: (n: bigint) => Promise<bigint>, head: bigint, t: bigint, lo = 0n): Promise<bigint> {
  if ((await ts(head)) <= t) throw new Error(`no block is stamped after ${t} yet (head ${head}): the snapshot block is not fixed`);
  if ((await ts(lo)) > t) throw new Error(`block ${lo} is already stamped after ${t}`);
  let a = lo, b = head;              // invariant: ts(a) ≤ t < ts(b)
  while (b - a > 1n) {
    const m = (a + b) / 2n;
    if ((await ts(m)) <= t) a = m; else b = m;
  }
  return a;
}

export const blockTime = (url: string, fetchImpl: Fetch = fetch) => async (n: bigint): Promise<bigint> => {
  const b = await rpc<{ timestamp: string } | null>(url, "eth_getBlockByNumber", [`0x${n.toString(16)}`, false], fetchImpl);
  if (!b) throw new Error(`block ${n} is not served`);
  return BigInt(b.timestamp);
};

/** At `latest`, on purpose: a public RPC keeps no historical state, and the only things read this way — a coin's curve, its decimals,
 *  the factory's hook and escrow, a draw's fixed fields — do not change. Balances never come from eth_call; they are rebuilt from logs. */
export const call = (url: string, to: string, data: string) => rpc<string>(url, "eth_call", [{ to, data }, "latest"]);
