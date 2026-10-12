/**
 * The coins board, read straight from the RPC (no indexer): every DBC pool whose config is one of Rook's (getProgramAccounts on DBC,
 * filtered by size and by the pool's `config` field, one call per config, returning only each pool's launch time), newest first;
 * then the pools and their coins' Metaplex metadata, a page at a time (getMultipleAccounts, 100 per call).
 */
import { decodeMetadata, type TokenMeta } from "./accounts";
export { solUsd } from "./accounts";
export { fetchOffChain, gatewayUrl, parseOffChain, type OffChainMeta } from "./offchain";
import { decodeConfig, decodePool, DBC_PROGRAM, metadataAddress, poolAddress, poolStats, POOL_ACTIVATION_OFFSET, POOL_CONFIG_OFFSET, VIRTUAL_POOL_LEN, type DbcConfig, type DbcPool } from "./dbc";
import { u64At } from "./borsh";
import type { Rpc } from "./rpc";
import type { CollectionViewV } from "./canister";
import type { SolanaConfig } from "./siteConfig";
import { isAddress } from "./keys";

/** Which Rook config a coin is on: one of Rook's collections (COLLECTIONS), or the genesis ROOK coin's own. */
export type ConfigRef = { config: string; slug: string; label: string };
export type PoolRefRow = { pool: string; config: string; launchedAt: number };
export type CoinRow = PoolRefRow & {
  slug: string; label: string; mint: string; creator: string; meta: TokenMeta | null; pool_: DbcPool;
  stats: ReturnType<typeof poolStats>;
};

/**
 * The board's configs: the site config's (`staticRefs`, the collections in listing order then the genesis ROOK coin's) merged with the
 * canister's `coins_collections` (operator decision 2026-10-12: a collection opens on demand, and the canister names its DBC config only
 * then, so a coin launched on a collection opened after this build is on a config the site file does not hold). Every `Open` row with a
 * config address the static list does not already name is added after it, labelled by the site collection with the same wX symbol (or
 * by the symbol itself, for a collection this build does not list). Rows not yet open, or without a valid address, add nothing; no
 * canister (`cols` null or undefined) leaves the static list as it is. Each config is still read on chain and must be a DBC config
 * (`readConfigs`) before any pool under it is listed. Pure: test/boardrefs.test.ts is its spec.
 */
export function boardRefs(staticRefs: ConfigRef[], cols: CollectionViewV[] | null | undefined,
  collections: readonly { slug: string; name: string; wxSymbol: string }[]): ConfigRef[] {
  if (!cols?.length) return staticRefs;
  const seen = new Set(staticRefs.map((r) => r.config));
  const out = staticRefs.slice();
  for (const c of cols) {
    if (c.stage !== "Open" || !isAddress(c.config) || seen.has(c.config)) continue;
    seen.add(c.config);
    const site = collections.find((x) => x.wxSymbol === c.symbol);
    out.push(site ? { config: c.config, slug: site.slug, label: site.name } : { config: c.config, slug: c.symbol.toLowerCase(), label: c.symbol });
  }
  return out;
}

/** The genesis coin's board label (the terminal's "Genesis (ROOK)" filter is its slug). */
export const ROOK_REF = { slug: "rook", label: "Genesis (ROOK)" } as const;

/** The site config's own board configs: each collection config the file names (listing order; none in the canister-only shape, where
 *  the canister names them as it opens them), then the genesis ROOK coin's. Nothing until the config is ready. Pure. */
export function siteRefsOf(cfg: Pick<SolanaConfig, "ready" | "collections" | "rook">): ConfigRef[] {
  if (!cfg.ready) return [];
  return [...cfg.collections.flatMap((c) => (c.config ? [{ config: c.config, slug: c.slug, label: c.name }] : [])), ...(cfg.rook ? [{ config: cfg.rook.config, ...ROOK_REF }] : [])];
}

/** ROOK's board row from its two known accounts (pool and config, one getMultipleAccounts call): null unless the pool is a DBC pool on
 *  ROOK's config for ROOK's mint and the config decodes. Its metadata is not read (the home page's card names ROOK itself). */
export async function readRookRow(rpc: Rpc, rook: { config: string; mint: string; pool: string }): Promise<CoinRow | null> {
  const [pa, ca] = await rpc.accounts([rook.pool, rook.config]);
  if (!pa || !ca || pa.owner !== DBC_PROGRAM || ca.owner !== DBC_PROGRAM) return null;
  let p: DbcPool, cfg: DbcConfig;
  try { p = decodePool(pa.data); cfg = decodeConfig(ca.data); } catch { return null; }
  if (p.config !== rook.config || p.baseMint !== rook.mint) return null;
  return { pool: rook.pool, config: rook.config, launchedAt: Number(p.activationPoint), ...ROOK_REF, mint: rook.mint, creator: p.creator, meta: null, pool_: p, stats: poolStats(cfg, p) };
}

export async function readConfigs(rpc: Rpc, refs: ConfigRef[]): Promise<Map<string, DbcConfig>> {
  const accs = await rpc.accounts(refs.map((r) => r.config));
  const out = new Map<string, DbcConfig>();
  accs.forEach((a, i) => { if (a && a.owner === DBC_PROGRAM) { try { out.set(refs[i].config, decodeConfig(a.data)); } catch { /* not a config */ } } });
  return out;
}

/** Every pool on these configs, newest first (by the pool's activation point: its launch time on a timestamp config). */
export async function listPools(rpc: Rpc, configs: string[], concurrency = 2): Promise<PoolRefRow[]> {
  const out: PoolRefRow[] = [];
  const queue = [...configs];
  const worker = async () => {
    for (let c = queue.shift(); c; c = queue.shift()) {
      const rows = await rpc.programAccounts(DBC_PROGRAM, [{ dataSize: VIRTUAL_POOL_LEN }, { memcmp: { offset: POOL_CONFIG_OFFSET, bytes: c } }], { offset: POOL_ACTIVATION_OFFSET, length: 8 });
      for (const r of rows) out.push({ pool: r.pubkey, config: c, launchedAt: r.account.data.length >= 8 ? Number(u64At(r.account.data, 0)) : 0 });
    }
  };
  await Promise.all(Array.from({ length: Math.min(concurrency, configs.length) }, worker));
  return sortNewest(out);
}
export const sortNewest = <T extends { launchedAt: number; pool: string }>(rows: T[]): T[] => rows.slice().sort((a, b) => b.launchedAt - a.launchedAt || (a.pool < b.pool ? -1 : 1));

/** One page of rows: the pools and their metadata. */
export async function loadRows(rpc: Rpc, refs: PoolRefRow[], configs: Map<string, DbcConfig>, labels: Map<string, ConfigRef>): Promise<CoinRow[]> {
  if (!refs.length) return [];
  const pools = await rpc.accounts(refs.map((r) => r.pool));
  const decoded = refs.map((r, i) => { const a = pools[i]; try { return a && a.owner === DBC_PROGRAM ? decodePool(a.data) : null; } catch { return null; } });
  const metas = await rpc.accounts(decoded.map((p, i) => metadataAddress(p ? p.baseMint : refs[i].pool)));
  const rows: CoinRow[] = [];
  refs.forEach((r, i) => {
    const p = decoded[i], cfg = configs.get(r.config), l = labels.get(r.config);
    if (!p || !cfg || !l) return;
    let meta: TokenMeta | null = null;
    try { meta = metas[i] ? decodeMetadata(metas[i]!.data) : null; } catch { meta = null; }
    rows.push({ ...r, slug: l.slug, label: l.label, mint: p.baseMint, creator: p.creator, meta, pool_: p, stats: poolStats(cfg, p) });
  });
  return rows;
}

/** A coin's pool from its mint: the one Rook config under which ["pool", config, mint, wSOL] exists. */
export async function findCoin(rpc: Rpc, mint: string, refs: ConfigRef[]): Promise<{ ref: ConfigRef; pool: string; poolAcc: DbcPool } | null> {
  const cands = refs.map((r) => poolAddress(r.config, mint));
  const accs = await rpc.accounts(cands);
  for (let i = 0; i < refs.length; i++) {
    const a = accs[i];
    if (a && a.owner === DBC_PROGRAM) { try { const p = decodePool(a.data); if (p.baseMint === mint && p.config === refs[i].config) return { ref: refs[i], pool: cands[i], poolAcc: p }; } catch { /* not a pool */ } }
  }
  return null;
}

