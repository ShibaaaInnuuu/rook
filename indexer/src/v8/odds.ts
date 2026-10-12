import type { Frac, ItemOdds, Odds, SnapshotToken, TraitOdds, VaultState } from "./types.js";

/**
 * docs/v8/redemption-draw.md §2.2: the exact probability of each item for the ticket a burn mined now would get.
 *
 * Items fall into groups by the first ticket that may draw them: the free set (every ticket), and each pending stamp s (tickets
 * with id ≥ s). Items of one group are exchangeable, so the state after the k unresolved tickets ahead is the count left in each
 * group. Each ticket ahead removes one item uniformly from the groups it may draw from; a dynamic programme over those counts,
 * in exact fractions, gives the marginal for our ticket: P(item of group g) = Σ_state P(state) · (left_g / n_g) · 1 / Σ left.
 * With no ticket ahead every item is 1/N. When the state space is too large (it is not, at the caps) a seeded Monte Carlo with its
 * standard error is used instead, and said. The odds assume every ticket ahead resolves (a ticket refunded while pinned draws nothing).
 */
export const MAX_STATES = 200_000;
export const MC_SAMPLES = 200_000;

const gcd = (a: bigint, b: bigint): bigint => { a = a < 0n ? -a : a; b = b < 0n ? -b : b; while (b) [a, b] = [b, a % b]; return a; };
export class F {
  readonly n: bigint; readonly d: bigint;
  constructor(n: bigint, d = 1n) { if (d === 0n) throw new Error("zero denominator"); const s = d < 0n ? -1n : 1n, k = gcd(n, d) || 1n; this.n = (s * n) / k; this.d = (s * d) / k; }
  add(o: F) { return new F(this.n * o.d + o.n * this.d, this.d * o.d); }
  mul(o: F) { return new F(this.n * o.n, this.d * o.d); }
  eq(o: F) { return this.n === o.n && this.d === o.d; }
  toFrac(): Frac { return { n: this.n.toString(), d: this.d.toString() }; }
  toNumber(): number { return Number((this.n * 10n ** 15n) / this.d) / 1e15; }
  static of(x: Frac) { return new F(BigInt(x.n), BigInt(x.d)); }
}
export const ZERO = new F(0n), ONE = new F(1n);

type Group = { stamp: number; items: number[] };

/** The odds for the next ticket (`state.ticketCount`) if it were burned at `state.block`. */
export function nextTicketOdds(state: VaultState, tokenOf: (itemId: number) => string = (id) => state.items.find((i) => i.itemId === id)?.tokenId ?? "?", o: { maxStates?: number; samples?: number } = {}): Odds {
  const forTicket = state.ticketCount;
  const groups: Group[] = [{ stamp: -1, items: state.free.slice() }];
  for (const p of state.pending) {
    const last = groups[groups.length - 1];
    if (last.stamp === p.stamp) last.items.push(p.itemId); else groups.push({ stamp: p.stamp, items: [p.itemId] });
  }
  const size = groups.reduce((a, g) => a + g.items.length, 0);
  const ahead = state.pinned.filter((t) => t.id < forTicket).map((t) => t.id).sort((a, b) => a - b);
  const redeemable = state.held >= state.openTickets + 1;
  const empty: Odds = { forTicket, size: Math.max(0, size - ahead.length), ahead: ahead.length, method: "uniform", stderr: null, samples: null, items: [], redeemable };
  if (size === 0 || !redeemable) return { ...empty, size: 0, items: [], redeemable: false };

  if (ahead.length === 0) {
    const p = new F(1n, BigInt(size));
    return { ...empty, size, items: groups.flatMap((g) => g.items).map((itemId) => ({ itemId, tokenId: tokenOf(itemId), p: p.toFrac(), pct: p.toNumber() * 100 })) };
  }
  const exact = exactMarginals(groups, ahead, o.maxStates ?? MAX_STATES);
  const samples = o.samples ?? MC_SAMPLES;
  const per = exact ?? monteCarlo(groups, ahead, samples);
  const items: ItemOdds[] = [];
  groups.forEach((g, gi) => { for (const itemId of g.items) items.push({ itemId, tokenId: tokenOf(itemId), p: per.p[gi].toFrac(), pct: per.p[gi].toNumber() * 100 }); });
  return { ...empty, size: size - ahead.length, method: exact ? "exact" : "monte-carlo", stderr: exact ? null : per.stderr, samples: exact ? null : samples, items };
}

/** Per group, the probability that ONE given item of it is our ticket's; null when the state space is past MAX_STATES. */
function exactMarginals(groups: Group[], ahead: number[], maxStates: number): { p: F[]; stderr: number } | null {
  const n = groups.map((g) => g.items.length);
  let states = new Map<string, { c: number[]; p: F }>([[n.join(","), { c: n.slice(), p: ONE }]]);
  for (const t of ahead) {
    const next = new Map<string, { c: number[]; p: F }>();
    for (const { c, p } of states.values()) {
      const eligible = groups.map((g, i) => (g.stamp <= t ? i : -1)).filter((i) => i >= 0);
      const total = eligible.reduce((a, i) => a + c[i], 0);
      if (total === 0) { const k = c.join(","); const s = next.get(k); next.set(k, { c, p: s ? s.p.add(p) : p }); continue; }   // unreachable while the ledger holds
      for (const i of eligible) {
        if (!c[i]) continue;
        const c2 = c.slice(); c2[i]--;
        const k = c2.join(","), q = p.mul(new F(BigInt(c[i]), BigInt(total)));
        const s = next.get(k); next.set(k, { c: c2, p: s ? s.p.add(q) : q });
      }
      if (next.size > maxStates) return null;
    }
    states = next;
  }
  const out = n.map(() => ZERO);
  for (const { c, p } of states.values()) {
    const left = c.reduce((a, b) => a + b, 0);
    if (!left) continue;
    c.forEach((ci, g) => { if (ci && n[g]) out[g] = out[g].add(p.mul(new F(BigInt(ci), BigInt(n[g] * left)))); });
  }
  return { p: out, stderr: 0 };
}

/** A seeded Monte Carlo of the same process (mulberry32), for a state space too large to enumerate. */
function monteCarlo(groups: Group[], ahead: number[], samples: number): { p: F[]; stderr: number } {
  let seed = 0x5eed_0f_0d;
  const rnd = () => { seed = (seed + 0x6d2b79f5) | 0; let t = seed; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
  const n = groups.map((g) => g.items.length), hits = n.map(() => 0);
  for (let s = 0; s < samples; s++) {
    const c = n.slice();
    for (const t of ahead) {
      const eligible = groups.map((g, i) => (g.stamp <= t ? i : -1)).filter((i) => i >= 0 && c[i] > 0);
      const total = eligible.reduce((a, i) => a + c[i], 0);
      if (!total) continue;
      let r = Math.floor(rnd() * total);
      for (const i of eligible) { if (r < c[i]) { c[i]--; break; } r -= c[i]; }
    }
    const left = c.reduce((a, b) => a + b, 0);
    let r = Math.floor(rnd() * left);
    for (let i = 0; i < c.length; i++) { if (r < c[i]) { hits[i]++; break; } r -= c[i]; }
  }
  // per item: the group's hit rate spread over its n items
  const p = hits.map((h, g) => new F(BigInt(h), BigInt(samples * Math.max(1, n[g]))));
  const stderr = Math.max(...hits.map((h, g) => { const q = h / samples; return Math.sqrt((q * (1 - q)) / samples) / Math.max(1, n[g]); }));
  return { p, stderr };
}

/** §2.2 "Per trait": the probability the drawn item has each value, the count in the vault and in the whole collection. */
export function traitOdds(odds: Odds, tokens: Map<string, SnapshotToken>, collectionCounts: Map<string, Map<string, number>>): TraitOdds[] {
  const by = new Map<string, Map<string, { items: number; p: F }>>();
  for (const it of odds.items) {
    const t = tokens.get(it.tokenId);
    if (!t) continue;
    for (const [trait, value] of Object.entries(t.traits)) {
      const m = by.get(trait) ?? new Map(); by.set(trait, m);
      const e = m.get(value) ?? { items: 0, p: ZERO };
      m.set(value, { items: e.items + 1, p: e.p.add(F.of(it.p)) });
    }
  }
  return [...by].sort(([a], [b]) => a.localeCompare(b)).map(([trait, m]) => ({ trait,
    values: [...m].map(([value, e]) => ({ value, items: e.items, p: e.p.toFrac(), pct: e.p.toNumber() * 100, inCollection: collectionCounts.get(trait)?.get(value) ?? 0 }))
      .sort((a, b) => b.pct - a.pct || a.value.localeCompare(b.value)) }));
}

/** "1 in 14" when the fraction has numerator 1, else the reduced fraction; and a percentage to one decimal. */
export const fracWords = (p: Frac): string => (p.n === "1" ? `1 in ${p.d}` : `${p.n} / ${p.d}`);
export const pctWords = (x: number): string => `${x < 0.1 && x > 0 ? x.toFixed(3) : x.toFixed(1)} %`;
