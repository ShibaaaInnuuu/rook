/**
 * Who holds a coin. Three sources, best first:
 *   1. getProgramAccounts on the token program, filtered by mint, sliced to owner+amount: every holder (complete).
 *   2. Helius DAS getTokenAccounts (HELIUS_API_KEY), paged: every holder (complete), for an RPC that refuses (1).
 *   3. getTokenLargestAccounts + the owners: the top 20 only (complete: false, no count).
 * Labels: the DBC pool's base vault (owned by DBC's pool authority) is `curve`, the DAMM v2 pool's vault is `pool`, the pool's
 * creator is `creator`. The holder count leaves out `curve` and `pool`.
 */
import { b58encode, b64decode, u64At } from "./bytes.js";
import { TOKEN_2022_PROGRAM, TOKEN_PROGRAM } from "./dbc.js";
import type { Fetch, SolRpc } from "./rpc.js";

export const DBC_POOL_AUTHORITY = "FhVo3mqL8PW5pH5U2CN4XE33DokiyZnUwuGpH2hmHLuM";
export const DAMM_V2_POOL_AUTHORITY = "HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC";

export type RawHolding = { tokenAccount: string; owner: string; amount: bigint };
export type Fetched = { complete: boolean; rows: RawHolding[]; via: "gpa" | "helius" | "largest"; gpaRefused?: boolean };
export type Labelled = { address: string; tokenAccount: string; amount: bigint; label: "creator" | "curve" | "pool" | null };
export type HolderSnapshot = { count: number | null; complete: boolean; items: Labelled[] };

export function label(rows: RawHolding[], k: { creator: string | null; curveVaults: string[]; poolVaults: string[] }, complete: boolean, keep = 100): HolderSnapshot {
  const curve = new Set(k.curveVaults.filter(Boolean)), pool = new Set(k.poolVaults.filter(Boolean));
  const byOwner = new Map<string, Labelled>();
  for (const r of rows) {
    if (r.amount <= 0n) continue;
    const lab: Labelled["label"] = curve.has(r.tokenAccount) || r.owner === DBC_POOL_AUTHORITY ? "curve"
      : pool.has(r.tokenAccount) || r.owner === DAMM_V2_POOL_AUTHORITY ? "pool" : r.owner === k.creator ? "creator" : null;
    // a vault is its own row; a wallet's several token accounts are one holder
    const key = lab === "curve" || lab === "pool" ? `${lab}:${r.tokenAccount}` : r.owner;
    const prev = byOwner.get(key);
    if (prev) prev.amount += r.amount;
    else byOwner.set(key, { address: r.owner, tokenAccount: r.tokenAccount, amount: r.amount, label: lab });
  }
  const all = [...byOwner.values()].sort((a, b) => (b.amount > a.amount ? 1 : b.amount < a.amount ? -1 : a.address.localeCompare(b.address)));
  const count = complete ? all.filter((x) => x.label !== "curve" && x.label !== "pool").length : null;
  return { count, complete, items: all.slice(0, keep) };
}

type GpaRow = { pubkey: string; account: { data: [string, string] } };
export async function viaGpa(rpc: SolRpc, mint: string, tokenProgram = TOKEN_PROGRAM): Promise<RawHolding[]> {
  const filters: unknown[] = [{ memcmp: { offset: 0, bytes: mint } }];
  if (tokenProgram === TOKEN_PROGRAM) filters.unshift({ dataSize: 165 });
  const rows = await rpc.call<GpaRow[]>("getProgramAccounts", [tokenProgram, { encoding: "base64", filters, dataSlice: { offset: 32, length: 40 }, commitment: "confirmed" }]);
  return rows.map((r) => { const d = b64decode(r.account.data[0]); return { tokenAccount: r.pubkey, owner: b58encode(d.subarray(0, 32)), amount: u64At(d, 32) }; });
}
export async function viaHelius(key: string, mint: string, f: Fetch = fetch, cluster = "mainnet"): Promise<RawHolding[]> {
  const out: RawHolding[] = [];
  for (let page = 1; page <= 50; page++) {
    const r = await f(`https://${cluster}.helius-rpc.com/?api-key=${encodeURIComponent(key)}`, { method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: page, method: "getTokenAccounts", params: { mint, page, limit: 1000 } }), signal: AbortSignal.timeout(20_000) });
    if (!r.ok) throw new Error(`helius getTokenAccounts: HTTP ${r.status}`);
    const j = (await r.json()) as { result?: { token_accounts?: { address: string; owner: string; amount: number | string }[] }; error?: { message?: string } };
    if (j.error) throw new Error(`helius getTokenAccounts: ${j.error.message}`);
    const xs = j.result?.token_accounts ?? [];
    for (const x of xs) out.push({ tokenAccount: x.address, owner: x.owner, amount: BigInt(typeof x.amount === "string" ? x.amount : Math.round(x.amount)) });
    if (xs.length < 1000) break;
  }
  return out;
}
export async function viaLargest(rpc: SolRpc, mint: string): Promise<RawHolding[]> {
  const top = await rpc.call<{ value: { address: string; amount: string }[] }>("getTokenLargestAccounts", [mint, { commitment: "confirmed" }]);
  const addrs = top.value.map((v) => v.address);
  if (!addrs.length) return [];
  const accs = await rpc.call<{ value: ({ data: { parsed?: { info?: { owner?: string } } } } | null)[] }>("getMultipleAccounts", [addrs, { encoding: "jsonParsed" }]);
  return top.value.map((v, i) => ({ tokenAccount: v.address, owner: accs.value[i]?.data?.parsed?.info?.owner ?? v.address, amount: BigInt(v.amount) }));
}

export async function fetchHolders(rpc: SolRpc, mint: string, o: { tokenProgram?: string; heliusKey?: string | null; fetch?: Fetch; cluster?: string; preferGpa?: boolean }): Promise<Fetched> {
  const prog = o.tokenProgram === TOKEN_2022_PROGRAM ? TOKEN_2022_PROGRAM : TOKEN_PROGRAM;
  // an empty answer is never "nobody holds it" (a coin with a pool always has a vault): it means the wrong program or a lagging index
  if (o.heliusKey) { try { const rows = await viaHelius(o.heliusKey, mint, o.fetch, o.cluster === "devnet" ? "devnet" : "mainnet"); if (rows.length) return { complete: true, rows, via: "helius" }; } catch { /* next */ } }
  let gpaRefused = false;
  if (o.preferGpa !== false) { try { const rows = await viaGpa(rpc, mint, prog); if (rows.length) return { complete: true, rows, via: "gpa" }; } catch { gpaRefused = true; } }
  return { complete: false, rows: await viaLargest(rpc, mint), via: "largest", gpaRefused };
}
