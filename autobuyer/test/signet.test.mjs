// The guards that keep the rehearsal on signet. Every one of these is a test that mainnet is REFUSED, not a test that
// signet is selected: the failure mode worth catching is a mainnet value arriving through a path nobody checked.
import assert from "node:assert/strict";
import { test } from "node:test";
import * as btc from "@scure/btc-signer";
import { schnorr } from "@noble/curves/secp256k1";
import {
  SIGNET,
  NETWORK_NAME,
  assertSignetParams,
  assertSignetAddress,
  assertSignetEndpoint,
  buyerAddress,
  SIGNET_ENDPOINTS,
} from "../signet.mjs";
import { taprootOutputKey, hexToBytes, bytesToHex } from "../lib.mjs";

test("mainnet network parameters are refused, and the message says the word", () => {
  assert.throws(() => assertSignetParams(btc.NETWORK), /MAINNET/);
  assert.throws(() => assertSignetParams({ ...SIGNET, bech32: "bc" }), /MAINNET/);
  assert.throws(() => assertSignetParams({ ...SIGNET, wif: 128 }), /wif is 128/);
  assert.throws(() => assertSignetParams(undefined), /no network parameters/);
  assert.equal(assertSignetParams(btc.TEST_NETWORK), btc.TEST_NETWORK, "signet shares testnet's parameters");
});

test("an address is checked by decoding it, not by trusting what produced it", () => {
  // BIP341's own test vector, which is a MAINNET address. It must fail here however respectable its provenance.
  assert.throws(
    () => assertSignetAddress("bc1p2wsldez5mud2yam29q22wgfh9439spgduvct83k3pm50fcxa5dps59h4z5"),
    /MAINNET/
  );
  assert.throws(() => assertSignetAddress("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2"), /MAINNET/);
  assert.throws(() => assertSignetAddress("3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"), /MAINNET/);
  // a signet/testnet SegWit v0 address: right HRP, wrong witness version, and bech32 rather than bech32m
  assert.throws(() => assertSignetAddress("tb1qw508d6qejxtdg4y5r3zarvaryvaxxpcs"), /bech32m|witness version/);
  assert.throws(() => assertSignetAddress(""), /no address/);
  assert.throws(() => assertSignetAddress("tb1pnotarealaddress"), /bech32m/);
});

test("a real signet taproot address passes, and is the one p2tr builds", () => {
  const internal = schnorr.getPublicKey(hexToBytes("07".repeat(32)));
  const pay = buyerAddress(internal);
  assert.ok(pay.address.startsWith("tb1p"), pay.address);
  assert.equal(assertSignetAddress(pay.address), pay.address);
  // and the same key on mainnet parameters is a bc1p address, which buyerAddress will not produce
  assert.ok(btc.p2tr(internal, undefined, btc.NETWORK).address.startsWith("bc1p"));
  assert.throws(() => buyerAddress(internal, btc.NETWORK), /MAINNET/);
  assert.throws(() => buyerAddress(internal.slice(1)), /31 bytes, not 32/);
});

test("the address commits to the TWEAKED output key, not to the internal key the canister returns", () => {
  // This is the mistake the rehearsal exists to catch: `schnorr_public_key` returns the UNTWEAKED key, and a scriptPubKey
  // built from it would never be spendable by a signature the subnet makes (the subnet tweaks). Bitcoin rejects it; a test
  // that verifies its own signature does not.
  const internal = schnorr.getPublicKey(hexToBytes("09".repeat(32)));
  const pay = buyerAddress(internal);
  assert.equal(bytesToHex(pay.tweakedPubkey), bytesToHex(taprootOutputKey(internal)));
  assert.notEqual(bytesToHex(pay.tweakedPubkey), bytesToHex(internal), "the tweak is not the identity");
  assert.equal(bytesToHex(pay.script.slice(2)), bytesToHex(taprootOutputKey(internal)), "OP_1 <32-byte Q>");
  assert.equal(pay.script[0], 0x51);
  assert.equal(pay.script[1], 0x20);
});

test("only signet endpoints may be called, by allowlist", () => {
  assert.throws(() => assertSignetEndpoint("https://mempool.space/api/blocks/tip/height"), /MAINNET/);
  assert.throws(() => assertSignetEndpoint("https://open-api.unisat.io/v1/indexer/blockchain/info"), /MAINNET/);
  assert.throws(() => assertSignetEndpoint("https://blockstream.info/api/blocks"), /MAINNET/);
  assert.throws(() => assertSignetEndpoint("https://example.com/signet/api"), /not on the signet allowlist/);
  // a prefix that merely starts the same is not the same host
  assert.throws(() => assertSignetEndpoint("https://mempool.space/signetEVIL/api"), /not on the signet allowlist/);
  for (const base of SIGNET_ENDPOINTS) assert.equal(assertSignetEndpoint(base), base);
  assert.equal(
    assertSignetEndpoint("https://mempool.space/signet/api/blocks/tip/height"),
    "https://mempool.space/signet/api/blocks/tip/height"
  );
});

test("the network name the canister is asked for is an allowlist of exactly one", () => {
  assert.equal(NETWORK_NAME, "signet");
  assert.notEqual(NETWORK_NAME, "mainnet");
});
