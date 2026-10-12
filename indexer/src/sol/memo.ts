/**
 * The canister's memo ledger as the indexer reads it (canister-only design §4.4 rule 4, §10; the writer is
 * autobuyer/canister-rs/src/chainlink/memo.rs): every Rook transaction that changes a supply or a backing balance carries
 * `rook:v1:<coll>:<kind>:<entry id>[:i<intent>]` as an SPL Memo. `<coll>` is the canister's collection index, `-` for a transaction
 * that touches no collection (an intent's refund); `<kind>` is an `OpKind` tag (chainlink/ledger.rs): mint, burn, refund, redeem,
 * remint, prize, sweep, coin, close, other; the entry id is the link's id in the canister's `ledger_log`.
 *
 * A memo is a LABEL. Anyone can write one in their own transaction, and the canister's own memo says which link a transaction was
 * meant as, never that money moved. So an event records only what the transaction's balances say (decode.ts: post − pre lamports
 * per account, post − pre amount per token account) next to the label; a memo with no balance change behind it is an event whose
 * deltas are empty, and a transfer without a memo is not an event here at all (the reconciler's supply check catches those).
 */
import type { DecodedTx } from "./decode.js";

export const PREFIX = "rook:v1:";
/** The `OpKind` tags the canister writes. A memo with another lowercase tag still parses (the canister may add one), and is kept as it is. */
export const KINDS = ["mint", "burn", "refund", "redeem", "remint", "prize", "sweep", "coin", "close", "other"] as const;
/** The kinds whose transaction mints wX (an unexplained one on chain is the reconciler's SEV1). */
export const MINTING_KINDS = ["mint", "remint", "prize"] as const;

export type MemoRef = { coll: number | null; kind: string; id: number; intent: number | null };

const INT = /^\d{1,15}$/;
/** One memo as written: null if it is not a Rook v1 memo (the same rules as memo.rs `parse`). */
export function parseMemo(text: string): MemoRef | null {
  const t = text.trim();
  if (!t.startsWith(PREFIX)) return null;
  const parts = t.slice(PREFIX.length).split(":");
  if (parts.length < 3 || parts.length > 4) return null;
  const [c, kind, id, i] = parts;
  let coll: number | null = null;
  if (c !== "-") { if (!/^\d{1,5}$/.test(c) || Number(c) > 65_535) return null; coll = Number(c); }
  if (!/^[a-z]+$/.test(kind) || !INT.test(id)) return null;
  let intent: number | null = null;
  if (i !== undefined) { if (!i.startsWith("i") || !INT.test(i.slice(1))) return null; intent = Number(i.slice(1)); }
  return { coll, kind, id: Number(id), intent };
}
/** The Rook memos in what `getSignaturesForAddress` reports as a signature's memo: `[<len>] <text>`, several joined by `; `. */
export function parseRpcMemo(rpcMemo: string | null | undefined): MemoRef[] {
  if (!rpcMemo) return [];
  return rpcMemo.split("; ").map((part) => { const m = /^\[(\d+)\] ([\s\S]*)$/.exec(part.trim()); return m ? m[2] : part.trim(); }).map(parseMemo).filter((x): x is MemoRef => x !== null);
}
/** True when a signature's RPC memo carries a Rook memo: a desk stream fetches only those transactions. An RPC that leaves the field out (undefined) is not trusted to have looked. */
export const carriesRookMemo = (rpcMemo: string | null | undefined): boolean => rpcMemo === undefined || parseRpcMemo(rpcMemo).length > 0;

export type DeskDeltas = { lamports: [address: string, delta: string][]; tokens: [owner: string | null, mint: string, delta: string][] };
export type DeskEventRow = { n: number; coll: number | null; kind: string; linkId: number; intent: number | null; memo: string;
  wxMint: string | null; wxDelta: bigint | null; payerLamports: bigint; moduleLamports: bigint | null; deltas: DeskDeltas };
export type Knowledge = { wxMintOf: (coll: number) => string | null; moduleOf: (coll: number) => string | null; isWxMint: (mint: string) => boolean };

/**
 * The desk events of one decoded transaction: one per Rook memo it carries, each with the same balance deltas (they are the
 * transaction's). `wxDelta` is the net change of the collection's wX across every token account the transaction touched: a MintTo
 * adds to one account with no counterpart, a Burn removes with none, a transfer sums to zero. So it is the supply change the
 * transaction made, read from balances and not from the memo's kind.
 */
export function deskEvents(d: DecodedTx, k: Knowledge): DeskEventRow[] {
  const out: DeskEventRow[] = [];
  if (!d.ok) return out;
  const byMint = new Map<string, bigint>();
  for (const t of d.tokens) if (k.isWxMint(t.mint)) byMint.set(t.mint, (byMint.get(t.mint) ?? 0n) + t.delta);
  const lam = new Map(d.lamports.map((l) => [l.address, l.delta]));
  const deltas: DeskDeltas = { lamports: d.lamports.map((l) => [l.address, String(l.delta)]), tokens: d.tokens.map((t) => [t.owner, t.mint, String(t.delta)]) };
  let n = 0;
  for (const text of d.memos) {
    const m = parseMemo(text);
    if (!m) continue;
    // the collection's wX when the memo names a collection the indexer knows; else the one wX the transaction moved, if it is one
    let wxMint = m.coll !== null ? k.wxMintOf(m.coll) : null;
    if (!wxMint && byMint.size === 1) wxMint = [...byMint.keys()][0];
    const module = m.coll !== null ? k.moduleOf(m.coll) : null;
    out.push({ n: n++, coll: m.coll, kind: m.kind, linkId: m.id, intent: m.intent, memo: text.trim(), wxMint, wxDelta: wxMint ? (byMint.get(wxMint) ?? 0n) : null,
      payerLamports: lam.get(d.feePayer) ?? 0n, moduleLamports: module ? (lam.get(module) ?? 0n) : null, deltas });
  }
  return out;
}
