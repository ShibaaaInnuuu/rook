import assert from "node:assert/strict";
import { test } from "node:test";
import { schnorr } from "@noble/curves/secp256k1";
import * as btc from "@scure/btc-signer";
import { taprootOutputKey, verifyBip340, hexToBytes, bytesToHex, blobArg, parseBlob } from "../lib.mjs";

test("BIP341 key-path tweak: our arithmetic reproduces the BIP's own test vector and the library's address", () => {
  // BIP341 wallet-test-vectors, scriptPubKey[0]: internal key, no script tree
  const internal = hexToBytes("d6889cb081036e0faefa3a35157ad71086b123b2b144b649798b494c300a961d");
  assert.equal(bytesToHex(taprootOutputKey(internal)), "53a1f6e454df1aa2776a2814a721372d6258050de330b3c6d10ee8f4e0dda343");
  assert.equal(btc.p2tr(internal).address, "bc1p2wsldez5mud2yam29q22wgfh9439spgduvct83k3pm50fcxa5dps59h4z5");
});

test("a signature made under the tweaked key verifies under the output key and NOT under the internal key", () => {
  // stands in for the subnet: tweak a local key the way `aux = #bip341` does, sign, and check both ways
  const d = hexToBytes("01".repeat(32)), internal = schnorr.getPublicKey(d), msg = hexToBytes("ab".repeat(32));
  const p = btc.p2tr(internal, undefined, btc.TEST_NETWORK);
  const tx = new btc.Transaction();
  tx.addInput({ txid: hexToBytes("22".repeat(32)), index: 1, witnessUtxo: { script: p.script, amount: 50_000n }, tapInternalKey: internal });
  tx.addOutputAddress(p.address, 49_000n, btc.TEST_NETWORK);
  tx.signIdx(d, 0);                                              // the library tweaks the private key for a key-path spend
  const sig = tx.getInput(0).tapKeySig;
  const sighash = tx.preimageWitnessV1(0, [p.script], btc.SigHash.DEFAULT, [50_000n]);
  assert.equal(verifyBip340(sig, sighash, taprootOutputKey(internal)), true);
  assert.equal(verifyBip340(sig, sighash, internal), false);
  assert.equal(verifyBip340(sig, msg, taprootOutputKey(internal)), false, "and not for another message");
});

test("the dfx blob codec round-trips every byte, including quotes and backslashes", () => {
  const all = Uint8Array.from({ length: 256 }, (_, i) => i);
  assert.deepEqual(parseBlob(`(${blobArg(all)})`), all);
  assert.deepEqual(parseBlob('(blob "a\\"b\\\\c\\00")'), Uint8Array.from([0x61, 0x22, 0x62, 0x5c, 0x63, 0x00]));
});
