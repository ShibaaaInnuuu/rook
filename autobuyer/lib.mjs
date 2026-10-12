// etched v2 spike helpers: BIP341 arithmetic done with standard libraries, and a thin wrapper over `dfx canister call`.
import { schnorr, secp256k1 } from "@noble/curves/secp256k1";
import { sha256 } from "@noble/hashes/sha256";
import { bytesToHex, hexToBytes, concatBytes } from "@noble/hashes/utils";
import { execFileSync } from "node:child_process";

const tagged = (tag, ...msgs) => { const t = sha256(new TextEncoder().encode(tag)); return sha256(concatBytes(t, t, ...msgs)); };

/** BIP341: Q = lift_x(P) + int(H_tapTweak(P || merkleRoot))·G. Returns the x-only output key. */
export function taprootOutputKey(internalXOnly, merkleRoot = new Uint8Array(0)) {
  const t = BigInt("0x" + bytesToHex(tagged("TapTweak", internalXOnly, merkleRoot)));
  const P = secp256k1.ProjectivePoint.fromHex("02" + bytesToHex(internalXOnly));   // lift_x: even y
  const Q = P.add(secp256k1.ProjectivePoint.BASE.multiply(t));
  return hexToBytes(Q.toAffine().x.toString(16).padStart(64, "0"));
}
export const verifyBip340 = (sig, msg, xOnlyPub) => schnorr.verify(sig, msg, xOnlyPub);

const DFX_ENV = { ...process.env, PATH: `${process.env.HOME}/Library/Application Support/org.dfinity.dfx/bin:${process.env.PATH}` };
const blobArg = (u8) => `blob "${[...u8].map((b) => "\\" + b.toString(16).padStart(2, "0")).join("")}"`;
const parseBlob = (out) => {
  const m = out.match(/blob "((?:[^"\\]|\\.)*)"/s); if (!m) throw new Error("no blob in: " + out.slice(0, 120));
  const s = m[1], bytes = [];
  for (let i = 0; i < s.length; i++) {
    if (s[i] === "\\") { const h = s.slice(i + 1, i + 3); if (/^[0-9a-fA-F]{2}$/.test(h)) { bytes.push(parseInt(h, 16)); i += 2; } else { bytes.push(s.charCodeAt(i + 1)); i += 1; } }
    else bytes.push(s.charCodeAt(i));
  }
  return Uint8Array.from(bytes);
};
export function call(method, args = "()", cwd = new URL("./canister/", import.meta.url).pathname) {
  return execFileSync("dfx", ["canister", "call", "autobuyer_spike", method, args], { cwd, env: DFX_ENV, encoding: "utf8", timeout: 120_000 });
}
export const canisterKey = () => parseBlob(call("taprootInternalKey"));
export const canisterSign = (sighash, merkleRoot = new Uint8Array(0)) => parseBlob(call("signKeyPath", `(${blobArg(sighash)}, ${blobArg(merkleRoot)})`));
export { bytesToHex, hexToBytes, blobArg, parseBlob };
