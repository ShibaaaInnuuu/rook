// Fix round 2 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round2.md): the round-2 monitor PoCs
// (review2/poc/D/monitor/test/audit2_nft_d.test.mjs) turned around, each with its control (the loss it must still page). No network.
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
  "function itemOf(uint256) view returns (uint256)",
  "function ticket(uint256) view returns ((uint8 stage, address owner, uint64 round, uint64 pinnedAt, uint64 pinnedAtBlock, uint64 deliverBy, uint64 deliverByBlock, uint256 itemId, bytes32 deliveryTx, address destination))"]);
const MOD = parseAbi(["function accepted() view returns (uint256)", "function netUnits() view returns (uint256)", "function buffer() view returns (uint256)",
  "function maxItemCost() view returns (uint256)", "function registry() view returns (address)"]);
const ERC = parseAbi(["function ownerOf(uint256) view returns (address)"]);
const sel = (abi, fn, args = []) => encodeFunctionData({ abi, functionName: fn, args }).slice(0, 10);
function world({ items = [], accepted = 93n * E18 / 100n, os = 0.93, cg = 0.93, owner = () => CUSTODIAN, ownerErr = {}, head = 26_000_000, logs = [], balance = 10n ** 16n,
  failItemCount = new Set(), nonce = 0, tickets = {} }) {
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
      // round 3: the wrapper's ticket (which item it delivers, to whom); a ticket not given is an RPC error
      if (s === sel(WRAP, "ticket", [0n])) { const id = String(BigInt(`0x${data.slice(10)}`)), t = tickets[id];
        if (!t) return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32000, message: "execution reverted" } }));
        return ok(encodeFunctionResult({ abi: WRAP, functionName: "ticket", result: { stage: 2, owner: A(0x1), round: 1n, pinnedAt: 1n, pinnedAtBlock: 1n, deliverBy: 1n, deliverByBlock: 1n,
          itemId: t.itemId, deliveryTx: `0x${"00".repeat(32)}`, destination: t.destination } })); }
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



// ---- D2-M-1 (a): an honest fill never pages "ETH left without a fill of ours"; ETH that really left still does ------------------------
const buyingState = (arrived, total) => ({ buys: [{ id: 1n, collection: blob(MILADY), module: blob(MODULE), wrapper: blob(WRAPPER), advanced: E18, attempts: 1, note: "",
  phase: { Buying: { arrived, token: tok(9180n), total, tx: blob("ab".repeat(32)) } } }],
  deliveries: [], readings: [], eth_out: 0n, deposits: [], last_mined: [[4n, 100n]],
  line: [{ kind: { Buy: { buy: 1n } }, nonce: 5n, hash: blob("ab".repeat(32)), value: total, max_fee: 20_000_000_000n, gas: 200_000n }] });

test("D2-M-1a (turned around): a fill of ours mined while the canister still says Buying is not ETH gone", async () => {
  const arrived = 97n * E18 / 100n, total = 93n * E18 / 100n, gas = 2n * 10n ** 15n, pot = 10n ** 16n;
  const state = buyingState(arrived, total);
  const { http } = world({ items: [], owner: () => CUSTODIAN, balance: pot + arrived - total - gas, nonce: 6 });
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, ic: { ethState: async () => state } });
  assert.ok(!r.alerts.some((a) => a.key.startsWith("eth-arrived-gone:")), JSON.stringify(r.alerts.map((a) => a.key)));
  // the control: the nonce is NOT used (nothing of ours mined) and the ETH is gone anyway: SEV1
  const { http: h2 } = world({ items: [], owner: () => CUSTODIAN, balance: pot / 2n, nonce: 5 });
  const r2 = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http: h2, ic: { ethState: async () => state } });
  assert.ok(r2.alerts.some((a) => a.sev === "SEV1" && a.key.startsWith("eth-arrived-gone:")));
  // and with the fill mined, ETH gone beyond what the fill and its gas could have spent still pages
  const { http: h3 } = world({ items: [], owner: () => CUSTODIAN, balance: 10n ** 15n, nonce: 6 });
  const r3 = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http: h3, ic: { ethState: async () => state } });
  assert.ok(r3.alerts.some((a) => a.sev === "SEV1" && a.key.startsWith("eth-arrived-gone:")));
  // the rule itself: the fill's total and gas at the line's max fee come off once its nonce is used, never before
  const b = state.buys[0], line = state.line[0];
  assert.equal(earmarkOf(b, line, 5n), arrived);
  assert.equal(earmarkOf(b, line, 6n), arrived - total - 20_000_000_000n * 300_000n);
  assert.equal(earmarkOf({ ...b, id: 2n }, line, 6n), arrived, "another buy's line is not this one's");
});

// ---- D2-M-1 (b): an honest redemption never pages "an NFT left custody"; any other owner still does ----------------------------------
test("D2-M-1b (turned around): a delivery of ours mined (Moved, recordDelivery not yet on 4663) is not a custody loss; a stranger still is", async () => {
  const DEST = A(0xd00d), THIEF = A(0xbad);
  const state = { buys: [], deposits: [], readings: [], eth_out: 0n, last_mined: [[5n, 200n]], line: [],
    deliveries: [{ wrapper: blob(WRAPPER), collection: blob(MILADY), ticket: 3n, token: tok(9180n), destination: blob(DEST), deliver_by: 1_800_600_000n,
      phase: { Moved: { tx: blob("cd".repeat(32)), block: 200n } }, note: "" }] };
  // round 3 (D3-H-1): once the transfer MINED (Moved, Record, Recording) any owner is fine (the redeemer may have sold it on); a Sent
  // one must be at its destination; the ticket (read on the wrapper) must be of this very item
  for (const [ph, owner, sev1] of [["Moved", DEST, false], ["Sent", DEST, false], ["Record", DEST, false], ["Recording", DEST, false],
    ["Moved", THIEF, false], ["Sent", THIEF, true], ["Waiting", DEST, true], ["Marked", DEST, true]]) {
    const st = { ...state, deliveries: [{ ...state.deliveries[0], phase: { [ph]: ph === "Waiting" ? null : ph === "Marked" ? { why: "x" } : { tx: blob("cd".repeat(32)), block: 200n } } }] };
    const { http } = world({ items: [9180n], owner: () => owner, nonce: 6, tickets: { 3: { itemId: 0n, destination: DEST } } });
    const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, ic: { ethState: async () => st } });
    assert.equal(r.alerts.some((a) => a.sev === "SEV1" && a.key.startsWith("eth-not-held:")), sev1, `${ph} to ${owner}`);
  }
});

// ---- D2-L-4: one failed read no longer restarts a collection's timers ------------------------------------------------------------------
test("D2-L-4 (turned around): a pool due a buy with no buy for 25 h is paged, one failed read at hour 12 or not", async () => {
  const MODX = parseAbi(["function netUnits() view returns (uint256)", "function buffer() view returns (uint256)"]);
  const state = { buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [], last_mined: [], line: [] };
  const due = (fail) => { const w = world({ failItemCount: fail ? new Set([WRAPPER]) : new Set() });
    return async (url, init) => {
      const b = init?.body ? JSON.parse(String(init.body)) : null;
      const s = b?.params?.[0]?.data?.slice(0, 10);
      if (url === "https://home" && s === encodeFunctionData({ abi: MODX, functionName: "netUnits" }).slice(0, 10))
        return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result: encodeFunctionResult({ abi: MODX, functionName: "netUnits", result: 2n * E18 }) }));
      return w.http(url, init);
    }; };
  const T = 1_800_000_000_000, H = 3_600_000;
  let prev = {}, pagedAt = null;
  for (const [t, fail] of [[0, false], [12 * H, true], [25 * H, false], [30 * H, false]]) {
    const r = await checkEthNft({ cfg: cfgOf(), prev, now: T + t, http: due(fail), ic: { ethState: async () => state } });
    prev = r.next;
    if (pagedAt === null && r.alerts.some((a) => a.key.startsWith("eth-pool-idle:"))) pagedAt = t / H;
  }
  assert.equal(pagedAt, 25, "due since hour 0: paged at hour 25, the blip at hour 12 notwithstanding");
});

// ---- the monitor's Candid types decode this round's state (every case the canister added: the way back, a held reading) ------------
test("round 2: eth_nft_state() with the way back's phases and line decodes with the monitor's IDL", async () => {
  const { readFileSync } = await import("node:fs");
  const { IDL } = await import("@icp-sdk/core/candid");
  const { EthState } = await import("../src/candid.mjs");
  const hex = readFileSync(new URL("../../autobuyer/canister-rs/tests/fixtures/eth-state-round2.hex", import.meta.url), "utf8").trim();
  const [st] = IDL.decode([EthState], Uint8Array.from(Buffer.from(hex, "hex")));
  assert.deepEqual(st.buys.map((b) => Object.keys(b.phase)[0]), ["BackFunded", "BackSending", "BackBridging", "Returning"]);
  // A2-L-1 (2026-09-27): the fourth is a gas-pot sweep on its way to fundReserve
  assert.deepEqual(st.buys.map((b) => b.sweep.length ? b.sweep[0] : null), [null, null, null, true]);
  assert.equal(Object.keys(st.line[0].kind)[0], "BackDeposit");
  assert.equal(earmarkOf(st.buys[0], st.line[0], 0n), 5n);
});

// ---- the launch monitor configuration (deployments/launch/monitor-nft.json): D8 and the no-good-post pause armed -------------------
test("round 2: the launch monitor template, filled in, is a valid configuration; without the no-good-post pause it is refused", async () => {
  const { readFileSync } = await import("node:fs");
  const { validate } = await import("../src/config.mjs");
  const raw = readFileSync(new URL("../../deployments/launch/monitor-nft.json", import.meta.url), "utf8");
  // ordinals at launch (nft-pilot.json `bitcoin`): the Bitcoin canary's three addresses, bech32 stand-ins
  const fill = (s) => s.replace(/"<the canister's (custody|float|buyer)_address\(\)>"/g, (_, k) => `"bc1p${{ custody: "q", float: "p", buyer: "z" }[k].repeat(58)}"`)
    .replace(/"<[^"]*canister id>"/g, '"rrkah-fqaaa-aaaaa-aaaaq-cai"').replace(/"<[^"]*(EOA|address|wrapper|floorModule)[^"]*>"/g, `"${A(0xabc)}"`)
    .replace(/"<[^"]*>"/g, '"x"').replace(/https:\/\/<[^"]*>/g, "https://host.example");
  const cfg = JSON.parse(fill(raw));
  cfg.builds = [{ commit: "c", buyer: "a".repeat(64), timelock: "b".repeat(64) }];
  delete cfg.install_arg_hex;
  cfg.evm.stack = { registry: A(0x1), chainId: 4663 };
  assert.doesNotThrow(() => validate(structuredClone(cfg)));
  assert.equal(cfg.eth_nft.collections.length, 11, "the allowlist (docs/v7/dormant-collections.md §1; Identity MD since 2026-10-09)");
  assert.ok(cfg.eth_nft.collections.every((c) => c.opensea_slug && (c.coingecko_id || (c.slug === "identitymd" && c.price_refs?.join() === "opensea"))),
    "both D8 references for every collection, OpenSea alone for Identity MD only (the operator, 2026-10-09)");
  assert.ok(cfg.evm.guardian.pause_on_no_good_post_h >= 2);
  const off = structuredClone(cfg);
  delete off.evm.guardian.pause_on_no_good_post_h;
  assert.throws(() => validate(off), /pause_on_no_good_post_h must be armed/);
});
