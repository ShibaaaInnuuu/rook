import assert from "node:assert/strict";
import { test } from "node:test";
import { amountProblem, depositInstructions, depositOf, FEE_LAMPORTS, intentsJson, landedSignature, lastFailure, MIN_BURN_UNITS, MIN_MINT_LAMPORTS, mintMinOut, openedSummary, openKindFor, refusalCopy, rememberedIntents, statusCopy, transferTokens } from "../src/rook/sol/intent";
import { ATA_PROGRAM, SYSTEM_PROGRAM, TOKEN_PROGRAM } from "../src/rook/sol/keys";
import { ata } from "../src/rook/sol/pda";
import type { IntentViewV } from "../src/rook/sol/canister";
import { intentView } from "./fixtures/intent";

/** W12 (docs/v9/canister-only-solana.md §6.1): the desk's copy for every state and refusal, and the deposit the wallet signs. */
const WALLET = "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy", A = "7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE", WX = "DyRkUpQxYG2VnP2SkMdQs5BTsVPeKCpSHLxzByc2Sxvj";
const NOW = 1_700_010_000;

test("the numbers are the canister's (desk/intent.rs): 0.0015 SOL fee, 0.05 SOL / 0.01 wX minimums, exactly 1 wX for a redemption", () => {
  assert.equal(FEE_LAMPORTS, 1_500_000n);
  assert.equal(MIN_MINT_LAMPORTS, 50_000_000n);
  assert.equal(MIN_BURN_UNITS, 10_000_000n);
  assert.deepEqual(depositOf("mint", "wMILADY"), { asset: "SOL", min: 50_000_000n, minText: "0.05 SOL", decimals: 9 });
  assert.equal(depositOf("burn", "wMILADY").asset, "wMILADY");
  assert.equal(depositOf("redeem_nft", "wMILADY").min, 1_000_000_000n);
  assert.equal(depositOf("redeem_coin", "wMILADY", "MOOD").asset, "MOOD");
});

test("open_intent's argument carries the connected wallet as recipient; a mint's min_out is 95 % of the quote at the deposit", () => {
  assert.deepEqual(openKindFor("mint", { coll: 3, wallet: WALLET, minOut: 10n }), { Mint: { coll: 3, recipient: WALLET, min_out: [10n] } });
  assert.deepEqual(openKindFor("mint", { coll: 3, wallet: WALLET, minOut: null }), { Mint: { coll: 3, recipient: WALLET, min_out: [] } });
  assert.deepEqual(openKindFor("burn", { coll: 3, wallet: WALLET }), { Burn: { coll: 3, recipient: WALLET, refund_to: [], min_out: [] } });
  assert.deepEqual(openKindFor("redeem_nft", { coll: 3, wallet: WALLET, destination: " burn " }), { RedeemNft: { coll: 3, destination: "burn", owner: WALLET } });
  assert.throws(() => openKindFor("redeem_nft", { coll: 3, wallet: WALLET, destination: "" }), /destination/);
  assert.deepEqual(openKindFor("redeem_coin", { coll: 3, wallet: WALLET, coin: A }), { RedeemCoin: { coll: 3, coin: A, recipient: WALLET, refund_to: [] } });
  assert.throws(() => openKindFor("redeem_coin", { coll: 3, wallet: WALLET }), /coin's mint/);
  const q = { Mint: { fee: 1_500_000n, min_deposit: 50_000_000n, coll: 3, ata_rent: 0n, deposit: 50_000_000n, to_module: 48_500_000n, unit_price: 200_000_000_000n, units: 242_500n } };
  assert.equal(mintMinOut(q, 100_000_000n), (242_500n * 2n * 9_500n) / 10_000n);
  assert.equal(mintMinOut(null, 100_000_000n), null);
  assert.equal(mintMinOut({ Burn: { fee: 0n, net: 0n, coll: 3, paid: 0n, units: 0n, min_units: 0n } }, 1n), null);
});

test("the deposit: a mint is a system transfer to A; a burn creates A's token account idempotently and transfers from the wallet's", () => {
  const [ix] = depositInstructions("mint", { wallet: WALLET, address: A, tokenAccount: null, mint: null, amount: 50_000_000n });
  assert.equal(ix.programId, SYSTEM_PROGRAM);
  assert.deepEqual(ix.keys.map((k) => [k.pubkey, k.isSigner, k.isWritable]), [[WALLET, true, true], [A, false, true]]);
  assert.deepEqual([...ix.data], [2, 0, 0, 0, 0x80, 0xf0, 0xfa, 2, 0, 0, 0, 0]);
  const tokenAccount = ata(A, WX);
  const [create, move] = depositInstructions("burn", { wallet: WALLET, address: A, tokenAccount, mint: WX, amount: 10_000_000n });
  assert.equal(create.programId, ATA_PROGRAM);
  assert.deepEqual(create.keys.map((k) => k.pubkey), [WALLET, tokenAccount, A, WX, SYSTEM_PROGRAM, TOKEN_PROGRAM]);
  assert.equal(move.programId, TOKEN_PROGRAM);
  assert.deepEqual(move.keys.map((k) => [k.pubkey, k.isSigner, k.isWritable]), [[ata(WALLET, WX), false, true], [tokenAccount, false, true], [WALLET, true, false]]);
  assert.deepEqual([...move.data], [3, 0x80, 0x96, 0x98, 0, 0, 0, 0, 0]);
  assert.deepEqual(transferTokens("a", "b", "c", 1n).data[0], 3);
  assert.throws(() => depositInstructions("burn", { wallet: WALLET, address: A, tokenAccount: WALLET, mint: WX, amount: 1n }), /not the deposit address's associated token account/);
  assert.throws(() => depositInstructions("burn", { wallet: WALLET, address: A, tokenAccount: null, mint: WX, amount: 1n }), /no token account/);
  assert.throws(() => depositInstructions("mint", { wallet: WALLET, address: A, tokenAccount: null, mint: null, amount: 0n }), /nothing to send/);
});

test("an amount is checked against the minimum, the balance and the one-whole-wX redemption rule", () => {
  assert.match(amountProblem("mint", null, null, "wX")!, /Enter an amount of SOL/);
  assert.match(amountProblem("mint", 10_000_000n, null, "wX")!, /Below the minimum: the canister acts on 0.05 SOL/);
  assert.match(amountProblem("mint", 100_000_000n, 110_000_000n, "wX")!, /More than your SOL/);
  assert.equal(amountProblem("mint", 100_000_000n, 130_000_000n, "wX"), null);
  assert.match(amountProblem("burn", 1_000_000n, null, "wX")!, /0.01 wX/);
  assert.match(amountProblem("burn", 20_000_000n, 10_000_000n, "wX")!, /More wX than you hold/);
  assert.match(amountProblem("redeem_nft", 2_000_000_000n, null, "wX")!, /exactly 1 wX/);
  assert.equal(amountProblem("redeem_nft", 1_000_000_000n, 5_000_000_000n, "wX"), null);
  assert.equal(amountProblem("redeem_coin", 1n, 1n, "wX"), null);
});

const tx = (op: string, result: string, signature = "5igNature") => ({ at: 1n, op, result, signature, link: 1n, memo: "rook:v1:0:mint:1" });

test("status copy: every state has a headline, a detail, a step and a tone; Done and Refunded are final and name the landed signature", () => {
  const open = statusCopy(intentView(), NOW, "wX");
  assert.deepEqual([open.state, open.step, open.final, open.tone], ["Open", 1, false, "wait"]);
  assert.match(open.detail, /Send SOL to the address below/);
  assert.match(open.detail, /stays open 21 h more/);
  const expiredOpen = statusCopy(intentView({ expires_at: BigInt(NOW - 1) }), NOW, "wX");
  assert.match(expiredOpen.detail, /It has expired/);
  const dust = statusCopy(intentView({ deposit: 10_000_000n, note: "waiting: 10000000 is below the minimum 50000000" }), NOW, "wX");
  assert.equal(dust.headline, "Deposit seen, below the minimum");
  assert.match(dust.detail, /0.01 SOL is at the address, under the minimum of 0.05 SOL/);
  const funded = statusCopy(intentView({ state: { Funded: null }, deposit: 100_000_000n, note: "funded: 100000000" }), NOW, "wX");
  assert.deepEqual([funded.headline, funded.step, funded.tone], ["Deposit finalized", 2, "work"]);
  const waits = statusCopy(intentView({ state: { Funded: null }, deposit: 100_000_000n, note: "waits: the floor is stale" }), NOW, "wX");
  assert.deepEqual([waits.headline, waits.tone], ["Deposit in, mint waiting", "warn"]);
  assert.match(waits.detail, /the floor is stale\. The canister retries every tick for 6 hours, then returns the deposit less the fee/);
  const exec = statusCopy(intentView({ state: { Executing: null }, deposit: 100_000_000n, in_flight: ["rook:v1:0:mint:4"] }), NOW, "wX");
  assert.deepEqual([exec.headline, exec.step], ["Mint signed, landing", 3]);
  assert.match(exec.detail, /its memo is rook:v1:0:mint:4/);
  const retry = statusCopy(intentView({ state: { Executing: null }, txs: [tx("Mint", "Failed: blockhash expired", "")] }), NOW, "wX");
  assert.match(retry.detail, /An earlier attempt did not land \(Mint: Failed: blockhash expired\); this is a retry/);
  const refunding = statusCopy(intentView({ state: { Executing: null }, refunding: ["a price bound"] }), NOW, "wX");
  assert.equal(refunding.headline, "Returning your deposit");
  const done = statusCopy(intentView({ state: { Done: null }, minted_units: 485_000n, txs: [tx("Mint", "landed")] }), NOW, "wX");
  assert.deepEqual([done.headline, done.step, done.final, done.tone], ["Done", 4, true, "good"]);
  assert.match(done.detail, /0.000485 wX minted to GGgX/);
  assert.equal(landedSignature(intentView({ txs: [tx("Mint", "landed")] })), "5igNature");
  const burn = statusCopy(intentView({ kind: "burn", state: { Done: null }, paid_lamports: 1_980_000_000n, txs: [tx("Burn", "landed")] }), NOW, "wX");
  assert.match(burn.detail, /1.98 SOL paid to/);
  const nft = statusCopy(intentView({ kind: "redeem_nft", state: { Done: null }, ticket: [12n], txs: [tx("Redeem", "landed")] }), NOW, "wX");
  assert.match(nft.detail, /Ticket #12 opened: the custodian delivers the NFT/);
  const coin = statusCopy(intentView({ kind: "redeem_coin", state: { Done: null }, paid_lamports: 2_500_000_000n, txs: [tx("RedeemCoin", "landed")] }), NOW, "wX");
  assert.match(coin.detail, /2.5 wX paid to/);
  const refunded = statusCopy(intentView({ state: { Refunded: null }, note: "the deposit goes back: a price bound", txs: [tx("Refund", "landed", "r3fund")] }), NOW, "wX");
  assert.deepEqual([refunded.headline, refunded.final, refunded.tone], ["Deposit returned", true, "warn"]);
  assert.match(refunded.detail, /less the fee: the deposit goes back: a price bound\. Nothing was minted/);
  assert.equal(landedSignature(intentView({ txs: [tx("Mint", "Failed: x", "bad"), tx("Refund", "landed", "r3fund")] })), "r3fund");
  assert.equal(landedSignature(intentView()), null);
  assert.equal(lastFailure(intentView({ txs: [tx("Mint", "Abandoned: nonce moved", "x"), tx("Mint", "landed")] })), "Mint: Abandoned: nonce moved");
  const expired = statusCopy(intentView({ state: { Expired: null } }), NOW, "wX");
  assert.deepEqual([expired.headline, expired.final, expired.tone], ["Expired", true, "bad"]);
  // every state yields non-empty copy
  for (const s of ["Open", "Funded", "Executing", "Done", "Refunded", "Expired"] as const) {
    const c = statusCopy(intentView({ state: { [s]: null } as IntentViewV["state"] }), NOW, "wX");
    assert.ok(c.headline.length > 3 && c.detail.length > 40, s);
  }
});

test("refusals are explained in the site's words, keeping the canister's message where it is the explanation", () => {
  assert.match(refusalCopy("the desk is paused"), /paused by the operator/);
  assert.match(refusalCopy("collection 3 is paused: no mint"), /mint is paused/);
  assert.match(refusalCopy("collection 3 is stopped by an incident"), /stopped by an incident/);
  assert.match(refusalCopy("5 intents an hour per caller: try again later"), /5 intents an hour per caller/);
  assert.match(refusalCopy("3 intents are already open for this recipient"), /3 intents are already open for this wallet/);
  assert.match(refusalCopy("300 intents are open: try again later"), /desk is full/);
  assert.match(refusalCopy("200 intents are waiting for a deposit: try again later"), /desk is full/);
  assert.match(refusalCopy("coin redemption is not enabled yet"), /not enabled/);
  assert.match(refusalCopy("destination: not a taproot address"), /destination was refused: destination: not a taproot address/);
  assert.equal(refusalCopy("no collection 9"), "The canister refused: no collection 9");
});

test("the opened reply is summarized with the quote, and the wallet's intents are remembered by id in this browser", () => {
  const mintQ = { id: 1n, fee: 1_500_000n, min_deposit: 50_000_000n, token_account: [] as [], address: A, expires_at: 1n,
    quote: [{ Mint: { fee: 1_500_000n, min_deposit: 50_000_000n, coll: 0, ata_rent: 2_039_280n, deposit: 50_000_000n, to_module: 46_460_720n, unit_price: 200_000_000_000n, units: 232_303n } }] as [{ Mint: { fee: bigint; min_deposit: bigint; coll: number; ata_rent: bigint; deposit: bigint; to_module: bigint; unit_price: bigint; units: bigint } }] };
  assert.match(openedSummary(mintQ, "mint", "wX"), /At the minimum deposit of 0.05 SOL you would get ≈ 0.000232 wX \(200 SOL per whole wX\); the fee is 0.0015 SOL, plus 0.002039 SOL rent/);
  assert.match(openedSummary({ ...mintQ, quote: [] }, "mint", "wX"), /No quote right now/);
  assert.match(openedSummary({ ...mintQ, quote: [{ Burn: { fee: 1_500_000n, net: 1_978_500_000n, coll: 0, paid: 1_980_000_000n, units: 10_000_000n, min_units: 10_000_000n } }] }, "burn", "wX"), /the books pay 1.98 SOL; 1.9785 SOL reaches you/);
  assert.match(openedSummary({ ...mintQ, quote: [] }, "redeem_nft", "wX"), /Send exactly 1 wX/);
  assert.match(openedSummary({ ...mintQ, quote: [] }, "redeem_coin", "wX"), /⌊vault × coins ÷ supply⌋/);
  const store = new Map<string, string>();
  store.set(`rook-intents:${WALLET}`, intentsJson([1n, 2n, 3n]));
  assert.deepEqual(rememberedIntents(WALLET, (k) => store.get(k) ?? null), [1n, 2n, 3n]);
  assert.deepEqual(rememberedIntents("other", (k) => store.get(k) ?? null), []);
  assert.deepEqual(rememberedIntents(WALLET, () => "garbage"), []);
  assert.equal(JSON.parse(intentsJson(Array.from({ length: 14 }, (_, i) => BigInt(i)))).length, 10, "the last ten are kept");
});
