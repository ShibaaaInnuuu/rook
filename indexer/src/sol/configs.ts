/**
 * Which DBC configs are Rook's. In order of precedence:
 *   1. ROOK_CONFIGS (env): `slug=Config,slug=Config` or a JSON array of {slug, config, listingId?}. For devnet, tests, the dry run.
 *   2. The site's config, web-rook/src/config/solana.ts (SITE_CONFIG_PATH): the same file the site builds from, so the two never disagree.
 *   3. The ceremony's day1-plan.json (DAY1_PLAN_PATH).
 * Before day 1 the site file holds empty strings: the list is empty and the service idles, saying so in /v1/health.
 */
import { existsSync, readFileSync } from "node:fs";
import { isAddress } from "./bytes.js";

export type RookConfig = { slug: string; listingId: number | null; config: string };
export type ConfigList = { configs: RookConfig[]; source: string; problems: string[] };

export function parseEnvConfigs(v: string): RookConfig[] {
  const s = v.trim();
  if (!s) return [];
  if (s.startsWith("[")) {
    const arr = JSON.parse(s) as { slug?: string; config?: string; listingId?: number | null }[];
    return arr.map((x, i) => ({ slug: String(x.slug ?? `config-${i}`), listingId: typeof x.listingId === "number" ? x.listingId : null, config: String(x.config ?? "") }));
  }
  return s.split(",").map((p) => p.trim()).filter(Boolean).map((p, i) => {
    const [a, b] = p.includes("=") ? p.split("=", 2) : [`config-${i}`, p];
    return { slug: a.trim(), listingId: null, config: b.trim() };
  });
}

/** The JSON object literal in web-rook/src/config/solana.ts (`export const SOLANA_SITE_CONFIG = {...};`). */
export function parseSiteConfig(text: string): RookConfig[] {
  const start = text.indexOf("{", text.indexOf("SOLANA_SITE_CONFIG"));
  const end = text.lastIndexOf("}");
  if (start < 0 || end < start) throw new Error("no SOLANA_SITE_CONFIG object in the site config");
  const j = JSON.parse(text.slice(start, end + 1)) as { collections?: { slug: string; listingId: number; config: string }[]; rook?: { config?: string } };
  const out: RookConfig[] = (j.collections ?? []).map((c) => ({ slug: c.slug, listingId: c.listingId, config: c.config }));
  if (j.rook?.config !== undefined) out.push({ slug: "rook", listingId: null, config: j.rook.config });
  return out;
}

export function parseDay1Plan(text: string): RookConfig[] {
  const j = JSON.parse(text) as { listings?: { slug: string; listing_id: number; config: string }[]; genesis_rook?: { config?: string } | null };
  const out: RookConfig[] = (j.listings ?? []).map((l) => ({ slug: l.slug, listingId: l.listing_id, config: l.config }));
  if (j.genesis_rook?.config) out.push({ slug: "rook", listingId: null, config: j.genesis_rook.config });
  return out;
}

/** Keeps only valid, distinct addresses; says what it dropped. Empty strings (the pre-day-1 placeholder) are dropped silently. */
export function clean(list: RookConfig[]): { configs: RookConfig[]; problems: string[] } {
  const problems: string[] = [], seen = new Set<string>(), configs: RookConfig[] = [];
  for (const c of list) {
    if (!c.config) continue;
    if (!isAddress(c.config)) { problems.push(`${c.slug}: not a Solana address: ${c.config}`); continue; }
    if (seen.has(c.config)) { problems.push(`${c.slug}: config ${c.config} is listed twice`); continue; }
    seen.add(c.config); configs.push(c);
  }
  return { configs, problems };
}

export function loadConfigs(env: Record<string, string | undefined>, defaults: { sitePath: string }): ConfigList {
  if (env.ROOK_CONFIGS && env.ROOK_CONFIGS.trim()) return { ...clean(parseEnvConfigs(env.ROOK_CONFIGS)), source: "env ROOK_CONFIGS" };
  const site = env.SITE_CONFIG_PATH ?? defaults.sitePath;
  if (existsSync(site)) {
    try {
      const r = clean(parseSiteConfig(readFileSync(site, "utf8")));
      if (r.configs.length || !env.DAY1_PLAN_PATH) return { ...r, source: `site config ${site}${r.configs.length ? "" : " (placeholders: day 1 has not happened)"}` };
    } catch (err) { if (!env.DAY1_PLAN_PATH) return { configs: [], problems: [String((err as Error).message)], source: `site config ${site}` }; }
  }
  if (env.DAY1_PLAN_PATH && existsSync(env.DAY1_PLAN_PATH)) {
    try { return { ...clean(parseDay1Plan(readFileSync(env.DAY1_PLAN_PATH, "utf8"))), source: `day1 plan ${env.DAY1_PLAN_PATH}` }; }
    catch (err) { return { configs: [], problems: [String((err as Error).message)], source: `day1 plan ${env.DAY1_PLAN_PATH}` }; }
  }
  return { configs: [], problems: ["no config source: set ROOK_CONFIGS, SITE_CONFIG_PATH or DAY1_PLAN_PATH"], source: "none" };
}
