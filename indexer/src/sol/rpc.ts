/**
 * Read-only Solana JSON-RPC with two URLs: the primary is used until it fails (network error, HTTP 429/5xx, timeout), then the
 * secondary; a failed endpoint is benched with exponential backoff and retried after. A method that is not a read is refused before
 * anything is sent: this package has no key and sends no transaction.
 */
export const SOL_READ_ONLY = ["getSignaturesForAddress", "getTransaction", "getProgramAccounts", "getMultipleAccounts", "getAccountInfo",
  "getTokenLargestAccounts", "getTokenSupply", "getTokenAccountsByOwner", "getSlot", "getBlockTime", "getHealth", "getVersion", "getTokenAccounts"] as const;
export type SolMethod = (typeof SOL_READ_ONLY)[number];
export type Fetch = typeof fetch;

export class SolRpcError extends Error {
  constructor(readonly kind: "network" | "http" | "rpc" | "refused" | "rate", message: string, readonly code?: number, readonly retryAfterMs?: number) { super(message); this.name = "SolRpcError"; }
}
export type EndpointState = { name: "primary" | "secondary"; ok: boolean; lastError: string | null; calls: number; benchedUntil: number; failures: number };
export type SolRpc = {
  call<T>(method: SolMethod, params: unknown[]): Promise<T>;
  state(): EndpointState[];
};

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
/** What an RPC error means for the next try: a rate limit or a server problem is worth another endpoint / a wait; a bad request is not. */
const retryable = (e: unknown) => e instanceof SolRpcError && (e.kind === "network" || e.kind === "rate" || (e.kind === "http" && ((e.code ?? 0) >= 500 || e.code === 403)) || (e.kind === "rpc" && (e.code === -32005 || /timeout|overloaded|too many|rate|behind|unavailable/i.test(e.message))));

export function solRpc(urls: string[], o: { fetch?: Fetch; timeoutMs?: number; maxTries?: number; minGapMs?: number; now?: () => number; sleep?: (ms: number) => Promise<void> } = {}): SolRpc {
  const f = o.fetch ?? fetch, now = o.now ?? Date.now, wait = o.sleep ?? sleep;
  const eps = urls.filter(Boolean).slice(0, 2).map((url, i) => ({ url, st: { name: (i === 0 ? "primary" : "secondary") as EndpointState["name"], ok: true, lastError: null as string | null, calls: 0, benchedUntil: 0, failures: 0 } }));
  if (!eps.length) throw new Error("no Solana RPC URL: set SOLANA_RPC_URL");
  let id = 0, nextAt = 0;
  const once = async <T>(ep: (typeof eps)[number], method: string, params: unknown[]): Promise<T> => {
    // a small gap between calls keeps a free endpoint's per-second limit happy (minGapMs 0 = none)
    if (o.minGapMs) { const t = now(), slot = Math.max(t, nextAt); nextAt = slot + o.minGapMs; if (slot > t) await wait(slot - t); }
    ep.st.calls++;
    if (process.env.DEBUG_RPC) console.error(`rpc ${ep.st.name} ${method} ${new Date().toISOString()}`);
    let r: Response;
    try {
      r = await f(ep.url, { method: "POST", headers: { "content-type": "application/json", "user-agent": "rook-indexer-sol/1" },
        body: JSON.stringify({ jsonrpc: "2.0", id: ++id, method, params }), signal: AbortSignal.timeout(o.timeoutMs ?? 30_000) });
    } catch (err) { throw new SolRpcError("network", `${method}: ${(err as Error)?.name ?? "network error"}`); }
    if (r.status === 429) { const ra = Number(r.headers.get("retry-after")); throw new SolRpcError("rate", `${method}: HTTP 429`, 429, Number.isFinite(ra) && ra > 0 ? Math.min(60, ra) * 1000 : undefined); }
    if (!r.ok) throw new SolRpcError("http", `${method}: HTTP ${r.status}`, r.status);
    const j = (await r.json().catch(() => null)) as { result?: T; error?: { code?: number; message?: string } } | null;
    if (!j) throw new SolRpcError("http", `${method}: not JSON`);
    if (j.error) {
      const msg = String(j.error.message ?? "").slice(0, 200);
      if (j.error.code === 429 || /rate limit|too many/i.test(msg)) throw new SolRpcError("rate", `${method}: ${msg}`, 429);
      throw new SolRpcError("rpc", `${method}: ${msg}`, j.error.code);
    }
    return j.result as T;
  };
  return {
    state: () => eps.map((e) => ({ ...e.st })),
    async call<T>(method: SolMethod, params: unknown[]): Promise<T> {
      if (!(SOL_READ_ONLY as readonly string[]).includes(method)) throw new SolRpcError("refused", `refused: ${method} is not a read, and this package only reads`);
      const tries = o.maxTries ?? 10;
      let last: unknown = null;
      for (let t = 0; t < tries; t++) {
        // the first endpoint not benched, else the one whose bench ends soonest (after waiting for it)
        const at = now();
        let ep = eps.find((e) => e.st.benchedUntil <= at);
        if (!ep) { ep = eps.reduce((a, b) => (a.st.benchedUntil <= b.st.benchedUntil ? a : b)); await wait(Math.min(30_000, ep.st.benchedUntil - at)); }
        try {
          const out = await once<T>(ep, method, params);
          ep.st.ok = true; ep.st.failures = 0; ep.st.lastError = null;
          return out;
        } catch (err) {
          last = err;
          if (!retryable(err)) throw err;
          ep.st.ok = false; ep.st.failures++; ep.st.lastError = String((err as Error).message).slice(0, 160);
          ep.st.benchedUntil = now() + Math.max((err as SolRpcError).retryAfterMs ?? 0, Math.min(60_000, 500 * 2 ** Math.min(7, ep.st.failures - 1)));
        }
      }
      throw last;
    },
  };
}
export const isRetryable = retryable;
