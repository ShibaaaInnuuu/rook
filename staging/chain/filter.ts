/**
 * docs/phase6.md. What a stranger may ask our staging node. The node behind the proxy will send a transaction FROM ANY ADDRESS it
 * is asked to and will rewrite its own storage on request: that is what makes a keyless staging chain possible, and it is exactly
 * what must never be reachable from outside. So this is an ALLOWLIST. A method that is not named here is refused, including every
 * one invented after this was written.
 */
export const PUBLIC_METHODS: ReadonlySet<string> = new Set([
  "eth_chainId", "net_version", "web3_clientVersion", "eth_blockNumber", "eth_getBlockByNumber", "eth_getBlockByHash", "eth_getBalance",
  "eth_getCode", "eth_getStorageAt", "eth_getTransactionCount", "eth_getTransactionByHash", "eth_getTransactionReceipt", "eth_getLogs",
  "eth_call", "eth_estimateGas", "eth_gasPrice", "eth_maxPriorityFeePerGas", "eth_feeHistory", "eth_syncing",
  "eth_sendRawTransaction",          // signed by the sender's own wallet for the staging chain: the only way a stranger changes anything
]);
/** Named so the refusal can say why; the allowlist above is what decides. */
const WHY: readonly [RegExp, string][] = [
  [/^(anvil|hardhat|evm|debug|personal|admin|miner|txpool|engine|ots|erigon|trace)_/, "an administrative method of the node"],
  [/^eth_(sendTransaction|sendUnsignedTransaction|sign|signTransaction|signTypedData.*|accounts|requestAccounts|coinbase)$/, "the node would act for an address nobody has signed for"],
  [/^(eth_newFilter|eth_newBlockFilter|eth_newPendingTransactionFilter|eth_getFilterChanges|eth_getFilterLogs|eth_uninstallFilter|eth_subscribe|eth_unsubscribe)$/, "stateful filters are not offered; use eth_getLogs"],
];
export type Verdict = { ok: true } | { ok: false; why: string };
export function verdict(method: unknown): Verdict {
  if (typeof method !== "string") return { ok: false, why: "no method" };
  if (PUBLIC_METHODS.has(method)) return { ok: true };
  return { ok: false, why: WHY.find(([re]) => re.test(method))?.[1] ?? "not on the list of methods this proxy forwards" };
}

export type Call = { jsonrpc?: string; id?: unknown; method?: unknown; params?: unknown };
/** A body is one call or a batch; a batch is judged call by call, and refused whole if it is too long. */
export function judge(body: unknown, maxBatch = 50): { forward: Call[]; refused: { id: unknown; why: string }[]; batch: boolean } | { error: string } {
  const batch = Array.isArray(body);
  const calls = (batch ? body : [body]) as Call[];
  if (!calls.length || calls.length > maxBatch) return { error: `a batch is 1 to ${maxBatch} calls` };
  const forward: Call[] = [], refused: { id: unknown; why: string }[] = [];
  for (const c of calls) {
    if (!c || typeof c !== "object") return { error: "not a JSON-RPC call" };
    const v = verdict(c.method);
    if (v.ok) forward.push(c); else refused.push({ id: c.id ?? null, why: `${String(c.method)}: ${v.why}` });
  }
  return { forward, refused, batch };
}

/** getLogs over an unbounded range would be served from the public RPC behind the fork, block by block. Bound it. */
export function boundedLogs(c: Call, maxRange = 500_000n): string | null {
  if (c.method !== "eth_getLogs") return null;
  const f = (Array.isArray(c.params) ? c.params[0] : null) as { fromBlock?: string; toBlock?: string; blockHash?: string } | null;
  if (!f || f.blockHash) return null;
  const from = f.fromBlock, to = f.toBlock ?? "latest";
  if (!from || !/^0x[0-9a-f]+$/i.test(from)) return "eth_getLogs needs a numeric fromBlock here (the deployment's fromBlock is in /info)";
  if (/^0x[0-9a-f]+$/i.test(to) && BigInt(to) - BigInt(from) > maxRange) return `eth_getLogs over more than ${maxRange} blocks`;
  return null;
}

/** Per-key counters in fixed windows: a request limit per IP, and the faucet's daily caps. */
export class Windows {
  private readonly hits = new Map<string, { start: number; n: number }>();
  constructor(private readonly windowMs: number, private readonly max: number, private readonly now: () => number = Date.now) {}
  take(key: string, n = 1): boolean {
    const t = this.now(), h = this.hits.get(key);
    if (!h || t - h.start >= this.windowMs) { this.hits.set(key, { start: t, n }); return n <= this.max; }
    if (h.n + n > this.max) return false;
    h.n += n; return true;
  }
  /** Whether `take` would succeed, without taking. */
  can(key: string, n = 1): boolean { const t = this.now(), h = this.hits.get(key); return !h || t - h.start >= this.windowMs ? n <= this.max : h.n + n <= this.max; }
  sweep(): void { const t = this.now(); for (const [k, h] of this.hits) if (t - h.start >= this.windowMs) this.hits.delete(k); }
}
