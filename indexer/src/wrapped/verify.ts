import { decodeFunctionResult, encodeFunctionData, type Hex } from "viem";
import { jsonCapped } from "../outbound.js";
import type { ItemCheck } from "../../types.js";
import { hex, rpcOf, type Fetch } from "../rpc.js";
import { ERC721_READS } from "./abis.js";

/**
 * docs/v5/ux-and-reserves.md §2, "Sources". Each held item checked where it actually lives, from public keyless sources a reader
 * can open themselves. A source that fails, disagrees with another, or cannot be parsed makes the item `unverified`; only a
 * source that positively names another owner makes it `elsewhere`; only every source agreeing on custody makes it `in-custody`.
 * Reads only: `rpcOf` refuses anything that is not a read, and the HTTP calls are GETs.
 */

/** The canister's own Bitcoin sources (docs/deploy/canister-mainnet.md, docs/v2/rates-and-indexer.md). */
export const ORD = "https://ordinals.com";
export const ESPLORA = ["https://mempool.space/api", "https://blockstream.info/api"] as const;
export const ORD_PAGE = (id: string) => `https://ordinals.com/inscription/${id}`;
export const BTC_TX_PAGE = (txid: string) => `https://mempool.space/tx/${txid}`;
export const BTC_ADDRESS_PAGE = (a: string) => `https://mempool.space/address/${a}`;

async function getJson(fetchImpl: Fetch, url: string, timeoutMs: number): Promise<unknown> {
  const r = await fetchImpl(url, { headers: { accept: "application/json", "user-agent": "etch-indexer/1 (proof of reserves, read-only)" }, signal: AbortSignal.timeout(timeoutMs) });
  if (!r.ok) { await r.body?.cancel().catch(() => undefined); throw new Error(`HTTP ${r.status}`); }
  return jsonCapped(r);   // E-12: fixed hosts, but a bounded body all the same
}
const msg = (e: unknown) => String((e as Error)?.message ?? e).slice(0, 120);
const host = (u: string) => { try { return new URL(u).host; } catch { return u; } };

export type BitcoinSources = { fetch: Fetch; ord?: string; esplora?: readonly string[]; now: () => number; timeoutMs?: number };

export async function checkInscription(inscriptionId: string, custody: string, src: BitcoinSources): Promise<ItemCheck> {
  const ord = src.ord ?? ORD, esplora = src.esplora ?? ESPLORA, t = src.timeoutMs ?? 10_000;
  const sources: ItemCheck["sources"] = [];
  const out = (verdict: ItemCheck["verdict"], reason: string | null, atBlock: number | null = null, arrivalTx: string | null = null): ItemCheck =>
    ({ verdict, reason, checkedAt: src.now(), atBlock, sources, arrivalTx });
  let where: { address?: unknown; satpoint?: unknown };
  const ordUrl = `${ord}/r/inscription/${inscriptionId}`;
  try { where = (await getJson(src.fetch, ordUrl, t)) as typeof where; }
  catch (e) { sources.push({ name: "ord", url: ORD_PAGE(inscriptionId), said: `no answer: ${msg(e)}`, ok: false }); return out("unverified", `ord did not answer (${msg(e)})`); }
  const address = typeof where?.address === "string" ? where.address : null;
  const sp = typeof where?.satpoint === "string" ? where.satpoint.match(/^([0-9a-f]{64}):(\d+):(\d+)$/) : null;
  if (!address || !sp) { sources.push({ name: "ord", url: ORD_PAGE(inscriptionId), said: "an answer without an address or a satpoint", ok: false }); return out("unverified", "ord's answer had no address or satpoint"); }
  const [, txid, voutS] = sp, vout = Number(voutS);
  if (address !== custody) {
    sources.push({ name: "ord", url: ORD_PAGE(inscriptionId), said: `at ${address}, not the custody address`, ok: false });
    return out("elsewhere", `ord places it at ${address}`, null, txid);
  }
  sources.push({ name: "ord", url: ORD_PAGE(inscriptionId), said: `at the custody address, in output ${txid}:${vout}`, ok: true });

  let height: number | null = null;
  for (const base of esplora) {
    const name = host(base);
    try {
      const tx = (await getJson(src.fetch, `${base}/tx/${txid}`, t)) as { vout?: { scriptpubkey_address?: string }[]; status?: { confirmed?: boolean; block_height?: number } };
      const spend = (await getJson(src.fetch, `${base}/tx/${txid}/outspend/${vout}`, t)) as { spent?: boolean };
      const pays = tx?.vout?.[vout]?.scriptpubkey_address;
      if (pays !== custody) { sources.push({ name, url: BTC_TX_PAGE(txid), said: `output ${vout} pays ${pays ?? "nothing it can name"}`, ok: false }); return out("unverified", `${name} disagrees with ord about output ${vout}`, height, txid); }
      if (spend?.spent !== false) { sources.push({ name, url: BTC_TX_PAGE(txid), said: spend?.spent ? "that output is spent" : "no spent flag", ok: false }); return out("unverified", `${name} says the output ord names is ${spend?.spent ? "spent" : "of unknown state"}`, height, txid); }
      if (!tx.status?.confirmed) { sources.push({ name, url: BTC_TX_PAGE(txid), said: "unconfirmed", ok: false }); return out("unverified", `${name} has the output unconfirmed`, height, txid); }
      const h = Number(tx.status.block_height);
      height = height === null ? h : Math.min(height, h);
      sources.push({ name, url: BTC_TX_PAGE(txid), said: `unspent, pays custody, confirmed at height ${h}`, ok: true });
    } catch (e) {
      sources.push({ name, url: BTC_TX_PAGE(txid), said: `no answer: ${msg(e)}`, ok: false });
      return out("unverified", `${name} did not answer (${msg(e)})`, height, txid);
    }
  }
  return out("in-custody", null, height, txid);
}

export type EvmSources = { fetch: Fetch; chainId: number; providers: string[]; now: () => number; margin?: number; timeoutMs?: number };
export type Pin = { ok: true; block: number; at: number | null } | { ok: false; error: string };

/** One recent block every provider can answer about: the lowest head over providers, less a margin. Every provider must be on `chainId`. */
export async function pinBlock(src: EvmSources): Promise<Pin> {
  if (src.providers.length === 0) return { ok: false, error: "no providers are configured for this chain" };
  try {
    // each provider's failure is named by its host, so the page can say which one to blame
    const rpcs = src.providers.map((u) => { const r = rpcOf(u, src.fetch, src.timeoutMs ?? 10_000);
      return <T,>(m: string, p: unknown[]) => r<T>(m, p).catch((e) => { throw new Error(`${host(u)}: ${msg(e)}`); }); });
    const ids = await Promise.all(rpcs.map((r) => r<string>("eth_chainId", [])));
    const wrong = ids.findIndex((x) => Number(BigInt(x)) !== src.chainId);
    if (wrong >= 0) return { ok: false, error: `${host(src.providers[wrong])} is on chain ${Number(BigInt(ids[wrong]))}, not ${src.chainId}` };
    const heads = await Promise.all(rpcs.map((r) => r<string>("eth_blockNumber", [])));
    const block = Math.min(...heads.map((h) => Number(BigInt(h)))) - (src.margin ?? 8);
    const b = await rpcs[0]<{ timestamp: Hex } | null>("eth_getBlockByNumber", [hex(block), false]);
    return { ok: true, block, at: b ? Number(BigInt(b.timestamp)) : null };
  } catch (e) { return { ok: false, error: msg(e) }; }
}

export async function checkOwner(tokenId: string, collection: string, custodian: string, pin: Pin, src: EvmSources, pageOf: (tokenId: string) => string | null): Promise<ItemCheck> {
  const sources: ItemCheck["sources"] = [];
  const page = pageOf(tokenId) ?? "";
  if (!pin.ok) return { verdict: "unverified", reason: `no block to read at: ${pin.error}`, checkedAt: src.now(), atBlock: null, sources, arrivalTx: null };
  const data = encodeFunctionData({ abi: ERC721_READS, functionName: "ownerOf", args: [BigInt(tokenId)] });
  const said = await Promise.all(src.providers.map(async (u) => {
    try {
      const out = await rpcOf(u, src.fetch, src.timeoutMs ?? 10_000)<Hex>("eth_call", [{ to: collection, data }, hex(pin.block)]);
      return { owner: (decodeFunctionResult({ abi: ERC721_READS, functionName: "ownerOf", data: out }) as string).toLowerCase(), reverted: false, error: null as string | null };
    } catch (e) { const m = msg(e); return { owner: null, reverted: /revert/i.test(m), error: m }; }
  }));
  said.forEach((x, i) => sources.push({ name: host(src.providers[i]), url: page, said: x.owner ? `ownerOf = ${x.owner} at block ${pin.block}` : x.reverted ? `ownerOf reverted at block ${pin.block}` : `no answer: ${x.error}`, ok: x.owner === custodian.toLowerCase() }));
  const base = { checkedAt: pin.at ?? src.now(), atBlock: pin.block, sources, arrivalTx: null };
  if (said.every((x) => x.owner === custodian.toLowerCase())) return { verdict: "in-custody", reason: null, ...base };
  if (said.every((x) => x.reverted)) return { verdict: "elsewhere", reason: "ownerOf reverts on every provider: the token does not exist or was burned", ...base };
  const owners = new Set(said.map((x) => x.owner));
  if (owners.size === 1 && !owners.has(null)) return { verdict: "elsewhere", reason: `every provider names ${[...owners][0]} as its owner`, ...base };
  return { verdict: "unverified", reason: said.some((x) => x.owner === null) ? "a provider did not answer" : "the providers disagree about its owner", ...base };
}
