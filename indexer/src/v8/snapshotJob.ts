import { parseAbi } from "viem";
import { reader, type Fetch } from "./jsonrpc.js";
import { diffSnapshots, snapshotHash, traitsOf } from "./rarity.js";
import type { Address, Hex, MetadataSnapshot, SnapshotDrift, SnapshotToken } from "./types.js";

/**
 * docs/v8/redemption-draw.md §2.1: one snapshot of a whole collection's metadata, taken once at deploy and hashed; later re-reads
 * are diffs, never applied. `tokenURI(id)` is read on Ethereum (keyless), an ipfs:// URI through public gateways, an http(s) one from
 * the project's own server (Milady and MAYC serve theirs so, measured 2026-09-27: the reason the snapshot exists). Only
 * `attributes[].trait_type/value` is kept. Bounded: a deadline and a size cap per answer, a fixed concurrency.
 */
const TOKEN_URI = parseAbi(["function tokenURI(uint256 id) view returns (string)"]);
export const IPFS_GATEWAYS = ["https://ipfs.io/ipfs/", "https://dweb.link/ipfs/"] as const;
const MAX_META = 256 * 1024;

export type SnapshotJob = { collection: Address; name: string; tokenIds: string[]; ethRpc: string; fetch: Fetch; now: number; concurrency?: number; gateways?: readonly string[]; timeoutMs?: number };

async function getJson(fetchImpl: Fetch, url: string, timeoutMs: number): Promise<unknown> {
  const r = await fetchImpl(url, { headers: { accept: "application/json" }, signal: AbortSignal.timeout(timeoutMs) });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  const t = await r.text();
  if (t.length > MAX_META) throw new Error("metadata too large");
  return JSON.parse(t);
}
/** The metadata URL(s) to try for a tokenURI: IPFS through each gateway, http(s) as is, a data: JSON inline. */
export function metadataUrls(uri: string, gateways: readonly string[] = IPFS_GATEWAYS): string[] {
  if (uri.startsWith("ipfs://")) { const path = uri.slice(7).replace(/^ipfs\//, ""); return gateways.map((g) => `${g}${path}`); }
  if (/^https?:\/\//.test(uri)) return [uri];
  return [];
}
export const ipfsCidOf = (uri: string): string | null => (uri.startsWith("ipfs://") ? uri.slice(7).replace(/^ipfs\//, "").split("/")[0] : null);

async function readToken(j: SnapshotJob, tokenId: string): Promise<SnapshotToken | null> {
  const eth = reader(j.ethRpc, "ethereum", j.fetch, null);
  const uri = await eth.call<string>(j.collection, TOKEN_URI, "tokenURI", [BigInt(tokenId)], "finalized");
  if (uri.startsWith("data:application/json;base64,")) { const t = traitsOf(JSON.parse(atob(uri.slice(29)))); return t ? { tokenId, traits: t } : null; }
  for (const url of metadataUrls(uri, j.gateways)) {
    try { const t = traitsOf(await getJson(j.fetch, url, j.timeoutMs ?? 15_000)); if (t) return { tokenId, traits: t }; } catch { /* the next gateway */ }
  }
  return null;
}

export async function takeSnapshot(j: SnapshotJob): Promise<{ snapshot: MetadataSnapshot; sha256: Hex; unreadable: string[] }> {
  const out: (SnapshotToken | null)[] = new Array(j.tokenIds.length);
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(j.concurrency ?? 4, j.tokenIds.length) }, async () => {
    while (next < j.tokenIds.length) { const i = next++; out[i] = await readToken(j, j.tokenIds[i]).catch(() => null); }
  }));
  const unreadable = j.tokenIds.filter((_, i) => !out[i]);
  if (unreadable.length) throw new Error(`${unreadable.length} tokens could not be read (${unreadable.slice(0, 5).join(", ")}…): a snapshot is whole or it is not taken`);
  const first = await reader(j.ethRpc, "ethereum", j.fetch, null).call<string>(j.collection, TOKEN_URI, "tokenURI", [BigInt(j.tokenIds[0])], "finalized");
  const snapshot: MetadataSnapshot = { version: 1, collection: j.collection, chainId: 1, name: j.name, takenAt: j.now,
    source: { tokenURI: first.replace(new RegExp(`${j.tokenIds[0]}(\\.json)?$`), "{id}$1"), ipfsCid: ipfsCidOf(first), note: `tokenURI read on ${new URL(j.ethRpc).host} at the finalized block; ${j.tokenIds.length} tokens` },
    tokens: out as SnapshotToken[] };
  return { snapshot, sha256: snapshotHash(snapshot), unreadable };
}

/** A later re-read against the pinned snapshot: what changed, shown and never applied. */
export async function driftOf(pinned: MetadataSnapshot, j: Omit<SnapshotJob, "tokenIds" | "collection" | "name">): Promise<SnapshotDrift> {
  const job = { ...j, collection: pinned.collection, name: pinned.name, tokenIds: pinned.tokens.map((t) => t.tokenId) };
  const now: (SnapshotToken | null)[] = [];
  for (const id of job.tokenIds) now.push(await readToken(job, id).catch(() => null));
  return diffSnapshots(pinned, { tokens: now }, j.now);
}
