// The guards and the arithmetic of the EVM rehearsal. Pre-registered in
// `docs/v2/2026-09-22-evm-and-runes-rehearsals.md`.
//
// These tests can prove the refusals and the encoding. They cannot prove the recovery parameter: a test that recovers with
// the same assumption it signed under agrees with itself, which is the whole lesson of the taproot bug. Only a node can
// settle that, and `evm-rehearsal.mjs` is what asks one.
import test from "node:test";
import assert from "node:assert/strict";
import { secp256k1 } from "@noble/curves/secp256k1";
import {
  CHAIN_ID,
  PRODUCTION_CHAIN_ID,
  RPC_URL,
  assertRehearsalChainId,
  assertRehearsalEndpoint,
  assertAddress,
  toMinimalBytes,
  rlp,
  eip1559Fields,
  signingHash,
  eip1559Signed,
  recoverAddress,
  recoverBothParities,
  addressOfPublicKey,
  bytesToHex,
  hexToBytes,
} from "../evm.mjs";

test("the rehearsal chain id is an allowlist of exactly one value", () => {
  assert.equal(assertRehearsalChainId(CHAIN_ID), CHAIN_ID);

  // Mainnet, named, so the refusal is legible rather than generic.
  assert.throws(() => assertRehearsalChainId(PRODUCTION_CHAIN_ID), /MAINNET/);
  assert.throws(() => assertRehearsalChainId(PRODUCTION_CHAIN_ID), /Never/);

  // Every near miss a denylist of "not 4663" would have waved straight through.
  for (const typo of [0, 1, 4630, 46_29, 46629, 46631, 466300, 4663000, -46630]) {
    assert.throws(() => assertRehearsalChainId(typo), /refused/, `chain ${typo} must refuse`);
  }
  // And values that merely compare equal, or are the right number in the wrong type.
  for (const wrongType of ["46630", 46630n, 46630.5, null, undefined, NaN, [46630], { chainId: 46630 }]) {
    assert.throws(() => assertRehearsalChainId(wrongType), /refused/, `${String(wrongType)} must refuse`);
  }
});

test("only the one testnet endpoint is reachable, and the production host is refused", () => {
  assert.equal(assertRehearsalEndpoint(RPC_URL), RPC_URL);
  assert.equal(assertRehearsalEndpoint(RPC_URL + "/"), RPC_URL + "/");
  for (const bad of [
    "https://rpc.chain.robinhood.com",
    "https://rpc.chain.robinhood.com/",
    "https://chain.robinhood.com/rpc",
    "https://mainnet.infura.io/v3/x",
    "https://eth.llamarpc.com",
    "http://rpc.testnet.chain.robinhood.com", // http, not the https we allowlisted
    "https://rpc.testnet.chain.robinhood.com.evil.example",
    "",
  ]) {
    assert.throws(() => assertRehearsalEndpoint(bad), /refused/, `${bad} must refuse`);
  }
  assert.throws(() => assertRehearsalEndpoint(undefined), /refused/);
});

test("RLP encodes zero as the empty string, which is the classic way to get a different hash", () => {
  // Vectors from the Ethereum yellow paper / the RLP spec, i.e. from outside this repository.
  assert.equal(bytesToHex(toMinimalBytes(0)), "", "zero is the EMPTY string, not 0x00");
  assert.equal(bytesToHex(toMinimalBytes(1)), "01");
  assert.equal(bytesToHex(toMinimalBytes(255)), "ff");
  assert.equal(bytesToHex(toMinimalBytes(256)), "0100");
  assert.equal(bytesToHex(toMinimalBytes(1024)), "0400");

  assert.equal(bytesToHex(rlp(new Uint8Array(0))), "80", "the empty string");
  assert.equal(bytesToHex(rlp([])), "c0", "the empty list");
  assert.equal(bytesToHex(rlp(Uint8Array.from([0x00]))), "00", "a single byte below 0x80 is itself");
  assert.equal(bytesToHex(rlp(Uint8Array.from([0x7f]))), "7f");
  assert.equal(bytesToHex(rlp(Uint8Array.from([0x80]))), "8180", "…but 0x80 needs a header");
  assert.equal(bytesToHex(rlp(new TextEncoder().encode("dog"))), "83646f67");
  assert.equal(
    bytesToHex(rlp([new TextEncoder().encode("cat"), new TextEncoder().encode("dog")])),
    "c88363617483646f67"
  );
  // The 56-byte boundary, where the long form starts.
  assert.equal(bytesToHex(rlp(new Uint8Array(55))).slice(0, 2), "b7", "55 bytes is still the short form");
  assert.equal(bytesToHex(rlp(new Uint8Array(56))).slice(0, 4), "b838", "56 bytes crosses into the long form");
  assert.equal(bytesToHex(rlp(new Uint8Array(1024))).slice(0, 6), "b90400");
});

test("a type-2 transaction is nine fields, the chain id is the first, and the guard runs inside the encoder", () => {
  const tx = {
    chainId: CHAIN_ID,
    nonce: 0,
    maxPriorityFeePerGas: 0,
    maxFeePerGas: 10_000_000,
    gasLimit: 31_000,
    to: "0x5b0b0649f0306d08b4c0ad37406a7082fc25ebc2",
    value: 0,
    data: "0x",
  };
  const fields = eip1559Fields(tx);
  assert.equal(fields.length, 9);
  assert.equal(bytesToHex(fields[0]), "b626", "46630");
  assert.equal(bytesToHex(fields[1]), "", "nonce 0 is the empty string");
  assert.equal(fields[5].length, 20, "the recipient is 20 raw bytes, not a 0x string");
  assert.deepEqual(fields[8], [], "an empty access list");

  // The refusal is not somewhere else that could be skipped: it is inside the encoder.
  assert.throws(() => eip1559Fields({ ...tx, chainId: PRODUCTION_CHAIN_ID }), /MAINNET/);
  assert.throws(() => eip1559Fields({ ...tx, chainId: 1 }), /refused/);

  const hash = signingHash(tx);
  assert.equal(hash.length, 32);
  assert.notEqual(bytesToHex(hash), bytesToHex(signingHash({ ...tx, nonce: 1 })), "the nonce is committed to");

  assert.throws(() => assertAddress("0x123"), /not an address/);
  assert.throws(() => assertAddress("5b0b0649f0306d08b4c0ad37406a7082fc25ebc2"), /not an address/);
});

test("the signed envelope trims r and s to minimal width, because a padded r is a different transaction", () => {
  const tx = {
    chainId: CHAIN_ID,
    nonce: 3,
    maxPriorityFeePerGas: 0,
    maxFeePerGas: 10_000_000,
    gasLimit: 31_000,
    to: "0x5b0b0649f0306d08b4c0ad37406a7082fc25ebc2",
    value: 0,
    data: "0x",
  };
  // An r whose top byte is zero: RLP must encode 31 bytes, not 32.
  const r = "00" + "ab".repeat(31);
  const s = "cd".repeat(32);
  const raw = eip1559Signed(tx, { recovery_id: 1, r, s });
  assert.equal(raw[0], 0x02, "the type byte is outside the RLP list");
  assert.ok(bytesToHex(raw).includes("9f" + "ab".repeat(31)), "r is encoded as 31 bytes (0x9f = 0x80 + 31)");
  assert.ok(bytesToHex(raw).includes("a0" + "cd".repeat(32)), "s is encoded as 32 bytes (0xa0 = 0x80 + 32)");

  assert.throws(() => eip1559Signed(tx, { recovery_id: 2, r, s }), /yParity/);
  assert.throws(() => eip1559Signed(tx, { recovery_id: undefined, r, s }), /yParity/);
});

test("exactly one parity recovers the signer, and the other recovers a valid wrong address", () => {
  // This is the danger stated as arithmetic. It is NOT evidence that the canister's `v` is right -- this test signs with a
  // key it holds, so it shares its own assumption. The node is what settles that, in `evm-rehearsal.mjs`.
  const priv = hexToBytes("46".repeat(32)); // the EIP-155 example key
  const expected = addressOfPublicKey(secp256k1.getPublicKey(priv, false));
  assert.equal(expected, "0x9d8a62f656a8d1615c1294fd71e9cfb3e4855a4f", "the address EIP-155 itself states");

  let sawZero = 0;
  let sawOne = 0;
  for (let i = 0; i < 24; i++) {
    const hash = hexToBytes(String(i).padStart(2, "0").repeat(32).slice(0, 64));
    const sig = secp256k1.sign(hash, priv);
    const r = sig.r.toString(16).padStart(64, "0");
    const s = sig.s.toString(16).padStart(64, "0");
    const both = recoverBothParities(hash, { r, s });
    const matching = both.filter((b) => b.address === expected);
    assert.equal(matching.length, 1, "exactly one parity is the signer");
    assert.equal(both.filter((b) => b.address !== null).length, 2, "and the other one still yields an address");
    assert.equal(recoverAddress(hash, { r, s, recovery_id: matching[0].recovery_id }), expected);
    if (matching[0].recovery_id === 0) sawZero++;
    else sawOne++;
  }
  assert.ok(sawZero > 3 && sawOne > 3, `both parities occur (${sawZero} zeros, ${sawOne} ones): a constant v would be wrong often`);
});
