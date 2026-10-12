import assert from "node:assert/strict";
import { test } from "node:test";
import { checkTaproot } from "../src/lib/bech32m.js";
import { volumeUntilNextBuy } from "../src/lib/estimate.js";

test("bech32m: BIP-350's own taproot vector passes; a typo, a segwit v0 address, the wrong length and mixed case do not", () => {
  const good = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";
  assert.deepEqual(checkTaproot(good), { ok: true });
  assert.deepEqual(checkTaproot(good.toUpperCase()), { ok: true });
  assert.equal(checkTaproot(good.replace("hemja", "hemjq")).ok, false, "one wrong character fails the checksum");
  assert.equal(checkTaproot("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").ok, false, "segwit v0 is not taproot");
  assert.equal(checkTaproot(good.slice(0, -1)).ok, false);
  assert.equal(checkTaproot("bc1P" + good.slice(4)).ok, false, "mixed case");
  assert.equal(checkTaproot("tb1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vq47zagq").ok, false, "testnet");
  assert.equal(checkTaproot("").ok, false);
});

test("volume until the next buy: the arithmetic, its rounding, and the cases where there is no number to give", () => {
  const E = 10n ** 18n, now = 1_000;
  // a 1 ETH cap, nothing in the vault, a 1% creator tax: 1 / (0.8 × 1.7%) = 73.53 ETH of volume (Phase 1's "about 74 ETH")
  const a = volumeUntilNextBuy({ capWei: E, capExpiry: now + 60, nowS: now, spendableWei: 0n, creatorTaxBps: 100 });
  assert.equal(a.volumeWei! / (E / 100n), 7352n);
  assert.equal(a.perEthToVaultWei, (E * 136n) / 10_000n, "1 ETH of volume sends 0.0136 ETH to the vault");
  // rounded UP: the volume given must actually cover the shortfall
  assert.ok((a.volumeWei! * a.perEthToVaultWei) / E >= a.shortfallWei);
  // a 10% tax needs about a sixth of the volume (11.68 ETH against 73.53) a 1% tax does
  const b = volumeUntilNextBuy({ capWei: E, capExpiry: now + 60, nowS: now, spendableWei: 0n, creatorTaxBps: 1000 });
  assert.equal(b.volumeWei! / (E / 100n), 1168n);
  // half-way there
  assert.equal(volumeUntilNextBuy({ capWei: E, capExpiry: now + 60, nowS: now, spendableWei: E / 2n, creatorTaxBps: 100 }).shortfallWei, E / 2n);
  // already affordable
  assert.equal(volumeUntilNextBuy({ capWei: E, capExpiry: now + 60, nowS: now, spendableWei: 2n * E, creatorTaxBps: 0 }).volumeWei, 0n);
  // no cap, or an expired one: no number, and a reason
  for (const o of [{ capWei: 0n, capExpiry: now + 60 }, { capWei: E, capExpiry: now - 1 }, { capWei: E, capExpiry: now }]) {
    const r = volumeUntilNextBuy({ ...o, nowS: now, spendableWei: 0n, creatorTaxBps: 100 });
    assert.equal(r.volumeWei, null); assert.ok(r.why.length > 10);
  }
});
