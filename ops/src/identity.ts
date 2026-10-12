import { Ed25519KeyIdentity } from "@icp-sdk/core/identity";
import { Secp256k1KeyIdentity } from "@icp-sdk/core/identity/secp256k1";
import type { Identity } from "@icp-sdk/core/agent";

/**
 * docs/v6/p1-services.md §2.1. THE ONLY FILE in ops/ that reads the operator's hot IC identity: OPERATOR_IDENTITY_PEM, the PEM text
 * `dfx identity export <name>` prints (secp256k1 "EC PRIVATE KEY"; an Ed25519 PKCS#8 "PRIVATE KEY" also loads), set by the operator
 * in the runner's secret store. The identity object signs the runner's calls and nothing else; it is never exported, printed or
 * written, and an error about it never repeats it. What leaves this file is the identity (to the agent) and its principal (text).
 */
export class IdentityError extends Error { constructor(message: string) { super(message); this.name = "IdentityError"; } }

const VAR = "OPERATOR_IDENTITY_PEM";
const ED25519_PKCS8 = ["302e020100300506032b657004220420", "3053020101300506032b657004220420"];

function derOf(pem: string, label: string, name: string): Uint8Array {
  const m = new RegExp(`-----BEGIN ${label}-----([\\s\\S]+?)-----END ${label}-----`).exec(pem);
  if (!m) throw new IdentityError(`${name} is set but holds no ${label} block (its value is not shown)`);
  return Uint8Array.from(Buffer.from(m[1].replace(/\s+/g, ""), "base64"));
}

/** An identity from PEM text (secp256k1 "EC PRIVATE KEY" or Ed25519 PKCS#8), or null when empty. `name` is only used in errors, which
 *  never repeat the value. The fueler's own identity (src/fuel/keys.ts) goes through the same parser. */
export function identityFromPem(raw: string | undefined, name: string): Identity | null {
  if (raw === undefined || raw.trim() === "") return null;
  const pem = raw.replace(/\\n/g, "\n").trim();       // a secret store may keep the newlines escaped
  try {
    if (pem.includes("-----BEGIN EC PRIVATE KEY-----")) return Secp256k1KeyIdentity.fromPem(pem);
    if (pem.includes("-----BEGIN PRIVATE KEY-----")) {
      const der = derOf(pem, "PRIVATE KEY", name), head = Buffer.from(der.slice(0, 16)).toString("hex");
      if (!ED25519_PKCS8.includes(head)) throw new IdentityError(`${name} holds a PKCS#8 key that is not Ed25519 (its value is not shown)`);
      return Ed25519KeyIdentity.fromSecretKey(der.slice(16, 48));
    }
  } catch (err) {
    if (err instanceof IdentityError) throw err;
    throw new IdentityError(`${name} could not be read as a key (its value is not shown)`);
  }
  throw new IdentityError(`${name} is neither an EC (secp256k1) nor an Ed25519 PEM key (its value is not shown)`);
}

/** The identity from the environment, or null when the variable is unset (a dry run needs none). */
export function operatorIdentity(env: Readonly<Record<string, string | undefined>> = process.env): Identity | null {
  return identityFromPem(env[VAR], VAR);
}
