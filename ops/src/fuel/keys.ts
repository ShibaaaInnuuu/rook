import { createPrivateKey, createPublicKey, sign as edSign, type KeyObject } from "node:crypto";
import { readFileSync } from "node:fs";
import type { Identity } from "@icp-sdk/core/agent";
import { base58 } from "@scure/base";
import { keccak256, type Hex, type TransactionSerializableEIP1559 } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { identityFromPem } from "../identity.js";

/**
 * docs/v7/cycles-from-fees.md §4.2 item 6. THE ONLY FILE that reads the fueler's secrets: FUELER_EVM_KEY (the `ops` EOA on the
 * Robinhood home, the same address on 4663 and Ethereum; on the Solana home only its Ethereum address is used, as Relay's recipient and
 * the ckETH helper's caller), FUELER_IC_IDENTITY_PEM (its own IC identity, never the runner's operator identity) and, on the Solana home
 * (docs/v9/fueler-solana.md), the Solana hot keypair = `Treasury.ops`: FUELER_SOL_KEYPAIR_FILE (a path to a solana-keygen JSON file,
 * 0600) or FUELER_SOL_KEYPAIR (the same JSON, as a sealed variable). Neither is
 * shared with the runner, the keeper or the guardian. Neither is exported, printed or written; an error about one never repeats it.
 * What leaves this file: the address, the identity object (to the agent), a `sign` that refuses any chain but the one asked, and a
 * `scrub` for the log.
 */
export class FuelKeyError extends Error { constructor(message: string) { super(message); this.name = "FuelKeyError"; } }

const EVM = "FUELER_EVM_KEY", IC = "FUELER_IC_IDENTITY_PEM", SOL_FILE = "FUELER_SOL_KEYPAIR_FILE", SOL_JSON = "FUELER_SOL_KEYPAIR";

export type EvmSigner = { address: Hex; sign(tx: TransactionSerializableEIP1559): Promise<{ raw: Hex; hash: Hex }>; scrub(line: string): string };

/** Null when unset (a dry run needs no key). A malformed value is refused without repeating it. */
export function fuelEvmSigner(env: Readonly<Record<string, string | undefined>> = process.env): EvmSigner | null {
  const raw = env[EVM];
  if (raw === undefined || raw === "") return null;
  const key = raw.trim();
  if (!/^(0x)?[0-9a-fA-F]{64}$/.test(key)) throw new FuelKeyError(`${EVM} is set but is not a 32-byte hex key (its value is not shown)`);
  const hex = (key.startsWith("0x") ? key : `0x${key}`).toLowerCase() as Hex;
  const account = privateKeyToAccount(hex);
  const bare = hex.slice(2);
  return {
    address: account.address.toLowerCase() as Hex,
    async sign(tx) {
      const raw = await account.signTransaction(tx);
      return { raw, hash: keccak256(raw) };
    },
    scrub(line) {
      let out = line;
      for (let i = out.toLowerCase().indexOf(bare); i >= 0; i = out.toLowerCase().indexOf(bare)) out = out.slice(0, i) + "[redacted]" + out.slice(i + bare.length);
      return out;
    },
  };
}

export function fuelIcIdentity(env: Readonly<Record<string, string | undefined>> = process.env): Identity | null {
  return identityFromPem(env[IC], IC);
}

/** The Solana hot key: its address, a signature over a message (ed25519), and the log scrub. The secret never leaves this closure. */
export type SolSigner = { address: string; signMessage(msg: Uint8Array): Uint8Array; scrub(line: string): string };

const PKCS8_ED25519 = Buffer.from("302e020100300506032b657004220420", "hex");

/** Null when neither variable is set (a dry run needs no key). A malformed or mismatched keypair is refused without repeating it. */
export function fuelSolSigner(env: Readonly<Record<string, string | undefined>> = process.env): SolSigner | null {
  const path = env[SOL_FILE], inline = env[SOL_JSON];
  if (path && inline) throw new FuelKeyError(`set ${SOL_FILE} or ${SOL_JSON}, not both`);
  if (!path && !inline) return null;
  let text: string;
  if (path) {
    try { text = readFileSync(path, "utf8"); } catch (e) { throw new FuelKeyError(`${SOL_FILE}: the file cannot be read (${(e as NodeJS.ErrnoException).code ?? "error"})`); }
  } else text = inline!;
  let bytes: Uint8Array;
  try {
    const arr = JSON.parse(text);
    if (!Array.isArray(arr) || arr.length !== 64 || !arr.every((x) => Number.isInteger(x) && x >= 0 && x <= 255)) throw new Error();
    bytes = Uint8Array.from(arr);
  } catch { throw new FuelKeyError(`${path ? SOL_FILE : SOL_JSON} is not a solana-keygen keypair (a JSON array of 64 bytes; its content is not shown)`); }
  let priv: KeyObject;
  try { priv = createPrivateKey({ key: Buffer.concat([PKCS8_ED25519, bytes.subarray(0, 32)]), format: "der", type: "pkcs8" }); }
  catch { throw new FuelKeyError("the Solana keypair's secret half is not an ed25519 seed"); }
  const pub = createPublicKey(priv).export({ format: "der", type: "spki" }).subarray(-32);
  if (!Buffer.from(pub).equals(Buffer.from(bytes.subarray(32)))) throw new FuelKeyError("the Solana keypair's public half does not match its secret (a corrupted file?)");
  const address = base58.encode(Uint8Array.from(pub));
  const secrets = [base58.encode(bytes), base58.encode(bytes.subarray(0, 32)), Buffer.from(bytes.subarray(0, 32)).toString("hex"), `[${Array.from(bytes).join(",")}]`,
    Array.from(bytes.subarray(0, 32)).join(",")];
  bytes.fill(0);
  return {
    address,
    signMessage(msg) { return Uint8Array.from(edSign(null, Buffer.from(msg), priv)); },
    scrub(line) {
      let out = line;
      for (const s of secrets) while (out.includes(s)) out = out.replace(s, "[redacted]");
      return out;
    },
  };
}
