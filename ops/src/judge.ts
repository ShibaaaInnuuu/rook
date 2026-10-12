import { Transaction } from "@scure/btc-signer";
import { toHex, type Hex } from "viem";
import { leafOf, proofFor, satsForWei, verifyProof, weiForSats } from "./frogmath.js";
export { leafOf, proofFor, satsForWei, verifyProof, weiForSats };

/**
 * docs/v6/p1-services.md §2.2. A TypeScript port of the canister's own judgement of a marketplace PSBT for one frog —
 * `frogbuy::judge`, `frogbuy::fee_bound`, `frogbuy::wei_for_sats` / `sats_for_wei`, `frogbuy::leaf_of` / `verify_proof`
 * (autobuyer/canister-rs/src/frogbuy.rs) — so the operator's offer tool refuses what the canister would refuse, BEFORE it is
 * handed in, and can print exactly what the canister would sign. The canister judges again whatever it is given: this is a
 * mirror for failing early, never a replacement. Same rules, same order, same names.
 */
export const DUST_P2TR = 330n;                  // custody.rs
export const MAX_DELIVERY_FEE_SATS = 25_000n;   // custody.rs
export const MAX_COST_BPS = 500n;               // policy.rs: 5%
export const CONVERSION_BPS = 200n;             // frogbuy.rs: the ETH→BTC allowance
export const MIN_PURCHASE_WEI = 50_000_000_000_000_000n;    // policy.rs: 0.05 ETH
export const MAX_PURCHASE_WEI = 500_000_000_000_000_000n;   // policy.rs: 0.5 ETH
const BPS = 10_000n, WEI = 10n ** 18n;

/** frogbuy::fee_bound: the miner fee a purchase may pay, within MAX_COST_BPS of the spend less the conversion allowance, and never above custody's absolute ceiling. */
export const feeBound = (spend: bigint): bigint => { const b = (spend * (MAX_COST_BPS - CONVERSION_BPS)) / BPS; return b < MAX_DELIVERY_FEE_SATS ? b : MAX_DELIVERY_FEE_SATS; };
// ---- the PSBT ----------------------------------------------------------------------------------------------------------------------

export type In = { index: number; outpoint: string; value: bigint | null; script: Uint8Array | null; witness: Uint8Array[] | null };
export type Out = { index: number; value: bigint; script: Uint8Array };
export type Parsed = { tx: Transaction; inputs: In[]; outputs: Out[] };

/** Parse a PSBT from hex or base64 bytes. Every input keeps its witness UTXO (value and script) and its final witness, if any. */
export function parsePsbt(bytes: Uint8Array): Parsed {
  const tx = Transaction.fromPSBT(bytes, { allowUnknownInputs: true, allowUnknownOutputs: true, disableScriptCheck: true, allowLegacyWitnessUtxo: true });
  const inputs: In[] = [];
  for (let i = 0; i < tx.inputsLength; i++) {
    const x = tx.getInput(i);
    inputs.push({ index: i, outpoint: `${Buffer.from(x.txid!).toString("hex")}:${x.index}`, value: x.witnessUtxo ? BigInt(x.witnessUtxo.amount) : null,
      script: x.witnessUtxo ? x.witnessUtxo.script : null, witness: x.finalScriptWitness ?? null });
  }
  const outputs: Out[] = [];
  for (let i = 0; i < tx.outputsLength; i++) { const o = tx.getOutput(i); outputs.push({ index: i, value: BigInt(o.amount!), script: o.script! }); }
  return { tx, inputs, outputs };
}

export type Seller = { outpoint: string; offset: bigint; value: bigint };
export type Facts = { float: Uint8Array; custody: Uint8Array; seller: Seller; capSats: bigint };
export type Judged = { ours: number[]; custodyVout: number; spend: bigint; fee: bigint };
export class Refused extends Error { constructor(readonly rule: string, detail: string) { super(`${rule}: ${detail}`); this.name = "Refused"; } }

const eq = (a: Uint8Array | null, b: Uint8Array) => !!a && a.length === b.length && a.every((x, i) => x === b[i]);
const isP2tr = (s: Uint8Array | null) => !!s && s.length === 34 && s[0] === 0x51 && s[1] === 0x20;

/** frogbuy::judge, rule for rule. */
export function judge(p: Parsed, toSign: readonly number[], f: Facts): Judged {
  // 1. every input's value is known
  const values: bigint[] = [];
  for (const i of p.inputs) { if (i.value === null) throw new Refused("UnknownInputValue", `input ${i.index} has no witness UTXO`); values.push(i.value); }
  // 2. ours are the float's; we sign only ours; nothing spends custody
  const ours = p.inputs.filter((i) => eq(i.script, f.float)).map((i) => i.index);
  for (const i of toSign) if (!ours.includes(i)) throw new Refused("ForeignInputToSign", `input ${i} is not the float's`);
  for (const i of p.inputs) if (eq(i.script, f.custody)) throw new Refused("SpendsCustody", `input ${i.index} spends custody`);
  // 3. exactly one input is the seller's, worth what was read
  const sellerIn = p.inputs.filter((i) => i.outpoint === f.seller.outpoint).map((i) => i.index);
  if (sellerIn.length !== 1) throw new Refused("SellerInput", `${sellerIn.length} inputs spend the seller's outpoint ${f.seller.outpoint}`);
  const s = sellerIn[0];
  if (values[s] !== f.seller.value) throw new Refused("SellerValue", `the PSBT says ${values[s]}, the providers read ${f.seller.value}`);
  // 4. no OP_RETURN at all
  const opr = p.outputs.find((o) => o.script[0] === 0x6a);
  if (opr) throw new Refused("OpReturn", `output ${opr.index}`);
  // 5. the money
  const inTotal = values.reduce((a, b) => a + b, 0n), outTotal = p.outputs.reduce((a, o) => a + o.value, 0n);
  if (outTotal > inTotal) throw new Refused("NegativeFee", `outputs ${outTotal} > inputs ${inTotal}`);
  const fee = inTotal - outTotal;
  const oursIn = ours.reduce((a, i) => a + values[i], 0n), oursOut = p.outputs.filter((o) => eq(o.script, f.float)).reduce((a, o) => a + o.value, 0n);
  const spend = oursIn > oursOut ? oursIn - oursOut : 0n;
  if (spend > f.capSats) throw new Refused("OverCap", `spend ${spend} sats > cap ${f.capSats}`);
  const bound = feeBound(spend);
  if (fee > bound) throw new Refused("FeeTooHigh", `fee ${fee} sats > bound ${bound}`);
  // 6. exactly one output pays custody, above dust
  const toCustody = p.outputs.filter((o) => eq(o.script, f.custody)).map((o) => o.index);
  if (toCustody.length !== 1) throw new Refused("CustodyOutputs", `${toCustody.length} outputs pay custody`);
  const c = toCustody[0];
  if (p.outputs[c].value < DUST_P2TR) throw new Refused("CustodyDust", `${p.outputs[c].value} sats`);
  // 7. the sat flow
  const sellerStart = values.slice(0, s).reduce((a, b) => a + b, 0n), pos = sellerStart + f.seller.offset;
  let start = 0n, landed: number | null = null;
  for (const o of p.outputs) { if (pos >= start && pos < start + o.value) { landed = o.index; break; } start += o.value; }
  if (landed !== c) throw new Refused("NotToCustody", `the inscribed sat lands in output ${landed}`);
  // 8. every input overlapping the custody output's range is the seller's or ours
  const cStart = p.outputs.slice(0, c).reduce((a, o) => a + o.value, 0n), cEnd = cStart + p.outputs[c].value;
  let a = 0n;
  for (let i = 0; i < values.length; i++) {
    const b = a + values[i];
    if (a < cEnd && b > cStart && i !== s && !ours.includes(i)) throw new Refused("ForeignSatsToCustody", `input ${i}'s sats reach custody`);
    a = b;
  }
  // 9. foreign witnesses are plain signatures: a key path, or P2WPKH
  for (const i of p.inputs) {
    if (ours.includes(i.index) || !i.witness) continue;
    const w = i.witness;
    const keyPath = w.length === 1 && (w[0].length === 64 || w[0].length === 65);
    const wpkh = w.length === 2 && w[1].length === 33 && !isP2tr(i.script);
    if (!keyPath && !wpkh) throw new Refused("ScriptPath", `input ${i.index}'s witness is not a plain signature`);
  }
  return { ours, custodyVout: c, spend, fee };
}

/** The BIP-341 key-path sighash (SIGHASH_DEFAULT) of input `i`: what the canister's `sighash::taproot_key_path` signs. */
export function keyPathSighash(p: Parsed, i: number): Hex {
  const scripts = p.inputs.map((x) => x.script!), amounts = p.inputs.map((x) => x.value!);
  const pre = (p.tx as unknown as { preimageWitnessV1(idx: number, s: Uint8Array[], t: number, a: bigint[]): Uint8Array }).preimageWitnessV1(i, scripts, 0, amounts);
  return toHex(pre);
}
