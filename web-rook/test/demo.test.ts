import assert from "node:assert/strict";
import { test } from "node:test";
import { isAddress, NATIVE_MINT } from "../src/rook/sol/keys";
import { deltaQuote, poolStats } from "../src/rook/sol/dbc";
import { COLLECTIONS } from "../src/rook/sol/collections";
import { SOL } from "../src/rook/sol/siteConfig";
import * as Demo from "../src/rook/sol/demo";
import { CONFIG_REFS, SITE } from "../src/rook/sol/hooks";
import { thresholdOf, thresholdWxOf } from "../src/rook/sol/graduation";

// ---- off by default --------------------------------------------------------------------------------------------------------------

test("demo is asked for only by ?demo (or ?demo=1), never by default; ?demo=0 / false / off are off", () => {
  for (const s of ["?demo", "?demo=", "?demo=1", "?x=2&demo", "?demo=true"]) assert.equal(Demo.isDemoSearch(s), true, s);
  for (const s of ["", "?", "?demox", "?demo=0", "?demo=false", "?demo=off", "?net=test", "?ademo=1"]) assert.equal(Demo.isDemoSearch(s), false, s);
});

test("DEMO off (no ?demo): the hooks see the real config and no fixture at all", () => {
  assert.equal(Demo.DEMO, false);
  assert.equal(SITE, SOL, "the pages read src/config/solana.ts's config, not the demo's");
  const demoConfigs = new Set(Demo.DEMO_REFS.map((r) => r.config));
  assert.ok(CONFIG_REFS.every((r) => !demoConfigs.has(r.config)), "no demo config reaches the board");
  if (!SOL.ready) assert.deepEqual(CONFIG_REFS, [], "before launch the board has no config to read");
  assert.ok(Demo.DEMO_SOL.collections.every((c) => c.config !== SOL.collections.find((x) => x.slug === c.slug)?.config), "the demo's day-1 addresses are its own");
});

// ---- the fixtures' shapes --------------------------------------------------------------------------------------------------------

const coins = Demo.demoCoins(2_000_000_000);
// The demo used to put a coin on every one of nft-pilot.json's 26 collections, the 14 Ordinals that were not wrapping yet included.
// Since the people choose (2026-10-12) a coin exists only on an open collection: the demo's world is DEMO_OPEN, on all four chains.
test("a board of 33 coins on the demo's open collections (all four chains) and ROOK, none on a collection that is not open, newest first, in every state", () => {
  assert.equal(coins.length, 33, `${coins.length} coins`);
  const by = Demo.demoCountBySlug();
  for (const slug of Demo.DEMO_OPEN) assert.ok(by[slug] >= 1, `${slug} has a coin`);
  for (const c of COLLECTIONS) if (!Demo.DEMO_OPEN.includes(c.slug)) assert.equal(by[c.slug], 0, `${c.slug} is not open: no coin`);
  const chains = new Set(Demo.DEMO_OPEN.map((s) => COLLECTIONS.find((c) => c.slug === s)!.chain));
  assert.deepEqual([...chains].sort(), ["Bitcoin", "Ethereum", "Robinhood Chain", "Solana"]);
  assert.ok(Demo.DEMO_OPEN.includes("bitcoin-frogs") && Demo.DEMO_OPEN.filter((s) => COLLECTIONS.find((c) => c.slug === s)!.chain === "Solana").length >= 2);
  assert.ok(!Demo.DEMO_OPEN.includes(Demo.DEMO_NOT_OPEN), "one pinned collection is not open, for the Open it path");
  assert.equal(by.rook, 1);
  for (let i = 1; i < coins.length; i++) assert.ok(coins[i - 1].launchedAt >= coins[i].launchedAt);
  const phases = new Set(coins.map((c) => c.stats.phase));
  assert.deepEqual([...phases].sort(), ["curve", "graduated", "graduating"]);
  assert.ok(coins.some((c) => c.stats.phase === "curve" && c.stats.progress < 0.02 && 2_000_000_000 - c.launchedAt < 600), "some just launched (inside the anti-sniper minutes)");
  const mid = coins.filter((c) => c.stats.phase === "curve" && c.stats.progress > 0.2 && c.stats.progress < 0.95).map((c) => c.stats.progress);
  assert.ok(mid.length >= 8 && Math.max(...mid) - Math.min(...mid) > 0.5, "mid-curve at various %");
});

// canister-only Rook (docs/v9/canister-only-solana.md §3): a collection coin is quoted in its collection's wX and graduates at the
// collection's threshold in whole wX; the demo's curve is the 85 SOL shape scaled to that threshold. The old demo put every coin on
// one of four SOL tiers (85 / 170 / 255 / 510 SOL): that model is retired, so the thresholds and market caps here are in wX.
test("each coin is well formed: made-up addresses, metadata, a picture that is a collection logo or a generated tile (ROOK: its own), stats that are its wX curve's", () => {
  const seen = new Set<string>();
  for (const c of coins) {
    for (const a of [c.mint, c.pool, c.config, c.creator, c.pool_.baseVault, c.pool_.quoteVault]) assert.ok(isAddress(a), `${c.meta.symbol}: ${a}`);
    assert.ok(!seen.has(c.mint)); seen.add(c.mint);
    assert.equal(c.pool_.baseMint, c.mint); assert.equal(c.pool_.config, c.config); assert.equal(c.meta.mint, c.mint);
    assert.ok(c.meta.name.length <= 32 && c.meta.symbol.length <= 10 && /^[A-Z0-9]+$/.test(c.meta.symbol));
    assert.ok(c.off.image && (c.off.image.startsWith("data:image/svg+xml,") || COLLECTIONS.some((x) => x.logo === c.off.image) || (c.slug === "rook" && c.off.image === "/rook-token/rook.png")), `${c.meta.symbol}'s picture`);
    assert.match(c.off.description ?? "", /Demo coin/);
    assert.equal(c.off.website, null); assert.equal(c.off.twitter, null); assert.equal(c.off.telegram, null);
    const cfg = Demo.demoConfigs().get(c.config)!;
    const t = thresholdWxOf(c.slug);
    assert.equal(cfg.migrationQuoteThreshold, BigInt(t ?? 85) * 1_000_000_000n, `${c.meta.symbol} graduates at its collection's threshold in whole wX (ROOK: 85 SOL)`);
    assert.equal(cfg.quoteMint, c.slug === "rook" ? NATIVE_MINT : Demo.demoWxMint(c.slug), `${c.meta.symbol} is quoted in ${c.slug === "rook" ? "SOL" : "its collection's wX"}`);
    assert.equal(cfg.migrationFeePercentage, c.slug === "rook" ? 0 : 50, "half the raise is the backing share; ROOK has none");
    assert.equal(c.stats.thresholdLamports, cfg.migrationQuoteThreshold);
    const s = poolStats(cfg, c.pool_);
    assert.equal(s.phase, c.stats.phase, c.meta.symbol); assert.equal(s.raisedLamports, c.stats.raisedLamports);
    assert.ok(Math.abs((s.marketCapSol ?? 0) - (c.stats.marketCapSol ?? 0)) < 1e-6 * (s.marketCapSol ?? 1));
    // the market cap reads in the quote: from a quarter of the graduation cap (2.5 × the threshold) at the start price, up to a
    // graduated coin's pool price (at most 2.6 × graduation's in the seeds)
    const grad = Number(cfg.migrationQuoteThreshold) / 1e9 * 2.5;
    assert.ok((c.stats.marketCapSol ?? 0) >= grad / 4 * (1 - 1e-5) && (c.stats.marketCapSol ?? 0) <= grad * 2.6 * (1 + 1e-5), `${c.meta.symbol} market cap ${c.stats.marketCapSol} against ${grad} at graduation`);
    assert.equal(Demo.demoFindCoin(c.mint)?.pool, Demo.DEMO_COINS().find((x) => x.meta.symbol === c.meta.symbol)?.pool);
  }
  assert.equal(Demo.demoFindCoin("11111111111111111111111111111111"), null);
});

test("the demo curves: one shape scaled per threshold raises exactly its threshold to the graduation point, with the market cap ×4 from start to graduation", () => {
  const base = Demo.demoCurveFor(85_000_000_000n);
  assert.deepEqual(base, Demo.DEMO_CURVE, "85 SOL is the base shape itself");
  for (const slug of [...COLLECTIONS.map((c) => c.slug), "rook"]) {
    const k = Demo.demoCurveOf(slug), want = Demo.demoThresholdUnits(slug);
    assert.equal(want, BigInt(thresholdWxOf(slug) ?? 85) * 1_000_000_000n);
    const raised = deltaQuote(k.START, k.GRAD, k.L1, false);
    const gap = raised > want ? raised - want : want - raised;
    assert.ok(gap * 10_000n <= want, `${slug}: the curve raises ${raised} for a threshold of ${want} (within 0.01 %)`);
    assert.ok(Math.abs(Number(k.GRAD) / Number(k.START) - 2) < 1e-6, "√P doubles from start to graduation (to the base curve's own rounding)");
  }
  // the 5 wX curve, Milady's: start market cap 3.125 wMILADY, 12.5 wMILADY at graduation
  const milady = Demo.demoConfigOf("milady");
  const mcap = (sqrt: bigint) => (Number(sqrt) / 2 ** 64) ** 2 * 1e-3 * 1e9;
  assert.ok(Math.abs(mcap(milady.sqrtStartPrice) - 3.125) < 1e-6 && Math.abs(mcap(milady.migrationSqrtPrice) - 12.5) < 1e-6);
  assert.equal(thresholdOf("milady")!.graduationMcapWx, 12.5);
  // the demo's wX/SOL rate is the day-1 floor; ROOK is SOL itself
  assert.equal(Demo.demoQuoteSol("milady"), 18.61); assert.equal(Demo.demoQuoteSol("rook"), 1);
});

test("the board's hooks: pools per config, rows from a page of pools, configs for every ref", () => {
  const milady = Demo.DEMO_REFS.find((r) => r.slug === "milady")!.config;
  const pools = Demo.demoPools([milady]);
  assert.equal(pools.length, Demo.demoCountBySlug().milady);
  assert.deepEqual(Demo.demoRows(pools).map((r) => r.slug), pools.map(() => "milady"));
  // every collection's config (a collection opened in the demo launches under its own) and ROOK's; the board reads the open ones
  assert.equal(Demo.demoConfigs().size, COLLECTIONS.length + 1, "every collection's config and ROOK's");
  assert.deepEqual(Demo.DEMO_REFS.map((r) => r.slug), [...COLLECTIONS.filter((c) => Demo.DEMO_OPEN.includes(c.slug)).map((c) => c.slug), "rook"]);
  assert.equal(Demo.DEMO_SOL.ready, true);
  assert.equal(Demo.DEMO_SOL.cloudinary, null, "no upload in demo");
});

// The old demo told the two-stage story (one coin "backed with wX after the deploy", the fund at 6.2 of 10.69 SOL): canister-only
// Rook has no deploy and no fund, and a coin's backing goes into its vault at graduation (docs/v9/canister-only-solana.md §3.3).
test("backing: the backing share of one graduated coin has been moved into its vault by the custodian (DBC's partner flag), the others' wait", () => {
  const graduated = Demo.DEMO_COINS().filter((c) => c.stats.phase === "graduated");
  assert.ok(graduated.length >= 3);
  const moved = graduated.filter((c) => (c.pool_.migrationFeeWithdrawStatus & 0b100) !== 0);
  assert.equal(moved.length, 1);
  assert.equal(moved[0].meta.symbol, Demo.DEMO_BACKED_SYMBOL); assert.equal(moved[0].backed, true);
  assert.ok(Demo.DEMO_COINS().filter((c) => !c.backed).every((c) => c.pool_.migrationFeeWithdrawStatus === 0));
  assert.ok(!("DEMO_FUND" in Demo) && !("demoBacking" in Demo), "no fund and no rook_backing fixture");
});

test("demo addresses are deterministic and well formed", () => {
  assert.equal(Demo.demoAddress("x"), Demo.demoAddress("x"));
  assert.notEqual(Demo.demoAddress("x"), Demo.demoAddress("y"));
  for (let i = 0; i < 200; i++) assert.ok(isAddress(Demo.demoAddress(`t${i}`)));
});
