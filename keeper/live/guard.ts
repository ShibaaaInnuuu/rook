import { privateKeyToAccount } from "viem/accounts";
import { createWalletClient, http, type Hex } from "viem";

/**
 * docs/phase6.md, tier 2: a SMALL REAL TEST on Robinhood Chain, run by the user. THE ONLY PLACE in this package that signs.
 *
 * It arms on FIVE facts together and says which is missing; any one absent and nothing is signed:
 *   1. the mode            <ROLE>_MODE=mainnet-test          (there is no other mode that signs; there is no "mainnet" mode at all)
 *   2. the latch           <ROLE>_LIVE=1
 *   3. the confirm string  <ROLE>_CONFIRM=<role>-live:4663:<the signer's address, lowercase>    typed by a person who knows both
 *   4. the deployment      a file whose label is exactly "test" and whose chainId is 4663, naming this signer in this role
 *   5. the node            reports chain id 4663, asked before every send
 * The key comes from <ROLE>_KEY, set by the user in their own environment. It is never logged, never written, never generated
 * here, and the address is the only thing printed. Nothing in this repository has ever held a real key.
 */
export const ROBINHOOD_CHAIN_ID = 4663;
export type Role = "keeper" | "attestor";
export type Env = Readonly<Record<string, string | undefined>>;
export type Fetch = typeof fetch;
export type Deployment = { chainId?: number; label?: string; keeper?: string; attestor?: string; registry?: string };
export type Armed = { address: string; send(tx: { to: string; data: Hex; value?: bigint }): Promise<Hex> };

export class NotArmed extends Error { constructor(readonly missing: string[]) { super(`not armed, nothing will be signed:\n  - ${missing.join("\n  - ")}`); this.name = "NotArmed"; } }

export const confirmFor = (role: Role, address: string): string => `${role}-live:${ROBINHOOD_CHAIN_ID}:${address.toLowerCase()}`;

export async function arm(role: Role, url: string, deployment: Deployment, env: Env = process.env, fetchImpl: Fetch = fetch): Promise<Armed> {
  const R = role.toUpperCase(), missing: string[] = [];
  if (env[`${R}_MODE`] !== "mainnet-test") missing.push(`${R}_MODE is not "mainnet-test" (the only mode that signs)`);
  if (env[`${R}_LIVE`] !== "1") missing.push(`${R}_LIVE is not 1`);
  if (deployment.label !== "test") missing.push(`the deployment's label is ${JSON.stringify(deployment.label ?? null)}, not "test": this package does not sign for any other deployment`);
  if (deployment.chainId !== ROBINHOOD_CHAIN_ID) missing.push(`the deployment's chainId is ${deployment.chainId}, not ${ROBINHOOD_CHAIN_ID}`);
  const key = env[`${R}_KEY`];
  let address: string | null = null;
  if (!key || !/^0x[0-9a-fA-F]{64}$/.test(key)) missing.push(`${R}_KEY is not set (or is not a 32-byte hex key); the user sets it, this code never makes one`);
  else {
    address = privateKeyToAccount(key as Hex).address;
    const named = deployment[role];
    if (!named || named.toLowerCase() !== address.toLowerCase()) missing.push(`the key's address ${address} is not the deployment's ${role} (${named ?? "none"})`);
    if (env[`${R}_CONFIRM`] !== confirmFor(role, address)) missing.push(`${R}_CONFIRM must be exactly "${confirmFor(role, address)}"`);
  }
  const chainId = async () => {
    const r = await fetchImpl(url, { method: "POST", headers: { "content-type": "application/json" }, body: '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' });
    return Number(BigInt(((await r.json()) as { result: string }).result));
  };
  if (!missing.length) { const id = await chainId(); if (id !== ROBINHOOD_CHAIN_ID) missing.push(`the node reports chain id ${id}, not ${ROBINHOOD_CHAIN_ID}`); }
  if (missing.length) throw new NotArmed(missing);
  const account = privateKeyToAccount(key as Hex);
  const chain = { id: ROBINHOOD_CHAIN_ID, name: "Robinhood Chain", nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 }, rpcUrls: { default: { http: [url] } } } as const;
  const wallet = createWalletClient({ account, chain, transport: http(url, { fetchFn: fetchImpl as typeof fetch }) });
  return {
    address: account.address,
    async send(tx) {
      const id = await chainId();
      if (id !== ROBINHOOD_CHAIN_ID) throw new Error(`refused: the node now reports chain id ${id}`);
      return wallet.sendTransaction({ to: tx.to as Hex, data: tx.data, value: tx.value ?? 0n });
    },
  };
}
