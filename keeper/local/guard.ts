/**
 * docs/phase2.md §3. THE ONLY PLACE in this package that can send a transaction, and it cannot send to a real chain.
 *
 * It refuses unless KEEPER_MODE is exactly "local" AND the node itself reports chain id 31337 — asked of the node, not of the
 * caller. It holds no key and there is none to give it: it sends with eth_sendTransaction from accounts the local node has
 * unlocked (anvil's dev accounts, whose keys anvil prints at every start and which are worthless anywhere real).
 * Everything under keeper/src/ remains unable to send at all; test/cannot-send.test.ts proves both halves.
 */
export const LOCAL_CHAIN_ID = 31337;
export type Env = Readonly<Record<string, string | undefined>>;
export type Fetch = typeof fetch;
export type Receipt = { status: string; transactionHash: string; blockNumber: string; gasUsed: string; logs: { address: string; topics: string[]; data: string }[] };

export type LocalSender = {
  /** Any JSON-RPC read, and the local node's own harness methods. */
  request<T>(method: string, params?: unknown[]): Promise<T>;
  /** Sends from an account the local node has unlocked, waits for the receipt, throws on a revert with the node's reason. */
  send(tx: { from: string; to: string; data: string; value?: bigint }): Promise<Receipt>;
  accounts(): Promise<string[]>;
};

export async function localSender(url: string, env: Env = process.env, fetchImpl: Fetch = fetch): Promise<LocalSender> {
  if (env.KEEPER_MODE !== "local") {
    throw new Error("refused: KEEPER_MODE is not \"local\". This package sends nothing to any real chain, and there is no mode that does.");
  }
  const raw = async <T>(method: string, params: unknown[] = []): Promise<T> => {
    const r = await fetchImpl(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
    const j = (await r.json()) as { result?: T; error?: { message: string; data?: unknown } };
    if (j.error) throw new Error(`${method}: ${j.error.message}${typeof j.error.data === "string" ? ` (${j.error.data})` : ""}`);
    return j.result as T;
  };
  const chainId = Number(BigInt(await raw<string>("eth_chainId")));
  if (chainId !== LOCAL_CHAIN_ID) {
    throw new Error(`refused: the node at this RPC reports chain id ${chainId}, not ${LOCAL_CHAIN_ID}. Only a local development chain is ever sent to.`);
  }
  // asked again before EVERY send: an RPC URL can be repointed while a process runs
  const stillLocal = async () => {
    const now = Number(BigInt(await raw<string>("eth_chainId")));
    if (now !== LOCAL_CHAIN_ID) throw new Error(`refused: the node now reports chain id ${now}`);
  };
  return {
    request: raw,
    accounts: () => raw<string[]>("eth_accounts"),
    async send(tx) {
      await stillLocal();
      const call = { from: tx.from, to: tx.to, data: tx.data, value: `0x${(tx.value ?? 0n).toString(16)}` };
      const hash = await raw<string>("eth_sendTransaction", [call]);
      for (let i = 0; i < 100; i++) {
        const receipt = await raw<Receipt | null>("eth_getTransactionReceipt", [hash]);
        if (receipt) {
          if (receipt.status !== "0x1") throw new Error(`reverted: ${tx.to} ${tx.data.slice(0, 10)} (tx ${hash})`);
          return receipt;
        }
        await new Promise((ok) => setTimeout(ok, 100));
      }
      throw new Error(`no receipt for ${hash}`);
    },
  };
}
