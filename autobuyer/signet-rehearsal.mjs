// The signet rehearsal. Pre-registered in `docs/v2/2026-09-22-signet-rehearsal.md`.
//
//   node signet-rehearsal.mjs derive    -- ask the canister for its key, build the signet address, print it and the faucet
//   node signet-rehearsal.mjs watch     -- poll signet until the operator's funding UTXO appears
//   node signet-rehearsal.mjs spend     -- build a self-send, sign it with the threshold key, broadcast it
//   node signet-rehearsal.mjs confirm   -- wait for the transaction to appear in a block
//   node signet-rehearsal.mjs status    -- print what is known so far
//
// SIGNET ONLY. See `signet.mjs` for the guards; they run on every phase, not once at the start.
//
// Chain data comes from mempool.space's signet API. UniSat's signet host also answers reads, but it needs SIGNET_KEY, and a
// step that cannot be run without asking someone else to paste a secret is a step that will not be re-run. mempool.space
// needs no key, serves UTXOs, raw-transaction broadcast and block confirmation from one host, and is the same explorer the
// canister's `btc_api_urls` names. Nothing here is trusted from one source that matters: the confirmation is re-read from
// the block, not from the broadcast reply.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import * as btc from "@scure/btc-signer";
import { assertSignetEndpoint, assertSignetAddress, buyerAddress, NETWORK_NAME, SIGNET } from "./signet.mjs";
import { taprootOutputKey, verifyBip340, bytesToHex, hexToBytes } from "./lib.mjs";

const DFX = join(homedir(), "Library/Application Support/org.dfinity.dfx/bin");
const CARGO = join(homedir(), ".cargo/bin");
const ENV = { ...process.env, PATH: `${DFX}:${CARGO}:${process.env.PATH}` };
const RS = join(import.meta.dirname, "canister-rs");
const STATE = join(import.meta.dirname, ".signet-rehearsal.json");
const API = "https://mempool.space/signet/api";
const FAUCET = "https://signetfaucet.com/";

const load = () => (existsSync(STATE) ? JSON.parse(readFileSync(STATE, "utf8")) : {});
const save = (o) => writeFileSync(STATE, JSON.stringify({ ...load(), ...o }, null, 2) + "\n");
const log = (s = "") => console.log(s);
const bold = (s) => `\x1b[1m${s}\x1b[0m`;

// ---- the canister, through the buyer's own entry points ---------------------------------------------------------------------

const dfx = (...args) => execFileSync("dfx", args, { cwd: RS, env: ENV, encoding: "utf8", timeout: 180_000 });
/// Which canister: the funded one's KEY is per canister, so naming the wrong one signs with a key that owns nothing. Defaults to
/// this project's `autobuyer`; `CANISTER=<id>` names another (the funded one lives in another dfx project), with this
/// project's interface file so the new methods decode.
const CANISTER = process.env.CANISTER?.trim() || "autobuyer";
const CANDID = join(RS, "etch_autobuyer.did");
const call = (method, arg) => dfx("canister", "call", "--candid", CANDID, CANISTER, method, ...(arg === undefined ? [] : [arg]));
/// `(variant { Ok = "…" })` -> the string; `Err` is thrown, because a refusal is a result and must not read as a value.
function unwrap(out) {
  const err = out.match(/Err = "((?:[^"\\]|\\.)*)"/s);
  if (err) throw new Error("canister refused: " + err[1].replaceAll('\\"', '"'));
  const ok = out.match(/Ok = "([^"]*)"/);
  if (!ok) throw new Error("unreadable canister reply: " + out.slice(0, 200));
  return ok[1];
}
/// The x-only key `schnorr_public_key` returns. NOTE: this is the canister's UNTWEAKED (internal) key -- see `checkKeyKind`.
const canisterInternalKey = () => hexToBytes(unwrap(call("taproot_key")));
const canisterSign = (sighash) =>
  hexToBytes(
    unwrap(
      call("rehearse_sign_sighash", `("${NETWORK_NAME}", "${bytesToHex(sighash)}")`)
    )
  );

// ---- signet chain data ------------------------------------------------------------------------------------------------------

async function api(path, init) {
  const url = assertSignetEndpoint(`${API}${path}`);
  const res = await fetch(url, { ...init, headers: { accept: "application/json", ...(init?.headers ?? {}) } });
  const text = (await res.text()).trim();
  if (!res.ok) throw new Error(`${url} -> HTTP ${res.status}: ${text.slice(0, 300)}`);
  return text;
}
const apiJson = async (p) => JSON.parse(await api(p));

// ---- the preflight that a test in this process could never do ---------------------------------------------------------------

/// Decide, from the subnet's own behaviour, whether `taproot_key()` returns the internal key or the output key -- and refuse
/// to go further unless the address we are about to publish commits to the key the subnet will actually sign under.
///
/// This is the whole reason for broadcasting. A test that signs and then verifies with the same assumption it used to build
/// the address agrees with itself no matter which key it had. Bitcoin does not.
function checkKeyKind(key, pay) {
  const probe = new Uint8Array(32).fill(0xa5);
  const sig = canisterSign(probe);
  if (sig.length !== 64) throw new Error(`a BIP340 signature is 64 bytes; got ${sig.length}`);
  const underTweaked = verifyBip340(sig, probe, taprootOutputKey(key));
  const underRaw = verifyBip340(sig, probe, key);
  const kind = underTweaked ? "internal (untweaked)" : underRaw ? "already the output key" : "neither";
  log(`   taproot_key() returns the ${bold(kind)} key`);
  log(`     signature verifies under tweak(key): ${underTweaked}`);
  log(`     signature verifies under key:        ${underRaw}`);
  if (!underTweaked) {
    throw new Error(
      "refused to publish an address: the subnet's signature does not verify under the BIP341 tweak of the key " +
        "taproot_key() returned, so the scriptPubKey this address commits to could not be spent by it. " +
        (underRaw
          ? "The key came back ALREADY TWEAKED, and p2tr would tweak it a second time."
          : "The signature verifies under neither key.")
    );
  }
  // and the address must commit to exactly that tweaked key
  if (bytesToHex(pay.tweakedPubkey) !== bytesToHex(taprootOutputKey(key))) {
    throw new Error("refused: the address's output key is not the tweak of the canister's key");
  }
  if (verifyBip340(sig, new Uint8Array(32), taprootOutputKey(key))) {
    throw new Error("refused: that signature verifies for a different message too, which is impossible for BIP340");
  }
  return { underTweaked, underRaw, kind };
}

// ---- phases -----------------------------------------------------------------------------------------------------------------

async function derive() {
  log(bold("\n1. The buyer's signet address, from a key nobody holds"));
  const key = canisterInternalKey();
  log(`   canister taproot_key(): ${bytesToHex(key)}`);
  const pay = buyerAddress(key, SIGNET); // asserts signet params AND decodes the address it produced
  const probe = checkKeyKind(key, pay);
  log(`   output key (tweaked):   ${bytesToHex(pay.tweakedPubkey)}`);
  log(`   scriptPubKey:           ${bytesToHex(pay.script)}`);
  log(`\n   ${bold("address: " + pay.address)}`);
  log(`   (bech32m HRP "tb", witness v1, 32-byte program -- checked by decoding, not by trusting the variable)`);
  const tip = await api("/blocks/tip/height");
  log(`   signet tip at derivation: ${tip}`);
  save({
    internalKey: bytesToHex(key),
    outputKey: bytesToHex(pay.tweakedPubkey),
    script: bytesToHex(pay.script),
    address: pay.address,
    probe,
    derivedAtTip: Number(tip),
    derivedAt: new Date().toISOString(),
  });
  log(bold("\n2. Funding -- the operator's to do, not this script's"));
  log(`   Send signet coin to      ${pay.address}`);
  log(`   Faucet                   ${FAUCET}`);
  log(`   Then run                 node signet-rehearsal.mjs watch`);
  log(`\n   No faucet is automated here. A faucet that a script drains is a faucet that stops existing.`);
}

async function watch() {
  const st = load();
  const address = assertSignetAddress(st.address ?? "");
  log(bold(`\nWatching ${address} for a funding UTXO`));
  for (let i = 0; ; i++) {
    const utxos = await apiJson(`/address/${address}/utxo`);
    if (utxos.length) {
      const u = utxos.sort((a, b) => b.value - a.value)[0];
      log(`\n   ${bold("funded")}: ${u.txid}:${u.vout} = ${u.value} sat` + (u.status?.confirmed ? ` (block ${u.status.block_height})` : " (unconfirmed)"));
      save({ utxo: { txid: u.txid, vout: u.vout, value: u.value, confirmed: !!u.status?.confirmed, fundingBlock: u.status?.block_height ?? null } });
      log(`   Next: node signet-rehearsal.mjs spend`);
      return;
    }
    process.stdout.write(`\r   nothing yet (${i + 1} checks, tip ${await api("/blocks/tip/height")})   `);
    await new Promise((r) => setTimeout(r, 20_000));
  }
}

async function spend() {
  const st = load();
  const address = assertSignetAddress(st.address ?? "");
  const key = hexToBytes(st.internalKey);
  const pay = buyerAddress(key, SIGNET);
  if (pay.address !== address) throw new Error("refused: the canister's key no longer derives the funded address");
  if (!st.utxo) throw new Error("no UTXO recorded; run `watch` first");
  const { txid, vout, value } = st.utxo;

  // Confirm the UTXO is still there and still ours, from chain data rather than from our own file.
  const live = await apiJson(`/address/${address}/utxo`);
  const found = live.find((u) => u.txid === txid && u.vout === vout);
  if (!found) throw new Error(`refused: ${txid}:${vout} is no longer an unspent output of ${address}`);
  if (found.value !== value) throw new Error(`refused: value changed from ${value} to ${found.value}`);

  const feeRates = await apiJson("/v1/fees/recommended").catch(() => ({ halfHourFee: 2 }));
  // One P2TR input, one P2TR output, key-path: ~111 vB. Signet is empty, but pay a real rate rather than a magic 1.
  const vbytes = 111n;
  const rate = BigInt(Math.max(1, Math.ceil(feeRates.halfHourFee ?? 2)));
  const fee = vbytes * rate;
  const send = BigInt(value) - fee;
  if (send < 546n) throw new Error(`refused: ${value} sat minus ${fee} sat fee leaves dust`);

  log(bold("\n3. Build -- a self-send, spending the funded UTXO"));
  log(`   in   ${txid}:${vout}  ${value} sat`);
  log(`   out  ${address}  ${send} sat`);
  log(`   fee  ${fee} sat (${rate} sat/vB x ${vbytes} vB)`);

  const tx = new btc.Transaction();
  tx.addInput({
    txid: hexToBytes(txid),
    index: vout,
    witnessUtxo: { script: pay.script, amount: BigInt(value) },
    tapInternalKey: key,
  });
  tx.addOutputAddress(address, send, SIGNET);
  const sighash = tx.preimageWitnessV1(0, [pay.script], btc.SigHash.DEFAULT, [BigInt(value)]);

  log(bold("\n4. Sign -- with the threshold key, through the buyer's own signing path"));
  log(`   sighash ${bytesToHex(sighash)}`);
  const sig = canisterSign(sighash);
  if (sig.length !== 64) throw new Error(`expected 64 signature bytes, got ${sig.length}`);
  if (!verifyBip340(sig, sighash, pay.tweakedPubkey)) throw new Error("refused: signature does not verify under the output key");
  if (verifyBip340(sig, sighash, key)) throw new Error("refused: signature verifies under the INTERNAL key, so the tweak was not applied");
  log(`   64 bytes, verify under the tweaked output key: true; under the internal key: false`);

  tx.updateInput(0, { tapKeySig: sig });
  tx.finalize();
  const raw = bytesToHex(tx.extract());
  log(`   finalised ${raw.length / 2} bytes, txid ${bold(tx.id)}`);

  log(bold("\n5. Broadcast to signet"));
  const accepted = await api("/tx", { method: "POST", body: raw, headers: { "content-type": "text/plain" } });
  log(`   node accepted it and returned txid ${bold(accepted)}`);
  if (accepted !== tx.id) throw new Error(`refused: node returned ${accepted}, we built ${tx.id}`);
  save({ spend: { txid: accepted, raw, fee: Number(fee), sent: Number(send), broadcastAt: new Date().toISOString() } });
  log(`   Next: node signet-rehearsal.mjs confirm`);
}

async function confirm() {
  const st = load();
  if (!st.spend) throw new Error("nothing broadcast; run `spend` first");
  const id = st.spend.txid;
  log(bold(`\n6. Waiting for ${id} to be mined`));
  for (let i = 0; ; i++) {
    const s = await apiJson(`/tx/${id}/status`);
    if (s.confirmed) {
      // Re-read from the block rather than believing the status object: the claim is "a block contains it".
      const txids = await apiJson(`/block/${s.block_hash}/txids`);
      const inBlock = txids.includes(id);
      const tip = Number(await api("/blocks/tip/height"));
      log(`\n   ${bold("confirmed")}`);
      log(`   txid         ${id}`);
      log(`   block height ${s.block_height}`);
      log(`   block hash   ${s.block_hash}`);
      log(`   block time   ${new Date(s.block_time * 1000).toISOString()}`);
      log(`   listed in that block's own txid list: ${bold(String(inBlock))}`);
      log(`   confirmations at tip ${tip}: ${tip - s.block_height + 1}`);
      if (!inBlock) throw new Error("the status said confirmed but the block does not list it");
      save({ confirmed: { blockHeight: s.block_height, blockHash: s.block_hash, blockTime: s.block_time, inBlockTxidList: inBlock, tipWhenChecked: tip } });
      log(`\n   https://mempool.space/signet/tx/${id}`);
      return;
    }
    process.stdout.write(`\r   in mempool, not yet mined (${i + 1} checks, tip ${await api("/blocks/tip/height")})   `);
    await new Promise((r) => setTimeout(r, 20_000));
  }
}

const status = async () => log(JSON.stringify(load(), null, 2));


/// Bitcoin broadcast (docs/v2/btc-broadcast.md, pre-registered): the same self-send as `spend`, of the pre-registered UTXO,
/// built and signed the same way -- but THE CANISTER broadcasts it (one node sends, every node reads back), then the canister is
/// asked, replicated, until it is confirmed, and mempool.space is asked by this script, not the canister. Then the identical
/// bytes go through the canister again. `UTXO=<txid>:<vout>` names the coin.
const callJson = (method, arg) => { const out = dfx("canister", "call", "--candid", CANDID, "--output", "json", CANISTER, method, arg);
  return JSON.parse(out.slice(out.lastIndexOf("\n{", out.length - 3) + 1)); };
const pairs = (xs) => xs.map((p) => (Array.isArray(p) ? p : [p["0"], p["1"]]));
async function canisterSpend() {
  const st = load();
  const address = assertSignetAddress(st.address ?? "");
  const key = hexToBytes(st.internalKey);
  const pay = buyerAddress(key, SIGNET);
  if (pay.address !== address) throw new Error("refused: the key on file no longer derives the funded address");
  if (bytesToHex(canisterInternalKey()) !== st.internalKey) throw new Error(`refused: canister ${CANISTER} does not hold this address's key`);
  const [utxoTxid, utxoVout] = (process.env.UTXO ?? "").split(":");
  if (!utxoTxid || utxoVout === undefined) throw new Error("name the coin: UTXO=<txid>:<vout>");
  const live = await apiJson(`/address/${address}/utxo`);
  const found = live.find((u) => u.txid === utxoTxid && u.vout === Number(utxoVout));
  if (!found) throw new Error(`refused: ${utxoTxid}:${utxoVout} is not an unspent output of ${address}`);
  const value = found.value;
  const feeRates = await apiJson("/v1/fees/recommended").catch(() => ({ halfHourFee: 2 }));
  const rate = BigInt(Math.max(1, Math.ceil(feeRates.halfHourFee ?? 2))), fee = 111n * rate, send = BigInt(value) - fee;
  if (send < 546n) throw new Error(`refused: ${value} sat minus ${fee} sat fee leaves dust`);
  log(bold("\n3. Build -- a self-send of the pre-registered coin"));
  log(`   in   ${utxoTxid}:${utxoVout}  ${value} sat\n   out  ${address}  ${send} sat\n   fee  ${fee} sat`);
  const tx = new btc.Transaction();
  tx.addInput({ txid: hexToBytes(utxoTxid), index: Number(utxoVout), witnessUtxo: { script: pay.script, amount: BigInt(value) }, tapInternalKey: key });
  tx.addOutputAddress(address, send, SIGNET);
  const sighash = tx.preimageWitnessV1(0, [pay.script], btc.SigHash.DEFAULT, [BigInt(value)]);
  const sig = canisterSign(sighash);
  if (!verifyBip340(sig, sighash, pay.tweakedPubkey)) throw new Error("refused: signature does not verify under the output key");
  tx.updateInput(0, { tapKeySig: sig });
  tx.finalize();
  const raw = bytesToHex(tx.extract());
  log(bold("\n4. Signed by the threshold key") + `  ${raw.length / 2} bytes, txid (script, @scure) ${bold(tx.id)}`);
  save({ canisterSpend: { txid: tx.id, raw, utxo: `${utxoTxid}:${utxoVout}`, fee: Number(fee), canister: CANISTER, at: new Date().toISOString() } });

  const send1 = (label) => {
    log(bold(`\n${label}`));
    const r = callJson("rehearse_broadcast_btc", `("signet", "${raw}")`);
    if (r.Err !== undefined) throw new Error("canister refused: " + r.Err);
    for (const [url, o] of pairs(r.Ok.providers)) log(`   ${url}\n      -> ${JSON.stringify(o)}`);
    log(`   read back at once: ${JSON.stringify(r.Ok.seen)}`);
    log(`   canister's txid ${r.Ok.txid}   ${r.Ok.txid === tx.id ? bold("(same as the script's)") : bold("(DIFFERENT)")}`);
    if (r.Ok.txid !== tx.id) throw new Error("FAIL: the canister watches a different txid");
    return r.Ok;
  };
  const first = send1("5. The CANISTER broadcasts it");
  save({ canisterSpend: { ...load().canisterSpend, providers: pairs(first.providers), seenAtOnce: first.seen } });

  log(bold("\n6. Ask the canister, replicated, until it is confirmed"));
  let seen;
  for (let i = 0; i < 360; i++) {
    seen = callJson("btc_tx_status", `("${tx.id}")`);
    if (seen.Confirmed) break;
    if (i % 6 === 0) log(`   ${new Date().toISOString().slice(11, 19)} ${JSON.stringify(seen)}`);
    await new Promise((r) => setTimeout(r, 10_000));
  }
  log(`   canister says: ${JSON.stringify(seen)}`);
  if (!seen?.Confirmed) throw new Error("FAIL: the canister never read it back as confirmed");

  log(bold("\n7. mempool.space, asked by this script"));
  const s = await apiJson(`/tx/${tx.id}/status`);
  const inBlock = (await apiJson(`/block/${s.block_hash}/txids`)).includes(tx.id);
  log(`   confirmed ${s.confirmed} at ${s.block_height} in ${s.block_hash}; listed in that block: ${inBlock}`);
  const agree = s.confirmed && inBlock && Number(seen.Confirmed.height) === s.block_height && seen.Confirmed.block_hash === s.block_hash;

  const before = (await apiJson(`/address/${address}`)).chain_stats.tx_count;
  const again = send1("8. The identical bytes, through the canister again");
  const after = (await apiJson(`/address/${address}`)).chain_stats.tx_count;
  const same = again.seen?.Confirmed && Number(again.seen.Confirmed.height) === s.block_height && after === before;
  log(`   address transactions ${before} -> ${after}: ${after === before ? bold("unchanged") : bold("CHANGED")}`);
  const pass = agree && same;
  save({ canisterConfirmed: { canisterSaid: seen, explorer: { height: s.block_height, hash: s.block_hash, inBlock },
    resend: { providers: pairs(again.providers), seen: again.seen, txCountBefore: before, txCountAfter: after }, pass, at: new Date().toISOString() } });
  log(`\n   ${bold(pass ? "PASS: the canister broadcast a new signet spend, read it back confirmed, the explorer agrees, and a resend changed nothing." : "FAIL")}`);
  log(`   https://mempool.space/signet/tx/${tx.id}`);
  if (!pass) process.exit(1);
}

const phases = { derive, watch, spend, confirm, "canister-spend": canisterSpend, status };
const which = process.argv[2];
if (!phases[which]) {
  console.error(`usage: node signet-rehearsal.mjs <${Object.keys(phases).join("|")}>`);
  process.exit(2);
}
await phases[which]();
