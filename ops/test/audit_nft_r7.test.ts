// Fix round 7 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round7.md), the runner's side of F7-L-1 / D7-I-3 / F7-I-2:
// while a 4663 transaction is out the runner's cadence is a PERIOD from the cycle's start (default 120 s, at most 300 s), so a slow
// cycle no longer pushes the next tick out of the canister's search reach (≈ 520 s from signing at worst); the variable takes plain
// digits only; a transaction past that reach (seen nowhere 15 min) no longer keeps the runner fast. The round-7 state decodes. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { parseRunnerConfig, pendingWantsHurry, PENDING_STALE_S, sleepAfter } from "../src/runner.js";

const ID = "ryjl3-tyaaa-aaaaa-aaaba-cai";

test("round 7: eth_nft_state() of this round decodes with the runner's IDL, pending and its used_since included", async () => {
  const { IDL } = await import("@icp-sdk/core/candid");
  const { buyerIdl } = await import("../src/ic.js");
  const svc = buyerIdl({ IDL }) as unknown as { _fields: [string, { retTypes: Parameters<typeof IDL.decode>[0] }][] };
  const f = svc._fields.find(([n]) => n === "eth_nft_state")![1];
  for (const n of [7, 6, 5]) {
    const hex = readFileSync(new URL(`../../autobuyer/canister-rs/tests/fixtures/eth-state-round${n}.hex`, import.meta.url), "utf8").trim();
    const [st] = IDL.decode(f.retTypes, Uint8Array.from(Buffer.from(hex, "hex"))) as [{ pending: [] | [{ nonce: bigint; used_since: [] | [bigint] }] }];
    assert.equal(st.pending[0]?.nonce, 7n, `round ${n}`);
    assert.ok(Array.isArray(st.pending[0]?.used_since));
  }
});

test("F7-L-1 / D7-I-3: the pending cadence is a period from the cycle's start, default 120 s, 60 to 300, plain digits only", () => {
  const cfg = parseRunnerConfig({ ROOK_HOME: "robinhood", CANISTER_ID: ID });
  assert.equal(cfg.pendingEveryS, 120);
  assert.equal(sleepAfter(cfg, true, 0), 120);
  assert.equal(sleepAfter(cfg, true, 90), 30, "a 90 s cycle: the next tick 120 s after it started");
  assert.equal(sleepAfter(cfg, true, 400), 10, "a cycle longer than the period: at once (10 s)");
  assert.equal(sleepAfter(cfg, false, 90), 600);
  for (const bad of ["301", "600", "0x3c", "6e1", "120.5", "59", " 120"]) {
    assert.throws(() => parseRunnerConfig({ ROOK_HOME: "robinhood", CANISTER_ID: ID, RUNNER_PENDING_EVERY_S: bad }), /RUNNER_PENDING_EVERY_S/, bad);
  }
  assert.equal(parseRunnerConfig({ ROOK_HOME: "robinhood", CANISTER_ID: ID, RUNNER_PENDING_EVERY_S: "300" }).pendingEveryS, 300);
});

test("F7-I-2: a transaction seen nowhere for 15 min is past the reach: the normal cadence again", () => {
  const now = 1_791_000_000;
  assert.equal(pendingWantsHurry(null, now), null);
  assert.equal(pendingWantsHurry([], now), false);
  assert.equal(pendingWantsHurry([{ used_since: [] }], now), true);
  assert.equal(pendingWantsHurry([{ used_since: [BigInt(now - 60)] }], now), true);
  assert.equal(pendingWantsHurry([{ used_since: [BigInt(now - PENDING_STALE_S)] }], now), false);
});
