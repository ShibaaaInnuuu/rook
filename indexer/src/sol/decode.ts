/**
 * One confirmed transaction (getTransaction, encoding jsonParsed, maxSupportedTransactionVersion 0) → what the indexer keeps:
 * DBC swaps, DAMM v2 swaps, DBC pool creations (with the name/symbol/uri from the instruction), curve completions, migrations;
 * and, for the canister's own transactions (W13, canister-only design §4.4 rule 4), the SPL Memo texts and what the balances say
 * moved (post − pre lamports per account, post − pre amount per token account). Pure: test/sol_decode.test.ts runs it over
 * recorded mainnet transactions.
 */
import { b58decode, startsWith, u32At } from "./bytes.js";
import { DAMM_V2_PROGRAM, DBC_PROGRAM, DISC, decodeEvent, type CurveCompleteEvent, type DammSwapEvent, type DbcSwapEvent, type InitPoolEvent } from "./dbc.js";

export type ParsedIx = { programId: string; accounts?: string[]; data?: string; parsed?: unknown; program?: string; stackHeight?: number | null };
export type TokenBalance = { accountIndex: number; mint: string; owner?: string; programId?: string; uiTokenAmount: { amount: string; decimals: number } };
export type ParsedTx = {
  slot: number; blockTime: number | null;
  transaction: { signatures: string[]; message: { accountKeys: ({ pubkey: string; signer: boolean; writable: boolean; source?: string } | string)[]; instructions: ParsedIx[] } };
  meta: { err: unknown; fee: number; innerInstructions?: { index: number; instructions: ParsedIx[] }[] | null; preBalances: number[]; postBalances: number[];
    preTokenBalances?: TokenBalance[] | null; postTokenBalances?: TokenBalance[] | null; logMessages?: string[] | null; loadedAddresses?: { writable: string[]; readonly: string[] } } | null;
};

export type SwapCtx = { programId: string; accounts: string[] };
export type DecodedSwap = { n: number; ev: DbcSwapEvent | DammSwapEvent; ix: SwapCtx | null; trader: string };
export type DecodedInit = { ev: InitPoolEvent; name: string | null; symbol: string | null; uri: string | null };
export type DecodedMigration = { virtualPool: string; dammPool: string; baseMint: string; config: string; version: "damm-v2" | "damm-v1" };
/** A lamport balance that changed (post − pre); a token balance that changed, by token account, with the owner the RPC reports. */
export type LamportDelta = { address: string; delta: bigint };
export type TokenDelta = { account: string; owner: string | null; mint: string; delta: bigint };
export type DecodedTx = { signature: string; slot: number; blockTime: number | null; ok: boolean; feePayer: string; signers: string[]; fee: number | null;
  swaps: DecodedSwap[]; inits: DecodedInit[]; completes: CurveCompleteEvent[]; migrations: DecodedMigration[];
  memos: string[]; lamports: LamportDelta[]; tokens: TokenDelta[] };

/** SPL Memo v2 and v1: `jsonParsed` gives `{ program: "spl-memo", parsed: "<text>" }`; a raw one is base58 of the UTF-8 text. */
export const MEMO_PROGRAMS = ["MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr", "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo"];

const keyOf = (k: ParsedTx["transaction"]["message"]["accountKeys"][number]) => (typeof k === "string" ? k : k.pubkey);
const dataOf = (ix: ParsedIx): Uint8Array | null => { if (typeof ix.data !== "string") return null; try { return b58decode(ix.data); } catch { return null; } };

/** A coin's name or symbol as shown: no control characters and no bidi overrides (a "‮" name would reverse the page's text), trimmed. */
export const cleanText = (s: string | null, max = 64): string | null => {
  if (s === null) return null;
  const t = s.replace(/[\u0000-\u001f\u007f-\u009f\u200b-\u200f\u202a-\u202e\u2060-\u2069\ufeff]/g, "").trim().slice(0, max);
  return t || null;
};
/** Borsh string at `at` → [string, next offset]; null when out of range. */
function str(d: Uint8Array, at: number): [string, number] | null {
  if (at + 4 > d.length) return null;
  const n = u32At(d, at);
  if (n > 2048 || at + 4 + n > d.length) return null;
  return [new TextDecoder().decode(d.subarray(at + 4, at + 4 + n)).replace(/\0+$/g, "").trim(), at + 4 + n];
}

export function decodeTx(tx: ParsedTx): DecodedTx {
  const msg = tx.transaction.message, keys = msg.accountKeys.map(keyOf);
  const signers = msg.accountKeys.filter((k) => typeof k !== "string" && k.signer).map(keyOf);
  const out: DecodedTx = { signature: tx.transaction.signatures[0], slot: tx.slot, blockTime: tx.blockTime ?? null, ok: !!tx.meta && tx.meta.err === null,
    feePayer: keys[0], signers, fee: tx.meta?.fee ?? null, swaps: [], inits: [], completes: [], migrations: [], memos: [], lamports: [], tokens: [] };
  if (!out.ok || !tx.meta) return out;
  // instructions in execution order: each outer one, then its inner ones
  const inner = new Map((tx.meta.innerInstructions ?? []).map((g) => [g.index, g.instructions]));
  const flat: ParsedIx[] = [];
  msg.instructions.forEach((ix, i) => { flat.push(ix); for (const x of inner.get(i) ?? []) flat.push(x); });

  // what moved, read from the transaction's own balances (a memo is a label, never evidence that anything moved)
  const pre = tx.meta.preBalances ?? [], post = tx.meta.postBalances ?? [];
  for (let i = 0; i < Math.min(pre.length, post.length, keys.length); i++) { const d = BigInt(post[i]) - BigInt(pre[i]); if (d !== 0n) out.lamports.push({ address: keys[i], delta: d }); }
  const tok = new Map<number, { pre: bigint; post: bigint; mint: string; owner: string | null }>();
  for (const b of tx.meta.preTokenBalances ?? []) tok.set(b.accountIndex, { pre: BigInt(b.uiTokenAmount.amount), post: 0n, mint: b.mint, owner: b.owner ?? null });
  for (const b of tx.meta.postTokenBalances ?? []) {
    const t = tok.get(b.accountIndex);
    if (t) { t.post = BigInt(b.uiTokenAmount.amount); t.owner ??= b.owner ?? null; }
    else tok.set(b.accountIndex, { pre: 0n, post: BigInt(b.uiTokenAmount.amount), mint: b.mint, owner: b.owner ?? null });
  }
  for (const [i, t] of [...tok.entries()].sort((a, b) => a[0] - b[0])) if (t.post !== t.pre && keys[i]) out.tokens.push({ account: keys[i], owner: t.owner, mint: t.mint, delta: t.post - t.pre });
  for (const ix of flat) {
    if (!MEMO_PROGRAMS.includes(ix.programId)) continue;
    if (typeof ix.parsed === "string") { out.memos.push(ix.parsed); continue; }
    const d = dataOf(ix);
    if (d) { try { out.memos.push(new TextDecoder("utf-8", { fatal: true }).decode(d)); } catch { /* not text: not a memo we read */ } }
  }

  let lastSwap: SwapCtx | null = null, lastInit: { name: string | null; symbol: string | null; uri: string | null } | null = null, n = 0;
  for (const ix of flat) {
    if (ix.programId !== DBC_PROGRAM && ix.programId !== DAMM_V2_PROGRAM) continue;
    const d = dataOf(ix);
    if (!d) continue;
    const ev = decodeEvent(ix.programId, d);
    if (ev) {
      if (ev.kind === "dbc-swap" || ev.kind === "damm-swap") {
        // the swap's user: DBC `payer` is account 9, DAMM v2 `payer` account 8; a non-signer there (a program PDA) falls back to the fee payer
        // DBC 0.2.x emits EvtSwap AND EvtSwap2 for one swap: the second event of the same swap instruction replaces the first (v2 says more)
        const prev = out.swaps.at(-1);
        if (prev && prev.ix !== null && prev.ix === lastSwap && prev.ev.kind === ev.kind && ev.kind === "dbc-swap") {
          if (ev.version === 2 || (prev.ev as DbcSwapEvent).version !== 2) prev.ev = ev;
          continue;
        }
        const ctx = lastSwap && lastSwap.programId === ix.programId ? lastSwap : null;
        const user = ctx ? ctx.accounts[ix.programId === DBC_PROGRAM ? 9 : 8] : undefined;
        out.swaps.push({ n: n++, ev, ix: ctx, trader: user && signers.includes(user) ? user : out.feePayer });
      } else if (ev.kind === "dbc-init") {
        out.inits.push({ ev, name: lastInit?.name ?? null, symbol: lastInit?.symbol ?? null, uri: lastInit?.uri ?? null });
        lastInit = null;
      } else if (ev.kind === "dbc-complete") out.completes.push(ev);
      continue;
    }
    const accts = ix.accounts ?? [];
    if (startsWith(d, DISC.swap) || startsWith(d, DISC.swap2)) { lastSwap = { programId: ix.programId, accounts: accts }; continue; }
    if (ix.programId !== DBC_PROGRAM) continue;
    if (startsWith(d, DISC.initSpl) || startsWith(d, DISC.init2022)) {
      const a = str(d, 8), b = a && str(d, a[1]), c = b && str(d, b[1]);
      lastInit = { name: cleanText(a ? a[0] : null), symbol: cleanText(b ? b[0] : null, 16), uri: c ? c[0].slice(0, 300) : null };
      continue;
    }
    if (startsWith(d, DISC.migrationDammV2) && accts.length >= 14) out.migrations.push({ virtualPool: accts[0], config: accts[2], dammPool: accts[4], baseMint: accts[13], version: "damm-v2" });
    else if (startsWith(d, DISC.migrateMeteoraDamm) && accts.length >= 10) out.migrations.push({ virtualPool: accts[0], config: accts[2], dammPool: accts[4], baseMint: accts[7], version: "damm-v1" });
  }
  return out;
}
