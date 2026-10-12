// The Bitcoin canary, on two REAL mainnet transactions as mempool.space and blockstream.info serve them (fixtures/esplora):
// bc1pfssf… received 2,910 sats in b6ceea8e… and spent them in 2127bc90… to bc1pfqwu…. Treated here as a custody address. The
// canister records are the canister's own Rust encoding (fixtures/canister.json). No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decodeReply } from "../src/candid.mjs";
import { GRACE_MS, STILL_MS, DOWN_MS, checkBitcoin, fetchAddressTxs, judgeAddress } from "../src/canary.mjs";
import { EMPTY_RECORDS, recordIndex } from "../src/records.mjs";

const F = JSON.parse(readFileSync(new URL("../fixtures/canister.json", import.meta.url), "utf8"));
const ES = (f) => JSON.parse(readFileSync(new URL(`../fixtures/esplora/${f}`, import.meta.url), "utf8"));
const ADDR = "bc1pfssfalzdt9h8uwqcrs73780l723mh2axrul7keaevke7a8pg4a7szrd0ht";
const OUT = "2127bc9003b23d8dd342a60cfe9cb44383288c0581a49aac35e513dc8e0fce57", IN = "b6ceea8eb7e46c5784c6ff3eb3af6e5eca91f53ed8a06cdda7fe6f6c1b5f738e";
const T0 = 1_790_000_000_000;
const views = () => [{ name: "mempool.space", base: "https://mempool.space/api", txs: ES("mempool-address-txs.json") },
  { name: "blockstream.info", base: "https://blockstream.info/api", txs: ES("blockstream-address-txs.json") }];
const records = ({ redemptions = "redemptions_delivered", frogs = true } = {}) => ({ ...EMPTY_RECORDS,
  redemptions: decodeReply("redemptions", F.replies[redemptions]), frog_buys: frogs ? decodeReply("frog_buys", F.replies.frog_buys) : [],
  holder_deposits: decodeReply("holder_deposits", F.replies.holder_deposits) });
const judge = (o = {}) => judgeAddress({ label: "custody", address: ADDR, views: o.views ?? views(), index: recordIndex(o.records ?? records()),
  prev: o.prev, now: o.now ?? T0, expect: o.expect, acknowledged: o.acknowledged, timelock: "tl" });

test("the fixtures are what the explorers served: both providers, the same two transactions", () => {
  const [m, b] = views();
  assert.deepEqual(m.txs.map((t) => t.txid), [OUT, IN]);
  assert.deepEqual(b.txs.map((t) => t.txid), [OUT, IN]);
  assert.ok("sigops" in m.txs[0] && !("sigops" in b.txs[0]), "the two really differ in shape; the canary reads only what both have");
});

test("a spend that matches a canister record (a delivery, same destination) raises nothing, and is judged once", () => {
  const r = judge();
  assert.deepEqual(r.alerts, [], JSON.stringify(r.alerts));
  assert.equal(r.next.judged[OUT], "matched");
  assert.equal(r.next.judged[IN], "inbound", "the inbound is the frog purchase the canister recorded landing at custody");
  assert.deepEqual(judge({ prev: r.next }).alerts, []);
});

test("a spend no record names is SEV1 with the transaction, both links and PAUSE NOW, and keeps repeating", () => {
  const r = judge({ records: records({ redemptions: "redemptions_none_delivered" }) });
  assert.equal(r.alerts.length, 1);
  const a = r.alerts[0];
  assert.deepEqual([a.kind, a.sev, a.persistent, a.key], ["custody-outbound", "SEV1", true, `outbound:${OUT}`]);
  assert.match(a.text, /UNEXPECTED OUTBOUND TRANSACTION FROM CUSTODY .*: PAUSE NOW/);
  assert.match(a.text, /spends 2,910 sats from custody \(b6ceea8e\S+:0\)/);
  assert.match(a.text, /pays: 330 sats → bc1pfqwuxf27gj0c8ayer55tg720fpz80d2jprrultlrlzeat2qrzasqkdyn8p/);
  assert.match(a.text, /seen by mempool\.space and blockstream\.info/);
  assert.match(a.text, /https:\/\/mempool\.space\/tx\/2127bc90/);
  assert.match(a.text, /https:\/\/blockstream\.info\/tx\/2127bc90/);
  assert.match(a.text, /call tl pause_target/);
  // the next poll produces it again (dedup decides when to resend), even once it has scrolled off the page
  assert.equal(judge({ prev: r.next, records: records({ redemptions: "redemptions_none_delivered" }) }).alerts[0].key, a.key);
  const gone = views().map((v) => ({ ...v, txs: v.txs.filter((t) => t.txid !== OUT) }));
  assert.equal(judge({ prev: r.next, views: gone }).alerts[0].key, a.key);
  // until it is acknowledged
  assert.deepEqual(judge({ prev: r.next, acknowledged: [OUT], records: records({ redemptions: "redemptions_none_delivered" }) }).alerts, []);
});

test("a spend that matches a delivery record by txid but pays another destination is SEV1", () => {
  const r = judge({ records: records({ redemptions: "redemptions_wrong_destination" }) });
  assert.equal(r.alerts.length, 1);
  assert.equal(r.alerts[0].sev, "SEV1");
  assert.match(r.alerts[0].text, /PAID THE WRONG ADDRESS/);
  assert.match(r.alerts[0].text, /names bc1pfssf\S+; output 0 pays bc1pfqwu/);
});

test("provider disagreement over an unexplained spend: SEV1 at once, naming both sides, never guessing", () => {
  const v = views();
  v[1].txs = v[1].txs.filter((t) => t.txid !== OUT);
  const r = judge({ views: v, records: records({ redemptions: "redemptions_none_delivered" }) });
  assert.equal(r.alerts.length, 1);
  assert.deepEqual([r.alerts[0].kind, r.alerts[0].sev], ["provider-disagreement", "SEV1"]);
  assert.match(r.alerts[0].text, /mempool\.space reports an UNEXPECTED spend .* blockstream\.info does not show it/);
  assert.match(r.alerts[0].text, /Not guessing which is right/);
});

test("provider disagreement over a recorded spend: nothing during propagation, SEV2 after 15 min, again after an hour", () => {
  const v = views();
  v[1].txs = v[1].txs.filter((t) => t.txid !== OUT);
  const a = judge({ views: v });
  assert.deepEqual(a.alerts, []);
  assert.equal(a.next.oneSided[OUT], T0);
  const b = judge({ views: v, prev: a.next, now: T0 + GRACE_MS });
  assert.deepEqual(b.alerts.map((x) => [x.kind, x.sev, x.key]), [["provider-disagreement", "SEV2", `disagree:${OUT}`]]);
  const c = judge({ views: v, prev: b.next, now: T0 + STILL_MS });
  assert.deepEqual(c.alerts.map((x) => x.key), [`disagree:${OUT}`, `disagree-1h:${OUT}`]);
  // they agree again: settled, and forgotten
  const d = judge({ prev: c.next, now: T0 + STILL_MS + 1 });
  assert.deepEqual(d.alerts, []);
  assert.equal(d.next.oneSided[OUT], undefined);
  assert.equal(d.next.judged[OUT], "matched");
});

test("both confirmed at different blocks: SEV2, not a guess", () => {
  const v = views();
  v[1].txs = v[1].txs.map((t) => (t.txid === OUT ? { ...t, status: { ...t.status, block_hash: "00".repeat(32) } } : t));
  const r = judge({ views: v });
  assert.ok(r.alerts.some((a) => a.kind === "provider-disagreement" && a.sev === "SEV2" && /disagree on where/.test(a.text)));
});

test("one provider down is not a disagreement: an unexplained spend is still SEV1, marked one provider only", () => {
  const v = views();
  v[1] = { ...v[1], txs: null, error: "HTTP 503" };
  const ok = judge({ views: v });
  assert.deepEqual(ok.alerts, []);
  assert.equal(ok.next.judged[OUT], undefined, "not settled on one provider's word");
  const bad = judge({ views: v, records: records({ redemptions: "redemptions_none_delivered" }) });
  assert.equal(bad.alerts[0].sev, "SEV1");
  assert.match(bad.alerts[0].text, /blockstream\.info did not answer \(one provider only\)/);
});

test("inbound: the expected test deposit is confirmed once; anything unexplained is INFO; a recorded purchase is silent", () => {
  const expect = [{ id: "custody#0", min_sats: 2000, note: "G10 drill test deposit" }];
  const r = judge({ records: records({ frogs: false }), expect });
  const a = r.alerts.filter((x) => x.kind === "custody-inbound");
  assert.equal(a.length, 1);
  assert.equal(a[0].sev, "INFO");
  assert.match(a[0].text, /✅ EXPECTED INBOUND seen at custody .*: G10 drill test deposit/);
  assert.match(a[0].text, /2,910 sats in b6ceea8e/);
  assert.equal(r.next.expectUsed[IN], "custody#0");
  assert.deepEqual(judge({ records: records({ frogs: false }), expect, prev: r.next }).alerts, [], "judged once: the next poll is silent");
  const big = judge({ records: records({ frogs: false }), expect: [{ id: "custody#0", min_sats: 10_000, note: "x" }] });
  assert.match(big.alerts.find((x) => x.kind === "custody-inbound").text, /unexplained INBOUND/, "below the expected amount it is not the test deposit");
  const none = judge({ records: records({ frogs: false }) });
  assert.equal(none.alerts.find((x) => x.kind === "custody-inbound").key, `inbound:${IN}`);
  assert.equal(judge().alerts.filter((x) => x.kind === "custody-inbound").length, 0);
});

test("fetch: the first page, then older pages until a judged txid, and a clear error on a bad answer", async () => {
  const page = (n, from) => Array.from({ length: n }, (_, i) => ({ txid: `${from}${String(i).padStart(4, "0")}`, status: { confirmed: true } }));
  const asked = [];
  const http = async (url) => {
    asked.push(url.replace("https://x/api", ""));
    const body = url.endsWith("/txs") ? page(25, "a") : url.includes("/txs/chain/a0024") ? page(25, "b") : url.includes("/txs/chain/b0024") ? page(3, "c") : [];
    return { ok: true, json: async () => body };
  };
  const all = await fetchAddressTxs("https://x/api", ADDR, new Set(), http);
  assert.equal(all.txs.length, 53);
  assert.deepEqual(asked, [`/address/${ADDR}/txs`, `/address/${ADDR}/txs/chain/a0024`, `/address/${ADDR}/txs/chain/b0024`]);
  asked.length = 0;
  const some = await fetchAddressTxs("https://x/api", ADDR, new Set(["a0003"]), http);
  assert.equal(some.txs.length, 25, "stops at a page holding a judged txid");
  assert.equal(asked.length, 1);
  await assert.rejects(fetchAddressTxs("https://x/api", ADDR, new Set(), async () => ({ ok: false, status: 429 })), /x answered HTTP 429/);
});

test("the poll reads the explorers FIRST and the canister's records SECOND, and a silent provider becomes an alert after 30 min", async () => {
  const order = [];
  const http = async (url) => { order.push(url.includes("mempool") ? "mempool" : "blockstream");
    if (url.includes("blockstream")) return { ok: false, status: 503 };
    return { ok: true, json: async () => ES("mempool-address-txs.json") }; };
  const readRecords = async () => { order.push("records"); return records(); };
  const cfg = { providers: ["https://mempool.space/api", "https://blockstream.info/api"], addresses: { custody: ADDR } };
  const a = await checkBitcoin({ cfg, prev: {}, now: T0, http, readRecords, timelock: "tl" });
  assert.equal(order.at(-1), "records");
  assert.ok(order.indexOf("records") > order.lastIndexOf("mempool"));
  assert.deepEqual(a.alerts, []);
  assert.equal(a.next.providers["blockstream.info"].downSince, T0);
  const b = await checkBitcoin({ cfg, prev: a.next, now: T0 + DOWN_MS, http, readRecords, timelock: "tl" });
  assert.deepEqual(b.alerts.map((x) => [x.kind, x.sev]), [["watcher-error", "SEV2"]]);
  assert.match(b.alerts[0].text, /blockstream\.info has not answered for 30 min \(blockstream\.info answered HTTP 503\)/);
  await assert.rejects(checkBitcoin({ cfg, prev: {}, now: T0, http: async () => ({ ok: false, status: 500 }), readRecords }), /no Bitcoin provider answered/);
});

test("the records index: every txid the canister vouches for, from each kind of record", () => {
  const i = recordIndex({ ...records(), sale_state: { sales: [
    { id: "1", phase: { Listed: { inscription: "x", outpoint: { txid: "ab".repeat(32), vout: 2 } } } },
    { id: "2", phase: { Moving: { inscription: "y", outpoint: { txid: "cd".repeat(32), vout: 0 }, txid: "ef".repeat(32) } } },
    { id: "3", phase: { Sold: { inscription: "z", fill_txid: "12".repeat(32) } } }] },
    purchases: [["4", `Settled { txid: "${"34".repeat(32)}" }`]] });
  assert.equal(i.byTxid.get(OUT).kind, "delivery");
  assert.equal(i.byTxid.get(IN).kind, "purchase");
  assert.equal(i.byTxid.get("55".repeat(32)).kind, "return");
  assert.equal(i.byTxid.get("ef".repeat(32)).kind, "sale");
  assert.equal(i.byTxid.get("12".repeat(32)).kind, "sale");
  assert.equal(i.byTxid.get("34".repeat(32)).kind, "vault purchase");
  assert.ok(i.saleOutpoints.has(`${"ab".repeat(32)}:2`));
  assert.equal(i.deliveries.get("5").txid, OUT);
  assert.deepEqual(i.floorFrogs, [{ id: "1", module: `0x${"bb".repeat(20)}`, cost: "650000000000000000" }]);
  assert.ok(i.mintOutpoints.has(`${IN}:0`));
});
