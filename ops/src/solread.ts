/**
 * docs/v9/solana-migration.md §5: the off-chain services' reads of the Solana home, keyless and read-only.
 *  - `solReader`: JSON-RPC over one or more providers (SOL_RPC_URLS), at `finalized`. An account must be byte-identical at every
 *    provider that answered (a disagreement is an error, never a pick); a balance is the least any provider reports. No method that
 *    sends or simulates exists here.
 *  - `mintSupply`: an SPL mint's supply (the wX mint the books home's reconciler checks, src/booksreserves.ts).
 * The rook_core account decoders and PDAs that lived here are retired with the programs (decision D-l, docs/v9/canister-only-solana.md
 * §0): the books are the canister's queries now.
 */

export type SolAccount = { owner: string; lamports: bigint; data: Uint8Array };
export type Filter = { dataSize: number } | { memcmp: { offset: number; bytes: string } };

export interface SolReader {
  account(addr: string, slice?: number): Promise<SolAccount | null>;
  accounts(addrs: string[]): Promise<(SolAccount | null)[]>;
  /** getProgramAccounts with filters: [pubkey, account] */
  programAccounts(program: string, filters: Filter[]): Promise<[string, SolAccount][]>;
  balance(addr: string): Promise<bigint>;
  slot(): Promise<number>;
  rent(dataLen: number): Promise<bigint>;
}

const READS = new Set(["getAccountInfo", "getMultipleAccounts", "getProgramAccounts", "getBalance", "getSlot", "getMinimumBalanceForRentExemption"]);

/** One JSON-RPC read. Refuses any method that is not a read. */
export async function solRpc(fetchImpl: typeof fetch, url: string, method: string, params: unknown[], timeoutMs = 20_000): Promise<any> { // eslint-disable-line @typescript-eslint/no-explicit-any
  if (!READS.has(method)) throw new Error(`${method} is not a read: refused`);
  const r = await fetchImpl(url, { method: "POST", headers: { "content-type": "application/json" }, redirect: "manual",
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }), signal: AbortSignal.timeout(timeoutMs) });
  if (!r.ok) throw new Error(`${hostOf(url)} ${method}: HTTP ${r.status}`);
  const j = await r.json() as { error?: { code?: number; message?: string }; result?: unknown };
  if (j.error) throw new Error(`${hostOf(url)} ${method}: ${j.error.code} ${String(j.error.message).slice(0, 160)}`);
  return j.result;
}
const hostOf = (u: string) => { try { return new URL(u).host; } catch { return "a provider"; } };

const decodeAcc = (v: any): SolAccount | null => (v ? { owner: String(v.owner), lamports: BigInt(v.lamports), data: Uint8Array.from(Buffer.from(v.data[0], "base64")) } : null); // eslint-disable-line @typescript-eslint/no-explicit-any
const sameAcc = (a: SolAccount | null, b: SolAccount | null) => (a === null) === (b === null) && (a === null || (a.owner === b!.owner && Buffer.from(a.data).equals(Buffer.from(b!.data))));

/** Every provider in `urls`, at `finalized`. Each read asks all of them; the ones that answer must agree. */
export function solReader(urls: string[], fetchImpl: typeof fetch = fetch): SolReader {
  if (!urls.length) throw new Error("no Solana RPC (SOL_RPC_URLS)");
  const all = async <X>(f: (u: string) => Promise<X>): Promise<X[]> => {
    const rs = await Promise.allSettled(urls.map(f));
    const ok = rs.filter((r): r is PromiseFulfilledResult<Awaited<X>> => r.status === "fulfilled").map((r) => r.value);
    if (!ok.length) throw new Error(`every Solana provider failed: ${rs.map((r) => (r.status === "rejected" ? String(r.reason?.message ?? r.reason).slice(0, 100) : "")).join("; ")}`);
    return ok;
  };
  const agree = <T>(xs: T[], eq: (a: T, b: T) => boolean, what: string): T => { for (const x of xs.slice(1)) if (!eq(xs[0], x)) throw new Error(`the Solana providers disagree on ${what}`); return xs[0]; };
  const min = (xs: bigint[]) => xs.reduce((m, x) => (x < m ? x : m));
  const fin = { commitment: "finalized" };
  return {
    async account(addr, slice) {
      const opts = { encoding: "base64", ...fin, ...(slice ? { dataSlice: { offset: 0, length: slice } } : {}) };
      const got = await all(async (u) => decodeAcc((await solRpc(fetchImpl, u, "getAccountInfo", [addr, opts])).value));
      const a = agree(got, sameAcc, `the account ${addr}`);
      return a && { ...a, lamports: min(got.map((g) => g!.lamports)) };
    },
    async accounts(addrs) {
      const out: (SolAccount | null)[] = [];
      for (let i = 0; i < addrs.length; i += 100) {
        const part = addrs.slice(i, i + 100);
        const got = await all(async (u) => ((await solRpc(fetchImpl, u, "getMultipleAccounts", [part, { encoding: "base64", ...fin }])).value as unknown[]).map(decodeAcc));
        for (let k = 0; k < part.length; k++) {
          const a = agree(got.map((g) => g[k] ?? null), sameAcc, `the account ${part[k]}`);
          out.push(a && { ...a, lamports: min(got.map((g) => g[k]!.lamports)) });
        }
      }
      return out;
    },
    async programAccounts(program, filters) {
      const got = await all(async (u) => ((await solRpc(fetchImpl, u, "getProgramAccounts", [program, { encoding: "base64", ...fin, filters }])) as { pubkey: string; account: unknown }[])
        .map((x) => [String(x.pubkey), decodeAcc(x.account)!] as [string, SolAccount]).sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)));
      return agree(got, (a, b) => a.length === b.length && a.every(([k, v], i) => k === b[i][0] && sameAcc(v, b[i][1])), `the accounts of ${program}`);
    },
    async balance(addr) { return min(await all(async (u) => BigInt((await solRpc(fetchImpl, u, "getBalance", [addr, fin])).value))); },
    async slot() { return Math.min(...(await all(async (u) => Number(await solRpc(fetchImpl, u, "getSlot", [fin]))))); },
    async rent(len) { return BigInt(await solRpc(fetchImpl, urls[0], "getMinimumBalanceForRentExemption", [len])); },
  };
}

/** `SOL_RPC_URLS=https://a,https://b` → the list; each must be https (http only for a local validator). */
export function parseSolRpcUrls(s: string | undefined, problems: string[], name = "SOL_RPC_URLS"): string[] {
  const urls = (s ?? "").split(",").map((x) => x.trim()).filter(Boolean);
  for (const u of urls) {
    try { const x = new URL(u); if (x.protocol !== "https:" && !["127.0.0.1", "localhost"].includes(x.hostname)) problems.push(`${name}: ${x.host} is not https`); }
    catch { problems.push(`${name}: an entry is not a URL`); }
  }
  return urls;
}

/** SPL mint (82 bytes): supply @36. */
export function mintSupply(d: Uint8Array): bigint {
  if (d.length < 82) throw new Error("not an SPL mint");
  return Buffer.from(d.buffer, d.byteOffset, d.byteLength).readBigUInt64LE(36);
}
