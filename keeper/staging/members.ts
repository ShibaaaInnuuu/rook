import { concat, keccak256, toHex, type Hex } from "viem";
import { leafOf as catalogueLeafOf, membersOf, merkleProof, merkleRoot } from "../../staging/catalogue/listings.js";
import { readManifest } from "../../staging/bootstrap/listings.js";

/**
 * The staging fork's fixture collection: 1,024 inscription ids that name nothing (…i0 to …i1023), and the Merkle tree the registry
 * checks them against. script/Deploy.s.sol builds the same root for chain 9466301; test/send-guards.test.ts pins it here and
 * test/DeployChecks.t.sol pins it there, to the same value.
 */
export const STAGING_MEMBERS = 1024;
export const memberId = (i: number): string => `${"0".repeat(64)}i${i}`;
export const leafOf = (id: string): Hex => keccak256(keccak256(toHex(id)));
const pair = (a: Hex, b: Hex): Hex => keccak256(BigInt(a) < BigInt(b) ? concat([a, b]) : concat([b, a]));

let levels: Hex[][] | null = null;
function tree(): Hex[][] {
  if (levels) return levels;
  const out: Hex[][] = [Array.from({ length: STAGING_MEMBERS }, (_, i) => leafOf(memberId(i)))];
  while (out[out.length - 1].length > 1) { const l = out[out.length - 1]; out.push(Array.from({ length: l.length / 2 }, (_, i) => pair(l[2 * i], l[2 * i + 1]))); }
  return (levels = out);
}
export const stagingRoot = (): Hex => tree()[tree().length - 1][0];
export function proofOf(i: number): Hex[] {
  const t = tree(), proof: Hex[] = [];
  for (let level = 0, at = i; level < t.length - 1; level++, at = Math.floor(at / 2)) proof.push(t[level][at ^ 1]);
  return proof;
}

/**
 * docs/phase11.md §1. A CATALOGUE listing's members: the few real inscription ids the catalogue holds for that collection,
 * then the 1,024 fixture ids that name nothing. Read from the manifest the listing creator writes (${STAGING_DATA}/listings.json),
 * which is also where the claim lives that this root is a PLACEHOLDER and commits to nothing like the whole collection.
 *
 * A listing the manifest does not name — the demo collection the deploy makes — keeps the fixture tree above, which
 * script/Deploy.s.sol and test/DeployChecks.t.sol pin between them.
 */
export type ListingMembers = { ids: string[]; leaves: Hex[]; real: number; root: Hex };

const byListing = new Map<string, ListingMembers | null>();

export function membersForListing(data: string, listingId: number): ListingMembers | null {
  const key = `${data}:${listingId}`;                    // keyed by the volume too: two data directories are two chains
  if (byListing.has(key)) return byListing.get(key)!;
  let out: ListingMembers | null = null;
  try {
    const m = readManifest(data);
    const l = m?.listings.find((x) => x.id === listingId);
    if (l && l.kind === "inscriptions" && l.rootCovers) {
      const ids = membersOf(l.rootCovers.realIds, l.rootCovers.fixtureMembers);
      const leaves = ids.map((id) => catalogueLeafOf(id));
      out = { ids, leaves, real: l.rootCovers.realIds.length, root: merkleRoot(leaves) };
    }
  } catch { out = null; }
  byListing.set(key, out);
  return out;
}

/** The proof of the member at `index`, as OpenZeppelin's MerkleProof folds it. */
export const proofForListing = (m: ListingMembers, index: number): Hex[] => merkleProof(m.leaves, index);
