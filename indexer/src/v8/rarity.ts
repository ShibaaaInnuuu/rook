import { sha256, stringToHex } from "viem";
import type { Hex, MetadataSnapshot, SnapshotDrift, SnapshotToken } from "./types.js";

/**
 * docs/v8/redemption-draw.md §2.1. The metadata snapshot's hash, the trait counts, and a rarity rank reproducible by anyone from
 * the snapshot alone: OpenRarity's method (the information content of a token's traits over the collection's entropy; a missing
 * trait counts as the value "none", the number of traits counts as a trait of its own, and a token holding a value no other token
 * has is ranked above every token that does not). Pure.
 *
 * Decision R2 (the recommended default): the rank is shown on the inventory, never on the redeem flow.
 */
export const NONE = "none";
export const TRAIT_COUNT = "trait count";

/** JSON with keys sorted at every level and no whitespace: the bytes the hash is taken over. */
export function canonicalJson(x: unknown): string {
  if (x === null || typeof x !== "object") return JSON.stringify(x);
  if (Array.isArray(x)) return `[${x.map(canonicalJson).join(",")}]`;
  return `{${Object.keys(x as object).sort().map((k) => `${JSON.stringify(k)}:${canonicalJson((x as Record<string, unknown>)[k])}`).join(",")}}`;
}
export const snapshotHash = (s: MetadataSnapshot): Hex => sha256(stringToHex(canonicalJson(s)));

/** Every trait type with each value's count across the collection, "none" and the trait count included. */
export function traitCounts(tokens: SnapshotToken[]): Map<string, Map<string, number>> {
  const types = new Set<string>();
  for (const t of tokens) for (const k of Object.keys(t.traits)) types.add(k);
  const out = new Map<string, Map<string, number>>();
  const bump = (k: string, v: string) => { const m = out.get(k) ?? new Map<string, number>(); out.set(k, m); m.set(v, (m.get(v) ?? 0) + 1); };
  for (const t of tokens) {
    for (const k of types) bump(k, t.traits[k] ?? NONE);
    bump(TRAIT_COUNT, String(Object.keys(t.traits).length));
  }
  return out;
}

export type Rarity = { tokenId: string; rank: number; score: number; unique: boolean };
/** Ranks every token of the snapshot: 1 is the rarest. Ties share a rank. */
export function rarityRanks(tokens: SnapshotToken[]): Map<string, Rarity> {
  const counts = traitCounts(tokens), N = tokens.length;
  let entropy = 0;
  for (const m of counts.values()) for (const c of m.values()) { const p = c / N; entropy -= p * Math.log2(p); }
  const types = [...counts.keys()].filter((k) => k !== TRAIT_COUNT);
  const scored = tokens.map((t) => {
    let ic = 0, unique = false;
    const values: [string, string][] = [...types.map((k) => [k, t.traits[k] ?? NONE] as [string, string]), [TRAIT_COUNT, String(Object.keys(t.traits).length)]];
    for (const [k, v] of values) { const c = counts.get(k)!.get(v)!; ic -= Math.log2(c / N); if (c === 1 && v !== NONE) unique = true; }
    return { tokenId: t.tokenId, score: entropy > 0 ? ic / entropy : 0, unique };
  });
  // rounded, so the rank does not depend on how a platform sums floating point
  const key = (x: { score: number }) => Math.round(x.score * 1e9);
  scored.sort((a, b) => Number(b.unique) - Number(a.unique) || key(b) - key(a) || cmpId(a.tokenId, b.tokenId));
  const out = new Map<string, Rarity>();
  let rank = 0;
  scored.forEach((s, i) => { const prev = scored[i - 1]; if (!prev || prev.unique !== s.unique || key(prev) !== key(s)) rank = i + 1; out.set(s.tokenId, { ...s, rank }); });
  return out;
}
const cmpId = (a: string, b: string) => (a.length - b.length) || (a < b ? -1 : a > b ? 1 : 0);

/** A later read of the metadata against the snapshot: every changed trait, shown and never applied. */
export function diffSnapshots(pinned: MetadataSnapshot, now: { tokens: (SnapshotToken | null)[] }, checkedAt: number): SnapshotDrift {
  const was = new Map(pinned.tokens.map((t) => [t.tokenId, t]));
  const changed: SnapshotDrift["changed"] = [];
  let unreadable = 0;
  for (const t of now.tokens) {
    if (!t) { unreadable++; continue; }
    const old = was.get(t.tokenId);
    const keys = new Set([...Object.keys(old?.traits ?? {}), ...Object.keys(t.traits)]);
    for (const k of [...keys].sort()) {
      const a = old?.traits[k] ?? null, b = t.traits[k] ?? null;
      if (a !== b) changed.push({ tokenId: t.tokenId, trait: k, was: a, now: b });
    }
  }
  return { checkedAt, changed, unreadable };
}

/** ERC-721 metadata JSON → the snapshot's traits: `attributes[].trait_type` / `value`, strings only, nothing else kept. */
export function traitsOf(meta: unknown): Record<string, string> | null {
  const a = (meta as { attributes?: unknown })?.attributes;
  if (!Array.isArray(a)) return null;
  const out: Record<string, string> = {};
  for (const x of a) {
    const k = (x as { trait_type?: unknown })?.trait_type, v = (x as { value?: unknown })?.value;
    if (typeof k === "string" && k.length <= 64 && (typeof v === "string" || typeof v === "number") && String(v).length <= 128) out[k] = String(v);
  }
  return out;
}
