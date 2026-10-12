/**
 * The Rook canister as the site reads it (docs/v9/canister-only-solana.md §6.1 intents, §10.1 the public queries): the Candid types of
 * the views the pages draw, typed from autobuyer/canister-rs/etch_autobuyer.did in the style of ops/src/ic.ts and monitor/src/candid.mjs
 * (records list only the fields read, Candid's record subtyping skips the rest; every variant lists every tag, because an unknown tag
 * fails the decode). The `decode*` helpers decode saved reply bytes, so test/canister.test.ts holds them against the canister's own
 * fixtures (autobuyer/canister-rs/tests/fixtures/views/*.hex, written by its tests). Queries are anonymous; the two updates are too:
 * `open_intent` (§6.2: openings are free and rate-limited per caller and per recipient) and `coins_request_open` (operator decision
 * 2026-10-12: a pinned collection opens when someone first asks for it; idempotent).
 */
import { Actor, HttpAgent } from "@icp-sdk/core/agent";
import { IDL } from "@icp-sdk/core/candid";
import { CANISTER_SITE_CONFIG } from "../../config/canister";

const N = IDL.Nat64, Blob = IDL.Vec(IDL.Nat8);
const R = (o: Record<string, IDL.Type>) => IDL.Record(o);
const Text = IDL.Text;

// ---- the books (§4, §10.1 `books`, `ledger`) ---------------------------------------------------------------------------------------
const ItemView = R({ key: Text, vout: IDL.Nat32, inscription_id: Text, deposit: IDL.Bool, source_tx: Text, item_id: IDL.Nat32 });
const ListingKind = IDL.Variant({ SolNft: R({ collection: Blob, custodian: Blob }), Inscriptions: R({ custody: Text, membership_root: Blob }), EthNft: R({ collection: Blob, custody: Blob, chain_id: N }) });
const TicketSummary = R({ id: N, destination: Text, deliver_by: IDL.Int64, undeliverable: IDL.Bool, stage: Text, item_id: IDL.Nat32 });
const BooksView = R({ fresh: IDL.Bool, transit_units: N, last_good_at: IDL.Int64, raffle_prize: N, expected_supply: IDL.Opt(N), reserve_units: N, revenue: N, poster_paused: IDL.Bool,
  advanced: N, coll: IDL.Nat16, held: N, kind: ListingKind, owed: N, wx_mint: Text, items_arrived: N, volume_7d: N, reserve: N, surplus: N, raffle_pot: N, target: N,
  tickets_open: IDL.Vec(TicketSummary), posted_at: IDL.Int64, module_units: N, mint_price: IDL.Opt(N), escaped: IDL.Bool, burn_pays_one: IDL.Opt(N), sol_backed: N,
  halted: IDL.Opt(Text), open_tickets: N, items: IDL.Vec(ItemView), accepted: N, burned: N, ticket_count: N, paused: IDL.Bool, backing: N, item_count: IDL.Nat32 });
// ---- the coins (§7 `coin_vaults`) ----------------------------------------------------------------------------------------------------
const CollectionView = R({ coins: IDL.Nat32, backed: IDL.Nat32, wx_mint: Text, stage: Text, index: IDL.Nat32, module: Text, config: Text, symbol: Text });
/** Audit C4-12: one pending opening request (`coins_open_queue`), in queue order; `position` 0 opens next; `waiting_since` 0: the opening
 *  pass does not wait on it, else since when (unix seconds); `why` its last wait reason; `skipped`: passed over for later requests (C4-04). */
const OpenStatus = R({ index: IDL.Nat32, position: IDL.Nat32, asked_at: N, waiting_since: N, why: Text, skipped: IDL.Bool });
const VaultView = R({ vault: IDL.Opt(Text), coin: Text, coll: IDL.Nat32, pool: Text, wx_paid: N, stage: Text, backed_at: N, coins_burned: N, wx_in: N });
// ---- the pool desk (§5) ----------------------------------------------------------------------------------------------------------------
const DeskView = R({ hi: IDL.Int32, lo: IDL.Int32, wx: N, read_at: N, active: IDL.Int32, native: N, coll: IDL.Nat32, pair: Text, wsol: N, fee_lamports: N, swept: N, fee_units: N,
  pos_x: N, pos_y: N, position: Text, arb_profit: N, arbs_last_hour: IDL.Nat32 });
// ---- the intent desk (§6.1) ---------------------------------------------------------------------------------------------------------
const State = IDL.Variant({ Refunded: IDL.Null, Done: IDL.Null, Open: IDL.Null, Executing: IDL.Null, Funded: IDL.Null, Expired: IDL.Null });
const TxView = R({ at: N, op: Text, result: Text, signature: Text, link: N, memo: Text });
const IntentView = R({ id: N, txs: IDL.Vec(TxView), min_deposit: N, ticket: IDL.Opt(N), coll: IDL.Nat16, kind: Text, note: Text, recipient: Text, token_account: IDL.Opt(Text),
  deposit: N, created_at: N, refund_to: Text, refunding: IDL.Opt(Text), funded_at: IDL.Opt(N), minted_units: N, state: State, address: Text, attention: IDL.Bool,
  in_flight: IDL.Opt(Text), expires_at: N, paid_lamports: N });
const OpenKind = IDL.Variant({
  Burn: R({ coll: IDL.Nat16, min_out: IDL.Opt(N), recipient: Text, refund_to: IDL.Opt(Text) }),
  RedeemNft: R({ destination: Text, owner: Text, coll: IDL.Nat16 }),
  Mint: R({ coll: IDL.Nat16, min_out: IDL.Opt(N), recipient: Text }),
  RedeemCoin: R({ coin: Text, coll: IDL.Nat16, recipient: Text, refund_to: IDL.Opt(Text) }),
});
const MintQuote = R({ fee: N, min_deposit: N, coll: IDL.Nat16, ata_rent: N, deposit: N, to_module: N, unit_price: N, units: N });
const BurnQuote = R({ fee: N, net: N, coll: IDL.Nat16, paid: N, units: N, min_units: N });
const Quote = IDL.Variant({ Burn: BurnQuote, Mint: MintQuote });
const Opened = R({ id: N, fee: N, min_deposit: N, token_account: IDL.Opt(Text), quote: IDL.Opt(Quote), address: Text, expires_at: N });
const OpenResult = IDL.Variant({ Ok: Opened, Err: Text });
const MintQuoteResult = IDL.Variant({ Ok: MintQuote, Err: Text });
const BurnQuoteResult = IDL.Variant({ Ok: BurnQuote, Err: Text });
// ---- the desk's status and the canister's own reconciliation (§10.2) ---------------------------------------------------------------
const Incident = R({ at: N, coll: IDL.Opt(IDL.Nat16), link: IDL.Opt(N), what: Text });
const FindingKind = IDL.Variant({ SupplyMismatch: IDL.Null, Unbooked: IDL.Null, Invariant: IDL.Null, BooksUnbacked: IDL.Null, HolderBurn: IDL.Null });
const Finding = R({ coll: IDL.Nat16, chain: N, kind: FindingKind, note: Text, books: N });
const Report = R({ at: N, unexplained: IDL.Vec(Text), checked: IDL.Nat32, memos_seen: IDL.Nat32, findings: IDL.Vec(Finding), halted: IDL.Vec(IDL.Nat16) });
const DeskStatus = R({ incidents: IDL.Vec(Incident), report: IDL.Opt(Report), open: IDL.Nat32, collections: IDL.Nat32, last_reconcile: N, funded: IDL.Nat32, needing_attention: IDL.Nat32,
  open_links: IDL.Nat32, executing: IDL.Nat32, halted: IDL.Vec(IDL.Tuple(IDL.Nat16, Text)), coin_redeem: IDL.Bool, paused: IDL.Bool });
// ---- the treasury (§10.1 `treasury`): the fields the site draws ----------------------------------------------------------------------
const Owed = IDL.Variant({ Ops: R({ lamports: N }), Surplus: R({ lamports: N, coll: IDL.Nat32 }) });
const Treasury = R({ reserves_pot: N, unallocated: N, total_received: N, total_allocated: N, total_to_reserves: N, total_to_raffles: N, total_to_ops: N, reserves_bps: IDL.Nat16,
  ops_bps: IDL.Nat16, raffle_bps: IDL.Nat16, reserve_cap_bps: IDL.Nat32, collections: IDL.Vec(IDL.Nat16) });
const TreasuryStatus = R({ off: IDL.Bool, last: IDL.Vec(IDL.Tuple(Text, N, Text)), owed: IDL.Vec(Owed), treasury: Treasury });
const FeeBook = R({ to_desk_lamports: N, to_desk_burned_wx: N, lp_wx: N, cashed_lamports: N, curve_wx: N, cashed_wx: N, to_desk_wx: N });
const Wait = R({ key: Text, why: Text, tries: IDL.Nat32, retry_at: N, since: N });
// ---- the raffle (§9.3, §10.1 `raffle` + `raffle_snapshot`): the whole record, and a page of a published snapshot ------------------------
const RafflePhase = IDL.Variant({ Committed: IDL.Null, Idle: IDL.Null, Drawn: IDL.Null });
const Raffle = R({ key: Blob, pot: N, total_spent: N, prizes_funded: N, claimed_at: IDL.Int64, committed_at: IDL.Int64, ticket: N, root: Blob, total_weight: N, rounds_cancelled: N,
  winner: Blob, drawn_at: IDL.Int64, total_in: N, entries: IDL.Nat32, rounds_expired: N, drand_round: N, phase: RafflePhase, prize: N, prizes_claimed: N, round: N, snapshot_slot: N });
const PageLeaf = R({ weight: N, owner: Text, cum_start: N });
const PageCoin = R({ wx: N, circ: N, mint: Text, vault_wx: Text, supply: N, holders: IDL.Nat32 });
const PageExcluded = R({ owner: Text, coin: Text, account: Text, amount: N, reason: Text });
const SnapshotPage = R({ listing: Text, slot_lo: N, excluded: IDL.Vec(PageExcluded), coins: IDL.Vec(PageCoin), root: Text, total_weight: N, slot: N, offset: IDL.Nat32, entries: IDL.Nat32,
  leaves: IDL.Vec(PageLeaf), raffle: Text, taken_at: N, round: N });

export const canisterIdl: IDL.InterfaceFactory = () => IDL.Service({
  books_collections: IDL.Func([], [IDL.Vec(IDL.Tuple(IDL.Nat16, Text))], ["query"]),
  books_view: IDL.Func([IDL.Nat16], [IDL.Opt(BooksView)], ["query"]),
  coins_collections: IDL.Func([], [IDL.Vec(CollectionView)], ["query"]),
  coins_vaults: IDL.Func([IDL.Nat32], [IDL.Vec(VaultView)], ["query"]),
  coins_open_queue: IDL.Func([], [IDL.Vec(OpenStatus)], ["query"]),
  coins_log: IDL.Func([], [IDL.Vec(IDL.Tuple(IDL.Nat32, FeeBook)), IDL.Vec(Wait), IDL.Vec(IDL.Tuple(Text, N, Text)), IDL.Vec(IDL.Tuple(IDL.Nat32, N))], ["query"]),
  pool_desks: IDL.Func([], [IDL.Vec(DeskView)], ["query"]),
  desk_status: IDL.Func([], [DeskStatus], ["query"]),
  treasury_status: IDL.Func([], [TreasuryStatus], ["query"]),
  raffle_status: IDL.Func([IDL.Nat16], [IDL.Opt(Raffle)], ["query"]),
  raffle_snapshot: IDL.Func([Text, N, IDL.Nat32, IDL.Nat32], [IDL.Opt(SnapshotPage)], ["query"]),
  intent_status: IDL.Func([N], [IDL.Opt(IntentView)], ["query"]),
  quote_mint: IDL.Func([IDL.Nat16, N], [MintQuoteResult], ["query"]),
  quote_burn: IDL.Func([IDL.Nat16, N], [BurnQuoteResult], ["query"]),
  open_intent: IDL.Func([OpenKind], [OpenResult], []),
  coins_request_open: IDL.Func([IDL.Nat32], [IDL.Variant({ Ok: Text, Err: Text })], []),
});

// ---- the same, as TypeScript ----------------------------------------------------------------------------------------------------------
export type Opt<T> = [] | [T];
export const opt = <T>(x: Opt<T> | undefined): T | null => (x && x.length ? x[0] : null);
export type ListingKindV = { EthNft: { chain_id: bigint; collection: Uint8Array | number[]; custody: Uint8Array | number[] } }
  | { Inscriptions: { membership_root: Uint8Array | number[]; custody: string } } | { SolNft: { collection: Uint8Array | number[]; custodian: Uint8Array | number[] } };
export type ItemV = { item_id: number; key: string; source_tx: string; vout: number; deposit: boolean; inscription_id: string };
export type TicketSummaryV = { id: bigint; stage: string; item_id: number; deliver_by: bigint; destination: string; undeliverable: boolean };
export type BooksViewV = { coll: number; kind: ListingKindV; wx_mint: string; paused: boolean; poster_paused: boolean; accepted: bigint; target: bigint; posted_at: bigint;
  fresh: boolean; escaped: boolean; last_good_at: bigint; backing: bigint; reserve: bigint; surplus: bigint; revenue: bigint; owed: bigint; reserve_units: bigint;
  transit_units: bigint; advanced: bigint; mint_price: Opt<bigint>; burn_pays_one: Opt<bigint>; volume_7d: bigint; items_arrived: bigint; held: bigint;
  open_tickets: bigint; burned: bigint; ticket_count: bigint; item_count: number; sol_backed: bigint; module_units: bigint; expected_supply: Opt<bigint>;
  items: ItemV[]; tickets_open: TicketSummaryV[]; halted: Opt<string>; raffle_pot: bigint; raffle_prize: bigint };
export type CollectionViewV = { index: number; symbol: string; stage: string; wx_mint: string; config: string; module: string; coins: number; backed: number };
export type OpenStatusV = { index: number; position: number; asked_at: bigint; waiting_since: bigint; why: string; skipped: boolean };
export type VaultViewV = { coll: number; coin: string; pool: string; stage: string; vault: Opt<string>; wx_in: bigint; coins_burned: bigint; wx_paid: bigint; backed_at: bigint };
export type DeskViewV = { coll: number; pair: string; position: string; wx: bigint; wsol: bigint; native: bigint; active: number; lo: number; hi: number; pos_x: bigint; pos_y: bigint;
  fee_units: bigint; fee_lamports: bigint; swept: bigint; arb_profit: bigint; arbs_last_hour: number; read_at: bigint };
export type IntentState = "Open" | "Funded" | "Executing" | "Done" | "Refunded" | "Expired";
export type TxV = { at: bigint; op: string; result: string; signature: string; link: bigint; memo: string };
export type IntentViewV = { id: bigint; kind: string; coll: number; state: Record<IntentState, null> | { [k in IntentState]?: null }; address: string; token_account: Opt<string>;
  recipient: string; refund_to: string; deposit: bigint; min_deposit: bigint; created_at: bigint; expires_at: bigint; funded_at: Opt<bigint>; minted_units: bigint;
  paid_lamports: bigint; ticket: Opt<bigint>; refunding: Opt<string>; in_flight: Opt<string>; note: string; attention: boolean; txs: TxV[] };
export type MintQuoteV = { fee: bigint; min_deposit: bigint; coll: number; ata_rent: bigint; deposit: bigint; to_module: bigint; unit_price: bigint; units: bigint };
export type BurnQuoteV = { fee: bigint; net: bigint; coll: number; paid: bigint; units: bigint; min_units: bigint };
export type QuoteV = { Mint: MintQuoteV } | { Burn: BurnQuoteV };
export type OpenedV = { id: bigint; fee: bigint; min_deposit: bigint; token_account: Opt<string>; quote: Opt<QuoteV>; address: string; expires_at: bigint };
export type OpenKindV = { Mint: { coll: number; recipient: string; min_out: Opt<bigint> } } | { Burn: { coll: number; recipient: string; refund_to: Opt<string>; min_out: Opt<bigint> } }
  | { RedeemNft: { coll: number; destination: string; owner: string } } | { RedeemCoin: { coll: number; coin: string; recipient: string; refund_to: Opt<string> } };
export type Result<T> = { Ok: T } | { Err: string };
export type FindingV = { coll: number; chain: bigint; books: bigint; note: string; kind: { SupplyMismatch: null } | { Unbooked: null } | { Invariant: null } | { BooksUnbacked: null } | { HolderBurn: null } };
export type ReportV = { at: bigint; unexplained: string[]; checked: number; memos_seen: number; findings: FindingV[]; halted: number[] };
export type IncidentV = { at: bigint; coll: Opt<number>; link: Opt<bigint>; what: string };
export type DeskStatusV = { incidents: IncidentV[]; report: Opt<ReportV>; open: number; collections: number; last_reconcile: bigint; funded: number; needing_attention: number;
  open_links: number; executing: number; halted: [number, string][]; coin_redeem: boolean; paused: boolean };
export type TreasuryV = { reserves_pot: bigint; unallocated: bigint; total_received: bigint; total_allocated: bigint; total_to_reserves: bigint; total_to_raffles: bigint; total_to_ops: bigint;
  reserves_bps: number; ops_bps: number; raffle_bps: number; reserve_cap_bps: number; collections: number[] };
export type OwedV = { Ops: { lamports: bigint } } | { Surplus: { lamports: bigint; coll: number } };
export type TreasuryStatusV = { off: boolean; last: [string, bigint, string][]; owed: OwedV[]; treasury: TreasuryV };
export type FeeBookV = { to_desk_lamports: bigint; to_desk_burned_wx: bigint; lp_wx: bigint; cashed_lamports: bigint; curve_wx: bigint; cashed_wx: bigint; to_desk_wx: bigint };
export type WaitV = { key: string; why: string; tries: number; retry_at: bigint; since: bigint };
/** `coins_log`: the fee books, the waits, the last landings, and the opening queue (`(collection, asked at)`, oldest first). */
export type CoinsLogV = [[number, FeeBookV][], WaitV[], [string, bigint, string][], [number, bigint][]];
/** A collection's raffle on the books (docs/v9/raffle-design.md): `key` the 32 bytes the draw is keyed by (the collection's wX mint), `pot`
 *  lamports earmarked for prizes at `K_tre`, `prize` wX base units held as the prize, `winner` zero until a claim; times in seconds. */
export type RaffleV = { key: Uint8Array | number[]; phase: { Committed: null } | { Idle: null } | { Drawn: null }; pot: bigint; prize: bigint; round: bigint; root: Uint8Array | number[];
  total_weight: bigint; entries: number; snapshot_slot: bigint; committed_at: bigint; drand_round: bigint; ticket: bigint; drawn_at: bigint; winner: Uint8Array | number[];
  claimed_at: bigint; total_in: bigint; total_spent: bigint; prizes_funded: bigint; prizes_claimed: bigint; rounds_expired: bigint; rounds_cancelled: bigint };
export type PageLeafV = { owner: string; cum_start: bigint; weight: bigint };
export type PageCoinV = { mint: string; vault_wx: string; wx: bigint; supply: bigint; circ: bigint; holders: number };
export type PageExcludedV = { owner: string; coin: string; account: string; amount: bigint; reason: string };
/** A page of a published snapshot (`raffle_snapshot(listing, round, offset, limit)`; `listing` and `raffle` are the collection's wX mint,
 *  base58): the leaves from `offset`, every coin counted and every account excluded, the root and the slots it was read between. */
export type SnapshotPageV = { listing: string; raffle: string; round: bigint; slot: bigint; slot_lo: bigint; taken_at: bigint; root: string; total_weight: bigint; entries: number;
  offset: number; leaves: PageLeafV[]; coins: PageCoinV[]; excluded: PageExcludedV[] };

export type RookCanister = {
  books_collections(): Promise<[number, string][]>;
  books_view(coll: number): Promise<Opt<BooksViewV>>;
  coins_collections(): Promise<CollectionViewV[]>;
  coins_vaults(coll: number): Promise<VaultViewV[]>;
  coins_open_queue(): Promise<OpenStatusV[]>;
  coins_log(): Promise<CoinsLogV>;
  pool_desks(): Promise<DeskViewV[]>;
  desk_status(): Promise<DeskStatusV>;
  treasury_status(): Promise<TreasuryStatusV>;
  raffle_status(coll: number): Promise<Opt<RaffleV>>;
  raffle_snapshot(listing: string, round: bigint, offset: number, limit: number): Promise<Opt<SnapshotPageV>>;
  intent_status(id: bigint): Promise<Opt<IntentViewV>>;
  quote_mint(coll: number, lamports: bigint): Promise<Result<MintQuoteV>>;
  quote_burn(coll: number, units: bigint): Promise<Result<BurnQuoteV>>;
  open_intent(kind: OpenKindV): Promise<Result<OpenedV>>;
  coins_request_open(index: number): Promise<Result<string>>;
};

/** The state's one tag ("Open", "Done", …). */
export const stateOf = (v: Pick<IntentViewV, "state">): IntentState => Object.keys(v.state)[0] as IntentState;
export const findingKindOf = (f: FindingV): string => Object.keys(f.kind)[0];
export const hexBytes = (h: string): Uint8Array => Uint8Array.from((h.trim().match(/.{1,2}/g) ?? []).map((b) => parseInt(b, 16)));

// ---- saved replies (the canister's test fixtures), decoded as the actor would --------------------------------------------------------
const one = <T>(t: IDL.Type, hex: string): T => IDL.decode([t], hexBytes(hex))[0] as unknown as T;
export const decodeBooksView = (hex: string): Opt<BooksViewV> => one(IDL.Opt(BooksView), hex);
export const decodeDeskStatus = (hex: string): DeskStatusV => one(DeskStatus, hex);
export const decodeTreasuryStatus = (hex: string): TreasuryStatusV => one(TreasuryStatus, hex);
export const decodePoolDesks = (hex: string): DeskViewV[] => one(IDL.Vec(DeskView), hex);
export const decodeCoinsLog = (hex: string): CoinsLogV => IDL.decode([IDL.Vec(IDL.Tuple(IDL.Nat32, FeeBook)), IDL.Vec(Wait), IDL.Vec(IDL.Tuple(Text, N, Text)), IDL.Vec(IDL.Tuple(IDL.Nat32, N))], hexBytes(hex)) as unknown as CoinsLogV;
export const decodeOpenQueue = (hex: string): OpenStatusV[] => one(IDL.Vec(OpenStatus), hex);
export const encodeOpenQueue = (v: OpenStatusV[]): Uint8Array => new Uint8Array(IDL.encode([IDL.Vec(OpenStatus)], [v]));
export const decodeIntentView = (hex: string): Opt<IntentViewV> => one(IDL.Opt(IntentView), hex);
export const decodeOpened = (hex: string): Result<OpenedV> => one(OpenResult, hex);
export const decodeRaffle = (hex: string): Opt<RaffleV> => one(IDL.Opt(Raffle), hex);
export const decodeSnapshotPage = (hex: string): Opt<SnapshotPageV> => one(IDL.Opt(SnapshotPage), hex);
/** For tests: a view encoded as the canister would reply it. */
export const encodeIntentView = (v: IntentViewV): Uint8Array => new Uint8Array(IDL.encode([IDL.Opt(IntentView)], [[v]]));
export const encodeOpened = (r: Result<OpenedV>): Uint8Array => new Uint8Array(IDL.encode([OpenResult], [r]));
export const encodeOpenKind = (k: OpenKindV): Uint8Array => new Uint8Array(IDL.encode([OpenKind], [k]));
export const encodeRaffle = (v: RaffleV | null): Uint8Array => new Uint8Array(IDL.encode([IDL.Opt(Raffle)], [v ? [v] : []]));
export const encodeSnapshotPage = (v: SnapshotPageV | null): Uint8Array => new Uint8Array(IDL.encode([IDL.Opt(SnapshotPage)], [v ? [v] : []]));
export const decodeOpenKind = (b: Uint8Array): OpenKindV => IDL.decode([OpenKind], b)[0] as unknown as OpenKindV;

// ---- the site's canister ----------------------------------------------------------------------------------------------------------------
export type CanisterConfig = { canisterId: string | null; icHost: string; problems: string[] };
const CANISTER_ID_RE = /^[a-z0-9]{5}(-[a-z0-9]{5}){3}-[a-z0-9]{3}$/;
export function loadCanisterConfig(raw: { canisterId?: string; icHost?: string } = CANISTER_SITE_CONFIG, idOverride?: string | null, hostOverride?: string | null): CanisterConfig {
  const problems: string[] = [];
  const id = (idOverride ?? "").trim() || (raw.canisterId ?? "").trim() || null;
  if (id && !CANISTER_ID_RE.test(id)) problems.push(`canisterId "${id}" is not a canister id`);
  const host = (hostOverride ?? "").trim() || raw.icHost || "https://icp-api.io";
  if (!/^https:\/\/|^http:\/\/(127\.0\.0\.1|localhost)(:\d+)?$/.test(host)) problems.push(`icHost must be https (or a local replica): ${host}`);
  return { canisterId: id && CANISTER_ID_RE.test(id) ? id : null, icHost: host, problems };
}
/** A local replica (dfx) fetches its root key; the IC's is built in. */
export const isLocalHost = (host: string): boolean => { try { return ["127.0.0.1", "localhost"].includes(new URL(host).hostname); } catch { return false; } };

const viteEnv = (import.meta as unknown as { env?: Record<string, string | undefined> }).env ?? {};
export const CANISTER: CanisterConfig = loadCanisterConfig(CANISTER_SITE_CONFIG, viteEnv.VITE_CANISTER_ID, viteEnv.VITE_IC_HOST);

let actorOne: Promise<RookCanister> | null = null;
/** The site's canister actor, once (anonymous). Throws when this build names no canister. */
export function canister(): Promise<RookCanister> {
  if (!CANISTER.canisterId) return Promise.reject(new Error("this build names no Rook canister (src/config/canister.ts)"));
  return (actorOne ??= HttpAgent.create({ host: CANISTER.icHost, shouldFetchRootKey: isLocalHost(CANISTER.icHost) })
    .then((agent) => Actor.createActor(canisterIdl, { agent, canisterId: CANISTER.canisterId! }) as unknown as RookCanister));
}
export const HAS_CANISTER = !!CANISTER.canisterId;
