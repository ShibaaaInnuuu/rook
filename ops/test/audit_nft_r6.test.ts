// Fix round 6 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round6.md), the runner's side: the state this round's
// canister writes (eth-state-round6.hex, every new field present) decodes with the runner's IDL, which now reads the pending 4663
// transaction; and while one is out the runner ticks sooner (F6-L-1: 4663's first provider keeps state ≈ 10 minutes, and the canister
// identifies a transaction every index misses only from state inside that reach). No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { parseRunnerConfig, sleepAfter } from "../src/runner.js";

test("round 6: eth_nft_state() with every new field present decodes with the runner's IDL, pending included", async () => {
  const { IDL } = await import("@icp-sdk/core/candid");
  const { buyerIdl } = await import("../src/ic.js");
  const svc = buyerIdl({ IDL }) as unknown as { _fields: [string, { retTypes: Parameters<typeof IDL.decode>[0] }][] };
  const f = svc._fields.find(([n]) => n === "eth_nft_state")![1];
  for (const [n, want] of [[6, 1], [5, 1], [4, 1]] as const) {
    const hex = readFileSync(new URL(`../../autobuyer/canister-rs/tests/fixtures/eth-state-round${n}.hex`, import.meta.url), "utf8").trim();
    const [st] = IDL.decode(f.retTypes, Uint8Array.from(Buffer.from(hex, "hex"))) as [{ pending: [] | [{ nonce: bigint }] }];
    assert.equal(st.pending.length, want, `round ${n}`);
    assert.equal(st.pending[0]?.nonce, 7n);
  }
});

test("F6-L-1: while a 4663 transaction is out the runner sleeps RUNNER_PENDING_EVERY_S (fix round 7: default 120 s), never longer than its cadence", () => {
  const cfg = parseRunnerConfig({ ROOK_HOME: "robinhood", CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai" });
  assert.equal(cfg.pendingEveryS, 120);
  assert.equal(sleepAfter(cfg, true), 120);
  assert.equal(sleepAfter(cfg, false), 600);
  assert.equal(sleepAfter(cfg, null), 600, "an unreadable state is no reason to hurry");
  assert.equal(sleepAfter(parseRunnerConfig({ ROOK_HOME: "robinhood", CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai", RUNNER_TICK_EVERY_S: "120" }), true), 120);
  assert.throws(() => parseRunnerConfig({ ROOK_HOME: "robinhood", CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai", RUNNER_PENDING_EVERY_S: "30" }), /RUNNER_PENDING_EVERY_S/);
  assert.throws(() => parseRunnerConfig({ ROOK_HOME: "robinhood", CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai", RUNNER_PENDING_EVERY_S: "900" }), /RUNNER_PENDING_EVERY_S/);
});
