// The operator's decisions of 2026-09-27 on the Ethereum NFT side, as the monitor mirrors them: J7 (15 ETH a collection, no total that
// binds), H3 at 25 ETH a day, and A2-L-1's gas-pot sweep (never read as an advance).
import assert from "node:assert/strict";
import { test } from "node:test";
import { STAGE0_EVM_CAP, STAGE0_LIMITS, diffInit, effectiveCaps } from "../src/argdiff.mjs";
import { recordIndex } from "../src/records.mjs";

// ---- the operator's Dwellir Ethereum provider (2026-09-27): eth_getLogs in ranges of at most 500 blocks; its key from the environment ----
import { encodeFunctionData, encodeFunctionResult, parseAbi } from "viem";
import { checkEthNft, logsInRanges } from "../src/ethnft.mjs";
import { expandSecrets, redact, validate } from "../src/config.mjs";
import { format } from "../src/alerts.mjs";
const E18 = 10n ** 18n;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;
const WRAPPER = A(0xaa), MODULE = A(0xbb), CUSTODIAN = A(0xc0), POSTER = A(0xf5), MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
const blob = (h) => Uint8Array.from(Buffer.from(h.replace(/^0x/, ""), "hex"));

const E = 10n ** 18n;
const col = (c) => ({ collection: c, wrapper: "0xaa", module: "0xbb", max_purchase_wei: String(E), max_premium_bps: 500 });
const eth = (cols, extra = {}) => [{ chain_id: "1", relay_max_fee_bps: 300, deposits: false, floor: [], collections: cols, gas_pot_target_wei: [], logs_max_blocks: [], ...extra }];
const base = { holder_deposits: [], wrapper_from_block: [], rpc_urls: [], btc_api_urls: [], evm_chains: [], outflow_limits: [], operator: [], frog_funding: [] };
const MIL = "0x5af0d9827e0c53e4799bb226655a1de152a425a5", AZUKI = "0xed5af388653567af2f388e6224dc7c4b3241c544";

test("J7: custody_caps = null lists each Ethereum collection at 6 items and 15 ETH, and the total is their sum", () => {
  const e = effectiveCaps({ custody_caps: [], wrapper: [], eth_nft: eth([col(MIL), col(AZUKI)]) });
  assert.deepEqual(e.collections.map((c) => [c.collection, c.max_items, c.max_value_wei]), [[`1:${MIL}`, 6, 15n * E], [`1:${AZUKI}`, 6, 15n * E]]);
  assert.deepEqual([e.global_max_items, e.global_max_value_wei], [12, 30n * E]);
  assert.equal(STAGE0_EVM_CAP.first_max_items, 24);
});

test("J7: a new Ethereum collection at 15 ETH and 22 items is what the running code accepts; 25 items is flagged as refused", () => {
  const caps = (n, v, g) => [{ collections: [{ collection: `1:${AZUKI}`, max_items: n, max_value_wei: String(v), max_item_wei: String(v) }],
    global_max_items: g, global_max_value_wei: String(v) }];
  const text = (c) => diffInit({ ...base, custody_caps: [], wrapper: [], eth_nft: [] }, { ...base, custody_caps: c, wrapper: [], eth_nft: [] })
    .lines.map((l) => l.text).join("\n");
  assert.doesNotMatch(text(caps(22, 15n * E, 22)), /refuses/);
  assert.match(text(caps(25, 15n * E, 25)), /NEW collection 1:0xed5a.*above stage 0 for a new collection/);
  assert.match(text(caps(22, 15n * E + 1n, 22)), /above stage 0/);
});

test("H3: stage 0's value a day is 25 ETH (the operator, 2026-09-27)", () => {
  assert.equal(STAGE0_LIMITS.global_value_wei, 25n * E);
});

test("A2-L-1: a change of eth_nft.gas_pot_target_wei is worded; a sweep is never read as an advance", () => {
  const t = diffInit({ ...base, custody_caps: [], wrapper: [], eth_nft: eth([col(MIL)]) },
    { ...base, custody_caps: [], wrapper: [], eth_nft: eth([col(MIL)], { gas_pot_target_wei: [String(E / 5n)] }) }).lines.map((l) => `${l.sev} ${l.text}`).join("\n");
  assert.match(t, /WARN eth_nft.gas_pot_target_wei: 0.05 ETH → 0.2 ETH/);
  const buy = (id, phase, sweep) => ({ id, collection: [1], module: [0xbb], wrapper: [0xaa], advanced: sweep ? 0n : E, attempts: 0, phase, note: "",
    sweep: sweep ? [true] : [] });
  const idx = recordIndex({ eth: { buys: [buy(1n, { Returning: { wei: 5n } }, true), buy(2n, { Returning: { wei: 5n } }, false)], deliveries: [], deposits: [] } });
  const frogs = JSON.stringify(idx.advances ?? idx, (_, v) => (typeof v === "bigint" ? v.toString() : v));
  assert.ok(!frogs.includes("eth:1"), "the sweep is not an advance record");
  assert.ok(frogs.includes("eth:2"), "the buy is");
});

// ---- the world -------------------------------------------------------------------------------------------------------------------
const WRAP = parseAbi(["function itemCount() view returns (uint256)", "function item(uint256) view returns ((uint256 tokenId, bytes32 sourceTx, bool deposit))",
  "function itemOf(uint256) view returns (uint256)"]);
const MOD = parseAbi(["function accepted() view returns (uint256)", "function netUnits() view returns (uint256)", "function buffer() view returns (uint256)",
  "function maxItemCost() view returns (uint256)", "function registry() view returns (address)"]);
const ERC = parseAbi(["function ownerOf(uint256) view returns (address)"]);
const sel = (abi, fn, args = []) => encodeFunctionData({ abi, functionName: fn, args }).slice(0, 10);
function world({ items = [], accepted = 93n * E18 / 100n, os = 0.93, cg = 0.93, owner = () => CUSTODIAN, ownerErr = {}, head = 26_000_000, logs = [], balance = 10n ** 16n,
  failItemCount = new Set(), nonce = 0 }) {
  const calls = [], ranges = [];
  const http = async (url, init) => {
    if (url.includes("opensea.io")) { calls.push("opensea"); return os === null ? new Response("{}", { status: 429 }) : new Response(JSON.stringify({ total: { floor_price: os } })); }
    if (url.includes("coingecko")) { calls.push("coingecko"); return cg === null ? new Response(JSON.stringify({ status: { error_code: 429 } }), { status: 429 }) : new Response(JSON.stringify({ floor_price: { native_currency: cg } })); }
    const b = JSON.parse(String(init.body));
    const ok = (result) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }));
    if (b.method === "eth_blockNumber") return ok(`0x${head.toString(16)}`);
    if (b.method === "eth_getBalance") return ok(`0x${balance.toString(16)}`);
    if (b.method === "eth_getTransactionCount") return ok(`0x${nonce.toString(16)}`);
    if (b.method === "eth_getLogs") { const from = Number(BigInt(b.params[0].fromBlock)), to = Number(BigInt(b.params[0].toBlock)); ranges.push([from, to]); return ok(logs.filter((l) => Number(BigInt(l.blockNumber)) >= from && Number(BigInt(l.blockNumber)) <= to)); }
    const { to, data } = b.params[0], s = data.slice(0, 10);
    if (url === "https://home") {
      if (s === sel(WRAP, "itemCount")) { if (failItemCount.has(to)) return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32005, message: "the network is busy" } })); return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemCount", result: BigInt(items.length) })); }
      if (s === sel(WRAP, "item", [0n])) { const i = Number(BigInt(`0x${data.slice(10)}`)); return ok(encodeFunctionResult({ abi: WRAP, functionName: "item", result: { tokenId: items[i], sourceTx: `0x${"11".repeat(32)}`, deposit: false } })); }
      if (s === sel(WRAP, "itemOf", [0n])) { const t = BigInt(`0x${data.slice(10)}`); return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemOf", result: BigInt(items.indexOf(t) + 1) })); }
      if (s === sel(MOD, "accepted")) return ok(encodeFunctionResult({ abi: MOD, functionName: "accepted", result: accepted }));
      if (s === sel(MOD, "registry")) return ok(encodeFunctionResult({ abi: MOD, functionName: "registry", result: to === MODULE ? POSTER : A(0xf6) }));
      for (const fn of ["netUnits", "buffer", "maxItemCost"]) if (s === sel(MOD, fn)) return ok(encodeFunctionResult({ abi: MOD, functionName: fn, result: 0n }));
    }
    if (s === sel(ERC, "ownerOf", [0n])) {
      if (ownerErr[url]) return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32000, message: ownerErr[url] } }));
      return ok(encodeFunctionResult({ abi: ERC, functionName: "ownerOf", result: owner(url, BigInt(`0x${data.slice(10)}`)) }));
    }
    throw new Error(`unexpected ${url} ${b.method} ${s}`);
  };
  return { http, calls, ranges };
}
const cfgOf = (extra = {}) => ({ buyer: "rrkah-fqaaa-aaaaa-aaaaq-cai", eth_nft: { home_rpc: ["https://home"], eth_rpc: ["https://a", "https://b"],
  collections: [{ slug: "milady", collection: MILADY, wrapper: WRAPPER, module: MODULE, custodian: CUSTODIAN, opensea_slug: "milady", coingecko_id: "milady-maker" }], ...extra } });
const empty = { buys: [], deliveries: [], deposits: [], eth_out: 0n, last_mined: [], line: [], readings: [] };

test("Dwellir: the deposit scan asks eth_getLogs in ranges of at most 500 blocks, then only the new blocks", async () => {
  const w = world({ head: 26_000_000 });
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http: w.http, ic: { ethState: async () => empty } });
  assert.ok(w.ranges.length >= 14 && w.ranges.every(([a, b]) => b - a + 1 <= 500), JSON.stringify(w.ranges));
  assert.deepEqual([w.ranges[0][0], w.ranges.at(-1)[1]], [26_000_000 - 7_200, 26_000_000], "the whole window, once");
  const w2 = world({ head: 26_000_010 });
  await checkEthNft({ cfg: cfgOf(), prev: r.next, now: 1_800_000_060_000, http: w2.http, ic: { ethState: async () => empty } });
  // review round 4 (D4-L-2): the ten new blocks and the 64 read last time, again (a node behind its load balancer heals a poll later)
  assert.deepEqual(w2.ranges, [[26_000_001 - 64, 26_000_010]], "the next poll reads the new blocks and an overlap of 64");
  // a smaller plan: eth_nft.logs_max_blocks
  const w3 = world({ head: 26_000_000 });
  await checkEthNft({ cfg: cfgOf({ logs_max_blocks: 100 }), now: 1_800_000_000_000, http: w3.http, ic: { ethState: async () => empty } });
  assert.ok(w3.ranges.every(([a, b]) => b - a + 1 <= 100) && w3.ranges.length >= 72);
  assert.throws(() => validate({ ...cfgOf({ logs_max_blocks: 5 }), timelock: "rrkah-fqaaa-aaaaa-aaaaq-cai" }), /logs_max_blocks/);
});

test("Dwellir: logsInRanges splits any span at the bound and keeps order", async () => {
  const asked = [];
  const got = await logsInRanges(async (a, b) => { asked.push([a, b]); return [{ a }]; }, 1_000n, 2_001n, 500n);
  assert.deepEqual(asked, [[1_000n, 1_499n], [1_500n, 1_999n], [2_000n, 2_001n]]);
  assert.equal(got.length, 3);
});

test("Dwellir: the key is read from the environment, never the file, and scrubbed from alerts; unset refuses the config", () => {
  const cfg = { eth_nft: { eth_rpc: ["https://api-ethereum-mainnet.n.dwellir.com/${DWELLIR_API_KEY}", "https://xrpc.cl/eth"], home_rpc: ["https://home"] } };
  assert.throws(() => expandSecrets(structuredClone(cfg), {}), /DWELLIR_API_KEY is not set/);
  const c = expandSecrets(structuredClone(cfg), { DWELLIR_API_KEY: "k3y-SECRET-123" });
  assert.equal(c.eth_nft.eth_rpc[0], "https://api-ethereum-mainnet.n.dwellir.com/k3y-SECRET-123");
  assert.equal(redact(`fetch ${c.eth_nft.eth_rpc[0]} failed`), "fetch https://api-ethereum-mainnet.n.dwellir.com/<redacted> failed");
  const text = format({ sev: "SEV2", kind: "canister", text: `provider https://api-ethereum-mainnet.n.dwellir.com/k3y-SECRET-123 refused` });
  assert.ok(!text.includes("k3y-SECRET-123") && text.includes("<redacted>"));
});
