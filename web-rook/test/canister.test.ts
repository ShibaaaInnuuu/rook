import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { b58decode } from "../src/rook/sol/base58";
import { decodeBooksView, decodeCoinsLog, decodeDeskStatus, decodeIntentView, decodeOpenKind, decodeOpened, decodePoolDesks, decodeRaffle, decodeSnapshotPage, decodeTreasuryStatus, encodeIntentView, encodeOpened, encodeOpenKind, encodeRaffle, encodeSnapshotPage, findingKindOf, loadCanisterConfig, stateOf, type IntentViewV, type RaffleV } from "../src/rook/sol/canister";
import { intentView } from "./fixtures/intent";
import { raffleRecord, snapshotPage, WINNER, WX_MINT } from "./fixtures/raffle";

/** W12 (docs/v9/canister-only-solana.md §10.1): the site's Candid types decode the canister's OWN reply bytes, the fixtures its tests write
 *  (autobuyer/canister-rs/tests/fixtures/views/*.hex, copied to test/fixtures/views; the copies must not drift from the canister's). */
const here = (p: string) => new URL(p, import.meta.url);
const hex = (name: string) => readFileSync(here(`fixtures/views/${name}`), "utf8").trim();
const CANISTER = here("../../autobuyer/canister-rs/tests/fixtures/views/");

test("the view fixtures are byte-identical copies of the canister's (when the repository is beside the package)", () => {
  if (!existsSync(CANISTER)) return;
  for (const n of ["books-view.hex", "desk-status.hex", "treasury-status.hex", "pool-desks.hex", "coins-log.hex"]) {
    assert.equal(hex(n), readFileSync(new URL(n, CANISTER), "utf8").trim(), `${n} drifted: re-copy it from autobuyer/canister-rs/tests/fixtures/views/`);
  }
});

test("books_view: the fixture (two items held, one ticket open, some SOL-backed units) decodes with every field the pages draw", () => {
  const r = decodeBooksView(hex("books-view.hex"));
  assert.equal(r.length, 1);
  const v = r[0]!;
  assert.equal(v.coll, 0);
  assert.equal(v.wx_mint, "DyRkUpQxYG2VnP2SkMdQs5BTsVPeKCpSHLxzByc2Sxvj");
  assert.deepEqual([v.held, v.open_tickets, v.items.length, v.tickets_open.length], [2n, 1n, 2, 1]);
  assert.equal(v.expected_supply[0], 1_000_000_000n + v.sol_backed);
  assert.equal(v.owed, v.backing + v.reserve + v.surplus + v.revenue);
  assert.ok(v.fresh && v.mint_price.length === 1 && v.mint_price[0]! > 0n);
  assert.ok("EthNft" in v.kind && v.kind.EthNft.chain_id === 1n && v.kind.EthNft.custody.length === 20);
  assert.equal(v.tickets_open[0].stage, "Pinned");
  assert.equal(v.tickets_open[0].destination, "0xe1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1");
  assert.equal(v.items[0].key, "01".repeat(32));
  assert.equal(v.halted.length, 0);
  assert.equal(v.accepted, 200_000_000_000n);
});

test("desk_status: a halted collection, its incident and the canister's own report decode", () => {
  const d = decodeDeskStatus(hex("desk-status.hex"));
  assert.deepEqual(d.halted, [[0, "pre_sign refused: SupplyMismatch"]]);
  assert.equal(d.incidents.length, 1);
  assert.equal(d.incidents[0].coll[0], 0);
  assert.equal(d.open, 1);
  assert.equal(d.coin_redeem, false);
  assert.equal(d.paused, false);
  const r = d.report[0]!;
  assert.equal(r.checked, 1);
  assert.equal(r.findings.length, 0);
  assert.equal(Array.from(r.halted).length, 0);
  assert.equal(d.last_reconcile, 1_700_000_000n);
});

test("treasury_status, pool_desks and coins_log decode (record subtyping keeps only the fields the site reads)", () => {
  const t = decodeTreasuryStatus(hex("treasury-status.hex"));
  assert.equal(t.off, false);
  assert.equal(t.treasury.reserves_bps, 9_000);
  assert.equal(t.treasury.total_received, 10_000_000_000n);
  assert.deepEqual(Array.from(t.treasury.collections), [0]);
  assert.equal(t.owed.length, 2);
  assert.ok("Ops" in t.owed[0] && t.owed[0].Ops.lamports === 1_000_000_000n);
  assert.ok("Surplus" in t.owed[1] && t.owed[1].Surplus.coll === 0);
  assert.equal(t.last[0][0], "allocate");
  const p = decodePoolDesks(hex("pool-desks.hex"));
  assert.equal(p.length, 1);
  assert.equal(p[0].pair, "EwhvTBi8sDnoJjTHneujyBBjxmQFQHw23HvENB5A6oFs");
  assert.deepEqual([p[0].lo, p[0].active, p[0].hi], [567, 586, 605]);
  assert.equal(p[0].arb_profit, 1_234_567n);
  const [fees, waits, last, queue] = decodeCoinsLog(hex("coins-log.hex"));
  assert.equal(fees[0][0], 0);
  assert.equal(fees[0][1].to_desk_lamports, 7n);
  assert.equal(waits[0].key, "open_config:1");
  assert.equal(last[0][0], "open_mint");
  assert.deepEqual(queue, [[2, 1_700_000_600n]], "the opening queue (coins_request_open, oldest first)");
});

test("intent_status and open_intent: the site's types round-trip through Candid (every State tag, every OpenKind)", () => {
  for (const s of ["Open", "Funded", "Executing", "Done", "Refunded", "Expired"] as const) {
    const v = intentView({ state: { [s]: null } as IntentViewV["state"], txs: [{ at: 1n, op: "Mint", result: "landed", signature: "sig", link: 3n, memo: "rook:v1:0:mint:3" }] });
    const back = decodeIntentView(Buffer.from(encodeIntentView(v)).toString("hex"))[0]!;
    assert.equal(stateOf(back), s);
    assert.equal(back.txs[0].memo, "rook:v1:0:mint:3");
  }
  const o = decodeOpened(Buffer.from(encodeOpened({ Ok: { id: 9n, fee: 1_500_000n, min_deposit: 50_000_000n, token_account: [], quote: [{ Mint: { fee: 1_500_000n, min_deposit: 50_000_000n, coll: 0, ata_rent: 2_039_280n, deposit: 50_000_000n, to_module: 46_460_720n, unit_price: 200_320_000_000n, units: 231_930n } }], address: "7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE", expires_at: 1n } })).toString("hex"));
  assert.ok("Ok" in o && o.Ok.id === 9n && "Mint" in o.Ok.quote[0]!);
  const err = decodeOpened(Buffer.from(encodeOpened({ Err: "the desk is paused" })).toString("hex"));
  assert.deepEqual(err, { Err: "the desk is paused" });
  for (const k of [
    { Mint: { coll: 1, recipient: "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy", min_out: [5n] as [bigint] } },
    { Burn: { coll: 1, recipient: "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy", refund_to: [] as [], min_out: [] as [] } },
    { RedeemNft: { coll: 1, destination: "burn", owner: "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy" } },
    { RedeemCoin: { coll: 1, coin: "7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE", recipient: "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy", refund_to: [] as [] } },
  ]) assert.deepEqual(decodeOpenKind(encodeOpenKind(k)), k);
});

// the canister's tests write no raffle fixture (tests/fixtures/views/ has none): the reply bytes are built here with the site's IDL, so the
// test holds the site's types against themselves and the did's field names and kinds, not against the canister's own bytes
test("raffle_status and raffle_snapshot: the site's types round-trip through Candid (every RafflePhase, a page of a published snapshot)", () => {
  const toHex = (b: Uint8Array) => Buffer.from(b).toString("hex");
  for (const phase of [{ Idle: null }, { Committed: null }, { Drawn: null }] as RaffleV["phase"][]) {
    const v = raffleRecord({ phase, winner: b58decode(WINNER), drawn_at: -1n });
    const back = decodeRaffle(toHex(encodeRaffle(v)))[0]!;
    assert.deepEqual(Object.keys(back.phase), Object.keys(phase));
    assert.deepEqual(Array.from(back.key), Array.from(b58decode(WX_MINT)));
    assert.deepEqual(Array.from(back.winner), Array.from(b58decode(WINNER)));
    assert.deepEqual(Array.from(back.root), Array.from(v.root));
    assert.deepEqual([back.pot, back.prize, back.round, back.total_weight, back.entries, back.snapshot_slot], [1_500_000_000n, 0n, 3n, 777n, 42, 370_000_000n]);
    assert.deepEqual([back.committed_at, back.drand_round, back.ticket, back.drawn_at, back.claimed_at], [1_791_000_000n, 20_900_000n, 555n, -1n, 1_791_100_000n], "int64 times, signed");
    assert.deepEqual([back.total_in, back.total_spent, back.prizes_funded, back.prizes_claimed, back.rounds_expired, back.rounds_cancelled], [10n, 20n, 30n, 40n, 50n, 60n]);
  }
  assert.equal(decodeRaffle(toHex(encodeRaffle(null))).length, 0, "no raffle on the books: opt none");
  const page = snapshotPage();
  const back = decodeSnapshotPage(toHex(encodeSnapshotPage(page)))[0]!;
  assert.deepEqual(back, page);
  assert.equal(back.listing, WX_MINT);
  assert.equal(back.leaves.reduce((a, l) => a + l.weight, 0n), back.total_weight);
  assert.equal(decodeSnapshotPage(toHex(encodeSnapshotPage(null))).length, 0, "a round the canister no longer keeps: opt none");
});

test("a finding's kind is its one tag; the canister config refuses a malformed id and keeps the IC host by default", () => {
  assert.equal(findingKindOf({ coll: 0, chain: 1n, books: 2n, note: "", kind: { HolderBurn: null } }), "HolderBurn");
  assert.deepEqual(loadCanisterConfig({ canisterId: "", icHost: "https://icp-api.io" }), { canisterId: null, icHost: "https://icp-api.io", problems: [] });
  const ok = loadCanisterConfig({ canisterId: "rrkah-fqaaa-aaaaa-aaaaq-cai" });
  assert.equal(ok.canisterId, "rrkah-fqaaa-aaaaa-aaaaq-cai");
  const bad = loadCanisterConfig({ canisterId: "not-an-id" });
  assert.equal(bad.canisterId, null);
  assert.match(bad.problems[0], /not a canister id/);
  assert.equal(loadCanisterConfig({ canisterId: "" }, "rrkah-fqaaa-aaaaa-aaaaq-cai", "http://127.0.0.1:4943").icHost, "http://127.0.0.1:4943");
  assert.match(loadCanisterConfig({ canisterId: "" }, null, "http://evil.example").problems[0], /must be https/);
});

// the canister pins collections in its unlock order (autobuyer/canister-rs/src/coins/unlock.rs ORDER: BAYC first), which is not the
// site's listing order (nft-pilot.json: Milady 0): a page must find the canister's index by the wX symbol, never pass its listingId
test("the canister's collection index is found by wX symbol in coins_collections, never the site's listing id", async () => {
  const { coinsCollectionIndex } = await import("../src/rook/sol/hooksCanister");
  const { COLLECTIONS } = await import("../src/rook/sol/collections");
  const cols = [
    { index: 0, symbol: "wBOREDAPE", stage: "Open", wx_mint: "", config: "", module: "", coins: 0, backed: 0 },
    { index: 1, symbol: "wMILADY", stage: "Open", wx_mint: "", config: "", module: "", coins: 0, backed: 0 },
  ];
  const milady = COLLECTIONS.find((c) => c.slug === "milady")!, bayc = COLLECTIONS.find((c) => c.slug === "bayc")!;
  assert.equal(coinsCollectionIndex(cols, milady.wxSymbol), 1);
  assert.equal(coinsCollectionIndex(cols, bayc.wxSymbol), 0);
  assert.notEqual(bayc.listingId, 0, "the site's listing id is another order");
  assert.equal(coinsCollectionIndex(cols, "wPUDGY"), null, "not pinned yet: no index");
  assert.equal(coinsCollectionIndex(undefined, "wMILADY"), null, "nothing read yet");
});
