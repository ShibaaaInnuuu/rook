// The sim's fake marketplace (a stand-in for UniSat / Magic Eden's classic listing): regtest only.
//
//   1. the SELLER lists: a 1-in / 1-out PSBT (the frog's output in, the asking price out to the seller) signed by the seller's
//      own wallet (ord's bitcoind wallet "ord") with SIGHASH_SINGLE|ANYONECANPAY (0x83) and finalized — what a classic
//      marketplace keeps when a frog is listed;
//   2. the MARKETPLACE builds the buyer's PSBT the way `create_bid` does for a padded listing (docs/v8/ordinals-wave.md §1.3):
//      in  [float pad 600, float pad 600, seller's frog (final witness copied in), float payment]
//      out [1,200 padding back to the float, the frog's output value to custody, the price to the seller, change to the float]
//      with every input's witness_utxo, the seller's input FINALIZED (this marketplace hands the seller's signature over), and
//      the buyer's inputs to sign = [0, 1, 3].
//
// It signs nothing of the buyer's. Input: one JSON argument (see run.sh / the harness). Output: one JSON line on stdout.
// Run from ops/ so @scure/btc-signer resolves from ops/node_modules.
import * as btc from "@scure/btc-signer";

const a = JSON.parse(process.argv[2]);
const hex = (s) => Uint8Array.from(Buffer.from(s, "hex"));
const toHex = (b) => Buffer.from(b).toString("hex");

async function rpc(method, params, wallet) {
  const url = wallet ? `${a.rpc}/wallet/${wallet}` : a.rpc;
  const r = await fetch(url, { method: "POST", headers: { "content-type": "application/json",
    authorization: "Basic " + Buffer.from(`${a.user}:${a.pass}`).toString("base64") },
    body: JSON.stringify({ jsonrpc: "1.0", id: "market", method, params }) });
  const j = await r.json();
  if (j.error) throw new Error(`${method}: ${JSON.stringify(j.error)}`);
  return j.result;
}

const btcOf = (sats) => (sats / 1e8).toFixed(8);

// 1. the seller's listing, signed SINGLE|ANYONECANPAY by the seller's own wallet
const payout = await rpc("getnewaddress", ["listing payout", "bech32m"], a.seller_wallet);
const listing = await rpc("createpsbt", [[{ txid: a.seller.txid, vout: a.seller.vout }], [{ [payout]: btcOf(a.price) }], 0, true]);
const signed = await rpc("walletprocesspsbt", [listing, true, "SINGLE|ANYONECANPAY", true, true], a.seller_wallet);
if (!signed.complete) throw new Error("the seller's wallet did not finalize the listing");
const dec = await rpc("decodepsbt", [signed.psbt]);
const witness = dec.inputs[0].final_scriptwitness;
if (!witness || witness.length !== 1 || witness[0].length !== 130 || !witness[0].endsWith("83")) {
  throw new Error(`the seller's witness is not one 65-byte SIGHASH_SINGLE|ANYONECANPAY signature: ${JSON.stringify(witness)}`);
}
const sequence = dec.tx.vin[0].sequence;
const payoutScript = dec.tx.vout[0].scriptPubKey.hex;

// 2. the buyer's PSBT, as the marketplace's create_bid would hand it over
const tx = new btc.Transaction({ allowUnknownOutputs: true, allowUnknownInputs: true, disableScriptCheck: true, version: 2, lockTime: 0 });
const fl = hex(a.float_script);
for (const p of a.pads) tx.addInput({ txid: p.txid, index: p.vout, sequence: 0xfffffffd, witnessUtxo: { script: fl, amount: BigInt(p.value) } });
tx.addInput({ txid: a.seller.txid, index: a.seller.vout, sequence, witnessUtxo: { script: hex(a.seller.script), amount: BigInt(a.seller.value) } });
tx.addInput({ txid: a.pay.txid, index: a.pay.vout, sequence: 0xfffffffd, witnessUtxo: { script: fl, amount: BigInt(a.pay.value) } });
const padIn = a.pads.reduce((s, p) => s + p.value, 0);
const change = a.pay.value - a.price - a.fee;
if (change < 546) throw new Error("the payment input does not cover the price and fee");
tx.addOutput({ script: fl, amount: BigInt(padIn) });
tx.addOutput({ script: hex(a.custody_script), amount: BigInt(a.seller.value) });
tx.addOutput({ script: hex(payoutScript), amount: BigInt(a.price) });
tx.addOutput({ script: fl, amount: BigInt(change) });
// the seller's signature goes in last (scure refuses to add inputs or outputs after a signed one; SINGLE|ANYONECANPAY would not mind)
tx.updateInput(2, { finalScriptWitness: witness.map(hex) }, true);
const psbt = toHex(tx.toPSBT());
console.log(JSON.stringify({ psbt_hex: psbt, to_sign: [0, 1, 3], seller_sighash: "0x83", seller_witness: witness[0], payout, payout_script: payoutScript,
  listing_psbt_b64: signed.psbt }));
