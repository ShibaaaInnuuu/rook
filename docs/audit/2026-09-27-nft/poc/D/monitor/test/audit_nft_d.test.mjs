// Audit D (docs/audit/2026-09-27-nft/D-services-deploy.md), reviewer D's monitor PoCs against 57160b0. Each test ASSERTS THE
// WEAKNESS: a pass means the weakness is present; a fix turns it red. No network.
import assert from "node:assert/strict";
import { test } from "node:test";
import { encodeAbiParameters, encodeEventTopics, encodeFunctionData, encodeFunctionResult, parseAbi, parseAbiItem } from "viem";
import { checkEthNft, floorAlerts } from "../src/ethnft.mjs";
import { judgeLogs, judgeStackLogs, TOPICS } from "../src/evm.mjs";
import { EMPTY_RECORDS, recordIndex } from "../src/records.mjs";
import { stackOf } from "../src/stack.mjs";

const E18 = 10n ** 18n;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;
const WRAPPER = A(0xaa), MODULE = A(0xbb), REGISTRY = A(0xd1), CUSTODIAN = A(0xc0), MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
const blob = (h) => Uint8Array.from(Buffer.from(h.replace(/^0x/, ""), "hex"));

// the NFT-only stack file as DeployStack writes it (one collection shown; the launch has three over one registry)
const NFT_STACK = { chainId: 4663, registry: REGISTRY, treasury: A(0xc7), poolManager: A(0xc8), keeper: A(0xc3), fromBlock: 1,
  collections: [{ slug: "milady", kind: "nft", wrapper: WRAPPER, floorModule: MODULE, raffleVault: A(0xe1), burnVault: A(0xe2), yoyoVault: A(0xe3),
    factory: A(0xf1), lock: A(0xf2), router: A(0xf3), mintDesk: A(0xf4), floorPoster: CUSTODIAN, posterSource: A(0xf5),
    custody: { kind: "evm", chainId: 1, collection: MILADY, custodian: CUSTODIAN } }] };
const FP = parseAbiItem("event FloorPosted(uint256 target, uint256 accepted, uint256 moveBps, uint16 confidenceBps, uint32 depthItems)");
const floorPosted = (target, accepted, n) => ({ address: MODULE, topics: encodeEventTopics({ abi: [FP], eventName: "FloorPosted" }),
  data: encodeAbiParameters(FP.inputs, [target, accepted, 250n, 10_000, 5]), blockNumber: `0x${(1000 + n).toString(16)}`,
  transactionHash: `0x${String(n).padStart(64, "0")}`, logIndex: "0x0", blockHash: `0x${String(n).padStart(64, "0")}`, removed: false });

// ---- D-H-2 ------------------------------------------------------------------------------------------------------------------------
test("D-H-2: the canister posts its raw reading (no floorBounds clamp); an honest +7.5 % move of the reading is judged 'a stolen key' (SEV1) and two such posts 30 min apart trigger the guardian's registry-wide pauseKeeper", () => {
  const st = stackOf({ stack: NFT_STACK });
  const T0 = 1_800_000_000_000;
  // accepted 0.93; the listing floor moves to 1.00 (one of Milady's two listings within +2 % sells: D6's measured depth).
  // ethflow.rs posts postFloor(1.00) every 30 min (POST_EVERY_SECS); the module allows 2.5 % per half hour (maxMoveBps 500/h).
  const p1 = floorPosted(100n * E18 / 100n, 95_325n * E18 / 100_000n, 1);        // accepted 0.93 × 1.025
  const p2 = floorPosted(100n * E18 / 100n, 977_081_250_000_000_000n, 2);         // accepted 0.95325 × 1.025
  const ctx = (t) => ({ st, now: t, minConf: { [MODULE]: 3000n }, times: { [`${p1.transactionHash}:0`]: T0, [`${p2.transactionHash}:0`]: T0 + 1_800_000 } });
  const one = judgeStackLogs({ logs: [p1], prev: {}, ctx: ctx(T0) });
  assert.equal(one.alerts[0].sev, "SEV1");
  assert.match(one.alerts[0].text, /THE KEEPER POSTED A milady FLOOR OUTSIDE THE MODULE'S BOUNDS.*a stolen key walking the floor, or a broken feed/s);
  const two = judgeStackLogs({ logs: [p2], prev: one.next, ctx: ctx(T0 + 1_800_000) });
  assert.equal(two.triggers.length, 1);
  assert.equal(two.triggers[0].action, "pause", "D8 arms the guardian (config.mjs refuses eth_nft without auto_pause): pauseKeeper() on the REGISTRY, every collection's poster zeroed");
  assert.match(two.triggers[0].why, /runaway milady floor: 2 posts outside the module's bounds, all up/);
});

// ---- D-M-1 ------------------------------------------------------------------------------------------------------------------------
const WRAP = parseAbi(["function itemCount() view returns (uint256)", "function item(uint256) view returns ((uint256 tokenId, bytes32 sourceTx, bool deposit))",
  "function itemOf(uint256) view returns (uint256)"]);
const MOD = parseAbi(["function accepted() view returns (uint256)"]);
const ERC = parseAbi(["function ownerOf(uint256) view returns (address)"]);
const sel = (abi, fn, args = []) => encodeFunctionData({ abi, functionName: fn, args }).slice(0, 10);
/** A fake world: 4663 at https://home, Ethereum providers https://a and https://b, OpenSea and CoinGecko. */
function world({ items = [], accepted = 93n * E18 / 100n, os = 0.93, cg = 0.93, owner = () => CUSTODIAN, ownerErr = {}, head = 26_000_000, logs = [], balance = 10n ** 16n, failItemCount = new Set() }) {
  const calls = [];
  const http = async (url, init) => {
    if (url.includes("opensea.io")) { calls.push("opensea"); return os === null ? new Response("{}", { status: 429 }) : new Response(JSON.stringify({ total: { floor_price: os } })); }
    if (url.includes("coingecko")) { calls.push("coingecko"); return cg === null ? new Response(JSON.stringify({ status: { error_code: 429 } }), { status: 429 }) : new Response(JSON.stringify({ floor_price: { native_currency: cg } })); }
    const b = JSON.parse(String(init.body));
    const ok = (result) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }));
    if (b.method === "eth_blockNumber") return ok(`0x${head.toString(16)}`);
    if (b.method === "eth_getBalance") return ok(`0x${balance.toString(16)}`);
    if (b.method === "eth_getLogs") { const from = Number(BigInt(b.params[0].fromBlock)); return ok(logs.filter((l) => Number(BigInt(l.blockNumber)) >= from)); }
    const { to, data } = b.params[0], s = data.slice(0, 10);
    if (url === "https://home") {
      if (s === sel(WRAP, "itemCount")) { if (failItemCount.has(to)) return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32005, message: "the network is busy" } })); return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemCount", result: BigInt(items.length) })); }
      if (s === sel(WRAP, "item", [0n])) { const i = Number(BigInt(`0x${data.slice(10)}`)); return ok(encodeFunctionResult({ abi: WRAP, functionName: "item", result: { tokenId: items[i], sourceTx: `0x${"11".repeat(32)}`, deposit: false } })); }
      if (s === sel(WRAP, "itemOf", [0n])) { const t = BigInt(`0x${data.slice(10)}`); return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemOf", result: BigInt(items.indexOf(t) + 1) })); }
      if (s === sel(MOD, "accepted")) return ok(encodeFunctionResult({ abi: MOD, functionName: "accepted", result: accepted }));
    }
    if (s === sel(ERC, "ownerOf", [0n])) {
      if (ownerErr[url]) return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32000, message: ownerErr[url] } }));
      return ok(encodeFunctionResult({ abi: ERC, functionName: "ownerOf", result: owner(url, BigInt(`0x${data.slice(10)}`)) }));
    }
    throw new Error(`unexpected ${url} ${b.method} ${s}`);
  };
  return { http, calls };
}
const cfgOf = (cols = [{ slug: "milady", collection: MILADY, wrapper: WRAPPER, module: MODULE, custodian: CUSTODIAN, opensea_slug: "milady", coingecko_id: "milady-maker" }]) =>
  ({ buyer: "rrkah-fqaaa-aaaaa-aaaaq-cai", eth_nft: { home_rpc: ["https://home"], eth_rpc: ["https://a", "https://b"], collections: cols } });
const fakeGuardian = () => { const g = { autoPause: true, calls: 0, paused: false, async pauseKeeper() { g.calls++; if (g.paused) return { done: false, skipped: "the keeper is already paused" }; g.paused = true; return { done: true, tx: "0xpause" }; } }; return g; };

test("D-M-1: a real 21 % drop in one collection pauses the whole registry, then every poll pages a NEW SEV1 key and calls pauseKeeper again (the key embeds nowS)", async () => {
  const g = fakeGuardian();
  const { http } = world({ accepted: 93n * E18 / 100n, os: 0.73, cg: 0.73 });   // the market fell 21.5 %; the module follows at most 5 %/h, 20 %/day
  const keys = [];
  for (let k = 0; k < 5; k++) {
    const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000 + k * 60_000, http, guardian: g });
    keys.push(...r.alerts.filter((a) => a.sev === "SEV1" && a.key.startsWith("eth-floor-pause:")).map((a) => a.key));
  }
  assert.equal(g.calls, 5, "pauseKeeper() every poll");
  assert.equal(new Set(keys).size, 5, "five distinct non-persistent SEV1 keys in five minutes: 60 SEV1 pages an hour for as long as the gap lasts");
});

test("D-M-1: after the Safe unpauses, the module can close at most ~5 % an hour, so the next poll pauses again: the D8 pause is sticky until the market comes back", async () => {
  const g = fakeGuardian();
  // the Safe unpaused; the canister's first post moved accepted 0.93 → 0.8835 (−5 %, one hour's worth at most); the market is at 0.73
  const { http } = world({ accepted: 8835n * E18 / 10_000n, os: 0.73, cg: 0.73 });
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, guardian: g });
  assert.equal(g.calls, 1, "paused again within one poll of the unpause");
  assert.ok(r.alerts.some((a) => a.key === `eth-floor-far:${MODULE}`));
});

test("D-M-1: 'more than 20 % from BOTH references' is one keyless reference when CoinGecko answers 429 (measured: the 6th keyless call in a few seconds), and nothing at all when both fail", async () => {
  const g = fakeGuardian();
  const one = world({ accepted: 93n * E18 / 100n, os: 0.70, cg: null });
  await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http: one.http, guardian: g });
  assert.equal(g.calls, 1, "OpenSea's floor_price alone paused every collection");
  const none = world({ accepted: 93n * E18 / 100n, os: null, cg: null });
  const g2 = fakeGuardian();
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http: none.http, guardian: g2 });
  assert.deepEqual([g2.calls, r.alerts.length], [0, 0], "both references down: the floor is unwatched and nothing says so");
  assert.deepEqual(floorAlerts({ slug: "x", module: MODULE, accepted: 5n * E18, refs: [] }), { alerts: [], pause: false }, "a floor 5× the market, no reference: silent");
});

// ---- D-M-2 ------------------------------------------------------------------------------------------------------------------------
const ETH_EV = {
  Undeliverable: parseAbiItem("event Undeliverable(uint256 indexed ticketId)"),
  Delivered: parseAbiItem("event Delivered(uint256 indexed ticketId, uint256 indexed tokenId, address destination, bytes32 txHash)"),
  Wrapped: parseAbiItem("event Wrapped(uint256 indexed itemId, uint256 indexed tokenId, bytes32 purchaseTx, address indexed to)"),
  Deposited: parseAbiItem("event Deposited(uint256 indexed itemId, uint256 indexed tokenId, bytes32 depositTx, address indexed to)"),
};
const ev = (name, args, n, data = "0x") => ({ address: WRAPPER, topics: encodeEventTopics({ abi: [ETH_EV[name]], eventName: name, args }), data, blockNumber: `0x${(2000 + n).toString(16)}`,
  transactionHash: `0x${String(n).padStart(64, "a")}`, logIndex: "0x0", removed: false });

test("D-M-2: the 4663 canary judges EthNftWrapper's Undeliverable against the BITCOIN redemptions: every honest eth markUndeliverable is a false SEV1 (anyone triggers one with redeemToRelist)", () => {
  // the canister's own record: ticket 7's delivery marked (a relist ticket is marked at once, ethflow as built §11.2)
  const eth = { buys: [], deposits: [], deliveries: [{ wrapper: blob(WRAPPER), collection: blob(MILADY), ticket: 7n, token: blob("00".repeat(31) + "01"), destination: blob("00".repeat(20)),
    deliver_by: 1n, phase: { Marked: { why: "a relist ticket is never delivered" } }, note: "" }] };
  const index = recordIndex({ ...EMPTY_RECORDS, eth });
  const r = judgeLogs({ logs: [ev("Undeliverable", { ticketId: 7n }, 1)], index, cfg: { wrappers: [WRAPPER], modules: [MODULE] } });
  assert.equal(r.alerts.length, 1);
  assert.equal(r.alerts[0].sev, "SEV1");
  assert.match(r.alerts[0].text, /AN ATTESTATION NO REFUSAL BACKS.*does not exist/s);
});

test("D-M-2: EthNftWrapper's custodian-gated Wrapped / Deposited / Delivered are invisible to the canary (other signatures): an unbacked mint and a false recordDelivery raise nothing", () => {
  const index = recordIndex({ ...EMPTY_RECORDS, eth: { buys: [], deliveries: [], deposits: [] } });   // no record backs any of these
  const logs = [
    ev("Wrapped", { itemId: 1n, tokenId: 9180n, to: A(0xbad) }, 2, encodeAbiParameters([{ type: "bytes32" }], [`0x${"de".repeat(32)}`])),
    ev("Deposited", { itemId: 2n, tokenId: 9181n, to: A(0xbad) }, 3, encodeAbiParameters([{ type: "bytes32" }], [`0x${"ad".repeat(32)}`])),
    ev("Delivered", { ticketId: 0n, tokenId: 9180n }, 4, encodeAbiParameters([{ type: "address" }, { type: "bytes32" }], [A(0xbad), `0x${"00".repeat(31)}01`])),
  ];
  assert.ok(logs.every((l) => !Object.values(TOPICS).includes(l.topics[0])), "none of their topics is in SIGS");
  assert.deepEqual(judgeLogs({ logs, index, cfg: { wrappers: [WRAPPER], modules: [MODULE] } }).alerts, []);
});

// ---- D-M-3 ------------------------------------------------------------------------------------------------------------------------
test("D-M-3: no Ethereum outbound canary: a bought-not-yet-minted NFT that left the canister, and an arrived buy's ETH that left, page nothing", async () => {
  const state = { buys: [
    { id: 1n, collection: blob(MILADY), module: blob(MODULE), advanced: E18, attempts: 1, note: "", phase: { Final: { arrived: 97n * E18 / 100n, token: blob((9180).toString(16).padStart(64, "0")), total: 93n * E18 / 100n, tx: blob("ab".repeat(32)) } } },
    { id: 2n, collection: blob(MILADY), module: blob(MODULE), advanced: E18, attempts: 1, note: "", phase: { Arrived: { wei: 97n * E18 / 100n, since: 1_799_999_000n } } },
  ], deliveries: [], readings: [], eth_out: 0n, deposits: [] };
  // the wrapper holds nothing yet (the Final buy is not minted); ownerOf(9180) is a stranger; the canister's balance is 0 (the arrived ETH is gone)
  const { http } = world({ items: [], owner: () => A(0xbad), balance: 0n });
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, ic: { ethState: async () => state } });
  assert.deepEqual(r.alerts.filter((a) => a.sev === "SEV1" || a.kind === "custody-outbound" || /gas|balance|ownerOf/.test(a.text)), [],
    "custody is read only for items the wrapper holds, and the gas pot only once an item is held");
});

// ---- D-L-3 ------------------------------------------------------------------------------------------------------------------------
test("D-L-3: an inbound NFT not judged within 7,200 blocks drops out of the watchdog's window: its page stops and nothing mints or returns it", async () => {
  const head = 26_000_000;
  const log = (n, tx) => ({ address: MILADY, topics: ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef", `0x${"00".repeat(12)}${"11".repeat(20)}`, `0x${"00".repeat(12)}${CUSTODIAN.slice(2)}`, `0x${"00".repeat(31)}05`],
    blockNumber: `0x${n.toString(16)}`, transactionHash: tx });
  const state = { buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] };
  const young = world({ head, logs: [log(head - 700, `0x${"a1".repeat(32)}`)] });
  const r1 = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http: young.http, ic: { ethState: async () => state } });
  assert.equal(r1.alerts.filter((a) => a.key.startsWith("eth-unclaimed:")).length, 1, "2 h on: paged");
  const old = world({ head, logs: [log(head - 7_300, `0x${"a1".repeat(32)}`)] });
  const r2 = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http: old.http, ic: { ethState: async () => state } });
  assert.equal(r2.alerts.filter((a) => a.key.startsWith("eth-unclaimed:")).length, 0, "a day on: silent, still unjudged (the runner's DEPOSIT_LOOKBACK_BLOCKS is the same 7,200)");
});

// ---- D-L-4 ------------------------------------------------------------------------------------------------------------------------
test("D-L-4: a provider ERROR is compared as if it were an owner: one flaky provider pages a new SEV2 key every poll; both failing alike is a false SEV1 custody loss", async () => {
  const keys = [];
  for (const head of [26_000_000, 26_000_005, 26_000_010]) {
    const { http } = world({ items: [9180n], head, ownerErr: { "https://b": "rate limited" } });
    const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http });
    keys.push(...r.alerts.filter((a) => a.key.startsWith("eth-owner-disagree:")).map((a) => a.key));
  }
  assert.equal(new Set(keys).size, 3, "the key embeds the block: one page per poll per held token");
  const both = world({ items: [9180n], ownerErr: { "https://a": "header not found", "https://b": "header not found" } });
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http: both.http });
  const lost = r.alerts.find((a) => a.key.startsWith("eth-not-held:"));
  assert.equal(lost?.sev, "SEV1");
  assert.match(lost.text, /owned by error: eth_call/);
});

// ---- D-L-5 ------------------------------------------------------------------------------------------------------------------------
test("D-L-5: one collection's failed read throws the whole ethnft checker: the next collection's floor is not compared and its guardian pause never runs", async () => {
  const W2 = A(0xab), M2 = A(0xbc);
  const cols = [{ slug: "milady", collection: MILADY, wrapper: WRAPPER, module: MODULE, custodian: CUSTODIAN },
    { slug: "azuki", collection: "0xed5af388653567af2f388e6224dc7c4b3241c544", wrapper: W2, module: M2, custodian: CUSTODIAN, opensea_slug: "azuki" }];
  const g = fakeGuardian();
  const { http } = world({ os: 0.40, failItemCount: new Set([WRAPPER]) });     // Azuki's floor far off; Milady's wrapper read is busy
  await assert.rejects(checkEthNft({ cfg: cfgOf(cols), now: 1_800_000_000_000, http, guardian: g }), /busy/);
  assert.equal(g.calls, 0);
});
