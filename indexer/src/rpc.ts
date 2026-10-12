import { decodeFunctionResult, encodeFunctionData, type Abi, type Hex } from "viem";

/** docs/phase4.md §1. One JSON-RPC function, and it refuses anything that is not a read. There is no signer in this package. */
export const READ_ONLY = ["eth_call", "eth_getLogs", "eth_blockNumber", "eth_chainId", "eth_getBlockByNumber", "eth_getBlockByHash", "eth_getBalance",
  // docs/phase10.md: a coin's logo is in no event, only in the calldata of the launch, so the transaction itself has to be read.
  "eth_getTransactionByHash"] as const;
export type Fetch = typeof fetch;
export type Rpc = <T>(method: string, params: unknown[]) => Promise<T>;
export type RawLog = { address: Hex; topics: Hex[]; data: Hex; blockNumber: Hex; blockHash: Hex; transactionHash: Hex; logIndex: Hex; removed?: boolean };
export type RawBlock = { number: Hex; hash: Hex; parentHash: Hex; timestamp: Hex };

export class RpcError extends Error {
  constructor(readonly kind: "network" | "http" | "rpc" | "refused", message: string) { super(message); this.name = "RpcError"; }
}

export function rpcOf(url: string, fetchImpl: Fetch = fetch, timeoutMs = 20_000): Rpc {
  let id = 0;
  return async <T>(method: string, params: unknown[]): Promise<T> => {
    if (!(READ_ONLY as readonly string[]).includes(method)) throw new RpcError("refused", `refused: ${method} is not a read, and this package only reads`);
    let r: Response;
    try {
      r = await fetchImpl(url, { method: "POST", headers: { "content-type": "application/json", "user-agent": "etch-indexer/1" },
        body: JSON.stringify({ jsonrpc: "2.0", id: ++id, method, params }), signal: AbortSignal.timeout(timeoutMs) });
    } catch (err) { throw new RpcError("network", `${method}: ${(err as Error)?.name ?? "network error"}`); }
    if (!r.ok) throw new RpcError("http", `${method}: HTTP ${r.status}`);
    const j = (await r.json().catch(() => null)) as { result?: T; error?: { message?: string } } | null;
    if (!j) throw new RpcError("http", `${method}: not JSON`);
    if (j.error) throw new RpcError("rpc", `${method}: ${String(j.error.message).slice(0, 200)}`);
    return j.result as T;
  };
}

export const hex = (n: number | bigint): Hex => `0x${BigInt(n).toString(16)}`;

export async function read<T>(rpc: Rpc, to: string, abi: Abi, functionName: string, args: unknown[] = [], block: string = "latest"): Promise<T> {
  const data = encodeFunctionData({ abi, functionName, args });
  const out = await rpc<Hex>("eth_call", [{ to, data }, block]);
  return decodeFunctionResult({ abi, functionName, data: out }) as T;
}

/** A value, or null with no throw: a failed live read must never break an indexed answer. */
export async function tryRead<T>(rpc: Rpc, to: string, abi: Abi, functionName: string, args: unknown[] = []): Promise<T | null> {
  try { return await read<T>(rpc, to, abi, functionName, args); } catch { return null; }
}

export const BACKOFF_MS = [1_000, 2_000, 5_000, 15_000, 30_000, 60_000] as const;
