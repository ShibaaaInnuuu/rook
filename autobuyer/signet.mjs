// The signet half of the autonomous buyer, proved against a real network. Pre-registered in
// `docs/v2/2026-09-22-signet-rehearsal.md`, which fixes what counts as a pass.
//
// SIGNET ONLY. Mainnet is refused here by construction, not by a flag left at its default:
//   - `SIGNET` is the only network object this module exports, and `assertSignetParams` rejects any object whose bech32 HRP
//     is not `tb` -- so `btc.NETWORK` cannot be passed in by accident;
//   - `assertSignetAddress` DECODES the address and checks the HRP, witness version and program length. It never trusts the
//     variable that was supposed to produce it;
//   - `assertSignetEndpoint` checks every URL against an allowlist of signet hosts+paths before a request is made, and
//     rejects anything carrying a mainnet marker.
// None of this can stop a determined operator -- the canister signs a hash and never sees a network. Said plainly, and said
// again in the doc, rather than implied to be more than it is.
import * as btc from "@scure/btc-signer";
import { bech32m } from "@scure/base";

/// Signet shares testnet's address format: HRP `tb`, and the same base58 version bytes. It is a different chain with a
/// different genesis and its own faucet, but an address cannot tell you which of the two it is for -- only that it is not
/// mainnet's. That is the strongest statement an address check can make, so it is the one made here.
export const SIGNET = Object.freeze({ bech32: "tb", pubKeyHash: 111, scriptHash: 196, wif: 239 });

/// The one network name this rehearsal accepts, matching `REHEARSAL_NETWORK` in `canister-rs/src/canister.rs`.
export const NETWORK_NAME = "signet";

export function assertSignetParams(net) {
  if (!net || typeof net !== "object") throw new Error("refused: no network parameters given");
  if (net.bech32 !== SIGNET.bech32) {
    throw new Error(
      `refused: network parameters have bech32 HRP ${JSON.stringify(net.bech32)}; signet is ${JSON.stringify(SIGNET.bech32)}.` +
        (net.bech32 === "bc" ? " That is MAINNET. This module does not build mainnet transactions." : "")
    );
  }
  for (const k of ["pubKeyHash", "scriptHash", "wif"]) {
    if (net[k] !== SIGNET[k]) throw new Error(`refused: network parameter ${k} is ${net[k]}, not signet's ${SIGNET[k]}`);
  }
  return net;
}

/// Decode the address and check what it actually is. A `bc1…` address fails here even if every variable upstream said signet.
export function assertSignetAddress(addr) {
  if (typeof addr !== "string" || addr.length === 0) throw new Error("refused: no address");
  if (/^(bc1|1|3)/.test(addr)) throw new Error(`refused: ${addr.slice(0, 8)}… is a MAINNET address. Never.`);
  let decoded;
  try {
    decoded = bech32m.decode(addr);
  } catch (e) {
    throw new Error(`refused: ${addr.slice(0, 12)}… is not bech32m (taproot addresses are): ${e.message}`);
  }
  if (decoded.prefix !== SIGNET.bech32) throw new Error(`refused: HRP is ${JSON.stringify(decoded.prefix)}, not "tb"`);
  const [version, ...program] = decoded.words;
  if (version !== 1) throw new Error(`refused: witness version ${version}, not 1 (taproot)`);
  const bytes = bech32m.fromWords(program);
  if (bytes.length !== 32) throw new Error(`refused: witness program is ${bytes.length} bytes, not 32`);
  if (!addr.startsWith("tb1p")) throw new Error(`refused: ${addr.slice(0, 6)}… does not start with tb1p`);
  return addr;
}

/// Every endpoint the rehearsal may talk to. Allowlist, not denylist: a host that is not on this list is refused even if it
/// looks harmless, because "looks harmless" is how a mainnet URL gets used.
export const SIGNET_ENDPOINTS = Object.freeze([
  "https://mempool.space/signet/api",
  "https://open-api-signet.unisat.io",
]);

const MAINNET_MARKERS = [
  /^https:\/\/mempool\.space\/api\b/,
  /^https:\/\/open-api\.unisat\.io/,
  /blockstream\.info\/api/,
  /\/mainnet\//,
];

export function assertSignetEndpoint(url) {
  if (typeof url !== "string") throw new Error("refused: no url");
  for (const m of MAINNET_MARKERS) {
    if (m.test(url)) throw new Error(`refused: ${url} is a MAINNET Bitcoin endpoint`);
  }
  if (!SIGNET_ENDPOINTS.some((base) => url === base || url.startsWith(base + "/"))) {
    throw new Error(`refused: ${url} is not on the signet allowlist ${JSON.stringify(SIGNET_ENDPOINTS)}`);
  }
  return url;
}

/// The buyer's signet address, from the x-only INTERNAL key the threshold key returns.
///
/// The naming matters and was got wrong once: `schnorr_public_key` returns the canister's UNTWEAKED key. `p2tr` applies
/// BIP341's tweak to it to get the output key Q that goes in the scriptPubKey, and the subnet applies the same tweak when it
/// signs (`aux = #bip341`). Treating the untweaked key as the output key produces an address whose scriptPubKey commits to P
/// while the signature is made under Q -- which no test that verifies its own signature will ever catch, and which a Bitcoin
/// node rejects instantly.
export function buyerAddress(internalXOnly, net = SIGNET) {
  assertSignetParams(net);
  if (internalXOnly?.length !== 32) throw new Error(`refused: internal key is ${internalXOnly?.length} bytes, not 32`);
  const pay = btc.p2tr(internalXOnly, undefined, net);
  assertSignetAddress(pay.address);
  return pay;
}
