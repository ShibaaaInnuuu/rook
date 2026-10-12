import { encodeAbiParameters, keccak256, concat, type Hex } from "viem";
import type { Address, RaffleDraw, RaffleStage } from "../../types.js";
import { hostRefusal, jsonCapped, TooLarge, type Fetch } from "../outbound.js";
import type { WrapperEvent } from "./ledger.js";

/**
 * docs/audit/2026-09-26/fixes-B-C-keeper-draws.md (C-02, C-03, C-04, E-02) and fixes-W2-site.md §3: a raffle vault's draws, folded
 * from its events, and the published ticket list checked against the root the vault holds. Pure but for `checkPublished`, which
 * fetches the list through the E-12 guard. It never trusts a proof from the list: it rebuilds the tree (the sentinel included) and
 * computes the winner's proof itself, and only from a list that reproduces the on-chain root.
 */
export const OPEN_TTL_S = 6 * 3_600, PIN_TIMEOUT_S = 7 * 86_400, VOID_AFTER_S = 7 * 86_400;
export const DRAWS_KEPT = 30;
/** The published list's cap: a weighted list of every holder of one coin, with proofs, is well under this. */
export const LIST_MAX_BYTES = 8 * 1024 * 1024;
const ZERO = "0x0000000000000000000000000000000000000000";

type Mut = Omit<RaffleDraw, "voidableAt"> & { voidableAt: number | null };

/** Every draw of `vault`, newest first, the latest `DRAWS_KEPT`. `published` and `claim` are filled by `withPublished`. */
export function foldDraws(vault: Address, events: WrapperEvent[]): RaffleDraw[] {
  const draws = new Map<number, Mut>();
  // review2 N-2: `SeedCarried(id, origin, seedRound)` is emitted just before its draw's `DrawAnnounced`, in the same transaction
  const carried = new Map<number, number>();
  const sorted = [...events].sort((a, b) => a.block - b.block || a.logIndex - b.logIndex);
  for (const e of sorted) {
    const a = e.args, id = Number(a.drawId);
    if (!Number.isInteger(id)) continue;
    let d = draws.get(id);
    if (e.name === "SeedCarried") { carried.set(id, Number(a.originId)); continue; }
    if (e.name === "DrawAnnounced") {
      // the snapshot time is left to `SnapshotFixed`: a carried draw's is its origin's, counted back from the ORIGIN's announcement
      // (SweepVault.snapshotTimeOf), so it is never computed here from this draw's `announcedAt`
      d = { id, vault, coin: String(a.coin) as Address, stage: "announced", seedRound: Number(a.seedRound), announcedAt: e.ts, snapshotAt: null,
        root: null, totalTickets: null, uri: null, challengeEnds: null, published: null, round: null, pinnedAt: null, winningTicket: null, resolvedAt: null,
        winner: null, claim: null, voidableAt: null, tx: e.tx, seedOrigin: carried.get(id) ?? null, seedPassedOn: false };
      draws.set(id, d);
      continue;
    }
    if (!d) continue;                               // an event of a draw announced before the read's first block: not ours to guess
    d.tx = e.tx;
    const stage = (s: RaffleStage) => { d!.stage = s; };
    switch (e.name) {
      case "SnapshotFixed": d.snapshotAt = Number(a.snapshotAt); break;
      case "DrawOpened": stage("open"); d.root = String(a.root) as Hex; d.totalTickets = String(a.totalTickets); d.uri = String(a.uri); d.challengeEnds = Number(a.challengeEnds); break;
      // review2 N-2: a cancel (Announced or Open, always before the pin) and a void of a draw never opened leave its seed round and
      // snapshot window to the coin's next announcement; a void of a pinned or resolved draw does not
      case "DrawCancelled": stage("cancelled"); d.seedPassedOn = true; break;
      case "DrawPinned": stage("pinned"); d.round = Number(a.round); d.pinnedAt = e.ts; break;
      case "DrawResolved": stage("resolved"); d.winningTicket = String(a.winningTicket); d.resolvedAt = e.ts; break;
      case "Won": stage("claimed"); d.winner = String(a.holder) as Address; break;
      case "DrawVoided": d.seedPassedOn = d.stage === "announced"; stage("voided"); break;
      // fixes-R3-carry: no eligible holder at the snapshot; no winner, the unit back to the coin, and the seed used up (not passed on)
      case "DrawEmpty": stage("empty"); d.snapshotAt = d.snapshotAt ?? Number(a.snapshotAt); break;
    }
  }
  for (const d of draws.values()) d.voidableAt = voidableAt(d);
  return [...draws.values()].sort((x, y) => y.id - x.id).slice(0, DRAWS_KEPT);
}

/** SweepVault.voidDraw's three clocks (the chain's time; the parent chain's half can add minutes). */
export function voidableAt(d: Pick<RaffleDraw, "stage" | "announcedAt" | "pinnedAt" | "resolvedAt">): number | null {
  if (d.stage === "announced") return d.announcedAt === null ? null : d.announcedAt + OPEN_TTL_S;
  if (d.stage === "pinned") return d.pinnedAt === null ? null : d.pinnedAt + PIN_TIMEOUT_S;
  if (d.stage === "resolved") return d.resolvedAt === null ? null : d.resolvedAt + VOID_AFTER_S;
  return null;
}

// ---- the tree, exactly as SweepVault.leaf and tools/snapshot build it (docs/snapshot-v1.md, v2 for SweepVault draws) -------------

/** `SweepVault.leaf(holder, first, end)`: keccak256(keccak256(abi.encode(holder, first, end))). */
export const ticketLeaf = (holder: string, first: bigint, end: bigint): Hex =>
  keccak256(keccak256(encodeAbiParameters([{ type: "address" }, { type: "uint256" }, { type: "uint256" }], [holder as Address, first, end])));
/** The sentinel leaf that commits the count and the time: `leaf(address(0), totalTickets, snapshotAt)` (C-03). */
export const sentinelLeaf = (totalTickets: bigint, snapshotAt: bigint): Hex => ticketLeaf(ZERO, totalTickets, snapshotAt);
const pairHash = (a: Hex, b: Hex): Hex => (BigInt(a) < BigInt(b) ? keccak256(concat([a, b])) : keccak256(concat([b, a])));

/** Sorted leaves, neighbours paired left to right, an odd node moved up unchanged (OpenZeppelin's commutative hash). */
export function tree(leaves: readonly Hex[]): { root: Hex; proofOf: (leaf: Hex) => Hex[] } {
  if (!leaves.length) throw new Error("an empty list has no root");
  const layers: Hex[][] = [[...leaves].map((l) => l.toLowerCase() as Hex).sort((a, b) => (BigInt(a) < BigInt(b) ? -1 : BigInt(a) > BigInt(b) ? 1 : 0))];
  while (layers[layers.length - 1].length > 1) {
    const l = layers[layers.length - 1], up: Hex[] = [];
    for (let i = 0; i < l.length; i += 2) up.push(i + 1 < l.length ? pairHash(l[i], l[i + 1]) : l[i]);
    layers.push(up);
  }
  return {
    root: layers[layers.length - 1][0],
    proofOf: (leaf) => {
      let i = layers[0].indexOf(leaf.toLowerCase() as Hex);
      if (i < 0) throw new Error("not a leaf of this tree");
      const proof: Hex[] = [];
      for (let k = 0; k < layers.length - 1; k++) { const sib = i ^ 1; if (sib < layers[k].length) proof.push(layers[k][sib]); i >>= 1; }
      return proof;
    },
  };
}
/** What `MerkleProof.verify` computes. */
export const verifyProof = (proof: readonly Hex[], root: Hex, leaf: Hex): boolean => proof.reduce<Hex>((h, p) => pairHash(h, p), leaf).toLowerCase() === root.toLowerCase();

type Range = { holder: Address; first: bigint; end: bigint };
/** The weighted entries of a published list (`tools/snapshot`'s `entries[]`: holder, first, end); a zero holder (a sentinel written as an entry) is skipped.
 *  `empty` (fixes-R3-carry: a draw opened with no tickets): a list with no holder is what is expected, and is returned as `[]`. */
export function rangesOf(list: unknown, empty = false): Range[] | string {
  const entries = (list as { entries?: unknown } | null)?.entries;
  if (!Array.isArray(entries) || (!entries.length && !empty)) return "the list has no entries";
  const out: Range[] = [];
  for (const x of entries as Record<string, unknown>[]) {
    const holder = String(x?.holder ?? ""), first = String(x?.first ?? ""), end = String(x?.end ?? "");
    if (!/^0x[0-9a-fA-F]{40}$/.test(holder) || !/^\d{1,78}$/.test(first) || !/^\d{1,78}$/.test(end)) return "an entry is not a weighted range (holder, first, end)";
    if (holder.toLowerCase() === ZERO) continue;
    out.push({ holder: holder as Address, first: BigInt(first), end: BigInt(end) });
  }
  return out.length || empty ? out : "the list has no holder";
}

/** The list against the draw: does it (plus the sentinel) reproduce the root, and if the draw is resolved, the winner's claim. */
export function judgeList(d: RaffleDraw, list: unknown): Pick<RaffleDraw, "published" | "claim"> {
  if (!d.root || d.totalTickets === null || d.snapshotAt === null) return { published: { status: "unreadable", why: "the draw names no root, count or snapshot time" }, claim: null };
  const ranges = rangesOf(list, d.totalTickets === "0");
  if (typeof ranges === "string") return { published: { status: "unreadable", why: ranges }, claim: null };
  const leaves = [...ranges.map((r) => ticketLeaf(r.holder, r.first, r.end)), sentinelLeaf(BigInt(d.totalTickets), BigInt(d.snapshotAt))];
  const t = tree(leaves);
  if (t.root.toLowerCase() !== d.root.toLowerCase()) return { published: { status: "differs", why: `the list rebuilds to ${t.root}, not the vault's root` }, claim: null };
  const published = { status: "matches" as const, why: null };
  if (d.stage !== "resolved" || d.winningTicket === null) return { published, claim: null };
  const w = BigInt(d.winningTicket);
  const win = ranges.find((r) => r.first <= w && w < r.end && r.end <= BigInt(d.totalTickets!));
  if (!win) return { published, claim: null };
  const leaf = ticketLeaf(win.holder, win.first, win.end), proof = t.proofOf(leaf);
  if (!verifyProof(proof, d.root, leaf)) return { published, claim: null };
  return { published, claim: { holder: win.holder, first: String(win.first), end: String(win.end), proof } };
}

/** `ipfs://` on the public gateways in turn, https as it is; anything else is not fetched. */
export const listUrls = (uri: string, gateways: readonly string[]): string[] =>
  uri.startsWith("ipfs://") ? gateways.map((g) => g + uri.slice(7).replace(/^ipfs\//, "")) : /^https:\/\//.test(uri) ? [uri] : [];

/** Fetch the published list at `uri` (E-12: name rules first, the guarded transport in production, 8 MB cap) and judge it. */
export async function checkPublished(d: RaffleDraw, fetchImpl: Fetch, gateways: readonly string[]): Promise<Pick<RaffleDraw, "published" | "claim">> {
  if (!d.uri) return { published: { status: "unreadable", why: "no list was published" }, claim: null };
  const urls = listUrls(d.uri, gateways);
  if (!urls.length) return { published: { status: "unreadable", why: "the list's address is neither https nor ipfs" }, claim: null };
  let why = "no answer";
  for (const url of urls) {
    const refused = hostRefusal(url);
    if (refused) { why = `refused: ${refused}`; continue; }
    try {
      const r = await fetchImpl(url, { method: "GET", headers: { accept: "application/json", "user-agent": "rook-indexer/1 (draw list check, read-only)" }, redirect: "manual", signal: AbortSignal.timeout(20_000) });
      if (!r.ok) { await r.body?.cancel().catch(() => undefined); why = `HTTP ${r.status}`; continue; }
      return judgeList(d, await jsonCapped(r, LIST_MAX_BYTES));
    } catch (err) { why = err instanceof TooLarge ? "the list is larger than 8 MB" : `no answer: ${String((err as Error)?.name ?? "error")}`; }
  }
  return { published: { status: "unreadable", why }, claim: null };
}
