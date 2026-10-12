import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { backingFigures, chainAddresses, collectionEquation, expectedSupply, moduleOf, overall, redemptionValue, reportSummary, type ChainSide } from "../src/rook/sol/booksReserves";
import { decodeBooksView, type VaultViewV } from "../src/rook/sol/canister";

/** W12 (docs/v9/canister-only-solana.md §10.2): the Reserves page's rules, the reconciler's (ops/src/booksreserves.ts), against the canister's
 *  own books_view bytes and a chain side that matches them, then one that fails each rule. */
const FIX = readFileSync(new URL("fixtures/views/books-view.hex", import.meta.url), "utf8").trim();
const v = decodeBooksView(FIX)[0]!;
const MODULE = "ModuLe1111111111111111111111111111111111111", VAULT = "VauLt11111111111111111111111111111111111111", RENT = 890_880n;
const expected = v.expected_supply[0]!;
const vaults: VaultViewV[] = [{ coll: 0, coin: "CoinMint", pool: "Pool", stage: "Backed", vault: [VAULT], wx_in: 2_500_000_000n, coins_burned: 0n, wx_paid: 0n, backed_at: 1n }];
const chain = (over: Partial<ChainSide> = {}): ChainSide => ({ supply: expected, moduleLamports: RENT + v.owed + 5n, rent: RENT, vaults: new Map([[VAULT, 2_500_000_000n]]), ...over });

test("the fixture's equation is green against a chain that matches it; the unbooked 5 lamports are reported, not counted", () => {
  const e = collectionEquation(v, MODULE, vaults, chain());
  assert.deepEqual([e.status, e.reasons], ["green", []]);
  assert.equal(e.expected, expected);
  assert.equal(expectedSupply(v), expected);
  assert.equal(e.supplyHolds, true);
  assert.equal(e.backingHolds, true);
  assert.equal(e.unbooked, 5n);
  assert.equal(e.balance, v.owed + 5n);
  assert.equal(e.vaults[0].status, "green");
  assert.equal(e.backingPerUnit, (v.backing * 1_000_000_000n) / v.sol_backed);
  assert.equal(overall([e]), "green");
  assert.deepEqual(chainAddresses(v, MODULE, vaults), [v.wx_mint, MODULE, VAULT]);
  assert.deepEqual(chainAddresses(v, null, []), [v.wx_mint]);
});

test("a supply off by one, a module short of its books, a short coin vault, a halted collection and a miscounted ledger are each RED", () => {
  const supply = collectionEquation(v, MODULE, vaults, chain({ supply: expected + 1n }));
  assert.equal(supply.status, "red");
  assert.match(supply.reasons[0], /wX supply .* on chain is not the books' expected supply/);
  assert.equal(supply.supplyHolds, false);
  const short = collectionEquation(v, MODULE, vaults, chain({ moduleLamports: RENT + v.owed - 1n }));
  assert.equal(short.status, "red");
  assert.match(short.reasons[0], /holds .* lamports above its rent; the books say it must hold/);
  assert.equal(short.backingHolds, false);
  const vault = collectionEquation(v, MODULE, vaults, chain({ vaults: new Map([[VAULT, 1n]]) }));
  assert.equal(vault.status, "red");
  assert.match(vault.reasons[0], /vault .* holds 1 wX base units, its book says 2500000000/);
  assert.equal(vault.vaults[0].status, "red");
  const halted = collectionEquation({ ...v, halted: ["pre_sign refused: SupplyMismatch"] }, MODULE, vaults, chain());
  assert.equal(halted.status, "red");
  assert.match(halted.reasons[0], /stopped by an incident/);
  const miscount = collectionEquation({ ...v, items: v.items.slice(1) }, MODULE, vaults, chain());
  assert.match(miscount.reasons[0], /count 2 held items and list 1/);
  const inconsistent = collectionEquation({ ...v, expected_supply: [] }, MODULE, vaults, chain());
  assert.match(inconsistent.reasons[0], /books are inconsistent/);
  assert.equal(expectedSupply({ held: 1n, open_tickets: 2n, sol_backed: 0n, module_units: 0n }), null);
  assert.equal(overall([supply, collectionEquation(v, MODULE, vaults, chain())]), "red");
});

test("a read that is missing is UNVERIFIED, never green: the mint, the module, a vault, or a module address the canister has not published", () => {
  const mint = collectionEquation(v, MODULE, vaults, chain({ supply: null }));
  assert.equal(mint.status, "unverified");
  assert.match(mint.reasons[0], /wX mint .* could not be read/);
  assert.equal(mint.supplyHolds, null);
  const mod = collectionEquation(v, MODULE, vaults, chain({ moduleLamports: null }));
  assert.equal(mod.status, "unverified");
  assert.match(mod.reasons[0], /module .* could not be read/);
  const vault = collectionEquation(v, MODULE, vaults, chain({ vaults: new Map() }));
  assert.equal(vault.status, "unverified");
  assert.equal(vault.vaults[0].status, "unverified");
  const noAddr = collectionEquation(v, null, vaults, chain());
  assert.equal(noAddr.status, "unverified");
  assert.match(noAddr.reasons[0], /names no module address/);
  const noVaultAddr = collectionEquation(v, MODULE, [{ ...vaults[0], vault: [] }], chain());
  assert.equal(noVaultAddr.vaults[0].status, "unverified");
  assert.equal(noVaultAddr.status, "green", "a vault with no address and a book is the row's problem, not a reason by itself");
  assert.equal(overall([mint, collectionEquation(v, MODULE, vaults, chain())]), "unverified");
  assert.equal(overall([]), "unverified");
  // a red rule outranks a missing read
  const both = collectionEquation(v, MODULE, vaults, chain({ supply: expected + 1n, moduleLamports: null }));
  assert.equal(both.status, "red");
  assert.equal(both.reasons.length, 2);
});

test("the backing figures and the coin's redemption value follow the books: circulating = (held − tickets) × 1 + SOL-backed; ⌊V × a ÷ S⌋", () => {
  const f = backingFigures(v);
  assert.equal(f.circulating, expected);
  assert.equal(f.nftBacked, 1_000_000_000n);
  assert.equal(f.solBacked, v.sol_backed);
  assert.deepEqual([f.itemsHeld, f.ticketsOpen], [2n, 1n]);
  assert.equal(f.acceptedFloor, v.accepted);
  assert.equal(f.mintPrice, v.mint_price[0]!);
  assert.equal(f.burnPaysOne, null);
  assert.equal(f.reserveShare, 0.25);
  assert.equal(redemptionValue(2_500_000_000n, 1_000_000n, 1_000_000_000_000n), 2_500n);
  assert.equal(redemptionValue(10n, 3n, 4n), 7n, "rounded down, never overpaying");
  assert.equal(redemptionValue(10n, 3n, 0n), 0n);
  assert.equal(moduleOf([{ index: 0, symbol: "wMILADY", stage: "Open", wx_mint: v.wx_mint, config: "", module: MODULE, coins: 0, backed: 0 }], 0), MODULE);
  assert.equal(moduleOf([{ index: 0, symbol: "wMILADY", stage: "Open", wx_mint: v.wx_mint, config: "", module: "", coins: 0, backed: 0 }], 0), null);
  assert.equal(moduleOf([], 0), null);
});

test("the canister's own report is summarized: fresh and clean is green; a mismatch, an unexplained memo or a stopped collection is red; old is stale", () => {
  const now = 1_700_003_600;
  const clean = reportSummary({ at: 1_700_000_000n, unexplained: [], checked: 3, memos_seen: 12, findings: [], halted: [] }, 1_700_000_000n, now);
  assert.deepEqual([clean.tone, clean.stale], ["green", false]);
  assert.equal(clean.line, "3 collections checked, 12 Rook memos read from the chain.");
  const stale = reportSummary({ at: 1_700_000_000n, unexplained: [], checked: 1, memos_seen: 1, findings: [], halted: [] }, 1_700_000_000n, now + 4 * 3_600);
  assert.deepEqual([stale.tone, stale.stale], ["unverified", true]);
  assert.equal(stale.line, "1 collection checked, 1 Rook memo read from the chain.");
  const soft = reportSummary({ at: 1_700_000_000n, unexplained: [], checked: 1, memos_seen: 1, findings: [{ coll: 0, chain: 1n, books: 2n, note: "", kind: { HolderBurn: null } }, { coll: 0, chain: 9n, books: 2n, note: "", kind: { Unbooked: null } }], halted: [] }, 1_700_000_000n, now);
  assert.equal(soft.tone, "green", "a holder's own burn and an unbooked gift are soft findings");
  assert.match(soft.line, /2 findings/);
  const bad = reportSummary({ at: 1_700_000_000n, unexplained: [], checked: 1, memos_seen: 1, findings: [{ coll: 0, chain: 1n, books: 2n, note: "", kind: { SupplyMismatch: null } }], halted: [] }, 1_700_000_000n, now);
  assert.equal(bad.tone, "red");
  const memo = reportSummary({ at: 1_700_000_000n, unexplained: ["rook:v1:0:mint:99"], checked: 1, memos_seen: 2, findings: [], halted: [] }, 1_700_000_000n, now);
  assert.equal(memo.tone, "red");
  assert.match(memo.line, /1 unexplained \(a mint among them is SEV1\)/);
  const halted = reportSummary({ at: 1_700_000_000n, unexplained: [], checked: 1, memos_seen: 1, findings: [], halted: [4] }, 1_700_000_000n, now);
  assert.equal(halted.tone, "red");
  assert.match(halted.line, /stopped collection: 4/);
  const none = reportSummary(null, 0n, now);
  assert.equal(none.tone, "unverified");
  assert.match(none.line, /not reconciled/);
  assert.match(reportSummary(null, 1_700_000_000n, now).line, /keeps no report/);
});
