// W11: the canister-only home's state, decoded from the canister's own bytes (autobuyer/canister-rs/tests/fixtures/views/*.hex, written
// by its tests) and paged: a halted collection, a drand disagreement, a stuck raffle commit, a wait of 7 h, a stale desk read. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const FIX = (f) => readFileSync(new URL(`../../autobuyer/canister-rs/tests/fixtures/views/${f}`, import.meta.url), "utf8").trim();

test("W11: the four views decode from the canister's bytes and page what matters", async () => {
  const { decodeReply } = await import("../src/candid.mjs");
  const { booksAlerts } = await import("../src/books.mjs");
  const treasury = decodeReply("treasury_status", FIX("treasury-status.hex"));
  const desks = decodeReply("pool_desks", FIX("pool-desks.hex"));
  const coins = decodeReply("coins_log", FIX("coins-log.hex"));
  const desk = decodeReply("desk_status", FIX("desk-status.hex"));
  // the treasury's books as the canister encodes them
  assert.equal(treasury.treasury.unallocated, "0");
  assert.equal(treasury.treasury.total_received, "10000000000");
  assert.deepEqual(treasury.treasury.ops, ["5555555555555555555555555555555555555555555555555555555555555555"]);
  assert.ok(treasury.owed.some((x) => "Surplus" in x));
  assert.equal(desks.length, 1);
  assert.equal(desks[0].coll, 0);
  assert.equal(desks[0].arb_profit, "1234567");
  assert.equal(coins[0][0][0], 0);
  assert.equal(coins[0][0][1].curve_wx, "1");
  assert.equal(coins[1][0].key, "open_config:1");
  assert.deepEqual(coins[3], [[2, "1700000600"]], "the opening queue: collection 2, asked by a creator");
  assert.equal(desk.halted[0][0], 0);
  assert.equal(desk.open, 1);
  const nowS = 1_700_000_000 + 7 * 3_600;
  const a = booksAlerts({ buyer: "b", treasury, desks, coins, desk, nowS });
  const keys = a.map((x) => x.key);
  const sev = Object.fromEntries(a.map((x) => [x.key, x.sev]));
  assert.ok(keys.includes("books-halted:b:0") && sev["books-halted:b:0"] === "SEV1", keys.join("\n"));
  assert.ok(keys.includes("books-drand-disagree:b:20025637") && sev["books-drand-disagree:b:20025637"] === "SEV2");
  assert.ok(keys.includes("books-commit-stuck:b:0:1"));
  assert.ok(keys.includes("books-wait:b:treasury:ops") && sev["books-wait:b:treasury:ops"] === "SEV3", "7 h: SEV3");
  assert.ok(keys.includes("books-wait:b:coins:open_config:1"));
  assert.ok(keys.includes("books-desk-stale:b:0"), "the desk read 7 h ago");
  assert.ok(keys.includes("books-reconcile-stale:b"));
  // a day later the waits are SEV2; nothing live is paged twice under one key
  const b = booksAlerts({ buyer: "b", treasury, desks, coins, desk, nowS: nowS + 24 * 3_600 });
  assert.equal(b.find((x) => x.key === "books-wait:b:treasury:ops").sev, "SEV2");
  assert.equal(new Set(b.map((x) => x.key)).size, b.length);
  // nothing: no alerts
  assert.deepEqual(booksAlerts({ buyer: "b", treasury: null, desks: null, coins: null, desk: null, nowS }), []);
});
