#!/usr/bin/env node
// Writes the site's Solana config (web-rook/src/config/solana.ts) from the launch's day1-plan.json (docs/v9/launch-day1-solana.md
// §3): every collection's DBC config (one per listing: 26 since docs/v9/ordinals-collections.md; canister-only Rook, docs/v9/canister-only-solana.md
// §3: quoted in the collection's wX, graduating at its threshold in whole wX) (by slug, in listing_order), their listing / claimer
// addresses, and the genesis ROOK coin's config, mint and pool (no program id: canister-only Rook deploys none; the raffle is the
// canister's).
// READ-ONLY on chain: with --check-rpc it reads each config (owner = DBC, 1,048 bytes, fee_claimer = leftover_receiver = the plan's
// claimer; ROOK's = the plan's `genesis_rook.fee_claimer`, the custodian's wallet, when the plan names it; the threshold = the
// collection's T_wX from deployments/launch/graduation-tiers.json, 85 SOL for ROOK) and the ROOK pool, and refuses on any mismatch.
// It never signs or sends anything, and the plan holds no secret.
//
//   node ops/solana/site-config.mjs --plan ~/rook-keys/day1/day1-plan.json [--check-rpc https://api.mainnet-beta.solana.com]
//        [--out web-rook/src/config/solana.ts] [--allow-cluster devnet] [--dry-run]
//
// The site keeps rpcUrl and cloudinary from the file already there. Review the diff, run `npm test` in web-rook/, rebuild.
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
/** The SOL tiers a threshold derives from: launch-params.mjs TIER_SOL (ops/test/launch_params.test.ts holds them equal; kept here so
 *  the site's tests need no launch-params import). */
export const TIER_SOL = [85, 170, 255, 510];
const LAMPORTS = 1_000_000_000n;
/** `T_wX = max(3, ⌈tier / floor⌉)` whole wX (docs/v9/canister-only-solana.md §3.2), in lamports as the canister's launch pins compute it. */
export function thresholdWx(tierSol, floorSol) {
  const floor = BigInt(Math.round(Number(floorSol) * 1e9)), tier = BigInt(tierSol) * LAMPORTS;
  const t = (tier + floor - 1n) / floor;
  return t < 3n ? 3 : Number(t);
}
const DBC = "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN";
const B58 = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/;

function args(argv) {
  const o = {};
  for (let i = 0; i < argv.length; i++) {
    const k = argv[i];
    if (!k.startsWith("--")) throw new Error(`unknown argument ${k}`);
    o[k.slice(2)] = argv[i + 1] && !argv[i + 1].startsWith("--") ? argv[++i] : true;
  }
  return o;
}

/** listing_order as the launch file derives it: nft-pilot.json's Ethereum collections in order, then Bitcoin Frogs, then `inscriptions[]`. */
export function listingOrder(pilot) {
  return [...pilot.collections.map((c) => c.slug), ...(pilot.bitcoin ? [pilot.bitcoin.slug] : []), ...(pilot.inscriptions ?? []).map((b) => b.slug)];
}

/** Pure: the site config from the plan, keeping the hand-kept fields (rpcUrl, cloudinary) of `prev`. Throws on any inconsistency. */
export function siteConfigFromPlan(plan, order, prev = {}, allowCluster = null) {
  const bad = (m) => { throw new Error(`refused: ${m}`); };
  const addr = (v, what) => (typeof v === "string" && B58.test(v) ? v : bad(`${what} is not an address: ${JSON.stringify(v)}`));
  if (plan.cluster !== "mainnet-beta" && plan.cluster !== allowCluster) bad(`the plan is for cluster ${plan.cluster} (pass --allow-cluster ${plan.cluster} for a rehearsal site)`);
  const ls = plan.listings;
  if (!Array.isArray(ls) || ls.length !== order.length) bad(`the plan has ${ls?.length} listings; listing_order has ${order.length}`);
  const collections = ls.map((l, i) => {
    if (l.listing_id !== i || l.slug !== order[i]) bad(`listing ${i} is ${l.slug} #${l.listing_id}; listing_order says ${order[i]} #${i}`);
    // a plan may still carry the listing's SOL tier (`graduation_sol`, the threshold's input); the site computes the threshold itself
    if (l.graduation_sol !== undefined && !TIER_SOL.includes(l.graduation_sol)) bad(`listing ${i} (${l.slug}) has graduation_sol ${JSON.stringify(l.graduation_sol)}; one of ${TIER_SOL.join(", ")}`);
    return { slug: l.slug, listingId: i, listing: addr(l.listing, `${l.slug}.listing`), claimer: addr(l.claimer, `${l.slug}.claimer`), config: addr(l.config, `${l.slug}.config`) };
  });
  const configs = collections.map((c) => c.config);
  if (new Set(configs).size !== configs.length) bad("two listings name the same config");
  const g = plan.genesis_rook ?? bad("the plan has no genesis_rook");
  if (g.fee_claimer !== undefined) addr(g.fee_claimer, "genesis_rook.fee_claimer");
  return {
    cluster: plan.cluster,
    rpcUrl: prev.rpcUrl ?? "https://solana-rpc.publicnode.com",
    generatedFrom: `day1-plan.json generated_at ${plan.generated_at} (${new Date(plan.generated_at * 1000).toISOString()})`,
    collections,
    rook: { config: addr(g.config, "genesis_rook.config"), mint: addr(g.mint, "genesis_rook.mint"), pool: addr(g.pool, "genesis_rook.pool") },
    cloudinary: prev.cloudinary ?? { cloudName: "", uploadPreset: "" },
  };
}

/** The object literal of an existing src/config/solana.ts (it is JSON between the export and the closing semicolon). */
export function readSiteConfig(text) {
  const m = text.match(/export const SOLANA_SITE_CONFIG = (\{[\s\S]*\});\s*$/);
  if (!m) throw new Error("src/config/solana.ts: SOLANA_SITE_CONFIG not found");
  return JSON.parse(m[1]);
}

/** The file, header kept: everything before the export is the existing comment block. */
export function renderSiteConfig(cfg, header) {
  const rows = cfg.collections.map((c) => `    ${JSON.stringify(c).replace(/":/g, "\": ").replace(/,"/g, ", \"").replace(/^\{/, "{ ").replace(/\}$/, " }")}`).join(",\n");
  const body = JSON.stringify({ ...cfg, collections: ["__ROWS__"] }, null, 2).replace(/\[\n\s*"__ROWS__"\n\s*\]/, `[\n${rows}\n  ]`);
  return `${header}export const SOLANA_SITE_CONFIG = ${body};\n`;
}

async function rpc(url, method, params) {
  const r = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }), signal: AbortSignal.timeout(20_000) });
  const j = await r.json();
  if (j.error) throw new Error(`${method}: ${JSON.stringify(j.error)}`);
  return j.result;
}
const B58A = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const b58 = (b) => { let n = 0n; for (const x of b) n = (n << 8n) | BigInt(x); let s = ""; while (n > 0n) { s = B58A[Number(n % 58n)] + s; n /= 58n; } for (const x of b) { if (x) break; s = "1" + s; } return s; };

/** Every collection's threshold in whole wX from deployments/launch/graduation-tiers.json (its tier and day-1 floor). */
export function thresholdsFromTiers(tiers) {
  return Object.fromEntries(Object.entries(tiers.collections).map(([slug, c]) => [slug, thresholdWx(c.tier, c.floor_sol)]));
}

async function checkOnChain(url, cfg, thresholds, rookClaimer = null) {
  const keys = [...cfg.collections.map((c) => c.config), cfg.rook.config, cfg.rook.pool];
  const r = await rpc(url, "getMultipleAccounts", [keys, { encoding: "base64", commitment: "confirmed" }]);
  const problems = [];
  r.value.forEach((v, i) => {
    const what = i < cfg.collections.length ? `${cfg.collections[i].slug} config ${keys[i]}` : i === cfg.collections.length ? `ROOK config ${keys[i]}` : `ROOK pool ${keys[i]}`;
    if (!v) return problems.push(`${what}: does not exist`);
    if (v.owner !== DBC) return problems.push(`${what}: owner ${v.owner}, not DBC`);
    const d = Buffer.from(v.data[0], "base64");
    if (i <= cfg.collections.length) {
      const want = i < cfg.collections.length ? cfg.collections[i].claimer : rookClaimer;
      if (d.length !== 1048) return problems.push(`${what}: ${d.length} bytes, not a PoolConfig`);
      if (want && (b58(d.subarray(40, 72)) !== want || b58(d.subarray(72, 104)) !== want)) problems.push(`${what}: fee claimer / leftover receiver is not ${want}`);
      // migration_quote_threshold (u64 at 264): the collection's threshold in whole wX (9 decimals, as SOL), 85 SOL for ROOK
      const slug = i < cfg.collections.length ? cfg.collections[i].slug : null;
      if (slug && thresholds[slug] === undefined) return problems.push(`${what}: ${slug} is not in graduation-tiers.json`);
      const th = d.readBigUInt64LE(264), wantTh = BigInt(slug ? thresholds[slug] : 85) * 1_000_000_000n;
      if (th !== wantTh) problems.push(`${what}: threshold ${th} base units, not ${wantTh}`);
    } else {
      if (d.length !== 424) return problems.push(`${what}: ${d.length} bytes, not a VirtualPool`);
      if (b58(d.subarray(72, 104)) !== cfg.rook.config) problems.push(`${what}: its config is not the ROOK config`);
      if (b58(d.subarray(136, 168)) !== cfg.rook.mint) problems.push(`${what}: its base mint is not the ROOK mint`);
    }
  });
  return problems;
}

async function main() {
  const a = args(process.argv.slice(2));
  if (!a.plan) throw new Error("--plan <day1-plan.json> is required");
  const out = a.out ?? join(ROOT, "web-rook/src/config/solana.ts");
  const plan = JSON.parse(readFileSync(a.plan, "utf8"));
  const order = listingOrder(JSON.parse(readFileSync(join(ROOT, "deployments/launch/nft-pilot.json"), "utf8")));
  const text = existsSync(out) ? readFileSync(out, "utf8") : "";
  const prev = text ? readSiteConfig(text) : {};
  const header = text ? text.slice(0, text.indexOf("export const SOLANA_SITE_CONFIG")) : "// The site's Solana configuration. WRITTEN BY ops/solana/site-config.mjs.\n";
  const cfg = siteConfigFromPlan(plan, order, prev, a["allow-cluster"] ?? null);
  if (a["check-rpc"]) {
    const thresholds = thresholdsFromTiers(JSON.parse(readFileSync(join(ROOT, "deployments/launch/graduation-tiers.json"), "utf8")));
    const problems = await checkOnChain(a["check-rpc"], cfg, thresholds, plan.genesis_rook?.fee_claimer ?? null);
    if (problems.length) throw new Error(`refused: on chain:\n  ${problems.join("\n  ")}`);
    console.log(`checked on chain (${a["check-rpc"]}): ${cfg.collections.length} configs pay their claimers at their wX thresholds, the ROOK config is at 85 SOL${plan.genesis_rook?.fee_claimer ? " and pays the custodian's wallet" : ""}, the ROOK pool is on it`);
  } else console.log("not checked on chain (pass --check-rpc <url> to read the configs and the ROOK pool)");
  const body = renderSiteConfig(cfg, header);
  if (a["dry-run"]) { process.stdout.write(body); return; }
  writeFileSync(out, body);
  console.log(`wrote ${out}: ${cfg.collections.length} collection configs, ROOK ${cfg.rook.mint}; review the diff, run npm test in web-rook/, rebuild`);
}

if (import.meta.url === `file://${process.argv[1]}`) main().catch((e) => { console.error(String(e.message ?? e)); process.exit(2); });
