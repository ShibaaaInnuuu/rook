import assert from "node:assert/strict";
import { test } from "node:test";
import { Transaction, p2tr, p2wpkh, NETWORK } from "@scure/btc-signer";
import { schnorr, secp256k1 } from "@noble/curves/secp256k1.js";
import type { Hex } from "viem";
import { feeBound, judge, leafOf, parsePsbt, proofFor, Refused, satsForWei, verifyProof, weiForSats, MIN_PURCHASE_WEI, MAX_PURCHASE_WEI } from "../src/judge.js";
import { bytesOf, fromUnisatBid, prepare, scriptOf, wantedCap, type Explorer, type OfferInput } from "../src/offer.js";

// docs/v6/p1-services.md §2.2: the offer tool refuses what the canister's frogbuy::judge refuses, rule by rule, and prints what it would sign.
const key = (n: number) => { const k = new Uint8Array(32); k[31] = n; return k; };
const addr = (n: number) => p2tr(schnorr.getPublicKey(key(n)), undefined, NETWORK).address!;
const FLOAT = addr(1), CUSTODY = addr(2), SELLER = addr(3), OTHER = addr(4);
const S = (a: string) => scriptOf(a);
const TX = (c: string) => c.repeat(64);
const SELLER_OUT = `${TX("a")}:0`, INSC = `${TX("a")}i0`;
const sig = new Uint8Array(64).fill(7);

type Spec = { inputs: { txid: string; vout: number; script: Uint8Array | null; value: bigint; witness?: Uint8Array[] }[]; outputs: { script: Uint8Array; value: bigint }[] };
function psbt(s: Spec): Uint8Array {
  const tx = new Transaction({ allowUnknownOutputs: true, allowUnknownInputs: true, disableScriptCheck: true });
  for (const i of s.inputs) tx.addInput({ txid: i.txid, index: i.vout, ...(i.script ? { witnessUtxo: { script: i.script, amount: i.value } } : {}) });
  for (const o of s.outputs) tx.addOutput({ script: o.script, amount: o.value });
  // the seller's signature, as a marketplace hands it over: a final witness on the seller's input (after the shape is fixed)
  s.inputs.forEach((i, k) => { if (i.witness) tx.updateInput(k, { finalScriptWitness: i.witness }, true); });
  return tx.toPSBT();
}
/** The marketplace's usual shape: the frog (seller, 546 sats) first into custody, the price to the seller, our change back. */
const good = (o: Partial<{ price: bigint; fee: bigint; change: bigint }> = {}): Spec => {
  const price = o.price ?? 500_000n, fee = o.fee ?? 2_000n, floatIn = 600_000n;
  return { inputs: [{ txid: TX("a"), vout: 0, script: S(SELLER), value: 546n, witness: [sig] }, { txid: TX("b"), vout: 1, script: S(FLOAT), value: floatIn }],
    outputs: [{ script: S(CUSTODY), value: 546n }, { script: S(SELLER), value: price }, { script: S(FLOAT), value: o.change ?? floatIn - price - fee }] };
};
const facts = (capSats = 600_000n) => ({ float: S(FLOAT), custody: S(CUSTODY), seller: { outpoint: SELLER_OUT, offset: 0n, value: 546n }, capSats });
const refusal = (spec: Spec, toSign = [1], f = facts()) => { try { judge(parsePsbt(psbt(spec)), toSign, f); return "passed"; } catch (e) { assert.ok(e instanceof Refused); return (e as Refused).rule; } };

test("the judge passes a good offer: spend = our input − our change, fee within its bound, the frog lands in the custody output", () => {
  const j = judge(parsePsbt(psbt(good())), [1], facts());
  assert.deepEqual(j, { ours: [1], custodyVout: 0, spend: 502_000n, fee: 2_000n });
  assert.equal(feeBound(502_000n), 15_060n, "3% of the spend (5% less the 2% conversion allowance)");
  assert.equal(feeBound(10_000_000n), 25_000n, "never above custody's absolute ceiling");
});

test("the judge refuses every way frogbuy::judge refuses, by the same names", () => {
  const g = good();
  const cases: [string, Spec, number[]?, ReturnType<typeof facts>?][] = [
    ["UnknownInputValue", { ...g, inputs: [g.inputs[0], { ...g.inputs[1], script: null }] }],
    ["ForeignInputToSign", g, [0]],
    ["SpendsCustody", { ...g, inputs: [...g.inputs, { txid: TX("c"), vout: 0, script: S(CUSTODY), value: 1_000n }] }],
    ["SellerInput", { ...g, inputs: [{ ...g.inputs[0], txid: TX("d") }, g.inputs[1]] }],
    ["SellerValue", { ...g, inputs: [{ ...g.inputs[0], value: 600n }, g.inputs[1]], outputs: [{ ...g.outputs[0], value: 600n }, g.outputs[1], g.outputs[2]] }],
    ["OpReturn", { ...g, outputs: [...g.outputs.slice(0, 2), { script: Uint8Array.from([0x6a, 0x01, 0x00]), value: 0n }, g.outputs[2]] }],
    ["NegativeFee", good({ change: 200_000n })],
    ["OverCap", good({ price: 590_000n, fee: 1_000n }), [1], facts(500_000n)],
    ["FeeTooHigh", good({ fee: 20_000n })],
    ["CustodyOutputs", { ...g, outputs: [{ script: S(OTHER), value: 546n }, ...g.outputs.slice(1)] }],
    ["CustodyDust", { ...g, inputs: [{ ...g.inputs[0] }, g.inputs[1]], outputs: [{ script: S(CUSTODY), value: 300n }, { script: S(OTHER), value: 246n }, ...g.outputs.slice(1)] }],
    ["NotToCustody", { ...g, outputs: [{ script: S(OTHER), value: 546n }, { script: S(CUSTODY), value: 546n }, g.outputs[1], { ...g.outputs[2], value: g.outputs[2].value - 546n }] }],
    ["ForeignSatsToCustody", { inputs: [g.inputs[0], { txid: TX("e"), vout: 0, script: S(OTHER), value: 1_000n, witness: [sig] }, g.inputs[1]],
      outputs: [{ script: S(CUSTODY), value: 1_546n }, g.outputs[1], g.outputs[2]] }, [2]],
    ["ScriptPath", { ...g, inputs: [{ ...g.inputs[0], witness: [sig, new Uint8Array(40), new Uint8Array(33)] }, g.inputs[1]] }],
  ];
  for (const [rule, spec, toSign, f] of cases) assert.equal(refusal(spec, toSign ?? [1], f ?? facts()), rule, rule);
  // P2WPKH witnesses (a signature and a 33-byte key) on a non-taproot input are plain signatures: allowed
  const wpkh = p2wpkh(secp256k1.getPublicKey(key(9), true), NETWORK).script;
  assert.equal(refusal({ ...g, inputs: [{ ...g.inputs[0], script: wpkh, witness: [new Uint8Array(71), new Uint8Array(33)] }, g.inputs[1]] }), "passed");
});

test("the arithmetic is the canister's: wei_for_sats rounds up with the 2% allowance; sats_for_wei is its inverse, stepped down to fit", () => {
  const rate = 3_185_800n;
  // frogbuy.rs: base = ⌈sats × 1e18 / rate⌉, then ⌈base × 10,200 / 10,000⌉
  const base = (502_000n * 10n ** 18n + rate - 1n) / rate;
  assert.equal(weiForSats(502_000n, rate), (base * 10_200n + 9_999n) / 10_000n);
  assert.equal(weiForSats(502_000n, rate), 160_725_720_384_204_910n);
  for (const w of [MIN_PURCHASE_WEI, 158_103_023_730_302_000n, MAX_PURCHASE_WEI, 123_456_789_012_345_678n]) {
    const s = satsForWei(w, rate);
    assert.ok(weiForSats(s, rate) <= w && weiForSats(s + 1n, rate) > w, `${w}`);
  }
  assert.throws(() => weiForSats(1n, 0n), /no rate/);
});

test("membership: leafOf is keccak(keccak(id)); the proof is OpenZeppelin's sorted-pair fold; a wrong id proves nothing", () => {
  const members = Array.from({ length: 7 }, (_, i) => `${TX("f")}i${i}`);
  const { root, proof } = proofFor(members, members[5]);
  assert.ok(verifyProof(proof, root, leafOf(members[5])));
  assert.ok(!verifyProof(proof, root, leafOf(`${TX("f")}i9`)));
  assert.throws(() => proofFor(members, INSC), /not in the member list/);
  // the staging collection's pinned root (keeper/test/send-guards.test.ts, test/DeployChecks.t.sol): same leaves, same tree
  const staging = Array.from({ length: 1024 }, (_, i) => `${"0".repeat(64)}i${i}`);
  assert.equal(proofFor(staging, staging[0]).root, "0xc46b9158cf27bde8deea8ab99cdb7bcf071d86c3a8ae4df8019df01f754af505");
});

const E = (name: string, o: Partial<Explorer> = {}): Explorer => ({ name, exists: true, confirmed: true, spent: false, value: 546n, ...o });
const input = (o: Partial<OfferInput> = {}): OfferInput => {
  const members = [INSC, `${TX("9")}i0`, `${TX("8")}i1`];
  const { root, proof } = proofFor(members, INSC);
  return { frogId: 4n, inscription: INSC, psbt: psbt(good()), toSign: [1], proof, root, float: FLOAT, custody: CUSTODY, capWei: 200_000_000_000_000_000n,
    satsPerEth: 3_185_800n, rateRef: "test", limitWei: null, ...o };
};
const LOC = { outpoint: SELLER_OUT, offset: 0n, value: 546n };
const CLEAN: { inscriptions: string[]; runes: string[]; value: bigint | null } = { inscriptions: [INSC], runes: [], value: 546n };

test("prepare: every check in the canister's order; the report names what it would sign, with the key-path sighash, and the frog_offer arguments", () => {
  const r = prepare(input(), LOC, CLEAN, [E("mempool.space"), E("blockstream.info")]);
  assert.equal(r.ok, true, r.refusal ?? "");
  assert.deepEqual(r.checks.map((c) => c.name), ["inscription id", "membership", "located", "contents", "unspent", "confirmed at ord's value", "judge", "limits", "cap"]);
  assert.equal(r.willSign.length, 1);
  assert.equal(r.willSign[0].input, 1);
  assert.match(r.willSign[0].sighash, /^0x[0-9a-f]{64}$/);
  assert.equal(r.inputs[0].owner, "seller (the frog)");
  assert.deepEqual(r.outputs.map((o) => o.owner), ["CUSTODY", "other", "float (ours)"]);
  assert.match(r.candid!, /^\(4 : nat64, "a{64}i0", "70736274ff[0-9a-f]+", vec \{ 1 : nat32 \}, vec \{ "[0-9a-f]{64}"; "[0-9a-f]{64}" \}\)$/);
  assert.equal(r.costWei, String(weiForSats(502_000n, 3_185_800n)));
});

test("prepare refuses before the PSBT on the facts: a bad proof, unagreed location, extra contents, a spent output, one explorer, a value mismatch; after it on the cap and limits", () => {
  const why = (o: Partial<OfferInput>, loc = LOC as typeof LOC | null, c = CLEAN as typeof CLEAN | null, ex = [E("mempool.space"), E("blockstream.info")]) => prepare(input(o), loc, c, ex).refusal;
  assert.match(why({ root: ("0x" + "11".repeat(32)) as Hex })!, /proof does not verify/);
  assert.match(why({}, null)!, /unlocated/);
  assert.match(why({}, LOC, { ...CLEAN, inscriptions: [INSC, `${TX("7")}i0`] })!, /exactly this inscription/);
  assert.match(why({}, LOC, { ...CLEAN, runes: ["DOGGOTOTHEMOON"] })!, /no rune/);
  assert.match(why({}, LOC, CLEAN, [E("mempool.space", { spent: true }), E("blockstream.info")])!, /listing is gone/);
  assert.match(why({}, LOC, CLEAN, [E("mempool.space")])!, /fewer than two explorers/);
  assert.match(why({}, LOC, CLEAN, [E("mempool.space", { value: 600n }), E("blockstream.info")])!, /not confirmed at ord's value/);
  assert.match(why({}, LOC, CLEAN, [E("mempool.space", { confirmed: false }), E("blockstream.info")])!, /not confirmed/);
  assert.match(why({ capWei: 150_000_000_000_000_000n })!, /refused by the judge: OverCap/);
  assert.match(why({ limitWei: 100_000_000_000_000_000n })!, /outside the purchase limits/);
  assert.match(why({ psbt: Uint8Array.from([1, 2, 3]) })!, /does not parse/);
});

test("the venue's answer: hex or base64 PSBT bytes; UniSat's create_bid shape (unverified) or nothing; the frog's cap from frog_buys()", () => {
  const b = psbt(good());
  assert.deepEqual(bytesOf(Buffer.from(b).toString("hex")), b);
  assert.deepEqual(bytesOf(Buffer.from(b).toString("base64")), b);
  assert.throws(() => bytesOf("not a psbt"), /neither hex nor base64/);
  assert.deepEqual(fromUnisatBid({ code: 0, data: { psbtBid: Buffer.from(b).toString("hex"), bidSignIndexes: [1] } }).toSign, [1]);
  assert.deepEqual(fromUnisatBid({ data: { psbtBid: Buffer.from(b).toString("base64"), toSignInputs: [{ index: 1 }] } }).toSign, [1]);
  assert.throws(() => fromUnisatBid({ data: { psbt: "x" } }), /no data\.psbtBid/);
  assert.throws(() => fromUnisatBid({ data: { psbtBid: Buffer.from(b).toString("hex") } }), /bidSignIndexes/);
  assert.equal(wantedCap([{ id: 4n, target: [], phase: { Wanted: { cap_wei: 7n } } }], 4n), 7n);
  assert.throws(() => wantedCap([{ id: 4n, target: [], phase: { Failed: { why: "x" } } }], 4n), /Failed, not waiting/);
  assert.throws(() => wantedCap([], 4n), /not in frog_buys/);
});

test("E-14 (R2 residual): a rehearsal floor above MAX_PURCHASE_WEI (the fork test's 0.6 ETH) is refused at the policy's limit even when the frog's cap allows it", () => {
  // the keeper's rehearsal fixture floor is 0.6 ETH, so its cap is 0.66 ETH (floor × 1.1); a frog bought at about that floor costs
  // more than policy.rs's MAX_PURCHASE_WEI (0.5 ETH): the canister refuses it, and so does the offer tool, before it is handed in
  const floatIn = 2_500_000n, price = 1_870_000n, fee = 2_000n;
  const spec: Spec = { inputs: [{ txid: TX("a"), vout: 0, script: S(SELLER), value: 546n, witness: [sig] }, { txid: TX("b"), vout: 1, script: S(FLOAT), value: floatIn }],
    outputs: [{ script: S(CUSTODY), value: 546n }, { script: S(SELLER), value: price }, { script: S(FLOAT), value: floatIn - price - fee }] };
  const r = prepare(input({ psbt: psbt(spec), capWei: 660_000_000_000_000_000n }), LOC, CLEAN, [E("mempool.space"), E("blockstream.info")]);
  assert.ok(BigInt(r.costWei!) > MAX_PURCHASE_WEI && BigInt(r.costWei!) <= 660_000_000_000_000_000n, `cost ${r.costWei}: over the limit, under the cap`);
  assert.equal(r.ok, false);
  assert.match(r.refusal!, /outside the purchase limits/);
  assert.equal(r.checks.find((c) => c.name === "limits")?.ok, false);
  // the same frog at a 0.45 ETH price passes the limit: the limit, not the cap, is what refused it
  const cheap = 1_380_000n;
  const ok: Spec = { ...spec, outputs: [spec.outputs[0], { script: S(SELLER), value: cheap }, { script: S(FLOAT), value: floatIn - cheap - fee }] };
  const r2 = prepare(input({ psbt: psbt(ok), capWei: 660_000_000_000_000_000n }), LOC, CLEAN, [E("mempool.space"), E("blockstream.info")]);
  assert.equal(r2.ok, true, r2.refusal ?? "");
});
