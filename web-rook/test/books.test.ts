import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import type { HandinReport, WrappedCollection, WrappedMarket } from "../src/api/types.js";
import { marketVenues, wrappedFixtures } from "../src/lib/wrappedSource.js";
import { quoteCoin } from "../src/lib/quote.js";
import { POLICIES, POLICY_INDEX, launchExtraNow } from "../src/rook/tiers.js";

/**
 * The indexer's reports as the pages read them through the copied types: a YOYO coin's loop, a coin's decaying launch fee on the
 * clock, the offer board (docs/v6/offer-handins.md), and the ticket quoting the fixture's Floor Module through its own spreads.
 */
const here = (p: string) => new URL(p, import.meta.url);
const read = async (f: string) => JSON.parse(readFileSync(here(`../public/fixtures/wrapped/${f}`), "utf8"));
const index = JSON.parse(readFileSync(here("../public/fixtures/wrapped/index.json"), "utf8")) as { generatedAt: number };
const E = 10n ** 18n;

test("the YOYO coin: open listings, proceeds waiting, bought back and burnt; every policy the indexer names has a line and a vault index", async () => {
  const src = await wrappedFixtures(index.generatedAt, read);
  const frogs = await src.get<WrappedCollection>("/v1/wrapped/frogs");
  const gamma = frogs.coinsList.find((c) => c.policy === "yoyo")!;
  assert.ok(gamma && gamma.yoyo, "the fixture has a YOYO coin with its loop");
  assert.equal(gamma.yoyo.openListings, gamma.yoyo.listings.filter((l) => l.stage === "listed").length);
  assert.ok(gamma.yoyo.listings.some((l) => l.stage === "sold" && l.proceedsWei !== null));
  assert.ok(BigInt(gamma.yoyo.boughtBackWei) > 0n && BigInt(gamma.yoyo.burnedCoins) > 0n);
  assert.ok(frogs.activity.some((a) => a.kind === "relisted") && frogs.activity.some((a) => a.kind === "bought-back"), "the new activity kinds are in the nest's tape");
  assert.ok(frogs.contracts.yoyoVault && frogs.contracts.offerBoard && frogs.contracts.treasury, "the contracts the Info tab links");
  for (const p of POLICIES) assert.equal(typeof POLICY_INDEX[p.id], "number");
  assert.deepEqual(POLICIES.map((p) => p.id), ["raffle", "burn", "yoyo"]);
});

test("the launch fee runs on the clock: full at launch, half at half, nothing at the end; a fixture's end is moved to now", async () => {
  const raw = (await read("wrapped-frogs.json")) as WrappedCollection;
  const src = await wrappedFixtures(index.generatedAt + 1_000, read);
  const frogs = await src.get<WrappedCollection>("/v1/wrapped/frogs");
  const gamma = frogs.coinsList.find((c) => c.policy === "yoyo")!, lf = gamma.launchFee!;
  assert.equal(lf.endsAt, raw.coinsList.find((c) => c.policy === "yoyo")!.launchFee!.endsAt! + 1_000, "endsAt is a time key: shifted with the set");
  assert.equal(launchExtraNow(lf, lf.endsAt! - lf.decayS), lf.bps);
  assert.equal(launchExtraNow(lf, lf.endsAt! - lf.decayS / 2), lf.bps / 2);
  assert.equal(launchExtraNow(lf, lf.endsAt!), 0); assert.equal(launchExtraNow(lf, lf.endsAt! + 5), 0);
  assert.equal(launchExtraNow(null, 0), 0, "no launch fee, nothing extra");
  assert.equal(launchExtraNow({ bps: 5_000, decayS: 600, extraBpsNow: 1_234, endsAt: null }, 0), 1_234, "with no end known, the read's own figure");
});

test("the ticket quotes the fixture's module through its own spreads and room, and the curve's launch fee is a fee line", async () => {
  const src = await wrappedFixtures(index.generatedAt, read);
  const frogs = await src.get<WrappedCollection>("/v1/wrapped/frogs");
  const m: WrappedMarket = frogs.market!;
  assert.ok(m.floorModule && m.floorModule.spreadMintBps > m.floorModule.spreadBurnBps, "the mint side carries the stress term");
  const alien = frogs.coinsList.find((c) => c.symbol === "ALIEN")!;
  const q = await quoteCoin("buy", E / 10n, marketVenues(m, alien.address, alien.phase, alien.sweepOnly), 100);
  assert.ok(q.ok);
  const mod = q.quote.route.id === "floorModule" ? q.quote.route : q.quote.alternatives.find((r) => r.id === "floorModule")!;
  assert.ok(mod && mod.executable, "the module is a routable venue");
  const mint = mod.legs.find((l) => l.venue === "floorMint")!;
  assert.ok(Math.abs(mint.spreadBps - (m.floorModule.spreadMintBps / (10_000 + m.floorModule.spreadMintBps)) * 10_000) < 1e-9, "the module's own mint spread, as a share of what goes in");
  const over = await quoteCoin("buy", BigInt(m.floorModule.mintRoomWei) + E, marketVenues(m, alien.address, alien.phase, alien.sweepOnly), 100);
  assert.ok(over.ok && over.quote.route.id !== "floorModule" && !over.quote.alternatives.some((r) => r.id === "floorModule"), "over the room the module is not a venue at all");
  const beta = frogs.coinsList.find((c) => c.symbol === "BETA")!;
  const withFee = marketVenues({ ...m, coins: { ...m.coins, [beta.address]: { ...m.coins[beta.address], curve: { ...m.coins[beta.address].curve!, launchExtraBps: 2_500 } } } }, beta.address, "trading", false);
  const b = await quoteCoin("buy", E / 10n, withFee, 100);
  assert.ok(b.ok && b.quote.route.id === "curve");
  assert.ok(b.quote.feeLines.some((f) => /Launch fee/.test(f.label) && f.bps === 2_500), "the decaying launch fee is its own line in Details");
});

test("the offer board: a Bitcoin nest's report has terms, open and settled hand-ins; an NFT nest says it has no board", async () => {
  const src = await wrappedFixtures(index.generatedAt, read);
  const h = await src.get<HandinReport>("/v1/handins/frogs");
  assert.ok(h.board && h.params && h.custodian);
  assert.equal(h.totals.open, h.open.length); assert.equal(h.totals.claimed + h.totals.slashed + h.totals.refunded, h.settled.length);
  assert.ok(h.open.every((x) => x.status === "open") && h.settled.every((x) => x.status !== "open"));
  assert.ok(h.settled.some((x) => x.status === "claimed" && x.rewardPaidWei !== null));
  const apes = await src.get<HandinReport>("/v1/handins/apes");
  assert.equal(apes.board, null); assert.match(apes.error ?? "", /no offer board/);
});
