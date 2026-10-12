// The Candid types the monitor reads, written out from autobuyer/timelock-rs/etch_timelock.did and
// autobuyer/canister-rs/etch_autobuyer.did, and the one decode that must not be lenient: the buyer's upgrade argument.
//
// Query replies are decoded leniently on purpose: a record here may name fewer fields than the canister sends (Candid record
// subtyping skips the rest), so a new field in a record the canary does not need does not break the canary. Every VARIANT tag
// is listed, because an unknown tag is an error, not a skip.
//
// The upgrade argument is the opposite: a field this monitor does not know would silently vanish from the diff. So its type
// table is read directly first (`tableLabels`), and any field or tag hash not in `KNOWN_INIT_LABELS` makes the diff CRITICAL.
import { IDL, idlLabelToId, lebDecode, slebDecode, PipeArrayBuffer } from "@icp-sdk/core/candid";
import { Principal } from "@icp-sdk/core/principal";

// ---- the timelock ----------------------------------------------------------------------------------------------------------
const Roles = IDL.Record({ proposers: IDL.Vec(IDL.Principal), guardians: IDL.Vec(IDL.Principal), veto_threshold: IDL.Nat8 });
const ProposalView = IDL.Record({
  id: IDL.Nat64, by: IDL.Principal, kind: IDL.Text, wasm_hash: IDL.Opt(IDL.Text), wasm_bytes: IDL.Opt(IDL.Nat64),
  arg_hex: IDL.Opt(IDL.Text), arg_hash: IDL.Opt(IDL.Text), controllers: IDL.Opt(IDL.Vec(IDL.Principal)), roles: IDL.Opt(Roles),
  proposed_ns: IDL.Nat64, executable_ns: IDL.Nat64, vetoes: IDL.Vec(IDL.Principal),
});
const Status = IDL.Record({
  target: IDL.Principal, roles: Roles, delay_secs: IDL.Nat64, min_delay_secs: IDL.Nat64, rehearsal_build: IDL.Bool,
  pending: IDL.Vec(ProposalView), target_epoch: IDL.Nat64, roles_epoch: IDL.Nat64, resume_votes: IDL.Vec(IDL.Principal),
  now_ns: IDL.Nat64,
  // canister review2 N-6 (fixes-R3-canister.md §6): the proposals being executed, with what cancelled them meanwhile. The timelock
  // sends a plain vec; read as opt so a timelock from before R3 (no field) still decodes (Candid: T <: opt T, absent → null)
  running: IDL.Opt(IDL.Vec(IDL.Tuple(ProposalView, IDL.Opt(IDL.Text)))),
});
const Event = IDL.Variant({
  Proposed: IDL.Record({ id: IDL.Nat64, at_ns: IDL.Nat64, view: ProposalView }),
  Cancelled: IDL.Record({ id: IDL.Nat64, at_ns: IDL.Nat64, by: IDL.Principal }),
  VetoVote: IDL.Record({ id: IDL.Nat64, at_ns: IDL.Nat64, by: IDL.Principal, votes: IDL.Nat8, needed: IDL.Nat8 }),
  Vetoed: IDL.Record({ id: IDL.Nat64, at_ns: IDL.Nat64, by: IDL.Vec(IDL.Principal) }),
  Executed: IDL.Record({ id: IDL.Nat64, at_ns: IDL.Nat64, by: IDL.Principal }),
  Failed: IDL.Record({ id: IDL.Nat64, at_ns: IDL.Nat64, by: IDL.Principal, why: IDL.Text }),
  Paused: IDL.Record({ at_ns: IDL.Nat64, paused: IDL.Bool, ok: IDL.Bool, why: IDL.Opt(IDL.Text) }),
  ResumeVote: IDL.Record({ at_ns: IDL.Nat64, by: IDL.Principal, votes: IDL.Nat8, needed: IDL.Nat8 }),
  RolesChanged: IDL.Record({ id: IDL.Nat64, at_ns: IDL.Nat64, roles: Roles }),
  // audit D, L-2: `execute` stops the buyer, installs, starts it again; either step failing is recorded
  NotStopped: IDL.Record({ id: IDL.Nat64, at_ns: IDL.Nat64, why: IDL.Text }),
  NotStarted: IDL.Record({ id: IDL.Nat64, at_ns: IDL.Nat64, why: IDL.Text }),
  // W14a: a large module's chunks were not uploaded to the buyer's chunk store (nothing stopped or installed; pending again)
  NotUploaded: IDL.Record({ id: IDL.Nat64, at_ns: IDL.Nat64, why: IDL.Text }),
  // canister review2 N-6: a guardian's start_target (an unknown tag fails the whole history decode, so it must be listed)
  Started: IDL.Record({ at_ns: IDL.Nat64, by: IDL.Principal, ok: IDL.Bool, why: IDL.Opt(IDL.Text) }),
});
const Cycles = IDL.Record({ balance: IDL.Nat, low_threshold: IDL.Nat });

export const timelockIdl = () => IDL.Service({
  status: IDL.Func([], [Status], ["query"]),
  history: IDL.Func([], [IDL.Vec(Event)], ["query"]),
  cycles: IDL.Func([], [Cycles], ["query"]),
});

// ---- the buyer: its install / upgrade argument -------------------------------------------------------------------------------
const CollectionCap = IDL.Record({ collection: IDL.Text, max_items: IDL.Nat32, max_value_wei: IDL.Nat, max_item_wei: IDL.Nat });
const CapPolicy = IDL.Record({ collections: IDL.Vec(CollectionCap), global_max_items: IDL.Nat32, global_max_value_wei: IDL.Nat });
const Limits = IDL.Record({
  per_collection_items: IDL.Nat32, global_items: IDL.Nat32, per_collection_mints: IDL.Nat32, global_mints: IDL.Nat32,
  global_value_wei: IDL.Nat, trip_factor: IDL.Nat32,
});
const AllowedZone = IDL.Record({ zone: IDL.Text, codehash: IDL.Text });
const EvmChain = IDL.Record({
  chain_id: IDL.Nat64, rpc_urls: IDL.Vec(IDL.Text), bucket: IDL.Nat64, seaport: IDL.Text, seaport_codehash: IDL.Text,
  max_fee_wei: IDL.Nat64, priority_fee_wei: IDL.Nat64, gas_cap: IDL.Nat64, fee_recipients: IDL.Vec(IDL.Text),
  max_fee_bps: IDL.Nat32, min_order_life_s: IDL.Nat64, signed_zones: IDL.Opt(IDL.Vec(AllowedZone)),
  zone_registries: IDL.Opt(IDL.Vec(IDL.Text)),
});
const FloorParams = IDL.Record({ min_listings: IDL.Nat8, listing_rank: IDL.Nat8, min_sales: IDL.Nat8, up_cap_bps: IDL.Nat16, down_cap_bps: IDL.Nat16 });
// 2026-10-12 (Robinhood Chain NFTs): a collection names its chain (none: eth_nft.chain_id) and whether its NFTs are token-bound accounts
const EthCollection = IDL.Record({ collection: IDL.Text, wrapper: IDL.Text, module: IDL.Text, max_purchase_wei: IDL.Nat, max_premium_bps: IDL.Nat16,
  chain_id: IDL.Opt(IDL.Nat64), token_bound: IDL.Opt(IDL.Bool), nft_value_bps: IDL.Opt(IDL.Nat16) });
// docs/v7/dormant-collections.md §3.2: the caps on wakes, the idle rule and the 4663 WakeDesk
// §11: the limits scale with the canister's cycles balance under max_active / wakes_per_day (in B cycles a day or once; the reserve ≥ 8 T)
const Scaling = IDL.Record({ active_b_per_day: IDL.Nat64, idle_b_per_day: IDL.Nat64, wake_b: IDL.Nat64, runway_days: IDL.Nat16, reserve_b: IDL.Nat64,
  exit_b_per_awake: IDL.Nat64 });
const Dormancy = IDL.Record({ max_active: IDL.Nat8, max_waking: IDL.Nat8, wakes_per_day: IDL.Nat8, wake_timeout_secs: IDL.Nat64,
  cooldown_secs: IDL.Nat64, idle_days: IDL.Nat16, wake_desk: IDL.Opt(IDL.Text), scaling: IDL.Opt(Scaling) });
export const EthNftConfig = IDL.Record({ chain_id: IDL.Nat64, collections: IDL.Vec(EthCollection), relay_max_fee_bps: IDL.Nat16, deposits: IDL.Bool,
  floor: IDL.Opt(FloorParams), gas_pot_target_wei: IDL.Opt(IDL.Nat), logs_max_blocks: IDL.Opt(IDL.Nat64), dormancy: IDL.Opt(Dormancy) });
// docs/v9/solana-migration.md §2.4: the Solana home (identity: cluster, rook_core = "books" (decision D-l: no program), nonce_seed;
// operational: rpc_urls, blockhash_fallback)
export const Home = IDL.Variant({
  Solana: IDL.Record({ cluster: IDL.Text, rpc_urls: IDL.Vec(IDL.Text), rook_core: IDL.Text, nonce_seed: IDL.Text, blockhash_fallback: IDL.Opt(IDL.Bool),
    // retired with rook_backing (decision D-l): null; kept so an earlier argument still decodes
    rook_backing: IDL.Opt(IDL.Text),
    // decision D-i (2026-10-10): the drand relays the registry keeper asks (operational; null = the canister's defaults)
    drand_urls: IDL.Opt(IDL.Vec(IDL.Text)) }),
});
export const Init = IDL.Record({
  chain_id: IDL.Nat64, vault: IDL.Text, rpc_urls: IDL.Vec(IDL.Text), btc_api_urls: IDL.Vec(IDL.Text), key_name: IDL.Text,
  accept_production_key_name: IDL.Bool, bridge_allowlist: IDL.Vec(IDL.Text), btc_network: IDL.Text,
  operator: IDL.Opt(IDL.Principal), wrapper: IDL.Opt(IDL.Text), evm_chains: IDL.Opt(IDL.Vec(EvmChain)),
  custody_caps: IDL.Opt(CapPolicy), outflow_limits: IDL.Opt(Limits),
  // audit D, H-2 and H-1 (fixes-D-canister.md): holder deposits off unless true; where the ticket-log scanner first reads
  holder_deposits: IDL.Opt(IDL.Bool), wrapper_from_block: IDL.Opt(IDL.Nat64),
  // docs/v6/relay-conversion.md §3.5: how a frog is paid for; null = Relay at 300 bps with no operator float
  frog_funding: IDL.Opt(IDL.Record({ max_fee_bps: IDL.Nat16, float_first: IDL.Bool })),
  // docs/v7/phase1-nft.md §4.1: the Ethereum NFT side; null = inert
  eth_nft: IDL.Opt(EthNftConfig),
  // docs/v9/solana-migration.md §4: a non-EVM home (Solana: rook_core's program, its providers, the line's nonce seed); null = the EVM home
  home: IDL.Opt(Home),
});

/** Every field name reachable from `Init`: the argument may carry these and nothing else. */
export const INIT_FIELDS = ["chain_id", "vault", "rpc_urls", "btc_api_urls", "key_name", "accept_production_key_name",
  "bridge_allowlist", "btc_network", "operator", "wrapper", "evm_chains", "custody_caps", "outflow_limits", "holder_deposits", "wrapper_from_block",
  "frog_funding", "float_first",
  "eth_nft", "module", "max_purchase_wei", "max_premium_bps", "relay_max_fee_bps", "deposits", "floor", "min_listings", "listing_rank", "min_sales",
  "up_cap_bps", "down_cap_bps", "gas_pot_target_wei", "logs_max_blocks", "token_bound", "nft_value_bps",
  "dormancy", "max_active", "max_waking", "wakes_per_day", "wake_timeout_secs", "cooldown_secs", "idle_days", "wake_desk",
  "scaling", "active_b_per_day", "idle_b_per_day", "wake_b", "runway_days", "reserve_b", "exit_b_per_awake",
  "collection", "max_items", "max_value_wei", "max_item_wei", "collections", "global_max_items", "global_max_value_wei",
  "per_collection_items", "global_items", "per_collection_mints", "global_mints", "global_value_wei", "trip_factor",
  "zone", "codehash", "bucket", "seaport", "seaport_codehash", "max_fee_wei", "priority_fee_wei", "gas_cap", "fee_recipients",
  "max_fee_bps", "min_order_life_s", "signed_zones", "zone_registries",
  // docs/v9/solana-migration.md §4: the Solana home (the variant's tag and its record's fields; rpc_urls is named above)
  "home", "Solana", "cluster", "rook_core", "nonce_seed", "blockhash_fallback", "rook_backing", "drand_urls"];
const KNOWN_INIT_LABELS = new Map(INIT_FIELDS.map((f) => [idlLabelToId(f), f]));

// ---- the buyer: the queries the canary and the watcher read ------------------------------------------------------------------
const Blob = IDL.Vec(IDL.Nat8);
const Outpoint = IDL.Record({ txid: Blob, vout: IDL.Nat32 });
const T = IDL.Record({ txid: Blob });
// audit D, M-1: a delivery re-priced keeps every earlier version's txid; H8's conflict-out spends the fee input back to custody
const Bump = IDL.Record({ first_at: IDL.Nat64, last_at: IDL.Nat64, fee: IDL.Nat64, earlier: IDL.Vec(Blob) });
const TB = IDL.Record({ txid: Blob, bump: IDL.Opt(Bump) });
// canister review2 M-1 (b): the conflict-out is re-priced every 2 h; every earlier version's txid is kept, and all pay custody
const ConflictTx = IDL.Record({ txid: Blob, fee: IDL.Nat64, signed_at: IDL.Opt(IDL.Nat64), earlier: IDL.Opt(IDL.Vec(Blob)) });
const Conflict = IDL.Record({ leaked_txid: Blob, tx: IDL.Opt(ConflictTx), earlier: IDL.Opt(IDL.Vec(Blob)) });
const RPhase = IDL.Variant({
  Owed: IDL.Null, Signed: TB, Sent: TB, Confirmed: T, AwaitingRecord: T, Recording: T, Done: T,
  Refused: IDL.Record({ reason: IDL.Text }), LostRace: T, Stuck: IDL.Record({ why: IDL.Text }), Relist: IDL.Null,
  Conflicting: IDL.Record({ reason: IDL.Text, conflict: Conflict }),
  // docs/v9/solana-migration.md (phase B2): on the Solana home, record_delivery out on the canister-wide Solana line as `intent`
  SolRecording: IDL.Record({ txid: Blob, intent: IDL.Nat64 }),
});
// canister review2 N-2 (the gated Wrapper): a refused redemption's markUndeliverable, from Due to Sent to Done
const Attest = IDL.Variant({
  Due: IDL.Record({ deep: IDL.Opt(Blob) }), Sent: IDL.Record({ hash: Blob, nonce: IDL.Nat64 }), Done: IDL.Record({ note: IDL.Text }),
  // phase B2: mark_undeliverable out on the Solana line
  SolSent: IDL.Record({ intent: IDL.Nat64 }),
});
const Redemption = IDL.Record({ ticket: IDL.Nat64, acquisition_id: IDL.Nat, destination: IDL.Text, phase: RPhase,
  inscription: IDL.Opt(IDL.Text), attest: IDL.Opt(Attest) });
// docs/v8/ow-protected.md §3.3: a protected buy's setup transaction (a float spend the canister signed), when it had one
const Bought = IDL.Record({ inscription: IDL.Text, outpoint: Outpoint, cost_wei: IDL.Nat, setup_txid: IDL.Opt(Blob) });
const WithBuy = IDL.Record({ buy: Bought });
const FPhase = IDL.Variant({
  Wanted: IDL.Record({}), Signed: WithBuy, Sent: WithBuy, Held: WithBuy, Minted: WithBuy, Settled: WithBuy, Stray: WithBuy,
  Stranded: WithBuy, Failed: IDL.Record({}),
  // audit D, L-5: a stale buy abandoned by spending one of its float inputs back to the float
  Abandoning: IDL.Record({ buy: Bought, conflict: Conflict }),
  // docs/v8/ow-protected.md §3.3: a protected Ordinals Wallet sale signed and handed back (an unknown tag fails the whole decode)
  Handed: IDL.Record({ buy: Bought, sale_txid: IDL.Text, at: IDL.Nat64 }),
});
const FrogTarget = IDL.Variant({ Sink: Blob, Desk: IDL.Nat64 });
const FrogRec = IDL.Record({ id: IDL.Nat64, target: IDL.Opt(FrogTarget), phase: FPhase });
const DPhase = IDL.Variant({
  Pending: IDL.Record({}), Ready: IDL.Record({ inscription: IDL.Text }), Minted: IDL.Record({ inscription: IDL.Text }),
  Refused: IDL.Record({}), Held: IDL.Record({}), Returning: T, Returned: T, NotAFrog: IDL.Null,
  // audit D, H-2/H-3
  ReturnConflicting: IDL.Record({ conflict: Conflict }), AwaitingProof: IDL.Record({ inscription: IDL.Text }), Parked: IDL.Record({ inscription: IDL.Text }),
});
const DepositRec = IDL.Record({ outpoint: Outpoint, phase: DPhase });
// docs/v6/relay-conversion.md §3.2: an advance turned into sats by Relay, one at a time
const ConvAsk = IDL.Record({ amount_wei: IDL.Nat, user: IDL.Text, recipient: IDL.Text, refund_to: IDL.Text, origin_chain_id: IDL.Nat64,
  destination_chain_id: IDL.Nat64, origin_currency: IDL.Text, destination_currency: IDL.Text });
const ConvOrder = { order_id: Blob, value: IDL.Nat, min_sats: IDL.Nat64, deadline: IDL.Nat64 };
const CPhase = IDL.Variant({
  NeedAdvance: IDL.Null, Advancing: IDL.Null, Funded: IDL.Record({ wei: IDL.Nat, since: IDL.Nat64 }), Depositing: IDL.Record(ConvOrder),
  Converting: IDL.Record({ ...ConvOrder, since: IDL.Nat64, balance_after: IDL.Nat }), Arrived: IDL.Record({ order_id: Blob, sats: IDL.Nat64 }),
  Returning: IDL.Record({ wei: IDL.Nat }), Returned: IDL.Record({ wei: IDL.Nat }), Settled: IDL.Record({ cost: IDL.Nat, spare_sats: IDL.Nat64 }),
  Stuck: IDL.Record({ why: IDL.Text }),
});
const ConvRec = IDL.Record({ frog: IDL.Nat64, module: Blob, advanced: IDL.Nat, attempts: IDL.Nat32, phase: CPhase, orders: IDL.Vec(Blob),
  deposits: IDL.Vec(Blob), arrival: IDL.Opt(IDL.Record({ outpoint: Outpoint, sats: IDL.Nat64 })), ask: IDL.Opt(ConvAsk), note: IDL.Text,
  // review C-01/C-03/C-08 (the `late` watch is not decoded here)
  arrived_at: IDL.Opt(IDL.Nat64), abandoned_sats: IDL.Opt(IDL.Nat64), returned_at: IDL.Opt(IDL.Nat64) });
const At = IDL.Record({ inscription: IDL.Text, outpoint: Outpoint });
const SPhase = IDL.Variant({
  Planned: IDL.Null, Redeeming: IDL.Record({}), Pinned: IDL.Null, Listing: IDL.Record({}), Listed: At, Recorded: At,
  Sold: IDL.Record({ inscription: IDL.Text, fill_txid: Blob }), Reported: IDL.Record({}), Delisting: At,
  Moving: IDL.Record({ inscription: IDL.Text, outpoint: Outpoint, txid: Blob }), Redeposit: At, Reclaiming: IDL.Null,
  Returned: IDL.Null, Failed: IDL.Record({}), Stuck: IDL.Record({}),
});
const SaleState = IDL.Record({ sales: IDL.Vec(IDL.Record({ id: IDL.Nat64, phase: SPhase })) });
const Flow = IDL.Variant({ Item: IDL.Null, Mint: IDL.Null, Value: IDL.Null, Return: IDL.Null });     // Return: audit D, H-2
const Act = IDL.Record({ key: IDL.Text, collection: IDL.Text, flow: Flow, amount: IDL.Nat, at: IDL.Nat64 });
const Cause = IDL.Variant({
  Tripwire: IDL.Record({ collection: IDL.Text, flow: Flow, demanded: IDL.Nat, limit: IDL.Nat }),
  Manual: IDL.Record({ note: IDL.Text }),
});
const Freeze = IDL.Variant({ Open: IDL.Null, Frozen: IDL.Record({ since: IDL.Nat64, cause: Cause }) });
// `acked`: audit D, H-2.4 — the keys a resume acknowledged (they wait on the limit but no longer count toward the trip)
const Outflow = IDL.Record({ freeze: Freeze, acked: IDL.Opt(IDL.Vec(Act)) });
// canister review2 N-1: the response cap in use, the end of a range known too big, a block read by ticket, and tickets whose
// events no provider could return (each loud: its delivery waits and refunds)
const Dense = IDL.Record({ block: IDL.Nat64, after: IDL.Opt(IDL.Nat64), shrink: IDL.Nat32, fails: IDL.Nat32 });
const TicketLogs = IDL.Record({ cursor: IDL.Opt(IDL.Nat64), span: IDL.Nat64, idle_through: IDL.Opt(IDL.Nat64), finalized: IDL.Opt(IDL.Nat64),
  failures: IDL.Nat32, last_error: IDL.Opt(IDL.Text), cap: IDL.Opt(IDL.Nat64), hot: IDL.Opt(IDL.Nat64), dense: IDL.Opt(Dense),
  dense_blocks: IDL.Opt(IDL.Nat64), unreadable: IDL.Opt(IDL.Vec(IDL.Tuple(IDL.Nat64, IDL.Nat64))) });
// canister review2 N-4: an entry that fails three reads while others read is quarantined (kept, visible here)
const FeeEntry = IDL.Record({ outpoint: Outpoint, value: IDL.Nat64, source: IDL.Text, fails: IDL.Opt(IDL.Nat32) });
const FeePool = IDL.Record({ entries: IDL.Vec(FeeEntry), quarantined: IDL.Opt(IDL.Vec(FeeEntry)) });

// docs/v7/phase1-nft.md §4.6: the Ethereum NFT side's state (the fields read; record subtyping skips the rest)
const R = (o) => IDL.Record(o);
const BuyTok = { arrived: IDL.Nat, token: Blob, total: IDL.Nat, tx: Blob };
const BPhase = IDL.Variant({
  NeedAdvance: IDL.Null, Advancing: IDL.Null, Funded: R({ wei: IDL.Nat, since: IDL.Nat64 }),
  Depositing: R({ order_id: Blob, value: IDL.Nat, min_out: IDL.Nat, deadline: IDL.Nat64 }),
  Bridging: R({ order_id: Blob, value: IDL.Nat, min_out: IDL.Nat, deadline: IDL.Nat64, since: IDL.Nat64 }),
  Arrived: R({ wei: IDL.Nat, since: IDL.Nat64 }), Buying: R(BuyTok), Bought: R({ ...BuyTok, block: IDL.Nat64 }), Final: R(BuyTok),
  Minting: R(BuyTok), Minted: R(BuyTok), Settling: R({ token: Blob, total: IDL.Nat, tx: Blob, cost: IDL.Nat }),
  Settled: R({ token: Blob, total: IDL.Nat, tx: Blob, cost: IDL.Nat }), Returning: R({ wei: IDL.Nat }), Returned: R({ wei: IDL.Nat }),
  Stuck: R({ why: IDL.Text }),
  // audit 2026-09-27-nft round 2 (A2-M-1): an arrived buy's ETH going back (Relay Ethereum → 4663, then returnAdvance)
  BackFunded: R({ wei: IDL.Nat, since: IDL.Nat64 }),
  BackSending: R({ wei: IDL.Nat, order_id: Blob, value: IDL.Nat, min_out: IDL.Nat, deadline: IDL.Nat64, tx: Blob }),
  BackBridging: R({ order_id: Blob, value: IDL.Nat, min_out: IDL.Nat, deadline: IDL.Nat64, since: IDL.Nat64 }),
});
// A2-L-1 (2026-09-27): `sweep` marks a gas-pot sweep (leftover back to its module through the way back, then fundReserve), not a buy
// review round 4 (C4-M-1): `late` is a Relay order written off unpaid (its watch kept, a late payout credited to it by its transaction)
// review round 5 (C5-M-1): `released_at`: the operator released its hold on the Relay leg; (D5-L-2) `last`: the open watch, if any
const LateOrder = R({ watch: R({ order_id: Blob, value: IDL.Nat, deadline: IDL.Nat64 }), back: IDL.Bool, let_go_at: IDL.Nat64,
  credited: IDL.Opt(IDL.Tuple(Blob, IDL.Nat, IDL.Bool)), released_at: IDL.Opt(IDL.Nat64) });
const EthBuy = R({ id: IDL.Nat64, collection: Blob, module: Blob, wrapper: Blob, advanced: IDL.Nat, attempts: IDL.Nat32, phase: BPhase, note: IDL.Text,
  sweep: IDL.Opt(IDL.Bool), late: IDL.Opt(LateOrder), last: IDL.Opt(R({ order_id: Blob })) });
const Why = R({ why: IDL.Text });
const DelPhase = IDL.Variant({
  Waiting: IDL.Null, Sent: R({ tx: Blob, nonce: IDL.Nat64 }), Moved: R({ tx: Blob, block: IDL.Nat64 }), Record: R({ tx: Blob }),
  Recording: R({ tx: Blob }), Recorded: R({ tx: Blob }), Cancelling: R({ of: Blob, tx: Blob, nonce: IDL.Nat64 }),
  MarkAfter: R({ why: IDL.Text, block: IDL.Nat64 }), Mark: Why, Marking: Why, Marked: Why, Gone: Why,
});
// round 3 (D3-H-1): `sent` is true once the wrapper's noteSent mined (the late-window refund is then off); an older state without the
// field decodes as none (record subtyping: an absent opt field is null)
const Delivery = R({ wrapper: Blob, collection: Blob, ticket: IDL.Nat, token: Blob, destination: Blob, deliver_by: IDL.Nat64, phase: DelPhase, note: IDL.Text,
  sent: IDL.Opt(IDL.Bool) });
const EthKind = IDL.Variant({ Buy: R({ buy: IDL.Nat64 }), Deliver: R({ wrapper: Blob, ticket: IDL.Nat }), Cancel: R({ wrapper: Blob, ticket: IDL.Nat }),
  Return: R({ tx: Blob, token: Blob }), CancelFill: R({ buy: IDL.Nat64 }), BackDeposit: R({ buy: IDL.Nat64 }) });
// holder deposits and strays (docs/v7/phase1-nft.md §2.4, D5)
const XPhase = IDL.Variant({ Seen: IDL.Null, Mint: IDL.Null, Minting: IDL.Null, Minted: IDL.Null, Return: Why, Returning: R({ tx: Blob }),
  Returned: R({ tx: Blob }), Held: Why, Refused: Why });
const EthDepositRec = R({ tx: Blob, collection: Blob, token: Blob, recipient: Blob, from: Blob, block: IDL.Nat64, phase: XPhase });
// round 2 (D2-M-1): what the line's transaction pays out and its gas bound, for the outbound canary's earmark once its nonce is used
const EthTx = R({ kind: EthKind, nonce: IDL.Nat64, hash: Blob, value: IDL.Nat, max_fee: IDL.Nat, gas: IDL.Nat64 });
const FloorPost = R({ price: IDL.Nat, confidence: IDL.Nat16, listing: IDL.Nat, sale_median: IDL.Nat });
const Reading = R({ collection: Blob, post: FloorPost, at: IDL.Nat64 });
// audit 2026-09-27-nft D-M-3: our last mined Ethereum transaction (nonce, block), and a halted line
// review round 5 (F5-M-1): the pending 4663 transaction, and since when it is seen nowhere; review round 6 (D6-L-1 / D6-L-2): its step
// (the page says what the operator can do for it) and when it was signed (paged on its age whatever the providers show)
const EthStep = IDL.Variant({ Advance: R({ buy: IDL.Nat64 }), Deposit: R({ buy: IDL.Nat64 }), GiveBack: R({ buy: IDL.Nat64 }), Mint: R({ buy: IDL.Nat64 }),
  Settle: R({ buy: IDL.Nat64 }), Record: R({ wrapper: Blob, ticket: IDL.Nat }), Mark: R({ wrapper: Blob, ticket: IDL.Nat }), Post: R({ collection: Blob }),
  MintDeposit: R({ tx: Blob }), Sent: R({ wrapper: Blob, ticket: IDL.Nat }),
  // docs/v7/dormant-collections.md §5: FloorModule.fill(coin) of a graduation queued while the module was stale
  Fill: R({ collection: Blob, coin: Blob }),
  // independent review DI-M-1: WakeDesk.markServed(id): the post that served a desk request, recorded inside its window
  Served: R({ collection: Blob, id: IDL.Nat64 }) });
const HomeTx = R({ step: EthStep, hash: Blob, nonce: IDL.Nat64, used_since: IDL.Opt(IDL.Nat64), sent_at: IDL.Opt(IDL.Nat64) });
// review round 6 (B6-M-1 / D6-I-1): the canister's own sale scan per collection: how far it is behind, what a re-anchor passed over, what the
// providers refused as too deep, the receipts it cannot read
// review round 8 (B8-L-1): `down`, where the walk back to the window's start has got to (the monitor pages it stuck above the start)
const SaleScan = R({ collection: Blob, down: IDL.Nat64, up: IDL.Nat64, hole: IDL.Opt(IDL.Tuple(IDL.Nat64, IDL.Nat64)), lost: IDL.Opt(IDL.Tuple(IDL.Nat64, IDL.Nat64)),
  behind: IDL.Opt(IDL.Nat64), rfail: IDL.Opt(IDL.Vec(IDL.Tuple(Blob, IDL.Nat32, IDL.Nat64))),
  // review round 7 (B7-L-3 / B7-L-1 / B7-L-2): the final head the scan was last current at, the hole refused as too deep by every provider
  // (first at, answers, not before), receipts answered as a rate limit (tx, answers in a row, not before), passes in a row that met one
  current: IDL.Opt(IDL.Nat64), hole_deep: IDL.Opt(IDL.Tuple(IDL.Nat64, IDL.Nat32, IDL.Nat64)),
  rbusy: IDL.Opt(IDL.Vec(IDL.Tuple(Blob, IDL.Nat32, IDL.Nat64))), busy: IDL.Opt(IDL.Tuple(IDL.Nat64, IDL.Nat32)) });
// docs/v7/dormant-collections.md: each collection's life (the monitor skips the floor and poster checks of one that is not ACTIVE, and pages
// a wake that is slow or timed out)
const LifePhase = IDL.Variant({ Dormant: IDL.Null, Waking: R({ since: IDL.Nat64, backfilled_at: IDL.Opt(IDL.Nat64) }), Active: R({ since: IDL.Nat64 }) });
const WakeBy = IDL.Variant({ Desk: R({ id: IDL.Nat64, requester: Blob }), Operator: IDL.Null, Deposit: R({ tx: Blob }), Graduation: IDL.Null });
const Life = R({ collection: Blob, phase: LifePhase, since: IDL.Nat64, cooldown_until: IDL.Nat64, ever_active: IDL.Bool, note: IDL.Text,
  queued: IDL.Opt(IDL.Tuple(WakeBy, IDL.Nat64)) });
// §11: the capacity as of the canister's last tick (how many can be awake on its cycles balance, how many are, how many hold positions)
const Capacity = R({ at: IDL.Nat64, balance: IDL.Nat, scaling: IDL.Bool, collections: IDL.Nat8, ceiling: IDL.Nat8, wakes_per_day: IDL.Nat8,
  effective_max_active: IDL.Nat8, awake: IDL.Nat8, pinned: IDL.Nat8, wake_headroom: IDL.Nat8, wakes_left_today: IDL.Nat8, next_wake_at: IDL.Nat, status: IDL.Text });
const Dorm = R({ lives: IDL.Vec(Life), wakes: IDL.Vec(IDL.Nat64), cap: IDL.Opt(Capacity) });
// docs/v9/solana-migration.md §4 (phase B1): the Solana home's one line: an incident (its nonce moved under a transaction that can never
// land), a first send every provider's preflight refused (`error`), the transaction in flight and the step it is
const SolSaved = R({ intent: IDL.Nat64, what: IDL.Text, signature: IDL.Text, sends: IDL.Nat32 });
const SolLine = R({ nonce_account: IDL.Opt(Blob), saved: IDL.Opt(SolSaved), incident: IDL.Opt(IDL.Text), error: IDL.Opt(IDL.Text) });
const SolHome = R({ line: SolLine, pending: IDL.Opt(R({ step: EthStep, intent: IDL.Nat64, at: IDL.Nat64 })), opening: IDL.Opt(IDL.Nat64) });
// docs/v9/solana-migration.md §2.4 (phase B2): the ONE line canister-wide (the `sol_line` query; B1 kept it in EthState.sol): the
// steps of every flow on it (the Ethereum side's, the frogs', the custodian's), its pending step, what landed for a flow not yet taken,
// and the flows waiting their turn
const N = IDL.Nat64;
const FStep = IDL.Variant({ Mint: R({ frog: N }), Report: R({ frog: N }), Fulfill: R({ frog: N, request: N }), Refunds: R({ sink: Blob }),
  Settle: R({ sink: Blob }), Deposit: R({ outpoint: Outpoint }), FloorClaim: R({ want: N }), Claim: R({ board: Blob, id: N }), Slash: R({ board: Blob, id: N }),
  Relist: R({ sale: N, count_before: N }), SaleRecord: R({ sale: N }), SaleReport: R({ sale: N }), SaleDeposit: R({ sale: N }), SaleReclaim: R({ sale: N }),
  Advance: R({ frog: N }), Convert: R({ frog: N }), GiveBack: R({ frog: N }), LateCredit: R({ frog: N }), Post: IDL.Null });
const CustodyStep = IDL.Variant({ Record: R({ ticket: N, txid: Blob }), Mark: R({ ticket: N }), Sent: R({ ticket: N, txid: Blob }) });
// W3 (docs/v9/canister-only-solana.md §13.1): the mint/burn desk's and the pool desk's steps (`Op`) on their own lines
const LineOwner = IDL.Variant({ Eth: IDL.Null, Frog: IDL.Null, Custody: IDL.Null, Desk: IDL.Null, Pool: IDL.Null, Coin: IDL.Null, Treasury: IDL.Null,
  SolNft: IDL.Null });
const LineOp = R({ owner: LineOwner, what: IDL.Text, id: N });
const CoinStep = IDL.Variant({ OpenMint: R({ coll: IDL.Nat32 }), OpenConfig: R({ coll: IDL.Nat32 }), Migrate: R({ coin: Blob }), MigrationFee: R({ coin: Blob }),
  Surplus: R({ coin: Blob }), Leftover: R({ coin: Blob }), BurnLeftover: R({ coin: Blob, amount: N }), ClaimCurveFee: R({ coin: Blob, units: N }),
  ClaimLpFee: R({ coin: Blob, units: N }), CashFees: R({ coll: IDL.Nat32, burn: N, to_desk: N, link: N }),
  Redeem: R({ n: N, coin: Blob, amount: N, payout: N, link: N }) });
const PoolStep = IDL.Variant({ OpenPair: R({ coll: IDL.Nat32, active: IDL.Int32, pair: Blob }), OpenArrays: R({ coll: IDL.Nat32, indexes: IDL.Vec(IDL.Int64) }),
  OpenPosition: R({ coll: IDL.Nat32, lower: IDL.Int32, width: IDL.Int32, position: Blob }), Mint: R({ coll: IDL.Nat32, units: N, lamports: N, link: N }),
  Add: R({ coll: IDL.Nat32, ax: N, ay: N, lo: IDL.Int32, hi: IDL.Int32 }), ArbUp: R({ coll: IDL.Nat32, units: N, lamports: N, min_out: N, link: N, profit: N }),
  ArbDown: R({ coll: IDL.Nat32, lamports: N, min_out: N, profit: N }), Burn: R({ coll: IDL.Nat32, units: N, paid: N, link: N }),
  Recenter: R({ coll: IDL.Nat32, active: IDL.Int32, lo: IDL.Int32, hi: IDL.Int32 }), Withdraw: R({ coll: IDL.Nat32, lo: IDL.Int32, hi: IDL.Int32 }),
  Move: R({ coll: IDL.Nat32, target: IDL.Int32 }), Claim: R({ coll: IDL.Nat32, lo: IDL.Int32, hi: IDL.Int32 }), Sweep: R({ coll: IDL.Nat32, lamports: N }),
  Unwrap: R({ coll: IDL.Nat32, lamports: N }) });
const TreasuryStep = IDL.Variant({ Surplus: R({ coll: IDL.Nat32, lamports: N, link: N }), Ops: R({ to: Blob, lamports: N }), Prize: R({ coll: IDL.Nat32, price: N, link: N }),
  Claim: R({ coll: IDL.Nat32, winner: Blob, round: N }), ClaimerSweep: R({ lamports: N }), Skim: R({ coll: IDL.Nat32, lamports: N, link: N }),
  Remint: R({ coll: IDL.Nat32, ticket: N, owner: Blob, link: N }) });
// W8c (docs/v9/solana-nfts-canister.md §1): the Solana NFT flow's steps on the money line
const SolNftVenue = IDL.Variant({ Tensor: IDL.Null, Mmm: IDL.Null, M2: IDL.Null });
const SolNftStep = IDL.Variant({ Buy: R({ coll: IDL.Nat16, asset: Blob, venue: SolNftVenue, cost: N, link: N }),
  Deliver: R({ coll: IDL.Nat16, ticket: N, to: Blob }),
  // 2026-10-12: the canister's address lookup table for Magic Eden M2 purchases (solnftflow::table), created on the money line
  Table: R({ coll: IDL.Nat16, key: Blob }) });
const LineStep = IDL.Variant({ Eth: EthStep, Frog: FStep, Custody: CustodyStep, Op: LineOp, Coin: CoinStep, Pool: PoolStep, Treasury: TreasuryStep,
  SolNft: SolNftStep });
const LinePending = R({ step: LineStep, intent: N, at: N });
const LineOutcome = R({ step: LineStep, intent: N, ok: IDL.Opt(IDL.Bool), signature: IDL.Text, at: N });
export const HomeLine = R({ line: SolLine, pending: IDL.Opt(LinePending), next_intent: N, opening: IDL.Opt(N), outcomes: IDL.Vec(LineOutcome),
  queue: IDL.Vec(IDL.Tuple(LineOwner, N)) });
// W3: the four send lines (money flows, the cranks, the mint/burn desk, the pool desk), each its own durable nonce; W14b: the
// canister's own timer as it last ran, and whether the operator switched it off
const LineId = IDL.Variant({ Money: IDL.Null, Crank: IDL.Null, Desk: IDL.Null, Pool: IDL.Null, Coin: IDL.Null, Treasury: IDL.Null });
const TimerSeen = R({ last_run: N, next_at: N, runs: N, skipped: N, busy: IDL.Bool });
const SolLinesReply = [IDL.Vec(IDL.Tuple(LineId, IDL.Opt(HomeLine))), IDL.Opt(TimerSeen), IDL.Bool];
export const EthState = R({ buys: IDL.Vec(EthBuy), deliveries: IDL.Vec(Delivery), readings: IDL.Vec(Reading), line: IDL.Opt(EthTx), eth_out: IDL.Nat,
  deposits: IDL.Vec(EthDepositRec), last_mined: IDL.Opt(IDL.Tuple(IDL.Nat64, IDL.Nat64)), halted: IDL.Opt(IDL.Text), pending: IDL.Opt(HomeTx),
  sale_scan: IDL.Opt(IDL.Vec(SaleScan)), dorm: IDL.Opt(Dorm), sol: IDL.Opt(SolHome),
  // phase B2: the Ethereum side's own step out on the canister-wide line
  sol_step: IDL.Opt(LinePending) });


// ---- W11 (canister-only): the books' views, generated from etch_autobuyer.did by the session's converter (keep in step with the did) --
const B_Incident = R({ at: N, coll: IDL.Opt(IDL.Nat16), link: IDL.Opt(N), what: IDL.Text });
const B_FindingKind = IDL.Variant({ SupplyMismatch: IDL.Null, Unbooked: IDL.Null, Invariant: IDL.Null, BooksUnbacked: IDL.Null, HolderBurn: IDL.Null });
const B_Finding = R({ coll: IDL.Nat16, chain: N, kind: B_FindingKind, note: IDL.Text, books: N });
const B_Report = R({ at: N, unexplained: IDL.Vec(IDL.Text), checked: IDL.Nat32, memos_seen: IDL.Nat32, findings: IDL.Vec(B_Finding), halted: IDL.Vec(IDL.Nat16) });
const B_DeskStatus = R({ incidents: IDL.Vec(B_Incident), report: IDL.Opt(B_Report), open: IDL.Nat32, collections: IDL.Nat32, last_reconcile: N, funded: IDL.Nat32, needing_attention: IDL.Nat32, open_links: IDL.Nat32, executing: IDL.Nat32, halted: IDL.Vec(IDL.Tuple(IDL.Nat16, IDL.Text)), coin_redeem: IDL.Bool, paused: IDL.Bool });
const B_DeskView = R({ hi: IDL.Int32, lo: IDL.Int32, wx: N, read_at: N, active: IDL.Int32, native: N, coll: IDL.Nat32, pair: IDL.Text, wsol: N, fee_lamports: N, swept: N, fee_units: N, pos_x: N, pos_y: N, position: IDL.Text, arb_profit: N, arbs_last_hour: IDL.Nat32 });
const B_FeeBook = R({ to_desk_lamports: N, to_desk_burned_wx: N, lp_wx: N, cashed_lamports: N, curve_wx: N, cashed_wx: N, to_desk_wx: N });
const B_Owed = IDL.Variant({ Ops: R({ lamports: N }), Surplus: R({ lamports: N, coll: IDL.Nat32 }) });
const B_PendingClaim = R({ weight: N, owner: Blob, proof: IDL.Vec(Blob), cum_start: N, round: N });
const B_PendingCommit = R({ root: Blob, total_weight: N, entries: IDL.Nat32, taken_at: N, round: N, snapshot_slot: N });
const B_Resume = IDL.Variant({ Collection: IDL.Null, Desk: IDL.Null, AllMints: IDL.Null });
const B_Treasury = R({ ops: IDL.Opt(Blob), round_total: N, round_paid: N, pending_raffle_bps: IDL.Nat16, round_weights: IDL.Vec(N), total_to_reserves: N, total_to_raffles: N, reserves_bps: IDL.Nat16, reserves_pot: N, ops_bps: IDL.Nat16, total_received: N, collections: IDL.Vec(IDL.Nat16), total_allocated: N, pending_ops_at: IDL.Int64, pending_raffle_at: IDL.Int64, pending_reserve_cap_bps: IDL.Nat32, pending_ops_bps: IDL.Nat16, reserve_cap_bps: IDL.Nat32, raffle_bps: IDL.Nat16, round_pot: N, round_started_at: IDL.Int64, unallocated: N, pending_shares_at: IDL.Int64, pending_ops: IDL.Opt(Blob), total_to_ops: N, pending_reserve_cap_at: IDL.Int64, pending_reserves_bps: IDL.Nat16 });
const B_Wait_1 = R({ key: IDL.Text, why: IDL.Text, tries: IDL.Nat32, retry_at: N, since: N });
const B_TreasuryStatus = R({ off: IDL.Bool, claims: IDL.Vec(IDL.Tuple(IDL.Nat32, B_PendingClaim)), drand_disagreement: IDL.Opt(IDL.Tuple(N, N, IDL.Text)), last: IDL.Vec(IDL.Tuple(IDL.Text, N, IDL.Text)), owed: IDL.Vec(B_Owed), resumes: IDL.Vec(IDL.Tuple(B_Resume, IDL.Nat32, N)), commits: IDL.Vec(IDL.Tuple(IDL.Nat32, B_PendingCommit)), waits: IDL.Vec(B_Wait_1), treasury: B_Treasury });
const B_Wait = R({ key: IDL.Text, why: IDL.Text, tries: IDL.Nat32, retry_at: N, since: N });
export const TREASURY_STATUS_REPLY = [B_TreasuryStatus];
export const POOL_DESKS_REPLY = [IDL.Vec(B_DeskView)];
// the 4th element: the opening queue (`coins_request_open`, oldest first: (collection, asked at); operator decision 2026-10-12)
export const COINS_LOG_REPLY = [IDL.Vec(IDL.Tuple(IDL.Nat32, B_FeeBook)), IDL.Vec(B_Wait), IDL.Vec(IDL.Tuple(IDL.Text, N, IDL.Text)), IDL.Vec(IDL.Tuple(IDL.Nat32, N))];
export const DESK_STATUS_REPLY = [B_DeskStatus];
// W15 (the canister-only rehearsal, ops/rehearsal/sol-devnet): the books of one collection, the listed collections and the pinned ones,
// generated from the did as the four above (the rehearsal's checklist reads them through the monitor's own decoder)
const B_ItemView = R({ key: IDL.Text, vout: IDL.Nat32, inscription_id: IDL.Text, deposit: IDL.Bool, source_tx: IDL.Text, item_id: IDL.Nat32 });
const B_ListingKind = IDL.Variant({ SolNft: R({ collection: Blob, custodian: Blob }), Inscriptions: R({ custody: IDL.Text, membership_root: Blob }),
  EthNft: R({ collection: Blob, custody: Blob, chain_id: N }) });
const B_TicketSummary = R({ id: N, destination: IDL.Text, deliver_by: IDL.Int64, undeliverable: IDL.Bool, stage: IDL.Text, item_id: IDL.Nat32 });
const B_BooksView = R({ fresh: IDL.Bool, transit_units: N, last_good_at: IDL.Int64, raffle_prize: N, expected_supply: IDL.Opt(N), reserve_units: N, revenue: N,
  poster_paused: IDL.Bool, advanced: N, coll: IDL.Nat16, held: N, kind: B_ListingKind, owed: N, wx_mint: IDL.Text, items_arrived: N, volume_7d: N, reserve: N,
  surplus: N, raffle_pot: N, target: N, tickets_open: IDL.Vec(B_TicketSummary), posted_at: IDL.Int64, module_units: N, mint_price: IDL.Opt(N), escaped: IDL.Bool,
  burn_pays_one: IDL.Opt(N), sol_backed: N, halted: IDL.Opt(IDL.Text), open_tickets: N, items: IDL.Vec(B_ItemView), accepted: N, burned: N, ticket_count: N,
  paused: IDL.Bool, backing: N, item_count: IDL.Nat32 });
const B_CollectionView = R({ coins: IDL.Nat32, backed: IDL.Nat32, wx_mint: IDL.Text, stage: IDL.Text, index: IDL.Nat32, module: IDL.Text, config: IDL.Text, symbol: IDL.Text });
export const BOOKS_VIEW_REPLY = [IDL.Opt(B_BooksView)];
export const BOOKS_COLLECTIONS_REPLY = [IDL.Vec(IDL.Tuple(IDL.Nat16, IDL.Text))];
export const COINS_COLLECTIONS_REPLY = [IDL.Vec(B_CollectionView)];

export const buyerIdl = () => IDL.Service({
  eth_nft_state: IDL.Func([], [EthState], ["query"]),
  // audit C3-05: the other chains' NFT sides (Robinhood Chain's collections) and the configuration that names their chains
  eth_nft_state_of: IDL.Func([IDL.Nat64], [IDL.Opt(EthState)], ["query"]),
  eth_nft_config: IDL.Func([], [IDL.Opt(EthNftConfig)], ["query"]),
  sol_line: IDL.Func([], [IDL.Opt(HomeLine)], ["query"]),
  sol_lines: IDL.Func([], SolLinesReply, ["query"]),
  treasury_status: IDL.Func([], TREASURY_STATUS_REPLY, ["query"]), pool_desks: IDL.Func([], POOL_DESKS_REPLY, ["query"]),
  coins_log: IDL.Func([], COINS_LOG_REPLY, ["query"]), desk_status: IDL.Func([], DESK_STATUS_REPLY, ["query"]),
  books_view: IDL.Func([IDL.Nat16], BOOKS_VIEW_REPLY, ["query"]), books_collections: IDL.Func([], BOOKS_COLLECTIONS_REPLY, ["query"]),
  coins_collections: IDL.Func([], COINS_COLLECTIONS_REPLY, ["query"]),
  cycles: IDL.Func([], [Cycles], ["query"]),
  paused: IDL.Func([], [IDL.Bool], ["query"]),
  redemptions: IDL.Func([], [IDL.Vec(Redemption)], ["query"]),
  holder_deposits: IDL.Func([], [IDL.Vec(DepositRec)], ["query"]),
  // Ordinals multi-collection: `opt nat16` names one Inscriptions listing; null (what the monitor sends) reads every collection's
  frog_buys: IDL.Func([IDL.Opt(IDL.Nat16)], [IDL.Vec(FrogRec)], ["query"]),
  frog_conversions: IDL.Func([IDL.Opt(IDL.Nat16)], [IDL.Vec(ConvRec)], ["query"]),
  sale_state: IDL.Func([], [SaleState], ["query"]),
  purchases: IDL.Func([], [IDL.Vec(IDL.Tuple(IDL.Nat64, IDL.Text))], ["query"]),
  custody_caps: IDL.Func([], [CapPolicy, IDL.Vec(IDL.Record({}))], ["query"]),
  custody_outflow: IDL.Func([], [Limits, Outflow], ["query"]),
  ticket_logs: IDL.Func([], [IDL.Opt(TicketLogs)], ["query"]),
  custody_fee_pool: IDL.Func([], [FeePool], ["query"]),
  log: IDL.Func([], [IDL.Vec(IDL.Text)], ["query"]),
  ord_providers: IDL.Func([], [IDL.Vec(IDL.Tuple(IDL.Text, IDL.Text))], ["query"]),
});

/** The reply types by method, for decoding saved replies (the tests' Rust-encoded fixtures) exactly as the actor would. */
export const REPLY_TYPES = {
  redemptions: [IDL.Vec(Redemption)], holder_deposits: [IDL.Vec(DepositRec)], frog_buys: [IDL.Vec(FrogRec)],
  sale_state: [SaleState], custody_caps: [CapPolicy, IDL.Vec(IDL.Record({}))], custody_outflow: [Limits, Outflow],
  ticket_logs: [IDL.Opt(TicketLogs)], custody_fee_pool: [FeePool], frog_conversions: [IDL.Vec(ConvRec)], eth_nft_state: [EthState],
  sol_line: [IDL.Opt(HomeLine)], sol_lines: SolLinesReply, eth_nft_state_of: [IDL.Opt(EthState)], eth_nft_config: [IDL.Opt(EthNftConfig)],
  treasury_status: TREASURY_STATUS_REPLY, pool_desks: POOL_DESKS_REPLY, coins_log: COINS_LOG_REPLY, desk_status: DESK_STATUS_REPLY,
  books_view: BOOKS_VIEW_REPLY, books_collections: BOOKS_COLLECTIONS_REPLY, coins_collections: COINS_COLLECTIONS_REPLY,
};

// ---- plain values ---------------------------------------------------------------------------------------------------------
const hex = (b) => Buffer.from(b).toString("hex");
/** Hex to a Uint8Array of its own. Not a Node Buffer: a small Buffer is a view into a shared pool, and the Candid reader
 *  takes the view's whole underlying ArrayBuffer, which then begins with somebody else's bytes. */
export const bytesOf = (h) => Uint8Array.from(Buffer.from(h, "hex"));

/** A decoded Candid value as plain JSON, in the shape `dfx --output json` prints (so watcher v1's pure functions read it):
 *  nat64/nat as decimal strings, principals as text, blobs as hex, opt as [] or [x]. */
export function plain(v) {
  if (typeof v === "bigint") return v.toString();
  if (v === null || v === undefined || typeof v !== "object") return v;
  if (v instanceof Principal || (typeof v.toText === "function" && v._isPrincipal)) return v.toText();
  if (v instanceof Uint8Array) return hex(v);
  if (Array.isArray(v)) return v.map(plain);
  return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, plain(x)]));
}

/** Decode a saved reply (hex) as the actor would, then make it plain. */
export function decodeReply(method, hexBytes) {
  const out = IDL.decode(REPLY_TYPES[method], bytesOf(hexBytes));
  return plain(out.length === 1 ? out[0] : out);
}

// ---- the argument's own type table --------------------------------------------------------------------------------------------
/** Every record-field and variant-tag hash in a Candid message's type table, read directly from the bytes. */
export function tableLabels(bytes) {
  const pipe = new PipeArrayBuffer(bytes);
  const magic = new TextDecoder().decode(pipe.read(4));
  if (magic !== "DIDL") throw new Error("not Candid (no DIDL header)");
  const labels = [];
  const n = Number(lebDecode(pipe));
  for (let i = 0; i < n; i++) {
    const op = Number(slebDecode(pipe));
    if (op === -18 || op === -19) slebDecode(pipe);                      // opt T, vec T
    else if (op === -20 || op === -21) {                                  // record, variant
      const k = Number(lebDecode(pipe));
      for (let j = 0; j < k; j++) { labels.push(Number(lebDecode(pipe))); slebDecode(pipe); }
    } else throw new Error(`type ${op} in the type table: Init has no functions, services or references`);
  }
  return labels;
}

/**
 * The buyer's upgrade argument, decoded. Never throws: `{ ok, init, unknown, error }`. `unknown` lists label hashes in the bytes
 * that `Init` does not have; `ok` is false if there are any, or if the bytes do not decode as exactly one `Init`.
 */
export function decodeInit(argHex) {
  let bytes;
  try {
    if (typeof argHex !== "string" || !/^([0-9a-f]{2})*$/i.test(argHex)) throw new Error("the argument is not hex");
    bytes = bytesOf(argHex);
    const unknown = [...new Set(tableLabels(bytes))].filter((l) => !KNOWN_INIT_LABELS.has(l));
    const vals = IDL.decode([Init], bytes);
    const init = plain(vals[0]);
    return { ok: unknown.length === 0, init, unknown, error: unknown.length ? `fields this monitor does not know (label hashes ${unknown.join(", ")})` : null };
  } catch (err) {
    return { ok: false, init: null, unknown: [], error: `the argument does not decode as the buyer's Init: ${String(err.message).split("\n")[0]}` };
  }
}

/** Encode an Init given as plain JSON (for the test alerts' sample proposals). */
export function encodeInitPlain(p) {
  const big = (x) => BigInt(x);
  const caps = (c) => ({ collections: c.collections.map((x) => ({ ...x, max_value_wei: big(x.max_value_wei), max_item_wei: big(x.max_item_wei) })),
    global_max_items: c.global_max_items, global_max_value_wei: big(c.global_max_value_wei) });
  const lim = (l) => ({ ...l, global_value_wei: big(l.global_value_wei) });
  const v = { ...p, home: p.home ?? [], chain_id: big(p.chain_id), operator: p.operator.map((x) => Principal.fromText(x)), evm_chains: [],
    custody_caps: p.custody_caps.map(caps), outflow_limits: p.outflow_limits.map(lim), frog_funding: p.frog_funding ?? [],
    eth_nft: (p.eth_nft ?? []).map((e) => ({ ...e, chain_id: big(e.chain_id), collections: e.collections.map((c) => ({ ...c, max_purchase_wei: big(c.max_purchase_wei), chain_id: (c.chain_id ?? []).map(big), token_bound: c.token_bound ?? [], nft_value_bps: c.nft_value_bps ?? [] })),
      floor: e.floor ?? [], gas_pot_target_wei: (e.gas_pot_target_wei ?? []).map(big), logs_max_blocks: (e.logs_max_blocks ?? []).map(big),
      dormancy: (e.dormancy ?? []).map((d) => ({ ...d, wake_timeout_secs: big(d.wake_timeout_secs), cooldown_secs: big(d.cooldown_secs), wake_desk: d.wake_desk ?? [],
        scaling: (d.scaling ?? []).map((s) => ({ ...s, active_b_per_day: big(s.active_b_per_day), idle_b_per_day: big(s.idle_b_per_day), wake_b: big(s.wake_b),
          reserve_b: big(s.reserve_b), exit_b_per_awake: big(s.exit_b_per_awake) })) })) })) };
  return Buffer.from(IDL.encode([Init], [v])).toString("hex");
}
