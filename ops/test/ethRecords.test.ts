import assert from "node:assert/strict";
import { request } from "node:http";
import { test } from "node:test";
import { RecordsCache, recordsFor } from "../src/ethRecords.js";
import type { EthStateV } from "../src/ic.js";
import { serve, served } from "../src/reconcile.js";

/** docs/v8/proof-of-reserves.md P4: the canister's own records, per collection, as the reserves page labels them. */
const b = (h: string) => Array.from(Buffer.from(h.replace(/^0x/, ""), "hex"));
const MIL = "0x5af0d9827e0c53e4799bb226655a1de152a425a5", AZUKI = "0xed5af388653567af2f388e6224dc7c4b3241c544";
const tok = (n: number) => b(n.toString(16).padStart(64, "0"));
const state: EthStateV = {
  buys: [
    { id: 7n, collection: b(MIL), module: b("11".repeat(20)), wrapper: b("22".repeat(20)), advanced: 970n, attempts: 1, ask: [], note: "settled",
      phase: { Settled: { token: tok(9180), total: 940n, tx: b("ab".repeat(32)), cost: 950n } } },
    { id: 8n, collection: b(MIL), module: b("11".repeat(20)), wrapper: b("22".repeat(20)), advanced: 970n, attempts: 1, ask: [], note: "arrived",
      phase: { Arrived: { wei: 960n, since: 1n } } },
    { id: 9n, collection: b(MIL), module: b("11".repeat(20)), wrapper: b("22".repeat(20)), advanced: 970n, attempts: 1, ask: [], note: "bridging",
      phase: { Bridging: { order_id: b("cd".repeat(32)), value: 970n, min_out: 950n, deadline: 9n, since: 2n } } },
    { id: 10n, collection: b(AZUKI), module: b("33".repeat(20)), wrapper: b("44".repeat(20)), advanced: 5n, attempts: 1, ask: [], note: "", phase: { NeedAdvance: null } },
  ],
  deliveries: [{ wrapper: b("22".repeat(20)), collection: b(MIL), ticket: 3n, token: tok(42), destination: b("55".repeat(20)), deliver_by: 1000n, phase: { Sent: { tx: b("ef".repeat(32)), nonce: 4n } }, note: "" }],
  readings: [], eth_out: 0n,
  deposits: [{ tx: b("99".repeat(32)), collection: b(MIL), token: tok(77) }, { tx: b("98".repeat(32)), collection: b(AZUKI), token: tok(1) }],
};

test("records: one collection's buys, deliveries and deposits, each phase named, the ETH where it is", () => {
  const r = recordsFor(state, null, MIL, 100);
  assert.equal(r.how, "query"); assert.equal(r.readAt, 100);
  assert.deepEqual(r.buys.map((x) => [x.id, x.phase, x.advancedWei, x.tokenId]), [["7", "Settled", "970", "9180"], ["8", "Arrived", "960", null], ["9", "Bridging", "970", null]]);
  assert.equal(r.buys[0].tx, `0x${"ab".repeat(32)}`); assert.equal(r.buys[2].orderId, `0x${"cd".repeat(32)}`);
  assert.deepEqual(r.deliveries, [{ ticket: 3, tokenId: "42", phase: "Sent", deliverBy: 1000, tx: `0x${"ef".repeat(32)}` }]);
  assert.deepEqual(r.deposits, [{ tx: `0x${"99".repeat(32)}`, tokenId: "77" }]);
  assert.equal(r.leftoverWei, null, "no per-collection leftover in the state: not guessed");
});
test("records: the caps are the collection's own headroom", () => {
  const caps = [{ collections: [{ collection: MIL, max_items: 16, max_value_wei: 15n, max_item_wei: 15n }], global_max_items: 75, global_max_value_wei: 75n },
    [{ collection: MIL, items_held: 13, items_in_flight: 1, value_wei: 12n, items_left: 3, value_left_wei: 3n }]] as Parameters<typeof recordsFor>[1];
  assert.deepEqual(recordsFor(state, caps, MIL, 1).caps, { maxItems: 16, maxValueWei: "15", itemsHeld: 13, itemsInFlight: 1, valueWei: "12" });
  assert.equal(recordsFor(state, caps, AZUKI, 1).caps, null);
});
test("records: cached, and served by the reconciler at /v1/eth-records", async () => {
  let calls = 0;
  const cache = new RecordsCache({ eth_nft_state: async () => { calls++; return state; }, custody_caps: async () => { throw new Error("no caps"); } }, [{ slug: "milady", collection: MIL }], 60, () => 1_000);
  await cache.get(); const v = await cache.get();
  assert.equal(calls, 1); assert.equal(v.milady.buys.length, 3);
  const srv = serve(() => served(null, 1_000, 10_800, 4663), 0, () => cache.get());
  await new Promise((ok) => srv.once("listening", ok));
  const port = (srv.address() as { port: number }).port;
  const body = await new Promise<string>((ok, no) => request({ host: "127.0.0.1", port, path: "/v1/eth-records" }, (res) => { let s = ""; res.on("data", (c) => (s += c)); res.on("end", () => ok(s)); }).on("error", no).end());
  srv.close();
  assert.equal(JSON.parse(body).milady.buys[0].tokenId, "9180");
});
