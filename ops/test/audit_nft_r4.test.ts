// Fix round 4 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round4.md), the runner's side of E4-I-4: the state this
// round's canister writes, with every one of its new fields present (a written-off Relay order, the pin floors, the scan's oversized
// ranges and skipped blocks, a HomeTx dropped after its nonce was taken), decodes with the runner's IDL. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

test("round 4: eth_nft_state() with every new field present decodes with the runner's IDL", async () => {
  const { IDL } = await import("@icp-sdk/core/candid");
  const { buyerIdl } = await import("../src/ic.js");
  const svc = buyerIdl({ IDL }) as unknown as { _fields: [string, { retTypes: Parameters<typeof IDL.decode>[0] }][] };
  const f = svc._fields.find(([n]) => n === "eth_nft_state")![1];
  const hex = readFileSync(new URL("../../autobuyer/canister-rs/tests/fixtures/eth-state-round4.hex", import.meta.url), "utf8").trim();
  const [st] = IDL.decode(f.retTypes, Uint8Array.from(Buffer.from(hex, "hex"))) as [{ buys: { phase: Record<string, unknown> }[] }];
  assert.deepEqual(st.buys.map((b) => Object.keys(b.phase)[0]), ["BackBridging", "Stuck"]);
});
