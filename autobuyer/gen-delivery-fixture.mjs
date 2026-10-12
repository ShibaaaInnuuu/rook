// The frog-delivery transaction built by an implementation that is not ours (@scure/btc-signer): its txid and both BIP341
// key-path digests, for custody.rs's test. The same inputs as that test: the real inscription-0 output and a real plain UTXO,
// spent from a stand-in script, at 2 sat/vB, as a version-3 (TRUC) transaction. Run: node gen-delivery-fixture.mjs
import * as btc from "@scure/btc-signer";
import { hex } from "@scure/base";
import { sha256 } from "@noble/hashes/sha2";
const ours = hex.decode("5120" + "01".repeat(32));
const dest = hex.decode("5120a60869f0dbcf1dc659c9cecbaf8050135ea9e8cdc487053f1dc6880949dc684c");
const frog = { txid: "717d25914a1191c30f4c1d709e3ec0e1fab79cc7bf92cd12a93bed68db63416a", vout: 0, value: 606n };
const fee = { txid: "57dd43538d17048682ca40662df1889d4d579bfcff16e81449a6541181457e9a", vout: 2, value: 2500n };
const rate = 2n, vsize = 212n, feeSats = vsize * rate;
const tx = new btc.Transaction({ version: 3, allowUnknownOutputs: true });   // TRUC (BIP 431): every delivery since canister review2 N-2
tx.addInput({ txid: hex.decode(frog.txid), index: frog.vout, sequence: 0xfffffffd, witnessUtxo: { script: ours, amount: frog.value } });
tx.addInput({ txid: hex.decode(fee.txid), index: fee.vout, sequence: 0xfffffffd, witnessUtxo: { script: ours, amount: fee.value } });
tx.addOutput({ script: dest, amount: frog.value });
tx.addOutput({ script: ours, amount: fee.value - feeSats });
const pre = [0, 1].map((i) => hex.encode(tx.preimageWitnessV1(i, [ours, ours], btc.SigHash.DEFAULT, [frog.value, fee.value])));
const legacy = tx.toBytes(false, false);
const txid = hex.encode(sha256(sha256(legacy)).reverse());
console.log(JSON.stringify({ txid, sighash0: pre[0], sighash1: pre[1], change: String(fee.value - feeSats) }));
