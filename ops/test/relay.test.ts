import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { askOf, convertOnce, encodeAddress, orderIdOf, preJudge, quoteBody, type Ask } from "../src/relay.js";
import { Runner } from "../src/runner.js";
import { logger } from "../src/log.js";
import type { ConvRecV } from "../src/ic.js";

// docs/v6/relay-conversion.md §3.1: the runner's pre-judge, on Relay's real quotes (fixtures shared with the canister's tests).
const FX = new URL("../../autobuyer/canister-rs/tests/fixtures/", import.meta.url);
const raw = (f: string) => readFileSync(new URL(f, FX), "utf8");
const Q26 = raw("relay-quote-4663-btc-2026-09-26.json");
const NOW = 1_790_441_670;                 // when the 2026-09-26 quote was made (its deadline is 7 days later)
const USER = "0xb02e4ec02ddccb6f68cfa47b84eeadebc6386ab2";
const FLOAT = "bc1p7ynpknzp0kene8twkvyarncd4740x37955hxu3d8xsczg9vn3a8q4v86hp";
const ASK: Ask = { frog: 7n, amountWei: 160_000_000_000_000_000n, user: USER, recipient: FLOAT, refundTo: USER, originChainId: 4663, destinationChainId: 8253038,
  originCurrency: "0x0000000000000000000000000000000000000000", destinationCurrency: "bc1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqmql8k8" };
const edit = (f: (q: any) => void): any => { const q = JSON.parse(Q26); f(q); return q; }; // eslint-disable-line @typescript-eslint/no-explicit-any

test("the order id recomputes from orderData for every captured quote, and equals the calldata's id", () => {
  for (const f of ["relay-quote-4663-btc-2026-09-26.json", "relay-quote-4663-btc-2026-09-23.json", "seaport/relay-quote-4663-8453-2026-09-23.json"]) {
    const q = JSON.parse(raw(f));
    const orderData = q.protocol.v2.orderData;
    if (orderData.output.chainId === "base") continue;         // an EVM destination the runner never asks for: its chain is refused by design
    assert.equal(orderIdOf(orderData), q.protocol.v2.orderId, f);
    assert.equal(`0x${q.steps[0].items[0].data.data.slice(-64)}`, q.protocol.v2.orderId, f);
  }
  assert.throws(() => orderIdOf(JSON.parse(raw("seaport/relay-quote-4663-8453-2026-09-23.json")).protocol.v2.orderData), /only robinhood, ethereum and bitcoin/);
});

test("address encoding as Relay's SDK: bech32 v0, bech32m v1, Base58 with the 0xff discriminator, EVM bytes", () => {
  assert.equal(encodeAddress("bc1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqmql8k8", "bitcoin"), `0x${"00".repeat(20)}`);           // as the SDK encodes it: version 0 and 19 zero bytes
  assert.match(encodeAddress(FLOAT, "bitcoin"), /^0x01[0-9a-f]{64}$/);
  assert.equal(encodeAddress("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2", "bitcoin"), "0xff0077bff20c60e522dfaa3350c39b030a5d004e839a");
  assert.equal(encodeAddress(USER.toUpperCase().replace("0X", "0x"), "robinhood"), USER);
  assert.throws(() => encodeAddress(USER, "base"), /only robinhood, ethereum and bitcoin/);
});

test("the real quote passes the pre-judge; the body asks for exactly the ask", () => {
  const j = preJudge(JSON.parse(Q26), ASK, NOW);
  assert.equal(j.orderId, "0xfef9de1e3046989ca671815e8444edb2fa9fa472b05c09c6014f6a6505ea5fdf");
  assert.equal(j.minSats, 495_542n);
  assert.equal(j.deadline, 1_791_046_471);
  assert.deepEqual(quoteBody(ASK), { user: USER, originChainId: 4663, destinationChainId: 8253038, originCurrency: ASK.originCurrency,
    destinationCurrency: ASK.destinationCurrency, recipient: FLOAT, tradeType: "EXACT_INPUT", amount: "160000000000000000", refundTo: USER });
});

test("every checkable lie is refused, naming it", () => {
  const OTHER_BTC = "bc1pw508d6qejxtdg4y5r3zarvary0c5xw7kw508d6qejxtdg4y5r3zarvary0c5xw7kt5nd6y";
  const OTHER_EVM = "0x000000000000000000000000000000000000beef";
  const d = (q: any) => q.steps[0].items[0].data; // eslint-disable-line @typescript-eslint/no-explicit-any
  const o = (q: any) => q.protocol.v2.orderData; // eslint-disable-line @typescript-eslint/no-explicit-any
  const cases: [any, RegExp][] = [ // eslint-disable-line @typescript-eslint/no-explicit-any
    // a recipient changed in the order no longer hashes to the id; changed in the id too, it is not ours
    [edit((q) => { o(q).output.payments[0].recipient = OTHER_BTC; }), /hashes to/],
    [edit((q) => { o(q).output.payments[0].recipient = OTHER_BTC; const id = orderIdOf(o(q)); q.protocol.v2.orderId = id; d(q).data = d(q).data.slice(0, 74) + id.slice(2); }), /not our float/],
    [edit((q) => { o(q).inputs[0].refunds[0].recipient = OTHER_EVM; const id = orderIdOf(o(q)); q.protocol.v2.orderId = id; d(q).data = d(q).data.slice(0, 74) + id.slice(2); }), /refund on robinhood/],
    [edit((q) => { o(q).inputs[0].refunds[1].recipient = OTHER_BTC; const id = orderIdOf(o(q)); q.protocol.v2.orderId = id; d(q).data = d(q).data.slice(0, 74) + id.slice(2); }), /refund on bitcoin/],
    [edit((q) => { d(q).chainId = 1; }), /another chain/],
    [edit((q) => { o(q).output.payments[0].currency = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"; const id = orderIdOf(o(q)); q.protocol.v2.orderId = id; d(q).data = d(q).data.slice(0, 74) + id.slice(2); }), /not native BTC/],
    [edit((q) => { d(q).to = OTHER_EVM; }), /pinned depository/],
    [edit((q) => { q.protocol.v2.paymentDetails.depository = OTHER_EVM; }), /another depository/],
    [edit((q) => { d(q).value = "200000000000000000"; }), /sends 200000000000000000/],
    [edit((q) => { d(q).data = d(q).data.replace(USER.slice(2), OTHER_EVM.slice(2)); }), /another depositor/],
    [edit((q) => { d(q).from = OTHER_EVM; }), /another sender/],
    [edit((q) => { o(q).output.payments[0].minimumAmount = "1"; }), /hashes to/],
    [edit((q) => { o(q).output.calls = ["0x00"]; const id = orderIdOf(o(q)); q.protocol.v2.orderId = id; d(q).data = d(q).data.slice(0, 74) + id.slice(2); }), /makes calls/],
    [edit((q) => { q.steps.push(q.steps[0]); }), /2 steps/],
    [edit((q) => { delete q.protocol; }), /no protocol.v2.orderData/],
    [edit((q) => { o(q).output.chainId = "base"; }), /cannot be hashed/],
  ];
  for (const [q, why] of cases) assert.throws(() => preJudge(q, ASK, NOW), why);
  // the deadline band: a quote made nine days ago, or one whose deadline is under a day away
  assert.throws(() => preJudge(JSON.parse(Q26), ASK, NOW + 7 * 86_400), /deadline/);
  assert.throws(() => preJudge(JSON.parse(Q26), ASK, NOW - 2 * 86_400), /deadline/);
  // an ask for another amount or another float
  assert.throws(() => preJudge(JSON.parse(Q26), { ...ASK, amountWei: 1n }, NOW), /not 1/);
  assert.throws(() => preJudge(JSON.parse(Q26), { ...ASK, recipient: OTHER_BTC }, NOW), /refund on bitcoin|not our float/);
});

const conv = (ask: boolean): ConvRecV => ({ frog: 7n, module: new Uint8Array(20), advanced: 160_200_000_000_000_000n, attempts: 1,
  phase: ask ? { Funded: { wei: 160_200_000_000_000_000n, since: BigInt(NOW) } } : { Converting: {} }, orders: [], deposits: [], arrival: [], note: "",
  ask: ask ? [{ amount_wei: ASK.amountWei, user: USER, recipient: FLOAT, refund_to: USER, origin_chain_id: 4663n, destination_chain_id: 8253038n,
    origin_currency: ASK.originCurrency, destination_currency: ASK.destinationCurrency }] : [] });

test("convertOnce: asks Relay for exactly the ask, hands in the raw answer, and only for a conversion that asks", async () => {
  assert.deepEqual(askOf(conv(true)), ASK);
  assert.equal(askOf(conv(false)), null);
  const calls: { url: string; body: unknown }[] = [];
  const handed: [bigint, string][] = [];
  const fetchStub = (async (url: string, init: { body: string }) => { calls.push({ url, body: JSON.parse(init.body) }); return new Response(Q26, { status: 200 }); }) as unknown as typeof fetch;
  const buyer = { frog_conversions: async () => [conv(false), conv(true)], relay_quote: async (f: bigint, t: string) => { handed.push([f, t]); return { Ok: "stored" }; } };
  assert.equal(await convertOnce({ buyer, fetch: fetchStub, log: logger(() => {}), nowS: () => NOW }), true);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "https://api.relay.link/quote");
  assert.deepEqual(calls[0].body, quoteBody(ASK));
  assert.deepEqual(handed, [[7n, Q26]]);
  // a lie is not handed in; an HTTP error is not either
  const lie = JSON.stringify(edit((q) => { q.steps[0].items[0].data.to = "0x000000000000000000000000000000000000beef"; }));
  const lines: string[] = [];
  const f2 = (async () => new Response(lie, { status: 200 })) as unknown as typeof fetch;
  handed.length = 0;
  assert.equal(await convertOnce({ buyer, fetch: f2, log: logger((l) => lines.push(l)), nowS: () => NOW }), false);
  assert.equal(handed.length, 0);
  assert.ok(lines.some((l) => l.includes("relay-quote-rejected") && l.includes("pinned depository")));
  const f3 = (async () => new Response('{"message":"blocked"}', { status: 400 })) as unknown as typeof fetch;
  assert.equal(await convertOnce({ buyer, fetch: f3, log: logger(() => {}), nowS: () => NOW }), false);
});

test("the runner: a quote handed in is judged by a second tick at once; none handed in, one tick", async () => {
  let ticks = 0;
  const b = { tick: async () => { ticks++; return { Ok: [`tick ${ticks}`] }; }, paused: async () => false };
  let hand = true;
  const r = new Runner(b, logger(() => {}), "p", "ryjl3-tyaaa-aaaaa-aaaba-cai", Date.now, async () => hand);
  await r.cycle();
  assert.equal(ticks, 2);
  hand = false;
  await r.cycle();
  assert.equal(ticks, 3);
  const plain = new Runner(b, logger(() => {}), "p", "ryjl3-tyaaa-aaaaa-aaaba-cai");
  await plain.cycle();
  assert.equal(ticks, 4);
  const failing = new Runner(b, logger(() => {}), "p", "ryjl3-tyaaa-aaaaa-aaaba-cai", Date.now, async () => { throw new Error("relay down"); });
  assert.equal((await failing.cycle())?.ok, true);
  assert.equal(ticks, 5);
});
