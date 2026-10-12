import assert from "node:assert/strict";
import { test } from "node:test";
import { build, combine, convert, defaultParams, fmt, inverse, ratio, scaled } from "../solana/launch-params.mjs";

/** Decision D-c (docs/v9/solana-migration.md §2.9): the launch file converts every ETH value at a live rate agreed by >= 2 sources. */
const L = 1_000_000_000n;
const q = (name: string, solPerEth: string) => ({ name, lamports_per_eth: scaled(solPerEth) });

test("rates are exact integers: a ratio of two prices and an inverse", () => {
  assert.equal(ratio("2483.16", "108.95"), 22791739330n); // 2483.16 / 108.95 = 22.791739330...
  assert.equal(inverse("0.0439040"), 22776967930n);       // 1 / 0.043904
  assert.equal(scaled("20"), 20n * L);
  assert.equal(scaled("0.0000000005"), 1n, "half up at the tenth decimal");
  assert.throws(() => scaled("-1"));
  assert.equal(fmt(22_800_000_000n), "22.8");
});

test("two sources at least, and refused when any two differ by more than 1 %", () => {
  assert.throws(() => combine([q("a", "22.8"), { name: "b", error: "HTTP 500" }]), /1 rate source\(s\) answered, 2 needed/);
  assert.throws(() => combine([q("a", "22.8"), q("b", "23.1")]), /differ by 132 bps/);
  const r = combine([q("a", "22.8"), q("b", "22.9"), { name: "c", error: "timeout" }, q("d", "22.85")]);
  assert.equal(r.lamports_per_eth, scaled("22.85"), "the median of the three that answered");
  assert.equal(r.used, 3);
  assert.equal(r.spread_bps, 44);
  const even = combine([q("a", "22.8"), q("b", "22.9")]);
  assert.equal(even.lamports_per_eth, scaled("22.8"), "an even count takes the lower middle: fewer lamports per ETH");
  assert.equal(combine([q("a", "22.8"), q("b", "23.0")], { maxSpreadBps: 100 }).spread_bps, 88);
});

test("conversions: wei at lamports per ETH, half up", () => {
  assert.equal(convert(10n ** 18n, scaled("22.8")), 22_800_000_000n);
  assert.equal(convert(2n * 10n ** 15n, scaled("22.8")), 45_600_000n);
  assert.equal(convert(5n * 10n ** 14n, scaled("22.8")), 11_400_000n);
  assert.equal(convert(1n, scaled("22.8")), 0n);
});

test("the launch file names every value's ETH origin, the rate, the network, and the init arguments", () => {
  const quotes = [q("coinbase", "22.8"), q("okx", "22.82"), { name: "kraken", error: "HTTP 503" }];
  const f = build({ rate: combine(quotes), quotes, at: "2026-10-09T22:00:00Z" });
  assert.equal(f.bitcoin_network, "Mainnet", "mainnet unless named");
  assert.equal(f.cluster, "mainnet-beta");
  assert.equal(f.rate.sol_per_eth, "22.8");
  assert.deepEqual(f.rate.sources.map((s: any) => s.name), ["coinbase", "okx", "kraken"]);
  assert.equal(f.rate.sources[2].error, "HTTP 503");
  assert.equal(f.values.revenue_target.lamports, "22800000000");
  assert.match(f.values.revenue_target.eth_origin, /StackChecks/);
  assert.equal(f.values.revenue_target.eth, "1");
  assert.equal(f.values.wake_bond.lamports, "45600000");
  assert.equal(f.values.wake_bond.eth, "0.002");
  assert.equal(f.values.wake_min_bond.lamports, "11400000");
  assert.equal(f.values.wake_max_bond.lamports, "22800000000");
  assert.deepEqual(f.init.init_desk, { bond: "45600000", min_bond: "11400000", max_bond: "22800000000", window: 604_800 });
  assert.equal(f.init.init_module.params.revenue_target, "22800000000");
  assert.equal(f.init.init_module.params.max_items_per_day, 3);
  assert.equal(f.init.init_registry.bitcoin_network, "Mainnet");
  assert.equal(f.fixed.frog_purchase.ceiling_lamports, "250000000000");
  assert.equal(f.review.reviewed_by, null);
});

test("a devnet rehearsal file may name signet; a mainnet-beta file may not", () => {
  const quotes = [q("a", "22.8"), q("b", "22.8")];
  const rate = combine(quotes);
  assert.equal(build({ rate, quotes, at: "t", cluster: "devnet", bitcoinNetwork: "signet" }).bitcoin_network, "Signet");
  assert.equal(build({ rate, quotes, at: "t", cluster: "localnet", bitcoinNetwork: "testnet4" }).init.init_registry.bitcoin_network, "Testnet4");
  assert.throws(() => build({ rate, quotes, at: "t", bitcoinNetwork: "signet" }), /mainnet-beta launch file names Bitcoin mainnet/);
  assert.throws(() => build({ rate, quotes, at: "t", bitcoinNetwork: "regtest" }), /bitcoin network regtest/);
  assert.throws(() => build({ rate, quotes, at: "t", wakeWindow: 3600 }), /wake window/);
  assert.deepEqual(defaultParams("1", 2).max_items_per_day, 12);
});

test("audit A6 (2026-10-10): ROOK metadata defaults to ROOK/ROOK and the hosted uri; fund-status counts the listing ceremony", async () => {
  const { meteoraSection } = await import("../solana/launch-params.mjs");
  const md = meteoraSection().genesis_rook.metadata;
  assert.equal(md.name.value, "ROOK");
  assert.equal(md.symbol.value, "ROOK");
  assert.equal(md.uri.value, "https://res.cloudinary.com/ddwebzwrk/raw/upload/v1791607907/rook-token/rook.json", "the hosted metadata (operator, 2026-10-10)");
  assert.ok(md.uri.value.length <= 200, "Metaplex MAX_URI_LENGTH");
  assert.doesNotMatch(meteoraSection().config_parameters.token_supply.post_migration_token_supply.source, /burns/);
  const { listingTarget, LISTING_CEREMONY_SIZES } = await import("../solana/fund-status.mjs");
  const rent = (n: number) => (128n + BigInt(n)) * 5_080n;
  const t = listingTarget(rent, 12);
  assert.equal(LISTING_CEREMONY_SIZES.per_listing.length, 9);
  assert.ok(t.total > 0n && t.total === t.once + 12n * t.per_listing + t.fees);
  // docs/v9/ordinals-collections.md: the default is every listing of the order (26: 11 Ethereum, Bitcoin Frogs, fourteen more Ordinals)
  const { LISTINGS } = await import("../solana/fund-status.mjs");
  assert.equal(LISTINGS, 26);
  assert.deepEqual(listingTarget(rent), listingTarget(rent, 26));
});

test("listing order: 11 Ethereum, Bitcoin Frogs at 11, then nft-pilot.json `inscriptions` at 12-25; every root is its members file's", async () => {
  const { listingOrder } = await import("../solana/launch-params.mjs");
  const { rootOf, inscriptionEntries, proofDepth } = await import("../solana/ordinals-members.mjs");
  const { readFileSync } = await import("node:fs");
  const order = listingOrder();
  assert.deepEqual(order.map((l: { listing_id: number }) => l.listing_id), [...Array(26).keys()]);
  assert.equal(order[11].slug, "bitcoin-frogs");
  assert.deepEqual(order.slice(12, 17).map((l: { slug: string }) => l.slug), ["nodemonkes", "omb", "bitcoin-puppets", "bitcoin-punks", "natcats"]);
  // 2026-10-10 (operator): the collections the first pass dropped, except Ordinal Punks, appended most liquid first; ids 0-16 unmoved
  assert.deepEqual(order.slice(17).map((l: { slug: string }) => l.slug),
    ["runestone", "bitcoin-weirdos", "cents", "rsic", "pizza-ninjas", "quantum-cats", "wizards-of-ord", "taproot-wizards", "bitcoin-wizards"]);
  assert.ok(order.slice(11).every((l: { kind: string }) => l.kind === "Inscriptions"));
  assert.ok(order.length <= 32, "rook_core Treasury MAX_COLLECTIONS: every listing's module must fit");
  assert.ok(order.every((l: { symbol: string; name: string }) => l.symbol.length <= 10 && l.name.length <= 32), "Metaplex symbol ≤ 10, rook_core MAX_NAME 32");
  assert.equal(new Set(order.map((l: { symbol: string }) => l.symbol)).size, order.length, "wX symbols are distinct");
  const pilot = JSON.parse(readFileSync(new URL("../../deployments/launch/nft-pilot.json", import.meta.url), "utf8"));
  const seen = new Set<string>();
  for (const e of inscriptionEntries(pilot)) {
    const m: string[] = JSON.parse(readFileSync(new URL(`../../${e.membersFile}`, import.meta.url), "utf8"));
    assert.equal(m.length, e.membersCount, `${e.slug}: membersCount`);
    assert.equal(rootOf(m), e.membersRoot, `${e.slug}: membersRoot is the root of membersFile`);
    assert.ok(proofDepth(m.length) <= 20, `${e.slug}: proofs fit one Solana transaction`);
    for (const id of m) { assert.ok(!seen.has(id), `${id} is in two collections`); seen.add(id); }
  }
  // docs/v9/ordinals-collections.md: a per-collection minimum purchase for the canister (D-b's 1 SOL is above some floors), at most 1 SOL
  for (const e of pilot.inscriptions.slice(5)) {
    assert.ok(e.minPurchaseSol > 0 && e.minPurchaseSol <= 1 && e.wrapping === "later", `${e.slug}: minPurchaseSol, wrapping later`);
    assert.ok(e.maxPurchaseLamports >= 10e9 && e.maxPurchaseLamports <= 250e9, `${e.slug}: purchase limit within 10..250 SOL`);
  }
});

test("graduation tiers (docs/v9/graduation-tiers.md): every listing has one; each tier's buying power covers 1.33 x the recorded floor", async () => {
  const lp = await import("../solana/launch-params.mjs");
  const sc = await import("../solana/site-config.mjs");
  const { execFileSync } = await import("node:child_process");
  assert.deepEqual(lp.TIER_SOL, [85, 170, 255, 510]);
  assert.deepEqual(sc.TIER_SOL, lp.TIER_SOL, "site-config.mjs's copy of the tiers");
  assert.ok(Math.abs(lp.buyingPower(85) - 30.946) < 0.01, "85 x 0.5 x 0.75 / 1.03");
  assert.equal(lp.tierFor(23.2), 85);
  assert.equal(lp.tierFor(23.3), 170);
  assert.equal(lp.tierFor(1000), null);
  const tiers = lp.graduationTiers();
  for (const l of lp.listingOrder()) {
    const c = tiers.collections[l.slug];
    assert.equal(l.graduation_sol, c.tier, `${l.slug}: listing_order carries graduation-tiers.json's tier`);
    assert.equal(lp.tierFor(Number(c.floor_sol)), c.tier, `${l.slug}: the smallest tier covering 1.33 x ${c.floor_sol} SOL`);
  }
  // each tier's curve is design_curve.mjs's for that threshold at the 50 % migration fee (a fixed-supply curve validates only for its own)
  // the section is typed from a JSON with a `_` note only: its tiers are read as a record (the test pins their shape below)
  const m = lp.meteoraSection() as unknown as { tiers: Record<string, any>; config_parameters: any }; // eslint-disable-line @typescript-eslint/no-explicit-any
  for (const t of lp.TIER_SOL) {
    const d = JSON.parse(execFileSync("node", [new URL("../../solana/tests/q1/design_curve.mjs", import.meta.url).pathname, "1000000000", "20", String(t), "50", "6"]).toString());
    const o = m.tiers[String(t)];
    assert.equal(o.migration_quote_threshold.value, d.migrationQuoteThreshold);
    assert.equal(o.sqrt_start_price.value, d.sqrtStartPrice);
    assert.deepEqual(o.curve.value.map((c: { sqrt_price: string; liquidity: string }) => [c.sqrt_price, c.liquidity]), d.curve.map((c: { sqrtPrice: string; liquidity: string }) => [c.sqrtPrice, c.liquidity]));
  }
  assert.deepEqual([m.tiers["85"].sqrt_start_price.value, m.tiers["85"].curve.value], [m.config_parameters.sqrt_start_price.value, m.config_parameters.curve.value], "85 is the D-e config");
});
