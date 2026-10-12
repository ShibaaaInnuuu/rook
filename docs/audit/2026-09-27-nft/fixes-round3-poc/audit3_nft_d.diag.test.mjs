// Round 3 of the 2026-09-27 NFT review, part D (review3/R3-services.md): monitor PoCs against HEAD 4949da0. Each test asserts the
// WEAKNESS: a pass means it is present. The world() fake is the fixer's own (monitor/test/audit_nft_d_r2.test.mjs). No network.
import assert from "node:assert/strict";
import { test } from "node:test";
import { encodeFunctionData, encodeFunctionResult, parseAbi } from "viem";
import { checkEthNft, earmarkOf } from "../src/ethnft.mjs";

const E18 = 10n ** 18n;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;
const WRAPPER = A(0xaa), MODULE = A(0xbb), CUSTODIAN = A(0xc0), POSTER = A(0xf5), MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
const blob = (h) => Uint8Array.from(Buffer.from(h.replace(/^0x/, ""), "hex"));
const tok = (n) => blob(n.toString(16).padStart(64, "0"));

// ---- the world -------------------------------------------------------------------------------------------------------------------
const WRAP = parseAbi(["function itemCount() view returns (uint256)", "function item(uint256) view returns ((uint256 tokenId, bytes32 sourceTx, bool deposit))",
  "function itemOf(uint256) view returns (uint256)", "function ticket(uint256) view returns ((uint8 stage, address owner, uint64 round, uint64 pinnedAt, uint64 pinnedAtBlock, uint64 deliverBy, uint64 deliverByBlock, uint256 itemId, bytes32 deliveryTx, address destination))"]);
const MOD = parseAbi(["function accepted() view returns (uint256)", "function netUnits() view returns (uint256)", "function buffer() view returns (uint256)",
  "function maxItemCost() view returns (uint256)", "function registry() view returns (address)"]);
const ERC = parseAbi(["function ownerOf(uint256) view returns (address)"]);
const sel = (abi, fn, args = []) => encodeFunctionData({ abi, functionName: fn, args }).slice(0, 10);
function world({ items = [], accepted = 93n * E18 / 100n, os = 0.93, cg = 0.93, owner = () => CUSTODIAN, ownerErr = {}, head = 26_000_000, logs = [], balance = 10n ** 16n,
  failItemCount = new Set(), nonce = 0 }) {
  const calls = [];
  const http = async (url, init) => {
    if (url.includes("opensea.io")) { calls.push("opensea"); return os === null ? new Response("{}", { status: 429 }) : new Response(JSON.stringify({ total: { floor_price: os } })); }
    if (url.includes("coingecko")) { calls.push("coingecko"); return cg === null ? new Response(JSON.stringify({ status: { error_code: 429 } }), { status: 429 }) : new Response(JSON.stringify({ floor_price: { native_currency: cg } })); }
    const b = JSON.parse(String(init.body));
    const ok = (result) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }));
    if (b.method === "eth_blockNumber") return ok(`0x${head.toString(16)}`);
    if (b.method === "eth_getBalance") return ok(`0x${balance.toString(16)}`);
    if (b.method === "eth_getTransactionCount") return ok(`0x${nonce.toString(16)}`);
    if (b.method === "eth_getLogs") { const from = Number(BigInt(b.params[0].fromBlock)); return ok(logs.filter((l) => Number(BigInt(l.blockNumber)) >= from)); }
    const { to, data } = b.params[0], s = data.slice(0, 10);
    if (url === "https://home") {
      if (s === sel(WRAP, "itemCount")) { if (failItemCount.has(to)) return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32005, message: "the network is busy" } })); return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemCount", result: BigInt(items.length) })); }
      if (s === sel(WRAP, "item", [0n])) { const i = Number(BigInt(`0x${data.slice(10)}`)); return ok(encodeFunctionResult({ abi: WRAP, functionName: "item", result: { tokenId: items[i], sourceTx: `0x${"11".repeat(32)}`, deposit: false } })); }
      if (s === sel(WRAP, "itemOf", [0n])) { const t = BigInt(`0x${data.slice(10)}`); return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemOf", result: BigInt(items.indexOf(t) + 1) })); }
      if (s === sel(WRAP, "ticket", [0n])) return ok(encodeFunctionResult({ abi: WRAP, functionName: "ticket", result: { stage: 2, owner: DEST, round: 0n, pinnedAt: 0n, pinnedAtBlock: 0n, deliverBy: 0n, deliverByBlock: 0n, itemId: BigInt(items.indexOf(9180n)), deliveryTx: `0x${"cd".repeat(32)}`, destination: DEST } }));
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
  return { http, calls };
}
const cfgOf = (cols = [{ slug: "milady", collection: MILADY, wrapper: WRAPPER, module: MODULE, custodian: CUSTODIAN, opensea_slug: "milady", coingecko_id: "milady-maker" }], extra = {}) =>
  ({ buyer: "rrkah-fqaaa-aaaaa-aaaaq-cai", eth_nft: { home_rpc: ["https://home"], eth_rpc: ["https://a", "https://b"], collections: cols, ...extra } });
const fakeGuardian = () => { const g = { autoPause: true, calls: [], registryPauses: 0,
  async pausePoster(p) { g.calls.push(p); return { done: true, tx: "0xpause" }; }, async pauseKeeper() { g.registryPauses++; return { done: true, tx: "0xreg" }; } }; return g; };
const stateWith = (reading) => ({ buys: [], deliveries: [], deposits: [], eth_out: 0n, last_mined: [], line: [],
  readings: reading ? [{ collection: blob(MILADY), post: { price: reading.price, confidence: 10_000, listing: reading.price, sale_median: reading.price }, at: BigInt(reading.at) }] : [] });

import { ethNftAlerts } from "../src/ethnft.mjs";

var DEST = A(0xd00d), BUYER2 = A(0xb0b), THIEF = A(0xbad);
const delivery = (phase, destination = DEST) => ({ wrapper: blob(WRAPPER), collection: blob(MILADY), ticket: 3n, token: tok(9180n), destination: blob(destination),
  deliver_by: 1_800_600_000n, phase, note: "" });
const notHeld = (r) => r.alerts.some((a) => a.sev === "SEV1" && a.key.startsWith("eth-not-held:"));

function world0({ items = [], accepted = 93n * E18 / 100n, os = 0.93, cg = 0.93, owner = () => CUSTODIAN, ownerErr = {}, head = 26_000_000, logs = [], balance = 10n ** 16n,
  failItemCount = new Set(), nonce = 0 }) {
  const calls = [];
  const http = async (url, init) => {
    if (url.includes("opensea.io")) { calls.push("opensea"); return os === null ? new Response("{}", { status: 429 }) : new Response(JSON.stringify({ total: { floor_price: os } })); }
    if (url.includes("coingecko")) { calls.push("coingecko"); return cg === null ? new Response(JSON.stringify({ status: { error_code: 429 } }), { status: 429 }) : new Response(JSON.stringify({ floor_price: { native_currency: cg } })); }
    const b = JSON.parse(String(init.body));
    const ok = (result) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }));
    if (b.method === "eth_blockNumber") return ok(`0x${head.toString(16)}`);
    if (b.method === "eth_getBalance") return ok(`0x${balance.toString(16)}`);
    if (b.method === "eth_getTransactionCount") return ok(`0x${nonce.toString(16)}`);
    if (b.method === "eth_getLogs") { const from = Number(BigInt(b.params[0].fromBlock)); return ok(logs.filter((l) => Number(BigInt(l.blockNumber)) >= from)); }
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
  return { http, calls };
}

// DIAGNOSTIC (not the reviewer's PoC): the same two D3-M-1 worlds, with the wrapper answering ticket(3) -> item 0 (token 9180) to DEST,
// which the round-3 monitor reads before it exempts a delivery. Prints, per phase, whether the SEV1 'eth-not-held' still fires.
test("DIAG D3-M-1 with ticket(uint256) answered", async () => {
  for (const ph of [{ Sent: { tx: blob("cd".repeat(32)), nonce: 5n } }, { Moved: { tx: blob("cd".repeat(32)), block: 200n } }, { Record: { tx: blob("cd".repeat(32)) } },
    { Recording: { tx: blob("cd".repeat(32)) } }]) {
    const st = { buys: [], deposits: [], readings: [], eth_out: 0n, last_mined: [[5n, 200n]], line: [], deliveries: [delivery(ph)] };
    const { http } = world({ items: [9180n], owner: () => BUYER2, nonce: 6 });
    const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, ic: { ethState: async () => st } });
    console.log(`DIAG D3-M-1 ${Object.keys(ph)[0]}, owner BUYER2 (resold): SEV1 eth-not-held = ${notHeld(r)}`);
  }
  const st = { buys: [], deposits: [], readings: [], eth_out: 0n, last_mined: [[5n, 200n]], line: [],
    deliveries: [delivery({ Cancelling: { of: blob("cd".repeat(32)), tx: blob("ce".repeat(32)), nonce: 6n } })] };
  const { http } = world({ items: [9180n], owner: () => DEST, nonce: 7 });
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, ic: { ethState: async () => st } });
  console.log(`DIAG D3-M-1 (b) Cancelling, owner DEST: SEV1 eth-not-held = ${notHeld(r)}`);
  // and without the ticket answer (the PoC's own world): the ticket read fails, so it pages by design
  const r0 = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http: (await import("node:util")) && world0({ items: [9180n], owner: () => DEST, nonce: 7 }).http, ic: { ethState: async () => st } });
  console.log(`DIAG D3-M-1 (b) without ticket(): SEV1 = ${notHeld(r0)}; text: ${r0.alerts.find((a) => a.key.startsWith("eth-not-held:"))?.text}`);
});
