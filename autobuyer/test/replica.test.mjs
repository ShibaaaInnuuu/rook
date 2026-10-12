// Needs the local replica with the spike canister deployed (`cd canister && dfx start --clean --background && dfx deploy`). Skips otherwise.
import assert from "node:assert/strict";
import { test } from "node:test";
import * as btc from "@scure/btc-signer";
import { canisterKey, canisterSign, taprootOutputKey, verifyBip340, hexToBytes } from "../lib.mjs";

let up = true;
try { canisterKey(); } catch { up = false; }

test("the threshold key signs a taproot key-path spend that a standard library finalises", { skip: !up && "no local replica" }, () => {
  const internal = canisterKey().slice(1), p = btc.p2tr(internal, undefined, btc.TEST_NETWORK);
  const tx = new btc.Transaction();
  tx.addInput({ txid: hexToBytes("33".repeat(32)), index: 0, witnessUtxo: { script: p.script, amount: 80_000n }, tapInternalKey: internal });
  tx.addOutputAddress(p.address, 79_000n, btc.TEST_NETWORK);
  const sighash = tx.preimageWitnessV1(0, [p.script], btc.SigHash.DEFAULT, [80_000n]);
  const sig = canisterSign(sighash);
  assert.equal(sig.length, 64);
  assert.equal(verifyBip340(sig, sighash, taprootOutputKey(internal)), true, "valid under the OUTPUT key: the subnet applied the BIP341 tweak");
  assert.equal(verifyBip340(sig, sighash, internal), false, "and therefore not under the internal key");
  tx.updateInput(0, { tapKeySig: sig });
  tx.finalize();
  assert.ok(tx.extract().length > 100);
});
