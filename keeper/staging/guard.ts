import { readFileSync } from "node:fs";
/**
 * docs/phase6.md, tier 1. THE ONLY PLACE that can send to our staging fork, and it cannot send anywhere else.
 *
 * It refuses unless KEEPER_MODE is exactly "staging", the RPC is on THIS machine, and the node itself reports the staging chain id (staging/chain-id.json; never 4663, never 46630) —
 * asked of the node before every send, because a URL can be repointed while a process runs. It holds no key and there is none to
 * give it: the staging node sends for whatever address it is asked to (its roles are random addresses nobody has a key to), and
 * it will only do that for a caller on the same machine, because the public proxy blocks eth_sendTransaction and every admin method.
 * Everything under keeper/src/ remains unable to send at all.
 */
/** Read from the repository's one source; a real network's number there is refused before anything else happens. */
export const STAGING_CHAIN_ID: number = (JSON.parse(readFileSync(new URL("../../staging/chain-id.json", import.meta.url), "utf8")) as { chainId: number }).chainId;
if (!Number.isSafeInteger(STAGING_CHAIN_ID) || [4663, 46630, 31337, 1].includes(STAGING_CHAIN_ID)) throw new Error(`refused: staging/chain-id.json names chain ${STAGING_CHAIN_ID}, which is not ours alone`);
export type Env = Readonly<Record<string, string | undefined>>;
export type Fetch = typeof fetch;
export type Receipt = { status: string; transactionHash: string; blockNumber: string; gasUsed: string; logs: { address: string; topics: string[]; data: string }[] };
export type StagingSender = {
  request<T>(method: string, params?: unknown[]): Promise<T>;
  send(tx: { from: string; to: string; data: string; value?: bigint }): Promise<Receipt>;
};

export const isThisMachine = (url: string): boolean => {
  try { return ["127.0.0.1", "localhost", "[::1]", "::1"].includes(new URL(url).hostname); } catch { return false; }
};

export async function stagingSender(url: string, env: Env = process.env, fetchImpl: Fetch = fetch): Promise<StagingSender> {
  if (env.KEEPER_MODE !== "staging") throw new Error("refused: KEEPER_MODE is not \"staging\"");
  if (!isThisMachine(url)) throw new Error("refused: the staging operator sends only through a node on this machine, never through a public address");
  const raw = async <T>(method: string, params: unknown[] = []): Promise<T> => {
    const r = await fetchImpl(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
    const j = (await r.json()) as { result?: T; error?: { message: string; data?: unknown } };
    if (j.error) throw new Error(`${method}: ${j.error.message}${typeof j.error.data === "string" ? ` (${j.error.data})` : ""}`);
    return j.result as T;
  };
  const check = async () => {
    const id = Number(BigInt(await raw<string>("eth_chainId")));
    if (id !== STAGING_CHAIN_ID) throw new Error(`refused: the node reports chain id ${id}, not ${STAGING_CHAIN_ID}. Robinhood Chain is 4663 and is never sent to from here.`);
  };
  await check();
  return {
    request: raw,
    async send(tx) {
      await check();
      const hash = await raw<string>("eth_sendTransaction", [{ from: tx.from, to: tx.to, data: tx.data, value: `0x${(tx.value ?? 0n).toString(16)}` }]);
      for (let i = 0; i < 150; i++) {
        const receipt = await raw<Receipt | null>("eth_getTransactionReceipt", [hash]);
        if (receipt) { if (receipt.status !== "0x1") throw new Error(`reverted: ${tx.to} ${tx.data.slice(0, 10)} (tx ${hash})`); return receipt; }
        await new Promise((ok) => setTimeout(ok, 100));
      }
      throw new Error(`no receipt for ${hash}`);
    },
  };
}
