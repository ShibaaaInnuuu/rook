// The EVM rehearsal. Pre-registered in `docs/v2/2026-09-22-evm-and-runes-rehearsals.md`.
//
//   node evm-rehearsal.mjs derive    -- ask the canister for its EVM address, check the node's chain id, print both
//   node evm-rehearsal.mjs watch     -- poll chain 46630 until the operator's funding arrives
//   node evm-rehearsal.mjs send      -- build a zero-value self-send, sign it with the threshold key, recover the sender
//                                       independently, then broadcast
//   node evm-rehearsal.mjs confirm   -- wait for the block, and read back the sender the NODE recovered
//   node evm-rehearsal.mjs canister-send -- B4: the same self-send, broadcast BY THE CANISTER and read back replicated
//   node evm-rehearsal.mjs status    -- print what is known so far
//
// CHAIN 46630 ONLY (Robinhood Chain testnet). See `evm.mjs` for the guards; they run on every phase, not once at the start.
//
// The claim being tested is narrow and the evidence for it is one field. `sign_with_ecdsa` returns r‖s and no recovery
// parameter; an EVM transaction needs `v`; a wrong `v` produces a valid transaction belonging to a DIFFERENT address. Every
// check this file can run shares an assumption with the code it is checking -- except one. `eth_getTransactionByHash(...)
// .from` is computed by the node, from the signature alone, by a party that has never heard of this repository. That field
// is the rehearsal.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import {
  CHAIN_ID,
  RPC_URL,
  EXPLORER,
  FAUCET,
  assertAddress,
  assertNodeAgreesOnChainId,
  eip1559Signed,
  recoverAddress,
  recoverBothParities,
  rpc,
  signingHash,
  txHash,
  bytesToHex,
} from "./evm.mjs";

const DFX = join(homedir(), "Library/Application Support/org.dfinity.dfx/bin");
const CARGO = join(homedir(), ".cargo/bin");
const ENV = { ...process.env, PATH: `${DFX}:${CARGO}:${process.env.PATH}` };
const RS = join(import.meta.dirname, "canister-rs");
const STATE = join(import.meta.dirname, ".evm-rehearsal.json");

const load = () => (existsSync(STATE) ? JSON.parse(readFileSync(STATE, "utf8")) : {});
const save = (o) => writeFileSync(STATE, JSON.stringify({ ...load(), ...o }, null, 2) + "\n");
const log = (s = "") => console.log(s);
const bold = (s) => `\x1b[1m${s}\x1b[0m`;

// ---- the canister, through the buyer's own entry points ---------------------------------------------------------------------

const dfx = (...args) => execFileSync("dfx", args, { cwd: RS, env: ENV, encoding: "utf8", timeout: 180_000 });

/// `(variant { Ok = … })` -> the payload; `Err` is thrown, because a refusal is a result and must not read as a value.
function unwrapText(out) {
  const err = out.match(/Err = "((?:[^"\\]|\\.)*)"/s);
  if (err) throw new Error("canister refused: " + err[1].replaceAll('\\"', '"'));
  const ok = out.match(/Ok = "([^"]*)"/);
  if (!ok) throw new Error("unreadable canister reply: " + out.slice(0, 300));
  return ok[1];
}

/// The `EvmSignature` record, out of candid's textual form. Field order is not relied on; each is found by name.
function unwrapSignature(out) {
  const err = out.match(/Err = "((?:[^"\\]|\\.)*)"/s);
  if (err) throw new Error("canister refused: " + err[1].replaceAll('\\"', '"'));
  const field = (name, re) => {
    const m = out.match(new RegExp(`${name}\\s*=\\s*${re}`));
    if (!m) throw new Error(`no ${name} in canister reply: ${out.slice(0, 400)}`);
    return m[1];
  };
  return {
    r: field("r", '"([0-9a-f]{64})"'),
    s: field("s", '"([0-9a-f]{64})"'),
    s_from_subnet: field("s_from_subnet", '"([0-9a-f]{64})"'),
    s_was_high: field("s_was_high", "(true|false)") === "true",
    recovery_id: Number(field("recovery_id", "([0-9]+)").replace(/_/g, "")),
    signer: field("signer", '"(0x[0-9a-f]{40})"'),
  };
}

const canisterEvmAddress = () => assertAddress(unwrapText(dfx("canister", "call", "autobuyer", "evm_address")));
const canisterSign = (hash) =>
  unwrapSignature(
    dfx("canister", "call", "autobuyer", "rehearse_sign_evm", `(${CHAIN_ID} : nat64, "${bytesToHex(hash)}")`)
  );

// ---- phases -----------------------------------------------------------------------------------------------------------------

async function derive() {
  log(bold("\n1. The buyer's EVM address, from a key nobody holds"));
  const chain = await assertNodeAgreesOnChainId();
  log(`   the node at ${RPC_URL}`);
  log(`   answers eth_chainId with ${bold(String(chain))} -- asked, not assumed`);

  const address = canisterEvmAddress();
  log(`\n   ${bold("address: " + address)}`);
  log(`   (the low 20 bytes of keccak(uncompressed threshold ECDSA public key), derivation path "evm")`);

  const balance = BigInt(await rpc("eth_getBalance", [address, "latest"]));
  const nonce = Number(BigInt(await rpc("eth_getTransactionCount", [address, "latest"])));
  const block = Number(BigInt(await rpc("eth_blockNumber")));
  log(`   balance ${balance} wei, nonce ${nonce}, chain head ${block}`);

  save({ chainId: chain, address, derivedAtBlock: block, derivedAt: new Date().toISOString() });

  log(bold("\n2. Funding -- the operator's to do, not this script's"));
  log(`   Send testnet ETH on chain ${CHAIN_ID} to   ${address}`);
  log(`   Faucet                                     ${FAUCET}`);
  log(`   Explorer                                   ${EXPLORER}/address/${address}`);
  log(`   Then run                                   node evm-rehearsal.mjs watch`);
  log(`\n   No faucet is automated here. This one sits behind a browser challenge, and a faucet a script drains is a`);
  log(`   faucet that stops existing. A self-send costs about ${31_000n * 10_000_000n} wei of gas, so a drop is plenty.`);
}

async function watch() {
  const st = load();
  const address = assertAddress(st.address ?? "");
  await assertNodeAgreesOnChainId();
  log(bold(`\nWatching ${address} on chain ${CHAIN_ID} for funding`));
  for (let i = 0; ; i++) {
    const balance = BigInt(await rpc("eth_getBalance", [address, "latest"]));
    if (balance > 0n) {
      log(`\n   ${bold("funded")}: ${balance} wei`);
      save({ funded: { wei: balance.toString(), at: new Date().toISOString() } });
      log(`   Next: node evm-rehearsal.mjs send`);
      return;
    }
    process.stdout.write(`\r   nothing yet (${i + 1} checks, head ${Number(BigInt(await rpc("eth_blockNumber")))})   `);
    await new Promise((r) => setTimeout(r, 15_000));
  }
}

/// Build a zero-value self-send, sign it through the threshold key, and check the recovery parameter every way this
/// machine can before a node is asked. Shared by `preflight` and `send` so that the bytes the node judges are built by one
/// piece of code, not two that might drift.
async function buildAndSign() {
  const st = load();
  const address = assertAddress(st.address ?? "");
  const chain = await assertNodeAgreesOnChainId();
  if (canisterEvmAddress() !== address) throw new Error("refused: the canister no longer derives the recorded address");

  const balance = BigInt(await rpc("eth_getBalance", [address, "latest"]));
  const nonce = Number(BigInt(await rpc("eth_getTransactionCount", [address, "pending"])));
  const baseFee = BigInt((await rpc("eth_getBlockByNumber", ["latest", false])).baseFeePerGas);

  const tx = {
    chainId: chain,
    nonce,
    maxPriorityFeePerGas: 0n,
    // Room for the base fee to move between building and landing, which on an Orbit chain it can.
    maxFeePerGas: baseFee * 4n,
    gasLimit: 0n,
    to: address, // a self-send: the point is the signature, not the payload
    value: 0n,
    data: "0x",
  };
  const gas = BigInt(await rpc("eth_estimateGas", [{ from: address, to: address, value: "0x0" }]));
  tx.gasLimit = (gas * 3n) / 2n;

  const cost = tx.gasLimit * tx.maxFeePerGas;

  log(bold("\n3. Build -- an EIP-1559 zero-value self-send"));
  log(`   chainId  ${tx.chainId}   nonce ${tx.nonce}`);
  log(`   to       ${tx.to}  (ourselves)`);
  log(`   gas      ${tx.gasLimit} at up to ${tx.maxFeePerGas} wei  (base fee ${baseFee})`);
  log(`   balance  ${balance} wei, worst-case cost ${cost} wei`);

  const hash = signingHash(tx);
  log(bold("\n4. Sign -- with the threshold key, and DERIVE the recovery parameter"));
  log(`   signing hash  ${bytesToHex(hash)}`);
  const sig = canisterSign(hash);
  log(`   r             ${sig.r}`);
  log(`   s             ${sig.s}`);
  log(`   s from subnet ${sig.s_from_subnet}`);
  log(`   s was high?   ${bold(String(sig.s_was_high))}  ${sig.s_was_high ? "(normalised for EIP-2)" : "(already low)"}`);
  log(`   recovery id   ${bold(String(sig.recovery_id))}  (the canister recovered ${sig.signer})`);
  if (sig.signer !== address) throw new Error(`refused: the canister recovered ${sig.signer}, not ${address}`);

  log(bold("\n5. Recover independently, before anything is broadcast"));
  const both = recoverBothParities(hash, sig);
  for (const b of both) {
    const mark = b.address === address ? bold(" <- ours") : "";
    log(`   parity ${b.recovery_id}: ${b.address ?? "(no key)"}${mark}`);
  }
  const matches = both.filter((b) => b.address === address);
  if (matches.length !== 1) throw new Error(`refused: ${matches.length} parities recover ${address}; exactly one must`);
  if (matches[0].recovery_id !== sig.recovery_id) {
    throw new Error(`refused: @noble says parity ${matches[0].recovery_id}, k256 said ${sig.recovery_id}`);
  }
  log(`   @noble/curves and the canister's k256 agree on parity ${sig.recovery_id}, independently.`);

  const raw = eip1559Signed(tx, sig);
  const rawHex = "0x" + bytesToHex(raw);
  const id = txHash(raw);
  // The last check we can make without a node: the assembled envelope must still recover to us.
  if (recoverAddress(hash, sig) !== address) throw new Error("refused: the assembled transaction does not recover to us");
  log(`   raw transaction ${raw.length} bytes, hash ${bold(id)}`);

  return { address, chain, tx, balance, baseFee, cost, hash, sig, both, raw, rawHex, id };
}

/// Submit the signed transaction while the address is still empty, and read the sender out of the node's refusal.
///
/// This is free evidence and it arrives before any funding does. A node recovers the sender from the signature BEFORE it
/// looks up a balance, so an "insufficient funds" rejection has to name an address -- and the address it names is the one
/// it recovered, computed by a party that has never heard of this repository. If `v` were wrong the node would recover a
/// different address and name that one instead. The two outcomes are distinguishable, which is the whole point; an
/// experiment whose outcomes are not distinguishable is not an experiment.
async function preflight() {
  const b = await buildAndSign();
  if (b.balance > 0n) {
    log(`\n   note: the address already holds ${b.balance} wei, so this may be accepted rather than refused.`);
  }
  log(bold(`\n6. Preflight -- submit it unfunded and read the sender out of the refusal`));
  let refusal = null;
  try {
    const accepted = await rpc("eth_sendRawTransaction", [b.rawHex]);
    log(`   the node ACCEPTED it: ${accepted}`);
    save({ preflight: { accepted, at: new Date().toISOString() } });
    return;
  } catch (e) {
    refusal = e.message;
  }
  log(`   the node refused it, and said:`);
  log(`   ${refusal}`);
  const named = [...refusal.matchAll(/0x[0-9a-fA-F]{40}/g)].map((m) => m[0].toLowerCase());
  const namesUs = named.includes(b.address);
  log(`\n   addresses named in the refusal: ${named.length ? named.join(", ") : "(none)"}`);
  log(`   the canister derived            ${b.address}`);
  if (namesUs) {
    log(`   ${bold("the node recovered our address. The recovery parameter is right.")}`);
  } else if (named.length) {
    throw new Error(`the node recovered ${named.join(", ")}, not ${b.address}: the recovery parameter is WRONG`);
  } else {
    log(`   the refusal names no address, so it settles nothing. Fund the address and run \`send\`.`);
  }
  save({
    preflight: {
      refusal,
      addressesNamed: named,
      nodeRecoveredUs: namesUs,
      signingHash: bytesToHex(b.hash),
      signature: b.sig,
      raw: b.rawHex,
      txHash: b.id,
      at: new Date().toISOString(),
    },
  });
}

async function send() {
  const b = await buildAndSign();
  const { address, tx, balance, cost, hash, sig, both, rawHex, id } = b;
  if (balance === 0n) throw new Error("refused: the address has no balance; run `watch` first");
  if (cost > balance) throw new Error(`refused: ${cost} wei of gas allowance exceeds the ${balance} wei balance`);

  log(bold(`\n6. Broadcast to chain ${CHAIN_ID}`));
  const accepted = await rpc("eth_sendRawTransaction", [rawHex]);
  log(`   node accepted it and returned ${bold(accepted)}`);
  if (accepted.toLowerCase() !== id.toLowerCase()) throw new Error(`refused: node returned ${accepted}, we built ${id}`);
  const baseFee = b.baseFee;
  save({
    signature: sig,
    sent: {
      txHash: accepted,
      raw: rawHex,
      signingHash: bytesToHex(hash),
      nonce: tx.nonce,
      gasLimit: tx.gasLimit.toString(),
      maxFeePerGas: tx.maxFeePerGas.toString(),
      baseFeeWhenBuilt: baseFee.toString(),
      recoveredLocally: both,
      broadcastAt: new Date().toISOString(),
    },
  });
  log(`   Next: node evm-rehearsal.mjs confirm`);
}

async function confirm() {
  const st = load();
  if (!st.sent) throw new Error("nothing broadcast; run `send` first");
  const address = assertAddress(st.address);
  const id = st.sent.txHash;
  await assertNodeAgreesOnChainId();
  log(bold(`\n7. Waiting for ${id} to be mined`));
  for (let i = 0; ; i++) {
    const receipt = await rpc("eth_getTransactionReceipt", [id]);
    if (receipt) {
      // Re-read the transaction itself: `from` is the NODE's recovery, and it is the only number here we did not compute.
      const got = await rpc("eth_getTransactionByHash", [id]);
      const blockNumber = Number(BigInt(receipt.blockNumber));
      const block = await rpc("eth_getBlockByNumber", [receipt.blockNumber, false]);
      const inBlock = block.transactions.map((t) => t.toLowerCase()).includes(id.toLowerCase());

      log(`\n   ${bold("confirmed")}`);
      log(`   tx hash       ${id}`);
      log(`   status        ${receipt.status} ${receipt.status === "0x1" ? "(success)" : "(REVERTED)"}`);
      log(`   block         ${blockNumber}`);
      log(`   block hash    ${receipt.blockHash}`);
      log(`   gas used      ${Number(BigInt(receipt.gasUsed))}`);
      log(`   listed in that block's own transaction list: ${bold(String(inBlock))}`);
      log(`   yParity in the mined transaction: ${got.yParity ?? got.v}`);
      log(`\n   ${bold("the node says from = " + got.from.toLowerCase())}`);
      log(`   the canister derived           ${address}`);
      const agrees = got.from.toLowerCase() === address;
      log(`   ${bold(agrees ? "they agree. The recovery parameter is right." : "THEY DISAGREE.")}`);
      if (!inBlock) throw new Error("the receipt said confirmed but the block does not list it");
      if (!agrees) throw new Error(`the node recovered ${got.from}, not ${address}: the recovery parameter is wrong`);

      save({
        confirmed: {
          blockNumber,
          blockHash: receipt.blockHash,
          status: receipt.status,
          gasUsed: Number(BigInt(receipt.gasUsed)),
          nodeRecoveredFrom: got.from.toLowerCase(),
          yParity: got.yParity ?? got.v,
          inBlockTransactionList: inBlock,
        },
      });
      log(`\n   ${EXPLORER}/tx/${id}`);
      return;
    }
    process.stdout.write(`\r   not yet mined (${i + 1} checks, head ${Number(BigInt(await rpc("eth_blockNumber")))})   `);
    await new Promise((r) => setTimeout(r, 10_000));
  }
}


/// B4 (pre-registered in the rehearsal doc): the same transaction as `send`, but the CANISTER broadcasts it -- one node sends,
/// every node reads back -- and then the canister is asked, replicated, until it reports it mined. The script checks the hash
/// and the sender independently: the canister's word is not the evidence.
const dfxJson = (...args) => {
  const out = dfx("canister", "call", "--output", "json", ...args);
  return JSON.parse(out.slice(out.lastIndexOf("\n{", out.length - 3) + 1));   // the reply is the last JSON object printed
};
/// dfx prints a candid tuple record `record { text; SendOutcome }` as {"0": …, "1": …}
const pairs = (xs) => xs.map((p) => (Array.isArray(p) ? p : [p["0"], p["1"]]));
function broadcastViaCanister(rawHex, id) {
  const r = dfxJson("autobuyer", "rehearse_broadcast_evm", `(${CHAIN_ID} : nat64, "${rawHex}")`);
  if (r.Err !== undefined) throw new Error("canister refused: " + r.Err);
  const sent = r.Ok;
  const providers = pairs(sent.providers);
  for (const [url, outcome] of providers) log(`   ${url}\n      -> ${JSON.stringify(outcome)}`);
  log(`   read back at once: ${JSON.stringify(sent.seen)}`);
  log(`   canister's hash ${sent.tx_hash}`);
  log(`   script's hash   ${id}   ${sent.tx_hash.toLowerCase() === id.toLowerCase() ? bold("(same)") : bold("(DIFFERENT)")}`);
  if (sent.tx_hash.toLowerCase() !== id.toLowerCase()) throw new Error("FAIL: the canister watches a different hash");
  return { providers, seen: sent.seen };
}

async function canisterSend() {
  const b = await buildAndSign();
  const { balance, cost, rawHex, id } = b;
  if (balance === 0n) throw new Error("refused: the address has no balance");
  if (cost > balance) throw new Error(`refused: ${cost} wei of gas allowance exceeds the ${balance} wei balance`);
  save({ canisterSent: { txHash: id, raw: rawHex, at: new Date().toISOString() } });   // the bytes first, as the buyer must
  log(bold(`\n6. The canister broadcasts it (one node sends, every node reads back)`));
  const r = broadcastViaCanister(rawHex, id);
  save({ canisterSent: { ...load().canisterSent, providers: r.providers, seenAtOnce: r.seen } });
  await canisterConfirm();
}

/// Poll the canister's replicated read-back, then check the result from this script's own node.
async function canisterConfirm() {
  const st = load();
  if (!st.canisterSent) throw new Error("nothing sent by the canister; run `canister-send` first");
  const address = assertAddress(st.address), id = st.canisterSent.txHash;
  log(bold(`\n7. Ask the canister, replicated, until it is mined`));
  let seen;
  for (let i = 0; i < 30; i++) {
    seen = dfxJson("autobuyer", "evm_tx_status", `("${id}")`);
    if (seen.Mined) break;
    process.stdout.write(`\r   ${JSON.stringify(seen)} (${i + 1} checks)          `);
    await new Promise((r) => setTimeout(r, 10_000));
  }
  log(`\n   canister says: ${JSON.stringify(seen)}`);
  if (!seen?.Mined) throw new Error("FAIL: the canister never reported it mined");
  log(bold(`\n8. And the node, asked by this script, not the canister`));
  const got = await rpc("eth_getTransactionByHash", [id]);
  const receipt = await rpc("eth_getTransactionReceipt", [id]);
  const from = got.from.toLowerCase(), block = Number(BigInt(receipt.blockNumber));
  log(`   node recovered sender ${from}   ${from === address ? bold("(the canister's address)") : bold("(NOT OURS)")}`);
  log(`   block ${block}, status ${receipt.status}`);
  const pass = from === address && receipt.status === "0x1" && seen.Mined.success === true && Number(seen.Mined.block) === block
    && seen.Mined.from === address;
  save({ canisterConfirmed: { canisterSaid: seen, nodeFrom: from, block, status: receipt.status, pass, at: new Date().toISOString() } });
  log(`\n   ${bold(pass ? "PASS: the canister sent it, read it back, and the chain agrees on hash, block and sender." : "FAIL")}`);
  log(`   ${EXPLORER}/tx/${id}`);
  if (!pass) process.exit(1);
}

/// Send the SAME bytes again, through the canister, after they are mined: the design says this is harmless (one nonce, so at
/// most one of them is ever mined). The bytes come back from the chain itself and must hash to what was mined.
async function canisterResend() {
  const st = load();
  const id = st.canisterSent?.txHash;
  if (!id || !st.canisterConfirmed?.pass) throw new Error("refused: resend only a transaction the canister has confirmed mined");
  // This node does not serve eth_getRawTransactionByHash, so the bytes are rebuilt from the mined transaction's own fields,
  // and must hash to what was mined: only the identical bytes can.
  const t = await rpc("eth_getTransactionByHash", [id]);
  const tx = { chainId: Number(BigInt(t.chainId)), nonce: Number(BigInt(t.nonce)), maxPriorityFeePerGas: BigInt(t.maxPriorityFeePerGas),
    maxFeePerGas: BigInt(t.maxFeePerGas), gasLimit: BigInt(t.gas), to: t.to, value: BigInt(t.value), data: t.input };
  const pad = (h) => BigInt(h).toString(16).padStart(64, "0");
  const raw = eip1559Signed(tx, { recovery_id: Number(BigInt(t.yParity ?? t.v)), r: pad(t.r), s: pad(t.s) });
  const rawHex = "0x" + bytesToHex(raw);
  if (txHash(raw).toLowerCase() !== id.toLowerCase()) throw new Error("refused: the rebuilt bytes do not hash to the mined transaction");
  log(`   rebuilt ${raw.length} bytes from the chain's record; they hash to ${bold(id)}`);
  const before = Number(BigInt(await rpc("eth_getTransactionCount", [st.address, "latest"])));
  log(bold(`\n9. The same bytes again, through the canister (nonce count now ${before})`));
  const r = broadcastViaCanister(rawHex, id);
  const after = Number(BigInt(await rpc("eth_getTransactionCount", [st.address, "latest"])));
  const pass = r.providers.every(([, o]) => !(o === "Accepted" || o?.Accepted !== undefined)) && after === before && r.seen?.Mined !== undefined;
  log(`   nonce count after ${after}: ${after === before ? bold("unchanged -- nothing new was mined") : bold("CHANGED")}`);
  save({ canisterResent: { providers: r.providers, seen: r.seen, nonceBefore: before, nonceAfter: after, pass, at: new Date().toISOString() } });
  log(`\n   ${bold(pass ? "PASS: resending the same bytes changed nothing, and the canister still reads it as mined." : "FAIL")}`);
  if (!pass) process.exit(1);
}

const status = async () => log(JSON.stringify(load(), null, 2));

const phases = { derive, watch, preflight, send, confirm, "canister-send": canisterSend, "canister-confirm": canisterConfirm, "canister-resend": canisterResend, status };
const which = process.argv[2];
if (!phases[which]) {
  console.error(`usage: node evm-rehearsal.mjs <${Object.keys(phases).join("|")}>`);
  process.exit(2);
}
await phases[which]();
