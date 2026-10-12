// Review round 5, part D (docs/audit/2026-09-27-nft/review5/R5-services.md): the monitor at 1dc2a61 against the round-4 fixes
// (D4-L-1, D4-L-2, D4-I-1, D4-I-3, C4-M-1's pages, the IDLs). The canister's OWN bytes: test/d4-state.hex (review4's fixture, the
// 1eb5788 build), test/d5-state.hex (this review's d5_fixture.rs, the 1dc2a61 build) and the canister's eth-state-round4.hex. A test
// named "D5-…" passes while the weakness is present; one named "holds: …" passes while the fix holds. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { encodeFunctionData, encodeFunctionResult, parseAbi } from "viem";
import { EthState, plain } from "../src/candid.mjs";
import { checkEthNft, ethNftAlerts } from "../src/ethnft.mjs";

const E18 = 10n ** 18n;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;
const rep = (b, n = 20) => `0x${b.repeat(n)}`;
const WRAPPER = rep("aa"), MODULE = rep("bb"), DEST = rep("d0"), CUSTODIAN = A(0xc0), POSTER = A(0xf5), MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
const BUYER2 = A(0xb0b), THIEF = A(0xbad), OTHER = A(0x0ce);
const NOW = 1_800_000_000_000;
const pad = (x) => `0x${BigInt(x).toString(16).padStart(64, "0")}`;
const padA = (a) => `0x${"0".repeat(24)}${a.slice(2)}`;
const TRANSFER_TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

const WRAP = parseAbi(["function itemCount() view returns (uint256)", "function item(uint256) view returns ((uint256 tokenId, bytes32 sourceTx, bool deposit))",
  "function itemOf(uint256) view returns (uint256)",
  "function ticket(uint256) view returns ((uint8 stage, address owner, uint64 round, uint64 pinnedAt, uint64 pinnedAtBlock, uint64 deliverBy, uint64 deliverByBlock, uint256 itemId, bytes32 deliveryTx, address destination))"]);
const MOD = parseAbi(["function accepted() view returns (uint256)", "function netUnits() view returns (uint256)", "function buffer() view returns (uint256)",
  "function maxItemCost() view returns (uint256)", "function registry() view returns (address)"]);
const ERC = parseAbi(["function ownerOf(uint256) view returns (address)"]);
const sel = (abi, fn, args = []) => encodeFunctionData({ abi, functionName: fn, args }).slice(0, 10);
const err = (message) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32000, message } }));
/**
 * The fixer's fake world (audit_nft_d_r4.test.mjs), with `receipt(url, hash)` per provider (null: not mined there; "error": the
 * provider errors) and `logs(url, from, to)`.
 */
function world({ items = [], owner = () => CUSTODIAN, head = () => 26_000_000, balance = 10n ** 17n, nonce = 7, tickets = {}, logs = () => [], receipt = () => null }) {
  const calls = [];
  const http = async (url, init) => {
    if (url.includes("opensea.io")) return new Response(JSON.stringify({ total: { floor_price: 0.93 } }));
    if (url.includes("coingecko")) return new Response(JSON.stringify({ floor_price: { native_currency: 0.93 } }));
    const b = JSON.parse(String(init.body));
    calls.push(b.method);
    const ok = (result) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }));
    if (b.method === "eth_blockNumber") return ok(`0x${head(url).toString(16)}`);
    if (b.method === "eth_getBalance") return ok(`0x${balance.toString(16)}`);
    if (b.method === "eth_getTransactionCount") return ok(`0x${nonce.toString(16)}`);
    if (b.method === "eth_getTransactionReceipt") {
      const r = receipt(url, String(b.params[0]).toLowerCase());
      calls.push(`receipt:${url}:${String(b.params[0]).slice(0, 6)}`);
      return r === "error" ? err("internal error") : ok(r ?? null);
    }
    if (b.method === "eth_getLogs") { const p = b.params[0]; const a = Number(BigInt(p.fromBlock)), z = Number(BigInt(p.toBlock)); calls.push(`logs:${a}-${z}`); return ok(logs(url, a, z)); }
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
const decodeHex = (h) => plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(h.trim(), "hex")))[0]);
const d4 = () => decodeHex(readFileSync(new URL("./d4-state.hex", import.meta.url), "utf8"));
const d5 = () => decodeHex(readFileSync(new URL("./d5-state.hex", import.meta.url), "utf8"));
const rcpt = ({ to = DEST, token, from = CUSTODIAN, address = MILADY, status = "0x1" }) => ({ status, blockNumber: "0x18cba7f",
  logs: [{ address, topics: [TRANSFER_TOPIC, padA(from), padA(to), pad(token)], data: "0x" }] });
const hx = (s) => `0x${String(s).replace(/^0x/, "").toLowerCase()}`;

// ---- D4-L-1: what holds --------------------------------------------------------------------------------------------------------------
test("holds: D4-L-1 exempts only a receipt that succeeded on EVERY provider and holds THIS collection's Transfer(custodian -> destination, token)", async () => {
  const st = d4();
  st.deliveries = [{ ...st.deliveries[1] }];                     // ticket 4, token 9183, Sent (canister bytes)
  const tx = hx(st.deliveries[0].phase.Sent.tx);
  const run = async (receipt) => {
    const w = world({ items: [9183n], tickets: { 4: { itemId: 0n } }, owner: (u, t) => (t === 9183n ? BUYER2 : CUSTODIAN), receipt });
    return notHeld(await checkEthNft({ cfg: cfgOf(), now: NOW, http: w.http, ic: { ethState: async () => st } })).length;
  };
  assert.equal(await run((u, h) => (h === tx ? rcpt({ token: 9183n }) : null)), 0, "both providers show it mined to the destination: exempt");
  const pages = {
    "another contract's Transfer": (u, h) => (h === tx ? rcpt({ token: 9183n, address: OTHER }) : null),
    "another token": (u, h) => (h === tx ? rcpt({ token: 9184n }) : null),
    "not from the custodian": (u, h) => (h === tx ? rcpt({ token: 9183n, from: OTHER }) : null),
    "to another address": (u, h) => (h === tx ? rcpt({ token: 9183n, to: BUYER2 }) : null),
    "reverted": (u, h) => (h === tx ? rcpt({ token: 9183n, status: "0x0" }) : null),
    "mined on one provider only": (u, h) => (h === tx && u === "https://a" ? rcpt({ token: 9183n }) : null),
    "the second provider errors": (u, h) => (u === "https://b" ? "error" : h === tx ? rcpt({ token: 9183n }) : null),
    "the first provider errors": (u, h) => (u === "https://a" ? "error" : h === tx ? rcpt({ token: 9183n }) : null),
  };
  for (const [what, receipt] of Object.entries(pages)) assert.equal(await run(receipt), 1, `${what}: SEV1 (fails closed)`);
});

test("holds: a Cancelling delivery is exempt only when the delivery it cancels (`of`) mined, never when the cancel mined", async () => {
  const st = d4();
  const OF = rep("44", 32), CANCEL = rep("4c", 32);
  st.deliveries = [{ ...st.deliveries[1], phase: { Cancelling: { of: OF.slice(2), tx: CANCEL.slice(2), nonce: "5" } } }];
  const run = async (receipt) => {
    const w = world({ items: [9183n], tickets: { 4: { itemId: 0n } }, owner: (u, t) => (t === 9183n ? BUYER2 : CUSTODIAN), receipt });
    return notHeld(await checkEthNft({ cfg: cfgOf(), now: NOW, http: w.http, ic: { ethState: async () => st } })).length;
  };
  assert.equal(await run((u, h) => (h === OF ? rcpt({ token: 9183n }) : null)), 0, "the delivery won the race: exempt");
  assert.equal(await run((u, h) => (h === CANCEL ? { status: "0x1", logs: [] } : null)), 1, "the cancel mined and the NFT is elsewhere: SEV1");
});

// ---- D5-L-1 ----------------------------------------------------------------------------------------------------------------------------
test("D5-L-1: a delivery whose record lands on 4663 during the poll (ticket Delivered) pages a false SEV1 since D4-I-1's stage check", async () => {
  const st = d5();
  st.deliveries = st.deliveries.filter((d) => String(d.ticket) === "7");   // ticket 7, token 9188, Recording (canister bytes)
  assert.equal(Object.keys(st.deliveries[0].phase)[0], "Recording");
  // the wrapper's items were read (item 0 = token 9188 held) before the canister's record mined; the ticket, read at the end of the
  // poll, is Delivered (stage 3): recordDelivery set it and deleted itemOf in one transaction
  const w = world({ items: [9188n], tickets: { 7: { itemId: 0n, stage: 3 } }, owner: (u, t) => (t === 9188n ? BUYER2 : CUSTODIAN) });
  const r = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w.http, ic: { ethState: async () => st } });
  assert.equal(notHeld(r).length, 1, "a recorded, honest delivery pages 'pause, then look'");
  assert.match(notHeld(r)[0].text, /ticket 7 \(Recording\) is of item 0 to 0xd0d0/, notHeld(r)[0].text);
  // control: one poll earlier (ticket still Resolved) the same delivery is exempt; the code before 1dc2a61 exempted both
  const w2 = world({ items: [9188n], tickets: { 7: { itemId: 0n, stage: 2 } }, owner: (u, t) => (t === 9188n ? BUYER2 : CUSTODIAN) });
  assert.equal(notHeld(await checkEthNft({ cfg: cfgOf(), now: NOW, http: w2.http, ic: { ethState: async () => st } })).length, 0);
});

test("D5-L-1 (the window): the wrapper's items are read first and the tickets last, after every collection's reads and the canary", async () => {
  const st = d5();
  st.deliveries = st.deliveries.filter((d) => String(d.ticket) === "7");
  const w = world({ items: [9188n, 1n, 2n, 3n, 4n, 5n, 6n, 7n], tickets: { 7: { itemId: 0n } }, owner: (u, t) => (t === 9188n ? BUYER2 : CUSTODIAN) });
  await checkEthNft({ cfg: cfgOf(), now: NOW, http: w.http, ic: { ethState: async () => st } });
  const firstItemOf = w.calls.indexOf("eth_call"), ticketAt = w.calls.findIndex((c) => c.startsWith("ticket:"));
  const between = w.calls.slice(firstItemOf, ticketAt).filter((c) => !c.startsWith("logs:") && !c.startsWith("receipt:")).length;
  assert.ok(between > 30, `requests between the first item read and the ticket read: ${between} (8 items, one collection)`);
});

// ---- D5-I-1 ----------------------------------------------------------------------------------------------------------------------------
test("D5-I-1: a re-priced delivery whose EARLIER version mined is not exempt: the monitor reads only the newest hash (line.earlier not decoded)", async () => {
  const st = d5();
  st.deliveries = st.deliveries.filter((d) => String(d.ticket) === "4");   // Sent { tx: V2 }, the line's earlier = [V1]
  const V1 = rep("44", 32), V2 = rep("45", 32);
  assert.equal(hx(st.deliveries[0].phase.Sent.tx), V2);
  assert.deepEqual(Object.keys(st.line[0]).sort(), ["gas", "hash", "kind", "max_fee", "nonce", "value"], "the monitor's EthTx has no `earlier`/`rivals`");
  const w = world({ items: [9183n], tickets: { 4: { itemId: 0n } }, owner: (u, t) => (t === 9183n ? BUYER2 : CUSTODIAN),
    receipt: (u, h) => (h === V1 ? rcpt({ token: 9183n }) : null) });
  const r = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w.http, ic: { ethState: async () => st } });
  assert.equal(notHeld(r).length, 1, "the redeemer resold within the tick: SEV1 although version 1 delivered it");
  assert.match(notHeld(r)[0].text, /not mined on a/);
});

// ---- D5-L-2 ----------------------------------------------------------------------------------------------------------------------------
test("D5-L-2: the SEV1 Stuck page tells the operator a fill-Stuck buy (token not ours after it mined) is 'still WATCHED' and to wait a day", () => {
  const st = d5();
  const a = ethNftAlerts({ buyer: "b", state: st, nowS: 1_800_000_000 }).alerts;
  const stuck = (id) => a.find((x) => x.key === `eth-stuck:b:${id}`);
  // buy 1: a forward order past its deadline, watched: the text is right
  assert.match(stuck(1).text, /still WATCHED/);
  // buy 2: the fill mined but the token is someone else's; nothing is watched (the watch ended at the arrival), eth_give_back closes it at once
  assert.match(stuck(2).text, /ownerOf is 0x0+ba\./);
  assert.match(stuck(2).text, /still WATCHED \(it holds the Relay leg for every collection\)/, "wrong: it holds nothing");
  assert.match(stuck(2).text, /not before its deadline \+ 1 h \+ 24 h\), eth_give_back\(2\) writes it off; a second eth_give_back\(2\) closes it/, "wrong: one call closes it");
  assert.doesNotMatch(stuck(2).text, /pause|custody/i, "no word that an NFT left custody");
  // the monitor's IDL does not decode `last`, so it cannot tell buy 1 from buy 2
  assert.equal(st.buys[0].last, undefined);
  // buy 6: a sweep written off: 'returnAdvance of 0, the module books the loss' is not what closes a sweep (Returned, nothing)
  assert.match(stuck(6).text, /returnAdvance of 0, the module books the loss/);
});

test("holds: the C4-M-1 pages fire on the canister's round-4 states (way back late, written-off orders SEV3 until credited)", () => {
  const st = d5();
  const a = ethNftAlerts({ buyer: "b", state: st, nowS: 1_800_000_000 }).alerts;
  const keys = a.map((x) => `${x.sev} ${x.key}`);
  assert.ok(keys.includes("SEV2 eth-back-late:b:5:1798990000"), keys.join("\n"));
  for (const id of [3, 4, 6]) assert.ok(keys.includes(`SEV3 eth-late-order:b:${id}`), `written-off order ${id}: ${keys.join(" | ")}`);
  assert.ok(!keys.some((k) => /eth-late-order:b:(1|2|5)$/.test(k)));
  assert.match(a.find((x) => x.key === "eth-stuck:b:3").text, /WRITTEN OFF/);
  // credited (the canister's own round-4 fixture: buy 1 written off AND credited): no SEV3
  const r4 = decodeHex(readFileSync(new URL("../../autobuyer/canister-rs/tests/fixtures/eth-state-round4.hex", import.meta.url), "utf8"));
  assert.ok(!ethNftAlerts({ buyer: "b", state: r4, nowS: 1_800_000_000 }).alerts.some((x) => x.key.startsWith("eth-late-order:")));
  assert.equal(r4.buys[1].late[0].credited[0][2], true);
});

test("D5-I-3: the SEV3 for a written-off order is sent once (not persistent); nothing pages when its payout actually lands", () => {
  const st = d5();
  const a = ethNftAlerts({ buyer: "b", state: st, nowS: 1_800_000_000 }).alerts.filter((x) => x.key.startsWith("eth-late-order:"));
  assert.ok(a.length === 3 && a.every((x) => !x.persistent));
});

// ---- D4-L-2 ----------------------------------------------------------------------------------------------------------------------------
const DEPOSIT_TX = `0x${"de".repeat(32)}`;
const depLog = (block) => ({ address: MILADY, blockNumber: `0x${block.toString(16)}`, transactionHash: DEPOSIT_TX, logIndex: "0x0",
  topics: [TRANSFER_TOPIC, padA(A(0x999)), padA(CUSTODIAN), pad(4242)], data: "0x" });

test("holds: D4-L-2 re-reads the last 64 blocks every poll, and a deposit read twice is one unclaimed entry and one page", async () => {
  const st = d4();
  st.deposits = [];
  const H = 26_000_000, D = H - 10;
  let head = H;
  const w = () => world({ head: () => head, logs: (u, a, b) => (D >= a && D <= b ? [depLog(D)] : []) });
  const r1 = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w().http, ic: { ethState: async () => st } });
  head = H + 5;
  const w2 = w();
  const r2 = await checkEthNft({ cfg: cfgOf(), now: NOW + 60_000, http: w2.http, ic: { ethState: async () => st }, prev: r1.next });
  assert.deepEqual(w2.calls.filter((c) => c.startsWith("logs:")), [`logs:${H + 1 - 64}-${H + 5}`], "one request: 64 + 5 blocks");
  assert.deepEqual(Object.keys(r2.next.unclaimed), [DEPOSIT_TX], "read twice, remembered once");
  head = H + 700;
  const r3 = await checkEthNft({ cfg: cfgOf(), now: NOW + 9_000_000, http: w().http, ic: { ethState: async () => st }, prev: r2.next });
  assert.equal(r3.alerts.filter((a) => a.key.startsWith("eth-unclaimed:")).length, 1);
});

test("D5-I-2 (a): a poll whose head has not moved past the cursor re-reads the WHOLE 7,200-block window (15 requests), not 64 blocks", async () => {
  const st = d4();
  st.deposits = [];
  const H = 26_000_000;
  let head = H;
  const w = () => world({ head: () => head });
  const r1 = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w().http, ic: { ethState: async () => st } });
  const w2 = w();   // same head (no new block in the poll interval, or a load-balanced node one block behind the last)
  await checkEthNft({ cfg: cfgOf(), now: NOW + 30_000, http: w2.http, ic: { ethState: async () => st }, prev: r1.next });
  assert.equal(w2.calls.filter((c) => c.startsWith("logs:")).length, 15);
  head = H - 1;
  const w3 = w();
  await checkEthNft({ cfg: cfgOf(), now: NOW + 60_000, http: w3.http, ic: { ethState: async () => st }, prev: r1.next });
  assert.equal(w3.calls.filter((c) => c.startsWith("logs:")).length, 15);
});

test("D5-I-2 (b): a logs node more than 64 blocks behind the head node still loses a deposit for good", async () => {
  const st = d4();
  st.deposits = [];
  const H = 26_000_000, D = H - 65;
  let head = H, backend = H - 70;
  const logs = (u, a, b) => (D >= a && D <= Math.min(b, backend) ? [depLog(D)] : []);
  const w = () => world({ head: () => head, logs });
  const r1 = await checkEthNft({ cfg: cfgOf(), now: NOW, http: w().http, ic: { ethState: async () => st } });
  head = H + 5; backend = head;
  const r2 = await checkEthNft({ cfg: cfgOf(), now: NOW + 60_000, http: w().http, ic: { ethState: async () => st }, prev: r1.next });
  head = H + 700; backend = head;
  const r3 = await checkEthNft({ cfg: cfgOf(), now: NOW + 9_000_000, http: w().http, ic: { ethState: async () => st }, prev: r2.next });
  assert.equal(r3.alerts.filter((a) => a.key.startsWith("eth-unclaimed:")).length, 0, "never read: never paged");
});

// ---- IDLs ------------------------------------------------------------------------------------------------------------------------------
test("holds: the monitor's IDL is subtype-safe for EthBuy.late: absent (an older canister) is none; extra fields (a newer one) are skipped", () => {
  const Blob = IDL.Vec(IDL.Nat8);
  const Watch = IDL.Record({ order_id: Blob, value: IDL.Nat, min_out: IDL.Nat, deadline: IDL.Nat64, balance_after: IDL.Nat, eth_before: IDL.Nat,
    eth_out_before: IDL.Nat, pin: IDL.Opt(IDL.Nat64), solver: IDL.Opt(Blob), future: IDL.Opt(IDL.Text) });
  const Late = IDL.Record({ watch: Watch, back: IDL.Bool, let_go_at: IDL.Nat64, credited: IDL.Opt(IDL.Tuple(Blob, IDL.Nat, IDL.Bool)), future: IDL.Nat });
  const Buy = (late) => IDL.Record({ id: IDL.Nat64, collection: Blob, module: Blob, wrapper: Blob, advanced: IDL.Nat, attempts: IDL.Nat32,
    phase: IDL.Variant({ Stuck: IDL.Record({ why: IDL.Text }) }), note: IDL.Text, ...(late ? { late: IDL.Opt(Late) } : {}) });
  const St = (late) => IDL.Record({ buys: IDL.Vec(Buy(late)), deliveries: IDL.Vec(IDL.Null), readings: IDL.Vec(IDL.Null), eth_out: IDL.Nat,
    deposits: IDL.Vec(IDL.Null) });
  const b = { id: 1n, collection: new Uint8Array(20), module: new Uint8Array(20), wrapper: new Uint8Array(20), advanced: 1n, attempts: 1, phase: { Stuck: { why: "x" } }, note: "" };
  const w = { order_id: new Uint8Array(32), value: 5n, min_out: 4n, deadline: 9n, balance_after: 1n, eth_before: 2n, eth_out_before: 3n, pin: [], solver: [], future: [] };
  const newer = plain(IDL.decode([EthState], IDL.encode([St(true)], [{ buys: [{ ...b, late: [{ watch: w, back: true, let_go_at: 5n, credited: [], future: 7n }] }], deliveries: [], readings: [], eth_out: 0n, deposits: [] }]))[0]);
  assert.equal(newer.buys[0].late[0].watch.value, "5");
  const older = plain(IDL.decode([EthState], IDL.encode([St(false)], [{ buys: [b], deliveries: [], readings: [], eth_out: 0n, deposits: [] }]))[0]);
  assert.deepEqual(older.buys[0].late, []);
});
