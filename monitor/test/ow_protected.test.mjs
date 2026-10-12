// docs/v8/ow-protected.md §3.3: a protected Ordinals Wallet buy spends the float twice when it needs a setup (the float's coins split,
// then the sale). The canary pages every float spend the canister did not record as an emergency, so both must be vouched for.
import assert from "node:assert/strict";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { buyerIdl, plain } from "../src/candid.mjs";
import { recordIndex, EMPTY_RECORDS } from "../src/records.mjs";

const SALE = "ab".repeat(32), SETUP = "cd".repeat(32);
const bought = (setup) => ({ inscription: `${"11".repeat(32)}i0`, outpoint: { txid: SALE, vout: 2 }, cost_wei: "1", setup_txid: setup ? [SETUP] : [] });

test("a protected buy's sale and its setup are both recorded float spends; a classic buy vouches only its own txid", () => {
  for (const phase of [{ Handed: { buy: bought(true), sale_txid: SALE, at: 1n } }, { Sent: { buy: bought(true) } }, { Held: { buy: bought(true) } }]) {
    const idx = recordIndex({ ...EMPTY_RECORDS, frog_buys: [{ id: 3n, target: [], phase }] });
    assert.equal(idx.byTxid.get(SALE)?.kind, "purchase", Object.keys(phase)[0]);
    assert.match(idx.byTxid.get(SETUP)?.what ?? "", /setup of frog purchase #3/, Object.keys(phase)[0]);
  }
  const classic = recordIndex({ ...EMPTY_RECORDS, frog_buys: [{ id: 4n, target: [], phase: { Sent: { buy: bought(false) } } }] });
  assert.equal(classic.byTxid.get(SETUP), undefined);
});

test("the monitor's Bought decodes with and without the setup txid (an older canister's answer has none)", () => {
  const svc = buyerIdl({ IDL });
  const frogBuys = svc._fields.find(([n]) => n === "frog_buys")[1];
  const [vecT] = frogBuys.retTypes;
  const Old = IDL.Vec(IDL.Record({ id: IDL.Nat64, target: IDL.Opt(IDL.Variant({ Sink: IDL.Vec(IDL.Nat8), Desk: IDL.Nat64 })),
    phase: IDL.Variant({ Sent: IDL.Record({ buy: IDL.Record({ inscription: IDL.Text, outpoint: IDL.Record({ txid: IDL.Vec(IDL.Nat8), vout: IDL.Nat32 }), cost_wei: IDL.Nat }) }) }) }));
  const old = IDL.encode([Old], [[{ id: 1n, target: [], phase: { Sent: { buy: { inscription: "x", outpoint: { txid: new Uint8Array(32), vout: 0 }, cost_wei: 1n } } } }]]);
  const [decoded] = IDL.decode([vecT], old);
  assert.deepEqual(plain(decoded)[0].phase.Sent.buy.setup_txid ?? [], []);
});

test("review P-09: the setup stays vouched after its frog ends Failed (remembered from the poll that saw it Handed)", async () => {
  const { readFileSync } = await import("node:fs");
  const { judgeAddress } = await import("../src/canary.mjs");
  const ES = (f) => JSON.parse(readFileSync(new URL(`../fixtures/esplora/${f}`, import.meta.url), "utf8"));
  const ADDR = "bc1pfssfalzdt9h8uwqcrs73780l723mh2axrul7keaevke7a8pg4a7szrd0ht";
  const OUT = "2127bc9003b23d8dd342a60cfe9cb44383288c0581a49aac35e513dc8e0fce57";   // stands in for the setup: a float spend
  const views = (txs) => [{ name: "mempool.space", base: "m", txs: txs ?? ES("mempool-address-txs.json") }, { name: "blockstream.info", base: "b", txs: txs ?? ES("blockstream-address-txs.json") }];
  const setupBuy = { ...bought(true), setup_txid: [OUT] };
  const handed = recordIndex({ ...EMPTY_RECORDS, frog_buys: [{ id: 3n, target: [], phase: { Handed: { buy: setupBuy, sale_txid: SALE, at: 1n } } }] });
  const failed = recordIndex({ ...EMPTY_RECORDS, frog_buys: [{ id: 3n, target: [], phase: { Failed: { why: "abandoned" } } }] });
  const T0 = 1_790_000_000_000;
  const p1 = judgeAddress({ label: "float", address: ADDR, views: views([]), index: handed, now: T0 });
  const p2 = judgeAddress({ label: "float", address: ADDR, views: views(), index: failed, prev: p1.next, now: T0 + 7 * 3_600_000 });
  assert.ok(!p2.alerts.some((a) => a.key === `outbound:${OUT}`), JSON.stringify(p2.alerts.map((a) => a.key)));
  assert.equal(p2.next.judged[OUT], "matched");
  // without the earlier poll, the same spend is unexplained: SEV1
  const cold = judgeAddress({ label: "float", address: ADDR, views: views(), index: failed, now: T0 });
  assert.ok(cold.alerts.some((a) => a.key === `outbound:${OUT}` && a.sev === "SEV1"));
  // forgotten after 30 days
  const late = judgeAddress({ label: "float", address: ADDR, views: views([]), index: failed, prev: p1.next, now: T0 + 31 * 86_400_000 });
  assert.equal(late.next.remembered[OUT], undefined);
});
