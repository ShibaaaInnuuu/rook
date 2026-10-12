import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { b58decode, b58encode, b64decode } from "../src/rook/sol/base58";
import { isAddress, METADATA_PROGRAM, NATIVE_MINT, keyBytes } from "../src/rook/sol/keys";
import { ata, localKeypair, pda } from "../src/rook/sol/pda";
import * as D from "../src/rook/sol/dbc";
import { decodeMetadata, decodeMint, decodePythPrice, SOL_USD_FEED_ID } from "../src/rook/sol/accounts";
import { compileMessage, decodeMessage, instructionsOf, parseTransaction, serializeMessage, serializeTransaction, txSize, unsignedTransaction, MAX_TX_BYTES } from "../src/rook/sol/tx";
import { buyIxs, launchIxs, newPoolRef, sellIxs, withSlippage } from "../src/rook/sol/trade";
import { loadSolanaConfig, type RawSiteConfig } from "../src/rook/sol/siteConfig";
import { availabilityOf, CHAINS, collectionBySlug, COLLECTIONS, custodyOf, destinationOf, LAUNCH_COUNT, LISTED_COLLECTIONS, PINNED_COLLECTIONS, REGISTRY_ORDER, ROBINHOOD_COLLECTIONS, SOLANA_COLLECTIONS } from "../src/rook/sol/collections";
import { launchableOf } from "../src/rook/sol/opening";
import { solText } from "../src/rook/sol/format";
import { failureOf } from "../src/rook/sol/send";
import { gatewayUrl, parseOffChain, sortNewest } from "../src/rook/sol/board";
import { candidates } from "../src/rook/sol/offchain";
import { uriProblem } from "../src/rook/sol/upload";
import { SOLANA_SITE_CONFIG } from "../src/config/solana";
import { COLLECTION_LINKS } from "../src/rook/sol/collectionLinks";
import { MIN_THRESHOLD_WX, THRESHOLDS, backingShareWxOf, quoteOf, raiseText, thresholdOf, thresholdText, thresholdUnitsOf, thresholdWx, wxText } from "../src/rook/sol/graduation";

const here = (p: string) => new URL(p, import.meta.url);
const json = (p: string) => JSON.parse(readFileSync(here(p), "utf8"));
const MAIN = json("fixtures/sol/mainnet-accounts.json");
const PLAN = json("fixtures/sol/day1-plan.rehearsal.json");
const bytes = (b64: string) => b64decode(b64);
const OPS = here("../../ops/solana/");

// ---- encodings --------------------------------------------------------------------------------------------------------------------

test("base58 round-trips, keeps leading zeros, and refuses a character outside the alphabet", () => {
  assert.equal(b58encode(new Uint8Array(32)), "11111111111111111111111111111111");
  for (const a of [D.DBC_PROGRAM, NATIVE_MINT, METADATA_PROGRAM]) assert.equal(b58encode(b58decode(a)), a);
  assert.deepEqual([...b58decode("1112")], [0, 0, 0, 1]);
  assert.throws(() => b58decode("0OIl"));
  assert.ok(isAddress(D.DBC_PROGRAM) && !isAddress("not-an-address") && !isAddress(""));
});

test("PDAs and ATAs: DBC's event authority, a mainnet pool's address and vaults, and the trader's token accounts, as on chain", () => {
  // the swap2 of mainnet tx 2wPWtMab… (test/fixtures/sol/mainnet-accounts.json's pool): event_authority, pool, vaults, user ATAs
  assert.equal(D.dbcEventAuthority(), "8Ks12pbrD6PXxfty1hVQiE9sc289zgU1zHkvXhrSdriF");
  const USDC = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", base = "yZqiHSnWJerJtq5oVZzWwpDAksBa8aAaqp7tztkmoon";
  const pool = D.poolAddress(MAIN.config.pubkey, base, USDC);
  assert.equal(pool, MAIN.pool.pubkey);
  assert.equal(D.tokenVault(base, pool), "67c1f52iMR1suyMeZ9Y7QyvEnSqGo29uPuz4m6NJia1U");
  assert.equal(D.tokenVault(USDC, pool), "7YtHvZQNFTQPioKmQemqBmiVMFUrbRTikrjuPsZHoxoz");
  const user = "GMtRYNmpsMYiRWXKxwS3i2j5LFEQUkbksUcTc1Qh2Hmx";
  assert.equal(ata(user, USDC), "C55pgxUad4SyCcRBBkbwukUvCddMdmBX4iL97v7JxKAX", "the sell's output: the trader's USDC account");
  assert.equal(ata(user, base), "8fwszedxLESkKpp9dzZkNA3tx8o5XHcHHB95mfMoZtr8", "the sell's input: the trader's coin account");
  assert.equal(D.metadataAddress(base), MAIN.metadata.pubkey);
});

test("legacy messages: every day-1 message the ceremony tool compiled decodes and recompiles byte for byte", () => {
  // the rehearsal's 12 compiled listing messages + ROOK's (rows 12-16 were added to the fixture by hand: docs/v9/ordinals-collections.md)
  const msgs = [...PLAN.listings.filter((l: { unsigned_message_b64?: string }) => l.unsigned_message_b64).map((l: { unsigned_message_b64: string }) => l.unsigned_message_b64), PLAN.genesis_rook.unsigned_message_b64];
  assert.equal(msgs.length, 13);
  for (const b64 of msgs) {
    const raw = bytes(b64), m = decodeMessage(raw);
    assert.deepEqual([...serializeMessage(m)], [...raw], "decode → serialize is the identity");
    const again = compileMessage(m.accountKeys[0], instructionsOf(m), m.recentBlockhash);
    assert.deepEqual([...serializeMessage(again)], [...raw], "recompiled from its instructions, the same bytes (account order included)");
  }
});

test("initialize_virtual_pool_with_spl_token: the site's builder IS the ceremony tool's ROOK launch instruction, byte for byte", () => {
  const raw = bytes(PLAN.genesis_rook.unsigned_message_b64), m = decodeMessage(raw), ixs = instructionsOf(m);
  const at = ixs.findIndex((ix) => ix.programId === D.DBC_PROGRAM && [...ix.data.subarray(0, 8)].join() === [...D.DISC.initializeVirtualPoolWithSplToken].join());
  assert.ok(at > 0, "the ROOK message carries the pool launch");
  const payer = PLAN.ceremony_payer, g = PLAN.genesis_rook;
  const mine = D.initializeVirtualPoolWithSplToken({ config: g.config, creator: payer, payer, baseMint: g.mint, name: g.name, symbol: g.symbol, uri: g.uri });
  assert.deepEqual(mine.keys.map((k) => k.pubkey), ixs[at].keys.map((k) => k.pubkey), "the 16 accounts, in order");
  assert.deepEqual([...mine.data], [...ixs[at].data], "discriminator + Borsh name, symbol, uri");
  assert.equal(mine.keys.find((k) => k.pubkey === g.mint)!.isSigner, true, "the mint keypair signs");
  assert.equal(D.poolAddress(g.config, g.mint), g.pool, "the pool PDA is day1-plan.json's");
  // the whole message rebuilt with the site's instruction in place of the tool's: identical bytes
  const swapped = [...ixs.slice(0, at), mine, ...ixs.slice(at + 1)];
  assert.deepEqual([...serializeMessage(compileMessage(payer, swapped, m.recentBlockhash))], [...raw]);
});

test("swap (exact in): discriminator, amount_in and minimum_amount_out little-endian; no referral is the program id; accounts in the IDL's order", () => {
  const p = newPoolRef(PLAN.listings[0].config, PLAN.genesis_rook.mint);
  const ix = D.swap({ ...p, user: PLAN.ceremony_payer, input: "11111111111111111111111111111112", output: "11111111111111111111111111111113", amountIn: 1_000_000_000n, minimumAmountOut: 258n });
  assert.deepEqual([...ix.data], [...D.DISC.swap, 0, 0xca, 0x9a, 0x3b, 0, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0]);
  assert.deepEqual(ix.keys.map((k) => k.pubkey), [D.DBC_POOL_AUTHORITY, p.config, p.pool, "11111111111111111111111111111112", "11111111111111111111111111111113", p.baseVault, p.quoteVault, p.baseMint, NATIVE_MINT, PLAN.ceremony_payer,
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", D.DBC_PROGRAM, D.dbcEventAuthority(), D.DBC_PROGRAM]);
  assert.deepEqual(ix.keys.filter((k) => k.isWritable).map((k) => k.pubkey), [p.pool, "11111111111111111111111111111112", "11111111111111111111111111111113", p.baseVault, p.quoteVault]);
});

test("a launch with a first buy fits one transaction with a full-length name and symbol and a 120-byte metadata URL; the 200-byte maximum does not with a buy", () => {
  const user = "GMtRYNmpsMYiRWXKxwS3i2j5LFEQUkbksUcTc1Qh2Hmx", mint = localKeypair().address, config = PLAN.listings[3].config;
  const sized = (uri: string, buy: bigint, quoteMint?: string) => txSize(compileMessage(user, [D.setComputeUnitLimit(300_000), D.setComputeUnitPrice(10_000n), ...launchIxs({ user, config, mint, quoteMint, name: "N".repeat(32), symbol: "S".repeat(10), uri, firstBuy: buy, minOut: 1n })], "11111111111111111111111111111111"));
  assert.ok(sized("https://x/" + "u".repeat(110), 1_000_000_000n) <= MAX_TX_BYTES, `launch + buy: ${sized("https://x/" + "u".repeat(110), 1n)} bytes`);
  assert.ok(sized("https://x/" + "u".repeat(190), 0n) <= MAX_TX_BYTES, "launch alone with the longest uri");
  const m = compileMessage(user, launchIxs({ user, config, mint, name: "Coin", symbol: "COIN", uri: "https://a/b.json", firstBuy: 1n, minOut: 1n }), "11111111111111111111111111111111");
  assert.equal(m.header.numRequiredSignatures, 2, "the wallet and the new mint sign, nobody else");
  assert.equal(m.accountKeys[0], user, "the wallet pays");
  // canister-only Rook: a collection's config is quoted in its wX, so the first buy is the wallet's wX (no wrap, nothing to unwrap)
  // and the pool, its quote vault and the swap name the wX (docs/v9/canister-only-solana.md §3.1)
  const WX = "DyRkUpQxYG2VnP2SkMdQs5BTsVPeKCpSHLxzByc2Sxvj";
  const wx = launchIxs({ user, config, mint, quoteMint: WX, name: "Coin", symbol: "COIN", uri: "https://a/b.json", firstBuy: 1_000_000_000n, minOut: 1n });
  assert.deepEqual(wx.map((ix) => ix.programId), [D.DBC_PROGRAM, "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL", D.DBC_PROGRAM], "init, the coin account, the swap: no system transfer, no sync, no close");
  assert.equal(wx[0].keys[4].pubkey, WX, "the pool is initialized with the wX as its quote");
  assert.equal(wx[0].keys[5].pubkey, D.poolAddress(config, mint, WX));
  assert.equal(wx[2].keys[3].pubkey, ata(user, WX), "the swap spends the wallet's wX account");
  assert.equal(wx[2].keys[8].pubkey, WX, "and names the wX as the quote mint");
  assert.ok(sized("https://x/" + "u".repeat(110), 1_000_000_000n, WX) < sized("https://x/" + "u".repeat(110), 1_000_000_000n), "shorter than the SOL launch: three instructions fewer");
  assert.equal(launchIxs({ user, config, mint, quoteMint: WX, name: "Coin", symbol: "COIN", uri: "https://a/b.json", firstBuy: 0n, minOut: 0n }).length, 1, "no first buy: the init alone");
  // the creator's first buy at the minimum fee (enable_first_swap_with_min_fee, 2026-10-11): the launch's swap names the Instructions
  // sysvar as its one remaining account (read-only, not a signer), after the 15 accounts of `swap`; on a SOL-quoted launch too
  const SYSVAR = "Sysvar1nstructions1111111111111111111111111";
  for (const ixs of [wx, launchIxs({ user, config, mint, name: "Coin", symbol: "COIN", uri: "https://a/b.json", firstBuy: 1n, minOut: 1n })]) {
    const sw = ixs.find((ix) => ix.programId === D.DBC_PROGRAM && ix !== ixs[0])!;
    assert.equal(sw.keys.length, 16);
    assert.deepEqual(sw.keys[15], { pubkey: SYSVAR, isWritable: false, isSigner: false });
  }
});

test("buy / sell instruction lists: wrap SOL, swap, unwrap; the coin account is created idempotently", () => {
  const user = "GMtRYNmpsMYiRWXKxwS3i2j5LFEQUkbksUcTc1Qh2Hmx", p = newPoolRef(PLAN.listings[0].config, PLAN.genesis_rook.mint);
  const b = buyIxs(user, p, 5n, 4n), s = sellIxs(user, p, 7n, 3n), noUnwrap = buyIxs(user, p, 5n, 4n, false);
  assert.deepEqual(b.map((ix) => ix.programId), ["ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL", "11111111111111111111111111111111", "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL", D.DBC_PROGRAM, "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"]);
  assert.equal(noUnwrap.length, b.length - 1);
  assert.ok([b[4], s[1]].every((ix) => ix.keys.length === 15), "an ordinary buy or sell names no Instructions sysvar (only a launch's first buy does)");
  assert.equal(s[1].keys[3].pubkey, ata(user, p.baseMint), "a sell's input is the coin account");
  assert.equal(s[1].keys[4].pubkey, ata(user, NATIVE_MINT), "and its output the wSOL account");
  assert.equal(withSlippage(10_000n, 100), 9_900n);
  assert.equal(withSlippage(10_000n, 99_999), 5_000n, "slippage is clamped at 50 %");
});

test("transactions on the wire: signatures first (zeros until signed), and a signature placed by index parses back", () => {
  const user = localKeypair(), m = compileMessage(user.address, [D.transferLamports(user.address, NATIVE_MINT, 1n)], "11111111111111111111111111111111");
  const msg = serializeMessage(m), wire = serializeTransaction(msg, [user.sign(msg)]);
  const p = parseTransaction(wire);
  assert.deepEqual([...p.message], [...msg]);
  assert.equal(p.signatures.length, 1);
  assert.equal(unsignedTransaction(m).length, wire.length);
});

// ---- decoders ---------------------------------------------------------------------------------------------------------------------

test("a mainnet DBC 0.2.1 PoolConfig and VirtualPool decode, and agree with each other: the curve's integral is the pool's quote reserve", () => {
  const c = D.decodeConfig(bytes(MAIN.config.data)), p = D.decodePool(bytes(MAIN.pool.data));
  assert.equal(p.config, MAIN.config.pubkey);
  assert.equal(p.baseMint, "yZqiHSnWJerJtq5oVZzWwpDAksBa8aAaqp7tztkmoon");
  assert.equal(p.baseVault, "67c1f52iMR1suyMeZ9Y7QyvEnSqGo29uPuz4m6NJia1U");
  assert.equal(c.quoteMint, "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
  assert.equal(c.tokenDecimal, 6);
  assert.equal(c.preMigrationTokenSupply, 10n ** 15n);
  assert.ok(c.curve.length >= 1 && c.sqrtStartPrice > 0n && p.sqrtPrice >= c.sqrtStartPrice);
  // ∫ quote from the start price to √P along the curve = what the curve took in (to rounding)
  let s = c.sqrtStartPrice, q = 0n, b = 0n;
  for (const pt of c.curve) { const hi = pt.sqrtPrice < p.sqrtPrice ? pt.sqrtPrice : p.sqrtPrice; if (hi > s) { q += D.deltaQuote(s, hi, pt.liquidity, true); b += D.deltaBase(s, hi, pt.liquidity, false); s = hi; } }
  const diff = q > p.quoteReserve ? q - p.quoteReserve : p.quoteReserve - q;
  assert.ok(diff * 1000n < p.quoteReserve, `integral ${q} vs reserve ${p.quoteReserve}`);
  const supplyGap = b + p.baseReserve - c.preMigrationTokenSupply;
  assert.ok(supplyGap >= -1000n && supplyGap <= 1000n, "coins sold along the curve + the base reserve = the supply");
  assert.throws(() => D.decodePool(bytes(MAIN.config.data)), /not a DBC VirtualPool/);
  assert.throws(() => D.decodeConfig(bytes(MAIN.pool.data)), /not a DBC PoolConfig/);
});

test("Metaplex metadata, an SPL mint and Pyth's SOL/USD decode from mainnet bytes", () => {
  const md = decodeMetadata(bytes(MAIN.metadata.data));
  assert.equal(md.mint, "yZqiHSnWJerJtq5oVZzWwpDAksBa8aAaqp7tztkmoon");
  assert.ok(md.name.length > 0 && md.name.length <= 32 && !md.name.includes("\0"));
  assert.ok(md.uri.length <= 200);
  const mint = decodeMint(bytes(MAIN.baseMint.data));
  assert.equal(mint.decimals, 6);
  assert.ok(mint.supply <= 10n ** 15n && mint.supply > 99n * 10n ** 13n, "1B coins less what holders burned");
  const py = decodePythPrice(bytes(MAIN.pyth.data));
  assert.equal(py.feedId, SOL_USD_FEED_ID);
  assert.ok(py.price > 10 && py.price < 10_000, `SOL/USD ${py.price}`);
});

// ---- the curve --------------------------------------------------------------------------------------------------------------------

/** Rook's collection curve (decision D-e; ops/solana/day1-pins.json, spike Q1 §3). */
const ROOK_CURVE = {
  sqrtStartPrice: 134452599321738966n,
  curve: [{ sqrtPrice: 268905193275807567n, liquidity: 215124158914782432327345953095692n }, { sqrtPrice: 79226673521066979257578248091n, liquidity: 3435308751277378593870180n }],
  baseFee: { cliffFeeNumerator: 500_000_000n, firstFactor: 60, secondFactor: 10n, thirdFactor: 8_166_666n, baseFeeMode: 0 },
  collectFeeMode: 0, tokenDecimal: 6, fixedSupply: true, preMigrationTokenSupply: 10n ** 15n, migrationQuoteThreshold: 85_000_000_000n,
} as unknown as D.DbcConfig;

test("curve math on Rook's D-e curve: start mcap ≈ 53 SOL, 85 SOL buys ≈ 800M coins and ends at ≈ 2.125e-7 SOL (spike Q1 §3)", () => {
  const start = D.priceSol(ROOK_CURVE.sqrtStartPrice, 6);
  assert.ok(Math.abs(start * 1e9 - 53.125) < 0.1, `start mcap ${start * 1e9}`);
  const r = D.quoteToBase(ROOK_CURVE, ROOK_CURVE.sqrtStartPrice, 85_000_000_000n)!;
  const coins = Number(r.out) / 1e6;
  assert.ok(Math.abs(coins - 799_999_984) < 2, `coins on the curve ${coins}`);
  assert.equal(r.nextSqrtPrice, ROOK_CURVE.curve[0].sqrtPrice, "the raise ends exactly at the first segment's top (the migration price)");
  assert.ok(Math.abs(D.priceSol(r.nextSqrtPrice, 6) - 2.125e-7) < 1e-10);
  // selling everything back returns the raise (to rounding, against the buyer)
  const back = D.baseToQuote(ROOK_CURVE, r.nextSqrtPrice, r.out)!;
  assert.ok(back.out <= 85_000_000_000n && 85_000_000_000n - back.out < 10n, `sold back for ${back.out}`);
  assert.equal(D.baseToQuote(ROOK_CURVE, ROOK_CURVE.sqrtStartPrice, 1_000_000n), null, "nothing can be sold below the start price");
});

test("the anti-sniper fee: 50 % at launch, linear to 1.000004 % after 10 minutes, then flat; a sell pays it on the SOL out", () => {
  const at = 1_000_000n;
  assert.equal(D.baseFeeNumerator(ROOK_CURVE, at, at), 500_000_000n);
  assert.equal(D.baseFeeNumerator(ROOK_CURVE, at, at + 9n), 500_000_000n, "periods of 10 s");
  assert.equal(D.baseFeeNumerator(ROOK_CURVE, at, at + 10n), 491_833_334n);
  assert.equal(D.baseFeeNumerator(ROOK_CURVE, at, at + 600n), 10_000_040n);
  assert.equal(D.baseFeeNumerator(ROOK_CURVE, at, at + 86_400n), 10_000_040n);
  const pool = { sqrtPrice: ROOK_CURVE.sqrtStartPrice, activationPoint: at };
  const early = D.quoteSwap(ROOK_CURVE, pool, "buy", 1_000_000_000n, at)!, late = D.quoteSwap(ROOK_CURVE, pool, "buy", 1_000_000_000n, at + 3_600n)!;
  assert.equal(early.fee, 500_000_000n, "1 SOL at launch pays 0.5 SOL (spike Q1 §1)");
  assert.equal(late.fee, 10_000_040n);
  assert.ok(late.amountOut > early.amountOut);
  const sell = D.quoteSwap(ROOK_CURVE, { sqrtPrice: late.nextSqrtPrice, activationPoint: at }, "sell", late.amountOut, at + 3_600n)!;
  assert.ok(sell.amountOut < 1_000_000_000n - late.fee, "a round trip loses both fees");
});

test("pool stats: progress to the 85 SOL threshold, the phase, the market cap", () => {
  const p = { ...D.decodePool(bytes(MAIN.pool.data)), sqrtPrice: ROOK_CURVE.sqrtStartPrice, quoteReserve: 42_500_000_000n, isMigrated: false };
  const s = D.poolStats(ROOK_CURVE, p);
  assert.equal(s.progress, 0.5);
  assert.equal(s.phase, "curve");
  assert.ok(Math.abs(s.marketCapSol! - 53.125) < 0.1);
  assert.equal(D.poolStats(ROOK_CURVE, { ...p, quoteReserve: 85_000_000_001n }).phase, "graduating");
  assert.equal(D.poolStats(ROOK_CURVE, { ...p, quoteReserve: 90_000_000_000n }).progress, 1);
  assert.equal(D.poolStats(ROOK_CURVE, { ...p, isMigrated: true }).phase, "graduated");
});

// ---- the site config --------------------------------------------------------------------------------------------------------------

test("the committed site config is the placeholder: not ready, no problem, nothing invented", () => {
  const c = loadSolanaConfig(SOLANA_SITE_CONFIG as RawSiteConfig);
  assert.deepEqual(c.problems, []);
  assert.equal(c.ready, false);
  assert.equal(c.collections.length, 26);
  assert.ok(c.collections.every((x) => x.config === null));
  assert.equal(c.rook, null);
  // canister-only Rook (docs/v9/canister-only-solana.md §16): no deploy fund and no fund wallet; ROOK's fees go to the custodian's
  // wallet, read from its config on chain. No program of Rook's own either: the raffle is the canister's (`raffle_status`), no rookCore
  assert.ok(!("fundWallet" in c) && !("rookBacking" in c) && !("rookCore" in c), "the fund wallet, rook_backing and rook_core are gone from the config");
  assert.equal(c.cloudinary, null, "no upload preset is configured");
  assert.equal(loadSolanaConfig(SOLANA_SITE_CONFIG as RawSiteConfig, "https://example.helius-rpc.com/?api-key=x").rpcUrl, "https://example.helius-rpc.com/?api-key=x", "VITE_SOLANA_RPC overrides the file");
});

test("ops/solana/site-config.mjs: a day1-plan.json becomes a ready config; a half-written, misordered or bad one is refused", async () => {
  if (!existsSync(OPS)) return;
  const sc = await import(new URL("site-config.mjs", OPS).href);
  const order = sc.listingOrder(json("../../deployments/launch/nft-pilot.json"));
  assert.deepEqual(order, LISTED_COLLECTIONS.map((c) => c.slug), "listing_order is the site config's collection order");
  const cfg: RawSiteConfig = sc.siteConfigFromPlan(PLAN, order, SOLANA_SITE_CONFIG);
  const ok = loadSolanaConfig(cfg);
  assert.deepEqual(ok.problems, []);
  assert.equal(ok.ready, true);
  assert.equal(ok.collections[11].config, PLAN.listings[11].config);
  assert.equal(ok.collections[16].config, PLAN.listings[16].config);
  assert.deepEqual(ok.rook, { config: PLAN.genesis_rook.config, mint: PLAN.genesis_rook.mint, pool: PLAN.genesis_rook.pool });
  // half-written, misordered, not an address
  const half = structuredClone(cfg); half.collections[4].config = "";
  assert.match(loadSolanaConfig(half).problems.join(" "), /half-written/);
  const swapped = structuredClone(cfg); [swapped.collections[0], swapped.collections[1]] = [swapped.collections[1], swapped.collections[0]];
  assert.match(loadSolanaConfig(swapped).problems.join(" "), /listing_order says milady/);
  const bad = structuredClone(cfg); bad.rook.mint = "0xdeadbeef";
  assert.match(loadSolanaConfig(bad).problems.join(" "), /not a Solana address/);
  assert.equal(loadSolanaConfig(bad).ready, false);
  // the script refuses a misordered plan and another cluster
  const p2 = structuredClone(PLAN); p2.listings.reverse();
  assert.throws(() => sc.siteConfigFromPlan(p2, order, {}), /listing_order says milady/);
  assert.throws(() => sc.siteConfigFromPlan({ ...PLAN, cluster: "devnet" }, order, {}), /cluster devnet/);
  // the committed file is the script's own rendering (so a regeneration diff shows only the addresses)
  const text = readFileSync(here("../src/config/solana.ts"), "utf8");
  assert.equal(sc.renderSiteConfig(sc.readSiteConfig(text), text.slice(0, text.indexOf("export const SOLANA_SITE_CONFIG"))), text);
});

// 41 since the Robinhood Chain measurement (2026-10-11, HANDOFF §8): the three robinhood-collections.json collections follow the 38
test("the collections are nft-pilot.json's, in listing_order, with its wX symbols, then the 12 Solana collections, then the 3 Robinhood Chain ones; every working one is pinned", () => {
  assert.deepEqual(COLLECTIONS.map((c) => c.listingId), [...Array(41).keys()]);
  assert.deepEqual(COLLECTIONS.slice(0, 26), LISTED_COLLECTIONS);
  assert.deepEqual(COLLECTIONS.slice(26, 38).map((c) => [c.slug, c.chain, c.wxSymbol, c.thresholdWx]), SOLANA_COLLECTIONS.map((c) => [c.slug, "Solana", c.wxSymbol, c.thresholdWx]));
  assert.deepEqual(COLLECTIONS.slice(38).map((c) => [c.slug, c.chain]), ROBINHOOD_COLLECTIONS.map((c) => [c.slug, "Robinhood Chain"]));
  // operator decision 2026-10-12, no picking: every collection with a pipeline is pinned (11 Ethereum, Bitcoin Frogs, 12 Solana, the 14
  // Ordinals), each opening on demand; the Robinhood Chain ones after the ceremony's 4663 readback
  assert.equal(PINNED_COLLECTIONS.length, 38);
  assert.deepEqual(["Ethereum", "Bitcoin", "Solana"].map((ch) => PINNED_COLLECTIONS.filter((c) => c.chain === ch).length), [11, 15, 12]);
  // docs/v9/ordinals-collections.md §5: ids 12-25 are nft-pilot.json's `wrapping: "later"` (pinned with the others since 2026-10-12)
  assert.deepEqual(COLLECTIONS.filter((c) => c.wrapping === "later").map((c) => c.listingId), [...Array(14).keys()].map((i) => 12 + i));
  const pilotPath = here("../../deployments/launch/nft-pilot.json");
  if (!existsSync(pilotPath)) return;
  const pilot = json("../../deployments/launch/nft-pilot.json");
  type P = { slug: string; name: string; symbol: string; membersCount?: number };
  const want = [...pilot.collections.map((c: P) => [c.slug, c.name, c.symbol]), [pilot.bitcoin.slug, pilot.bitcoin.name, pilot.bitcoin.symbol],
    ...pilot.inscriptions.map((c: P) => [c.slug, c.name, c.symbol])];
  assert.deepEqual(LISTED_COLLECTIONS.map((c) => [c.slug, c.wrappedName, c.wxSymbol]), want);
  for (const b of [pilot.bitcoin, ...pilot.inscriptions] as P[])
    assert.equal(COLLECTIONS.find((c) => c.slug === b.slug)?.contract, `inscriptions (${b.membersCount!.toLocaleString("en-US")} members)`);
  // the two inputs of each collection's threshold (docs/v9/canister-only-solana.md §3.2) are graduation-tiers.json's tier and day-1
  // floor, committed here because the Docker build context has no deployments/; the Solana collections' rows are solana-collections.json's
  const tiers = json("../../deployments/launch/graduation-tiers.json");
  assert.deepEqual(LISTED_COLLECTIONS.map((c) => [c.tierSol, c.floorSol]), LISTED_COLLECTIONS.map((c) => [tiers.collections[c.slug].tier, Number(tiers.collections[c.slug].floor_sol)]));
  const solana = json("../../deployments/launch/solana-collections.json");
  type K = { key: string; collection_key: string };
  for (const c of solana.collections as K[]) assert.equal(COLLECTIONS.find((x) => x.slug === c.key)?.contract, c.collection_key, `${c.key}: its collection key`);
  type S = { key: string; name: string; symbol: string; tier_sol: number; floor_sol: number; threshold_wx: number; supply: number };
  assert.deepEqual(SOLANA_COLLECTIONS.map((c) => [c.slug, c.name, c.wxSymbol, c.tierSol, c.floorSol, c.thresholdWx, c.supply]),
    (solana.collections as S[]).map((c) => [c.key, c.name, c.symbol, c.tier_sol, c.floor_sol, c.threshold_wx, c.supply]), "the Solana collections, in the launch file's order");
});

// docs/v9/canister-only-solana.md §3.2: T_wX = max(3, ⌈tier_SOL / floor_SOL⌉) whole wX at the day-1 floors, the design's own table
const DESIGN_TABLE: Record<string, [number, number]> = {
  milady: [5, 93], azuki: [7, 96], mayc: [5, 93], pudgypenguins: [5, 295], bayc: [4, 547], doodles: [15, 89], moonbirds: [10, 89], clonex: [26, 88],
  lilpudgys: [15, 88], checks: [25, 87], identitymd: [5, 202], nodemonkes: [5, 87], omb: [7, 92], "bitcoin-puppets": [12, 92], "bitcoin-punks": [37, 87],
  natcats: [40, 86], runestone: [82, 85], "bitcoin-weirdos": [174, 85], cents: [22, 85], rsic: [152, 85], "pizza-ninjas": [13, 85], "quantum-cats": [11, 87],
  "bitcoin-frogs": [29, 87], "wizards-of-ord": [21, 85], "taproot-wizards": [5, 177], "bitcoin-wizards": [9, 91],
};
test("graduation thresholds in whole wX: every collection against the design's §3.2 table, the rest from the launch files; ROOK at 85 SOL", () => {
  // the rule itself, as launch_pins.rs computes it: a ceiling in lamports, never below 3
  assert.equal(thresholdWx(85, 18.61), 5); assert.equal(thresholdWx(85, 17), 5, "85 / 17 is exactly 5"); assert.equal(thresholdWx(85, 16.99), 6);
  assert.equal(thresholdWx(510, 136.84), 4); assert.equal(thresholdWx(85, 0.49), 174); assert.equal(thresholdWx(85, 100), MIN_THRESHOLD_WX, "never below 3");
  assert.throws(() => thresholdWx(85, 0));
  for (const c of LISTED_COLLECTIONS) {
    const t = thresholdOf(c.slug)!, [wx, sol] = DESIGN_TABLE[c.slug];
    assert.equal(t.wx, wx, `${c.slug}: T_wX`);
    // the design's "≈ SOL" column [A] rounds from the unrounded ETH / BTC floors: Lil Pudgys (88) and Quantum Cats (87) land one SOL off
    assert.ok(Math.abs(t.approxSol - sol) <= 1, `${c.slug}: ≈ ${t.approxSol} SOL at the day-1 floor, the design says ${sol}`);
    assert.equal(t.approxSol, Math.round(wx * c.floorSol));
    assert.equal(t.wx, thresholdWx(c.tierSol, c.floorSol)); assert.equal(t.symbol, c.wxSymbol); assert.equal(t.chain, c.chain); assert.equal(t.page, true);
    assert.equal(t.backingWx, wx / 2); assert.equal(t.graduationMcapWx, wx * 2.5);
    assert.equal(backingShareWxOf(c.slug), wx / 2);
    assert.deepEqual(quoteOf(c.slug), { symbol: c.wxSymbol, decimals: 9 });
    assert.deepEqual(thresholdUnitsOf({ collection: { slug: c.slug } }), { amount: wx, symbol: c.wxSymbol }, "a collection with no coin yet: the computed threshold");
    assert.ok(t.wx * t.floorSol >= 85 - 0.5, `${c.slug}: the raise is worth about the tier or more at the day-1 floor`);
  }
  assert.equal(Object.keys(DESIGN_TABLE).length, LISTED_COLLECTIONS.length);
  // the Solana collections: the launch file's threshold_wx (§16: 6 to 71), the same rule on the floors it measured
  for (const c of SOLANA_COLLECTIONS) {
    const t = thresholdOf(c.slug)!;
    assert.equal(t.wx, c.thresholdWx); assert.equal(t.wx, thresholdWx(c.tierSol, c.floorSol), `${c.slug}: threshold_wx is the rule on its floor`);
    assert.equal(t.chain, "Solana"); assert.equal(t.page, true, "a Solana collection has its page"); assert.ok(t.wx >= 6 && t.wx <= 71, `${c.slug}: ${t.wx}`);
    assert.deepEqual(quoteOf(c.slug), { symbol: c.wxSymbol, decimals: 9 });
  }
  assert.equal(THRESHOLDS.length, COLLECTIONS.length);
  assert.deepEqual(THRESHOLDS.slice(0, 5).map((t) => t.wx), [5, 7, 5, 5, 4], "listing order first");
  // ROOK: SOL-paired, 85 SOL, no backing; a coin's own config wins over the computed threshold once it exists
  assert.equal(thresholdOf("rook"), null); assert.equal(backingShareWxOf("rook"), 0); assert.deepEqual(quoteOf("rook"), { symbol: "SOL", decimals: 9 });
  assert.deepEqual(thresholdUnitsOf({ collection: { slug: "rook" } }), { amount: 85, symbol: "SOL" });
  assert.deepEqual(thresholdUnitsOf({ curve: { thresholdLamports: "5000000000" }, collection: { slug: "milady" } }), { amount: 5, symbol: "wMILADY" });
  assert.deepEqual(thresholdUnitsOf({ curve: { thresholdLamports: "0" }, collection: { slug: "azuki" } }), { amount: 7, symbol: "wAZUKI" }, "an empty config falls back");
  // the copy: "5 wMILADY (≈ 93 SOL at the day-1 floor)"
  assert.equal(thresholdText("milady"), "5 wMILADY (≈ 93 SOL at the day-1 floor)");
  assert.equal(thresholdText("bitcoin-weirdos", false), "174 wWEIRDO");
  assert.equal(thresholdText("rook"), "85 SOL");
  assert.equal(raiseText({ collection: { slug: "bayc" } }), "4 wBOREDAPE");
  assert.equal(wxText(2.5), "2.5"); assert.equal(wxText(174), "174");
});

// The Robinhood Chain collections (2026-10-11) have OpenSea only: robinhood-collections.json records no X handle or website, and the
// site shows none it has not verified, so the X handle is required of every other collection.
test("every collection has its official links: an X handle (none recorded for Robinhood Chain), https only, OpenSea for Ethereum and Robinhood Chain, Ordinals Wallet for Ordinals", () => {
  assert.deepEqual(Object.keys(COLLECTION_LINKS).sort(), COLLECTIONS.map((c) => c.slug).sort());
  for (const c of COLLECTIONS) {
    const l = COLLECTION_LINKS[c.slug];
    if (c.chain === "Robinhood Chain") assert.equal(l.x, undefined, c.slug); else assert.match(l.x ?? "", /^[A-Za-z0-9_]{1,15}$/, c.slug);
    for (const u of [l.website, ...l.marketplaces.map((m) => m.url)].filter(Boolean)) assert.match(u!, /^https:\/\/[a-z0-9.-]+\.[a-z]+(\/[A-Za-z0-9_./-]*)?$/, `${c.slug}: ${u}`);
    assert.equal(l.marketplaces[0].name, c.chain === "Ethereum" || c.chain === "Robinhood Chain" ? "OpenSea" : c.chain === "Solana" ? "Magic Eden" : "Ordinals Wallet", c.slug);
  }
  for (const r of ROBINHOOD_COLLECTIONS) assert.equal(COLLECTION_LINKS[r.slug].marketplaces[0].url, `https://opensea.io/collection/${r.openseaSlug}`);
  const pilotPath = here("../../deployments/launch/nft-pilot.json");
  if (!existsSync(pilotPath)) return;
  const pilot = json("../../deployments/launch/nft-pilot.json");
  for (const e of pilot.collections as { slug: string; openseaSlug: string }[]) assert.equal(COLLECTION_LINKS[e.slug].marketplaces[0].url, `https://opensea.io/collection/${e.openseaSlug}`);
  for (const b of [pilot.bitcoin, ...pilot.inscriptions] as { slug: string; owSlug: string }[]) assert.equal(COLLECTION_LINKS[b.slug].marketplaces[0].url, `https://ordinalswallet.com/collection/${b.owSlug}`);
});

// ---- the collection set (HANDOFF §8, 2026-10-12: Ordinals multi-collection, Robinhood Chain NFTs, opening on demand) -----------------

test("the collection set: 41 on four chains, 38 pinned at launch (the 14 Ordinals included), 3 Robinhood Chain after the ceremony; symbols fit the canister", () => {
  const per = (list: readonly { chain: string }[]) => CHAINS.map((ch) => list.filter((c) => c.chain === ch).length);
  assert.deepEqual(CHAINS, ["Ethereum", "Solana", "Bitcoin", "Robinhood Chain"]);
  assert.deepEqual(per(COLLECTIONS), [11, 12, 15, 3], "Ethereum, Solana, Bitcoin (Frogs + 14 Ordinals), Robinhood Chain");
  assert.deepEqual(per(PINNED_COLLECTIONS), [11, 12, 15, 0], "the 38 the ceremony pins");
  assert.ok(LISTED_COLLECTIONS.slice(12).every((c) => availabilityOf(c) === "pinned"), "the 14 other Ordinals are pinned");
  assert.deepEqual(COLLECTIONS.filter((c) => availabilityOf(c) === "after-ceremony").map((c) => c.slug), ["stonkbrokers", "chainmancers", "rekttradooors"]);
  // the canister's symbol limit is 10 bytes (unlock.rs; the measurement's symbols can exceed it, ceremony runbook §2)
  for (const c of COLLECTIONS) assert.ok(new TextEncoder().encode(c.wxSymbol).length <= 10 && c.wxSymbol.startsWith("w"), `${c.slug}: ${c.wxSymbol}`);
  assert.equal(new Set(COLLECTIONS.map((c) => c.wxSymbol)).size, 41, "every symbol is its own");
  assert.equal(new Set(COLLECTIONS.map((c) => c.slug)).size, 41);
  // QUOTRONS (an ERC-404 pair: token transfers mint and burn its NFTs) is left out by the operator, 2026-10-12
  assert.ok(!COLLECTIONS.some((c) => /quotron/i.test(c.slug + c.name + c.wxSymbol)), "QUOTRONS is not a Rook collection");
  // StonkBrokers is token-bound (ERC-6551), and the only one
  assert.deepEqual(COLLECTIONS.filter((c) => c.tokenBound).map((c) => c.slug), ["stonkbrokers"]);
  // the Robinhood Chain thresholds from graduation-tiers.json's tiers and floors: StonkBrokers tier 170 → 6, Chain Mancers 14, Rekt Tradooors 71
  assert.deepEqual(["stonkbrokers", "chainmancers", "rekttradooors"].map((s) => [thresholdOf(s)!.tierSol, thresholdOf(s)!.wx]), [[170, 6], [85, 14], [85, 71]]);
  // custody and destination per chain
  assert.deepEqual(CHAINS.map(custodyOf), ["the canister's EVM address, on Ethereum", "the canister's own Solana wallet", "the canister's taproot custody address", "the canister's EVM address, on Robinhood Chain"]);
  assert.match(destinationOf("Solana"), /Solana wallet/); assert.match(destinationOf("Robinhood Chain"), /Robinhood Chain/); assert.match(destinationOf("Bitcoin"), /bc1p/);
  // the registry: the canister's 41, the 38 pinned first (the Ordinals last of them), then the Robinhood Chain ones
  assert.equal(REGISTRY_ORDER.length, 41);
  assert.deepEqual([...REGISTRY_ORDER].sort(), COLLECTIONS.map((c) => c.slug).sort(), "the registry and the site name the same collections");
  assert.deepEqual(REGISTRY_ORDER.slice(0, LAUNCH_COUNT).map((s) => collectionBySlug(s)!).sort((a, b) => a.listingId - b.listingId), [...PINNED_COLLECTIONS]);
  assert.deepEqual(REGISTRY_ORDER.slice(38), ["stonkbrokers", "chainmancers", "rekttradooors"]);
  assert.equal(LAUNCH_COUNT, 38);
});

test("the Robinhood Chain collections are robinhood-collections.json's, with graduation-tiers.json's tiers and floors; the registry is unlock.rs's", () => {
  const rhPath = here("../../deployments/launch/robinhood-collections.json");
  if (!existsSync(rhPath)) return;
  const rh = json("../../deployments/launch/robinhood-collections.json");
  type R = { slug: string; name: string; symbol: string; chainId: number; collection: string; openseaSlug: string; tokenBound?: boolean };
  assert.equal(rh.chainId, 4663);
  assert.deepEqual(ROBINHOOD_COLLECTIONS.map((c) => [c.slug, c.wrappedName, c.wxSymbol, c.contract, c.openseaSlug, !!c.tokenBound]),
    (rh.collections as R[]).map((c) => [c.slug, c.name, c.symbol, c.collection, c.openseaSlug, !!c.tokenBound]));
  const tiers = json("../../deployments/launch/graduation-tiers.json");
  for (const c of ROBINHOOD_COLLECTIONS) assert.deepEqual([c.tierSol, c.floorSol], [tiers.collections[c.slug].tier, Number(tiers.collections[c.slug].floor_sol)], c.slug);
  const unlock = here("../../autobuyer/canister-rs/src/coins/unlock.rs");
  if (!existsSync(unlock)) return;
  const text = readFileSync(unlock, "utf8");
  const order = text.slice(text.indexOf("pub const ORDER"), text.indexOf("];", text.indexOf("pub const ORDER")));
  const entries = [...order.matchAll(/e\("([^"]+)", "([^"]+)"/g)].map((m) => [m[1], m[2]]);
  assert.deepEqual(entries.map(([s]) => s), [...REGISTRY_ORDER], "REGISTRY_ORDER is unlock.rs's ORDER");
  for (const [slug, symbol] of entries) assert.equal(collectionBySlug(slug)?.wxSymbol, symbol, `${slug}: the canister's symbol`);
  assert.match(text, new RegExp(`pub const LAUNCH_COUNT: usize = ${LAUNCH_COUNT};`));
});

test("the launch wizard's list: the canister's pinned collections once read (a Robinhood Chain one once pinned), the 38 otherwise", () => {
  const row = (index: number, symbol: string, stage: string) => ({ index, symbol, stage, wx_mint: "", config: "", module: "", coins: 0, backed: 0 });
  assert.deepEqual(launchableOf(undefined), PINNED_COLLECTIONS);
  assert.deepEqual(launchableOf([]), PINNED_COLLECTIONS, "a canister that pinned nothing yet: the static list");
  const cols = [row(0, "wBOREDAPE", "Open"), row(2, "wMADLAD", "Pinned"), row(4, "wBFROG", "Requested"), row(39, "wCHAINMANC", "Pinned"), row(8, "wDOODLE", 'Refused("config differs")')];
  assert.deepEqual(launchableOf(cols).map((c) => c.slug), ["bayc", "bitcoin-frogs", "mad_lads", "chainmancers"], "the site's order; a refused one left out");
});

// ---- the rest ----------------------------------------------------------------------------------------------------------------------
// The deploy fund (its target, the fund line) and rook_backing's Vault went with the Solana programs: canister-only Rook deploys none
// (docs/v9/canister-only-solana.md §0, §16); src/rook/sol/fund.ts and backing.ts are gone and their tests with them.

test("solText prints base units (lamports, or wX at 9 decimals) as a decimal, trailing zeros dropped", () => {
  assert.equal(solText(9_870_123_456n, 2), "9.87");
  assert.equal(solText(1_000_000_000n), "1");
  assert.equal(solText(2_500_000_000n, 2), "2.5");
  assert.equal(solText(-1_234_000_000n, 3), "−1.234");
});

test("a simulation's failure is said in a line; board helpers; the coin's metadata JSON and its URL checks", () => {
  assert.equal(failureOf({ InstructionError: [4, { Custom: 6005 }] }, ["Program log: AnchorError occurred. Error Code: ExceededSlippage. Error Number: 6005. Error Message: Exceeded slippage tolerance."]), "Exceeded slippage tolerance");
  assert.match(failureOf("AccountNotFound", []), /no SOL/);
  assert.deepEqual(sortNewest([{ pool: "a", launchedAt: 1 }, { pool: "b", launchedAt: 3 }, { pool: "c", launchedAt: 2 }]).map((r) => r.pool), ["b", "c", "a"]);
  assert.equal(gatewayUrl("ipfs://bafy/x.json"), "https://ipfs.io/ipfs/bafy/x.json");
  assert.equal(gatewayUrl("http://insecure"), null);
  assert.deepEqual(candidates("https://ipfs.io/ipfs/QmcbeVBGcefNJQhKXRSpWa7keU1zyDTJ9LzabcSn3WG6Kq"), ["https://ipfs.io/ipfs/QmcbeVBGcefNJQhKXRSpWa7keU1zyDTJ9LzabcSn3WG6Kq", "https://gateway.pinata.cloud/ipfs/QmcbeVBGcefNJQhKXRSpWa7keU1zyDTJ9LzabcSn3WG6Kq", "https://dweb.link/ipfs/QmcbeVBGcefNJQhKXRSpWa7keU1zyDTJ9LzabcSn3WG6Kq"], "an IPFS path falls back to two other gateways");
  assert.deepEqual(candidates("https://res.cloudinary.com/x/raw/upload/a.json"), ["https://res.cloudinary.com/x/raw/upload/a.json"]);
  assert.deepEqual(parseOffChain({ image: "javascript:alert(1)", description: "d", extensions: { website: "https://w.example", twitter: "@x" } }), { image: null, description: "d", website: "https://w.example", twitter: "@x", telegram: null });
  // the metadata JSON itself is built by server.mjs now: test/upload.test.ts holds its shape
  assert.equal(uriProblem("https://res.cloudinary.com/x/raw/upload/v1/a.json"), null);
  assert.match(uriProblem("http://x/a.json")!, /https/);
  assert.match(uriProblem("https://x/" + "a".repeat(200))!, /200/);
});
