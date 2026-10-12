import { concat, encodeFunctionData, keccak256, parseAbi, toHex, type Hex } from "viem";

/**
 * docs/phase11.md. The catalogue (web/public/catalogue/catalogue.json, Phase 9) turned into listings — the ONE place that
 * decides what a listing is. Pure: a parsed catalogue in, planned listings out. No node, no key, no chain, no clock.
 *
 * The same function serves both chains. Staging sends the calls through the node's impersonation of the owner
 * (staging/bootstrap/listings.ts); a real chain gets `listingCalls()` printed as calldata for the multisig to sign, and is
 * REFUSED the staging fixture members that make a placeholder root (see `plannedListings`).
 */

/** Our own chains: the staging fork and a local anvil. Only these may pad a membership root with fixture members. */
export const OUR_CHAINS: readonly number[] = [9466301, 31337];

export type CatalogueItem = { id: string; num?: number; name?: string };
export type CatalogueCollection = {
  id: string; slug: string; name: string; kind: "inscriptions"; supply?: number | null; floorSats?: number | null;
  /** the sats traded in the seven days before the fetch, when a source would state one (docs/phase9.md §11) */
  volume7dSats?: number | null;
  /**
   * a lifetime figure where the source published one. It ranks the whole history, not this week — and since no
   * marketplace states a Bitcoin collection's week, it is what the collections are ranked by (docs/phase9.md §11.3).
   */
  volumeAllTimeSats?: number | null;
  sampleInscription?: string | null; items?: CatalogueItem[] | null; sourceUrl?: string;
};
export type CatalogueRune = {
  id: string; slug: string; name: string; kind: "rune"; runeId: string; divisibility: number; symbol?: string;
  supply?: string | null; floorSats?: number | null; volume7dSats?: number | null; sourceUrl?: string;
};
export type Catalogue = {
  fetchedAt: string;
  /** the two bases in one sentence, written by the fetch. Absent on a file fetched before it was recorded. */
  rankedBy?: string;
  collections: CatalogueCollection[]; runes: CatalogueRune[];
};

/** What the root of this listing actually commits to. Written into the manifest so the claim travels with the listing. */
export type RootCovers = {
  /** inscription ids the catalogue really holds for this collection */
  realIds: string[];
  /** staging ids that name nothing, added so a long-lived fork can keep delivering; 0 on a real chain */
  fixtureMembers: number;
  placeholder: boolean;
  note: string;
};

export type PlannedListing = {
  kind: "inscriptions" | "rune";
  /** as it will be listed on chain */
  name: string;
  /** the registry's `ref`: a collection's slug, a rune's id */
  ref: string;
  divisibility: number;
  /** inscriptions only; a rune commits to nothing */
  membershipRoot: Hex | null;
  btcVault: string;
  rootCovers: RootCovers | null;
  /** the catalogue's dated floor, in sats. Null for every rune: no keyless market would give one up. */
  floorSats: number | null;
  /** 1..5, from the catalogue's own volume: what the simulator launches into more often. `ListingPlan.weightBasis` says which figure. */
  weight: number;
  sourceUrl: string | null;
};

export type SkippedListing = { slug: string; name: string; kind: string; why: string };

export type ListingPlan = {
  source: string;
  fetchedAt: string;
  chainId: number;
  /** which of the catalogue's volume figures the weights were taken from, said in words rather than left to be assumed */
  weightBasis: string;
  listings: PlannedListing[];
  skipped: SkippedListing[];
};

export const CATALOGUE_PATH = "web/public/catalogue/catalogue.json";
/** The fixture collection Phase 6 built: 1,024 ids that name nothing. keeper/staging/members.ts builds the same leaves. */
export const STAGING_FIXTURE_MEMBERS = 1024;
export const fixtureMemberId = (i: number): string => `${"0".repeat(64)}i${i}`;

/**
 * A Bitcoin address NOBODY can be paid at: hyphens are not in the bech32m charset, so this is not a valid address and no wallet
 * will send to it. Deliberately not a BIP-350 test vector — a *valid* address invites a page to draw a QR code someone could
 * scan. docs/phase11.md §3. A real listing needs a real taproot address per vault, from whoever holds the custody keys.
 */
export const btcVaultFixture = (slug: string): string => `bc1p-staging-fixture-not-a-real-address-${slug}`;

/** ListingRegistry.leafOf: keccak256(bytes.concat(keccak256(bytes(id)))). */
export const leafOf = (inscriptionId: string): Hex => keccak256(keccak256(toHex(inscriptionId)));
const pair = (a: Hex, b: Hex): Hex => keccak256(BigInt(a) < BigInt(b) ? concat([a, b]) : concat([b, a]));

/**
 * A sorted-pair Merkle tree, as OpenZeppelin's `MerkleProof.verify` reads one. An odd node at any level is carried up unchanged
 * rather than paired with a copy of itself, so no leaf can be proven twice by the shape of the tree.
 */
export function merkleLevels(leaves: readonly Hex[]): Hex[][] {
  if (!leaves.length) throw new Error("a membership root over no inscription id would commit to nothing");
  const levels: Hex[][] = [[...leaves]];
  while (levels[levels.length - 1].length > 1) {
    const l = levels[levels.length - 1], next: Hex[] = [];
    for (let i = 0; i + 1 < l.length; i += 2) next.push(pair(l[i], l[i + 1]));
    if (l.length % 2) next.push(l[l.length - 1]);
    levels.push(next);
  }
  return levels;
}
export function merkleRoot(leaves: readonly Hex[]): Hex { const l = merkleLevels(leaves); return l[l.length - 1][0]; }

/** The proof of the leaf at `index`, in the order `MerkleProof.verify` folds it. */
export function merkleProof(leaves: readonly Hex[], index: number): Hex[] {
  const levels = merkleLevels(leaves), proof: Hex[] = [];
  for (let level = 0, at = index; level < levels.length - 1; level++, at = Math.floor(at / 2)) {
    const sibling = levels[level][at ^ 1];
    if (sibling !== undefined) proof.push(sibling);        // an odd node at this level has none: it is carried up as it is
  }
  return proof;
}
/** OpenZeppelin's own fold, so a test can check a proof without a chain. */
export const processProof = (proof: readonly Hex[], leaf: Hex): Hex => proof.reduce((acc, p) => pair(acc, p), leaf);

/** The inscription ids the catalogue actually holds for a collection: its sample, and the few items kept for the picture strip. */
export function knownIdsOf(c: CatalogueCollection): string[] {
  const out: string[] = [];
  for (const id of [c.sampleInscription, ...(c.items ?? []).map((i) => i?.id)]) {
    if (typeof id === "string" && /^[0-9a-f]{64}i\d+$/.test(id) && !out.includes(id)) out.push(id);
  }
  return out;
}

/** The leaves of a staging collection: the real ids first, then the fixture members. Order is part of the commitment. */
export const membersOf = (realIds: readonly string[], fixtureMembers: number): string[] =>
  [...realIds, ...Array.from({ length: fixtureMembers }, (_x, i) => fixtureMemberId(i))];

/** 1..5 from the catalogue's own volume, doubling each step. Not a hand-picked favourite list. */
export function weightOf(volumeSats: number | null | undefined, smallest: number): number {
  if (!volumeSats || volumeSats <= 0 || smallest <= 0) return 1;
  return Math.max(1, Math.min(5, Math.round(Math.log2(Math.max(1, volumeSats / smallest)))));
}

/**
 * Which of the catalogue's volume figures the weights come from, decided ONCE for the whole file rather than per
 * collection, so that no two weights are taken from different windows and compared as though they were the same number.
 *
 * A catalogue whose collections carry `volume7dSats` is weighed by the seven days before the fetch. One whose
 * collections carry only a lifetime figure is weighed by that — which is every keyed fetch since docs/phase9.md §11.3,
 * because no marketplace states a Bitcoin collection's week — and the plan says LIFETIME in words rather than letting a
 * reader take the weights for this week's.
 */
export const WEIGHT_BASES = {
  week: { field: "volume7dSats", said: "the catalogue's seven-day volume, which is what its collections are ranked by" },
  lifetime: { field: "volumeAllTimeSats", said: "the catalogue's LIFETIME volume for collections: no marketplace states a Bitcoin collection's week (docs/phase9.md §11.3), so the weights rank the whole history of a market rather than this week" },
  none: { field: null, said: "nothing: the catalogue carries no volume figure at all, so every collection weighs the same" },
} as const;

export function weightBasisOf(cat: Catalogue): (typeof WEIGHT_BASES)[keyof typeof WEIGHT_BASES] {
  if (cat.collections.some((c) => (c.volume7dSats ?? 0) > 0)) return WEIGHT_BASES.week;
  if (cat.collections.some((c) => (c.volumeAllTimeSats ?? 0) > 0)) return WEIGHT_BASES.lifetime;
  return WEIGHT_BASES.none;
}

/**
 * Every rune sits in the middle, deliberately, whatever volume the catalogue carries for it.
 *
 * A keyed catalogue DOES rank its runes, by the seven days before the fetch — but its collections are ranked by lifetime
 * volume (docs/phase9.md §11.3), and a weight is a single scale the simulator compares across both kinds. Weighing runes
 * by a week and collections by a decade on that one scale would put every rune at the bottom of it for a reason that has
 * nothing to do with either market. So the runes take a fixed middling weight and the plan's `weightBasis` names the
 * collections' figure, which is the only one any weight is computed from.
 */
export const RUNE_WEIGHT = 2;

export type PlanOptions = {
  chainId: number;
  /** how many fixture members pad a collection's root. Refused off our own chains. */
  fixtureMembers?: number;
  source?: string;
};

/**
 * The catalogue as listings. Every collection that holds at least one real inscription id, and every rune.
 *
 * `fixtureMembers` > 0 is a STAGING device: it pads the root so a fork that is never reset can keep settling inscriptions after
 * the handful of real ids are held. It is refused on any chain that is not ours, because a real listing's root must commit to
 * the collection's real membership and nothing else (docs/phase11.md §1).
 */
export function plannedListings(cat: Catalogue, o: PlanOptions): ListingPlan {
  const fixtureMembers = o.fixtureMembers ?? 0;
  if (fixtureMembers > 0 && !OUR_CHAINS.includes(o.chainId)) {
    throw new Error(`refused: chain ${o.chainId} is not ours, and a real listing's membership root may not be padded with `
      + `${fixtureMembers} staging fixture members. Build the root from the collection's verified item list (docs/phase11.md §1).`);
  }
  const listings: PlannedListing[] = [], skipped: SkippedListing[] = [];
  const basis = weightBasisOf(cat);
  const volumeOf = (c: CatalogueCollection): number => (basis.field ? c[basis.field] ?? 0 : 0);
  const volumes = cat.collections.map(volumeOf).filter((v) => v > 0);
  const smallest = volumes.length ? Math.min(...volumes) : 0;

  for (const c of cat.collections) {
    const realIds = knownIdsOf(c);
    if (!realIds.length) {
      skipped.push({ slug: c.slug, name: c.name, kind: "inscriptions",
        why: "the catalogue holds no inscription id for it (sampleInscription is null and items is empty), so its membership root "
          + "would commit either to nothing, which ListingRegistry rejects, or to fixture ids alone, which would be a vault "
          + "claiming a real collection while committing to ids that name nothing" });
      continue;
    }
    const members = membersOf(realIds, fixtureMembers);
    listings.push({
      kind: "inscriptions", name: c.name, ref: c.slug, divisibility: 0,
      membershipRoot: merkleRoot(members.map(leafOf)),
      btcVault: btcVaultFixture(c.slug),
      rootCovers: { realIds, fixtureMembers, placeholder: true,
        note: `a placeholder: the catalogue holds ${realIds.length} of this collection's inscription ids, not its membership`
          + `${fixtureMembers ? `, so the root is those ${realIds.length} ids and ${fixtureMembers} staging fixture ids that name nothing` : ""}`
          + ". Nothing here has verified the collection's full item list." },
      floorSats: c.floorSats ?? null,
      weight: weightOf(volumeOf(c), smallest),
      sourceUrl: c.sourceUrl ?? null,
    });
  }

  for (const r of cat.runes) {
    if (!/^\d+:\d+$/.test(r.runeId ?? "")) { skipped.push({ slug: r.slug, name: r.name, kind: "rune", why: `its rune id ${JSON.stringify(r.runeId)} is not <block>:<tx>` }); continue; }
    if (!Number.isInteger(r.divisibility) || r.divisibility < 0 || r.divisibility > 38) { skipped.push({ slug: r.slug, name: r.name, kind: "rune", why: `divisibility ${r.divisibility} is outside what the registry accepts (0-38)` }); continue; }
    listings.push({
      kind: "rune", name: r.name, ref: r.runeId, divisibility: r.divisibility, membershipRoot: null,
      btcVault: btcVaultFixture(r.slug), rootCovers: null,
      floorSats: null,             // the catalogue has no rune floor, so no cap is ever posted: docs/phase11.md §2
      weight: RUNE_WEIGHT, sourceUrl: r.sourceUrl ?? null,
    });
  }
  return { source: o.source ?? CATALOGUE_PATH, fetchedAt: cat.fetchedAt, chainId: o.chainId,
    weightBasis: basis.said, listings, skipped };
}

export const REGISTRY_ABI = parseAbi([
  "function listInscriptions(string name, string slug, bytes32 membershipRoot, string btcVault) returns (uint256)",
  "function listRune(string name, string runeId, uint8 divisibility, string btcVault) returns (uint256)",
  "function count() view returns (uint256)",
  "function listing(uint256 id) view returns ((uint8 kind, bool paused, uint8 divisibility, address vault, bytes32 membershipRoot, string name, string ref, string btcVault))",
  "function owner() view returns (address)",
]);

export type ListingCall = { ref: string; name: string; functionName: "listInscriptions" | "listRune"; args: unknown[]; calldata: Hex };

/** The owner-only call each planned listing needs. On staging the node sends it; on a real chain a multisig signs this calldata. */
export function listingCalls(plan: ListingPlan): ListingCall[] {
  return plan.listings.map((l) => (l.kind === "inscriptions"
    ? { ref: l.ref, name: l.name, functionName: "listInscriptions" as const, args: [l.name, l.ref, l.membershipRoot as Hex, l.btcVault],
        calldata: encodeFunctionData({ abi: REGISTRY_ABI, functionName: "listInscriptions", args: [l.name, l.ref, l.membershipRoot as Hex, l.btcVault] }) }
    : { ref: l.ref, name: l.name, functionName: "listRune" as const, args: [l.name, l.ref, l.divisibility, l.btcVault],
        calldata: encodeFunctionData({ abi: REGISTRY_ABI, functionName: "listRune", args: [l.name, l.ref, l.divisibility, l.btcVault] }) }));
}

/**
 * What the keeper posts a cap from: the catalogue's dated floor per collection slug, and NOTHING for runes — the catalogue has
 * no rune floor, so a rune vault posts no cap and `declarePurchase` refuses (docs/phase11.md §2).
 */
export function pricesFor(plan: ListingPlan): { inscriptions: Record<string, string>; runes: Record<string, string>; note: string } {
  const inscriptions: Record<string, string> = {};
  for (const l of plan.listings) if (l.kind === "inscriptions" && l.floorSats) inscriptions[l.ref] = String(l.floorSats);
  return { inscriptions, runes: {},
    note: `staging fixture: floors are the catalogue's, from ${plan.fetchedAt}, and are never refreshed. Runes carry no floor, `
      + "so no cap is posted for one and its vault refuses to declare a purchase." };
}

/**
 * Which listing each simulated coin launches into. Every vault gets a coin before any vault gets a second, popular collections
 * come first and come round again sooner, and a rune comes round often enough that pro-rata distributions stay on screen.
 *
 * Takes anything with a kind and a weight, so the simulator can order the listings it read from the manifest with the same
 * function the plan is built with. docs/phase11.md §7.
 *
 * TWO THINGS IT CANNOT PROMISE, because the catalogue decides them and not this function (docs/phase9.md §11.3):
 *  - runes never come two in a row. They are spread as evenly as the two counts allow, which is every other coin when the
 *    lists are the same length and closer together still if the runes outnumber the collections. Fifty of each kind makes
 *    that the normal case rather than the odd one.
 *  - a collection is only reached again after the first pass if some collection outweighs the rest. A catalogue whose
 *    volumes sit inside one band weighs every collection the same, so the later rounds take EVERY collection rather than
 *    none: the last pass admits weight 1, which is what stops a flat catalogue from launching into runes for ever.
 */
export function launchOrder<T extends { kind: "inscriptions" | "rune"; weight: number }>(listings: readonly T[], n: number): T[] {
  const runes = listings.filter((l) => l.kind === "rune");
  const cols = listings.filter((l) => l.kind === "inscriptions").slice().sort((a, b) => b.weight - a.weight);
  if (!cols.length && !runes.length) return [];
  const out: T[] = [];
  // the first pass reaches every vault exactly once: the busiest collections first, the runes spread evenly through them
  for (let ci = 0, ri = 0; out.length < n && (ci < cols.length || ri < runes.length); ) {
    const takeRune = ri < runes.length && (ci >= cols.length || ri * cols.length < ci * runes.length);
    out.push(takeRune ? runes[ri++] : cols[ci++]);
  }
  // then round again by weight, a rune every fourth, so the popular ones carry more coins than the tail. The last pass
  // admits weight 1, so every collection is in the sequence at least once however flat the catalogue's volumes are.
  const seq: T[] = [];
  for (let pass = 2; pass <= 6; pass++) for (const c of cols) if (c.weight >= 7 - pass) seq.push(c);
  for (let i = 0, ci = 0, ri = 0; out.length < n; i++) {
    const takeRune = i % 4 === 3 && runes.length > 0;
    out.push(takeRune ? runes[ri++ % runes.length] : (seq.length ? seq[ci++ % seq.length] : runes[ri++ % runes.length]));
  }
  return out;
}
