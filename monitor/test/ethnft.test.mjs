import assert from "node:assert/strict";
import { test } from "node:test";
import { encodeFunctionResult, parseAbi } from "viem";
import { checkEthNft, ethNftAlerts, floorAlerts, poolAlerts, unclaimedAlerts, POOL_IDLE_S, ARRIVED_LATE_S, BRIDGING_LATE_S, DEPOSIT_SEEN_LATE_S, RETURN_LATE_S } from "../src/ethnft.mjs";
import { EthNftConfig, Init, decodeInit, encodeInitPlain, INIT_FIELDS } from "../src/candid.mjs";
import { diffInit } from "../src/argdiff.mjs";
import { recordIndex } from "../src/records.mjs";

// docs/v7/phase1-nft.md §6: the monitor's half of the Ethereum NFT side.
const E = 10n ** 18n;
const blob = (h) => Uint8Array.from(Buffer.from(h, "hex"));
const NOW = 1_800_000_000;

test("buy states that last too long page; a stuck one is SEV1; a returned one is INFO", () => {
  const b = (phase, extra = {}) => ({ id: 1n, collection: blob("5a".repeat(20)), module: blob("bb".repeat(20)), advanced: E, attempts: 1, phase, note: "", ...extra });
  const s = { buys: [
    b({ Bridging: { order_id: blob("ab".repeat(32)), value: E, min_out: 97n * E / 100n, deadline: BigInt(NOW + 86_400), since: BigInt(NOW - BRIDGING_LATE_S) } }),
    { ...b({ Stuck: { why: "past the deadline" } }), id: 2n },
    { ...b({ Arrived: { wei: E, since: BigInt(NOW - ARRIVED_LATE_S) } }), id: 3n },
    { ...b({ Returned: { wei: E } }), id: 4n },
    { ...b({ Funded: { wei: E, since: BigInt(NOW - 60) } }), id: 5n },
  ], deliveries: [] };
  const { alerts } = ethNftAlerts({ buyer: "x", state: s, nowS: NOW });
  const by = (k) => alerts.find((a) => a.key.startsWith(k));
  assert.equal(by("eth-bridge-late").sev, "SEV2");
  assert.equal(by("eth-stuck").sev, "SEV1");
  assert.equal(by("eth-arrived").sev, "SEV2");
  assert.equal(by("eth-returned").sev, "INFO");
  assert.equal(by("eth-funded"), undefined, "a minute of waiting is not late");
});

test("a delivery near its deadline is SEV1; one resolved over an hour ago SEV2; recorded and marked ones are quiet", () => {
  const d = (phase, by) => ({ wrapper: blob("aa".repeat(20)), collection: blob("5a".repeat(20)), ticket: 3n, token: blob("00".repeat(31) + "4d"), destination: blob("a1".repeat(20)),
    deliver_by: BigInt(by), phase, note: "" });
  const near = ethNftAlerts({ buyer: "x", state: { buys: [], deliveries: [d({ Waiting: null }, NOW + 3600)] }, nowS: NOW }).alerts;
  assert.equal(near[0].sev, "SEV1");
  const late = ethNftAlerts({ buyer: "x", state: { buys: [], deliveries: [d({ Sent: { tx: blob("01"), nonce: 1n } }, NOW + 7 * 86_400 - 7200)] }, nowS: NOW }).alerts;
  assert.equal(late[0].sev, "SEV2");
  const quiet = ethNftAlerts({ buyer: "x", state: { buys: [], deliveries: [d({ Recorded: { tx: blob("01") } }, NOW + 60), d({ Marked: { why: "x" } }, NOW + 60)] }, nowS: NOW }).alerts;
  assert.equal(quiet.length, 0);
});

test("the floor against the public references: 10 % from the accepted floor warns; the canister's own reading 20 % from BOTH (agreeing) references pages and pauses", () => {
  const refs = (v) => [{ name: "OpenSea", value: v }, { name: "CoinGecko", value: v }];
  const nowS = 1_800_000_000, fresh = (p) => ({ price: p, at: nowS - 60 });
  assert.equal(floorAlerts({ slug: "milady", module: "m", accepted: 93n * E / 100n, refs: refs(93n * E / 100n), nowS }).alerts.length, 0);
  const w = floorAlerts({ slug: "milady", module: "m", accepted: 110n * E / 100n, refs: refs(E), nowS });
  assert.equal(w.alerts.length, 0, "exactly 10 % is not over it");
  assert.equal(floorAlerts({ slug: "milady", module: "m", accepted: 115n * E / 100n, refs: refs(E), nowS }).alerts[0].sev, "SEV2");
  // the lagging accepted floor alone never pauses (audit 2026-09-27-nft D-M-1): the canister's reading does
  const lag = floorAlerts({ slug: "milady", module: "m", accepted: 70n * E / 100n, reading: fresh(E), refs: refs(E), nowS });
  assert.equal(lag.pause, false, "the module lags a real move by design: not a pause");
  const p = floorAlerts({ slug: "milady", module: "m", accepted: 70n * E / 100n, reading: fresh(70n * E / 100n), refs: refs(E), nowS });
  assert.equal(p.alerts.find((a) => a.key === "eth-floor-far:m").sev, "SEV1");
  assert.equal(p.pause, true);
  // a stale reading, one reference, or two that disagree never pause
  assert.equal(floorAlerts({ slug: "m", module: "m", accepted: E, reading: { price: E / 2n, at: nowS - 7_200 }, refs: refs(E), nowS }).pause, false);
  assert.equal(floorAlerts({ slug: "m", module: "m", accepted: E, reading: fresh(E / 2n), refs: [{ name: "a", value: E }, { name: "b", value: 0n }], nowS }).pause, false);
  assert.equal(floorAlerts({ slug: "m", module: "m", accepted: E, reading: fresh(E / 2n), refs: [{ name: "a", value: E }, { name: "b", value: 2n * E }], nowS }).pause, false);
  // latched: no second pause until acknowledged
  assert.equal(floorAlerts({ slug: "m", module: "m", accepted: E, reading: fresh(E / 2n), refs: refs(E), nowS, latched: true }).pause, false);
  // no reference at all: the floor is unwatched, and that pages
  assert.equal(floorAlerts({ slug: "m", module: "m", accepted: E, refs: [{ name: "a", value: 0n }, { name: "b", value: 0n }], nowS }).alerts[0].key, "eth-floor-unwatched:m");
});

test("custody is read on both Ethereum providers: a token owned elsewhere is SEV1, a disagreement SEV2", async () => {
  const W = "0x" + "aa".repeat(20), C = "0x" + "5a".repeat(20), M = "0x" + "bb".repeat(20), US = "0x" + "cc".repeat(20);
  const WRAP = parseAbi(["function itemCount() view returns (uint256)", "function item(uint256) view returns ((uint256 tokenId, bytes32 sourceTx, bool deposit))",
    "function itemOf(uint256) view returns (uint256)"]);
  const ERC = parseAbi(["function ownerOf(uint256) view returns (address)"]);
  const MOD = parseAbi(["function accepted() view returns (uint256)"]);
  const owners = { a: { 7: US, 8: "0x" + "ee".repeat(20) }, b: { 7: US, 8: "0x" + "ee".repeat(20) } };
  const http = async (url, init) => {
    const u = String(url);
    if (u.includes("opensea") || u.includes("coingecko")) throw new Error("offline");
    const b = JSON.parse(init.body);
    const ok = (result) => ({ json: async () => ({ result }) });
    if (b.method === "eth_blockNumber") return ok("0x100");
    if (b.method === "eth_getBalance") return ok("0x" + (10n ** 16n).toString(16));
    const { to, data } = b.params[0];
    const sel = data.slice(0, 10);
    if (to === W && sel === "0x6bfb0d01") return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemCount", result: 2n }));
    if (to === W && sel === "0x898e6219") { const i = Number(BigInt("0x" + data.slice(10))); return ok(encodeFunctionResult({ abi: WRAP, functionName: "item", result: { tokenId: BigInt(7 + i), sourceTx: "0x" + "00".repeat(32), deposit: false } })); }
    if (to === W && sel === "0xfb048705") { const t = Number(BigInt("0x" + data.slice(10))); return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemOf", result: BigInt(t - 6) })); }
    if (to === C) { const t = Number(BigInt("0x" + data.slice(10))); return ok(encodeFunctionResult({ abi: ERC, functionName: "ownerOf", result: owners[u.includes("a.") ? "a" : "b"][t] })); }
    if (to === M) return ok(encodeFunctionResult({ abi: MOD, functionName: "accepted", result: E }));
    throw new Error(`unexpected ${to} ${sel}`);
  };
  const cfg = { eth_nft: { home_rpc: ["https://home"], eth_rpc: ["https://a.eth", "https://b.eth"], collections: [{ slug: "milady", collection: C, wrapper: W, module: M, custodian: US }] } };
  const r = await checkEthNft({ cfg, http, now: NOW * 1000 });
  assert.equal(r.alerts.filter((a) => a.key.startsWith("eth-not-held")).length, 1, "token 8 is owned elsewhere");
  assert.equal(r.alerts.find((a) => a.key.startsWith("eth-not-held")).sev, "SEV1");
  owners.b[7] = "0x" + "dd".repeat(20);
  const r2 = await checkEthNft({ cfg, http, now: NOW * 1000 });
  assert.equal(r2.alerts.find((a) => a.key.startsWith("eth-owner-disagree")).sev, "SEV2");
});

test("the watcher decodes an upgrade argument that names eth_nft, and its fields are known", () => {
  const base = { chain_id: 4663n, vault: "0x00", rpc_urls: ["a", "b"], btc_api_urls: ["c", "d"], key_name: "key_1", accept_production_key_name: true,
    bridge_allowlist: [], btc_network: "mainnet", operator: [], wrapper: [], evm_chains: [], custody_caps: [], outflow_limits: [], holder_deposits: [], wrapper_from_block: [] };
  const withEth = { ...base, eth_nft: [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, floor: [], gas_pot_target_wei: [], logs_max_blocks: [], collections: [{ collection: "0x5a", wrapper: "0xaa",
    module: "0xbb", max_purchase_wei: E, max_premium_bps: 500 }] }] };
  const hex = encodeInitPlain(withEth);
  const d = decodeInit(hex);
  assert.ok(d.ok, d.error);
  assert.equal(d.init.eth_nft[0].collections[0].module, "0xbb");
  for (const f of ["eth_nft", "relay_max_fee_bps", "deposits", "max_premium_bps", "gas_pot_target_wei"]) assert.ok(INIT_FIELDS.includes(f));
  void EthNftConfig; void Init;
});

test("the watcher words an eth_nft change: on, a collection added, deposits on", () => {
  const base = { holder_deposits: [], wrapper_from_block: [], rpc_urls: [], btc_api_urls: [], evm_chains: [], custody_caps: [], outflow_limits: [], operator: [], frog_funding: [] };
  const col = (c) => ({ collection: c, wrapper: "0xaa", module: "0xbb", max_purchase_wei: String(E), max_premium_bps: 500 });
  const cfg = (cols, deposits = false) => [{ chain_id: "1", relay_max_fee_bps: 300, deposits, floor: [], collections: cols }];
  const texts = (a, b) => diffInit({ ...base, eth_nft: a }, { ...base, eth_nft: b }).lines.map((l) => `${l.sev} ${l.verb} ${l.text}`).join("\n");
  assert.match(texts([], cfg([col("0x5a")])), /CRITICAL LOOSENS eth_nft: off → ON/);
  assert.match(texts(cfg([col("0x5a")]), cfg([col("0x5a"), col("0xed")])), /CRITICAL LOOSENS eth_nft: adds collection 0xed/);
  assert.match(texts(cfg([col("0x5a")]), cfg([col("0x5a")], true)), /CRITICAL LOOSENS eth_nft.deposits: off → ON/);
  assert.equal(diffInit({ ...base, eth_nft: cfg([col("0x5a")]) }, { ...base, eth_nft: cfg([col("0x5a")]) }).unchanged.includes("eth_nft"), true);
});

test("an Ethereum buy backs its module's Advanced / AdvanceSettled events (no false SEV1 for the NFT module)", () => {
  const eth = { buys: [{ id: 2n, module: "bb".repeat(20), advanced: "500200000000000000", phase: { Settled: {} } },
    { id: 3n, module: "bb".repeat(20), advanced: "7", phase: { Bridging: {} } }, { id: 4n, module: "bb".repeat(20), advanced: "0", phase: { NeedAdvance: null } }] };
  const idx = recordIndex({ redemptions: [], holder_deposits: [], frog_buys: [], sale_state: { sales: [] }, purchases: [], conversions: [], eth });
  assert.deepEqual(idx.advances, [{ frog: "eth:2", module: `0x${"bb".repeat(20)}`, advanced: "500200000000000000", phase: "Settled" },
    { frog: "eth:3", module: `0x${"bb".repeat(20)}`, advanced: "7", phase: "Converting" }]);
});

test("holder deposits (D5): a claim not final in 2 h, a stray not returned in 2 h and a held NFT page; a returned stray is INFO", () => {
  const x = (phase, t) => ({ tx: blob(t.repeat(32)), collection: blob("5a".repeat(20)), token: blob("00".repeat(31) + "07"), recipient: blob("44".repeat(20)),
    from: blob("33".repeat(20)), block: 100n, phase });
  const state = { buys: [], deliveries: [], deposits: [x({ Seen: null }, "01"), x({ Return: { why: "no claim data" } }, "02"), x({ Held: { why: "the wrapper holds it" } }, "03"),
    x({ Returned: { tx: blob("ff") } }, "04"), x({ Minted: null }, "05"), x({ Refused: { why: "junk" } }, "06")] };
  const first = ethNftAlerts({ buyer: "x", state, nowS: NOW });
  assert.deepEqual(first.alerts.map((a) => a.sev).sort(), ["INFO", "SEV2"], "at first sight only the held one pages (and the return is news)");
  const later = ethNftAlerts({ buyer: "x", state, nowS: NOW + Math.max(DEPOSIT_SEEN_LATE_S, RETURN_LATE_S), prev: first.next }).alerts;
  const by = (k) => later.find((a) => a.key.startsWith(k));
  assert.equal(by("eth-deposit-seen").sev, "SEV2");
  assert.equal(by("eth-return-late").sev, "SEV2");
  assert.equal(by("eth-deposit-held").sev, "SEV2");
  assert.equal(by("eth-returned-stray").sev, "INFO");
  assert.equal(later.length, 4, "minted and refused ones are quiet");
});

test("an NFT that reached the custodian and was never handed in pages after 2 h; judged ones, our own fills and ERC-20s do not", () => {
  const state = { buys: [{ id: 1n, phase: { Bought: { tx: blob("cc".repeat(32)) } } }], deposits: [{ tx: blob("bb".repeat(32)) }] };
  const log = (tx, block, n = 4) => ({ transactionHash: `0x${tx.repeat(32)}`, blockNumber: `0x${block.toString(16)}`, topics: Array(n).fill("0x") });
  const a = unclaimedAlerts({ buyer: "x", slug: "milady", head: 10_000n, state, logs: [log("aa", 9_000), log("bb", 9_000), log("cc", 9_000), log("dd", 9_000, 3), log("ee", 9_900), log("aa", 9_000)] });
  assert.equal(a.length, 1);
  assert.match(a[0].text, /eth_deposit\("0xaaaa/);
});

test("a pool due a buy with no open buy for a day pages; pooling below one item plus the buffer never does", () => {
  const E = 10n ** 18n, base = { buyer: "b", slug: "bayc", collection: "0xBC", buffer: E / 20n, maxItemCost: 7n * E, nowS: 1_000 };
  // BAYC at 9 ETH: one graduation leaves 0.82 units — pooling, silent however long
  let r = poolAlerts({ ...base, netUnits: 82n * E / 100n, open: false });
  assert.equal(r.due, false);
  r = poolAlerts({ ...base, netUnits: 82n * E / 100n, open: false, nowS: 1_000 + 10 * POOL_IDLE_S });
  assert.equal(r.alerts.length, 0);
  // the second graduation: 1.64 units, due; the clock starts
  r = poolAlerts({ ...base, netUnits: 164n * E / 100n, open: false });
  assert.equal(r.due, true);
  assert.equal(r.alerts.length, 0);
  assert.equal(r.next.dueSince["0xbc"], 1_000);
  const late = poolAlerts({ ...base, netUnits: 164n * E / 100n, open: false, nowS: 1_000 + POOL_IDLE_S, prev: r.next });
  assert.equal(late.alerts.length, 1);
  assert.equal(late.alerts[0].sev, "SEV2");
  assert.match(late.alerts[0].text, /bayc: the pool holds 1\.6400 units \(a buy is due at 1\.0500\)/);
  // an open buy for this collection: nothing, and the clock resets
  const open = poolAlerts({ ...base, netUnits: 164n * E / 100n, open: true, nowS: 1_000 + POOL_IDLE_S, prev: r.next });
  assert.equal(open.alerts.length, 0);
  assert.equal(open.next.dueSince["0xbc"], undefined);
  // a stale floor is named
  const stale = poolAlerts({ ...base, netUnits: 2n * E, maxItemCost: 0n, open: false, nowS: 1_000 + POOL_IDLE_S, prev: r.next });
  assert.match(stale.alerts[0].text, /maxItemCost is 0/);
  // exactly one item plus the buffer is due; a wei under is not
  assert.equal(poolAlerts({ ...base, netUnits: E + E / 20n, open: false }).due, true);
  assert.equal(poolAlerts({ ...base, netUnits: E + E / 20n - 1n, open: false }).due, false);
});
