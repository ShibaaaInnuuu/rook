import { hostOf } from "./config.js";

/**
 * docs/v6/p1-services.md §1.2. The keeper's JSON-RPC: several endpoints, one chain id. An endpoint that reports another chain id is
 * DROPPED (and named by host only: a URL may carry an API key); the chain id is asked again of the endpoint a raw transaction goes
 * through, immediately before it goes. The method list is closed; `eth_sendRawTransaction` is the one that sends, and the node
 * never signs for us (`eth_sendTransaction`, `eth_sign*` are not on the list).
 */
export const METHODS = ["eth_chainId", "eth_blockNumber", "eth_getBlockByNumber", "eth_call", "eth_getLogs", "eth_estimateGas",
  "eth_getTransactionCount", "eth_getTransactionReceipt", "eth_maxPriorityFeePerGas", "eth_getBalance", "eth_getCode",
  "eth_sendRawTransaction"] as const;
export type Method = (typeof METHODS)[number];
export type Fetch = typeof fetch;
export type Log = (level: "debug" | "info" | "warn" | "error", event: string, fields?: Record<string, unknown>) => void;

/** A JSON-RPC error answer: the node answered, and said no (a revert, a nonce too low). Not a reason to try another endpoint. */
export class RpcAnswer extends Error {
  constructor(readonly method: string, message: string, readonly code: number | null, readonly data: unknown) { super(`${method}: ${message}`); this.name = "RpcAnswer"; }
}

/**
 * Audit E-04: both public 4663 RPCs answer a throttle as a JSON-RPC error inside HTTP 200 (`{"code":429,"message":"Too Many
 * Requests"}`, `{"code":-32005,"message":"the network is busy"}`). That is an outage of that endpoint, not the chain's answer: the
 * next endpoint is tried. A query refused as too large ("logs matched by query exceeds limit of 10000") is neither: it goes back to
 * the caller, which splits it (service/logs.ts).
 */
export const isTooLarge = (message: string): boolean =>
  /exceeds limit|too many (logs|results)|query returned more than|block range|range (is )?too (large|wide)|limit exceeded|response size/i.test(message);
export const isRateLimited = (e: RpcAnswer): boolean =>
  !isTooLarge(e.message) && (e.code === 429 || e.code === -32005 || e.code === -32016 || /too many requests|rate.?limit|network is busy|try again later|over capacity|capacity exceeded/i.test(e.message));
/** The keeper refuses: a fact about the world that no retry fixes by itself (wrong chain, wrong key). */
export class Refusal extends Error { constructor(message: string) { super(message); this.name = "Refusal"; } }

type Endpoint = { url: string; host: string; ok: boolean };

export class ChainRpc {
  private readonly eps: Endpoint[];
  private id = 0;
  private readonly pause: (ms: number) => Promise<void>;
  constructor(urls: readonly string[], readonly chainId: number, private readonly fetchImpl: Fetch = fetch, private readonly log: Log = () => {},
    private readonly timeoutMs = 20_000, o: { retryRounds?: number; backoffMs?: number; sleep?: (ms: number) => Promise<void> } = {}) {
    this.eps = urls.map((url) => ({ url, host: hostOf(url), ok: false }));
    this.retryRounds = o.retryRounds ?? 2;
    this.backoffMs = o.backoffMs ?? 1_000;
    this.pause = o.sleep ?? ((ms) => new Promise((ok) => setTimeout(ok, ms)));
  }
  private readonly retryRounds: number;
  private readonly backoffMs: number;

  /** Ask every endpoint its chain id; keep the ones that say ours. None: refused. */
  async init(): Promise<string[]> {
    for (const e of this.eps) {
      try {
        const id = Number(BigInt(await this.raw<string>(e, "eth_chainId", [])));
        e.ok = id === this.chainId;
        if (!e.ok) this.log("error", "rpc-wrong-chain", { host: e.host, reports: id, expected: this.chainId, note: "this endpoint is not used" });
      } catch (err) {
        e.ok = false;
        this.log("warn", "rpc-unreachable", { host: e.host, error: String((err as Error).message).slice(0, 160) });
      }
    }
    const live = this.eps.filter((e) => e.ok);
    if (live.length === 0) throw new Refusal(`no RPC reports chain id ${this.chainId} (${this.eps.map((e) => e.host).join(", ")})`);
    return live.map((e) => e.host);
  }

  hosts(): { host: string; ok: boolean }[] { return this.eps.map((e) => ({ host: e.host, ok: e.ok })); }

  private async raw<T>(e: Endpoint, method: Method, params: unknown[]): Promise<T> {
    if (!(METHODS as readonly string[]).includes(method)) throw new Refusal(`refused: ${method} is not on the keeper's method list`);
    const r = await this.fetchImpl(e.url, { method: "POST", headers: { "content-type": "application/json", "user-agent": "rook-keeper/1" },
      body: JSON.stringify({ jsonrpc: "2.0", id: ++this.id, method, params }), signal: AbortSignal.timeout(this.timeoutMs) });
    if (!r.ok) throw new Error(`${method}: HTTP ${r.status} from ${e.host}`);
    const j = (await r.json()) as { result?: T; error?: { message?: string; code?: number; data?: unknown } };
    if (j.error) throw new RpcAnswer(method, String(j.error.message ?? "error").slice(0, 300), j.error.code ?? null, j.error.data);
    return j.result as T;
  }

  /**
   * A read or a simulation: the first endpoint that answers. A JSON-RPC error IS an answer (a revert is not an outage), except a
   * throttle (`isRateLimited`), which moves on to the next endpoint like a network failure. When every endpoint failed, it waits
   * and tries them all again, `retryRounds` more times at most.
   */
  async call<T>(method: Method, params: unknown[]): Promise<T> {
    if (method === "eth_sendRawTransaction") throw new Refusal("refused: raw transactions go through sendRaw, which asks the chain id first");
    let last: unknown = null;
    for (let round = 0; round <= this.retryRounds; round++) {
      if (round > 0) await this.pause(this.backoffMs * round);
      for (const e of this.eps.filter((x) => x.ok)) {
        try { return await this.raw<T>(e, method, params); } catch (err) {
          if (err instanceof Refusal) throw err;
          if (err instanceof RpcAnswer && !isRateLimited(err)) throw err;
          last = err;
          this.log("warn", "rpc-fallback", { host: e.host, method, error: String((err as Error).message).slice(0, 160), throttled: err instanceof RpcAnswer });
        }
      }
    }
    throw new Error(`every RPC failed for ${method}: ${String((last as Error)?.message ?? "no endpoint").slice(0, 200)}`);
  }

  /**
   * Broadcast signed bytes. Before the bytes go to an endpoint, that endpoint is asked its chain id; a changed answer drops it and
   * the send is refused there. The same bytes may go to the next endpoint on a network failure: they are the same transaction.
   */
  async sendRaw(raw: string): Promise<{ host: string; hash: string | null; known: boolean }> {
    let last: unknown = null;
    for (const e of this.eps.filter((x) => x.ok)) {
      let id: number;
      try { id = Number(BigInt(await this.raw<string>(e, "eth_chainId", []))); } catch (err) { last = err; continue; }
      if (id !== this.chainId) {
        e.ok = false;
        this.log("error", "rpc-wrong-chain", { host: e.host, reports: id, expected: this.chainId, note: "dropped before a send" });
        last = new Refusal(`refused: ${e.host} now reports chain id ${id}, not ${this.chainId}`);
        continue;
      }
      try { return { host: e.host, hash: await this.raw<string>(e, "eth_sendRawTransaction", [raw]), known: false }; } catch (err) {
        if (err instanceof RpcAnswer) {
          if (/already known|known transaction|already imported/i.test(err.message)) return { host: e.host, hash: null, known: true };
          if (!isRateLimited(err)) throw err;
        }
        last = err;                     // a network failure or a throttle: the same bytes go to the next endpoint
      }
    }
    if (last instanceof Refusal) throw last;
    throw new Error(`the transaction could not be broadcast: ${String((last as Error)?.message ?? "no endpoint").slice(0, 200)}`);
  }
}
