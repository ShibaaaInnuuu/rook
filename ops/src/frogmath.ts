import { concat, keccak256, toHex, type Hex } from "viem";

/**
 * The frog arithmetic both the offer tool (src/judge.ts, which re-exports these) and the runner's Ordinals Wallet step
 * (src/owprotected.ts) use: frogbuy.rs's rate conversions and ListingRegistry's membership proof. Pure: no PSBT, no keys.
 * Moved out of judge.ts unchanged (2026-10-09), so the runner's image carries no offer-tool judge (ops/test/guards.test.ts).
 */
const BPS = 10_000n, WEI = 10n ** 18n;
const CONVERSION_BPS = 200n;             // frogbuy.rs: the ETH→BTC allowance (judge.ts exports the same value)

const divCeil = (a: bigint, b: bigint) => (a + b - 1n) / b;
/** frogbuy::wei_for_sats: ⌈sats × 1e18 / rate⌉ × (1 + CONVERSION_BPS), rounded up. */
export function weiForSats(sats: bigint, satsPerEth: bigint): bigint {
  if (satsPerEth === 0n) throw new Error("no rate");
  return divCeil(divCeil(sats * WEI, satsPerEth) * (BPS + CONVERSION_BPS), BPS);
}
/** frogbuy::sats_for_wei: the inverse, rounded down, stepped down until `weiForSats` fits. */
export function satsForWei(wei: bigint, satsPerEth: bigint): bigint {
  const net = (wei * BPS) / (BPS + CONVERSION_BPS);
  let s = (net * satsPerEth) / WEI;
  while (s > 0n && weiForSats(s, satsPerEth) > wei) s -= 1n;
  return s;
}

/** ListingRegistry.leafOf / frogbuy::leaf_of: keccak256(keccak256(the id's bytes)). */
export const leafOf = (inscriptionId: string): Hex => keccak256(keccak256(toHex(inscriptionId)));
const pair = (a: Hex, b: Hex): Hex => keccak256(BigInt(a) <= BigInt(b) ? concat([a, b]) : concat([b, a]));
/** OpenZeppelin's MerkleProof.verify (sorted pairs), as frogbuy::verify_proof. */
export const verifyProof = (proof: readonly Hex[], root: Hex, leaf: Hex): boolean => proof.reduce<Hex>((h, p) => pair(h, p), leaf).toLowerCase() === root.toLowerCase();
/** A proof for `id` in the tree of `members` built as script/Deploy.s.sol and staging/members.ts build it: leaves in list order, pairs sorted, an odd node moves up. */
export function proofFor(members: readonly string[], id: string): { root: Hex; proof: Hex[] } {
  const i = members.indexOf(id);
  if (i < 0) throw new Error(`${id} is not in the member list`);
  let level: Hex[] = members.map(leafOf), at = i;
  const proof: Hex[] = [];
  while (level.length > 1) {
    const sib = at ^ 1;
    if (sib < level.length) proof.push(level[sib]);
    const next: Hex[] = [];
    for (let k = 0; k < level.length; k += 2) next.push(k + 1 < level.length ? pair(level[k], level[k + 1]) : level[k]);
    level = next; at >>= 1;
  }
  return { root: level[0], proof };
}

/**
 * Ordinals multi-collection (docs/v9/ordinals-collections.md §4): the same tree as `proofFor`, built ONCE per member list and kept (a
 * WeakMap on the list), so a large collection (Runestone's 112,384 ids) is hashed once per process, not once per proof.
 */
export type MemberTree = { root: Hex; size: number; has(id: string): boolean; proof(id: string): Hex[] };
const trees = new WeakMap<readonly string[], MemberTree>();
export function memberTree(members: readonly string[]): MemberTree {
  const kept = trees.get(members);
  if (kept) return kept;
  if (!members.length) throw new Error("an empty member list");
  const index = new Map<string, number>();
  members.forEach((m, i) => { if (!index.has(m)) index.set(m, i); });
  const levels: Hex[][] = [members.map(leafOf)];
  while (levels[levels.length - 1].length > 1) {
    const level = levels[levels.length - 1], next: Hex[] = [];
    for (let k = 0; k < level.length; k += 2) next.push(k + 1 < level.length ? pair(level[k], level[k + 1]) : level[k]);
    levels.push(next);
  }
  const t: MemberTree = {
    root: levels[levels.length - 1][0], size: members.length, has: (id) => index.has(id),
    proof(id) {
      let at = index.get(id);
      if (at === undefined) throw new Error(`${id} is not in the member list`);
      const proof: Hex[] = [];
      for (const level of levels.slice(0, -1)) { const sib = at ^ 1; if (sib < level.length) proof.push(level[sib]); at >>= 1; }
      return proof;
    },
  };
  trees.set(members, t);
  return t;
}

