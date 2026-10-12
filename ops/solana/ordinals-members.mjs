#!/usr/bin/env node
// The Inscriptions listings' membership: who is in each Ordinals collection, and the Merkle root rook_core's listing pins
// (ListingKind::Inscriptions { membership_root }). Read-only: public keyless APIs, nothing is signed or sent.
//
//   node ops/solana/ordinals-members.mjs fetch <ow-slug> <out.json> [--gallery <inscription id> | --parent <id>[,<id>…] |
//        --grandparent <id>] [--me <magic-eden legacy slug>] [--owgh <ordinals-wallet github slug>] [--min-sources 2]
//       Downloads the member list from Ordinals Wallet and the on-chain source, keeps the ids BOTH agree on (the intersection: an id one
//       source lacks is left out, never added: a member nobody else recognises is what a holder deposit could abuse), orders
//       them by inscription number (ascending, Ordinals Wallet's `num`, as bitcoin-frogs-members.json is) and writes the list
//       plus `<out>.provenance.json` (each source's URL, count, and the ids it alone had).
//   node ops/solana/ordinals-members.mjs root <members.json>
//       Prints the count and the root.
//   node ops/solana/ordinals-members.mjs check [deployments/launch/nft-pilot.json]
//       Every Inscriptions entry of the launch file (`bitcoin`, then `inscriptions[]`): membersFile's count and root equal
//       membersCount / membersRoot. Exit 1 otherwise.
//
// Sources (deployments/launch/btc/README.md):
//   ow       GET https://turbo.ordinalswallet.com/collection/<slug>/inscriptions      (the canister's buy venue; keyless)
//   gallery  https://ordinals.com/gallery/<id>[/page/<n>]                              (an on-chain ord gallery inscription)
//   parent   GET https://ordinals.com/r/children/<id>/<page>                          (on-chain parent/child provenance; several
//            parents: the union of their children)
//   grandparent  the children of every child of <id> (RSIC: one root inscription, ten sub-parents of 2,100 each)
//   me       https://raw.githubusercontent.com/TheWizardsOfOrd/ordinals-collections/main/legacy/collections/<slug>.json
//            (Magic Eden's collection export, kept by The Wizards of Ord's curated registry; its collections.json names the
//            gallery / parent inscription each collection is recognised by). A REFERENCE: compared and recorded, not agreed on
//   owgh     https://raw.githubusercontent.com/ordinals-wallet/ordinals-collections/main/collections/<slug>/inscriptions.json
//            (Ordinals Wallet's public registry, last updated 2024-08). A REFERENCE, like me
//
// The tree is ops/src/frogmath.ts `proofFor`'s, script/Deploy.s.sol's and rook_core hashing.rs's: leaves in list order,
// leaf = keccak256(keccak256(id bytes)), pairs sorted (OpenZeppelin MerkleProof), an odd node moves up unchanged.
import { readFileSync, writeFileSync } from "node:fs";
import { keccak256, toHex, concat } from "viem";

export const leafOf = (id) => keccak256(keccak256(toHex(id)));
const pair = (a, b) => keccak256(BigInt(a) <= BigInt(b) ? concat([a, b]) : concat([b, a]));

/** The root of `members` (pure). */
export function rootOf(members) {
  if (!members.length) throw new Error("an empty member list has no root");
  let level = members.map(leafOf);
  while (level.length > 1) {
    const next = [];
    for (let k = 0; k < level.length; k += 2) next.push(k + 1 < level.length ? pair(level[k], level[k + 1]) : level[k]);
    level = next;
  }
  return level[0];
}

/** ceil(log2(n)): the longest proof in a tree of n leaves (mint_item carries it; ≈ 25 fit a Solana transaction). */
export const proofDepth = (n) => (n <= 1 ? 0 : Math.ceil(Math.log2(n)));

const ID = /^[0-9a-f]{64}i\d+$/;

/** Every Inscriptions entry of the launch file, in listing order (`bitcoin`, then `inscriptions[]`). */
export function inscriptionEntries(pilot) {
  return [...(pilot.bitcoin ? [pilot.bitcoin] : []), ...(pilot.inscriptions ?? [])];
}

/** The intersection of the sources' id sets, ordered by `order` (id → inscription number). Pure. */
export function agree(sources, order) {
  if (sources.length < 1) throw new Error("no source");
  const sets = sources.map((s) => new Set(s.ids));
  const both = [...sets[0]].filter((id) => sets.every((s) => s.has(id)));
  const missing = both.filter((id) => !order.has(id));
  if (missing.length) throw new Error(`${missing.length} agreed ids have no inscription number (e.g. ${missing[0]})`);
  both.sort((a, b) => order.get(a) - order.get(b));
  const only = sources.map((s, i) => ({ name: s.name, only: s.ids.filter((id) => !sets.every((t, j) => j === i || t.has(id))) }));
  return { members: both, only };
}

async function get(url, accept) {
  for (let t = 0; t < 6; t++) {
    try {
      const r = await fetch(url, { headers: { "user-agent": "rook-members/1 (read-only)", ...(accept ? { accept } : {}) }, signal: AbortSignal.timeout(120_000) });
      if (r.ok) return accept === "application/json" ? r.json() : r.text();
      if (r.status !== 429 && r.status < 500) throw new Error(`${url}: HTTP ${r.status}`);
    } catch (e) {
      if (t === 5) throw e;
    }
    await new Promise((s) => setTimeout(s, 2000 * (t + 1)));
  }
  throw new Error(`${url}: no answer`);
}

async function pages(fn, from = 0) {
  const out = [];
  for (let p = from; ; p++) {
    const got = await fn(p);
    if (!got.length) return out;
    out.push(...got);
  }
}

async function fetchGallery(id) {
  const re = new RegExp(`href=/gallery/${id}/(\\d+)><iframe[^>]*src=/preview/([0-9a-f]{64}i\\d+)>`, "g");
  const seen = new Map();
  for (let p = 0; ; p++) {
    const h = await get(p ? `https://ordinals.com/gallery/${id}/page/${p}` : `https://ordinals.com/gallery/${id}`);
    const rows = [...h.matchAll(re)];
    if (!rows.length) break;
    for (const [, i, x] of rows) seen.set(Number(i), x);
    if (!h.includes(`/gallery/${id}/page/${p + 1}>next`)) break;
  }
  return [...seen.entries()].sort((a, b) => a[0] - b[0]).map(([, x]) => x);
}

async function fetchChildren(id) {
  return pages(async (p) => {
    const j = await get(`https://ordinals.com/r/children/${id}/${p}`, "application/json");
    return j.ids?.length ? j.ids : [];
  });
}

function args(argv) {
  const o = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i].startsWith("--")) o[argv[i].slice(2)] = argv[++i];
    else o._.push(argv[i]);
  }
  return o;
}

async function main() {
  const a = args(process.argv.slice(2));
  const [cmd, x, y] = a._;
  if (cmd === "root") {
    const m = JSON.parse(readFileSync(x, "utf8"));
    console.log(JSON.stringify({ count: m.length, root: rootOf(m), proof_depth: proofDepth(m.length) }));
    return;
  }
  if (cmd === "check") {
    const path = x ?? new URL("../../deployments/launch/nft-pilot.json", import.meta.url);
    const pilot = JSON.parse(readFileSync(path, "utf8"));
    const repo = new URL("../../", import.meta.url);
    let bad = 0;
    for (const e of inscriptionEntries(pilot)) {
      const m = JSON.parse(readFileSync(new URL(e.membersFile, repo), "utf8"));
      const dup = m.length - new Set(m).size, badIds = m.filter((id) => !ID.test(id)).length;
      const root = rootOf(m), ok = m.length === e.membersCount && root === e.membersRoot && !dup && !badIds;
      if (!ok) bad++;
      console.log(`${ok ? "ok  " : "BAD "} ${e.slug}: ${m.length} members (file says ${e.membersCount}), root ${root}${root === e.membersRoot ? "" : ` ≠ ${e.membersRoot}`}${dup ? `, ${dup} duplicates` : ""}${badIds ? `, ${badIds} malformed ids` : ""}`);
    }
    process.exit(bad ? 1 : 0);
  }
  if (cmd !== "fetch" || !x || !y) throw new Error("usage: fetch <ow-slug> <out.json> [--gallery <id> | --parent <id>[,<id>…] | --grandparent <id>] [--me <slug>] | root <members.json> | check [nft-pilot.json]");
  const ow = await get(`https://turbo.ordinalswallet.com/collection/${x}/inscriptions`, "application/json");
  const order = new Map(ow.map((r) => [r.id, r.num]));
  const sources = [{ name: "ordinalswallet", url: `https://turbo.ordinalswallet.com/collection/${x}/inscriptions`, ids: ow.map((r) => r.id) }];
  if (a.gallery) sources.push({ name: "ord-gallery", url: `https://ordinals.com/gallery/${a.gallery}`, ids: await fetchGallery(a.gallery) });
  if (a.parent) {
    const ps = a.parent.split(",");
    let ids = [];
    for (const p of ps) ids = ids.concat(await fetchChildren(p)); // concat, not push(...): 112,384 Runestone ids overflow the stack
    sources.push({ name: "ord-children", url: ps.map((p) => `https://ordinals.com/r/children/${p}`).join(" "), ids });
  }
  if (a.grandparent) {
    const ps = await fetchChildren(a.grandparent);
    let ids = [];
    for (const p of ps) ids = ids.concat(await fetchChildren(p));
    sources.push({ name: "ord-grandchildren", url: `https://ordinals.com/r/children/${a.grandparent} → each child's /r/children (${ps.length} parents)`, ids });
  }
  /** Reference lists: compared and recorded, never part of the agreement (Magic Eden's legacy export is a 2024 snapshot). */
  const refs = [];
  if (a.me) {
    const url = `https://raw.githubusercontent.com/TheWizardsOfOrd/ordinals-collections/main/legacy/collections/${a.me}.json`;
    refs.push({ name: "magiceden-legacy", url, ids: (await get(url, "application/json")).map((r) => r.id) });
  }
  if (a.owgh) {
    const url = `https://raw.githubusercontent.com/ordinals-wallet/ordinals-collections/main/collections/${a.owgh}/inscriptions.json`;
    refs.push({ name: "ordinalswallet-github", url, ids: (await get(url, "application/json")).map((r) => r.id) });
  }
  const min = Number(a["min-sources"] ?? 2);
  if (sources.length < min) throw new Error(`refused: ${sources.length} source(s), ${min} needed`);
  for (const s of sources) {
    const bad = s.ids.filter((id) => !ID.test(id));
    if (bad.length) throw new Error(`${s.name}: ${bad.length} malformed ids (e.g. ${bad[0]})`);
  }
  const { members, only } = agree(sources, order);
  writeFileSync(y, JSON.stringify(members, null, 0).replace(/^\[/, "[\n").replace(/","/g, '",\n"').replace(/\]$/, "\n]") + "\n");
  const prov = {
    slug: x, taken_at: new Date().toISOString(), rule: "the ids every entry of sources has (intersection; references are compared, not agreed on), ordered by inscription number ascending",
    count: members.length, root: rootOf(members), proof_depth: proofDepth(members.length),
    sources: sources.map((s, i) => ({ name: s.name, url: s.url, count: s.ids.length, unique: new Set(s.ids).size, left_out: only[i].only })),
    references: refs.map((r) => {
      const m = new Set(members), have = new Set(r.ids);
      const extra = r.ids.filter((id) => !m.has(id)), lacks = members.filter((id) => !have.has(id));
      // the ids themselves when few (a stale snapshot lacking thousands is summarised: its count says it)
      return { name: r.name, url: r.url, count: r.ids.length, not_in_members: extra.length <= 50 ? extra : extra.length,
        members_missing: lacks.length <= 50 ? lacks : lacks.length };
    }),
  };
  writeFileSync(y.replace(/\.json$/, ".provenance.json"), JSON.stringify(prov, null, 2) + "\n");
  console.log(JSON.stringify({ slug: x, count: prov.count, root: prov.root, sources: prov.sources.map((s) => `${s.name} ${s.count} (left out ${s.left_out.length})`) }));
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((err) => { console.error(String(err.message ?? err)); process.exit(2); });
}
