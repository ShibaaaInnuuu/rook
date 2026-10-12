/**
 * W12, the site's own two-hop buy (docs/v9/canister-only-solana.md §5.1, §17): src/rook/sol/{dlmm,damm,route}.ts against the bytes
 * the canister's reference (`solix2::route::two_hop_buy`, `solix2::{dlmm,dbc,damm}`) was checked against: instructions that LANDED in
 * LiteSVM on the mainnet program bytes and the accounts those programs wrote (test/fixtures/sol/two-hop.json, copied from
 * autobuyer/canister-rs/tests/fixtures/solix2/{flow,dlmm}.json), and the numbers tests/solix2_dlmm.rs and tests/solix2_builders.rs assert.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { sha256 } from "@noble/hashes/sha2";
import { b64decode } from "../src/rook/sol/base58";
import { NATIVE_MINT, TOKEN_PROGRAM } from "../src/rook/sol/keys";
import { ata, isOnCurve } from "../src/rook/sol/pda";
import { keyBytes } from "../src/rook/sol/keys";
import * as D from "../src/rook/sol/dbc";
import * as L from "../src/rook/sol/dlmm";
import * as M from "../src/rook/sol/damm";
import * as R from "../src/rook/sol/route";
import { withSlippage, type PoolRef } from "../src/rook/sol/trade";
import { compileMessage, txSize, MAX_TX_BYTES, type Instruction } from "../src/rook/sol/tx";

const here = (p: string) => new URL(p, import.meta.url);
const FIX = JSON.parse(readFileSync(here("fixtures/sol/two-hop.json"), "utf8"));
const K = FIX.keys as Record<string, string>;
type FixIx = { name?: string; program: string; accounts: [string, boolean, boolean][]; data: string; params: Record<string, unknown> };
const hex = (s: string) => Uint8Array.from(s.match(/../g)!.map((b) => parseInt(b, 16)));
const fixtureIx = (j: FixIx): Instruction => ({ programId: j.program, keys: j.accounts.map(([pubkey, isSigner, isWritable]) => ({ pubkey, isSigner, isWritable })), data: hex(j.data) });
const ixs = (name: string): FixIx[] => (FIX.instructions as FixIx[]).filter((i) => i.name === name);
const account = (name: string) => { const a = (FIX.accounts as { name: string; owner: string; data: string; expect: Record<string, unknown> }[]).find((a) => a.name === name)!; return { owner: a.owner, data: b64decode(a.data), expect: a.expect }; };
const show = (i: Instruction) => `program ${i.programId}\n${i.keys.map((k) => `  ${k.pubkey} s=${k.isSigner} w=${k.isWritable}`).join("\n")}\ndata ${[...i.data].map((b) => b.toString(16).padStart(2, "0")).join("")}`;
const assertSame = (name: string, built: Instruction, want: Instruction) => {
  assert.equal(built.programId, want.programId, `${name}: program`);
  assert.deepEqual(built.keys, want.keys, `${name}: accounts\n-- built --\n${show(built)}\n-- landed --\n${show(want)}`);
  assert.deepEqual([...built.data], [...want.data], `${name}: data\n-- built --\n${show(built)}\n-- landed --\n${show(want)}`);
};
const disc = (name: string) => [...sha256(new TextEncoder().encode(name)).subarray(0, 8)];
const n = (v: unknown) => BigInt(v as string | number);

const WX = K.wx_mint, COIN = K.coin_mint;
const lb = L.lbDesk(WX);
const curve: PoolRef = { pool: K.dbc_pool, config: K.dbc_config, baseMint: COIN, quoteMint: WX, baseVault: D.tokenVault(COIN, K.dbc_pool), quoteVault: D.tokenVault(WX, K.dbc_pool) };

// ---- DLMM: addresses, the swap2 builder, the price arithmetic ----------------------------------------------------------------------------

test("DLMM: the discriminators are Anchor's, and the desk's pair, preset, position and bin arrays derive as the program did", () => {
  assert.deepEqual([...L.DLMM_DISC.swap2], disc("global:swap2"));
  assert.deepEqual([...L.DLMM_DISC.lbPair], disc("account:LbPair"));
  assert.deepEqual([...L.DLMM_DISC.binArray], disc("account:BinArray"));
  assert.deepEqual([...D.DISC.swap2], disc("global:swap2"), "DBC's swap2 is the same name");
  assert.deepEqual([...M.DAMM_DISC.swap], disc("global:swap"));
  assert.deepEqual([...M.DAMM_DISC.pool], disc("account:Pool"));
  assert.equal(L.presetAddress(L.DESK_PRESET), K.dlmm_preset, "preset 262");
  assert.equal(lb.pair, K.dlmm_pair, "the desk's wX/SOL pair");
  assert.equal(lb.y, NATIVE_MINT);
  const lo = Number(K.lo), hi = Number(K.hi);
  assert.equal(L.positionAddress(K.dlmm_pair, K.k_desk, lo, hi - lo + 1), K.dlmm_position);
  assert.deepEqual([-1, 0, 69, 70, -70, -71].map(L.arrayIndex), [-1, 0, 0, 1, -1, -2], "a negative bin lies in a negative array");
  assert.deepEqual(L.swapArrays(K.dlmm_pair, 586, true, 2), [L.binArrayAddress(K.dlmm_pair, 8), L.binArrayAddress(K.dlmm_pair, 9)], "a buy walks up from the active bin's array");
  assert.deepEqual(L.swapArrays(K.dlmm_pair, 586, false, 2), [L.binArrayAddress(K.dlmm_pair, 8), L.binArrayAddress(K.dlmm_pair, 7)]);
  assert.equal(L.binArrayAddress(K.dlmm_pair, 8), (ixs("dlmm.swap2")[0].params.arrays as string[])[0], "the array the landed swap walked");
  assert.ok(!isOnCurve(keyBytes(K.dlmm_pair)) && !isOnCurve(keyBytes(K.dlmm_position)) && isOnCurve(keyBytes(K.k_desk)));
});

test("DLMM swap2: the site's builder IS each landed swap2 (a buy, a sell, the desk's own), byte for byte", () => {
  const landed = ixs("dlmm.swap2");
  assert.equal(landed.length, 3);
  for (const j of landed) {
    const p = j.params as { pair: string; x: string; y: string; user: string; input: string; output: string; amount_in: number; min_out: number; arrays: string[] };
    const mine = L.swap2({ pair: p.pair, x: p.x, y: p.y, binStep: L.DESK_BIN_STEP }, p.user, p.input, p.output, n(p.amount_in), n(p.min_out), p.arrays);
    assertSame("dlmm.swap2", mine, fixtureIx(j));
    assert.equal(mine.keys.length, 17, "16 accounts + one bin array");
    assert.equal(mine.data.length, 8 + 8 + 8 + 4, "disc, amount_in, min_out, an empty RemainingAccountsInfo");
  }
});

test("DLMM prices: bin 0 is 1, each bin is 1.005× the last, DLMM's own bin prices are reproduced to 1e-12, and 18.61 SOL is bin 586", () => {
  const ONE = 1n << 64n;
  assert.equal(L.priceQ64(50, 0), ONE);
  assert.ok(Math.abs(Number(L.priceQ64(50, 1)!) / 2 ** 64 - 1.005) < 1e-12);
  assert.ok(Math.abs(Number(L.priceQ64(50, -1)!) / 2 ** 64 - 1 / 1.005) < 1e-12);
  assert.ok(Math.abs(Number(L.priceQ64(50, 586)!) / 2 ** 64 / 1.005 ** 586 - 1) < 1e-9);
  for (const id of [-5000, -1, 0, 1, 586, 1200]) {
    const p = L.priceQ64(50, id)!;
    assert.equal(L.binForPrice(50, p), id);
    assert.equal(L.binForPrice(50, p + p / 100_000n), id);
    assert.equal(L.binForPrice(50, p - p / 100_000n), id);
  }
  assert.equal(L.binForPrice(50, L.priceOfFloor(18_610_000_000n)), 586, "Milady's floor at step 50");
  assert.equal(L.binForPrice(50, 0n), null);
  // the bins the program wrote carry its own Q64.64 prices: the port agrees to within the last few units of a 2^-64 fraction
  const arr = L.decodeBinArray(account("bin_array_8").data);
  assert.equal(arr.index, 8);
  assert.equal(arr.lbPair, K.dlmm_pair);
  assert.equal(arr.bins.length, 70);
  let checked = 0;
  for (const [id, b] of L.binsOf([arr])) {
    const mine = L.priceQ64(50, id)!;
    const diff = mine > b.price ? mine - b.price : b.price - mine;
    assert.ok(diff <= b.price / 1_000_000_000_000n, `bin ${id}: mine ${mine}, DLMM's ${b.price}`);
    checked++;
  }
  assert.equal(checked, 70);
  assert.ok(Math.abs(L.solPerWxAt(50, 586)! - 18.61) < 0.05, "the active bin's price is the floor it was opened at");
});

test("DLMM LbPair: decodes as the program wrote it, and the fee rates are DLMM's (0.4 % base; the variable fee by the volatility accumulator)", () => {
  for (const name of ["lb_pair_after_add", "lb_pair_rebalance_past"]) {
    const { data, expect } = account(name);
    const p = L.decodeLbPair(data);
    assert.equal(p.activeId, expect.active_id, name);
    assert.equal(p.binStep, expect.bin_step);
    assert.equal(p.status, expect.status);
    assert.equal(p.pairType, expect.pair_type);
    assert.equal(p.tokenXMint, expect.token_x_mint);
    assert.equal(p.tokenYMint, expect.token_y_mint);
    assert.equal(p.reserveX, expect.reserve_x);
    assert.equal(p.reserveY, expect.reserve_y);
    assert.equal(p.volatilityAccumulator, expect.volatility_accumulator);
    assert.ok(L.pairEnabled(p));
    assert.equal(p.tokenXMint, WX, "x = wX, y = wSOL: the price is SOL per wX");
    assert.equal(p.tokenYMint, NATIVE_MINT);
    assert.deepEqual([p.baseFactor, p.filterPeriod, p.decayPeriod, p.reductionFactor, p.variableFeeControl, p.maxVolatilityAccumulator], [8_000, 120, 1_200, 5_000, 10_000, 250_000], "preset 262");
    assert.equal(L.baseFeeRate(p), 4_000_000n, "0.4 %");
    assert.equal(L.variableFeeRate(p, 0), 0n);
    assert.equal(L.variableFeeRate(p, 10_000), 25_000n, "one bin of volatility: 10,000 · (10,000 · 50)² / 1e11");
    assert.equal(L.variableFeeRate(p, 250_000), 15_625_000n, "the accumulator's cap: 1.5625 %");
    assert.equal(L.feeRate(p, 250_000), 19_625_000n);
  }
  const p = L.decodeLbPair(account("lb_pair_after_add").data);
  const q = { ...p, volatilityAccumulator: 100_000, volatilityReference: 40_000, lastUpdateTimestamp: 1_000, activeId: 600 };
  assert.deepEqual(L.referencesAt(q, 1_060), { vol: 40_000, idx: 586 }, "inside the filter period nothing moves");
  assert.deepEqual(L.referencesAt(q, 1_200), { vol: 50_000, idx: 600 }, "past the filter period: 100,000 × 5,000 / 10,000");
  assert.deepEqual(L.referencesAt(q, 2_300), { vol: 0, idx: 600 }, "past the decay period: 0");
  assert.equal(L.currentFeeRate(p, 1_000_000), 4_000_000n, "a quiet pair: the base fee");
  assert.throws(() => L.decodeLbPair(data(903)), /not a DLMM LbPair/);
  assert.throws(() => L.decodeLbPair(account("bin_array_8").data), /not a DLMM LbPair/);
  assert.throws(() => L.decodeBinArray(account("lb_pair_after_add").data), /not a DLMM BinArray/);
  function data(len: number) { return account("lb_pair_after_add").data.subarray(0, len); }
});

test("DLMM quote: the bin walk quotes the first 0.5 SOL buy the chain filled (26,652,637 wX base units) to within 0.05 %, and flags what the arrays cannot fill", () => {
  const pair = L.decodeLbPair(account("lb_pair_after_add").data);
  const bins = L.binsOf([L.decodeBinArray(account("bin_array_8").data)]);
  const buy = ixs("dlmm.swap2")[0].params as { amount_in: number; got: number };
  const amountIn = n(buy.amount_in), got = n(buy.got);
  assert.equal(got, 26_652_637n);
  const q = L.quoteDlmmSwap(pair, bins, false, amountIn, 1_000_000);
  assert.ok(q.complete);
  const diff = q.amountOut > got ? q.amountOut - got : got - q.amountOut;
  assert.ok(diff * 2_000n <= got, `quoted ${q.amountOut}, the chain filled ${got}`);
  assert.equal(q.amountInUsed, amountIn);
  assert.ok(q.endId > pair.activeId, "a buy walks up from the active bin");
  assert.ok(q.fee > (amountIn * 35n) / 10_000n && q.fee < (amountIn * 50n) / 10_000n, `the fee on a 0.5 SOL buy is about 0.4 %: ${q.fee}`);
  // the price paid: ≈ 18.76 SOL per wX against the 18.61 floor (+0.8 %: spike S4 measured +0.81 % on 0.1–0.5 SOL)
  const paid = Number(amountIn) / Number(q.amountOut);
  assert.ok(paid > 18.61 && paid < 18.61 * 1.012, `${paid} SOL per wX`);
  const sell = L.quoteDlmmSwap(pair, bins, true, got, 1_000_000);
  assert.ok(sell.amountOut > 0n && sell.endId <= pair.activeId, "a sell walks down and takes the bid bins");
  assert.ok(sell.amountOut < amountIn && sell.amountOut > (amountIn * 98n) / 100n, "a round trip loses the two fees and the spread, ≈ 1 %");
  const big = L.quoteDlmmSwap(pair, bins, false, 100n * 1_000_000_000n, 1_000_000);
  assert.ok(!big.complete && big.amountInUsed < 100n * 1_000_000_000n, "more than the arrays show: partial, flagged");
  const empty = L.quoteDlmmSwap(pair, new Map(), false, amountIn, 1_000_000);
  assert.ok(!empty.complete && empty.amountOut === 0n && empty.amountInUsed === 0n, "no arrays: nothing");
});

// ---- the coin hop: DBC swap2 and DAMM v2 swap ---------------------------------------------------------------------------------------------

test("DBC swap2 and DAMM v2 swap: the site's builders are the landed instructions (the mode-1 fill, the pool swaps), byte for byte; the migrated pool derives", () => {
  const sw2 = ixs("dbc.swap2")[0], p = sw2.params as Record<string, string | number>;
  const mine = D.swap2({ ...curve, user: String(p.payer), input: String(p.input), output: String(p.output), amount0: n(p.amount_0), amount1: n(p.amount_1), swapMode: Number(p.swap_mode) as 0 | 1 | 2 });
  assertSame("dbc.swap2", mine, fixtureIx(sw2));
  assert.equal(D.poolAddress(K.dbc_config, COIN, WX), K.dbc_pool, "the curve's address with wX as the quote");
  assert.equal(M.migratedPool(COIN, WX).pool, K.migrated_pool, "the DAMM v2 pool DBC migrates into");
  assert.equal(M.customizablePoolAddress(WX, NATIVE_MINT), K.wx_sol_pool, "the one customizable pool of a pair (S1's wX/SOL pool)");
  assert.equal(M.dammEventAuthority(), K.damm_event_authority);
  assert.equal(D.dbcEventAuthority(), K.dbc_event_authority);
  for (const j of ixs("damm.swap")) {
    const q = j.params as Record<string, string | number>;
    const ref = M.dammPoolAt(String(q.pool), String(q.token_a_mint), String(q.token_b_mint));
    assertSame("damm.swap", M.dammSwap(ref, String(q.payer), String(q.input), String(q.output), n(q.amount_in), n(q.minimum_amount_out)), fixtureIx(j));
  }
  const { data, expect } = account("migrated_pool");
  const pool = M.decodeDammPool(data);
  assert.deepEqual([pool.tokenAMint, pool.tokenBMint, pool.tokenAVault, pool.tokenBVault], [expect.token_a_mint, expect.token_b_mint, expect.token_a_vault, expect.token_b_vault]);
  assert.equal(pool.collectFeeMode, expect.collect_fee_mode);
  assert.equal(pool.collectFeeMode, M.COLLECT_FEE_ONLY_B, "fees in wX (token b) only");
  assert.ok(pool.permanentLockLiquidity > 0n, "the LP is permanently locked");
  assert.equal(pool.feeNumerator, 10_000_000n, "the migrated pool's fee: 1 %");
  assert.equal(pool.dynamicFeeInitialized, 0);
  assert.ok(pool.liquidity > 0n && pool.sqrtPrice >= pool.sqrtMinPrice && pool.sqrtPrice <= pool.sqrtMaxPrice);
  const ref = M.migratedPool(COIN, WX);
  assert.deepEqual([ref.vaultA, ref.vaultB], [pool.tokenAVault, pool.tokenBVault], "the vaults the builder addresses are the pool's");
  assert.throws(() => M.decodeDammPool(account("dbc_config").data), /not a DAMM v2 Pool/);
});

test("the coin hop's quotes: the T_wX = 5 curve raises exactly 5 wX to its first point (the canister's number); DAMM v2 takes the 1 % on the wX in and moves √P as a constant product", () => {
  const cfg = D.decodeConfig(account("dbc_config").data), pool = D.decodePool(account("dbc_pool_graduated").data);
  assert.equal(cfg.quoteMint, WX);
  assert.equal(cfg.migrationQuoteThreshold, 5_000_000_000n, "T_wX = 5 whole wX");
  // guard::quote_raised(start, curve, curve[0].sqrt_price) is 5 wX to within a base unit (tests/solix2_builders.rs)
  const raised = D.deltaQuote(cfg.sqrtStartPrice, cfg.curve[0].sqrtPrice, cfg.curve[0].liquidity, false);
  const gap = raised > 5_000_000_000n ? raised - 5_000_000_000n : 5_000_000_000n - raised;
  assert.ok(gap <= 1n, `${raised}`);
  assert.equal(pool.quoteReserve, 5_000_000_001n, "the graduated pool holds the threshold");
  // a quote on the curve before any buy: 5,212,251 wX base units (the bundle's wx_min) buys coins, fee on the wX in at the anti-sniper rate
  const fresh = { ...pool, sqrtPrice: cfg.sqrtStartPrice };
  const q = D.quoteSwap(cfg, fresh, "buy", 5_212_251n, pool.activationPoint + 601n);
  assert.ok(q && q.amountOut > 0n && q.fee > 0n);
  assert.equal(q!.feeNumerator, 10_000_040n, "1.000004 % after ten minutes");
  // the config the canister creates turns the first swap at the minimum fee on (byte 365): a launch's first buy at t0 pays 1.000004 %,
  // any other buy at t0 the anti-sniper 50 % (proven on DBC's mainnet bytes: solana/tests/first_swap_min_fee.rs)
  assert.equal(cfg.firstSwapMinFee, true);
  assert.equal(D.minBaseFeeNumerator(cfg), 10_000_040n);
  const t0 = pool.activationPoint;
  assert.equal(D.quoteSwap(cfg, fresh, "buy", 1_000_000_000n, t0, true)!.fee, 10_000_040n, "the creator's launch buy");
  assert.equal(D.quoteSwap(cfg, fresh, "buy", 1_000_000_000n, t0)!.fee, 500_000_000n, "everyone else at launch");
  assert.equal(D.quoteSwap({ ...cfg, firstSwapMinFee: false }, fresh, "buy", 1_000_000_000n, t0, true)!.fee, 500_000_000n, "a config without the flag");
  // DAMM v2: the fee on the input (fees in b only, b in), √P up by (in − fee) · 2^128 / L, the coins out by the constant product
  const dp = M.decodeDammPool(account("migrated_pool").data);
  const wxIn = 500_000_000n, dq = M.quoteDammSwap(dp, "buy", wxIn)!;
  assert.ok(dq);
  assert.equal(dq.fee, (wxIn * 10_000_000n + 999_999_999n) / 1_000_000_000n, "1 % of the wX in, rounded up");
  assert.equal(dq.nextSqrtPrice, dp.sqrtPrice + ((wxIn - dq.fee) << 128n) / dp.liquidity);
  assert.equal(dq.amountOut, D.deltaBase(dp.sqrtPrice, dq.nextSqrtPrice, dp.liquidity, false));
  assert.ok(dq.amountOut > 0n);
  // the price paid is above the pool's spot price (the fee and the impact: 0.5 wX is a fifth of this thin pool's ≈ 2.5 wX side), never below
  const spot = M.dammPriceRaw(dp), paid = Number(wxIn) / Number(dq.amountOut);
  assert.ok(paid > spot * 1.01 && paid < spot * 1.3, `${paid} vs spot ${spot}`);
  const tiny = M.quoteDammSwap(dp, "buy", 1_000_000n)!;
  assert.ok(Number(1_000_000n) / Number(tiny.amountOut) < spot * 1.011, "a 0.001 wX buy pays the fee and almost no impact");
  // a sell takes its fee on the wX out and moves √P down; selling the coins back from the post-buy price lands within a unit of the
  // starting price and returns the wX in less the two 1 % fees (the inverse of the constant product, rounded the program's way)
  const after = { ...dp, sqrtPrice: dq.nextSqrtPrice };
  const sq = M.quoteDammSwap(after, "sell", dq.amountOut)!;
  assert.ok(sq && sq.nextSqrtPrice < after.sqrtPrice, "√P moves down");
  assert.ok(sq.nextSqrtPrice >= dp.sqrtPrice && sq.nextSqrtPrice - dp.sqrtPrice < 1_000n, `back to the start: ${sq.nextSqrtPrice} vs ${dp.sqrtPrice}`);
  assert.equal(sq.fee, (sq.amountOut + sq.fee) / 100n + ((sq.amountOut + sq.fee) % 100n === 0n ? 0n : 1n), "1 % of the wX out, rounded up");
  assert.ok(sq.amountOut + sq.fee <= wxIn - dq.fee && sq.amountOut + sq.fee >= wxIn - dq.fee - 2n, `${sq.amountOut + sq.fee} of ${wxIn - dq.fee}`);
  // the same coins sold at the pool's price before the buy fetch much less: this pool is thin (≈ 2.7 wX a side)
  assert.ok(M.quoteDammSwap(dp, "sell", dq.amountOut)!.amountOut < (wxIn * 80n) / 100n);
  assert.equal(M.quoteDammSwap(dp, "buy", 0n), null);
  assert.equal(M.quoteDammSwap({ ...dp, sqrtMaxPrice: dp.sqrtPrice }, "buy", 1_000_000n), null, "a buy past the pool's range");
});

// ---- the route ------------------------------------------------------------------------------------------------------------------------

test("the two-hop buy: every instruction but the first hop is the landed bundle's (both bundles), the first hop is DLMM swap2, one signer pays, and it fits a legacy transaction", () => {
  const sizes: Record<string, number> = {};
  for (const b of FIX.bundles as { name: string; params: Record<string, string | number>; instructions: FixIx[] }[]) {
    const p = b.params, user = String(p.user), pre = b.name === "site.two_hop_pre_graduation";
    const hop: R.CoinHop = pre ? { kind: "curve", curve } : { kind: "graduated", pool: M.migratedPool(COIN, WX) };
    const arrays = L.swapArrays(lb.pair, 586, true, 1);
    const route = R.twoHopBuyIxs(user, lb, hop, n(p.sol_in), n(p.wx_min), n(p.coin_min), arrays);
    const full = [D.setComputeUnitLimit(Number(p.cu_limit)), D.setComputeUnitPrice(n(p.cu_price)), ...route];
    const want = b.instructions.map(fixtureIx);
    assert.equal(full.length, want.length, "ten instructions");
    for (let i = 0; i < full.length; i++) {
      if (i === 6) continue; // hop 1: the landed bundle swapped on the DAMM v2 pool of the time; §17 moved the wX/SOL pool to DLMM
      assertSame(`${b.name} #${i}`, full[i], want[i]);
    }
    const hop1 = full[6];
    assert.equal(hop1.programId, L.DLMM_PROGRAM);
    assert.deepEqual([...hop1.data], [...L.DLMM_DISC.swap2, ...D.transferLamports(user, user, n(p.sol_in)).data.subarray(4), ...D.transferLamports(user, user, n(p.wx_min)).data.subarray(4), 0, 0, 0, 0], "swap2 { sol_in, wx_min, no slices }");
    assert.equal(hop1.keys[4].pubkey, ata(user, NATIVE_MINT), "the wSOL account in");
    assert.equal(hop1.keys[5].pubkey, ata(user, WX), "the wX account out");
    assert.equal(hop1.keys[16].pubkey, arrays[0], "the active bin's array follows");
    // the coin hop spends exactly wx_min: the same amount the first hop floors
    assert.deepEqual([...full[8].data.subarray(8, 16)], [...hop1.data.subarray(16, 24)]);
    const m = compileMessage(user, full, "11111111111111111111111111111111");
    assert.equal(m.header.numRequiredSignatures, 1, "the wallet alone signs");
    assert.equal(m.accountKeys[0], user, "and pays");
    sizes[b.name] = txSize(m);
    assert.ok(sizes[b.name] <= MAX_TX_BYTES, `${b.name}: ${sizes[b.name]} bytes`);
    // two bin arrays (a buy that crosses into the next array) still fit
    const two = [D.setComputeUnitLimit(400_000), D.setComputeUnitPrice(10_000n), ...R.twoHopBuyIxs(user, lb, hop, n(p.sol_in), n(p.wx_min), n(p.coin_min), L.swapArrays(lb.pair, 586, true, 2))];
    assert.ok(txSize(compileMessage(user, two, "11111111111111111111111111111111")) <= MAX_TX_BYTES);
    // without the unwrap (the simulation that reads the coin account), one instruction fewer
    assert.equal(R.twoHopBuyIxs(user, lb, hop, 1n, 1n, 1n, arrays, false).length, route.length - 1);
  }
  // spikes §3.5 measured 1,033 B on the curve and 903 B graduated with the DAMM v2 first hop; the DLMM hop names the pair, two
  // reserves, the oracle, the memo program, DLMM's event authority and one bin array in place of the DAMM pool's accounts
  assert.ok(sizes["site.two_hop_pre_graduation"] > sizes["site.two_hop_post_graduation"]);
  assert.ok(sizes["site.two_hop_pre_graduation"] >= 1_033 && sizes["site.two_hop_pre_graduation"] <= 1_232, `${sizes["site.two_hop_pre_graduation"]}`);
});

test("the one-hop buy, the two-hop sell: shapes, accounts and the amounts that bind the hops", () => {
  const user = "7EWrbxU7YpHthanStG9yF6KyHS77LBPH6f52ANJmL9rs";
  const hop: R.CoinHop = { kind: "curve", curve };
  const one = R.wxBuyIxs(user, hop, 7n, 3n);
  assert.deepEqual(one.map((i) => i.programId), ["ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL", D.DBC_PROGRAM]);
  assert.equal(one[1].keys[3].pubkey, ata(user, WX), "wX in");
  assert.equal(one[1].keys[4].pubkey, ata(user, COIN), "coin out");
  assert.equal(one[1].keys[8].pubkey, WX, "the quote mint account is wX, not wSOL");
  const arrays = L.swapArrays(lb.pair, 586, false, 2);
  const sell = R.twoHopSellIxs(user, lb, hop, 1_000_000n, 5_000n, 90n, arrays);
  assert.deepEqual(sell.map((i) => i.programId), ["ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL", D.DBC_PROGRAM, "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL", L.DLMM_PROGRAM, TOKEN_PROGRAM]);
  assert.deepEqual([...sell[1].data], [...D.DISC.swap, ...D.transferLamports(user, user, 1_000_000n).data.subarray(4), ...D.transferLamports(user, user, 5_000n).data.subarray(4)], "the curve: exactly the coins in, at least the wX the pair will take");
  assert.deepEqual([...sell[3].data.subarray(8, 24)], [...D.transferLamports(user, user, 5_000n).data.subarray(4), ...D.transferLamports(user, user, 90n).data.subarray(4)], "the pair: exactly that wX in, at least the SOL floor");
  assert.equal(sell[3].keys[4].pubkey, ata(user, WX), "wX in");
  assert.equal(sell[3].keys[5].pubkey, ata(user, NATIVE_MINT), "wSOL out, then closed to SOL");
  assert.equal(sell[3].keys[17].pubkey, arrays[1], "the arrays walk down");
  assert.equal(sell[4].keys[0].pubkey, ata(user, NATIVE_MINT));
  const grad: R.CoinHop = { kind: "graduated", pool: M.migratedPool(COIN, WX) };
  assert.equal(R.coinHopIx(user, grad, 5n, 1n).programId, M.DAMM_PROGRAM);
  assert.equal(R.coinSellIx(user, grad, 5n, 1n).keys[2].pubkey, ata(user, COIN), "a graduated sell puts the coins in");
});

test("the quotes bind the hops with the slippage: the coin hop spends exactly the first hop's floor, each floor is its quote less the slippage, and what cannot fill is null", () => {
  const pair = L.decodeLbPair(account("lb_pair_after_add").data);
  const bins = L.binsOf([L.decodeBinArray(account("bin_array_8").data)]);
  const cfg = D.decodeConfig(account("dbc_config").data), pool = D.decodePool(account("dbc_pool_graduated").data);
  const fresh = { ...pool, sqrtPrice: cfg.sqrtStartPrice };
  const now = Number(pool.activationPoint) + 601;
  const w = { pair, bins };
  const q = R.quoteTwoHopBuy(w, { kind: "curve", cfg, pool: fresh }, 100_000_000n, 100, now)!;
  assert.ok(q);
  assert.equal(q.wxMin, withSlippage(q.wxOut, 100));
  assert.equal(q.coinMin, withSlippage(q.coinOut, 100));
  assert.ok(q.wxOut > q.wxMin && q.coinOut > q.coinMin);
  assert.ok(q.wxOut > 5_000_000n && q.wxOut < 5_500_000n, `0.1 SOL at ≈ 18.7 SOL per wX ≈ 5.3M base units (the bundle's wx_min was 5,212,251): ${q.wxOut}`);
  assert.ok(q.dlmmRate >= 4_000_000n && q.dlmmRate <= 4_025_000n, `a quiet pair: 0.4 % plus at most one bin of volatility (0.0025 %): ${q.dlmmRate}`);
  assert.equal(q.coinFeeNumerator, 10_000_040n);
  // the coin hop quotes wxMin, not wxOut: more slippage on hop 1 means fewer coins quoted on hop 2
  const loose = R.quoteTwoHopBuy(w, { kind: "curve", cfg, pool: fresh }, 100_000_000n, 1_000, now)!;
  assert.ok(loose.wxMin < q.wxMin && loose.coinOut < q.coinOut);
  const none = R.quoteTwoHopBuy(w, { kind: "curve", cfg, pool: fresh }, 100_000_000n, 0, now)!;
  assert.equal(none.wxMin, none.wxOut, "no slippage: the floor is the quote");
  assert.equal(R.quoteTwoHopBuy(w, { kind: "curve", cfg, pool: fresh }, 0n, 100, now), null);
  assert.equal(R.quoteTwoHopBuy(w, { kind: "curve", cfg, pool: fresh }, 100n * 1_000_000_000n, 100, now), null, "the pair's visible depth cannot fill 100 SOL");
  assert.equal(R.quoteTwoHopBuy({ pair, bins: new Map() }, { kind: "curve", cfg, pool: fresh }, 100_000_000n, 100, now), null, "no bin arrays read: no quote");
  // graduated: the DAMM v2 hop
  const dp = M.decodeDammPool(account("migrated_pool").data);
  const g = R.quoteTwoHopBuy(w, { kind: "graduated", pool: dp }, 100_000_000n, 100, now)!;
  assert.ok(g && g.coinOut > 0n && g.coinFeeNumerator === 10_000_000n && g.wxMin === q.wxMin);
  // the sell: hop 1's floor is what hop 2 spends; hop 2's floor is the SOL out less the slippage
  const s = R.quoteTwoHopSell(w, { kind: "curve", cfg, pool }, 1_000_000_000n, 100, now)!;
  assert.ok(s && s.wxOut > 0n && s.solOut > 0n);
  assert.equal(s.wxMin, withSlippage(s.wxOut, 100));
  assert.equal(s.solMin, withSlippage(s.solOut, 100));
  const paidPerWx = Number(s.solOut) / Number(s.wxMin);
  assert.ok(paidPerWx < 18.61 && paidPerWx > 18.61 * 0.985, `a sell gets a hair under the floor: ${paidPerWx}`);
  assert.equal(R.quoteTwoHopSell(w, { kind: "curve", cfg, pool }, 0n, 100, now), null);
  // the one-hop buy
  const one = R.quoteWxBuy({ kind: "curve", cfg, pool: fresh }, q.wxMin, 100, now)!;
  assert.deepEqual([one.coinOut, one.coinMin, one.feeWx], [q.coinOut, q.coinMin, q.feeWx], "the same coin hop");
  assert.equal(R.quoteWxBuy({ kind: "curve", cfg, pool: fresh }, 0n, 100, now), null);
  // slippage bounds: 0 ≤ bps, clamped at 50 %
  assert.equal(withSlippage(10_000n, 0), 10_000n);
  assert.equal(withSlippage(10_000n, -5), 10_000n);
  assert.equal(withSlippage(10_000n, 100_000), 5_000n);
});

test("the pair's price view: SOL per wX at the active bin, and the arrays a read hands the swap", () => {
  const pair = L.decodeLbPair(account("lb_pair_after_add").data);
  assert.ok(Math.abs(R.wxPriceSol({ pair })! - 18.61) < 0.05);
  assert.equal(R.wxPriceSol(null), null);
  const past = L.decodeLbPair(account("lb_pair_rebalance_past").data);
  assert.equal(past.activeId, 614);
  assert.ok(R.wxPriceSol({ pair: past })! > 21 && R.wxPriceSol({ pair: past })! < 22, "28 bins up: ≈ 1.005^28 × 18.61 ≈ 21.4 SOL");
  assert.equal(R.ARRAYS_A_SIDE, 2);
});
