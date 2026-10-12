import { readFileSync } from "node:fs";
import { memberTree, type MemberTree } from "./frogmath.js";
import type { Log } from "./log.js";
import { pairListings, rootKey, type FrogColl, type InscListing } from "./solfloor.js";

/**
 * Ordinals multi-collection (docs/v9/ordinals-collections.md §4, the merged canister): the runner's per-collection Bitcoin steps.
 *
 * RUNNER_INSCRIPTIONS=<slug>=<owSlug>:<membersFile>[:<unisat>],…  the collections whose listings the runner BUYS: Ordinals Wallet's
 *   protected listings under `owSlug` (empty: that step off), UniSat's fixed-price ones under `unisat` (needs UNISAT_API_KEY), each
 *   inscription proved from `membersFile` (a JSON array of inscription ids, the leaves of the collection's root). The collection is the
 *   canister's whose `root` is the members file's tree root (`frog_collections()`), and it is served only while `live`.
 * The old scalars (RUNNER_OW_SLUG, RUNNER_OW_MEMBERS, RUNNER_UNISAT_COLLECTION) still read, as Bitcoin Frogs' entry.
 *
 * The proof step (on by default on the Solana home): a holder deposit waiting for its membership proof (`holder_deposits()`,
 * `AwaitingProof` or `Parked`) gets it, `frog_proof(coll, inscription, proof)`, from the members file of the LIVE collection that holds the
 * inscription (the launch file's `membersFile`, each checked against the canister's root before it is used).
 */
export const BITCOIN_FROGS_SLUG = "bitcoin-frogs";
const ID = /^[0-9a-f]{64}i[0-9]+$/;
const SLUG = /^[a-z0-9_-]{1,64}$/;

export type InscEntry = { slug: string; owSlug: string | null; membersFile: string; members: string[]; unisat: string | null };

/** A members file: a non-empty JSON array of inscription ids. */
export function loadMembers(path: string): string[] {
  const m = JSON.parse(readFileSync(path, "utf8"));
  if (!Array.isArray(m) || !m.length || !m.every((x) => typeof x === "string" && ID.test(x))) throw new Error("not a non-empty array of inscription ids");
  return m;
}

/** RUNNER_INSCRIPTIONS, and the old scalars as Bitcoin Frogs' entry (refused when both name it). */
export function parseInscriptionsEnv(env: Readonly<Record<string, string | undefined>>, problems: string[]): InscEntry[] {
  const out: InscEntry[] = [];
  const add = (slug: string, owSlug: string | null, membersFile: string, unisat: string | null, from: string) => {
    if (!SLUG.test(slug)) { problems.push(`${from}: "${slug}" is not a collection slug`); return; }
    if (out.some((e) => e.slug === slug)) { problems.push(`${from}: ${slug} is named twice`); return; }
    if (owSlug !== null && !SLUG.test(owSlug)) problems.push(`${from}: ${slug}'s owSlug ${owSlug} is not an Ordinals Wallet collection slug`);
    if (unisat !== null) {
      if (!/^[a-z0-9-]+$/.test(unisat)) problems.push(`${from}: ${slug}'s ${unisat} is not a UniSat collection id`);
      if (!env.UNISAT_API_KEY) problems.push(`${from}: ${slug}'s UniSat step needs UNISAT_API_KEY`);
    }
    if (owSlug === null && unisat === null) problems.push(`${from}: ${slug} names neither an Ordinals Wallet slug nor a UniSat collection: nothing to buy with`);
    let members: string[] = [];
    try { members = loadMembers(membersFile); } catch (err) { problems.push(`${from}: ${slug}'s members file ${membersFile || "(none)"} (${(err as Error).message.slice(0, 120)})`); }
    out.push({ slug, owSlug, membersFile, members, unisat });
  };
  for (const part of (env.RUNNER_INSCRIPTIONS ?? "").split(",").map((s) => s.trim()).filter(Boolean)) {
    const eq = part.indexOf("=");
    const [ow, file, unisat, ...rest] = (eq < 0 ? "" : part.slice(eq + 1)).split(":");
    if (eq < 1 || file === undefined || rest.length) { problems.push(`RUNNER_INSCRIPTIONS: "${part}" is not <slug>=<owSlug>:<membersFile>[:<unisat>]`); continue; }
    add(part.slice(0, eq), ow || null, file, unisat || null, "RUNNER_INSCRIPTIONS");
  }
  // back-compat: the scalars of the one-collection runner are Bitcoin Frogs' entry
  const owSlug = env.RUNNER_OW_SLUG ? String(env.RUNNER_OW_SLUG) : null, unisat = env.RUNNER_UNISAT_COLLECTION ? String(env.RUNNER_UNISAT_COLLECTION) : null;
  if (owSlug !== null || unisat !== null) {
    if (out.some((e) => e.slug === BITCOIN_FROGS_SLUG)) problems.push(`RUNNER_OW_SLUG / RUNNER_UNISAT_COLLECTION and RUNNER_INSCRIPTIONS both name ${BITCOIN_FROGS_SLUG}: keep one`);
    else add(BITCOIN_FROGS_SLUG, owSlug, env.RUNNER_OW_MEMBERS ?? "", unisat, "RUNNER_OW_SLUG / RUNNER_UNISAT_COLLECTION (with RUNNER_OW_MEMBERS)");
  }
  return out;
}

/** Each entry's canister collection: the one whose root is the entry's members tree root (none: not in the canister, or another members set). */
export function entryColls(entries: readonly InscEntry[], colls: readonly FrogColl[]): Map<string, FrogColl> {
  const byRoot = new Map(colls.map((c) => [rootKey(c.root), c]));
  const out = new Map<string, FrogColl>();
  for (const e of entries) { if (!e.members.length) continue; const c = byRoot.get(rootKey(memberTree(e.members).root)); if (c) out.set(e.slug, c); }
  return out;
}

/** A members file that failed to load (or did not match its root) is read again after this: a file fixed on disk is picked up (audit C4-06). */
export const MEMBERS_RETRY_S = 300;

/**
 * The proof step's members: each launch listing's file, loaded once, its tree checked against the canister's root before any use. A load
 * that failed is retried after MEMBERS_RETRY_S, not kept for the process (audit C4-06). On each load the new set is checked against the
 * sets already loaded, and an overlap is logged (audit C4-10: a shared inscription gets no proof, see frogProofOnce).
 */
export class ListingMembers {
  private readonly trees = new Map<number, { t: MemberTree | Error; root: string; at: number; slug: string }>();
  constructor(private readonly load: (path: string) => string[] = loadMembers, private readonly o: { log?: Log; nowS?: () => number } = {}) {}
  tree(l: InscListing, root: string): MemberTree {
    const now = (this.o.nowS ?? (() => Math.floor(Date.now() / 1000)))();
    let e = this.trees.get(l.id);
    if (e === undefined || rootKey(e.root) !== rootKey(root) || (e.t instanceof Error && now - e.at >= MEMBERS_RETRY_S)) {
      let t: MemberTree | Error, members: readonly string[] = [];
      try {
        const m = this.load(l.membersFile), mt = memberTree(m);
        t = rootKey(mt.root) === rootKey(root) ? mt : new Error(`${l.slug}'s members file ${l.membersFile} has root ${mt.root}, not the canister's ${root}`);
        if (!(t instanceof Error)) members = m;
      } catch (err) { t = new Error(`${l.slug}'s members file ${l.membersFile || "(none)"}: ${(err as Error).message.slice(0, 160)}`); }
      e = { t, root, at: now, slug: l.slug };
      this.trees.set(l.id, e);
      // audit C4-10: warn at load time when two members files share inscriptions (each such inscription then gets no proof)
      if (this.o.log) for (const [id, o] of this.trees) {
        if (id === l.id || o.t instanceof Error) continue;
        const ot = o.t, shared = members.filter((x) => ot.has(x));
        if (shared.length) this.o.log("warn", "frog-proof-members-overlap", { slugs: [l.slug, o.slug], shared: shared.length, first: shared[0],
          why: "these inscriptions are in both members files: a deposit of one gets no proof (the runner does not pick the collection)" });
      }
    }
    if (e.t instanceof Error) throw e.t;
    return e.t;
  }
}

export type ProofDeps = {
  buyer: { frog_collections(): Promise<FrogColl[]>; holder_deposits(): Promise<{ phase: Record<string, unknown> }[]>;
    frog_proof(coll: number, inscription: string, proof: string[]): Promise<{ Ok: string } | { Err: string }> };
  log: Log; listings: InscListing[]; members: ListingMembers; nowS?: () => number;
  /** `${coll}:${inscription}` → when it was handed in (seconds): not again for PROOF_AGAIN_S */
  handed?: Map<string, number>;
  /** what keeps the step from working (a members file that does not match its root, an inscription two collections hold), for /healthz */
  problems?: Set<string>;
  /** audit C4-06: the keys of `problems` this step added at its last pass, removed at the start of the next (the set is shared) */
  owned?: Set<string>;
};
/** A proof handed in and still awaited is handed in again after this (a deposit seen before the proof, or a refusal fixed). */
export const PROOF_AGAIN_S = 6 * 3_600;

/** One pass. True when a proof was kept (the runner then ticks again at once, so the deposit is looked at now). */
export async function frogProofOnce(d: ProofDeps): Promise<boolean> {
  const now = d.nowS ?? (() => Math.floor(Date.now() / 1000)), handed = (d.handed ??= new Map());
  // audit C4-06: the step's problems are recomputed each pass (what no longer holds clears); the set's other keys are kept
  const before = new Set(d.owned ?? []);
  for (const k of before) d.problems?.delete(k);
  d.owned?.clear();
  const report = (why: string, event: string, fields: Record<string, unknown>) => {
    if (!before.has(why) && !d.owned?.has(why)) d.log("error", event, { ...fields, why });
    d.problems?.add(why); d.owned?.add(why);
  };
  const deposits = await d.buyer.holder_deposits();
  const waiting = [...new Set(deposits.map((x) => {
    const p = (x.phase.AwaitingProof ?? x.phase.Parked) as { inscription?: string } | undefined;
    return p && typeof p.inscription === "string" && ID.test(p.inscription) ? p.inscription : null;
  }).filter((x): x is string => x !== null))];
  if (!waiting.length) return false;
  const colls = await d.buyer.frog_collections(), pairs = pairListings(d.listings, colls);
  const live = d.listings.filter((l) => pairs.get(l.id)?.live).map((l) => ({ l, c: pairs.get(l.id)! }));
  const trees = new Map<number, MemberTree>();
  for (const { l, c } of live) {
    try { trees.set(l.id, d.members.tree(l, c.root)); } catch (err) { report((err as Error).message, "frog-proof-members", { coll: c.coll }); }
  }
  let kept = false;
  for (const id of waiting) {
    const holders = live.filter(({ l }) => trees.get(l.id)?.has(id));
    if (!holders.length) continue;
    // audit C4-10: an inscription in more than one live collection's members gets no proof: the runner does not pick the collection
    if (new Set(holders.map((h) => h.c.coll)).size > 1) {
      const names = holders.map((h) => `${h.l.slug} (coll ${h.c.coll})`).join(" and ");
      report(`${id} is in the members of ${names}: no proof handed in (the runner does not pick the collection)`, "frog-proof-ambiguous",
        { inscription: id, slugs: holders.map((h) => h.l.slug), colls: holders.map((h) => h.c.coll) });
      continue;
    }
    const { l, c } = holders[0], t = trees.get(l.id)!;
    const key = `${c.coll}:${id}`;
    if ((handed.get(key) ?? -Infinity) + PROOF_AGAIN_S > now()) continue;
    handed.set(key, now());
    try {
      const r = await d.buyer.frog_proof(c.coll, id, t.proof(id).map((h) => h.slice(2)));
      if ("Ok" in r) { kept = true; d.log("info", "frog-proof-handed-in", { coll: c.coll, slug: l.slug, inscription: id, answer: r.Ok }); }
      else d.log("warn", "frog-proof-refused", { coll: c.coll, slug: l.slug, inscription: id, error: r.Err });
    } catch (err) { d.log("warn", "frog-proof-call-failed", { coll: c.coll, inscription: id, error: String((err as Error).message).slice(0, 200) }); }
  }
  return kept;
}

/**
 * The buying steps, one Bitcoin hand-in per cycle across every collection (the float is shared: two hand-ins in one cycle could name the
 * same inputs): the live collections in turn, starting after the one that acted last; for each, `step(entry, coll)` (Ordinals Wallet,
 * then UniSat when it handed nothing in). A collection the canister lacks or keeps dormant is skipped.
 */
export function bitcoinStepsOnce(o: { entries: readonly InscEntry[]; colls: () => Promise<FrogColl[]>; log: Log;
  step: (e: InscEntry, coll: number) => Promise<boolean>; problems?: Set<string> }): () => Promise<boolean> {
  let next = 0;
  // audit C4-06: the problems this step added at its last pass; each pass removes them and adds what still holds (the set is shared)
  let owned = new Set<string>();
  return async () => {
    const colls = await o.colls(), by = entryColls(o.entries, colls);
    const before = owned;
    owned = new Set();
    for (const k of before) o.problems?.delete(k);
    for (const e of o.entries) {
      if (by.has(e.slug) || !e.members.length) continue;
      const why = `${e.slug}: no canister collection has the root of ${e.membersFile} (frog_collections()): its buying step is idle`;
      if (!before.has(why)) o.log("warn", "inscriptions-unpaired", { slug: e.slug, why });
      o.problems?.add(why); owned.add(why);
    }
    const live = o.entries.filter((e) => by.get(e.slug)?.live);
    for (let k = 0; k < live.length; k++) {
      const i = (next + k) % live.length, e = live[i];
      let acted = false;
      try { acted = await o.step(e, by.get(e.slug)!.coll); } catch (err) { o.log("warn", "step-failed", { step: "inscriptions", slug: e.slug, error: String((err as Error).message).slice(0, 200) }); }
      if (acted) { next = i + 1; return true; }
    }
    return false;
  };
}
