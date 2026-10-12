#!/usr/bin/env node
// C5-I-1 (docs/v7/phase1-nft.md §5.1; README.md here): after the operator's own real Relay transfer, fetch its transactions READ-ONLY
// (eth_getTransactionByHash, eth_getTransactionReceipt) and run them through the canister's own payout judge
// (autobuyer/canister-rs/examples/relay_verify.rs: relayconv::decode_execute / call_request_hash / call_executed, ethflow::judge_payout).
//
//   node ops/rehearsal/relay/verify.mjs --us 0x<the address paid> --payout 0x<fill or refund tx> --payout-chain 1|4663
//        [--deposit 0x<the deposit tx> --deposit-chain 4663|1] [--named <wei>] [--request-id 0x<steps[0].requestId>] [--relay-status]
//
// Env (optional): ETH_RPC (default https://rpc.mevblocker.io), HOME_RPC (default https://rpc.mainnet.chain.robinhood.com),
// RELAY_API (default https://api.relay.link), RELAY_API_KEY (only for /requests/v3; read from the environment, never printed).
// Nothing is signed or sent. Output files go to $OUT (default /tmp/rook-golive/relay-verify).
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const CANISTER = resolve(HERE, "../../../autobuyer/canister-rs");
const RPC = { 1: process.env.ETH_RPC ?? "https://rpc.mevblocker.io", 4663: process.env.HOME_RPC ?? "https://rpc.mainnet.chain.robinhood.com" };
const API = process.env.RELAY_API ?? "https://api.relay.link";
const OUT = process.env.OUT ?? "/tmp/rook-golive/relay-verify";

const args = {};
const argv = process.argv.slice(2);
for (let i = 0; i < argv.length; i++) {
  const k = argv[i];
  if (!k.startsWith("--")) throw new Error(`unexpected ${k}`);
  if (k === "--relay-status") args["relay-status"] = true;
  else args[k.slice(2)] = argv[++i];
}
const H32 = /^0x[0-9a-fA-F]{64}$/, A20 = /^0x[0-9a-fA-F]{40}$/;
const need = (c, m) => { if (!c) { console.error(`refused: ${m}`); process.exit(2); } };
need(A20.test(args.us ?? ""), "--us <the address the payout should pay>");
need(H32.test(args.payout ?? ""), "--payout <the fill or refund transaction hash>");
need(["1", "4663"].includes(args["payout-chain"]), "--payout-chain 1 or 4663");
if (args.deposit) need(H32.test(args.deposit) && ["1", "4663"].includes(args["deposit-chain"]), "--deposit <hash> needs --deposit-chain 1 or 4663");

async function rpc(chain, method, params) {
  const r = await fetch(RPC[chain], { method: "POST", headers: { "content-type": "application/json", "user-agent": "rook-golive" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
  const j = await r.json();
  if (j.error) throw new Error(`chain ${chain} ${method}: ${JSON.stringify(j.error).slice(0, 160)}`);
  return j.result;
}
async function fetchLeg(name, hash, chain) {
  const [tx, receipt] = await Promise.all([rpc(chain, "eth_getTransactionByHash", [hash]), rpc(chain, "eth_getTransactionReceipt", [hash])]);
  need(tx && receipt, `${name} ${hash}: no transaction or receipt on chain ${chain} (not mined yet, or the wrong chain)`);
  const f = `${OUT}/${name}-${hash.slice(0, 10)}.json`;
  writeFileSync(f, JSON.stringify({ chain: Number(chain), tx, receipt }, null, 1));
  console.log(`${name}: ${hash} on chain ${chain}, block ${Number(receipt.blockNumber)}, status ${receipt.status}, ${receipt.logs.length} logs -> ${f}`);
  return f;
}
async function relayStatus(hash) {
  // v2 is keyless (throttled since 2026-09-01, RETIRED 2026-11-24); v3 needs a key
  const key = process.env.RELAY_API_KEY;
  // measured 2026-10-09: v3 ignores `hash` (v2's name) and answers the latest 20 requests of anyone; it filters by depositTxHash,
  // fillTxHash or refundTxHash. Each is tried in turn and only a filtered answer is shown.
  try {
    let r, j, reqs = [];
    if (key) {
      for (const f of ["depositTxHash", "fillTxHash", "refundTxHash"]) {
        r = await fetch(`${API}/requests/v3?${f}=${hash}`, { headers: { "x-api-key": key } });
        j = await r.json(); reqs = j.requests ?? [];
        if (reqs.length) break;
      }
    } else { r = await fetch(`${API}/requests/v2?hash=${hash}`); j = await r.json(); reqs = j.requests ?? []; }
    console.log(`Relay ${key ? "/requests/v3" : "/requests/v2 (keyless; retires 2026-11-24)"} for ${hash}: HTTP ${r.status}, ${reqs.length} request(s)`);
    for (const q of reqs) console.log(`  request ${q.id}: status ${q.status}; in ${JSON.stringify((q.data?.inTxs ?? []).map((t) => [t.chainId, t.hash]))}; out ${JSON.stringify((q.data?.outTxs ?? []).map((t) => [t.chainId, t.hash]))}${q.data?.refundCurrencyData ? "; REFUND" : ""}`);
    if (!reqs.length && j.message) console.log(`  ${String(j.message).slice(0, 200)}`);
  } catch (e) { console.log(`Relay status for ${hash}: ${e.message}`); }
}

mkdirSync(OUT, { recursive: true });
const payout = await fetchLeg("payout", args.payout, args["payout-chain"]);
const deposit = args.deposit ? await fetchLeg("deposit", args.deposit, args["deposit-chain"]) : null;
if (args["relay-status"]) for (const h of [args.deposit, args.payout].filter(Boolean)) await relayStatus(h);
const ex = ["run", "--quiet", "--example", "relay_verify", "--", "--us", args.us, "--payout", payout];
if (deposit) ex.push("--deposit", deposit);
for (const k of ["named", "order-value", "order-id", "request-id"]) if (args[k]) ex.push(`--${k}`, args[k]);
console.log("");
try {
  process.stdout.write(execFileSync("cargo", ex, { cwd: CANISTER, encoding: "utf8", env: { ...process.env, PATH: `${process.env.HOME}/.cargo/bin:${process.env.PATH}` } }));
} catch (e) {
  process.stdout.write(e.stdout ?? ""); process.stderr.write(e.stderr ?? "");
  process.exit(e.status ?? 1);
}
