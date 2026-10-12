import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { decodeEventLog, decodeFunctionResult, encodeFunctionData, parseAbi, type Hex } from "viem";
import { CATALOGUE_PATH, STAGING_FIXTURE_MEMBERS, listingCalls, plannedListings, pricesFor,
  type Catalogue, type ListingPlan, type PlannedListing } from "../catalogue/listings.js";
import { assertBootstrapTarget } from "./bootstrap.js";

/**
 * docs/phase11.md §6. The catalogue's 58 listings, created on OUR chain only, in batches, beside a proxy that is already
 * answering. Every listing is a contract deployment (ListingRegistry._list does `new AssetVault`), so this is 58 deployments
 * and it does not belong inside the boot path: the node is up, /healthz is up, and this fills the inventory behind them,
 * writing its progress to ${data}/listings.json after every batch.
 *
 * It is idempotent. It reads what the registry already holds and creates only what is missing, so a restart in the middle
 * finishes the inventory instead of duplicating it.
 *
 * A real chain never comes here: `npm run listings:calldata` prints the same calls for a multisig to sign.
 */
const LISTED = parseAbi(["event Listed(uint256 indexed id, uint8 kind, address vault, string name, string ref)"]);
const REGISTRY = parseAbi([
  "function count() view returns (uint256)",
  "function listing(uint256 id) view returns ((uint8 kind, bool paused, uint8 divisibility, address vault, bytes32 membershipRoot, string name, string ref, string btcVault))",
]);

export type ManifestListing = {
  id: number; kind: "inscriptions" | "rune"; name: string; ref: string; vault: string; btcVault: string;
  divisibility: number; membershipRoot: string | null; floorSats: number | null; weight: number;
  fromCatalogue: boolean; rootCovers: PlannedListing["rootCovers"]; sourceUrl: string | null;
};
export type Progress = { created: number; total: number; done: boolean; startedAt: string; finishedAt: string | null; ms: number | null };
export type Manifest = {
  source: string; fetchedAt: string; chainId: number; placeholderRoots: boolean; note: string;
  progress: Progress; listings: ManifestListing[]; skipped: ListingPlan["skipped"];
};

export const MANIFEST_NOTE =
  "Every membership root here is a PLACEHOLDER: the catalogue holds a few of each collection's inscription ids, not its "
  + "membership, and the rest of each root is staging fixture ids that name nothing. Nothing has verified any collection's full "
  + "item list. Every btcVault is a marker string that is not a valid Bitcoin address, so nobody can be paid at one. Runes carry "
  + "no floor and so no cap: their vaults refuse to declare a purchase. docs/phase11.md.";

export const manifestPath = (data: string): string => `${data}/listings.json`;
export const readManifest = (data: string): Manifest | null => {
  try { return JSON.parse(readFileSync(manifestPath(data), "utf8")) as Manifest; } catch { return null; }
};

export function readCatalogue(repo: string): Catalogue {
  const path = `${repo}/${CATALOGUE_PATH}`;
  if (!existsSync(path)) throw new Error(`the catalogue is not here: ${path}. The staging image copies it in (staging/chain/Dockerfile); without it there is nothing to list.`);
  const cat = JSON.parse(readFileSync(path, "utf8")) as Catalogue;
  if (!Array.isArray(cat.collections) || !Array.isArray(cat.runes)) throw new Error(`${path} is not a catalogue: it has no collections and runes`);
  return cat;
}

type Rpc = <T>(method: string, params?: unknown[]) => Promise<T>;
const rpcFor = (node: string, fetchImpl: typeof fetch): Rpc => async <T>(method: string, params: unknown[] = []): Promise<T> => {
  const r = await fetchImpl(node, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
  const j = (await r.json()) as { result?: T; error?: { message: string } };
  if (j.error) throw new Error(`${method}: ${j.error.message}`);
  return j.result as T;
};

export type CreateOptions = {
  node: string; data: string; repo: string;
  /** how many listings are sent before their receipts are awaited and progress is written */
  batch?: number;
  /** test seam: the parsed catalogue, and the chain's own fetch */
  catalogue?: Catalogue;
  fetchImpl?: typeof fetch;
  log?: (o: Record<string, unknown>) => void;
  /** written into every collection's root on our own chains; 0 builds a root over the real ids alone */
  fixtureMembers?: number;
};

/** Creates whatever the registry is missing. Returns the manifest it wrote. */
export async function createListings(o: CreateOptions): Promise<Manifest> {
  const fetchImpl = o.fetchImpl ?? fetch, log = o.log ?? (() => {});
  const rpc = rpcFor(o.node, fetchImpl);
  const chainId = Number(BigInt(await rpc<Hex>("eth_chainId")));
  assertBootstrapTarget(chainId, o.node);           // our own chains, this machine: the same refusal the deploy makes
  const d = JSON.parse(readFileSync(`${o.data}/deployment.json`, "utf8")) as Record<string, string>;
  const cat = o.catalogue ?? readCatalogue(o.repo);
  const plan = plannedListings(cat, { chainId, fixtureMembers: o.fixtureMembers ?? STAGING_FIXTURE_MEMBERS });
  const calls = listingCalls(plan);

  const call = async <T>(to: string, functionName: "count" | "listing", args: unknown[] = []): Promise<T> => {
    const data = encodeFunctionData({ abi: REGISTRY, functionName, args: args as never });
    return decodeFunctionResult({ abi: REGISTRY, functionName, data: await rpc<Hex>("eth_call", [{ to, data }, "latest"]) }) as T;
  };

  // what is already there, by kind+ref+name: the demo listings keep their ids, and a restart does not list anything twice
  const have = new Map<string, ManifestListing>();
  const startCount = Number(await call<bigint>(d.registry, "count"));
  for (let i = 0; i < startCount; i++) {
    const l = await call<{ kind: number; divisibility: number; vault: string; membershipRoot: string; name: string; ref: string; btcVault: string }>(d.registry, "listing", [BigInt(i)]);
    const kind = l.kind === 0 ? "inscriptions" as const : "rune" as const;
    have.set(`${kind}:${l.ref}:${l.name}`, { id: i, kind, name: l.name, ref: l.ref, vault: l.vault, btcVault: l.btcVault,
      divisibility: l.divisibility, membershipRoot: kind === "inscriptions" ? l.membershipRoot : null, floorSats: null, weight: 1,
      fromCatalogue: false, rootCovers: null, sourceUrl: null });
  }

  const listings: ManifestListing[] = [];
  const todo: { planned: PlannedListing; call: (typeof calls)[number] }[] = [];
  for (let i = 0; i < plan.listings.length; i++) {
    const p = plan.listings[i], key = `${p.kind}:${p.ref}:${p.name}`, existing = have.get(key);
    if (existing) listings.push({ ...existing, floorSats: p.floorSats, weight: p.weight, fromCatalogue: true, rootCovers: p.rootCovers, sourceUrl: p.sourceUrl });
    else todo.push({ planned: p, call: calls[i] });
  }

  const startedAt = new Date().toISOString(), t0 = Date.now();
  const manifest: Manifest = { source: plan.source, fetchedAt: plan.fetchedAt, chainId, placeholderRoots: true, note: MANIFEST_NOTE,
    progress: { created: listings.length, total: plan.listings.length, done: todo.length === 0, startedAt, finishedAt: null, ms: null },
    listings, skipped: plan.skipped };
  const flush = () => writeFileSync(manifestPath(o.data), JSON.stringify(manifest, null, 2) + "\n");

  writePrices(o.data, plan);
  if (!todo.length) { manifest.progress.finishedAt = new Date().toISOString(); manifest.progress.ms = 0; flush(); log({ event: "listings-already-there", have: listings.length, skipped: plan.skipped.length }); return manifest; }
  flush();
  log({ event: "listings-start", create: todo.length, have: listings.length, of: plan.listings.length, skipped: plan.skipped.length,
    note: "every root is a placeholder and every Bitcoin address is a marker nobody can be paid at (docs/phase11.md)" });

  const size = o.batch ?? 8;
  let nonce = Number(BigInt(await rpc<Hex>("eth_getTransactionCount", [d.owner, "pending"])));
  for (let at = 0; at < todo.length; at += size) {
    const slice = todo.slice(at, at + size), b0 = Date.now();
    // one nonce each, assigned here: the node would otherwise hand the same one to every request of a batch sent together
    const hashes = await Promise.all(slice.map((t) => rpc<Hex>("eth_sendTransaction", [{ from: d.owner, to: d.registry, data: t.call.calldata, nonce: `0x${(nonce++).toString(16)}` }])));
    for (let i = 0; i < slice.length; i++) {
      const receipt = await mined(rpc, hashes[i]);
      if (receipt.status !== "0x1") throw new Error(`listing ${slice[i].planned.ref} reverted`);
      const ev = receipt.logs.map((l) => { try { return decodeEventLog({ abi: LISTED, topics: l.topics, data: l.data }); } catch { return null; } }).find((x) => x?.eventName === "Listed");
      if (!ev) throw new Error(`listing ${slice[i].planned.ref} emitted no Listed event`);
      const args = ev.args as unknown as { id: bigint; vault: string };
      const p = slice[i].planned;
      listings.push({ id: Number(args.id), kind: p.kind, name: p.name, ref: p.ref, vault: args.vault, btcVault: p.btcVault,
        divisibility: p.divisibility, membershipRoot: p.membershipRoot, floorSats: p.floorSats, weight: p.weight,
        fromCatalogue: true, rootCovers: p.rootCovers, sourceUrl: p.sourceUrl });
    }
    manifest.progress.created = listings.length;
    manifest.progress.done = listings.length >= plan.listings.length;
    flush();
    log({ event: "listings-batch", created: listings.length, of: plan.listings.length, ms: Date.now() - b0 });
  }
  manifest.progress.finishedAt = new Date().toISOString();
  manifest.progress.ms = Date.now() - t0;
  manifest.listings.sort((a, b) => a.id - b.id);
  flush();
  log({ event: "listings-done", created: listings.length, skipped: plan.skipped.length, ms: manifest.progress.ms });
  return manifest;
}

/**
 * The floors the keeper posts caps from: the catalogue's, per slug. Anything already there is kept, so the demo listings the
 * deploy made (demo-stones, and the DEMO•RUNE the rune flow is demonstrated with) keep the fixture prices they had. No REAL
 * rune is given one: the manifest is what the operator reads for those, by listing id, and it says the floor is null.
 */
export function writePrices(data: string, plan: ListingPlan): void {
  const path = `${data}/prices.json`;
  const now = pricesFor(plan);
  let old: { inscriptions?: Record<string, string>; runes?: Record<string, string> } = {};
  try { old = JSON.parse(readFileSync(path, "utf8")); } catch { old = { inscriptions: { "demo-stones": "150000" }, runes: { "840000:1": "12" } }; }
  writeFileSync(path, JSON.stringify({ inscriptions: { ...old.inscriptions, ...now.inscriptions }, runes: { ...old.runes }, note: now.note }, null, 2) + "\n");
}

type Receipt = { status: Hex; logs: { topics: [Hex, ...Hex[]]; data: Hex }[] };
async function mined(rpc: Rpc, hash: Hex, tries = 600): Promise<Receipt> {
  for (let i = 0; i < tries; i++) {
    const r = await rpc<Receipt | null>("eth_getTransactionReceipt", [hash]);
    if (r) return r;
    await new Promise((ok) => setTimeout(ok, 50));
  }
  throw new Error(`no receipt for ${hash} after ${tries} tries`);
}
