import { Actor, HttpAgent, type Identity } from "@icp-sdk/core/agent";
import { IDL } from "@icp-sdk/core/candid";

/**
 * docs/v6/p1-services.md §2–§3. The canister methods ops/ calls, typed from autobuyer/canister-rs/etch_autobuyer.did. Records are
 * written with only the fields read (Candid's record subtyping skips the rest); every variant lists every tag, because an unknown
 * tag is an error. The runner and the offer tool call with the operator's identity; the reconciler calls anonymously (queries).
 */
const Result = IDL.Variant({ Ok: IDL.Text, Err: IDL.Text });
const ResultList = IDL.Variant({ Ok: IDL.Vec(IDL.Text), Err: IDL.Text });
const Outpoint = IDL.Record({ txid: IDL.Vec(IDL.Nat8), vout: IDL.Nat32 });
const Bought = IDL.Record({ inscription: IDL.Text, outpoint: Outpoint, spend_sats: IDL.Nat64, cost_wei: IDL.Nat });
const WithBuy = IDL.Record({ buy: Bought });
const FPhase = IDL.Variant({
  Wanted: IDL.Record({ cap_wei: IDL.Nat }), Signed: WithBuy, Sent: WithBuy, Held: WithBuy, Minted: WithBuy, Settled: WithBuy, Stray: WithBuy,
  Stranded: IDL.Record({ buy: Bought, why: IDL.Text }), Failed: IDL.Record({ why: IDL.Text }),
  // every tag the canister can answer (an unknown tag fails the whole decode): Abandoning was missing until 2026-10-09; its conflict is
  // read as reserved (not used here). Handed: docs/v8/ow-protected.md §3.3, a protected sale signed and handed to Ordinals Wallet.
  Abandoning: IDL.Record({ buy: Bought, conflict: IDL.Reserved }),
  Handed: IDL.Record({ buy: Bought, sale_txid: IDL.Text, at: IDL.Nat64 }),
});
// docs/v8/ow-protected.md §3.4: a protected listing as the runner hands it in, and the signed sale it reads back
const ProtectedListing = IDL.Record({ outpoint: IDL.Text, seller_address: IDL.Text, satoshi_price: IDL.Nat64, escrow_price: IDL.Opt(IDL.Nat64),
  creator_address: IDL.Opt(IDL.Text) });
const ProtectedSigned = IDL.Record({ sale_txid: IDL.Text, sale_psbt: IDL.Text, setup_psbt: IDL.Opt(IDL.Text) });
const FrogTarget = IDL.Variant({ Sink: IDL.Vec(IDL.Nat8), Desk: IDL.Nat64 });
const FrogRec = IDL.Record({ id: IDL.Nat64, target: IDL.Opt(FrogTarget), phase: FPhase });
const CollectionCap = IDL.Record({ collection: IDL.Text, max_items: IDL.Nat32, max_value_wei: IDL.Nat, max_item_wei: IDL.Nat });
const CapPolicy = IDL.Record({ collections: IDL.Vec(CollectionCap), global_max_items: IDL.Nat32, global_max_value_wei: IDL.Nat });
// docs/v6/relay-conversion.md §3.2: a conversion (an advance turned into sats by Relay) and what the runner must quote
const Blob = IDL.Vec(IDL.Nat8);
const ConvAsk = IDL.Record({ amount_wei: IDL.Nat, user: IDL.Text, recipient: IDL.Text, refund_to: IDL.Text, origin_chain_id: IDL.Nat64,
  destination_chain_id: IDL.Nat64, origin_currency: IDL.Text, destination_currency: IDL.Text });
const Order = { order_id: Blob, value: IDL.Nat, min_sats: IDL.Nat64, deadline: IDL.Nat64 };
const CPhase = IDL.Variant({
  NeedAdvance: IDL.Null, Advancing: IDL.Null, Funded: IDL.Record({ wei: IDL.Nat, since: IDL.Nat64 }), Depositing: IDL.Record(Order),
  Converting: IDL.Record({ ...Order, since: IDL.Nat64, balance_after: IDL.Nat }), Arrived: IDL.Record({ order_id: Blob, sats: IDL.Nat64 }),
  Returning: IDL.Record({ wei: IDL.Nat }), Returned: IDL.Record({ wei: IDL.Nat }), Settled: IDL.Record({ cost: IDL.Nat, spare_sats: IDL.Nat64 }),
  Stuck: IDL.Record({ why: IDL.Text }),
});
const ConvRec = IDL.Record({ frog: IDL.Nat64, module: Blob, advanced: IDL.Nat, attempts: IDL.Nat32, phase: CPhase, orders: IDL.Vec(Blob),
  deposits: IDL.Vec(Blob), arrival: IDL.Opt(IDL.Record({ outpoint: Outpoint, sats: IDL.Nat64 })), ask: IDL.Opt(ConvAsk), note: IDL.Text,
  // review C-01/C-03/C-08 (the `late` watch is not decoded here)
  arrived_at: IDL.Opt(IDL.Nat64), abandoned_sats: IDL.Opt(IDL.Nat64), returned_at: IDL.Opt(IDL.Nat64) });
// docs/v7/phase1-nft.md §4.6: the Ethereum NFT side (only the fields the runner and the reconciler read)
const Rec = (o: Record<string, IDL.Type>) => IDL.Record(o);
const BuyTok = { arrived: IDL.Nat, token: Blob, total: IDL.Nat, tx: Blob };
const BPhase = IDL.Variant({
  NeedAdvance: IDL.Null, Advancing: IDL.Null, Funded: Rec({ wei: IDL.Nat, since: IDL.Nat64 }),
  Depositing: Rec({ order_id: Blob, value: IDL.Nat, min_out: IDL.Nat, deadline: IDL.Nat64 }),
  Bridging: Rec({ order_id: Blob, value: IDL.Nat, min_out: IDL.Nat, deadline: IDL.Nat64, since: IDL.Nat64 }),
  Arrived: Rec({ wei: IDL.Nat, since: IDL.Nat64 }), Buying: Rec(BuyTok), Bought: Rec({ ...BuyTok, block: IDL.Nat64 }), Final: Rec(BuyTok),
  Minting: Rec(BuyTok), Minted: Rec(BuyTok), Settling: Rec({ token: Blob, total: IDL.Nat, tx: Blob, cost: IDL.Nat }),
  Settled: Rec({ token: Blob, total: IDL.Nat, tx: Blob, cost: IDL.Nat }), Returning: Rec({ wei: IDL.Nat }), Returned: Rec({ wei: IDL.Nat }),
  Stuck: Rec({ why: IDL.Text }),
  // audit 2026-09-27-nft round 2 (A2-M-1): an arrived buy's ETH going back (Relay Ethereum → 4663, then returnAdvance)
  BackFunded: Rec({ wei: IDL.Nat, since: IDL.Nat64 }),
  BackSending: Rec({ wei: IDL.Nat, order_id: Blob, value: IDL.Nat, min_out: IDL.Nat, deadline: IDL.Nat64, tx: Blob }),
  BackBridging: Rec({ order_id: Blob, value: IDL.Nat, min_out: IDL.Nat, deadline: IDL.Nat64, since: IDL.Nat64 }),
});
const EthBuy = IDL.Record({ id: IDL.Nat64, collection: Blob, module: Blob, wrapper: Blob, advanced: IDL.Nat, attempts: IDL.Nat32, phase: BPhase,
  ask: IDL.Opt(ConvAsk), note: IDL.Text });
const Why = Rec({ why: IDL.Text });
const DelPhase = IDL.Variant({
  Waiting: IDL.Null, Sent: Rec({ tx: Blob, nonce: IDL.Nat64 }), Moved: Rec({ tx: Blob, block: IDL.Nat64 }), Record: Rec({ tx: Blob }),
  Recording: Rec({ tx: Blob }), Recorded: Rec({ tx: Blob }), Cancelling: Rec({ of: Blob, tx: Blob, nonce: IDL.Nat64 }),
  MarkAfter: Rec({ why: IDL.Text, block: IDL.Nat64 }), Mark: Why, Marking: Why, Marked: Why, Gone: Why,
});
const Delivery = IDL.Record({ wrapper: Blob, collection: Blob, ticket: IDL.Nat, token: Blob, destination: Blob, deliver_by: IDL.Nat64, phase: DelPhase, note: IDL.Text });
const FloorPost = IDL.Record({ price: IDL.Nat, confidence: IDL.Nat16, depth: IDL.Nat32, listing: IDL.Nat, sale_median: IDL.Nat, sale_p25: IDL.Nat,
  listings: IDL.Nat32, sales: IDL.Nat32, clamped: IDL.Bool });
const Reading = IDL.Record({ collection: Blob, post: FloorPost, at: IDL.Nat64, pin: IDL.Nat64, due: IDL.Bool });
// the claims the canister has judged (docs/v7/phase1-nft.md §2.4, D5): the runner hands in only what it has not seen
const EthDepositRec = IDL.Record({ tx: Blob, collection: Blob, token: Blob });
// review round 6 (F6-L-1): `pending`, the 4663 transaction out (its nonce, and round 7 (F7-I-2) since when it is seen nowhere: record subtyping skips the rest), so the runner ticks sooner
const EthState = IDL.Record({ buys: IDL.Vec(EthBuy), deliveries: IDL.Vec(Delivery), readings: IDL.Vec(Reading), eth_out: IDL.Nat,
  deposits: IDL.Vec(EthDepositRec), pending: IDL.Opt(IDL.Record({ nonce: IDL.Nat64, used_since: IDL.Opt(IDL.Nat64) })) });
// 2026-10-12 (Robinhood Chain NFTs): a collection's own chain (`None`: the config's) and whether each NFT is a token-bound account (StonkBrokers)
const EthCollection = IDL.Record({ collection: IDL.Text, wrapper: IDL.Text, module: IDL.Text, max_purchase_wei: IDL.Nat, max_premium_bps: IDL.Nat16,
  chain_id: IDL.Opt(IDL.Nat64), token_bound: IDL.Opt(IDL.Bool), nft_value_bps: IDL.Opt(IDL.Nat16) });
const EthNftConfig = IDL.Record({ chain_id: IDL.Nat64, collections: IDL.Vec(EthCollection), relay_max_fee_bps: IDL.Nat16, deposits: IDL.Bool });
// docs/v7/dormant-collections.md §4.2: each collection's life and the status the site shows ("warming up (~N min)")
const CollectionLife = IDL.Record({ collection: IDL.Text, wrapper: IDL.Text, module: IDL.Text, state: IDL.Text, since: IDL.Nat64, eta_secs: IDL.Opt(IDL.Nat64),
  queue_position: IDL.Opt(IDL.Nat32), woken_by: IDL.Opt(IDL.Text), cooldown_until: IDL.Opt(IDL.Nat64), status: IDL.Text, note: IDL.Text });
// docs/v7/dormant-collections.md §11: how many collections the canister's cycles balance keeps awake ("N of M collections can be awake right now")
const Capacity = IDL.Record({ at: IDL.Nat64, balance: IDL.Nat, scaling: IDL.Bool, collections: IDL.Nat8, ceiling: IDL.Nat8, wakes_per_day: IDL.Nat8,
  effective_max_active: IDL.Nat8, awake: IDL.Nat8, pinned: IDL.Nat8, wake_headroom: IDL.Nat8, wakes_left_today: IDL.Nat8, next_wake_at: IDL.Nat, status: IDL.Text });
// Ordinals multi-collection (docs/v9/ordinals-collections.md §4): every Inscriptions listing in the books, live or dormant
const FrogReading = IDL.Record({ at: IDL.Nat64, sats: IDL.Nat64, confidence_bps: IDL.Nat16, depth_items: IDL.Nat32 });
const FrogCollection = IDL.Record({ coll: IDL.Nat16, wx_mint: IDL.Text, root: IDL.Text, live: IDL.Bool, frogs_open: IDL.Nat64, floor_reading: IDL.Opt(FrogReading) });
// a holder deposit (`holder_deposits`), only what the proof step reads: its outpoint and phase (every tag listed; the payloads the step
// does not read are reserved)
const R = IDL.Reserved, Insc = IDL.Record({ inscription: IDL.Text });
const DPhase = IDL.Variant({ Parked: Insc, AwaitingProof: Insc, Held: R, NotAFrog: R, Ready: R, Minted: R, Returning: R, ReturnConflicting: R, Refused: R,
  Returned: R, Pending: R });
const DepositRec = IDL.Record({ outpoint: Outpoint, phase: DPhase });
const Headroom = IDL.Record({ collection: IDL.Text, items_held: IDL.Nat32, items_in_flight: IDL.Nat32, value_wei: IDL.Nat, items_left: IDL.Nat32, value_left_wei: IDL.Nat });

// ---- W11 (canister-only): the books' views, generated from etch_autobuyer.did by the session's converter (keep in step with the did) ----
const B_ItemView = IDL.Record({ key: IDL.Text, vout: IDL.Nat32, inscription_id: IDL.Text, deposit: IDL.Bool, source_tx: IDL.Text, item_id: IDL.Nat32 });
const B_ListingKind = IDL.Variant({ SolNft: IDL.Record({ collection: IDL.Vec(IDL.Nat8), custodian: IDL.Vec(IDL.Nat8) }), Inscriptions: IDL.Record({ custody: IDL.Text, membership_root: IDL.Vec(IDL.Nat8) }), EthNft: IDL.Record({ collection: IDL.Vec(IDL.Nat8), custody: IDL.Vec(IDL.Nat8), chain_id: IDL.Nat64 }) });
const B_TicketSummary = IDL.Record({ id: IDL.Nat64, destination: IDL.Text, deliver_by: IDL.Int64, undeliverable: IDL.Bool, stage: IDL.Text, item_id: IDL.Nat32 });
const B_BooksView = IDL.Record({ fresh: IDL.Bool, transit_units: IDL.Nat64, last_good_at: IDL.Int64, raffle_prize: IDL.Nat64, expected_supply: IDL.Opt(IDL.Nat64), reserve_units: IDL.Nat64, revenue: IDL.Nat64, poster_paused: IDL.Bool, advanced: IDL.Nat64, coll: IDL.Nat16, held: IDL.Nat64, kind: B_ListingKind, owed: IDL.Nat64, wx_mint: IDL.Text, items_arrived: IDL.Nat64, volume_7d: IDL.Nat64, reserve: IDL.Nat64, surplus: IDL.Nat64, raffle_pot: IDL.Nat64, target: IDL.Nat64, tickets_open: IDL.Vec(B_TicketSummary), posted_at: IDL.Int64, module_units: IDL.Nat64, mint_price: IDL.Opt(IDL.Nat64), escaped: IDL.Bool, burn_pays_one: IDL.Opt(IDL.Nat64), sol_backed: IDL.Nat64, halted: IDL.Opt(IDL.Text), open_tickets: IDL.Nat64, items: IDL.Vec(B_ItemView), accepted: IDL.Nat64, burned: IDL.Nat64, ticket_count: IDL.Nat64, paused: IDL.Bool, backing: IDL.Nat64, item_count: IDL.Nat32 });
const B_CollectionView = IDL.Record({ coins: IDL.Nat32, backed: IDL.Nat32, wx_mint: IDL.Text, stage: IDL.Text, index: IDL.Nat32, module: IDL.Text, config: IDL.Text, symbol: IDL.Text });
const B_DeskView = IDL.Record({ hi: IDL.Int32, lo: IDL.Int32, wx: IDL.Nat64, read_at: IDL.Nat64, active: IDL.Int32, native: IDL.Nat64, coll: IDL.Nat32, pair: IDL.Text, wsol: IDL.Nat64, fee_lamports: IDL.Nat64, swept: IDL.Nat64, fee_units: IDL.Nat64, pos_x: IDL.Nat64, pos_y: IDL.Nat64, position: IDL.Text, arb_profit: IDL.Nat64, arbs_last_hour: IDL.Nat32 });
const B_VaultView = IDL.Record({ vault: IDL.Opt(IDL.Text), coin: IDL.Text, coll: IDL.Nat32, pool: IDL.Text, wx_paid: IDL.Nat64, stage: IDL.Text, backed_at: IDL.Nat64, coins_burned: IDL.Nat64, wx_in: IDL.Nat64 });

/** W11: a saved `books_view` reply (hex), decoded as the actor would (the canister's tests write tests/fixtures/views/books-view.hex). */
export const decodeBooksView = (hexStr: string): [] | [BooksViewV] => IDL.decode([IDL.Opt(B_BooksView)], Uint8Array.from(Buffer.from(hexStr, "hex")))[0] as [] | [BooksViewV];

export const buyerIdl: IDL.InterfaceFactory = () => IDL.Service({
  tick: IDL.Func([], [ResultList], []),
  paused: IDL.Func([], [IDL.Bool], ["query"]),
  // Ordinals multi-collection (the merged did): every frog method names its collection (`coll`, the books id of the Inscriptions listing);
  // the views take `opt nat16` (none: every collection)
  frog_collections: IDL.Func([], [IDL.Vec(FrogCollection)], ["query"]),
  frog_offer: IDL.Func([IDL.Nat16, IDL.Nat64, IDL.Text, IDL.Text, IDL.Vec(IDL.Nat32), IDL.Vec(IDL.Text)], [Result], []),
  frog_buys: IDL.Func([IDL.Opt(IDL.Nat16)], [IDL.Vec(FrogRec)], ["query"]),
  frog_offer_protected: IDL.Func([IDL.Nat16, IDL.Nat64, IDL.Text, IDL.Text, IDL.Text, ProtectedListing, IDL.Opt(IDL.Text), IDL.Vec(IDL.Text)], [Result], []),
  frog_proof: IDL.Func([IDL.Nat16, IDL.Text, IDL.Vec(IDL.Text)], [Result], []),
  probe_frog: IDL.Func([IDL.Nat16, IDL.Text], [Result], []),
  frog_wake: IDL.Func([IDL.Nat16], [Result], []),
  frog_sleep: IDL.Func([IDL.Nat16], [Result], []),
  holder_deposits: IDL.Func([], [IDL.Vec(DepositRec)], ["query"]),
  unmarked_custody: IDL.Func([IDL.Opt(IDL.Nat16)], [IDL.Vec(IDL.Text)], ["query"]),
  frog_protected_signed: IDL.Func([IDL.Nat64], [IDL.Opt(ProtectedSigned)], ["query"]),
  float_internal_key: IDL.Func([], [Result], []),
  custody_caps: IDL.Func([], [CapPolicy, IDL.Vec(Headroom)], ["query"]),
  float_address: IDL.Func([], [Result], []),
  custody_address: IDL.Func([], [Result], []),
  frog_conversions: IDL.Func([IDL.Opt(IDL.Nat16)], [IDL.Vec(ConvRec)], ["query"]),
  relay_quote: IDL.Func([IDL.Nat64, IDL.Text], [Result], []),
  relay_give_back: IDL.Func([IDL.Nat16, IDL.Nat64], [Result], []),
  relay_arrived_exit: IDL.Func([IDL.Nat64], [Result], []),
  relay_late_release: IDL.Func([IDL.Nat64], [Result], []),
  evm_address: IDL.Func([], [Result], []),
  eth_nft_state: IDL.Func([], [EthState], ["query"]),
  eth_nft_config: IDL.Func([], [IDL.Opt(EthNftConfig)], ["query"]),
  // 2026-10-12 (Robinhood Chain NFTs): the side of one chain (None: no collection there), and the operator's line exit on that side
  eth_nft_state_of: IDL.Func([IDL.Nat64], [IDL.Opt(EthState)], ["query"]),
  eth_line_clear_on: IDL.Func([IDL.Nat64], [Result], []),
  eth_nft_collections: IDL.Func([], [IDL.Vec(CollectionLife)], ["query"]),
  eth_nft_capacity: IDL.Func([], [IDL.Opt(Capacity)], ["query"]),
  eth_relay_quote: IDL.Func([IDL.Nat64, IDL.Text], [Result], []),
  eth_listing: IDL.Func([IDL.Nat64, IDL.Text], [Result], []),
  eth_floor_evidence: IDL.Func([IDL.Text, IDL.Vec(IDL.Text), IDL.Vec(IDL.Text)], [Result], []),
  eth_deposit: IDL.Func([IDL.Text], [Result], []),
  // docs/v9/solana-migration.md §2.9 D-i: the Solana home's frog floor reading (sats per item, confidence bps, depth), posted by the canister
  // as the registry keeper at the next tick; and the canister's Solana address (threshold Ed25519)
  // Ordinals multi-collection: the reading names its collection (`coll`)
  frog_floor_reading: IDL.Func([IDL.Nat16, IDL.Nat64, IDL.Nat16, IDL.Nat32], [Result], []),
  // docs/v9/solana-nfts-canister.md §5 (src/solnftflow/): Magic Eden's listings and stats of an open Solana collection
  sol_nft_evidence: IDL.Func([IDL.Nat16, IDL.Text, IDL.Text], [Result], []),
  sol_address: IDL.Func([], [Result], []),
  books_view: IDL.Func([IDL.Nat16], [IDL.Opt(B_BooksView)], ["query"]),
  books_collections: IDL.Func([], [IDL.Vec(IDL.Tuple(IDL.Nat16, IDL.Text))], ["query"]),
  coins_collections: IDL.Func([], [IDL.Vec(B_CollectionView)], ["query"]),
  coins_vaults: IDL.Func([IDL.Nat32], [IDL.Vec(B_VaultView)], ["query"]),
  pool_desks: IDL.Func([], [IDL.Vec(B_DeskView)], ["query"]),
});

export type CapPolicyV = { collections: { collection: string; max_items: number; max_value_wei: bigint; max_item_wei: bigint }[]; global_max_items: number; global_max_value_wei: bigint };
export type HeadroomV = { collection: string; items_held: number; items_in_flight: number; value_wei: bigint; items_left: number; value_left_wei: bigint };
export type FrogRecV = { id: bigint; target: [] | [{ Sink: Uint8Array } | { Desk: bigint }]; phase: Record<string, unknown> };

export type ConvAskV = { amount_wei: bigint; user: string; recipient: string; refund_to: string; origin_chain_id: bigint; destination_chain_id: bigint;
  origin_currency: string; destination_currency: string };
export type ConvRecV = { frog: bigint; module: Uint8Array | number[]; advanced: bigint; attempts: number; phase: Record<string, unknown>; orders: (Uint8Array | number[])[];
  deposits: (Uint8Array | number[])[]; arrival: [] | [{ outpoint: { txid: Uint8Array | number[]; vout: number }; sats: bigint }]; ask: [] | [ConvAskV]; note: string };

export type EthBuyV = { id: bigint; collection: Uint8Array | number[]; module: Uint8Array | number[]; wrapper: Uint8Array | number[]; advanced: bigint;
  attempts: number; phase: Record<string, any>; ask: [] | [ConvAskV]; note: string }; // eslint-disable-line @typescript-eslint/no-explicit-any
export type DeliveryV = { wrapper: Uint8Array | number[]; collection: Uint8Array | number[]; ticket: bigint; token: Uint8Array | number[];
  destination: Uint8Array | number[]; deliver_by: bigint; phase: Record<string, any>; note: string }; // eslint-disable-line @typescript-eslint/no-explicit-any
export type EthStateV = { buys: EthBuyV[]; deliveries: DeliveryV[]; readings: { collection: Uint8Array | number[]; post: { price: bigint; confidence: number;
  depth: number; listing: bigint; sale_median: bigint }; at: bigint; pin: bigint; due: boolean }[]; eth_out: bigint;
  deposits: { tx: Uint8Array | number[]; collection: Uint8Array | number[]; token: Uint8Array | number[] }[]; pending?: [] | [{ nonce: bigint; used_since?: [] | [bigint] }] };
export type CollectionLifeV = { collection: string; wrapper: string; module: string; state: string; since: bigint; eta_secs: [] | [bigint];
  queue_position: [] | [number]; woken_by: [] | [string]; cooldown_until: [] | [bigint]; status: string; note: string };
export type CapacityV = { at: bigint; balance: bigint; scaling: boolean; collections: number; ceiling: number; wakes_per_day: number;
  effective_max_active: number; awake: number; pinned: number; wake_headroom: number; wakes_left_today: number; next_wake_at: bigint; status: string };
export type EthNftConfigV = { chain_id: bigint; collections: { collection: string; wrapper: string; module: string; max_purchase_wei: bigint; max_premium_bps: number;
  /** 2026-10-12: absent in a reply decoded before the field existed (tests' stubs); `[]`: the config's chain / not token-bound */
  chain_id?: [] | [bigint]; token_bound?: [] | [boolean]; /** audit C3-01: a token-bound collection's NFT-alone share of a price */ nft_value_bps?: [] | [number] }[];
  relay_max_fee_bps: number; deposits: boolean };

export type ListingKindV = { EthNft: { chain_id: bigint; collection: number[] | Uint8Array; custody: number[] | Uint8Array } }
  | { Inscriptions: { membership_root: number[] | Uint8Array; custody: string } } | { SolNft: { collection: number[] | Uint8Array; custodian: number[] | Uint8Array } };
export type BooksViewV = { coll: number; kind: ListingKindV; wx_mint: string; paused: boolean; poster_paused: boolean; accepted: bigint; target: bigint; posted_at: bigint;
  fresh: boolean; escaped: boolean; last_good_at: bigint; backing: bigint; reserve: bigint; surplus: bigint; revenue: bigint; owed: bigint; reserve_units: bigint;
  transit_units: bigint; advanced: bigint; mint_price: [] | [bigint]; burn_pays_one: [] | [bigint]; volume_7d: bigint; items_arrived: bigint; held: bigint;
  open_tickets: bigint; burned: bigint; ticket_count: bigint; item_count: number; sol_backed: bigint; module_units: bigint; expected_supply: [] | [bigint];
  items: { item_id: number; key: string; source_tx: string; vout: number; deposit: boolean; inscription_id: string }[];
  tickets_open: { id: bigint; stage: string; item_id: number; deliver_by: bigint; destination: string; undeliverable: boolean }[];
  halted: [] | [string]; raffle_pot: bigint; raffle_prize: bigint };
export type CoinsCollectionV = { index: number; symbol: string; stage: string; wx_mint: string; config: string; module: string; coins: number; backed: number };
export type VaultV = { coll: number; coin: string; pool: string; stage: string; vault: [] | [string]; wx_in: bigint; coins_burned: bigint; wx_paid: bigint; backed_at: bigint };
/** W11: one collection's pool desk as `pool_desks` serves it (docs/v9/canister-only-solana.md §5). */
export type DeskViewV = { hi: number; lo: number; wx: bigint; read_at: bigint; active: number; native: bigint; coll: number; pair: string; wsol: bigint; fee_lamports: bigint;
  swept: bigint; fee_units: bigint; pos_x: bigint; pos_y: bigint; position: string; arb_profit: bigint; arbs_last_hour: number };

/** Ordinals multi-collection: one Inscriptions listing as `frog_collections` serves it (`root` hex, with or without 0x). */
export type FrogCollectionV = { coll: number; wx_mint: string; root: string; live: boolean; frogs_open: bigint;
  floor_reading: [] | [{ at: bigint; sats: bigint; confidence_bps: number; depth_items: number }] };
export type HolderDepositV = { outpoint: { txid: Uint8Array | number[]; vout: number }; phase: { AwaitingProof: { inscription: string } } | { Parked: { inscription: string } }
  | Record<string, unknown> };
export type Buyer = {
  books_view(coll: number): Promise<[] | [BooksViewV]>;
  books_collections(): Promise<[number, string][]>;
  coins_collections(): Promise<CoinsCollectionV[]>;
  coins_vaults(coll: number): Promise<VaultV[]>;
  pool_desks(): Promise<DeskViewV[]>;
  tick(): Promise<{ Ok: string[] } | { Err: string }>;
  paused(): Promise<boolean>;
  frog_collections(): Promise<FrogCollectionV[]>;
  frog_offer(coll: number, id: bigint, inscription: string, psbtHex: string, toSign: number[], proof: string[]): Promise<{ Ok: string } | { Err: string }>;
  frog_buys(coll: [] | [number]): Promise<FrogRecV[]>;
  frog_proof(coll: number, inscription: string, proof: string[]): Promise<{ Ok: string } | { Err: string }>;
  probe_frog(coll: number, inscription: string): Promise<{ Ok: string } | { Err: string }>;
  frog_wake(coll: number): Promise<{ Ok: string } | { Err: string }>;
  frog_sleep(coll: number): Promise<{ Ok: string } | { Err: string }>;
  holder_deposits(): Promise<HolderDepositV[]>;
  unmarked_custody(coll: [] | [number]): Promise<string[]>;
  custody_caps(): Promise<[CapPolicyV, HeadroomV[]]>;
  float_address(): Promise<{ Ok: string } | { Err: string }>;
  custody_address(): Promise<{ Ok: string } | { Err: string }>;
  // docs/v8/ow-protected.md §3.4
  float_internal_key(): Promise<{ Ok: string } | { Err: string }>;
  frog_offer_protected(coll: number, id: bigint, inscription: string, salePsbtHex: string, parentHex: string, listing: import("./owprotected.js").ProtectedListingV,
    setupPsbtHex: [] | [string], proof: string[]): Promise<{ Ok: string } | { Err: string }>;
  frog_protected_signed(id: bigint): Promise<[] | [import("./owprotected.js").ProtectedSignedV]>;
  frog_conversions(coll: [] | [number]): Promise<ConvRecV[]>;
  relay_quote(frog: bigint, quoteJson: string): Promise<{ Ok: string } | { Err: string }>;
  relay_give_back(coll: number, frog: bigint): Promise<{ Ok: string } | { Err: string }>;
  relay_arrived_exit(frog: bigint): Promise<{ Ok: string } | { Err: string }>;
  relay_late_release(frog: bigint): Promise<{ Ok: string } | { Err: string }>;
  evm_address(): Promise<{ Ok: string } | { Err: string }>;
  eth_nft_state(): Promise<EthStateV>;
  eth_nft_config(): Promise<[] | [EthNftConfigV]>;
  /** 2026-10-12: the side of `chainId` (`[]`: no collection there) */
  eth_nft_state_of(chainId: bigint): Promise<[] | [EthStateV]>;
  eth_line_clear_on(chainId: bigint): Promise<{ Ok: string } | { Err: string }>;
  /** docs/v7/dormant-collections.md: absent from a canister built before it (the call then fails, and every collection is served as before) */
  eth_nft_collections(): Promise<CollectionLifeV[]>;
  /** docs/v7/dormant-collections.md §11: absent from a canister built before it (the runner then serves no capacity) */
  eth_nft_capacity(): Promise<[] | [CapacityV]>;
  eth_relay_quote(buy: bigint, quoteJson: string): Promise<{ Ok: string } | { Err: string }>;
  eth_listing(buy: bigint, orderHex: string): Promise<{ Ok: string } | { Err: string }>;
  eth_floor_evidence(collection: string, listings: string[], sales: string[]): Promise<{ Ok: string } | { Err: string }>;
  eth_deposit(txHash: string): Promise<{ Ok: string } | { Err: string }>;
  /** docs/v9 §2.9 D-i (the Solana home): refused on the EVM home */
  frog_floor_reading(coll: number, sats: bigint, confidenceBps: number, depthItems: number): Promise<{ Ok: string } | { Err: string }>;
  /** docs/v9/solana-nfts-canister.md §5: Magic Eden's listings and stats of an open Solana collection */
  sol_nft_evidence(coll: number, listingsJson: string, statsJson: string): Promise<{ Ok: string } | { Err: string }>;
  sol_address(): Promise<{ Ok: string } | { Err: string }>;
};

export const IC_HOST = "https://icp-api.io";
/** A local replica (dfx) fetches its root key; the IC's is built in. Only 127.0.0.1 / localhost count as local. */
export const isLocalHost = (host: string): boolean => { try { return ["127.0.0.1", "localhost"].includes(new URL(host).hostname); } catch { return false; } };

export async function buyer(canisterId: string, o: { host?: string; identity?: Identity | null } = {}): Promise<Buyer> {
  const host = o.host ?? IC_HOST;
  const agent = await HttpAgent.create({ host, identity: o.identity ?? undefined, shouldFetchRootKey: isLocalHost(host) });
  return Actor.createActor(buyerIdl, { agent, canisterId }) as unknown as Buyer;
}
