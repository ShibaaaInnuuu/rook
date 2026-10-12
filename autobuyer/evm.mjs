// The EVM half of the autonomous buyer, proved against a real network. Pre-registered in
// `docs/v2/2026-09-22-evm-and-runes-rehearsals.md`, which fixes what counts as a pass.
//
// CHAIN 46630 ONLY -- Robinhood Chain **testnet**. Chain 4663 is Robinhood Chain **mainnet**, where `AssetVault` and the
// money are, and it is refused here by construction rather than by a flag left at its default:
//   - `CHAIN_ID` is the only chain this module will build a transaction for, and `assertRehearsalChainId` is an allowlist
//     of exactly one value. Not "anything but 4663": a typo, a zero, a string and a chain invented next year all refuse;
//   - `assertRehearsalEndpoint` checks every URL against an allowlist of one host before a request is made;
//   - `assertNodeAgreesOnChainId` ASKS THE NODE and refuses unless it answers 0xb626. It never trusts the variable that was
//     supposed to hold the chain id, in the same way `assertSignetAddress` decodes an address instead of trusting a string.
// None of this can stop a determined operator -- the canister signs a hash and never sees a network. Said plainly here and
// again in the doc, rather than implied to be more than it is.
import { secp256k1 } from "@noble/curves/secp256k1";
import { keccak_256 } from "@noble/hashes/sha3";
import { bytesToHex, hexToBytes, concatBytes } from "@noble/hashes/utils";

/// The one chain this module builds for. 0xb626.
export const CHAIN_ID = 46630;

/// Robinhood Chain mainnet. Named only so a refusal can say what it refused.
export const PRODUCTION_CHAIN_ID = 4663;

/// The one RPC endpoint the rehearsal may talk to.
export const RPC_URL = "https://rpc.testnet.chain.robinhood.com";

export const EXPLORER = "https://explorer.testnet.chain.robinhood.com";

/// The faucet the operator uses. Never called from here: a faucet that a script drains is a faucet that stops existing,
/// and this one sits behind a browser challenge besides.
export const FAUCET = "https://faucet.testnet.chain.robinhood.com/";

const MAINNET_MARKERS = [
  /^https:\/\/rpc\.chain\.robinhood\.com/,
  /^https:\/\/([a-z0-9-]+\.)?chain\.robinhood\.com/, // the bare production host and anything under it
  /mainnet/i,
  /\bmain\.chain\b/,
];

/// Describe a value in an error message without the description itself being able to throw. `JSON.stringify` throws on a
/// BigInt, which would turn a clean refusal into a TypeError and lose the reason -- found by a test that passed `46630n`.
const show = (v) => {
  try {
    return typeof v === "bigint" ? `${v}n` : JSON.stringify(v) ?? String(v);
  } catch {
    return Object.prototype.toString.call(v);
  }
};

/// An allowlist of exactly one chain id. The argument must be a `number` that is exactly `CHAIN_ID`: the string "46630",
/// the bigint `46630n`, and 46630.5 all refuse, because a value that merely compares equal is not the same as the right
/// value -- `"46630" == 46630` is true in this language, and that is the kind of thing a guard exists to not rely on.
export function assertRehearsalChainId(id) {
  if (typeof id !== "number" || !Number.isInteger(id)) {
    throw new Error(`refused: chain id ${show(id)} is not an integer. The only accepted value is ${CHAIN_ID}.`);
  }
  if (id === PRODUCTION_CHAIN_ID) {
    throw new Error(
      `refused: ${PRODUCTION_CHAIN_ID} is Robinhood Chain MAINNET, where the vault and the money are. This module builds ` +
        `for ${CHAIN_ID} (testnet) and nothing else. Never.`
    );
  }
  if (id !== CHAIN_ID) {
    throw new Error(
      `refused: chain id ${id}. The only accepted value is ${CHAIN_ID}, and it is an allowlist of one on purpose -- ` +
        `"anything but ${PRODUCTION_CHAIN_ID}" would accept a typo.`
    );
  }
  return id;
}

/// Every endpoint the rehearsal may talk to. Allowlist, not denylist: a host that is not on this list is refused even if it
/// looks harmless, because "looks harmless" is how a mainnet URL gets used.
export const REHEARSAL_ENDPOINTS = Object.freeze([RPC_URL, EXPLORER]);

export function assertRehearsalEndpoint(url) {
  if (typeof url !== "string") throw new Error("refused: no url");
  for (const m of MAINNET_MARKERS) {
    if (m.test(url) && !REHEARSAL_ENDPOINTS.some((b) => url === b || url.startsWith(b + "/"))) {
      throw new Error(`refused: ${url} carries a MAINNET marker`);
    }
  }
  if (!REHEARSAL_ENDPOINTS.some((base) => url === base || url.startsWith(base + "/"))) {
    throw new Error(`refused: ${url} is not on the rehearsal allowlist ${JSON.stringify(REHEARSAL_ENDPOINTS)}`);
  }
  return url;
}

// ---- RLP, because an EVM transaction is an RLP list and nothing here has ever serialised one ---------------------------------

/// Minimal-width big-endian bytes for a non-negative integer. RLP has no leading zeros, and `0` is the EMPTY string --
/// not a zero byte. Getting that wrong changes the transaction hash, so it is the first thing the tests pin down.
export function toMinimalBytes(value) {
  let v = BigInt(value);
  if (v < 0n) throw new Error("refused: RLP encodes no negative integers");
  if (v === 0n) return new Uint8Array(0);
  let hex = v.toString(16);
  if (hex.length % 2) hex = "0" + hex;
  return hexToBytes(hex);
}

function encodeLength(len, offset) {
  if (len < 56) return Uint8Array.from([offset + len]);
  const lenBytes = toMinimalBytes(len);
  return concatBytes(Uint8Array.from([offset + 55 + lenBytes.length]), lenBytes);
}

/// RLP-encode a byte string or a (possibly nested) list of them. Integers must be converted by the caller with
/// `toMinimalBytes`, so that the "how wide is this number" decision is always visible.
export function rlp(item) {
  if (Array.isArray(item)) {
    const body = concatBytes(...item.map(rlp), new Uint8Array(0));
    return concatBytes(encodeLength(body.length, 0xc0), body);
  }
  if (!(item instanceof Uint8Array)) throw new Error(`refused: rlp() takes bytes or arrays, got ${typeof item}`);
  if (item.length === 1 && item[0] < 0x80) return item;
  return concatBytes(encodeLength(item.length, 0x80), item);
}

// ---- the transaction ---------------------------------------------------------------------------------------------------------

/// An EIP-1559 (type 0x02) transaction, as the nine fields that are signed. `chainId` is checked against the allowlist here
/// rather than anywhere further downstream, because this is the last place it is still a number rather than bytes.
export function eip1559Fields(tx) {
  assertRehearsalChainId(tx.chainId);
  const addr = assertAddress(tx.to);
  return [
    toMinimalBytes(tx.chainId),
    toMinimalBytes(tx.nonce),
    toMinimalBytes(tx.maxPriorityFeePerGas),
    toMinimalBytes(tx.maxFeePerGas),
    toMinimalBytes(tx.gasLimit),
    hexToBytes(addr.slice(2)),
    toMinimalBytes(tx.value),
    tx.data && tx.data !== "0x" ? hexToBytes(tx.data.slice(2)) : new Uint8Array(0),
    [], // accessList
  ];
}

/// The bytes that get hashed to produce what the canister signs: `0x02 || rlp([...nine fields])`.
export const eip1559SigningPayload = (tx) => concatBytes(Uint8Array.from([0x02]), rlp(eip1559Fields(tx)));

export const signingHash = (tx) => keccak_256(eip1559SigningPayload(tx));

/// The signed, broadcastable transaction: the same nine fields plus `yParity`, `r` and `s`.
export function eip1559Signed(tx, { recovery_id, r, s }) {
  if (recovery_id !== 0 && recovery_id !== 1) throw new Error(`refused: yParity must be 0 or 1, got ${recovery_id}`);
  const fields = eip1559Fields(tx);
  // r and s are stripped of leading zero bytes: RLP integers are minimal-width, and a 32-byte-padded r is a DIFFERENT
  // encoding with a different hash that a node will reject as invalid RLP.
  const trim = (h) => toMinimalBytes(BigInt("0x" + h));
  return concatBytes(Uint8Array.from([0x02]), rlp([...fields, toMinimalBytes(recovery_id), trim(r), trim(s)]));
}

// ---- recovery, done independently of the canister --------------------------------------------------------------------------

export const assertAddress = (a) => {
  if (typeof a !== "string" || !/^0x[0-9a-fA-F]{40}$/.test(a)) throw new Error(`refused: ${JSON.stringify(a)} is not an address`);
  return a.toLowerCase();
};

export const addressOfPublicKey = (uncompressed65) => "0x" + bytesToHex(keccak_256(uncompressed65.slice(1)).slice(12));

/// Recover the sender from a hash and a signature, using `@noble/curves` -- a DIFFERENT implementation from the `k256` the
/// canister recovers with. Agreement between the two is worth something; agreement of a library with itself is not.
export function recoverAddress(hash, { r, s, recovery_id }) {
  const sig = new secp256k1.Signature(BigInt("0x" + r), BigInt("0x" + s)).addRecoveryBit(recovery_id);
  return addressOfPublicKey(sig.recoverPublicKey(hash).toRawBytes(false));
}

/// Both parities, so a caller can show that exactly one of them is the signer rather than assert it.
export const recoverBothParities = (hash, { r, s }) =>
  [0, 1].map((recovery_id) => {
    try {
      return { recovery_id, address: recoverAddress(hash, { r, s, recovery_id }) };
    } catch (e) {
      return { recovery_id, address: null, error: e.message };
    }
  });

/// The transaction hash a node will know this transaction by: keccak of the whole signed envelope.
export const txHash = (signedRaw) => "0x" + bytesToHex(keccak_256(signedRaw));

// ---- talking to the node -----------------------------------------------------------------------------------------------------

export async function rpc(method, params = []) {
  const url = assertRehearsalEndpoint(RPC_URL);
  const res = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} -> HTTP ${res.status}: ${text.slice(0, 300)}`);
  const body = JSON.parse(text);
  if (body.error) throw new Error(`${method} -> ${JSON.stringify(body.error)}`);
  return body.result;
}

/// Ask the node which chain it is, and refuse unless it says 46630. The variable that was supposed to hold the chain id is
/// never the check -- this is the EVM twin of decoding an address instead of trusting the string that produced it.
export async function assertNodeAgreesOnChainId() {
  const hex = await rpc("eth_chainId");
  const id = Number(BigInt(hex));
  if (id === PRODUCTION_CHAIN_ID) {
    throw new Error(`refused: ${RPC_URL} answers chain ${id}, which is Robinhood Chain MAINNET. Stopping.`);
  }
  if (id !== CHAIN_ID) {
    throw new Error(`refused: ${RPC_URL} answers chain ${id}, not the ${CHAIN_ID} this rehearsal is for.`);
  }
  return id;
}

export { bytesToHex, hexToBytes, keccak_256 };
