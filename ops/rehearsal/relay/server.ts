// C5-I-1 (docs/v7/phase1-nft.md §5.1; README.md here) as a page: the four real Relay transfers of the rehearsal, each quoted, judged,
// signed in the operator's own wallet, followed to its payout, and run through the canister's own payout judge (verify.mjs).
//
//   cd ops && RELAY_API_KEY=… npx tsx rehearsal/relay/server.ts        then open http://localhost:8766
//
// It listens on 127.0.0.1 only. It holds no wallet key: the browser's wallet signs. RELAY_API_KEY stays in this process (Relay's API does
// not accept the key from a browser page) and is never sent to the page or printed. The quote is judged here by the canister's own rules
// as the runner applies them (ops/src/ethnft.ts preJudgeEth, at the canister's 3 % bound): one deposit to Relay's pinned depository,
// depositNative(you, orderId), the order id recomputed from the order, the output and every refund to you, no calls, no fees.
import { execFile } from "node:child_process";
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { preJudgeEth } from "../../src/ethnft.js";
import { ETH_NATIVE } from "../../src/relay.js";

const PORT = Number(process.env.PORT ?? 8766);
const API = "https://api.relay.link";
const KEY = process.env.RELAY_API_KEY ?? "";
const PAGE = new URL("./page.html", import.meta.url);
const VERIFY = new URL("./verify.mjs", import.meta.url).pathname;
const MAX_WEI = 10_000_000_000_000_000n;            // 0.01 ETH: the rehearsal's own bound
const A20 = /^0x[0-9a-fA-F]{40}$/, H32 = /^0x[0-9a-fA-F]{64}$/;
const LEGS: Record<string, [number, number]> = { out: [4663, 1], back: [1, 4663] };

type Json = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any
const scrub = (s: string) => (KEY ? s.split(KEY).join("[redacted]") : s);

/** The quote, judged. A forced refund (Relay's documented `debug-force-refund` referrer) needs the key. */
export async function quote(b: Json): Promise<Json> {
  const leg = LEGS[b.dir];
  if (!leg) throw new Error("dir is out or back");
  if (!A20.test(b.user ?? "")) throw new Error("user is not an address");
  const amount = BigInt(b.amountWei);
  if (amount <= 0n || amount > MAX_WEI) throw new Error("amount is over the rehearsal's 0.01 ETH");
  if (b.forceRefund && !KEY) throw new Error("a forced refund needs RELAY_API_KEY in the server's environment");
  const [origin, destination] = leg, user = String(b.user).toLowerCase();
  // the runner's own quote body (ops/src/ethnft.ts ethQuoteBody): no slippage setting, as the canister's quotes are made
  const body: Json = { user, originChainId: origin, destinationChainId: destination, originCurrency: ETH_NATIVE, destinationCurrency: ETH_NATIVE,
    recipient: user, refundTo: user, tradeType: "EXACT_INPUT", amount: amount.toString(), ...(b.forceRefund ? { referrer: "debug-force-refund" } : {}) };
  const r = await fetch(`${API}/quote/v2`, { method: "POST", signal: AbortSignal.timeout(20_000),
    headers: { "content-type": "application/json", ...(KEY ? { "x-api-key": KEY } : {}) }, body: JSON.stringify(body) });
  const q = await r.json();
  if (!r.ok) throw new Error(`Relay refused the quote: HTTP ${r.status} ${scrub(String(q?.message ?? ""))}`);
  const judged = preJudgeEth(q, { buy: 0n, amountWei: amount, user, recipient: user, refundTo: user, originChainId: origin, destinationChainId: destination },
    Math.floor(Date.now() / 1000), 300);
  const d = q.steps[0].items[0].data, out = q.protocol.v2.orderData.output.payments[0];
  return { tx: { to: d.to, data: d.data, value: "0x" + BigInt(d.value).toString(16), chainId: origin }, requestId: q.steps[0].requestId,
    orderId: judged.orderId, minOut: String(judged.minOut), expectedOut: String(out.expectedAmount), origin, destination,
    payoutChain: b.forceRefund ? origin : destination, forceRefund: !!b.forceRefund };
}

async function status(requestId: string): Promise<Json> {
  if (!H32.test(requestId)) throw new Error("not a request id");
  const r = await fetch(`${API}/intents/status/v3?requestId=${requestId}`, { signal: AbortSignal.timeout(20_000) });
  const j = await r.json();
  return { status: j.status ?? null, inTxHashes: j.inTxHashes ?? [], txHashes: j.txHashes ?? [] };
}

/** The canister's own judge on the real transactions (verify.mjs, which runs the canister's relay_verify example). */
function verify(b: Json): Promise<string> {
  const ok = A20.test(b.us ?? "") && H32.test(b.payout ?? "") && H32.test(b.deposit ?? "") && H32.test(b.requestId ?? "")
    && ["1", "4663"].includes(String(b.payoutChain)) && ["1", "4663"].includes(String(b.depositChain));
  if (!ok) return Promise.reject(new Error("verify needs us, deposit, depositChain, payout, payoutChain, requestId"));
  const args = [VERIFY, "--us", b.us, "--deposit", b.deposit, "--deposit-chain", String(b.depositChain), "--payout", b.payout,
    "--payout-chain", String(b.payoutChain), "--request-id", b.requestId, "--relay-status"];
  return new Promise((ok2) => execFile("node", args, { env: process.env, timeout: 600_000, maxBuffer: 4_000_000 },
    (err, stdout, stderr) => ok2(scrub(`${stdout}${stderr}${err && !stdout ? `\n${err.message}` : ""}`))));
}

if (process.argv[1] && /server\.ts$/.test(process.argv[1])) {
  createServer(async (req, res) => {
    const send = (code: number, v: unknown, type = "application/json") => { res.writeHead(code, { "content-type": type }); res.end(typeof v === "string" ? v : JSON.stringify(v)); };
    try {
      const url = new URL(req.url ?? "/", "http://localhost");
      if (req.method === "GET" && url.pathname === "/") return send(200, readFileSync(PAGE, "utf8"), "text/html; charset=utf-8");
      if (req.method === "GET" && url.pathname === "/api/status") return send(200, await status(url.searchParams.get("requestId") ?? ""));
      if (req.method === "GET" && url.pathname === "/api/config") return send(200, { key: !!KEY });
      if (req.method === "POST") {
        let raw = ""; for await (const c of req) { raw += c; if (raw.length > 10_000) throw new Error("too large"); }
        const b = JSON.parse(raw || "{}");
        if (url.pathname === "/api/quote") return send(200, await quote(b));
        if (url.pathname === "/api/verify") return send(200, { text: await verify(b) });
      }
      send(404, { error: "not found" });
    } catch (e) { send(400, { error: scrub(String((e as Error).message)).slice(0, 500) }); }
  }).listen(PORT, "127.0.0.1", () => console.log(`Relay rehearsal: http://localhost:${PORT} (Relay key ${KEY ? "set" : "NOT set: the forced refunds will be refused"})`));
}
