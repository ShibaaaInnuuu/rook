// Fix round 5 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round5.md), the runner's side: the state this round's
// canister writes, with every one of its new fields present (a written-off order holding the Relay leg and one released, the watch's
// request id and 4663 pin, a pending HomeTx with its pin, the scan's remembered oversized blocks, stuck start and day's bytes, the
// operator's word on an unseen home transaction, a payout claim in the inbox: E5-I-3), decodes with the runner's IDL. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

test("round 5: eth_nft_state() with every new field present decodes with the runner's IDL", async () => {
  const { IDL } = await import("@icp-sdk/core/candid");
  const { buyerIdl } = await import("../src/ic.js");
  const svc = buyerIdl({ IDL }) as unknown as { _fields: [string, { retTypes: Parameters<typeof IDL.decode>[0] }][] };
  const f = svc._fields.find(([n]) => n === "eth_nft_state")![1];
  const hex = readFileSync(new URL("../../autobuyer/canister-rs/tests/fixtures/eth-state-round5.hex", import.meta.url), "utf8").trim();
  const [st] = IDL.decode(f.retTypes, Uint8Array.from(Buffer.from(hex, "hex"))) as [{ buys: { phase: Record<string, unknown> }[] }];
  assert.deepEqual(st.buys.map((b) => Object.keys(b.phase)[0]), ["Bridging", "Stuck", "Returned"]);
});
