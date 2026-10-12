import { privateKeyToAccount } from "viem/accounts";
import { keccak256, type Hex, type TransactionSerializableEIP1559 } from "viem";

/**
 * docs/v6/p1-services.md §1.1. THE ONLY FILE in keeper/service/ that reads a private key, and the only one that holds it. The
 * keeper's key comes from KEEPER_PRIVATE_KEY; the standalone resolver's (any funded account, no role: fixes-W2-services.md §2) from
 * RESOLVER_PRIVATE_KEY. Both are set by the operator in the host's secret store (Railway), never by anything in this repository. It stays in this closure: the module exports an address, a signing function and a scrubber, never the key. This
 * file prints nothing (test/service-guards.test.ts), and an error about the key never contains it.
 */
export type Signer = {
  readonly address: Hex;
  /** Signs an EIP-1559 transaction whose chain id must be `chainId`; returns the raw bytes and their hash. */
  sign(tx: TransactionSerializableEIP1559, chainId: number): Promise<{ raw: Hex; hash: Hex }>;
  /** `line` with the key cut out, wherever and however it appears (with or without 0x, any case). */
  scrub(line: string): string;
};

export class SignerError extends Error { constructor(message: string) { super(message); this.name = "SignerError"; } }

const VARS = { keeper: "KEEPER_PRIVATE_KEY", resolver: "RESOLVER_PRIVATE_KEY" } as const;

/** Null when the variable is unset (a dry run needs no key). A malformed value is refused without repeating it. */
export function signerFromEnv(env: Readonly<Record<string, string | undefined>> = process.env, role: keyof typeof VARS = "keeper"): Signer | null {
  const VAR = VARS[role];
  const raw = env[VAR];
  if (raw === undefined || raw === "") return null;
  const key = raw.trim();
  if (!/^(0x)?[0-9a-fA-F]{64}$/.test(key)) throw new SignerError(`${VAR} is set but is not a 32-byte hex key (its value is not shown)`);
  const hex = (key.startsWith("0x") ? key : `0x${key}`).toLowerCase() as Hex;
  const account = privateKeyToAccount(hex);
  const bare = hex.slice(2);
  return {
    address: account.address,
    async sign(tx, chainId) {
      if (tx.chainId !== chainId) throw new SignerError(`refused: a transaction for chain ${tx.chainId} while the keeper signs for ${chainId}`);
      const raw = await account.signTransaction(tx);
      return { raw, hash: keccak256(raw) };
    },
    scrub(line) {
      // case-insensitive, with or without 0x: the bare 64 hex characters are what matter
      let out = line;
      const lower = out.toLowerCase();
      if (!lower.includes(bare)) return out;
      let i = lower.indexOf(bare);
      while (i >= 0) { out = out.slice(0, i) + "[redacted]" + out.slice(i + bare.length); i = out.toLowerCase().indexOf(bare); }
      return out;
    },
  };
}
