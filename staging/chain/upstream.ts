import { createServer, type Server } from "node:http";

/**
 * docs/phase6.md, found by the second smoke run: the public Robinhood RPC keeps about a quarter of an hour of historical state.
 * A fork pinned at a block older than that cannot read ANY account or slot it has not already cached ("historical state … is not
 * available"), so a hosted fork stopped paying its faucet ten minutes after boot.
 *
 * This sits between anvil and the real RPC, on 127.0.0.1. It forwards READS only (an allowlist: it can never send). When the
 * real RPC refuses a state read because the block is too old, the same read is asked again at "latest". So: state the fork has
 * never touched is read as it is on Robinhood Chain NOW, not as it was at the fork block. For a wallet that has never been used
 * that is the same thing; for a contract it can, in principle, mix two moments. That is a stated property of staging, counted in
 * /healthz (`upstreamFallbacks`), and avoided altogether by pointing STAGING_UPSTREAM at an archive node.
 */
export const UPSTREAM_READS = /^(eth_(chainId|blockNumber|gasPrice|maxPriorityFeePerGas|feeHistory|call|estimateGas|getBalance|getTransactionCount|getCode|getStorageAt|getProof|getLogs|getBlockBy(Number|Hash)|getBlockReceipts|getTransactionBy(Hash|BlockHashAndIndex|BlockNumberAndIndex)|getTransactionReceipt|getBlockTransactionCountBy(Number|Hash))|net_version|web3_clientVersion)$/;
const STATE_AT_LAST_PARAM = new Set(["eth_getBalance", "eth_getTransactionCount", "eth_getCode", "eth_getStorageAt", "eth_getProof"]);
export const TOO_OLD = /historical state|missing trie node|state (is )?not available|pruned|header not found|unknown block/i;

type Call = { jsonrpc?: string; id?: unknown; method?: string; params?: unknown[] };
type Reply = { jsonrpc: "2.0"; id: unknown; result?: unknown; error?: { code: number; message: string } };

export function upstreamShim(o: { upstream: string; port: number; fetchImpl?: typeof fetch; log?: (r: Record<string, unknown>) => void }): { server: Server; stats: { fallbacks: number; refused: number } } {
  const f = o.fetchImpl ?? fetch, stats = { fallbacks: 0, refused: 0 };
  const ask = async (c: Call): Promise<Reply> => {
    const r = await f(o.upstream, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: c.id ?? 1, method: c.method, params: c.params ?? [] }) });
    return (await r.json()) as Reply;
  };
  const one = async (c: Call): Promise<Reply> => {
    const id = c.id ?? null;
    if (typeof c.method !== "string" || !UPSTREAM_READS.test(c.method)) { stats.refused++; return { jsonrpc: "2.0", id, error: { code: -32601, message: `refused: ${String(c.method)} is not a read` } }; }
    let reply = await ask(c);
    if (reply.error && TOO_OLD.test(reply.error.message ?? "") && STATE_AT_LAST_PARAM.has(c.method) && Array.isArray(c.params) && c.params.length >= 2) {
      reply = await ask({ ...c, params: [...c.params.slice(0, -1), "latest"] });
      if (!reply.error) { if (stats.fallbacks++ === 0) o.log?.({ event: "upstream-too-old", note: "the real RPC no longer serves the fork block's state; untouched state is now read at latest" }); }
    }
    return { ...reply, id };
  };
  const server = createServer((req, res) => {
    let body = ""; req.on("data", (d) => { body += d; if (body.length > 2_000_000) req.destroy(); });
    req.on("end", () => { void (async () => {
      try {
        const j = JSON.parse(body) as Call | Call[];
        const out = Array.isArray(j) ? await Promise.all(j.map(one)) : await one(j);
        res.writeHead(200, { "content-type": "application/json" }); res.end(JSON.stringify(out));
      } catch (err) { res.writeHead(502, { "content-type": "application/json" }); res.end(JSON.stringify({ jsonrpc: "2.0", id: null, error: { code: -32000, message: String((err as Error).message) } })); }
    })(); });
  });
  server.listen(o.port, "127.0.0.1");
  return { server, stats };
}
