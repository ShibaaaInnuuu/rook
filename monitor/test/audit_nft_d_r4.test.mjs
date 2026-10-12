// Fix round 4 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round4.md): the monitor's PoCs of review round 4, part D
// (review4/poc/D/monitor/test/audit4_nft_d.test.mjs) turned around, on the same fake world (the canister's own encoded state,
// test/d4-state.hex, through the monitor's IDL and plain(), as ic.mjs hands it over). Each FAILS on the code before this round.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { encodeFunctionData, encodeFunctionResult, parseAbi } from "viem";
import { EthState, plain } from "../src/candid.mjs";
import { checkEthNft, earmarkOf, ethNftAlerts } from "../src/ethnft.mjs";
import { TOPICS, judgeLogs } from "../src/evm.mjs";
import { EMPTY_RECORDS, recordIndex } from "../src/records.mjs";
import { expandSecrets, redact } from "../src/config.mjs";
import { deliver } from "../src/alerts.mjs";
import { statusBody } from "../src/main.mjs";

const E18 = 10n ** 18n;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;
const rep = (b, n = 20) => `0x${b.repeat(n)}`;
const WRAPPER = rep("aa"), MODULE = rep("bb"), DEST = rep("d0"), CUSTODIAN = A(0xc0), POSTER = A(0xf5), MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
const BUYER2 = A(0xb0b), THIEF = A(0xbad);
const NOW = 1_800_000_000_000;
const pad = (x) => `0x${BigInt(x).toString(16).padStart(64, "0")}`;
const padA = (a) => `0x${"0".repeat(24)}${a.slice(2)}`;

const WRAP = parseAbi(["function itemCount() view returns (uint256)", "function item(uint256) view returns ((uint256 tokenId, bytes32 sourceTx, bool deposit))",
  "function itemOf(uint256) view returns (uint256)",
  "function ticket(uint256) view returns ((uint8 stage, address owner, uint64 round, uint64 pinnedAt, uint64 pinnedAtBlock, uint64 deliverBy, uint64 deliverByBlock, uint256 itemId, bytes32 deliveryTx, address destination))"]);
const MOD = parseAbi(["function accepted() view returns (uint256)", "function netUnits() view returns (uint256)", "function buffer() view returns (uint256)",
  "function maxItemCost() view returns (uint256)", "function registry() view returns (address)"]);
const ERC = parseAbi(["function ownerOf(uint256) view returns (address)"]);
const sel = (abi, fn, args = []) => encodeFunctionData({ abi, functionName: fn, args }).slice(0, 10);
const err = (message) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32000, message } }));
/** `logs(from, to)`: what eth_getLogs answers for a range; `head()`: what eth_blockNumber answers. */
function world({ items = [], owner = () => CUSTODIAN, head = () => 26_000_000, balance = 10n ** 17n, nonce = 7, tickets = {}, logs = () => [], receipts = {} }) {
  const calls = [];
  const http = async (url, init) => {
    if (url.includes("opensea.io")) return new Response(JSON.stringify({ total: { floor_price: 0.93 } }));
    if (url.includes("coingecko")) return new Response(JSON.stringify({ floor_price: { native_currency: 0.93 } }));
    const b = JSON.parse(String(init.body));
    calls.push(b.method);
    const ok = (result) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }));
    if (b.method === "eth_blockNumber") return ok(`0x${head().toString(16)}`);
    if (b.method === "eth_getBalance") return ok(`0x${balance.toString(16)}`);
    if (b.method === "eth_getTransactionCount") return ok(`0x${nonce.toString(16)}`);
    if (b.method === "eth_getTransactionReceipt") return ok(receipts[String(b.params[0]).toLowerCase()] ?? null);
    if (b.method === "eth_getLogs") { const p = b.params[0]; const a = Number(BigInt(p.fromBlock)), z = Number(BigInt(p.toBlock)); calls.push(`logs:${a}-${z}`); return ok(logs(a, z)); }
    const { data } = b.params[0], s = data.slice(0, 10);
    if (url === "https://home") {
      if (s === sel(WRAP, "itemCount")) return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemCount", result: BigInt(items.length) }));
      if (s === sel(WRAP, "item", [0n])) { const i = Number(BigInt(`0x${data.slice(10)}`)); return ok(encodeFunctionResult({ abi: WRAP, functionName: "item", result: { tokenId: items[i], sourceTx: `0x${"11".repeat(32)}`, deposit: false } })); }
      if (s === sel(WRAP, "itemOf", [0n])) { const t = BigInt(`0x${data.slice(10)}`); return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemOf", result: BigInt(items.indexOf(t) + 1) })); }
      if (s === sel(WRAP, "ticket", [0n])) {
        const id = String(BigInt(`0x${data.slice(10)}`)), t = tickets[id];
        calls.push(`ticket:${id}`);
        if (!t) return err("execution reverted");
        return ok(encodeFunctionResult({ abi: WRAP, functionName: "ticket", result: { stage: t.stage ?? 2, owner: A(0x1), round: 1n, pinnedAt: 1n, pinnedAtBlock: 1n, deliverBy: 1n,
          deliverByBlock: 1n, itemId: t.itemId, deliveryTx: `0x${"00".repeat(32)}`, destination: t.destination ?? DEST } }));
      }
      if (s === sel(MOD, "accepted")) return ok(encodeFunctionResult({ abi: MOD, functionName: "accepted", result: 93n * E18 / 100n }));
      if (s === sel(MOD, "registry")) return ok(encodeFunctionResult({ abi: MOD, functionName: "registry", result: POSTER }));
      for (const fn of ["netUnits", "buffer", "maxItemCost"]) if (s === sel(MOD, fn)) return ok(encodeFunctionResult({ abi: MOD, functionName: fn, result: 0n }));
    }
    if (s === sel(ERC, "ownerOf", [0n])) return ok(encodeFunctionResult({ abi: ERC, functionName: "ownerOf", result: owner(url, BigInt(`0x${data.slice(10)}`)) }));
    throw new Error(`unexpected ${url} ${b.method} ${s}`);
  };
  return { http, calls };
}
const cfgOf = (x = {}) => ({ buyer: "rrkah-fqaaa-aaaaa-aaaaq-cai", eth_nft: { home_rpc: ["https://home"], eth_rpc: ["https://a", "https://b"],
  collections: [{ slug: "milady", collection: MILADY, wrapper: WRAPPER, module: MODULE, custodian: CUSTODIAN, opensea_slug: "milady", coingecko_id: "milady-maker" }], ...x } });
const notHeld = (r) => r.alerts.filter((a) => a.sev === "SEV1" && a.key.startsWith("eth-not-held:"));

// the canister's own bytes, as the live monitor sees them (ic.mjs: the actor's IDL decode, then plain())
const live = () => plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(readFileSync(new URL("./d4-state.hex", import.meta.url), "utf8").trim(), "hex")))[0]);

// ---- (1) F3-1 end to end ----------------------------------------------------------------------------------------------------------
const TRANSFER_TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
/** Our delivery's receipt: success and the collection's Transfer(custodian → `to`, token). */
const receipt = (to, token, status = "0x1") => ({ status, blockNumber: "0x18cba7f", logs: [{ address: MILADY, topics: [TRANSFER_TOPIC, padA(CUSTODIAN), padA(to), pad(token)], data: "0x" }] });

// ---- D4-L-1 -----------------------------------------------------------------------------------------------------------------------
test("D4-L-1: a delivery that mined but is still Sent in the canister, NFT already sold on: its own receipt exempts it (no false SEV1)", async () => {
  const st = live();
  st.deliveries = [{ ...st.deliveries[1] }];     // ticket 4, token 9183, Sent (its transfer mined a block ago)
  const tx = `0x${String(st.deliveries[0].phase.Sent.tx).replace(/^0x/, "").toLowerCase()}`;
  const w = world({ items: [9183n], tickets: { 4: { itemId: 0n } }, owner: (u, t) => (t === 9183n ? BUYER2 : CUSTODIAN), receipts: { [tx]: receipt(DEST, 9183n) } });
  const r = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w.http, ic: { ethState: async () => st } });
  assert.deepEqual(notHeld(r), [], "an honest redemption resold within a tick does not page");
  assert.ok(w.calls.includes("eth_getTransactionReceipt"), "the Sent transfer's receipt was read");
  // controls: the receipt shows the token going elsewhere, it reverted, or it is not mined: SEV1 as before
  for (const receipts of [{ [tx]: receipt(THIEF, 9183n) }, { [tx]: receipt(DEST, 9183n, "0x0") }, {}]) {
    const w2 = world({ items: [9183n], tickets: { 4: { itemId: 0n } }, owner: (u, t) => (t === 9183n ? BUYER2 : CUSTODIAN), receipts });
    const r2 = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w2.http, ic: { ethState: async () => st } });
    assert.equal(notHeld(r2).length, 1, JSON.stringify(receipts).slice(0, 80));
  }
});

// ---- D4-L-2 -----------------------------------------------------------------------------------------------------------------------
const DEPOSIT_TX = `0x${"de".repeat(32)}`;
const depLog = (block) => ({ address: MILADY, blockNumber: `0x${block.toString(16)}`, transactionHash: DEPOSIT_TX, logIndex: "0x0",
  topics: [TRANSFER_TOPIC, padA(A(0x999)), padA(CUSTODIAN), pad(4242)], data: "0x" });
test("D4-L-2: a block the provider answered short is read again at the next poll; the unclaimed deposit there pages", async () => {
  const st = live();
  st.deposits = [];
  const H = 26_000_000, D = H - 1;
  let head = H, backend = H - 2;
  const logs = (a, b) => (D >= a && D <= Math.min(b, backend) ? [depLog(D)] : []);
  const mk = () => world({ head: () => head, logs });
  const r1 = await checkEthNft({ cfg: cfgOf(), now: NOW, http: mk().http, ic: { ethState: async () => st } });
  head = H + 700; backend = head;
  const w2 = mk();
  const r2 = await checkEthNft({ cfg: cfgOf(), now: NOW + 8_400_000, http: w2.http, ic: { ethState: async () => st }, prev: r1.next });
  assert.ok(w2.calls.some((c) => c === `logs:${H + 1 - 64}-${H + 1 - 64 + 499}`), `the next poll reads the last 64 blocks again: ${w2.calls.filter((c) => c.startsWith("logs:")).join(" ")}`);
  assert.ok(r2.alerts.some((a) => a.key === `eth-unclaimed:rrkah-fqaaa-aaaaa-aaaaq-cai:${DEPOSIT_TX}`), "the unclaimed deposit at H - 1 pages");
});

// ---- D4-I-1, D4-I-2 ---------------------------------------------------------------------------------------------------------------
test("D4-I-1: a REFUNDED ticket's stale Moved delivery exempts nothing: the item held by the wrapper and owned elsewhere pages", async () => {
  const st = live();
  st.deliveries = [{ ...st.deliveries[3] }];     // ticket 6, token 9185, Moved
  const w = world({ items: [9185n], tickets: { 6: { itemId: 0n, stage: 4 } }, owner: (u, t) => (t === 9185n ? THIEF : CUSTODIAN) });
  const r = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w.http, ic: { ethState: async () => st } });
  assert.equal(notHeld(r).length, 1);
});

test("D4-I-2: /status's alert heads are redacted", async () => {
  const cfg = { eth_nft: { eth_rpc: ["https://api-ethereum-mainnet.n.dwellir.com/${D4_KEY}", "https://xrpc.cl/eth"] } };
  expandSecrets(cfg, { D4_KEY: "0123456789abcdef-SECRET" });
  const state = { sent: {} };
  await deliver([{ kind: "watcher-error", sev: "SEV3", key: "k", text: `a provider error echoing ${cfg.eth_nft.eth_rpc[0]}` }], state, { send: async () => true }, NOW);
  const body = JSON.stringify(statusBody({ timelock: "t", network: "ic" }, state, NOW));
  assert.ok(!body.includes("SECRET") && body.includes("<redacted>"), body.slice(0, 300));
});

// ---- the round-4 state the canister encodes decodes with the monitor's IDL (E4-I-4) ---------------------------------------------------
test("the round-4 eth_nft_state() fixture (every new field present) decodes, and a written-off order pages its note", () => {
  const hex = readFileSync(new URL("../../autobuyer/canister-rs/tests/fixtures/eth-state-round4.hex", import.meta.url), "utf8").trim();
  const st = plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(hex, "hex")))[0]);
  assert.equal(st.buys.length, 2);
  assert.equal(st.buys[1].late.length, 1, "the written-off order is read");
  assert.deepEqual(st.deliveries[0].sent, [true]);
  const a = ethNftAlerts({ buyer: "b", state: st, nowS: 1_800_000_000 }).alerts;
  assert.ok(a.some((x) => x.key.startsWith("eth-stuck:") && /WRITTEN OFF/.test(x.text)));
});
