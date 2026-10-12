#!/usr/bin/env node
// docs/v9/robinhood-everything.md §3 (2026-10-12): measure the twelve ERC-721s of Robinhood Chain (4663) that Nosh's CollectionRegistry
// lists (docs/v4/nft.md §1) against Rook's listing criteria (docs/v9/ordinals-collections.md §2), on OpenSea (the only 4663 venue whose
// protocol has code on chain: Seaport 1.6), and print the launch-file entries a passing collection would add.
//
//   ! OPENSEA_API_KEY=… node scripts/measure-robinhood-nfts.mjs            (a table, then the entries as JSON)
//   options (env): SOL_PER_ETH (else Coinbase's ETH→SOL rate), OPENSEA_CHAIN (OpenSea's name for Robinhood Chain; default "robinhood",
//   NOT verified offline: the script stops with a clear error when OpenSea does not know it), SALE_PAGES (default 10 pages of 50 events)
//
// The criteria, every one required:
//   - liquid: at least 30 single-item sales in 30 days (OpenSea's sale events, quantity 1);
//   - buyable by the canister: at least 3 single-item Seaport 1.6 listings from at least 3 sellers (the canister's floor needs 3);
//   - a floor inside the purchase bounds: 1 SOL to 10 SOL an item at the current SOL/ETH rate. A TOKEN-BOUND collection (StonkBrokers:
//     each NFT is an ERC-6551 account holding tokenized stocks; the operator included it 2026-10-12) has its own purchase cap instead,
//     floor × 1.25 within the canister's 25 ETH ceiling, and its own tier.
// The entries: graduation-tiers.json (tier: the smallest of 85/170/255/510 SOL, or the next multiple of 85, whose buying power per
// graduation, tier × 0.5 × 0.75 / 1.03, covers 1.33 × the floor; 85 for every floor of 1–10 SOL) and nft-pilot.json (chainId 4663, the
// address, the floor in SOL, T_wX = max(3, ⌈tier / floor⌉), maxPurchaseWei, the OpenSea slug, tokenBound). Read-only: GETs only.

export const CHAIN_ID = 4663;
export const OPENSEA_API = "https://api.opensea.io";
export const SEAPORT_1_6 = "0x0000000000000068f116a894984e2db1123eb395";
const E = 10n ** 18n;

/** docs/v4/nft.md §1: Nosh's CollectionRegistry, the 12 real ERC-721s on 4663 (read 2026-09-23). */
export const COLLECTIONS = [
  { name: "Hashcats", address: "0xca75df55cc9c476db27a7375d1fc8e794cf80721" },
  { name: "StonkBrokers", address: "0x539cdd042c2f3d93ebc5be7dfff0c79f3b4fabf0", tokenBound: true,
    note: "TOKEN-BOUND: each NFT is an ERC-6551 account holding tokenized stocks; bought and priced as the NFT only (its seller can drain the account before a sale); custody then holds tokenized stocks (a legal question the operator has not resolved)" },
  { name: "Chain Mancers", address: "0x797a2e030b7e49107c8f07bf0300ea9cae88ca57" },
  { name: "CASHCAT", address: "0xe3b34c4bb0f12c82143745eee6a6cf4e3154b1fa" },
  { name: "Rare Friends Genesis", address: "0x116eaa62241751e0c98da43d458600c6c17cd361" },
  { name: "RH MACHINES", address: "0x8c71d170fbd94bcba93bb08fc2cfd0e8620cd9ce" },
  { name: "Opencatz AI", address: "0xf0474980e09c3023655a9ca3e71a763358214efb" },
  // read on chain 2026-10-12: an ERC-404 (DN404-style) pair, the NFT driven by the ERC-20 QUOTRONS 0x5a86828efd322bfb16d93cfed16ee9bc14940d7f
  // (18 decimals, 1,077 tokens): token transfers mint and burn the NFTs, so a held ID can vanish. Left out (operator, 2026-10-12).
  { name: "QUOTRONS", address: "0x027aca2794e44f24950d81227dcd516ffbb49d6e", excluded: "ERC-404 (DN404 pair with the ERC-20 0x5a86…7d7f): token transfers mint and burn the NFTs; left out by the operator" },
  { name: "The Royal Mechanica", address: "0x99374499930204e1a7f76299e5e8c71fcd0e3aac" },
  { name: "AGNTS", address: "0x57efd86cf3ab1812cb47f0fd3c3d5bf0efb1f2d5" },
  { name: "Rekt Tradooors", address: "0x7b3ecfa33657de415ff269dc97dfa82954cee706" },
  { name: "Ape Hood", address: "0x47484196a99df52bf8fe53fb664c54821c13219d" },
];

export const MIN_SALES_30D = 30;
export const MIN_LISTINGS = 3;
export const MIN_SELLERS = 3;
export const MIN_FLOOR_SOL = 1;
export const MAX_FLOOR_SOL = 10;
export const TIERS_SOL = [85, 170, 255, 510];
/** The canister's per-item ceiling (`floortrigger::PURCHASE_LIMIT_CEILING_WEI`) and the token-bound collection's own cap over its floor. */
export const PURCHASE_CEILING_WEI = 25n * E;
export const TOKEN_BOUND_CAP_BPS = 12_500n;
/** A normal collection's purchase cap: 1.5 × its floor (the pilot's 2 ETH for a 0.82 ETH Milady is ≈ 2.4 ×), at least 0.05 ETH. */
export const CAP_BPS = 15_000n;
export const MIN_PURCHASE_WEI = 5n * 10n ** 16n;

/** The graduation tier (graduation-tiers.json `rule`): the smallest tier whose buying power covers 1.33 × the floor. */
export function tierOf(floorSol) {
  const covers = (t) => t * 0.5 * 0.75 / 1.03 >= 1.33 * floorSol;
  const t = TIERS_SOL.find(covers);
  if (t !== undefined) return t;
  let x = TIERS_SOL[TIERS_SOL.length - 1];
  while (!covers(x)) x += 85;
  return x;
}

/** T_wX = max(3, ⌈tier / floor⌉): the wX a coin's graduation needs (whole units). */
export function twxOf(tier, floorSol) {
  return Math.max(3, Math.ceil(tier / floorSol));
}

const weiToEth = (w) => Number(w) / 1e18;
const roundUp = (w, step) => ((w + step - 1n) / step) * step;

/** One collection's measurement from OpenSea's answers (pure). `sales`: sale events of the last 30 days; `listings`: the best listings. */
export function judge(c, { slug, sales, listings, statsFloorEth }, solPerEth) {
  const single = sales.filter((e) => Number(e.quantity ?? 1) === 1 && (e.nft || e.asset)).length;
  const ok = listings.filter((l) => (l.protocol_address ?? "").toLowerCase() === SEAPORT_1_6 && (l.protocol_data?.parameters?.offer ?? []).length === 1
    && l.price?.current?.value);
  const prices = ok.map((l) => BigInt(l.price.current.value)).sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  const sellers = new Set(ok.map((l) => String(l.protocol_data.parameters.offerer).toLowerCase())).size;
  const bookFloorWei = prices[0] ?? null;
  const floorEth = bookFloorWei !== null ? weiToEth(bookFloorWei) : statsFloorEth ?? null;
  const floorSol = floorEth !== null ? floorEth * solPerEth : null;
  const why = [];
  if (single < MIN_SALES_30D) why.push(`${single} single-item sales in 30 days (< ${MIN_SALES_30D})`);
  if (ok.length < MIN_LISTINGS || sellers < MIN_SELLERS) why.push(`${ok.length} Seaport 1.6 listings from ${sellers} sellers (< ${MIN_LISTINGS} from ${MIN_SELLERS})`);
  let maxPurchaseWei = null;
  if (c.excluded) why.push(`excluded: ${c.excluded}`);
  if (floorEth === null) why.push("no floor");
  else if (c.tokenBound) {
    maxPurchaseWei = roundUp((BigInt(Math.round(floorEth * 1e6)) * 10n ** 12n * TOKEN_BOUND_CAP_BPS) / 10_000n, 10n ** 16n);
    if (maxPurchaseWei > PURCHASE_CEILING_WEI) why.push(`its cap ${weiToEth(maxPurchaseWei)} ETH (1.25 × the floor) is over the canister's 25 ETH ceiling`);
  } else {
    if (floorSol < MIN_FLOOR_SOL || floorSol > MAX_FLOOR_SOL) why.push(`floor ${floorSol.toFixed(2)} SOL outside [${MIN_FLOOR_SOL}, ${MAX_FLOOR_SOL}] SOL`);
    const w = roundUp((BigInt(Math.round(floorEth * 1e6)) * 10n ** 12n * CAP_BPS) / 10_000n, 10n ** 16n);
    maxPurchaseWei = w < MIN_PURCHASE_WEI ? MIN_PURCHASE_WEI : w;
  }
  const verdict = why.length ? `out: ${why.join("; ")}` : c.tokenBound ? "in (TOKEN-BOUND: its own cap and tier)" : "in";
  const tier = floorSol !== null ? tierOf(floorSol) : null;
  return { name: c.name, address: c.address, slug, tokenBound: !!c.tokenBound, sales30d: single, listings: ok.length, sellers,
    lowest3Eth: prices.slice(0, 3).map(weiToEth), floorEth, floorSol, tier, twx: tier !== null && floorSol ? twxOf(tier, floorSol) : null,
    maxPurchaseWei, pass: why.length === 0, verdict };
}

/** The launch-file entries a passing collection adds (graduation-tiers.json `collections.<key>`, nft-pilot.json `collections[]`). */
export function entries(m, solPerEth, takenAt) {
  const key = m.slug ?? m.name.toLowerCase().replace(/[^a-z0-9]+/g, "-");
  return {
    graduationTiers: { [key]: { tier: m.tier, floor: `${m.floorEth} ETH`, floor_sol: m.floorSol.toFixed(2),
      nfts_per_graduation: ((m.tier * 0.5 * 0.75 / 1.03) / m.floorSol).toFixed(2), source: `opensea ${m.slug} (Robinhood Chain), ${takenAt}, ${solPerEth} SOL/ETH` } },
    nftPilot: { slug: key, name: `Wrapped ${m.name}`, symbol: `w${m.name.replace(/[^A-Za-z0-9]/g, "").toUpperCase().slice(0, 9)}` /* the canister takes at most 10 bytes */, chainId: CHAIN_ID,
      collection: m.address, floorSol: Number(m.floorSol.toFixed(4)), tier: m.tier, twx: m.twx, maxPurchaseWei: m.maxPurchaseWei.toString(),
      custodyItems: 3, custodyWei: (m.maxPurchaseWei * 3n).toString(), openseaSlug: m.slug, tokenBound: m.tokenBound },
  };
}

// ---- the reads (GETs only) ---------------------------------------------------------------------------------------------------------

async function get(f, url, key) {
  const r = await f(url, { headers: { accept: "application/json", ...(key ? { "x-api-key": key } : {}) }, redirect: "manual", signal: AbortSignal.timeout(20_000) });
  if (!r.ok) throw new Error(`${new URL(url).pathname}: HTTP ${r.status}`);
  return r.json();
}

/** SOL per ETH: the override, else Coinbase's ETH rates. */
export async function solPerEthOf(f, override) {
  if (override) return Number(override);
  const j = await get(f, "https://api.coinbase.com/v2/exchange-rates?currency=ETH");
  const x = Number(j?.data?.rates?.SOL);
  if (!(x > 0)) throw new Error("Coinbase gave no ETH→SOL rate (set SOL_PER_ETH)");
  return x;
}

/** One collection's reads: its OpenSea slug by contract, its stats, its best listings, its sale events of the last 30 days. */
export async function read(f, c, { key, chain = "robinhood", nowS, salePages = 10 }) {
  const contract = await get(f, `${OPENSEA_API}/api/v2/chain/${chain}/contract/${c.address}`, key);
  const slug = contract?.collection;
  if (!slug) throw new Error(`OpenSea names no collection for ${c.address} on chain "${chain}"`);
  const stats = await get(f, `${OPENSEA_API}/api/v2/collections/${slug}/stats`, key).catch(() => null);
  const listings = (await get(f, `${OPENSEA_API}/api/v2/listings/collection/${slug}/best?limit=100`, key))?.listings ?? [];
  const after = nowS - 30 * 86_400;
  const sales = [];
  let next = null;
  for (let i = 0; i < salePages; i++) {
    const j = await get(f, `${OPENSEA_API}/api/v2/events/collection/${slug}?event_type=sale&after=${after}&limit=50${next ? `&next=${encodeURIComponent(next)}` : ""}`, key);
    sales.push(...(j?.asset_events ?? []).filter((e) => Number(e.event_timestamp ?? nowS) >= after));
    next = j?.next ?? null;
    if (!next) break;
  }
  const statsFloorEth = Number(stats?.total?.floor_price) || null;
  return { slug, sales, listings, statsFloorEth };
}

export async function measureAll(f, env, nowS = Math.floor(Date.now() / 1000)) {
  const key = env.OPENSEA_API_KEY;
  if (!key) throw new Error("OPENSEA_API_KEY is not set");
  const solPerEth = await solPerEthOf(f, env.SOL_PER_ETH);
  const chain = env.OPENSEA_CHAIN || "robinhood";
  const out = [];
  for (const c of COLLECTIONS) {
    try { out.push(judge(c, await read(f, c, { key, chain, nowS, salePages: Number(env.SALE_PAGES || 10) }), solPerEth)); }
    catch (e) { out.push({ name: c.name, address: c.address, tokenBound: !!c.tokenBound, pass: false, verdict: `unread: ${e.message}` }); }
  }
  return { solPerEth, chain, takenAt: new Date(nowS * 1000).toISOString().slice(0, 16) + "Z", rows: out };
}

export function table(r) {
  const f = (x, d = 2) => (x === null || x === undefined ? "–" : typeof x === "number" ? x.toFixed(d) : String(x));
  const lines = [`Robinhood Chain NFTs on OpenSea ("${r.chain}"), ${r.takenAt}, ${r.solPerEth} SOL/ETH`, "",
    "| collection | slug | sales 30 d (single) | listings (sellers) | lowest 3 (ETH) | floor ETH | floor SOL | tier | T_wX | verdict |",
    "|---|---|---:|---|---|---:|---:|---:|---:|---|"];
  for (const m of r.rows) {
    lines.push(`| ${m.name}${m.tokenBound ? " (token-bound)" : ""} | ${m.slug ?? "–"} | ${m.sales30d ?? "–"} | ${m.listings ?? "–"} (${m.sellers ?? "–"}) | ${(m.lowest3Eth ?? []).map((x) => f(x, 4)).join(" ") || "–"} | ${f(m.floorEth, 4)} | ${f(m.floorSol)} | ${m.tier ?? "–"} | ${m.twx ?? "–"} | ${m.verdict} |`);
  }
  return lines.join("\n");
}

async function main() {
  const r = await measureAll(fetch, process.env);
  console.log(table(r));
  const pass = r.rows.filter((m) => m.pass);
  const add = pass.map((m) => entries(m, r.solPerEth, r.takenAt));
  console.log("\ngraduation-tiers.json collections (to add):");
  console.log(JSON.stringify(Object.assign({}, ...add.map((a) => a.graduationTiers)), null, 2));
  console.log("\nnft-pilot.json collections (to add; chainId 4663, the canister's eth_nft collections name chain_id 4663 and token_bound):");
  console.log(JSON.stringify(add.map((a) => a.nftPilot), null, 2));
  for (const m of r.rows.filter((x) => x.tokenBound)) console.log(`\nnote: ${m.name}: ${COLLECTIONS.find((c) => c.address === m.address).note}`);
}

if (import.meta.url === `file://${process.argv[1]}`) main().catch((e) => { console.error(e.message); process.exit(1); });
