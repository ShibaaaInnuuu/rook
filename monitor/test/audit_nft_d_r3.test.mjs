// Fix round 3 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/review3/R3-services.md, part D): the round-3 monitor PoCs
// (review3/poc/D/monitor/test/audit3_nft_d.test.mjs) turned around, each with its control (the loss it must still page), and the
// monitor's side of the canister/wrapper change (a delivery recorded from its own receipt; noteSent / DeliverySent; the way back's
// exits). No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { encodeAbiParameters, encodeEventTopics, encodeFunctionData, encodeFunctionResult, parseAbi, parseAbiItem, toEventSelector } from "viem";
import { diffInit } from "../src/argdiff.mjs";
import { EthState, decodeReply, plain } from "../src/candid.mjs";
import { judgeAddress } from "../src/canary.mjs";
import { redactUrl, validate } from "../src/config.mjs";
import { checkEthNft, ethNftAlerts } from "../src/ethnft.mjs";
import { TOPICS, checkEvm, judgeLogs } from "../src/evm.mjs";
import { checkLiveness } from "../src/liveness.mjs";
import { EMPTY_RECORDS, recordIndex } from "../src/records.mjs";

const E18 = 10n ** 18n;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;
const WRAPPER = A(0xaa), MODULE = A(0xbb), CUSTODIAN = A(0xc0), POSTER = A(0xf5), MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
const blob = (h) => Uint8Array.from(Buffer.from(h.replace(/^0x/, ""), "hex"));
const tok = (n) => blob(n.toString(16).padStart(64, "0"));
const DEST = A(0xd00d), BUYER2 = A(0xb0b), THIEF = A(0xbad);
const NOW = 1_800_000_000_000;

// ---- the world (the PoCs' own, plus the wrapper's ticket(), and a balance that can move between reads) ------------------------------
const WRAP = parseAbi(["function itemCount() view returns (uint256)", "function item(uint256) view returns ((uint256 tokenId, bytes32 sourceTx, bool deposit))",
  "function itemOf(uint256) view returns (uint256)",
  "function ticket(uint256) view returns ((uint8 stage, address owner, uint64 round, uint64 pinnedAt, uint64 pinnedAtBlock, uint64 deliverBy, uint64 deliverByBlock, uint256 itemId, bytes32 deliveryTx, address destination))"]);
const MOD = parseAbi(["function accepted() view returns (uint256)", "function netUnits() view returns (uint256)", "function buffer() view returns (uint256)",
  "function maxItemCost() view returns (uint256)", "function registry() view returns (address)"]);
const ERC = parseAbi(["function ownerOf(uint256) view returns (address)"]);
const sel = (abi, fn, args = []) => encodeFunctionData({ abi, functionName: fn, args }).slice(0, 10);
const err = (message) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32000, message } }));
/** `tickets`: { id: { itemId, destination } } (a ticket not given reverts); `balance`, `nonce`: a value or a function of nothing. */
function world({ items = [], owner = () => CUSTODIAN, head = 26_000_000, balance = 10n ** 16n, nonce = 0, tickets = {} }) {
  const calls = [];
  const val = (x) => (typeof x === "function" ? x() : x);
  const http = async (url, init) => {
    if (url.includes("opensea.io")) return new Response(JSON.stringify({ total: { floor_price: 0.93 } }));
    if (url.includes("coingecko")) return new Response(JSON.stringify({ floor_price: { native_currency: 0.93 } }));
    const b = JSON.parse(String(init.body));
    calls.push(b.method);
    const ok = (result) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }));
    if (b.method === "eth_blockNumber") return ok(`0x${head.toString(16)}`);
    if (b.method === "eth_getBalance") return ok(`0x${val(balance).toString(16)}`);
    if (b.method === "eth_getTransactionCount") return ok(`0x${val(nonce).toString(16)}`);
    if (b.method === "eth_getLogs") return ok([]);
    const { to, data } = b.params[0], s = data.slice(0, 10);
    if (url === "https://home") {
      if (s === sel(WRAP, "itemCount")) return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemCount", result: BigInt(items.length) }));
      if (s === sel(WRAP, "item", [0n])) { const i = Number(BigInt(`0x${data.slice(10)}`)); return ok(encodeFunctionResult({ abi: WRAP, functionName: "item", result: { tokenId: items[i], sourceTx: `0x${"11".repeat(32)}`, deposit: false } })); }
      if (s === sel(WRAP, "itemOf", [0n])) { const t = BigInt(`0x${data.slice(10)}`); return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemOf", result: BigInt(items.indexOf(t) + 1) })); }
      if (s === sel(WRAP, "ticket", [0n])) {
        const id = String(BigInt(`0x${data.slice(10)}`)), t = tickets[id];
        calls.push(`ticket:${id}`);
        if (!t) return err("execution reverted");
        return ok(encodeFunctionResult({ abi: WRAP, functionName: "ticket", result: { stage: 2, owner: A(0x1), round: 1n, pinnedAt: 1n, pinnedAtBlock: 1n, deliverBy: 1n,
          deliverByBlock: 1n, itemId: t.itemId, deliveryTx: `0x${"00".repeat(32)}`, destination: t.destination ?? DEST } }));
      }
      if (s === sel(MOD, "accepted")) return ok(encodeFunctionResult({ abi: MOD, functionName: "accepted", result: 93n * E18 / 100n }));
      if (s === sel(MOD, "registry")) return ok(encodeFunctionResult({ abi: MOD, functionName: "registry", result: to === MODULE ? POSTER : A(0xf6) }));
      for (const fn of ["netUnits", "buffer", "maxItemCost"]) if (s === sel(MOD, fn)) return ok(encodeFunctionResult({ abi: MOD, functionName: fn, result: 0n }));
    }
    if (s === sel(ERC, "ownerOf", [0n])) return ok(encodeFunctionResult({ abi: ERC, functionName: "ownerOf", result: owner(url, BigInt(`0x${data.slice(10)}`)) }));
    throw new Error(`unexpected ${url} ${b.method} ${s}`);
  };
  return { http, calls };
}
const cfgOf = () => ({ buyer: "rrkah-fqaaa-aaaaa-aaaaq-cai", eth_nft: { home_rpc: ["https://home"], eth_rpc: ["https://a", "https://b"],
  collections: [{ slug: "milady", collection: MILADY, wrapper: WRAPPER, module: MODULE, custodian: CUSTODIAN, opensea_slug: "milady", coingecko_id: "milady-maker" }] } });
const TX = blob("cd".repeat(32));
const body = (ph) => ({ Sent: { tx: TX, nonce: 5n }, Moved: { tx: TX, block: 200n }, Record: { tx: TX }, Recording: { tx: TX }, Recorded: { tx: TX },
  Cancelling: { of: TX, tx: blob("ce".repeat(32)), nonce: 6n } })[ph];
const delivery = (ph, { destination = DEST, ticket = 3n, sent = [] } = {}) => ({ wrapper: blob(WRAPPER), collection: blob(MILADY), ticket, token: tok(9180n),
  destination: blob(destination), deliver_by: 1_800_600_000n, phase: { [ph]: body(ph) }, note: "", sent });
const stateOf = (o = {}) => ({ buys: [], deposits: [], readings: [], eth_out: 0n, last_mined: [[5n, 200n]], line: [], deliveries: [], ...o });
const notHeld = (r) => r.alerts.filter((a) => a.sev === "SEV1" && a.key.startsWith("eth-not-held:"));
const run = (st, w) => checkEthNft({ cfg: cfgOf(), now: NOW, http: w.http, ic: { ethState: async () => st } });

// ---- D3-M-1 (the monitor half of D3-H-1), turned around -------------------------------------------------------------------------
test("D3-M-1 (turned around): a delivery of ours that MINED (Moved/Record/Recording), the redeemer already sold the NFT on: no SEV1", async () => {
  for (const ph of ["Moved", "Record", "Recording"]) {
    const st = stateOf({ deliveries: [delivery(ph)] });
    const w = world({ items: [9180n], owner: () => BUYER2, nonce: 6, tickets: { 3: { itemId: 0n } } });
    const r = await run(st, w);
    assert.deepEqual(notHeld(r), [], `${ph}: an honest redemption followed by a resale is not a custody loss`);
    assert.ok(w.calls.includes("ticket:3"), "the ticket is read on the wrapper, for the suspect only");
  }
  // controls: an item with no delivery of ours still pages
  const w0 = world({ items: [9180n], owner: () => BUYER2, nonce: 6, tickets: { 3: { itemId: 0n } } });
  assert.equal(notHeld(await run(stateOf(), w0)).length, 1, "no delivery of ours: SEV1");
  assert.ok(!w0.calls.some((c) => String(c).startsWith("ticket:")), "no delivery candidate: no ticket read");
  // a Sent delivery (not yet mined) whose owner is NOT its destination still pages
  const r1 = await run(stateOf({ deliveries: [delivery("Sent")] }), world({ items: [9180n], owner: () => BUYER2, nonce: 6, tickets: { 3: { itemId: 0n } } }));
  assert.equal(notHeld(r1).length, 1, "Sent, owner not the destination: SEV1");
  // a Sent delivery at its destination is fine (the transfer is just mining)
  const r2 = await run(stateOf({ deliveries: [delivery("Sent")] }), world({ items: [9180n], owner: () => DEST, nonce: 6, tickets: { 3: { itemId: 0n } } }));
  assert.deepEqual(notHeld(r2), []);
  // a mined delivery whose ticket the wrapper will not answer: paged, never silently exempt
  const r3 = await run(stateOf({ deliveries: [delivery("Moved")] }), world({ items: [9180n], owner: () => BUYER2, nonce: 6, tickets: {} }));
  assert.equal(notHeld(r3).length, 1);
  assert.match(notHeld(r3)[0].text, /ticket 3 of the wrapper could not be read/);
  // a mined delivery whose ticket (on the wrapper) is to another destination than the canister's record: paged
  const r4 = await run(stateOf({ deliveries: [delivery("Moved")] }), world({ items: [9180n], owner: () => BUYER2, nonce: 6, tickets: { 3: { itemId: 0n, destination: THIEF } } }));
  assert.equal(notHeld(r4).length, 1);
});

test("D3-M-1 (b) / D3-I-7 (turned around): a delivery's transfer won the race against our deadline cancel (Cancelling, owner = destination): no SEV1", async () => {
  const st = stateOf({ deliveries: [delivery("Cancelling")] });
  assert.deepEqual(notHeld(await run(st, world({ items: [9180n], owner: () => DEST, nonce: 7, tickets: { 3: { itemId: 0n } } }))), []);
  // control: Cancelling and anyone else holds it: SEV1
  assert.equal(notHeld(await run(st, world({ items: [9180n], owner: () => THIEF, nonce: 7, tickets: { 3: { itemId: 0n } } }))).length, 1);
});

// ---- D3-L-1, turned around ----------------------------------------------------------------------------------------------------------
test("D3-L-1 (turned around): an old Recorded delivery of the same token to the same address no longer masks a loss; a bought token is never exempt", async () => {
  const X = DEST;
  // token 9180 was item 5 once (ticket 3, Recorded to X); it came back and is held as item 0; it is at X with no delivery of item 0
  const st = stateOf({ last_mined: [[9n, 900n]], deliveries: [delivery("Recorded", { destination: X })] });
  const r = await run(st, world({ items: [9180n], owner: () => X, nonce: 10, tickets: { 3: { itemId: 5n, destination: X } } }));
  assert.equal(notHeld(r).length, 1, "the old record is of item 5, not the item held now");
  assert.match(notHeld(r)[0].text, /ticket 3 \(Recorded\) is of item 5/);
  // a mined delivery of ANOTHER ticket whose item is not this one does not mask it either
  const st2 = stateOf({ deliveries: [delivery("Moved", { ticket: 4n })] });
  assert.equal(notHeld(await run(st2, world({ items: [9180n], owner: () => BUYER2, nonce: 10, tickets: { 4: { itemId: 1n } } }))).length, 1);
  // a token the canister BOUGHT again (Bought, not yet in the wrapper), at X, with any delivery of it on record: SEV1, no ticket read
  const bought = { id: 4n, collection: blob(MILADY), module: blob(MODULE), wrapper: blob(WRAPPER), advanced: E18, attempts: 1, note: "",
    phase: { Bought: { arrived: E18, token: tok(9180n), total: E18 / 2n, tx: blob("ab".repeat(32)), block: 950n } } };
  for (const ph of ["Recorded", "Moved", "Sent"]) {
    const w = world({ items: [], owner: () => X, nonce: 10, tickets: { 3: { itemId: 0n, destination: X } } });
    const rb = await run(stateOf({ buys: [bought], deliveries: [delivery(ph, { destination: X })] }), w);
    assert.equal(notHeld(rb).length, 1, `a bought token is never a delivery (${ph})`);
    assert.ok(!w.calls.some((c) => String(c).startsWith("ticket:")));
  }
  // control: the right item's mined delivery is exempt (any owner)
  assert.deepEqual(notHeld(await run(stateOf({ deliveries: [delivery("Recorded", { destination: X })] }),
    world({ items: [9180n], owner: () => BUYER2, nonce: 10, tickets: { 3: { itemId: 0n, destination: X } } }))), []);
});

// ---- D3-L-2, turned around ----------------------------------------------------------------------------------------------------------
const bridgingBuy = (id = 1n) => ({ id, collection: blob(MILADY), module: blob(MODULE), wrapper: blob(WRAPPER), advanced: E18, attempts: 1, note: "",
  phase: { Bridging: { order_id: blob("11".repeat(32)), value: E18, min_out: 97n * E18 / 100n, deadline: 1_800_100_000n, since: 1_799_999_000n } } });
const arrivedBuy = (id = 1n, wei = 97n * E18 / 100n) => ({ ...bridgingBuy(id), phase: { Arrived: { wei, since: 1_800_000_000n } } });

test("D3-L-2 (turned around): Relay's arrival lands between the poll's first block and the second records read: no false 'ETH gone'", async () => {
  const pot = 10n ** 16n, wei = 97n * E18 / 100n;
  let n = 0, landed = false;
  const ic = { ethState: async () => (n++ === 0 ? stateOf({ buys: [bridgingBuy()], last_mined: [[4n, 100n]] }) : ((landed = true), stateOf({ buys: [arrivedBuy()], last_mined: [[4n, 100n]] }))) };
  // the chain: before the canister saw the arrival the ETH was not there; by the time its records say Arrived it is
  const w = world({ items: [], balance: () => (landed ? pot + wei : pot), nonce: 5 });
  const r = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w.http, ic });
  assert.ok(!r.alerts.some((a) => a.key.startsWith("eth-arrived-gone:")), JSON.stringify(r.alerts.map((a) => a.key)));
  // control: the records say Arrived and the ETH is not there (read after them): SEV1, keyed by the buy
  const w2 = world({ items: [], balance: pot, nonce: 5 });
  const r2 = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w2.http, ic: { ethState: async () => stateOf({ buys: [arrivedBuy()], last_mined: [[4n, 100n]] }) } });
  const gone = r2.alerts.filter((a) => a.sev === "SEV1" && a.key.startsWith("eth-arrived-gone:"));
  assert.deepEqual(gone.map((a) => a.key), [`eth-arrived-gone:${CUSTODIAN}:1`]);
  // the key names the earmarked buys (sorted): a false page for one set never mutes a real one for another
  const r3 = await checkEthNft({ cfg: cfgOf(), now: NOW, http: world({ items: [], balance: 0n, nonce: 5 }).http,
    ic: { ethState: async () => stateOf({ buys: [arrivedBuy(10n), bridgingBuy(3n), arrivedBuy(9n)], last_mined: [[4n, 100n]] }) } });
  assert.deepEqual(r3.alerts.filter((a) => a.key.startsWith("eth-arrived-gone:")).map((a) => a.key), [`eth-arrived-gone:${CUSTODIAN}:9,10`]);
});

// ---- D3-I-1 and D3-I-4 ------------------------------------------------------------------------------------------------------------
const wayBack = (phase) => ({ buys: [{ id: 7n, collection: blob(MILADY), module: blob(MODULE), wrapper: blob(WRAPPER), advanced: E18, attempts: 1, note: "", phase }],
  deliveries: [], deposits: [] });

test("D3-I-1 (turned around): right after eth_give_back (BackFunded since 0) nothing pages; 2 h after it was first seen, a sane wait", () => {
  const T = 1_800_000_000;
  const st = wayBack({ BackFunded: { wei: E18, since: 0n } });
  const r = ethNftAlerts({ buyer: "b", state: st, nowS: T });
  assert.deepEqual(r.alerts.filter((a) => a.key.startsWith("eth-back-funded:")), [], "since 0 is 'just asked', not 1970");
  const r2 = ethNftAlerts({ buyer: "b", state: st, nowS: T + 2 * 3600, prev: r.next });
  const a = r2.alerts.find((x) => x.key.startsWith("eth-back-funded:"));
  assert.ok(a, "paged at 2 h from first seen");
  assert.equal(a.key, `eth-back-funded:b:7:${T}`);
  assert.match(a.text, /has waited 120 min/);
  assert.match(a.text, /stops holding other collections' buys and waits again at Arrived/);
  assert.match(a.text, /eth_give_back\(7\) puts it back at Arrived now/);
  // a real since still counts from itself
  const r3 = ethNftAlerts({ buyer: "b", state: wayBack({ BackFunded: { wei: E18, since: BigInt(T - 3 * 3600) } }), nowS: T });
  assert.match(r3.alerts.find((x) => x.key.startsWith("eth-back-funded:")).text, /has waited 180 min/);
});

test("D3-I-4: a way-back deposit signed on the Ethereum line and unmined for an hour pages SEV2 (eth-back-sending)", () => {
  const T = 1_800_000_000;
  const st = wayBack({ BackSending: { wei: E18, order_id: blob("01".repeat(32)), value: E18, min_out: 9n * E18 / 10n, deadline: BigInt(T + 3600), tx: blob("02".repeat(32)) } });
  const r = ethNftAlerts({ buyer: "b", state: st, nowS: T });
  assert.deepEqual(r.alerts, []);
  assert.deepEqual(ethNftAlerts({ buyer: "b", state: st, nowS: T + 59 * 60, prev: r.next }).alerts, []);
  const r2 = ethNftAlerts({ buyer: "b", state: st, nowS: T + 3600, prev: r.next });
  assert.deepEqual(r2.alerts.map((a) => [a.sev, a.key, a.persistent]), [["SEV2", "eth-back-sending:b:7", true]]);
  assert.match(r2.alerts[0].text, /way-back deposit .* 0x0202.* has not been mined for 60 min/);
  assert.match(r2.alerts[0].text, /The Ethereum line waits on it/);
});

test("the way back's new exits are in the page texts: Stuck closes as a loss by eth_give_back; BackBridging stops being watched", () => {
  const T = 1_800_000_000;
  // review round 5 (D5-L-2): the watch (`last`) tells a watched Stuck from a fill-Stuck buy
  const st0 = wayBack({ Stuck: { why: "the way back's Relay order is past its deadline" } });
  st0.buys[0].last = [{ order_id: blob("01".repeat(32)) }];
  const stuck = ethNftAlerts({ buyer: "b", state: st0, nowS: T }).alerts[0];
  assert.equal(stuck.sev, "SEV1");
  // review round 4 (C4-M-1): a stuck order is still watched until the operator writes it off; a written-off one is closed as a loss
  assert.match(stuck.text, /still WATCHED/);
  assert.match(stuck.text, /eth_give_back\(7\) writes it off; a second eth_give_back\(7\) closes it/);
  const late = { order_id: blob("01".repeat(32)), value: E18, deadline: BigInt(T - 90_000) };
  const off = wayBack({ Stuck: { why: "written off" } });
  off.buys[0].late = [{ watch: late, back: true, let_go_at: BigInt(T), credited: [] }];
  const w = ethNftAlerts({ buyer: "b", state: off, nowS: T }).alerts;
  assert.match(w.find((a) => a.key.startsWith("eth-stuck:")).text, /WRITTEN OFF .*eth_give_back\(7\) closes it as a loss.*eth_credit_payout\(7,/);
  assert.match(w.find((a) => a.key.startsWith("eth-late-order:")).text, /written off unpaid/);
  const bb = ethNftAlerts({ buyer: "b", state: wayBack({ BackBridging: { order_id: blob("01".repeat(32)), value: E18, min_out: E18 / 2n, deadline: BigInt(T), since: BigInt(T - 3600) } }), nowS: T }).alerts[0];
  assert.match(bb.text, /stays watched until its payout is read/);
});

// ---- (2) eth-deliver-deadline for a delivery whose transfer mined ------------------------------------------------------------------
test("eth-deliver-deadline: a MINED delivery says so (no '30 days' as if safe); SEV1 until noteSent landed, SEV2 after; other phases unchanged", () => {
  const nowS = 1_800_600_000 - 3600;       // an hour before deliverBy
  const alertFor = (d) => ethNftAlerts({ buyer: "b", state: { buys: [], deposits: [], deliveries: [d] }, nowS }).alerts.find((a) => a.key.startsWith("eth-deliver-deadline:"));
  for (const ph of ["Moved", "Record", "Recording"]) {
    const unsent = alertFor(delivery(ph, { sent: [] }));
    assert.equal(unsent.sev, "SEV1", ph);
    assert.match(unsent.text, /its transfer MINED \(0xcdcd/);
    assert.match(unsent.text, /the canister records it once it is final/);
    assert.match(unsent.text, /noteSent\) is NOT on 4663: the wrapper's late-window refund is still possible/);
    assert.doesNotMatch(unsent.text, /the holder waits 30 days/);
    for (const sent of [[true], true]) {
      const a = alertFor(delivery(ph, { sent }));
      assert.equal(a.sev, "SEV2", `${ph} sent ${JSON.stringify(sent)}`);
      assert.match(a.text, /late-window refund is off for this ticket/);
      assert.notEqual(a.key, unsent.key, "a note landing is a new page, not muted by the SEV1's key");
    }
  }
  // an older state (no `sent` field at all) is "not sent"
  const old = delivery("Moved"); delete old.sent;
  assert.equal(alertFor(old).sev, "SEV1");
  // Sent / Waiting: unchanged
  const sentPh = alertFor(delivery("Sent"));
  assert.equal(sentPh.sev, "SEV1");
  assert.equal(sentPh.key, "eth-deliver-deadline:b:3");
  assert.match(sentPh.text, /the canister cancels a pending transfer and marks it undeliverable, or the holder waits 30 days/);
});

// ---- (3) DeliverySent from an Ethereum NFT wrapper ---------------------------------------------------------------------------------
const DS = parseAbiItem("event DeliverySent(uint256 indexed ticketId, uint256 indexed tokenId, bytes32 txHash)");
const sentLog = (ticketId, txHash, n = 1, address = WRAPPER) => ({ address, topics: encodeEventTopics({ abi: [DS], eventName: "DeliverySent", args: { ticketId, tokenId: 9180n } }),
  data: encodeAbiParameters([{ type: "bytes32" }], [txHash]), blockNumber: `0x${(3000 + n).toString(16)}`, transactionHash: `0x${String(n).padStart(64, "e")}`, logIndex: "0x0", removed: false });
const nftCfg = { wrappers: [WRAPPER], modules: [MODULE], nft_wrappers: [WRAPPER], nft_collection_of: { [WRAPPER]: MILADY } };
const idx = (deliveries) => recordIndex({ ...EMPTY_RECORDS, eth: { buys: [], deposits: [], deliveries } });
const TXH = `0x${"cd".repeat(32)}`;

test("DeliverySent: the topic is EthNftWrapper's event; backed only by a delivery of ours MINED in that very tx; anything else is SEV1", () => {
  assert.equal(TOPICS.NftDeliverySent, toEventSelector(DS));
  for (const ph of ["Moved", "Record", "Recording", "Recorded"])
    assert.deepEqual(judgeLogs({ logs: [sentLog(3n, TXH)], index: idx([delivery(ph)]), cfg: nftCfg }).alerts, [], ph);
  const cases = [["another tx", [delivery("Moved")], `0x${"ab".repeat(32)}`], ["Sent (not mined)", [delivery("Sent")], TXH],
    ["Waiting", [{ ...delivery("Sent"), phase: { Waiting: null } }], TXH], ["no delivery", [], TXH], ["another ticket", [delivery("Moved", { ticket: 4n })], TXH]];
  for (const [what, ds, h] of cases) {
    const r = judgeLogs({ logs: [sentLog(3n, h)], index: idx(ds), cfg: nftCfg });
    assert.deepEqual(r.alerts.map((a) => a.sev), ["SEV1"], what);
    assert.match(r.alerts[0].text, /A SENT NOTE WITH NO MINED DELIVERY/);
    assert.match(r.alerts[0].text, /noteSent is custodian-only: signed with the canister's EVM key; it switches off the ticket's late-window refund, so its holder is refunded only if the canister marks it undeliverable/);
  }
  // the same event from an address that is not an NFT wrapper is not this rule's
  assert.deepEqual(judgeLogs({ logs: [sentLog(3n, `0x${"ab".repeat(32)}`, 1, A(0x99))], index: idx([]), cfg: nftCfg }).alerts, []);
});

test("DeliverySent is fetched: the 4663 canary asks eth_getLogs by address only (no topic filter), so every wrapper event reaches judgeLogs", async () => {
  const filters = [];
  const http = async (url, init) => {
    const { method, params } = JSON.parse(init.body);
    if (method === "eth_getLogs") filters.push(params[0]);
    const result = method === "eth_blockNumber" ? "0x" + (71_200_100).toString(16) : [];
    return { ok: true, json: async () => ({ jsonrpc: "2.0", id: 1, result }) };
  };
  await checkEvm({ cfg: { rpcs: ["https://a.example/rpc", "https://b.example/rpc"], wrappers: [WRAPPER], floor_module: MODULE, from_block: 71_200_000, confirmations: 20 },
    prev: {}, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl" });
  assert.ok(filters.length > 0);
  for (const f of filters) {
    assert.equal(f.topics, undefined, "no topic filter");
    assert.ok([].concat(f.address).map((x) => x.toLowerCase()).includes(WRAPPER));
  }
});

// ---- (4) the Candid Delivery gains `sent`; older states decode; the live (plain) shape compares ------------------------------------
test("candid: Delivery.sent (opt bool) decodes, an older Delivery without it decodes as none; plain() blobs compare in the checks", () => {
  const Deliveries = EthState._fields.find(([n]) => n === "deliveries")[1];
  const Delivery = Deliveries._type;
  const OldDelivery = IDL.Record(Object.fromEntries(Delivery._fields.filter(([n]) => n !== "sent")));
  const d = { wrapper: blob(WRAPPER), collection: blob(MILADY), ticket: 3n, token: tok(9180n), destination: blob(DEST), deliver_by: 1_800_600_000n,
    phase: { Moved: { tx: TX, block: 200n } }, note: "" };
  const [old] = IDL.decode([IDL.Vec(Delivery)], IDL.encode([IDL.Vec(OldDelivery)], [[d]]));
  assert.deepEqual(old[0].sent, [], "an absent opt field is none");
  const [now] = IDL.decode([IDL.Vec(Delivery)], IDL.encode([IDL.Vec(Delivery)], [[{ ...d, sent: [true] }]]));
  assert.deepEqual(now[0].sent, [true]);
  // the round-2 state the canister encoded (no deliveries) still decodes with the new type
  const hex = readFileSync(new URL("../../autobuyer/canister-rs/tests/fixtures/eth-state-round2.hex", import.meta.url), "utf8").trim();
  assert.equal(IDL.decode([EthState], Uint8Array.from(Buffer.from(hex, "hex")))[0].buys.length, 4);
  // ic.mjs hands the checks plain() JSON: blobs as hex text. The deadline page, the record index and DeliverySent read it right
  const p = plain(now[0]);
  assert.equal(typeof p.wrapper, "string");
  const a = ethNftAlerts({ buyer: "b", state: { buys: [], deposits: [], deliveries: [p] }, nowS: 1_800_600_000 - 3600 }).alerts[0];
  assert.equal(a.sev, "SEV2");
  assert.match(a.text, /token 9180 to 0x000000000000000000000000000000000000d00d/);
  assert.deepEqual(judgeLogs({ logs: [sentLog(3n, TXH)], index: idx([p]), cfg: nftCfg }).alerts, [], "plain() records back the note");
});

test("the live (plain) eth_nft_state shape: a mined delivery is exempt and a bought token at a thief pages (blobs as hex text compared right)", async () => {
  const bought = { id: 4n, collection: blob(MILADY), module: blob(MODULE), wrapper: blob(WRAPPER), advanced: E18, attempts: 1, note: "",
    phase: { Bought: { arrived: E18, token: tok(9181n), total: E18 / 2n, tx: blob("ab".repeat(32)), block: 950n } } };
  const st = plain(stateOf({ buys: [bought], deliveries: [delivery("Moved")] }));
  const w = world({ items: [9180n], owner: (u, t) => (t === 9180n ? BUYER2 : THIEF), nonce: 6, tickets: { 3: { itemId: 0n } } });
  const r = await run(st, w);
  assert.deepEqual(notHeld(r).map((a) => a.key), [`eth-not-held:${MILADY}:9181`], "the mined delivery is exempt; the bought token is custody, and it is gone");
});

// ---- (5) the argument diff never prints a provider's key ----------------------------------------------------------------------------
test("argdiff: provider URLs are printed as scheme://host[:port] (+ '/…'); compared in full; a key change is 'reconfigured'", () => {
  assert.equal(redactUrl("https://api-ethereum-mainnet.n.dwellir.com/SECRETKEY"), "https://api-ethereum-mainnet.n.dwellir.com/…");
  assert.equal(redactUrl("https://u:pw@rpc.example:8545"), "https://rpc.example:8545/…");
  assert.equal(redactUrl("https://rpc.example/?apikey=K"), "https://rpc.example/…");
  assert.equal(redactUrl("https://rpc.example#K"), "https://rpc.example/…");
  assert.equal(redactUrl("https://rpc.example"), "https://rpc.example");
  assert.equal(redactUrl("https://rpc.example/"), "https://rpc.example");
  assert.equal(redactUrl("not a url KEY"), "<not a URL>");
  const base = { rpc_urls: ["https://rpc.mainnet.chain.robinhood.com", "https://gw.example/v2/OLDKEY"], btc_api_urls: ["https://mempool.space/api", "https://blockstream.info/api"] };
  const text = (next) => diffInit(base, { ...base, ...next }).lines.map((l) => `${l.verb} ${l.text}`).join("\n");
  const keyed = text({ rpc_urls: ["https://rpc.mainnet.chain.robinhood.com", "https://gw.example/v2/NEWKEY"] });
  assert.match(keyed, /CHANGES 4663 RPC providers; https:\/\/gw\.example\/… reconfigured \(a path or key changed\)/);
  assert.doesNotMatch(keyed, /OLDKEY|NEWKEY|adds|removes/);
  const added = text({ rpc_urls: [...base.rpc_urls, "https://u:pw@third.example:8545/x?apikey=K3"] });
  assert.match(added, /adds https:\/\/third\.example:8545\/…/);
  assert.doesNotMatch(added, /pw|K3|OLDKEY/);
  const swapped = text({ btc_api_urls: ["https://mempool.space/api", "https://esplora.example/KEY9/api"] });
  assert.match(swapped, /adds https:\/\/esplora\.example\/…; removes https:\/\/blockstream\.info\/…/);
  assert.doesNotMatch(swapped, /KEY9/);
  // the same set in another order is no change; a key change is always a change
  assert.equal(diffInit(base, { ...base, rpc_urls: [...base.rpc_urls].reverse() }).lines.length, 0);
  assert.ok(diffInit(base, { ...base, rpc_urls: ["https://rpc.mainnet.chain.robinhood.com", "https://gw.example/v2/OLDKEY2"] }).lines.some((l) => l.sev === "CRITICAL"));
});

test("the rest of the monitor: a bad draws provider, a liveness page and a Bitcoin explorer link never print a URL's key", async () => {
  // config refusal text
  const cfg = { timelock: "rrkah-fqaaa-aaaaa-aaaaq-cai", builds: [{ commit: "c", buyer: "a".repeat(64), timelock: "b".repeat(64) }], buyer: "rrkah-fqaaa-aaaaa-aaaaq-cai",
    evm: { rpcs: ["https://a.example", "https://b.example"] }, draws: { providers: [{ url: "https://p.example/KEYP", span: -1 }, "wss://q.example/KEYQ"] } };
  assert.throws(() => validate(cfg), (e) => /https:\/\/p\.example\/…: span/.test(e.message) && /wss:\/\/q\.example\/… needs an http/.test(e.message) && !/KEYP|KEYQ/.test(e.message));
  // liveness
  const lcfg = { liveness: [{ name: "keeper", url: "https://keeper.example/healthz?token=KEYL" }] };
  const a = await checkLiveness({ cfg: lcfg, prev: {}, now: NOW, http: async () => ({ status: 503 }) });
  const b = await checkLiveness({ cfg: lcfg, prev: a.next, now: NOW + 31 * 60_000, http: async () => ({ status: 503 }) });
  assert.match(b.alerts[0].text, /keeper is DOWN or refusing: https:\/\/keeper\.example\/… has not answered/);
  assert.doesNotMatch(b.alerts[0].text, /KEYL/);
  // the Bitcoin canary's explorer links: a public esplora's path is kept, a keyed base is cut
  const F = JSON.parse(readFileSync(new URL("../fixtures/canister.json", import.meta.url), "utf8"));
  const ES = (f) => JSON.parse(readFileSync(new URL(`../fixtures/esplora/${f}`, import.meta.url), "utf8"));
  const OUT = "2127bc9003b23d8dd342a60cfe9cb44383288c0581a49aac35e513dc8e0fce57";
  const r = judgeAddress({ label: "custody", address: "bc1pfssfalzdt9h8uwqcrs73780l723mh2axrul7keaevke7a8pg4a7szrd0ht",
    views: [{ name: "mempool.space", base: "https://mempool.space/api", txs: ES("mempool-address-txs.json") },
      { name: "esplora.example", base: "https://esplora.example/KEYB/api", txs: ES("blockstream-address-txs.json") }],
    index: recordIndex({ ...EMPTY_RECORDS, redemptions: decodeReply("redemptions", F.replies.redemptions_none_delivered), frog_buys: decodeReply("frog_buys", F.replies.frog_buys),
      holder_deposits: decodeReply("holder_deposits", F.replies.holder_deposits) }), now: NOW, timelock: "tl" });
  const t = r.alerts.map((x) => x.text).join("\n");
  assert.ok(t.includes(`https://mempool.space/tx/${OUT}`), t);
  assert.ok(t.includes(`https://esplora.example/… tx ${OUT}`), t);
  assert.doesNotMatch(t, /KEYB/);
});
