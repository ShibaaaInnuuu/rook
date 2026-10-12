// Decoding against recorded mainnet transactions (test/fixtures/sol/*.json: getTransaction jsonParsed, recorded 2026-10-10 with
// scripts/sol-explore.ts). The pools are other launchpads' DBC pools: the decoder does not care whose config it is.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeTx, type ParsedTx } from "../src/sol/decode.js";
import { priceFromSqrt, type DammSwapEvent, type DbcSwapEvent } from "../src/sol/dbc.js";
import { dammTrade, dbcTrade, ordOf, cursorOf, ordFromCursor } from "../src/sol/trades.js";

const fx = (name: string) => JSON.parse(readFileSync(new URL(`./fixtures/sol/${name}.json`, import.meta.url), "utf8")) as ParsedTx;
const near = (a: number, b: number, rel = 0.03) => assert.ok(Math.abs(a - b) / b < rel, `${a} vs ${b}`);

test("a DBC sell that emits both EvtSwap and EvtSwap2 is ONE trade (the v2 event), fee added back to the SOL side", () => {
  const d = decodeTx(fx("dbc-swap-45hYJ2Vc5o"));
  assert.equal(d.ok, true);
  assert.equal(d.swaps.length, 1);
  const ev = d.swaps[0].ev as DbcSwapEvent;
  assert.equal(ev.kind, "dbc-swap"); assert.equal(ev.version, 2); assert.equal(ev.direction, 0);
  assert.equal(ev.pool, "87fubRzjsnHW8MKp2sAJMWwBKyJuVfJpQe2knhGKZuUJ"); assert.equal(ev.config, "D1P6ADtf6sgWUcJBdesMsjv9YahaLJuHnN7HRDWYVus");
  assert.equal(ev.inputAmount, 109477369029n); assert.equal(ev.outputAmount, 3466899n);
  assert.equal(ev.tradingFee, 145976n); assert.equal(ev.protocolFee, 36493n);
  assert.equal(d.swaps[0].trader, "BUQaUYtAmdm5bSUwqVkSsWCt4R53eQ3cYy2D6nT4r2gA");
  // the seller's lamport balance moved by output − the network fee: output_amount is what the seller got
  const t = dbcTrade(ev, 6, 0)!;
  assert.equal(t.side, "sell"); assert.equal(t.sol, 3466899n); assert.equal(t.tokens, 109477369029n); assert.equal(t.fee, 182469n);
  assert.equal(t.price, (3466899 + 182469) / 1e9 / (109477369029 / 1e6));
  near(t.price, priceFromSqrt(ev.nextSqrtPrice, 6));
});

test("a DBC buy on an output-fee config: the fee is in coins, so the price divides the SOL by the coins before the fee", () => {
  const d = decodeTx(fx("dbc-swap-2NN6rBsMCB"));
  assert.equal(d.swaps.length, 1);
  const ev = d.swaps[0].ev as DbcSwapEvent;
  assert.equal(ev.direction, 1); assert.equal(ev.inputAmount, 110000000n); assert.equal(ev.inputExcludedFee, 110000000n);
  const t = dbcTrade(ev, 6, 1)!;
  assert.equal(t.side, "buy"); assert.equal(t.sol, 110000000n); assert.equal(t.tokens, 11099694458n); assert.equal(t.fee, null);
  assert.equal(t.price, 0.11 / ((11099694458 + 89694501 + 22423625) / 1e6));
  near(t.price, priceFromSqrt(ev.nextSqrtPrice, 6));
});

test("a DBC sell on an output-fee config", () => {
  const d = decodeTx(fx("dbc-swap-PMoADv323G"));
  const t = dbcTrade(d.swaps[0].ev as DbcSwapEvent, 6, 1)!;
  assert.equal(t.side, "sell"); assert.equal(t.sol, 304045470n); assert.equal(t.tokens, 31285002825n);
  near(t.price, priceFromSqrt((d.swaps[0].ev as DbcSwapEvent).nextSqrtPrice, 6));
});

test("a creation with a first buy that completes the curve: init (name, symbol, uri from the instruction), the buy, the completion", () => {
  const d = decodeTx(fx("dbc-swap-5LAUFFX9zK"));
  assert.equal(d.inits.length, 1);
  const i = d.inits[0];
  assert.equal(i.ev.pool, "GYJrkcixUR1o4wG96JYUo2vQdjskohkqgZewM1arBDNL"); assert.equal(i.ev.baseMint, "8gTju7eND1D8pm52664toFE6gHZKMSGA8jnM9PvQoJkp");
  assert.equal(i.ev.creator, "8HMn6b9Vt9mp2kRGhxgvJMDTaZLPMzVdpKqsbfYLaFPM"); assert.equal(i.ev.config, "9q4yEJsVqUQ4SQp24o8moARNrBFFCm1vZDKLo3KPkqTe");
  assert.equal(i.name, "Goon Index 6900"); assert.equal(i.symbol, "GI6900"); assert.equal(i.uri, "https://metadata.j7tracker.io/metadata/mjs4DjusjX.json");
  assert.equal(d.swaps.length, 1);
  const ev = d.swaps[0].ev as DbcSwapEvent;
  assert.equal(ev.direction, 1); assert.equal(ev.quoteReserve, ev.migrationThreshold);
  assert.equal(d.completes.length, 1); assert.equal(d.completes[0].pool, i.ev.pool);
  assert.equal(d.swaps[0].ix?.accounts[7], i.ev.baseMint, "the swap's account 7 is the base mint: a coin can be registered from its first swap");
  assert.equal(d.swaps[0].ix?.accounts[1], i.ev.config, "DBC's swap takes the config as account 1: getSignaturesForAddress(config) sees every swap");
});

test("a migration to DAMM v2 names the new pool (account 4) and the coin (account 13)", () => {
  const d = decodeTx(fx("migrate-damm-v2-FQgKq1qWYL"));
  assert.deepEqual(d.migrations, [{ virtualPool: "GYJrkcixUR1o4wG96JYUo2vQdjskohkqgZewM1arBDNL", config: "9q4yEJsVqUQ4SQp24o8moARNrBFFCm1vZDKLo3KPkqTe",
    dammPool: "F9xaKn1zZCaa1NTiC96VKTfmjnd6Vx2e1FrJqbedVm92", baseMint: "8gTju7eND1D8pm52664toFE6gHZKMSGA8jnM9PvQoJkp", version: "damm-v2" }]);
  assert.equal(d.swaps.length, 0);
});

test("a DAMM v2 swap after graduation: B→A with the coin as token A is a buy, the fee on the SOL input", () => {
  const d = decodeTx(fx("damm-swap-23RE4dZAqF"));
  assert.equal(d.swaps.length, 1);
  const ev = d.swaps[0].ev as DammSwapEvent;
  assert.equal(ev.kind, "damm-swap"); assert.equal(ev.pool, "F9xaKn1zZCaa1NTiC96VKTfmjnd6Vx2e1FrJqbedVm92"); assert.equal(ev.direction, 1);
  assert.equal(ev.inputAmount, 100000n); assert.equal(ev.inputExcludedFee, 99000n); assert.equal(ev.outputAmount, 1719709958n);
  const t = dammTrade(ev, 6, true)!;
  assert.equal(t.side, "buy"); assert.equal(t.venue, "pool"); assert.equal(t.sol, 100000n); assert.equal(t.fee, 1000n);
  assert.equal(t.price, 99000 / 1e9 / (1719709958 / 1e6));
  // the same event read with the coin as token B would be a sell: the side really does come from the pool's token order
  assert.equal(dammTrade(ev, 6, false)!.side, "sell");
  near(t.price, priceFromSqrt(ev.nextSqrtPrice, 6), 0.05);
});

test("a failed transaction yields nothing", () => {
  const tx = fx("dbc-swap-45hYJ2Vc5o");
  const d = decodeTx({ ...tx, meta: { ...tx.meta!, err: { InstructionError: [0, "Custom"] } } });
  assert.equal(d.ok, false); assert.equal(d.swaps.length, 0);
});

test("trade order and the trades cursor round-trip", () => {
  const o = ordOf(455165963, 424, 1);
  assert.equal(o, 455165963 * 1_000_000 + 42_401);
  assert.equal(cursorOf(o), "455165963:42401");
  assert.equal(ordFromCursor(cursorOf(o)), o);
  assert.equal(ordFromCursor("1:9999999"), null);
  assert.ok(ordOf(10, 0, 5) < ordOf(10, 1, 0) && ordOf(10, 9999, 99) < ordOf(11, 0, 0));
});

test("a DBC buy on a quote-fee config (Rook's own collect_fee_mode 0): the fee comes off the SOL input", () => {
  const d = decodeTx(fx("mode0-buy-5HQQ9hLVAy"));
  const ev = d.swaps[0].ev as DbcSwapEvent;
  assert.equal(ev.direction, 1); assert.equal(ev.inputAmount, 2635022n); assert.equal(ev.inputExcludedFee, 2621846n);
  assert.equal(ev.inputAmount - ev.inputExcludedFee!, ev.tradingFee + ev.protocolFee + ev.referralFee);
  const t = dbcTrade(ev, 6, 0)!;
  assert.equal(t.side, "buy"); assert.equal(t.sol, 2635022n); assert.equal(t.fee, 13176n);
  assert.equal(t.price, 2621846 / 1e9 / (64027006502 / 1e6));
  near(t.price, priceFromSqrt(ev.nextSqrtPrice, 6), 0.001);
});

test("a DBC swap made by another program through CPI (a v0 transaction with lookup tables) is decoded from the inner instructions", () => {
  const tx = fx("cpi-swap-5V3AS5sLKH");
  assert.ok(!tx.transaction.message.instructions.some((i) => i.programId === "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN"), "no outer DBC instruction");
  const d = decodeTx(tx);
  assert.equal(d.swaps.length, 1);
  const ev = d.swaps[0].ev as DbcSwapEvent;
  assert.equal(ev.config, "D1P6ADtf6sgWUcJBdesMsjv9YahaLJuHnN7HRDWYVus"); assert.equal(ev.direction, 1);
  assert.equal(d.swaps[0].trader, "BUQaUYtAmdm5bSUwqVkSsWCt4R53eQ3cYy2D6nT4r2gA");
  near(dbcTrade(ev, 6, 0)!.price, priceFromSqrt(ev.nextSqrtPrice, 6), 0.001);
});

test("names and symbols lose control characters and bidi overrides", async () => {
  const { cleanText } = await import("../src/sol/decode.js");
  assert.equal(cleanText("‮sniocemeM"), "sniocemeM");
  assert.equal(cleanText(" a\u0000b‏ "), "ab");
  assert.equal(cleanText("‮"), null);
  assert.equal(cleanText("x".repeat(100), 10), "x".repeat(10));
});

test("swap2 exact-out / partial fill: the trade uses what the swap consumed (included_fee_input_amount), not the request", async () => {
  const { dbcTrade } = await import("../src/sol/trades.js");
  // synthetic EvtSwap2 (layout as the recorded ones): an exact-out buy asking for 1,000 coins with up to 0.02 SOL, of which 0.0101 SOL was used
  const ev: DbcSwapEvent = { kind: "dbc-swap", version: 2, pool: "P", config: "C", direction: 1, hasReferral: false, swapMode: 2,
    inputAmount: 10_100_000n, inputExcludedFee: 10_000_000n, outputAmount: 1_000_000_000n, amountLeft: 0n, nextSqrtPrice: 0n,
    tradingFee: 80_000n, protocolFee: 20_000n, referralFee: 0n, quoteReserve: 1n, migrationThreshold: 2n, timestamp: 0n };
  const t = dbcTrade(ev, 6, 0)!;
  assert.equal(t.sol, 10_100_000n); assert.equal(t.tokens, 1_000_000_000n); assert.equal(t.price, 0.01 / 1000); assert.equal(t.fee, 100_000n);
  // a partial fill that left input unspent: amount_left is not part of what was paid
  const p = dbcTrade({ ...ev, swapMode: 1, amountLeft: 5_000_000n }, 6, 0)!;
  assert.equal(p.sol, 10_100_000n);
  // a sell whose coins in are zero is no trade
  assert.equal(dbcTrade({ ...ev, direction: 0, inputAmount: 0n, inputExcludedFee: 0n }, 6, 0), null);
});
