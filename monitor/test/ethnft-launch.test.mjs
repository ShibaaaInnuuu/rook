import assert from "node:assert/strict";
import { test } from "node:test";
import { validate } from "../src/config.mjs";

// docs/v7/phase1-nft.md §9 (decided 2026-09-27): the launch decisions the monitor holds.
test("D8: an eth_nft monitor needs an armed guardian unless the config says in words it does not", () => {
  const A = "0x" + "11".repeat(20);
  const base = { every_secs: 60, eth_nft: { home_rpc: ["https://h"], eth_rpc: ["https://a", "https://b"], collections: [{ slug: "milady", collection: A, wrapper: A, module: A, custodian: A }] } };
  assert.throws(() => validate(structuredClone(base)), /D8 needs evm.guardian/);
  assert.doesNotThrow(() => { try { validate({ ...structuredClone(base), eth_nft: { ...base.eth_nft, guardian_pause: false } }); } catch (e) { if (/D8/.test(e.message)) throw e; } });
});
