// Does the canister sign the SAME transaction it checked? Build one that passes every check in psbt.rs, ask the canister to
// sign it, and compare its digest to @scure/btc-signer's. The runestone is encoded by the CANISTER's own encoder, so the
// bytes it verifies are bytes it produced, and the transaction around them is built by a library it has never seen.
import * as btc from "@scure/btc-signer";
import { hex } from "@scure/base";
import { schnorr } from "@noble/curves/secp256k1";
import { execFileSync } from "node:child_process";

const DFX = `${process.env.HOME}/Library/Application Support/org.dfinity.dfx/bin/dfx`;
const RS = new URL("./canister-rs/", import.meta.url).pathname;
const call = (m, a) => execFileSync(DFX, ["canister", "call", "autobuyer", m, a], { cwd: RS, encoding: "utf8" });
const unwrap = (s) => { const m = s.match(/"((?:[^"\\]|\\.)*)"/s); if (!m) throw new Error(s.slice(0, 300)); return m[1]; };

const OURS_KEY = hex.encode(schnorr.getPublicKey(hex.decode("0".repeat(62) + "0b")));
const WINNER_KEY = hex.encode(schnorr.getPublicKey(hex.decode("0".repeat(62) + "16")));
const OURS = hex.decode("5120" + OURS_KEY), WINNER = hex.decode("5120" + WINNER_KEY);
const RUNE = "840000:3", UNITS = 1000n;

// 1. the runestone, from the canister's own encoder, delivering UNITS to output 1
const spec = JSON.stringify({ edicts: [{ id: RUNE, amount: String(UNITS), output: 1 }] });
const stone = hex.decode(unwrap(call("rehearse_runestone_encode", `("${spec.replace(/"/g, '\\"')}")`)));
console.log(`1. runestone from the canister      ${hex.encode(stone)}`);

// 2. a transaction that satisfies the quote: 100,000 in, 546 to the winner, 98,000 back to us, 1,454 to miners
const IN_VALUE = 100_000n;
const tx = new btc.Transaction({ allowUnknownOutputs: true, version: 2 });
tx.addInput({ txid: hex.decode("ab".repeat(32)), index: 0, sequence: 0xfffffffd,
  witnessUtxo: { script: OURS, amount: IN_VALUE }, tapInternalKey: hex.decode(OURS_KEY) });
tx.addOutput({ script: stone, amount: 0n });        // 0: the runestone
tx.addOutput({ script: WINNER, amount: 546n });     // 1: the winner, which the edict names
tx.addOutput({ script: OURS, amount: 98_000n });    // 2: our change
const psbt = hex.encode(tx.toPSBT(0));
const expected = hex.encode(tx.preimageWitnessV1(0, [OURS], btc.SigHash.DEFAULT, [IN_VALUE]));
console.log(`2. @scure says the digest is        ${expected}`);

// 3. the quote, RECORDED on a funded purchase before any PSBT is in play. sign_purchase takes no quote: it reads this one.
const blob = (b) => `blob "${[...b].map((x) => "\\" + x.toString(16).padStart(2, "0")).join("")}"`;
const quoteFor = (ours, maxSpend = 5000) => `(record { rune = record { block = 840000 : nat64; tx = 3 : nat32 }; units = ${UNITS} : nat; `
  + `max_spend_sats = ${maxSpend} : nat64; max_fee_sats = 2000 : nat64; `
  + `recipient_script = ${blob(WINNER)}; our_script = ${blob(ours)} })`;
const expectRefusal = (out, words, what) => {
  if (!out.includes("Err") || !out.includes(words)) throw new Error(`${what}: expected a refusal containing "${words}", got ${out.slice(0, 300)}`);
  console.log(`   ✓ refused: ${what}`);
};
const ID = BigInt(Date.now());   // a fresh rehearsal purchase per run: the canister refuses to open an id twice
call("rehearse_open_purchase", `(${ID} : nat64, 1000000000000000 : nat)`);
expectRefusal(call("sign_purchase", `(${ID} : nat64, "${hex.encode(tx.toPSBT(0))}", 0 : nat32)`), "only against a recorded quote",
  "signing a funded purchase that has no recorded quote");
call("record_quote", `(${ID} : nat64, ${quoteFor(OURS)})`);
console.log(`3. purchase ${ID}: quote recorded before the PSBT`);
expectRefusal(call("record_quote", `(${ID} : nat64, ${quoteFor(OURS, 500_000)})`), "already has a recorded quote",
  "re-quoting it for 100x the spend, after the fact");

// 4. the canister verifies against the RECORDED quote, computes its OWN digest, and signs that
const out = call("sign_purchase", `(${ID} : nat64, "${psbt}", 0 : nat32)`);
const got = (out.match(/sighash = "([0-9a-f]{64})"/) || [])[1];
const sig = (out.match(/signature = "([0-9a-f]{128})"/) || [])[1];
const txid = (out.match(/txid = "([0-9a-f]{64})"/) || [])[1];
if (!got) throw new Error(out.slice(0, 400));
console.log(`4. the canister says the digest is  ${got}`);
console.log(`   txid, known before broadcast     ${txid}`);

if (got !== expected) throw new Error("the canister hashed a different transaction from the one it was given");
console.log(`\n   ✓ the canister's digest equals an independent library's, on a transaction it verified first`);

// 5. and the signature is real. `taproot_key()` returns the INTERNAL key — `sign_taproot` passes aux Bip341 with an empty
// merkle root, so the subnet applies the tweak and the signature verifies under the OUTPUT key, which is a different
// number. Getting these two the wrong way round is what once made a rehearsal's coins unspendable, so both are checked.
const internal = unwrap(call("taproot_key", "()"));
const output = unwrap(call("taproot_output_key", "()"));
// and the canister's own tweak must agree with the library's, or one of the two is wrong about BIP341
const byLibrary = hex.encode(btc.p2tr(hex.decode(internal)).tweakedPubkey);
if (output !== byLibrary) throw new Error(`the canister tweaked to ${output}, the library to ${byLibrary}`);
const ok = schnorr.verify(hex.decode(sig), hex.decode(got), hex.decode(output));
const underInternal = (() => { try { return schnorr.verify(hex.decode(sig), hex.decode(got), hex.decode(internal)); } catch { return false; } })();
console.log(`   internal key                     ${internal}`);
console.log(`   tweaked output key               ${output}  (canister and @scure agree)`);
console.log(`   ✓ verifies under the OUTPUT key:   ${ok}`);
console.log(`   ✓ and NOT under the internal one:  ${!underInternal}`);
if (!ok || underInternal) throw new Error("the signature is not what it claims to be");

// 6. one quote pays for one transaction: the same purchase will not sign a second, different one -- even one that would
// pass every check -- and an unknown purchase signs nothing
const tx2 = new btc.Transaction({ allowUnknownOutputs: true, version: 2 });
tx2.addInput({ txid: hex.decode("cd".repeat(32)), index: 1, sequence: 0xfffffffd,
  witnessUtxo: { script: OURS, amount: IN_VALUE }, tapInternalKey: hex.decode(OURS_KEY) });
tx2.addOutput({ script: stone, amount: 0n });
tx2.addOutput({ script: WINNER, amount: 546n });
tx2.addOutput({ script: OURS, amount: 98_000n });
expectRefusal(call("sign_purchase", `(${ID} : nat64, "${hex.encode(tx2.toPSBT(0))}", 0 : nat32)`), "one quote pays for one transaction",
  "a second transaction under the same purchase");
expectRefusal(call("sign_purchase", `(${ID + 1n} : nat64, "${psbt}", 0 : nat32)`), "there is no purchase", "a purchase that does not exist");
// and a recorded quote the transaction does not satisfy is still refused before the key is touched
const ID2 = ID + 2n;
call("rehearse_open_purchase", `(${ID2} : nat64, 1000000000000000 : nat)`);
call("record_quote", `(${ID2} : nat64, ${quoteFor(WINNER)})`);
expectRefusal(call("sign_purchase", `(${ID2} : nat64, "${psbt}", 0 : nat32)`), "ForeignInputToSign", "an input the recorded quote says is not ours");
console.log(`\n   ✓ #23: the quote is the recorded one, fixed before the PSBT, and one quote signs one transaction`);
