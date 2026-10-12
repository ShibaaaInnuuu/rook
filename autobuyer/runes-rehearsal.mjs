// The runes rehearsal. Pre-registered in `docs/v2/2026-09-22-evm-and-runes-rehearsals.md`.
//
//   node runes-rehearsal.mjs mint       -- encode a mint runestone IN THE CANISTER, sign it, broadcast it to signet
//   node runes-rehearsal.mjs confirm    -- wait for the block, then ask an ord indexer whether a rune actually moved
//   node runes-rehearsal.mjs transfer   -- encode an EDICT moving the minted unit, sign it, broadcast it
//   node runes-rehearsal.mjs verify     -- confirm the transfer, and check the unit is on the OUTPUT the edict named
//   node runes-rehearsal.mjs status     -- print what is known so far
//
// SIGNET ONLY. The guards in `signet.mjs` run on every phase, exactly as they do for the satoshi rehearsal.
//
// What makes this different from a self-send is the third party. A Bitcoin node will relay any well-formed OP_RETURN
// without an opinion about what it means -- so a node accepting this proves only that the bytes are a valid output. The
// claim under test is that the bytes MEAN what the buyer thinks, and the only thing that can disagree about that is an
// `ord` indexer. UniSat's signet indexer is one, it answers without a key, and it has never heard of this repository.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import * as btc from "@scure/btc-signer";
import { assertSignetEndpoint, assertSignetAddress, buyerAddress, NETWORK_NAME, SIGNET } from "./signet.mjs";
import { verifyBip340, bytesToHex, hexToBytes } from "./lib.mjs";
import { script as referenceScript } from "./runestone.mjs";

const DFX = join(homedir(), "Library/Application Support/org.dfinity.dfx/bin");
const ENV = { ...process.env, PATH: `${DFX}:${process.env.PATH}` };
const RS = join(import.meta.dirname, "canister-rs");
const STATE = join(import.meta.dirname, ".runes-rehearsal.json");
const API = "https://mempool.space/signet/api";
const ORD = "https://open-api-signet.unisat.io";

/// UNCOMMON•GOODS on signet: open mint, one unit a go, a cap no one will reach, no height bounds. Chosen because minting
/// it needs no marketplace, no counterparty and nobody's permission -- a mint is a runestone and nothing else.
const RUNE = "189444:15";
const RUNE_NAME = "UNCOMMON•GOODS";

const load = () => (existsSync(STATE) ? JSON.parse(readFileSync(STATE, "utf8")) : {});
const save = (o) => writeFileSync(STATE, JSON.stringify({ ...load(), ...o }, null, 2) + "\n");
const log = (s = "") => console.log(s);
const bold = (s) => `\x1b[1m${s}\x1b[0m`;

// ---- the canister ------------------------------------------------------------------------------------------------------------

const dfx = (...args) => execFileSync("dfx", args, { cwd: RS, env: ENV, encoding: "utf8", timeout: 180_000 });
function unwrap(out) {
  const err = out.match(/Err = "((?:[^"\\]|\\.)*)"/s);
  if (err) throw new Error("canister refused: " + err[1].replaceAll('\\"', '"'));
  const ok = out.match(/Ok = "((?:[^"\\]|\\.)*)"/s);
  if (!ok) throw new Error("unreadable canister reply: " + out.slice(0, 300));
  return ok[1].replaceAll('\\"', '"');
}
const canisterKey = () => hexToBytes(unwrap(dfx("canister", "call", "autobuyer", "taproot_key")));
const canisterSign = (sighash) =>
  hexToBytes(unwrap(dfx("canister", "call", "autobuyer", "rehearse_sign_sighash", `("${NETWORK_NAME}", "${bytesToHex(sighash)}")`)));

/// The encoder under test, reached the way the buyer would reach it: it is the canister's Rust, not a second
/// implementation in this file that would only ever agree with itself.
///
/// …and then checked against `runestone.mjs`, which IS a second implementation, written from the protocol description
/// rather than from the Rust, with hand-computed constants in its tests. Two independent encoders agreeing on the bytes is
/// evidence; one encoder agreeing with its own decoder is not, and that distinction is the whole subject of this rehearsal.
function encodeRunestone(spec) {
  const fromCanister = hexToBytes(unwrap(dfx("canister", "call", "autobuyer", "rehearse_runestone_encode", `("${JSON.stringify(spec).replaceAll('"', '\\"')}")`)));
  const fromReference = referenceScript(spec);
  if (bytesToHex(fromCanister) !== bytesToHex(fromReference)) {
    throw new Error(
      `refused: the two encoders disagree.\n  canister-rs: ${bytesToHex(fromCanister)}\n  runestone.mjs: ${bytesToHex(fromReference)}`
    );
  }
  return fromCanister;
}

/// …and the decoder, given scripts read back off a block.
const decodeRunestone = (scriptsHex) =>
  JSON.parse(unwrap(dfx("canister", "call", "autobuyer", "rehearse_runestone_decode", `("${JSON.stringify(scriptsHex).replaceAll('"', '\\"')}")`)));

// ---- chain data ---------------------------------------------------------------------------------------------------------------

async function api(path, init) {
  const url = assertSignetEndpoint(`${API}${path}`);
  const res = await fetch(url, { ...init, headers: { accept: "application/json", ...(init?.headers ?? {}) } });
  const text = (await res.text()).trim();
  if (!res.ok) throw new Error(`${url} -> HTTP ${res.status}: ${text.slice(0, 300)}`);
  return text;
}
const apiJson = async (p) => JSON.parse(await api(p));

/// The ord indexer. A different party, a different codebase, and the only one here with an opinion about what a runestone
/// means. Its host is on `signet.mjs`'s allowlist like every other endpoint.
async function ord(path) {
  const url = assertSignetEndpoint(`${ORD}${path}`);
  const res = await fetch(url, { headers: { accept: "application/json" } });
  const text = (await res.text()).trim();
  if (!res.ok) throw new Error(`${url} -> HTTP ${res.status}: ${text.slice(0, 300)}`);
  const body = JSON.parse(text);
  if (body.code !== 0) throw new Error(`${url} -> ${text.slice(0, 300)}`);
  return body.data;
}

const runeBalance = async (address) => {
  const d = await ord(`/v1/indexer/address/${address}/runes/balance-list?start=0&limit=50`);
  return (d.detail ?? []).find((r) => r.runeid === RUNE) ?? null;
};

// ---- building and signing, shared by both transactions ------------------------------------------------------------------------

/// Pick the largest confirmed UTXO that carries no rune we care about, so a spend never moves the asset by accident.
async function fundingUtxo(address, avoid = []) {
  const utxos = await apiJson(`/address/${address}/utxo`);
  const usable = utxos
    .filter((u) => u.status?.confirmed)
    .filter((u) => !avoid.some((a) => a.txid === u.txid && a.vout === u.vout))
    .sort((a, b) => b.value - a.value);
  if (!usable.length) throw new Error(`no confirmed spendable UTXO at ${address}`);
  return usable[0];
}

/// Build, sign through the threshold key, and broadcast. `outputs` are `{ address, value }` or `{ script, value }`, in the
/// order the edicts' output indices refer to.
async function buildSignBroadcast({ label, outputs, opReturn, feeRate = 1n }) {
  const key = canisterKey();
  const pay = buyerAddress(key, SIGNET);
  const address = assertSignetAddress(pay.address);
  const utxo = await fundingUtxo(address);

  const tx = new btc.Transaction({ allowUnknownOutputs: true });
  tx.addInput({
    txid: hexToBytes(utxo.txid),
    index: utxo.vout,
    witnessUtxo: { script: pay.script, amount: BigInt(utxo.value) },
    tapInternalKey: key,
  });

  let spent = 0n;
  for (const o of outputs) {
    if (o.script) tx.addOutput({ script: o.script, amount: BigInt(o.value) });
    else tx.addOutputAddress(o.address, BigInt(o.value), SIGNET);
    spent += BigInt(o.value);
  }
  const opReturnIndex = outputs.length;
  tx.addOutput({ script: opReturn, amount: 0n });

  // One P2TR input, key path (~58 vB) + overhead (~11) + each P2TR output (43) + the OP_RETURN.
  const vbytes = BigInt(69 + 43 * (outputs.length + 1) + opReturn.length);
  const fee = vbytes * feeRate;
  const change = BigInt(utxo.value) - spent - fee;
  if (change < 546n) throw new Error(`refused: ${utxo.value} sat leaves only ${change} sat of change`);
  tx.addOutputAddress(address, change, SIGNET);
  const changeIndex = opReturnIndex + 1;

  log(bold(`\n${label} -- build`));
  log(`   in    ${utxo.txid}:${utxo.vout}  ${utxo.value} sat`);
  outputs.forEach((o, i) => log(`   out ${i} ${o.address ?? "script"}  ${o.value} sat`));
  log(`   out ${opReturnIndex} OP_RETURN runestone  0 sat   ${bytesToHex(opReturn)}`);
  log(`   out ${changeIndex} change -> ${address}  ${change} sat`);
  log(`   fee   ${fee} sat (${feeRate} sat/vB x ~${vbytes} vB)`);

  const sighash = tx.preimageWitnessV1(0, [pay.script], btc.SigHash.DEFAULT, [BigInt(utxo.value)]);
  const sig = canisterSign(sighash);
  if (sig.length !== 64) throw new Error(`expected 64 signature bytes, got ${sig.length}`);
  if (!verifyBip340(sig, sighash, pay.tweakedPubkey)) throw new Error("refused: signature does not verify under the output key");
  if (verifyBip340(sig, sighash, key)) throw new Error("refused: signature verifies under the INTERNAL key, so the tweak was not applied");
  tx.updateInput(0, { tapKeySig: sig });
  tx.finalize();

  const raw = bytesToHex(tx.extract());
  log(bold(`${label} -- broadcast`));
  const accepted = await api("/tx", { method: "POST", body: raw, headers: { "content-type": "text/plain" } });
  if (accepted !== tx.id) throw new Error(`refused: node returned ${accepted}, we built ${tx.id}`);
  log(`   a signet node relayed it: ${bold(accepted)}`);
  return { txid: accepted, raw, fee: Number(fee), opReturnIndex, changeIndex, address, spentFrom: utxo };
}

/// Wait for a transaction, then re-read the raw bytes off the chain and give the canister's DECODER the scripts a block
/// holds -- not the ones this process remembered.
async function confirmAndDecode(txid) {
  log(bold(`\nWaiting for ${txid} to be mined`));
  for (let i = 0; ; i++) {
    const s = await apiJson(`/tx/${txid}/status`);
    if (s.confirmed) {
      const txids = await apiJson(`/block/${s.block_hash}/txids`);
      if (!txids.includes(txid)) throw new Error("the status said confirmed but the block does not list it");
      const full = await apiJson(`/tx/${txid}`);
      const scripts = full.vout.map((o) => o.scriptpubkey);
      log(`   ${bold("confirmed")} in block ${s.block_height} (${s.block_hash})`);
      log(`   listed in that block's own txid list: true`);
      log(bold("\n   what the canister's own decoder reads back from the chain's bytes"));
      const decoded = decodeRunestone(scripts);
      log(`   ${JSON.stringify(decoded)}`);
      return { blockHeight: s.block_height, blockHash: s.block_hash, blockTime: s.block_time, scripts, decoded };
    }
    process.stdout.write(`\r   in mempool, not yet mined (${i + 1} checks, tip ${await api("/blocks/tip/height")})   `);
    await new Promise((r) => setTimeout(r, 20_000));
  }
}

// ---- phases ---------------------------------------------------------------------------------------------------------------------

async function mint() {
  const pay = buyerAddress(canisterKey(), SIGNET);
  const address = assertSignetAddress(pay.address);
  log(bold(`\n1. Mint ${RUNE_NAME} (${RUNE}) -- a runestone and nothing else`));
  log(`   buyer address ${address}`);

  // The minted amount is unallocated, so it follows the pointer. Output 0 is a plain 546-sat output of ours; the pointer
  // names it explicitly rather than leaning on "the first non-OP_RETURN output", which is a default.
  const spec = { mint: RUNE, pointer: 0 };
  const opReturn = encodeRunestone(spec);
  log(`   spec       ${JSON.stringify(spec)}`);
  log(`   encoded    ${bytesToHex(opReturn)}`);
  log(`   agreed on by canister-rs AND runestone.mjs, two implementations written from different sources`);
  if (opReturn[0] !== 0x6a || opReturn[1] !== 0x5d) throw new Error("refused: that is not an OP_RETURN OP_13 script");

  const r = await buildSignBroadcast({ label: "2. Mint", outputs: [{ address, value: 546 }], opReturn });
  save({ rune: RUNE, runeName: RUNE_NAME, address, mint: { ...r, spec, opReturn: bytesToHex(opReturn) } });
  log(`\n   Next: node runes-rehearsal.mjs confirm`);
}

async function confirm() {
  const st = load();
  if (!st.mint) throw new Error("nothing minted; run `mint` first");
  const before = await runeBalance(st.address);
  const c = await confirmAndDecode(st.mint.txid);
  if (c.decoded.cenotaph) throw new Error("the canister's own decoder calls this a cenotaph: the runes would be burned");

  log(bold(`\n3. Ask an ord indexer what that transaction MEANT`));
  log(`   a node relayed the bytes. Only an indexer has an opinion about the rune.`);
  let after = null;
  for (let i = 0; i < 30; i++) {
    after = await runeBalance(st.address);
    if (after) break;
    process.stdout.write(`\r   indexer has not credited it yet (${i + 1} checks)   `);
    await new Promise((r) => setTimeout(r, 20_000));
  }
  log("");
  if (!after) {
    log(`   ${bold("the indexer credits nothing.")} The OP_RETURN is in a block; the rune did not move.`);
    save({ mintConfirmed: { ...c, indexerBalance: null, balanceBefore: before?.amount ?? null } });
    throw new Error("the mint was relayed and mined but no rune was credited -- this is the floor, not the ceiling");
  }
  log(`   ${bold(`the indexer credits ${after.amount} ${after.spacedRune} to ${st.address}`)}`);
  log(`   rune id ${after.runeid}, divisibility ${after.divisibility}`);
  save({ mintConfirmed: { ...c, indexerBalance: after, balanceBefore: before?.amount ?? null } });
  log(`\n   Next: node runes-rehearsal.mjs transfer`);
}

async function transfer() {
  const st = load();
  if (!st.mintConfirmed?.indexerBalance) throw new Error("no indexed rune balance; run `confirm` first");
  const address = assertSignetAddress(st.address);

  // The rune sits on the mint's output 0. Spend exactly that, so the edict has something to move.
  const holding = { txid: st.mint.txid, vout: 0 };
  log(bold(`\n4. Transfer 1 ${RUNE_NAME} by EDICT`));
  log(`   the rune is on ${holding.txid}:${holding.vout}`);

  const spec = { edicts: [{ id: RUNE, amount: "1", output: 0 }], pointer: 2 };
  const opReturn = encodeRunestone(spec);
  log(`   spec       ${JSON.stringify(spec)}`);
  log(`   encoded    ${bytesToHex(opReturn)}   (both encoders agree)`);

  // Two inputs: the rune-bearing output, and a fee UTXO. The edict names output 0, which is where the asset must land.
  const key = canisterKey();
  const pay = buyerAddress(key, SIGNET);
  const runeUtxo = (await apiJson(`/address/${address}/utxo`)).find((u) => u.txid === holding.txid && u.vout === holding.vout);
  if (!runeUtxo) throw new Error(`refused: ${holding.txid}:${holding.vout} is not an unspent output of ${address}`);
  const feeUtxo = await fundingUtxo(address, [holding]);

  const tx = new btc.Transaction({ allowUnknownOutputs: true });
  for (const u of [runeUtxo, feeUtxo]) {
    tx.addInput({
      txid: hexToBytes(u.txid),
      index: u.vout,
      witnessUtxo: { script: pay.script, amount: BigInt(u.value) },
      tapInternalKey: key,
    });
  }
  const recipientValue = 546n;
  tx.addOutputAddress(address, recipientValue, SIGNET); // output 0: the edict's named recipient
  tx.addOutput({ script: opReturn, amount: 0n }); // output 1: the runestone
  const inTotal = BigInt(runeUtxo.value) + BigInt(feeUtxo.value);
  const vbytes = BigInt(11 + 2 * 58 + 43 * 2 + opReturn.length);
  const fee = vbytes * 1n;
  const change = inTotal - recipientValue - fee;
  if (change < 546n) throw new Error(`refused: change would be ${change} sat`);
  tx.addOutputAddress(address, change, SIGNET); // output 2: change, which the pointer names

  log(`   in  ${runeUtxo.txid}:${runeUtxo.vout} ${runeUtxo.value} sat (holds the rune)`);
  log(`   in  ${feeUtxo.txid}:${feeUtxo.vout} ${feeUtxo.value} sat (fees)`);
  log(`   out 0 recipient ${recipientValue} sat  <- the edict names this output`);
  log(`   out 1 OP_RETURN runestone 0 sat`);
  log(`   out 2 change ${change} sat  <- the pointer names this one`);
  log(`   fee ${fee} sat`);

  const amounts = [BigInt(runeUtxo.value), BigInt(feeUtxo.value)];
  for (let i = 0; i < 2; i++) {
    const sighash = tx.preimageWitnessV1(i, [pay.script, pay.script], btc.SigHash.DEFAULT, amounts);
    const sig = canisterSign(sighash);
    if (!verifyBip340(sig, sighash, pay.tweakedPubkey)) throw new Error(`refused: input ${i} signature does not verify`);
    tx.updateInput(i, { tapKeySig: sig });
  }
  tx.finalize();
  const raw = bytesToHex(tx.extract());
  const accepted = await api("/tx", { method: "POST", body: raw, headers: { "content-type": "text/plain" } });
  if (accepted !== tx.id) throw new Error(`refused: node returned ${accepted}, we built ${tx.id}`);
  log(`\n   a signet node relayed it: ${bold(accepted)}`);
  save({ transfer: { txid: accepted, raw, spec, opReturn: bytesToHex(opReturn), recipientOutput: 0, pointerOutput: 2 } });
  log(`   Next: node runes-rehearsal.mjs verify`);
}

async function verify() {
  const st = load();
  if (!st.transfer) throw new Error("nothing transferred; run `transfer` first");
  const c = await confirmAndDecode(st.transfer.txid);
  if (c.decoded.cenotaph) throw new Error("the decoder calls the mined transaction a cenotaph");
  const edict = c.decoded.edicts[0];
  log(bold("\n5. Does the chain's own copy say what we encoded?"));
  log(`   encoded  ${JSON.stringify(st.transfer.spec.edicts[0])}`);
  log(`   decoded  ${JSON.stringify(edict)}`);
  if (!edict || edict.id !== st.rune || edict.amount !== "1" || edict.output !== 0) {
    throw new Error("the decoded edict is not the one that was encoded");
  }
  log(`   ${bold("they match")} -- and these are the bytes a block holds, re-read, not the ones we kept.`);

  log(bold("\n6. Does the indexer agree the asset is on the output the edict named?"));
  let placed = null;
  for (let i = 0; i < 30; i++) {
    // The rune id contains a colon, which must be percent-encoded in a path segment. Checked against the live
    // indexer rather than assumed: the unencoded form is not reliably accepted.
    const utxos = await ord(
      `/v1/indexer/address/${st.address}/runes/${encodeURIComponent(st.rune)}/utxo?start=0&limit=50`
    ).catch(() => null);
    placed = (utxos?.utxo ?? []).find((u) => u.txid === st.transfer.txid && u.vout === st.transfer.recipientOutput);
    if (placed) break;
    process.stdout.write(`\r   indexer has not placed it yet (${i + 1} checks)   `);
    await new Promise((r) => setTimeout(r, 20_000));
  }
  log("");
  if (!placed) {
    save({ transferConfirmed: { ...c, placed: null } });
    throw new Error("the indexer does not place the rune on the output the edict named");
  }
  const amount = placed.runes?.[0]?.amount ?? "?";
  log(`   ${bold(`the indexer puts ${amount} ${st.runeName} on ${placed.txid}:${placed.vout}`)}`);
  log(`   which is output ${st.transfer.recipientOutput} -- the one the edict named, not the pointer's.`);
  save({ transferConfirmed: { ...c, placed } });
  log(`\n   https://mempool.space/signet/tx/${st.transfer.txid}`);
}


/// An experiment, not a rehearsal step.
///
/// `Runestone`'s `cenotaph` field is documented as "true when a tag we do not understand appeared", and `decode()` never
/// sets it for one: it reads every unrecognised tag's value and carries on. The Runes protocol's forward-compatibility
/// rule is the usual one -- an unrecognised ODD tag is ignored, an unrecognised EVEN tag makes the runestone a cenotaph
/// and burns what it carries. If that rule holds, the decoder has the same hole that `payload_of` had: a venue adds one
/// even tag, this code reads "delivers N units to the winner", and `ord` burns the lot.
///
/// Rather than implement a tag allowlist from memory of a specification, mint the same rune again with an unrecognised
/// even tag attached and ask the indexer what happened. The outcomes are distinguishable: if the tag is ignored the
/// balance goes up by one, and if it is a cenotaph the mint does not happen and the balance does not move. Nothing is at
/// risk -- a failed mint costs a fee.
async function probe() {
  const st = load();
  const pay = buyerAddress(canisterKey(), SIGNET);
  const address = assertSignetAddress(pay.address);
  const TAG = 254; // even, large, and not assigned by anything we know of

  const before = await runeBalance(address);
  log(bold(`\n7. Probe: what does an ord indexer do with an unrecognised EVEN tag?`));
  log(`   balance before: ${before?.amount ?? 0} ${RUNE_NAME}`);

  const spec = { mint: RUNE, pointer: 0, unknownTag: { tag: TAG, value: 0 } };
  const opReturn = referenceScript(spec);
  log(`   a mint of ${RUNE}, plus tag ${TAG} = 0`);
  log(`   encoded by runestone.mjs (the canister's encoder cannot emit an unknown tag, and should not be able to)`);
  log(`   ${bytesToHex(opReturn)}`);

  const decoded = decodeRunestone([bytesToHex(opReturn)]);
  log(`\n   what the buyer's own decoder makes of it: ${JSON.stringify(decoded)}`);
  log(`   ${decoded.cenotaph ? "it calls it a cenotaph" : bold("it does NOT call it a cenotaph")}`);

  const r = await buildSignBroadcast({ label: "   probe", outputs: [{ address, value: 546 }], opReturn });
  save({ probe: { tag: TAG, ...r, opReturn: bytesToHex(opReturn), decoded, balanceBefore: before?.amount ?? "0" } });
  log(`\n   broadcast. Run \`node runes-rehearsal.mjs probe-verdict\` once it is mined.`);
}

/// Read the indexer's answer to the probe.
async function probeVerdict() {
  const st = load();
  if (!st.probe) throw new Error("nothing probed; run `probe` first");
  const c = await confirmAndDecode(st.probe.txid);
  const before = BigInt(st.probe.balanceBefore || "0");
  let after = before;
  for (let i = 0; i < 20; i++) {
    const b = await runeBalance(st.address);
    after = BigInt(b?.amount ?? "0");
    if (after !== before) break;
    process.stdout.write(`\r   indexer still says ${after} (${i + 1} checks)   `);
    await new Promise((r) => setTimeout(r, 20_000));
  }
  log("");
  log(bold("\n8. The verdict"));
  log(`   balance before the probe: ${before}`);
  log(`   balance after:            ${after}`);
  const minted = after > before;
  if (minted) {
    log(`   ${bold(`the indexer IGNORED tag ${st.probe.tag} and minted anyway.`)}`);
    log(`   So an unrecognised even tag is not a cenotaph here, and the decoder's behaviour matches the indexer's.`);
  } else {
    log(`   ${bold(`the indexer did NOT mint: tag ${st.probe.tag} made it a cenotaph.`)}`);
    log(`   The buyer's decoder read the same bytes as a valid runestone. That is a real divergence and the`);
    log(`   same shape as the payload_of hole: the decoder would report a delivery that ord burns.`);
  }
  save({ probeVerdict: { ...c, before: before.toString(), after: after.toString(), mintedAnyway: minted } });
}

const status = async () => log(JSON.stringify(load(), null, 2));

const phases = { mint, confirm, transfer, verify, probe, "probe-verdict": probeVerdict, status };
const which = process.argv[2];
if (!phases[which]) {
  console.error(`usage: node runes-rehearsal.mjs <${Object.keys(phases).join("|")}>`);
  process.exit(2);
}
await phases[which]();
