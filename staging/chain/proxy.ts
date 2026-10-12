import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { existsSync, readFileSync } from "node:fs";
import { boundedLogs, judge, Windows, type Call } from "./filter.js";

/**
 * docs/phase6.md. The only door into the staging node. JSON-RPC by an allowlist (chain/filter.ts), a capped faucet, the published
 * snapshots, /info and /healthz. The node listens on 127.0.0.1 only; whatever is not forwarded from here cannot be reached.
 */
export type ProxyOptions = { node: string; data: string; port: number; chainId: number; forkBlock: number; faucetWei: bigint; publicUrl: string;
  waitS: number; faucetPerIp?: number; upstream?: { fallbacks: number; refused: number }; fetchImpl?: typeof fetch; now?: () => number; log?: (o: Record<string, unknown>) => void };

const DAY = 86_400_000;
const ADDRESS = /^0x[0-9a-fA-F]{40}$/;

export function proxy(o: ProxyOptions) {
  const f = o.fetchImpl ?? fetch, now = o.now ?? Date.now, log = o.log ?? (() => {});
  const perIp = new Windows(10_000, 300, now);                 // 300 calls per 10 s per address: a page load is a few dozen
  const faucetByAddress = new Windows(DAY, 1, now), faucetByIp = new Windows(DAY, o.faucetPerIp ?? 5, now), faucetAll = new Windows(DAY, 500, now);
  setInterval(() => { perIp.sweep(); faucetByAddress.sweep(); faucetByIp.sweep(); }, 60_000).unref();

  const node = async <T>(method: string, params: unknown[] = []): Promise<T> => {
    const r = await f(o.node, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
    const j = (await r.json()) as { result?: T; error?: { message: string } };
    if (j.error) throw new Error(j.error.message);
    return j.result as T;
  };
  const ipOf = (req: IncomingMessage) => String(req.headers["x-forwarded-for"] ?? "").split(",")[0].trim() || req.socket.remoteAddress || "?";
  const send = (res: ServerResponse, status: number, body: unknown, extra: Record<string, string> = {}) => {
    res.writeHead(status, { "content-type": "application/json", "access-control-allow-origin": "*", "access-control-allow-headers": "content-type",
      "access-control-allow-methods": "GET, POST, OPTIONS", "cache-control": "no-store", ...extra });
    res.end(typeof body === "string" ? body : JSON.stringify(body));
  };
  const bodyOf = (req: IncomingMessage, max = 256_000) => new Promise<string>((ok, no) => {
    let n = 0; const parts: Buffer[] = [];
    req.on("data", (c: Buffer) => { n += c.length; if (n > max) { no(new Error("too large")); req.destroy(); } else parts.push(c); });
    req.on("end", () => ok(Buffer.concat(parts).toString("utf8"))); req.on("error", no);
  });
  const deployment = () => { try { return JSON.parse(readFileSync(`${o.data}/deployment.json`, "utf8")) as Record<string, unknown>; } catch { return null; } };
  /**
   * docs/phase11.md §6. How much of the catalogue's inventory exists yet, read from the manifest the creator rewrites after
   * every batch. Null before it has written anything at all. `done: false` is the honest signal for anything that wants to
   * wait; the container is ready either way, because the node and this proxy are what readiness means here.
   */
  const listingProgress = () => {
    try {
      const m = JSON.parse(readFileSync(`${o.data}/listings.json`, "utf8")) as { progress?: { created: number; total: number; done: boolean } };
      if (!m.progress) return null;
      const { created, total, done } = m.progress;
      return { created, total, done, of: `${created} of ${total} vaults created`, roots: "placeholder: docs/phase11.md" };
    } catch { return null; }
  };

  async function rpc(req: IncomingMessage, res: ServerResponse): Promise<void> {
    let parsed: unknown;
    try { parsed = JSON.parse(await bodyOf(req)); } catch { return send(res, 400, { jsonrpc: "2.0", id: null, error: { code: -32700, message: "not JSON, or too large" } }); }
    const j = judge(parsed);
    if ("error" in j) return send(res, 400, { jsonrpc: "2.0", id: null, error: { code: -32600, message: j.error } });
    if (!perIp.take(ipOf(req), j.forward.length + j.refused.length)) return send(res, 429, { jsonrpc: "2.0", id: null, error: { code: -32005, message: "slow down: 300 calls per 10 s" } });
    const out: unknown[] = j.refused.map((r) => ({ jsonrpc: "2.0", id: r.id, error: { code: -32601, message: `refused by the staging proxy — ${r.why}` } }));
    const go: Call[] = [];
    for (const c of j.forward) { const why = boundedLogs(c); if (why) out.push({ jsonrpc: "2.0", id: c.id ?? null, error: { code: -32602, message: why } }); else go.push(c); }
    if (j.refused.length) log({ event: "refused", ip: ipOf(req), methods: j.refused.map((r) => r.why.split(":")[0]) });
    if (go.length) {
      const r = await f(o.node, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(j.batch ? go : go[0]) });
      const answer = (await r.json()) as unknown;
      if (Array.isArray(answer)) out.push(...answer); else out.push(answer);
    }
    send(res, 200, j.batch ? out : out[0]);
  }

  async function faucet(req: IncomingMessage, res: ServerResponse): Promise<void> {
    let address = "";
    try { address = String((JSON.parse(await bodyOf(req, 2_000)) as { address?: string }).address ?? ""); } catch { /* falls through */ }
    if (!ADDRESS.test(address)) return send(res, 400, { error: "send {\"address\": \"0x…\"}" });
    const key = address.toLowerCase(), ip = ipOf(req);
    const d = deployment();
    if (d && Object.values(d).some((v) => typeof v === "string" && v.toLowerCase() === key)) return send(res, 400, { error: "that is one of the deployment's own addresses" });
    // all three are asked before any is counted: a refusal must not use up somebody's turn
    if (!faucetByAddress.can(key)) return send(res, 429, { error: "once a day for an address" });
    if (!faucetByIp.can(ip)) return send(res, 429, { error: `${o.faucetPerIp ?? 5} addresses a day from one place` });
    if (!faucetAll.can("all")) return send(res, 429, { error: "the faucet has given all it gives in a day" });
    faucetByAddress.take(key); faucetByIp.take(ip); faucetAll.take("all");
    const have = BigInt(await node<string>("eth_getBalance", [address, "latest"]));
    await node("anvil_setBalance", [address, `0x${(have + o.faucetWei).toString(16)}`]);
    await node("evm_mine");
    log({ event: "faucet", address: key, ip });
    send(res, 200, { address, addedWei: String(o.faucetWei), note: `sETH on chain ${o.chainId}: worth nothing anywhere` });
  }

  const server = createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://x");
    const done = (p: Promise<void>) => p.catch((err) => { log({ event: "error", path: url.pathname, error: String((err as Error).message).slice(0, 200) }); if (!res.headersSent) send(res, 502, { error: "the staging node did not answer" }); });
    if (req.method === "OPTIONS") return send(res, 204, "");
    if (req.method === "POST" && url.pathname === "/") return void done(rpc(req, res));
    if (req.method === "POST" && url.pathname === "/faucet") return void done(faucet(req, res));
    if (req.method === "GET" && url.pathname === "/healthz") return void done((async () => {
      const id = Number(BigInt(await node<string>("eth_chainId"))), head = Number(BigInt(await node<string>("eth_blockNumber")));
      // docs/phase11.md §6: the inventory fills beside this endpoint, after the node and the proxy are up. The container IS
      // ready while it fills — failing the healthcheck would restart it mid-inventory — so the count is reported instead.
      send(res, id === o.chainId ? 200 : 500, { ok: id === o.chainId, chainId: id, head, deployed: deployment() !== null,
        listings: listingProgress(), upstreamFallbacks: o.upstream?.fallbacks ?? null });
    })());
    if (req.method === "GET" && url.pathname === "/info") return send(res, 200, { name: "etched staging", chainId: o.chainId, forkOf: 4663, forkBlock: o.forkBlock,
      rpc: o.publicUrl || null, faucetWei: String(o.faucetWei), shortenedWaitS: o.waitS, listings: listingProgress(),
      membership: "Every collection's membership root on this chain is a PLACEHOLDER over the few inscription ids the catalogue holds plus fixture ids that name nothing. Nothing here has verified any collection's full item list. Every Bitcoin address is a marker string that is not a valid address. docs/phase11.md.",
      note: "A copy of Robinhood Chain at one block, with its own chain id so nothing signed here can be replayed there. Prices, Bitcoin receipts and deliveries are fixtures. The contracts' public waits are cut short once they have been public for shortenedWaitS.",
      deployment: deployment() });
    if (req.method === "GET" && url.pathname.startsWith("/snapshots/")) {
      const name = url.pathname.slice("/snapshots/".length);
      const file = `${o.data}/snapshots/${name}`;
      if (!/^snapshot-0x[0-9a-f]{40}-\d+-[a-z-]+\.json$/.test(name) || !existsSync(file)) return send(res, 404, { error: "no such snapshot" });
      return send(res, 200, readFileSync(file, "utf8"), { "cache-control": "public, max-age=31536000, immutable" });
    }
    send(res, 404, { error: "POST / is JSON-RPC; GET /info, /healthz, /snapshots/<file>; POST /faucet" });
  });
  return { server, listen: () => new Promise<void>((ok) => server.listen(o.port, "0.0.0.0", ok)) };
}
