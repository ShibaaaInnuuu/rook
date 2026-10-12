/**
 * The checked view of src/config/solana.ts. The day-1 addresses are all set or all empty (the placeholder); anything in between, a
 * slug out of `listing_order`, or a string that is not an address is a problem, and a config with problems builds no transaction.
 * Pure: test/sol.test.ts is its spec.
 */
import { SOLANA_SITE_CONFIG } from "../../config/solana";
import { LISTED_COLLECTIONS as COLLECTIONS, type SolCollection } from "./collections";
import { isAddress, type Address } from "./keys";

export type RawSiteConfig = {
  cluster: string; rpcUrl: string; generatedFrom: string | null;
  collections: { slug: string; listingId: number; listing: string; claimer: string; config: string }[];
  rook: { config: string; mint: string; pool: string }; cloudinary: { cloudName: string; uploadPreset: string };
};
export type SolanaConfig = {
  /** true once the day-1 addresses are filled in and valid: every one, or ROOK's alone (the canister names the collections' configs) */
  ready: boolean;
  problems: string[];
  cluster: string; rpcUrl: string;
  collections: (SolCollection & { config: Address | null; listing: Address | null; claimer: Address | null })[];
  rook: { config: Address; mint: Address; pool: Address } | null;
  cloudinary: { cloudName: string; uploadPreset: string } | null;
  generatedFrom: string | null;
};

/** Keyless, answers browsers; refuses getProgramAccounts without a token (src/config/solana.ts says what production needs). */
export const PUBLIC_RPC = "https://solana-rpc.publicnode.com";

export function loadSolanaConfig(raw: RawSiteConfig = SOLANA_SITE_CONFIG as RawSiteConfig, rpcOverride?: string | null): SolanaConfig {
  const problems: string[] = [];
  const addr = (v: unknown, what: string): Address | null => {
    if (v === "" || v === null || v === undefined) return null;
    if (!isAddress(v)) { problems.push(`${what} is not a Solana address: ${String(v)}`); return null; }
    return v;
  };
  const rows = Array.isArray(raw.collections) ? raw.collections : [];
  if (rows.length !== COLLECTIONS.length) problems.push(`the config names ${rows.length} collections; Rook has ${COLLECTIONS.length}`);
  const collections = COLLECTIONS.map((c, i) => {
    const r = rows[i] as Partial<RawSiteConfig["collections"][number]> | undefined;
    if (r && (r.slug !== c.slug || r.listingId !== c.listingId)) problems.push(`collection ${i} is ${String(r.slug)} #${String(r.listingId)}; listing_order says ${c.slug} #${c.listingId}`);
    return { ...c, config: addr(r?.config, `${c.slug}.config`), listing: addr(r?.listing, `${c.slug}.listing`), claimer: addr(r?.claimer, `${c.slug}.claimer`) };
  });
  const rookCfg = addr(raw.rook?.config, "rook.config"), rookMint = addr(raw.rook?.mint, "rook.mint"), rookPool = addr(raw.rook?.pool, "rook.pool");
  // Two complete shapes: every day-1 address (a day1-plan.json, written by ops/solana/site-config.mjs), or ROOK's three alone with
  // every collection config empty: canister-only Rook, where the canister names a collection's config when it opens it (the board adds
  // its Open collections, src/rook/sol/board.ts boardRefs) and ROOK is the operator's, launched at the ceremony
  // (docs/v9/ceremony-canister-only.md §4). Anything in between is half-written.
  const day1 = [...collections.map((c) => c.config), rookCfg, rookMint, rookPool];
  const filled = day1.filter(Boolean).length;
  const rookOnly = !!(rookCfg && rookMint && rookPool) && collections.every((c) => !c.config);
  const complete = filled === day1.length || rookOnly;
  if (filled && !complete) problems.push(`${filled} of ${day1.length} day-1 addresses are set: the config is half-written (all of them, or ROOK's three alone; run ops/solana/site-config.mjs again)`);
  const configs = collections.map((c) => c.config).filter(Boolean);
  if (new Set(configs).size !== configs.length) problems.push("two collections name the same DBC config");
  const rpc = (rpcOverride ?? "").trim() || raw.rpcUrl || PUBLIC_RPC;
  if (!/^https:\/\/|^http:\/\/(127\.0\.0\.1|localhost)(:\d+)?\//.test(rpc.endsWith("/") ? rpc : rpc + "/")) problems.push(`rpcUrl must be https: ${rpc}`);
  const cl = raw.cloudinary;
  const cloudinary = cl && cl.cloudName && cl.uploadPreset ? { cloudName: cl.cloudName, uploadPreset: cl.uploadPreset } : null;
  if (cloudinary && !/^[a-z0-9_-]+$/i.test(cloudinary.cloudName + cloudinary.uploadPreset)) problems.push("cloudinary names may hold letters, digits, _ and - only");
  return {
    ready: filled > 0 && complete && problems.length === 0, problems,
    cluster: raw.cluster, rpcUrl: rpc,
    collections, rook: rookCfg && rookMint && rookPool ? { config: rookCfg, mint: rookMint, pool: rookPool } : null,
    cloudinary, generatedFrom: raw.generatedFrom ?? null,
  };
}

const viteEnv = (import.meta as unknown as { env?: Record<string, string | undefined> }).env ?? {};
/** The site's config, once, with the build's VITE_SOLANA_RPC (if any) in place of the file's RPC. */
export const SOL: SolanaConfig = loadSolanaConfig(SOLANA_SITE_CONFIG as RawSiteConfig, viteEnv.VITE_SOLANA_RPC);
export const collectionOfConfig = (cfg: SolanaConfig, config: Address) => cfg.collections.find((c) => c.config === config) ?? null;
