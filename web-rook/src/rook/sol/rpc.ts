/** A small JSON-RPC client for the Solana methods the site calls from the browser. No websocket: confirmations are polled. */
import { b64decode, b64encode } from "./base58";

export type Account = { owner: string; lamports: number; data: Uint8Array; executable: boolean };
export type Commitment = "processed" | "confirmed" | "finalized";
type Raw = { owner: string; lamports: number; data: [string, string]; executable: boolean } | null;
const acct = (v: Raw): Account | null => (v ? { owner: v.owner, lamports: v.lamports, data: b64decode(v.data[0]), executable: v.executable } : null);

export class Rpc {
  private id = 0;
  constructor(public readonly url: string, private readonly fetcher: typeof fetch = (...a) => fetch(...a)) {}

  /** One JSON-RPC call; a 429 (rate limit) is retried three times, 0.8 s, 1.6 s, 3.2 s apart. */
  async call<T>(method: string, params: unknown[] = [], attempt = 0): Promise<T> {
    const r = await this.fetcher(this.url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: ++this.id, method, params }) });
    if (r.status === 429 && attempt < 3) { await new Promise((ok) => setTimeout(ok, 800 * 2 ** attempt)); return this.call<T>(method, params, attempt + 1); }
    if (!r.ok) throw new Error(`${method}: HTTP ${r.status}${r.status === 429 ? " (the RPC is rate-limiting; try again in a moment)" : ""}`);
    const j = (await r.json()) as { result?: T; error?: { code: number; message: string } };
    if (j.error) throw new Error(`${method}: ${j.error.message}`);
    return j.result as T;
  }

  async account(address: string, commitment: Commitment = "confirmed"): Promise<Account | null> {
    const r = await this.call<{ value: Raw }>("getAccountInfo", [address, { encoding: "base64", commitment }]);
    return acct(r.value);
  }
  /** getMultipleAccounts in pages of 100 (the method's limit), in the order asked. */
  async accounts(addresses: string[], commitment: Commitment = "confirmed"): Promise<(Account | null)[]> {
    const out: (Account | null)[] = [];
    for (let i = 0; i < addresses.length; i += 100) {
      const r = await this.call<{ value: Raw[] }>("getMultipleAccounts", [addresses.slice(i, i + 100), { encoding: "base64", commitment }]);
      out.push(...r.value.map(acct));
    }
    return out;
  }
  async programAccounts(program: string, filters: unknown[], dataSlice?: { offset: number; length: number }): Promise<{ pubkey: string; account: Account }[]> {
    const r = await this.call<{ pubkey: string; account: NonNullable<Raw> }[]>("getProgramAccounts", [program, { encoding: "base64", commitment: "confirmed", filters, ...(dataSlice ? { dataSlice } : {}) }]);
    return r.map((x) => ({ pubkey: x.pubkey, account: acct(x.account)! }));
  }
  async balance(address: string): Promise<bigint> {
    return BigInt((await this.call<{ value: number }>("getBalance", [address, { commitment: "confirmed" }])).value);
  }
  async rentExempt(bytes: number): Promise<bigint> { return BigInt(await this.call<number>("getMinimumBalanceForRentExemption", [bytes])); }
  async latestBlockhash(): Promise<{ blockhash: string; lastValidBlockHeight: number }> {
    return (await this.call<{ value: { blockhash: string; lastValidBlockHeight: number } }>("getLatestBlockhash", [{ commitment: "confirmed" }])).value;
  }
  async genesisHash(): Promise<string> { return this.call<string>("getGenesisHash"); }
  /** Simulate a transaction (signatures not verified, the blockhash replaced), optionally returning accounts' state after it. */
  async simulate(wire: Uint8Array, accounts: string[] = []): Promise<{ err: unknown; logs: string[]; unitsConsumed?: number; accounts: (Account | null)[] }> {
    const r = await this.call<{ value: { err: unknown; logs: string[] | null; unitsConsumed?: number; accounts?: Raw[] | null } }>("simulateTransaction", [b64encode(wire), {
      encoding: "base64", sigVerify: false, replaceRecentBlockhash: true, commitment: "confirmed",
      ...(accounts.length ? { accounts: { encoding: "base64", addresses: accounts } } : {}),
    }]);
    return { err: r.value.err, logs: r.value.logs ?? [], unitsConsumed: r.value.unitsConsumed, accounts: (r.value.accounts ?? []).map(acct) };
  }
  async send(wire: Uint8Array): Promise<string> {
    return this.call<string>("sendTransaction", [b64encode(wire), { encoding: "base64", skipPreflight: false, preflightCommitment: "confirmed", maxRetries: 5 }]);
  }
  async signatureStatus(sig: string): Promise<{ confirmationStatus?: string; err: unknown } | null> {
    return (await this.call<{ value: ({ confirmationStatus?: string; err: unknown } | null)[] }>("getSignatureStatuses", [[sig], { searchTransactionHistory: false }])).value[0];
  }
  async blockHeight(): Promise<number> { return this.call<number>("getBlockHeight", [{ commitment: "confirmed" }]); }
}

/** Wait until a signature is confirmed, it failed, or its blockhash expired. */
export async function confirm(rpc: Rpc, sig: string, lastValidBlockHeight: number, sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))): Promise<void> {
  for (let i = 0; i < 120; i++) {
    const s = await rpc.signatureStatus(sig).catch(() => null);
    if (s?.err) throw new Error(`the transaction failed on chain: ${JSON.stringify(s.err)}`);
    if (s && (s.confirmationStatus === "confirmed" || s.confirmationStatus === "finalized")) return;
    if (i % 5 === 4 && (await rpc.blockHeight().catch(() => 0)) > lastValidBlockHeight) throw new Error("the transaction expired before it landed (its blockhash is too old). Nothing was spent; try again.");
    await sleep(1_000);
  }
  throw new Error(`not confirmed after two minutes; look it up: ${sig}`);
}
