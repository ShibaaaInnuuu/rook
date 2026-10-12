import { createPublicClient, createWalletClient, defineChain, http, type Account, type Hex, type PublicClient, type WalletClient } from "viem";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import { STAGING_CHAIN_ID, STAGING_CURRENCY, STAGING_NAME, assertStagingChain } from "../chain-id.js";

/**
 * docs/phase8.md §1. The simulator's wallets: made here, funded by the node's own admin method, and thrown away when the
 * process ends.
 *
 * THE KEYS ARE NEVER LOGGED AND NEVER WRITTEN. They exist in this process and nowhere else; a restart makes new ones. They are
 * deliberately NOT the node's well-known development accounts — docs/phase2.md found sweeper delegations on those addresses on
 * this chain, and anything sent to a delegated address goes somewhere else.
 *
 * Funding is `anvil_setBalance`, not the faucet: the faucet's daily allowance belongs to whoever is looking at staging, and the
 * simulator must never spend it.
 */
export type Sim = {
  pub: PublicClient;
  wallets: { account: Account; client: WalletClient }[];
  /** the node, on this machine: admin methods the public proxy refuses */
  admin<T>(method: string, params?: unknown[]): Promise<T>;
  fund(address: string, wei: bigint): Promise<void>;
  mined(hash: Hex): Promise<{ status: "success" | "reverted" }>;
};

export async function openSim(node: string, count: number): Promise<Sim> {
  if (!["127.0.0.1", "localhost", "[::1]", "::1"].includes(new URL(node).hostname))
    throw new Error("refused: the simulator drives a node on this machine only, never a public address");
  const admin = async <T>(method: string, params: unknown[] = []): Promise<T> => {
    const r = await fetch(node, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
    const j = (await r.json()) as { result?: T; error?: { message: string } };
    if (j.error) throw new Error(`${method}: ${j.error.message}`);
    return j.result as T;
  };
  assertStagingChain(Number(BigInt(await admin<string>("eth_chainId"))));   // refuses 4663, refuses the public testnet, refuses anything but ours

  const chain = defineChain({ id: STAGING_CHAIN_ID, name: STAGING_NAME, nativeCurrency: STAGING_CURRENCY, rpcUrls: { default: { http: [node] } } });
  const pub = createPublicClient({ chain, transport: http(node) }) as PublicClient;
  const wallets = [];
  for (let i = 0; i < count; i++) {
    const account = privateKeyToAccount(generatePrivateKey());                       // the key stays here
    if ((await pub.getBytecode({ address: account.address })) !== undefined) continue;   // a delegated address: skip it, silently
    await admin("anvil_setBalance", [account.address, `0x${(200n * 10n ** 18n).toString(16)}`]);
    wallets.push({ account, client: createWalletClient({ account, chain, transport: http(node) }) });
  }
  if (!wallets.length) throw new Error("no usable wallet could be made");
  return {
    pub, wallets, admin,
    async fund(address, wei) {
      const have = BigInt(await admin<Hex>("eth_getBalance", [address, "latest"]));
      if (have < wei) await admin("anvil_setBalance", [address, `0x${(have + wei).toString(16)}`]);
    },
    async mined(hash) {
      const r = await pub.waitForTransactionReceipt({ hash, pollingInterval: 400, timeout: 60_000 });
      return { status: r.status };
    },
  };
}
