/**
 * docs/v9/fueler-solana.md: everything the fueler reads from or sends to Solana, behind one interface (the engine is tested against a
 * fake of it). This file signs nothing: a transaction arrives signed, after engine.ts ran `allowedSol` on its instructions.
 * Reads that decide money are taken from every provider: a balance is the least any provider reports, an account must be identical
 * at every provider that has it, a signature counts once any provider shows it finalized, and a blockhash is "expired" only when
 * every provider's finalized height is past it.
 */
export type SolAccount = { owner: string; lamports: bigint; data: Uint8Array };
export type SolStatus = { err: unknown; confirmation: "processed" | "confirmed" | "finalized" | null };

export interface SolIo {
  solBalance(addr: string): Promise<bigint>;
  /** The account at "confirmed" (null when it does not exist); `slice` reads only that many bytes from the start. */
  solAccount(addr: string, slice?: number): Promise<SolAccount | null>;
  solRent(dataLen: number): Promise<bigint>;
  solBlockhash(): Promise<{ blockhash: string; lastValidBlockHeight: number }>;
  /** The finalized block height (the least any provider reports). */
  solBlockHeight(): Promise<number>;
  solStatus(sig: string): Promise<SolStatus | null>;
  /** Simulate an (unsigned or signed) transaction: its error and logs. */
  solSimulate(tx: Uint8Array): Promise<{ err: unknown; logs: string[]; units: number }>;
  solSend(tx: Uint8Array): Promise<void>;
}

async function rpc(url: string, method: string, params: unknown[], timeoutMs = 20_000): Promise<any> { // eslint-disable-line @typescript-eslint/no-explicit-any
  const r = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
    signal: AbortSignal.timeout(timeoutMs) });
  if (!r.ok) throw new Error(`${method}: HTTP ${r.status}`);
  const j = await r.json();
  if (j.error) throw new Error(`${method}: ${j.error.code} ${String(j.error.message).slice(0, 160)}`);
  return j.result;
}
async function all<X>(urls: string[], f: (u: string) => Promise<X>): Promise<X[]> {
  const rs = await Promise.allSettled(urls.map(f));
  const ok = rs.filter((r): r is PromiseFulfilledResult<Awaited<X>> => r.status === "fulfilled").map((r) => r.value);
  if (!ok.length) throw new Error(`every Solana provider failed: ${rs.map((r) => (r.status === "rejected" ? String(r.reason?.message ?? r.reason).slice(0, 80) : "")).join("; ")}`);
  return ok;
}
const b64 = (b: Uint8Array) => Buffer.from(b).toString("base64");

export function realSolIo(urls: string[]): SolIo {
  if (urls.length < 1) throw new Error("no Solana RPC");
  return {
    async solBalance(addr) {
      const v = await all(urls, async (u) => BigInt((await rpc(u, "getBalance", [addr, { commitment: "confirmed" }])).value));
      return v.reduce((m, x) => (x < m ? x : m));
    },
    async solAccount(addr, slice) {
      const opts = { encoding: "base64", commitment: "confirmed", ...(slice ? { dataSlice: { offset: 0, length: slice } } : {}) };
      const got = await all(urls, async (u) => (await rpc(u, "getAccountInfo", [addr, opts])).value);
      const seen = got.filter(Boolean).map((v) => ({ owner: String(v.owner), lamports: BigInt(v.lamports), data: Uint8Array.from(Buffer.from(v.data[0], "base64")) }));
      if (!seen.length) return null;
      for (const s of seen.slice(1))
        if (s.owner !== seen[0].owner || !Buffer.from(s.data).equals(Buffer.from(seen[0].data))) throw new Error(`the Solana providers disagree on the account ${addr}`);
      return { ...seen[0], lamports: seen.map((s) => s.lamports).reduce((m, x) => (x < m ? x : m)) };
    },
    async solRent(len) { return BigInt(await rpc(urls[0], "getMinimumBalanceForRentExemption", [len])); },
    async solBlockhash() {
      const v = (await rpc(urls[0], "getLatestBlockhash", [{ commitment: "confirmed" }])).value;
      return { blockhash: String(v.blockhash), lastValidBlockHeight: Number(v.lastValidBlockHeight) };
    },
    async solBlockHeight() { return Math.min(...(await all(urls, async (u) => Number(await rpc(u, "getBlockHeight", [{ commitment: "finalized" }]))))); },
    async solStatus(sig) {
      const got = (await all(urls, async (u) => (await rpc(u, "getSignatureStatuses", [[sig], { searchTransactionHistory: true }])).value?.[0] ?? null))
        .filter(Boolean) as { err: unknown; confirmationStatus: SolStatus["confirmation"] }[];
      if (!got.length) return null;
      const fin = got.find((s) => s.confirmationStatus === "finalized") ?? got[0];
      return { err: fin.err ?? null, confirmation: fin.confirmationStatus ?? null };
    },
    async solSimulate(tx) {
      const v = (await rpc(urls[0], "simulateTransaction", [b64(tx), { encoding: "base64", sigVerify: false, replaceRecentBlockhash: false, commitment: "confirmed" }])).value;
      return { err: v.err ?? null, logs: v.logs ?? [], units: Number(v.unitsConsumed ?? 0) };
    },
    async solSend(tx) {
      // every provider; an answer like "already processed" is a rebroadcast, not a failure: the status decides
      await Promise.allSettled(urls.map((u) => rpc(u, "sendTransaction", [b64(tx), { encoding: "base64", skipPreflight: true, maxRetries: 0 }])));
    },
  };
}
