import { decodeFunctionResult, encodeFunctionData, type Abi } from "viem";
import type { CallLog, Hex } from "./types.js";

/**
 * A keyless JSON-RPC reader for the browser and the CLI: reads only (a fixed list of methods), a deadline per call, a bounded
 * answer, and every call logged so the page can show what it asked and what it was told. A URL with a key in it is refused here,
 * because this code ships to visitors (proof-of-reserves.md, P2: keyless providers only in client code).
 */
/** `fetch`, or anything shaped like it (a test's fake, the CLI's guarded fetch). */
export type Fetch = (url: string, init?: { method?: string; headers?: Record<string, string>; body?: string; signal?: AbortSignal }) => Promise<{ ok: boolean; status: number; text(): Promise<string>; json(): Promise<unknown> }>;
const READS = new Set(["eth_chainId", "eth_blockNumber", "eth_getBlockByNumber", "eth_getBlockByHash", "eth_call", "eth_getBalance", "eth_getLogs", "eth_getTransactionReceipt", "eth_getTransactionByHash"]);
export const MAX_ANSWER = 4_000_000;

export const hostOf = (u: string): string => { try { return new URL(u).host; } catch { return u; } };
/** A URL that looks like it carries a credential: a path segment of 20+ key-like characters, a query, or user info. */
export function looksKeyed(u: string): boolean {
  try {
    const x = new URL(u);
    if (x.username || x.password || x.search) return true;
    return x.pathname.split("/").some((seg) => /^[A-Za-z0-9_-]{20,}$/.test(seg) && !/^[a-z]+$/.test(seg));
  } catch { return true; }
}

export class RpcError extends Error { constructor(message: string, readonly reverted = false) { super(message); this.name = "RpcError"; } }

export type Reader = {
  url: string; host: string; chain: CallLog["chain"];
  raw<T>(method: string, params: unknown[]): Promise<T>;
  call<T>(to: string, abi: Abi, fn: string, args: unknown[], block: number | "latest" | "finalized"): Promise<T>;
};
export function reader(url: string, chain: CallLog["chain"], fetchImpl: Fetch, log: CallLog[] | null, timeoutMs = 12_000): Reader {
  if (!/^https?:\/\//.test(url)) throw new Error(`${url} is not an http(s) URL`);
  if (looksKeyed(url)) throw new Error(`${hostOf(url)}: this URL looks like it carries a key; the verifier uses keyless providers only`);
  const host = hostOf(url);
  let id = 0;
  const raw = async <T,>(method: string, params: unknown[]): Promise<T> => {
    if (!READS.has(method)) throw new Error(`${method} is not a read`);
    const t0 = Date.now(), p = summarize(params);
    const done = (ok: boolean, result: string) => log?.push({ chain, host, method, params: p, result, ok, ms: Date.now() - t0 });
    let text: string;
    try {
      const r = await fetchImpl(url, { method: "POST", headers: { "content-type": "application/json", accept: "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: ++id, method, params }), signal: AbortSignal.timeout(timeoutMs) });
      if (!r.ok) { done(false, `HTTP ${r.status}`); throw new RpcError(`HTTP ${r.status}`); }
      text = await r.text();
    } catch (e) { if (e instanceof RpcError) throw e; const m = String((e as Error)?.message ?? e).slice(0, 100); done(false, m); throw new RpcError(m); }
    if (text.length > MAX_ANSWER) { done(false, "answer too large"); throw new RpcError("answer too large"); }
    let j: { result?: T; error?: { message?: string; code?: number } };
    try { j = JSON.parse(text); } catch { done(false, "not JSON"); throw new RpcError("not JSON"); }
    if (j.error) { const m = String(j.error.message ?? "error").slice(0, 120), rev = /revert/i.test(m) || j.error.code === 3; done(false, m); throw new RpcError(m, rev); }
    done(true, summarize(j.result));
    return j.result as T;
  };
  const call = async <T,>(to: string, abi: Abi, fn: string, args: unknown[], block: number | "latest" | "finalized"): Promise<T> => {
    const data = encodeFunctionData({ abi, functionName: fn, args } as never);
    const tag = typeof block === "number" ? `0x${block.toString(16)}` : block;
    const out = await raw<Hex>("eth_call", [{ to, data }, tag]);
    if (out === "0x") throw new RpcError(`${fn} returned nothing (no contract at ${to}?)`, true);
    return decodeFunctionResult({ abi, functionName: fn, data: out } as never) as T;
  };
  return { url, host, chain, raw, call };
}

function summarize(x: unknown): string {
  const s = JSON.stringify(x, (_k, v) => (typeof v === "bigint" ? v.toString() : v)) ?? "";
  return s.length > 160 ? `${s.slice(0, 157)}…` : s;
}
