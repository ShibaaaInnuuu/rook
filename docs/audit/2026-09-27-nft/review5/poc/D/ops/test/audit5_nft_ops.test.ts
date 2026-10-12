// Review round 5, part D: the runner's IDL (ops/src/ic.ts) decodes the canister's round-4 state written by the 1dc2a61 build's own
// encoder (review5/poc/D/canister/d5_fixture.rs: written-off orders, a re-priced line, a Recording delivery) and the canister's own
// tests/fixtures/eth-state-round4.hex. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

test("holds: the runner's IDL decodes the round-4 canister states (d5-state.hex, eth-state-round4.hex)", async () => {
  const { IDL } = await import("@icp-sdk/core/candid");
  const { buyerIdl } = await import("../src/ic.js");
  const svc = buyerIdl({ IDL }) as unknown as { _fields: [string, { retTypes: Parameters<typeof IDL.decode>[0] }][] };
  const f = svc._fields.find(([n]) => n === "eth_nft_state")![1];
  const dec = (p: string) => IDL.decode(f.retTypes, Uint8Array.from(Buffer.from(readFileSync(new URL(p, import.meta.url), "utf8").trim(), "hex")))[0] as
    { buys: { id: bigint; phase: Record<string, unknown> }[]; deliveries: { phase: Record<string, unknown> }[] };
  const d5 = dec("../../monitor/test/d5-state.hex");
  assert.deepEqual(d5.buys.map((b) => Object.keys(b.phase)[0]), ["Stuck", "Stuck", "Stuck", "Returned", "BackBridging", "Stuck"]);
  assert.deepEqual(d5.deliveries.map((d) => Object.keys(d.phase)[0]), ["Sent", "Recording"]);
  const r4 = dec("../../autobuyer/canister-rs/tests/fixtures/eth-state-round4.hex");
  assert.equal(r4.buys.length, 2);
});
