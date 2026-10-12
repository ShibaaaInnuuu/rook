// Build a transaction with @scure/btc-signer, ask IT for the BIP341 sighash, and print both the PSBT and the digest.
// The Rust implementation is then checked against this — an independent library whose signatures Bitcoin has accepted —
// rather than against a second copy of my own reasoning.
import * as btc from "@scure/btc-signer";
import { hex } from "@scure/base";

const cases = [];

function build(name, { inputs, outputs, version = 2, locktime = 0 }) {
  const tx = new btc.Transaction({ allowUnknownOutputs: true, version, lockTime: locktime });
  for (const i of inputs) {
    tx.addInput({
      txid: hex.decode(i.txid),
      index: i.vout,
      sequence: i.sequence,
      witnessUtxo: { script: hex.decode(i.script), amount: BigInt(i.value) },
      tapInternalKey: hex.decode(i.internalKey),
    });
  }
  for (const o of outputs) tx.addOutput({ script: hex.decode(o.script), amount: BigInt(o.value) });

  const amounts = inputs.map((i) => BigInt(i.value));
  const scripts = inputs.map((i) => hex.decode(i.script));
  const digests = inputs.map((_, n) => hex.encode(tx.preimageWitnessV1(n, scripts, btc.SigHash.DEFAULT, amounts)));
  cases.push({ name, psbt: hex.encode(tx.toPSBT(0)), digests });
}

// a real shape: one taproot input of ours, an OP_RETURN runestone, a recipient, and change back to us
// real x-only keys, derived from fixed secrets, because a taproot script must carry a point that is actually on the curve
import { schnorr } from "@noble/curves/secp256k1";
const xonly = (n) => hex.encode(schnorr.getPublicKey(hex.decode(String(n).padStart(64, "0"))));
const OURS_KEY = xonly(11), WINNER_KEY = xonly(22);
const OURS = "5120" + OURS_KEY;
const WINNER = "5120" + WINNER_KEY;
const INTERNAL = OURS_KEY;

build("one input, three outputs, a runestone among them", {
  inputs: [{ txid: "a".repeat(64), vout: 1, sequence: 0xfffffffd, value: 100_000, script: OURS, internalKey: INTERNAL }],
  outputs: [
    { script: "6a5d0714c0a23314c801", value: 0 },   // OP_RETURN OP_13 <runestone>
    { script: WINNER, value: 546 },
    { script: OURS, value: 98_000 },
  ],
});

build("two inputs, so sha_amounts and sha_scriptpubkeys span more than one", {
  version: 1,
  locktime: 800_000,
  inputs: [
    { txid: "b".repeat(64), vout: 0, sequence: 0xffffffff, value: 55_555, script: OURS, internalKey: INTERNAL },
    { txid: "c".repeat(64), vout: 7, sequence: 0, value: 21_000_000, script: WINNER, internalKey: INTERNAL },
  ],
  outputs: [{ script: WINNER, value: 21_050_000 }],
});

build("a big value and a long script, to exercise compact size", {
  inputs: [{ txid: "d".repeat(64), vout: 4294967295, sequence: 1, value: 2_100_000_000_000_000, script: OURS, internalKey: INTERNAL }],
  outputs: [{ script: "6a4c" + "fd" + "ff".repeat(253), value: 0 }, { script: OURS, value: 2_099_999_999_999_000 }],
});

console.log(JSON.stringify(cases, null, 2));
