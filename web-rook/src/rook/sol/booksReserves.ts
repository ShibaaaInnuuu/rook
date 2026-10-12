/**
 * The proof of reserves on the books home, as the Reserves page draws it (docs/v9/canister-only-solana.md §4.4, §10.2): per collection
 * the canister's books (`books_view`) against Solana at finalized, with the reconciler's rules (ops/src/booksreserves.ts), read-only:
 *  - the wX mint's supply equals the ledger's `expected_supply = (held − open_tickets)·10⁹ + sol_backed − module_units`;
 *  - `M[i]`'s lamports above its rent cover `owed = backing + reserve + surplus + revenue`; more is "unbooked" (never counted);
 *  - each coin vault (`K_coin[c]`'s wX account) holds at least its book `wx_in − wx_paid`;
 *  - the books list as many items as they count held.
 * The items' custody (an inscription at the custody taproot, an Ethereum NFT owned by the custody address) is the reconciler's check,
 * with two sources each: the page shows every item's custody proof and says who located it. Pure: test/booksreserves.test.ts is its spec.
 */
import type { BooksViewV, CollectionViewV, ReportV, VaultViewV } from "./canister";

export type ReserveStatus = "green" | "red" | "unverified";
/** What the chain said at finalized: null where a read failed or an account does not exist. */
export type ChainSide = { supply: bigint | null; moduleLamports: bigint | null; rent: bigint; vaults: Map<string, bigint | null> };
export type VaultRow = { coin: string; vault: string | null; book: bigint; onChain: bigint | null; stage: string; status: ReserveStatus };
export type Equation = {
  coll: number; wxMint: string; module: string | null; status: ReserveStatus; reasons: string[]; halted: string | null;
  held: bigint; openTickets: bigint; itemsListed: number; solBacked: bigint; moduleUnits: bigint; expected: bigint | null; supply: bigint | null; supplyHolds: boolean | null;
  owed: bigint; balance: bigint | null; unbooked: bigint; backingHolds: boolean | null; vaults: VaultRow[];
  /** `backing` per SOL-backed unit, lamports per whole wX: what a burn can be paid at most from the pot. */
  backingPerUnit: bigint | null;
};

export const ONE = 1_000_000_000n;
export const expectedSupply = (v: Pick<BooksViewV, "held" | "open_tickets" | "sol_backed" | "module_units">): bigint | null => {
  const u = (v.held - v.open_tickets) * ONE + v.sol_backed - v.module_units;
  return v.held < v.open_tickets || u < 0n ? null : u;
};

/** The equation of one collection, with the reasons it is not green (red: a rule fails; unverified: a read is missing, never green). */
export function collectionEquation(v: BooksViewV, module: string | null, vaults: VaultViewV[], chain: ChainSide): Equation {
  const reasons: string[] = [], soft: string[] = [];
  const halted = v.halted.length ? v.halted[0] : null;
  if (halted) reasons.push(`stopped by an incident in the canister: ${halted.slice(0, 200)}`);
  const expected = v.expected_supply.length ? v.expected_supply[0] : null;
  const supply = chain.supply;
  if (supply === null) soft.push(`the wX mint ${v.wx_mint} could not be read`);
  else if (expected === null) reasons.push(`the canister's books are inconsistent (no expected supply): held ${v.held}, open tickets ${v.open_tickets}, module units ${v.module_units}, SOL-backed ${v.sol_backed}`);
  else if (supply !== expected) reasons.push(`wX supply ${supply} on chain is not the books' expected supply (held ${v.held} − open tickets ${v.open_tickets}) × 10⁹ + SOL-backed ${v.sol_backed} − module units ${v.module_units} = ${expected}`);
  if (BigInt(v.items.length) !== v.held) reasons.push(`the books count ${v.held} held items and list ${v.items.length}`);
  let balance: bigint | null = null, unbooked = 0n, backingHolds: boolean | null = null;
  if (!module) { if (v.owed > 0n) soft.push(`the books owe ${v.owed} lamports at M[${v.coll}] but the canister names no module address for it yet: the balance is unverified`); else backingHolds = true; }
  else if (chain.moduleLamports === null) soft.push(`the module ${module} could not be read`);
  else {
    balance = chain.moduleLamports - chain.rent;
    if (balance < 0n) balance = 0n;
    if (balance < v.owed) { reasons.push(`M[${v.coll}] holds ${balance} lamports above its rent; the books say it must hold ${v.owed} (backing ${v.backing} + reserve ${v.reserve} + surplus ${v.surplus} + revenue ${v.revenue})`); backingHolds = false; }
    else { backingHolds = true; unbooked = balance - v.owed; }
  }
  const rows: VaultRow[] = vaults.map((x) => {
    const addr = x.vault.length ? x.vault[0] : null, book = x.wx_in - x.wx_paid;
    if (!addr) return { coin: x.coin, vault: null, book, onChain: null, stage: x.stage, status: book > 0n ? "unverified" : "green" };
    const onChain = chain.vaults.get(addr) ?? null;
    if (onChain === null) { soft.push(`coin ${x.coin}'s vault ${addr} could not be read (its book: ${book} wX base units)`); return { coin: x.coin, vault: addr, book, onChain, stage: x.stage, status: "unverified" }; }
    if (onChain < book) { reasons.push(`coin ${x.coin}'s vault ${addr} holds ${onChain} wX base units, its book says ${book}`); return { coin: x.coin, vault: addr, book, onChain, stage: x.stage, status: "red" }; }
    return { coin: x.coin, vault: addr, book, onChain, stage: x.stage, status: "green" };
  });
  const status: ReserveStatus = reasons.length ? "red" : soft.length ? "unverified" : "green";
  return {
    coll: v.coll, wxMint: v.wx_mint, module, status, reasons: [...reasons, ...soft], halted,
    held: v.held, openTickets: v.open_tickets, itemsListed: v.items.length, solBacked: v.sol_backed, moduleUnits: v.module_units, expected, supply,
    supplyHolds: supply === null || expected === null ? null : supply === expected,
    owed: v.owed, balance, unbooked, backingHolds, vaults: rows,
    backingPerUnit: v.sol_backed > 0n ? (v.backing * ONE) / v.sol_backed : null,
  };
}

/** The page's one-line verdict over every collection. */
export function overall(eqs: Equation[]): ReserveStatus {
  if (eqs.some((e) => e.status === "red")) return "red";
  if (eqs.some((e) => e.status === "unverified") || !eqs.length) return "unverified";
  return "green";
}

/** The canister's own last reconciliation (`desk_status.report`), summarized: its time, what it checked and what it found. */
export function reportSummary(r: ReportV | null, lastReconcile: bigint, now: number): { at: number; stale: boolean; line: string; tone: ReserveStatus } {
  const at = Number(r?.at ?? lastReconcile);
  const ageS = at ? now - at : Infinity;
  const stale = ageS > 3 * 3_600;
  if (!r) return { at, stale, line: at ? "The canister has reconciled but keeps no report of it." : "The canister has not reconciled its books against Solana yet.", tone: "unverified" };
  const halted = Array.from(r.halted);
  const bad = r.findings.filter((f) => !("HolderBurn" in f.kind) && !("Unbooked" in f.kind)).length;
  const tone: ReserveStatus = bad || r.unexplained.length || halted.length ? "red" : stale ? "unverified" : "green";
  const parts = [`${r.checked} collection${r.checked === 1 ? "" : "s"} checked`, `${r.memos_seen} Rook memo${r.memos_seen === 1 ? "" : "s"} read from the chain`];
  if (r.unexplained.length) parts.push(`${r.unexplained.length} unexplained (a mint among them is SEV1)`);
  if (r.findings.length) parts.push(`${r.findings.length} finding${r.findings.length === 1 ? "" : "s"}`);
  if (halted.length) parts.push(`stopped collection${halted.length === 1 ? "" : "s"}: ${halted.join(", ")}`);
  return { at, stale, line: parts.join(", ") + ".", tone };
}

/** The module address of a collection, from `coins_collections` (empty before the collection was ever looked at). */
export const moduleOf = (cols: CollectionViewV[], coll: number): string | null => cols.find((c) => c.index === coll)?.module || null;

/** The accounts the chain side needs, in one `getMultipleAccounts` order: the wX mint, the module, then each vault. */
export function chainAddresses(v: BooksViewV, module: string | null, vaults: VaultViewV[]): string[] {
  return [v.wx_mint, ...(module ? [module] : []), ...vaults.flatMap((x) => (x.vault.length ? [x.vault[0]] : []))];
}

/** The collection's backing figures for its page: what is behind the wX in circulation. */
export function backingFigures(v: BooksViewV): { circulating: bigint; nftBacked: bigint; solBacked: bigint; itemsHeld: bigint; ticketsOpen: bigint; acceptedFloor: bigint; mintPrice: bigint | null; burnPaysOne: bigint | null; reserveShare: number } {
  const circulating = expectedSupply(v) ?? 0n;
  const nftBacked = (v.held > v.open_tickets ? v.held - v.open_tickets : 0n) * ONE;
  const total = v.backing + v.reserve;
  return { circulating, nftBacked, solBacked: v.sol_backed, itemsHeld: v.held, ticketsOpen: v.open_tickets, acceptedFloor: v.accepted,
    mintPrice: v.mint_price.length ? v.mint_price[0] : null, burnPaysOne: v.burn_pays_one.length ? v.burn_pays_one[0] : null,
    reserveShare: total > 0n ? Number((v.reserve * 10_000n) / total) / 10_000 : 0 };
}

/** A coin's redemption value: `⌊vault × a ÷ supply⌋` wX for `a` coins (§7, rounded down, never overpaying). */
export const redemptionValue = (vaultWx: bigint, coins: bigint, supply: bigint): bigint => (supply > 0n && coins > 0n ? (vaultWx * coins) / supply : 0n);
