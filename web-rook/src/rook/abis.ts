/**
 * ABIs this site names beyond web/'s copied ones, by signature, from src/ (the repository's way: written by hand, then checked).
 * test/abis.test.ts compares every entry here with forge's compiled ABI whenever `out/` is beside the package, and fails on drift.
 *
 * CurveFactory's five-argument launch and tier bounds (src/curve/CurveFactory.sol), and what the 2026-09-26 contract fixes changed
 * (docs/audit/2026-09-26/fixes-W2-site.md §1): FrogCurve's deadline overloads, close gate and escape hatch (A-07, A-03, A-01);
 * the factory's launch gate (A-03); the Wrapper without `redraw` and with `resolve` (C-02, E-03); the raffle vault's keyless draw
 * calls (C-02..C-04); the drand beacon's `randomnessOf`. Everything else the pages show is read through the indexer's reports.
 */
export const CURVE_FACTORY = [
  { type: "function", name: "launch", stateMutability: "nonpayable",
    inputs: [{ name: "name", type: "string" }, { name: "symbol", type: "string" }, { name: "policy", type: "uint8" }, { name: "sweepOnly", type: "bool" }, { name: "feeBps", type: "uint16" }],
    outputs: [{ type: "address" }, { type: "address" }] },
  { type: "function", name: "minFeeBps", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "maxFeeBps", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "feeOf", stateMutability: "view", inputs: [{ name: "coin", type: "address" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "launchesOpen", stateMutability: "view", inputs: [], outputs: [{ type: "bool" }] },
] as const;

/** docs/v7/dormant-collections.md §3.3: the WakeDesk, where anyone asks for a dormant collection's wake with a bond (back to them when
 *  they launch a coin on it), and what the site reads of it. */
export const WAKE_DESK = [
  { type: "function", name: "request", stateMutability: "payable", inputs: [{ name: "wrapper", type: "address" }], outputs: [{ name: "id", type: "uint256" }] },
  { type: "function", name: "bond", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "openOf", stateMutability: "view", inputs: [{ name: "wrapper", type: "address" }], outputs: [{ name: "open", type: "bool" }, { name: "id", type: "uint256" }] },
  { type: "function", name: "reclaim", stateMutability: "nonpayable", inputs: [{ name: "id", type: "uint256" }, { name: "coinIndex", type: "uint256" }], outputs: [] },
  // independent review DI-M-2: when the open request was asked, and the desk's window (what the page says about a request not served)
  { type: "function", name: "requestAt", stateMutability: "view", inputs: [{ name: "id", type: "uint256" }],
    outputs: [{ name: "wrapper", type: "address" }, { name: "requester", type: "address" }, { name: "at", type: "uint64" }, { name: "bond_", type: "uint128" }, { name: "closed", type: "bool" }] },
  { type: "function", name: "window", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
] as const;
/** dormant-review DR-M-4: the Launch page's own read of the module's freshness; independent review 2 (D2-I-1): and of when its floor
 *  was last posted (with no lives served, the wake is offered only once the module has been stale for longer than a post cadence). */
export const FLOOR_MODULE_FRESH = [
  { type: "function", name: "fresh", stateMutability: "view", inputs: [], outputs: [{ type: "bool" }] },
  { type: "function", name: "postedAt", stateMutability: "view", inputs: [], outputs: [{ type: "uint64" }] },
] as const;

/** FrogCurve as the site calls it now: only the deadline overloads of buy and sell (A-07), so a request can never pick the old ones. */
export const FROG_CURVE_P1 = [
  { type: "function", name: "buy", stateMutability: "payable", inputs: [{ name: "minOut", type: "uint256" }, { name: "deadline_", type: "uint256" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "sell", stateMutability: "nonpayable", inputs: [{ name: "amountIn", type: "uint256" }, { name: "minEth", type: "uint256" }, { name: "deadline_", type: "uint256" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "refund", stateMutability: "nonpayable", inputs: [{ name: "amount", type: "uint256" }], outputs: [{ type: "uint256" }, { type: "uint256" }] },
  { type: "function", name: "graduate", stateMutability: "nonpayable", inputs: [], outputs: [] },
  { type: "function", name: "openRefunds", stateMutability: "nonpayable", inputs: [], outputs: [] },
  { type: "function", name: "state", stateMutability: "view", inputs: [], outputs: [{ type: "uint8" }] },
  { type: "function", name: "canClose", stateMutability: "view", inputs: [], outputs: [{ type: "bool" }] },
  { type: "function", name: "canGraduate", stateMutability: "view", inputs: [], outputs: [{ type: "bool" }] },
  { type: "function", name: "refundsOpenAt", stateMutability: "view", inputs: [], outputs: [{ name: "at", type: "uint64" }, { name: "atBlock", type: "uint64" }] },
  // R3 N-3: an unfilled curve's refunds wait up to STUCK_GRACE longer while the queue holds them (the keeper paused)
  { type: "function", name: "STUCK_GRACE", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "error", name: "QueueFull", inputs: [] },
  { type: "error", name: "TooEarly", inputs: [] },
] as const;

/** Wrapper's redemption as built after C-02/E-03: no redraw; anyone resolves (in ticket order) and refunds when due. */
export const WRAPPER_P1 = [
  { type: "function", name: "redeem", stateMutability: "nonpayable", inputs: [{ name: "taprootAddress", type: "string" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "refund", stateMutability: "nonpayable", inputs: [{ name: "id", type: "uint256" }], outputs: [] },
  { type: "function", name: "resolve", stateMutability: "nonpayable", inputs: [{ name: "id", type: "uint256" }, { name: "signature", type: "bytes" }], outputs: [] },
  { type: "function", name: "nextToResolve", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "beacon", stateMutability: "view", inputs: [], outputs: [{ type: "address" }] },
  { type: "function", name: "balanceOf", stateMutability: "view", inputs: [{ name: "account", type: "address" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "PIN_TIMEOUT", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "REFUND_GRACE", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  // R3 §4 (canister review2 N-2): a resolved ticket refunds at deliverBy + REFUND_GRACE once the custodian attests it undeliverable,
  // else at deliverBy + LATE_DELIVERY_WINDOW; `refund` reverts NotUndeliverable in between
  { type: "function", name: "undeliverable", stateMutability: "view", inputs: [{ name: "ticketId", type: "uint256" }], outputs: [{ type: "bool" }] },
  { type: "function", name: "LATE_DELIVERY_WINDOW", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "event", name: "Undeliverable", anonymous: false, inputs: [{ name: "ticketId", type: "uint256", indexed: true }] },
  { type: "error", name: "NotUndeliverable", inputs: [] },
  { type: "error", name: "TooEarly", inputs: [] },
  { type: "error", name: "WrongStage", inputs: [] },
] as const;

const DRAW_TUPLE = { type: "tuple", components: [
  { name: "stage", type: "uint8" }, { name: "coin", type: "address" }, { name: "challengeEnds", type: "uint64" }, { name: "challengeEndsBlock", type: "uint64" },
  { name: "round", type: "uint64" }, { name: "pinnedAt", type: "uint64" }, { name: "pinnedAtBlock", type: "uint64" }, { name: "resolvedAt", type: "uint64" },
  { name: "resolvedAtBlock", type: "uint64" }, { name: "root", type: "bytes32" }, { name: "totalTickets", type: "uint256" }, { name: "winningTicket", type: "uint256" },
  { name: "winner", type: "address" }, { name: "uri", type: "string" }, { name: "seedRound", type: "uint64" }, { name: "announcedAt", type: "uint64" },
  { name: "announcedAtBlock", type: "uint64" }, { name: "snapshotAt", type: "uint64" },
] } as const;
/** SweepVault's keyless draw calls (the keeper's announce and open are not the site's) and the draw read a claim is checked against. */
export const RAFFLE_VAULT = [
  { type: "function", name: "requestDraw", stateMutability: "nonpayable", inputs: [{ name: "id", type: "uint256" }], outputs: [] },
  { type: "function", name: "resolve", stateMutability: "nonpayable", inputs: [{ name: "id", type: "uint256" }, { name: "signature", type: "bytes" }], outputs: [] },
  { type: "function", name: "claim", stateMutability: "nonpayable",
    inputs: [{ name: "id", type: "uint256" }, { name: "holder", type: "address" }, { name: "firstTicket", type: "uint256" }, { name: "endTicket", type: "uint256" }, { name: "proof", type: "bytes32[]" }], outputs: [] },
  { type: "function", name: "voidDraw", stateMutability: "nonpayable", inputs: [{ name: "id", type: "uint256" }], outputs: [] },
  { type: "function", name: "draw", stateMutability: "view", inputs: [{ name: "id", type: "uint256" }], outputs: [DRAW_TUPLE] },
  { type: "function", name: "beacon", stateMutability: "view", inputs: [], outputs: [{ type: "address" }] },
  // R3 N-2: a draw voided or cancelled before its pin leaves its seed round (and so its snapshot time) to the coin's next draw
  { type: "function", name: "carriedSeed", stateMutability: "view", inputs: [{ name: "coin", type: "address" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "seedOrigin", stateMutability: "view", inputs: [{ name: "id", type: "uint256" }], outputs: [{ type: "uint256" }] },
  { type: "event", name: "SeedCarried", anonymous: false, inputs: [{ name: "drawId", type: "uint256", indexed: true }, { name: "originId", type: "uint256", indexed: true }, { name: "seedRound", type: "uint64", indexed: false }] },
  // fixes-R3-carry: an Open draw with no tickets (no eligible holder at the snapshot), closed by requestDraw with no winner
  { type: "event", name: "DrawEmpty", anonymous: false, inputs: [{ name: "drawId", type: "uint256", indexed: true }, { name: "coin", type: "address", indexed: true }, { name: "snapshotAt", type: "uint64", indexed: false }] },
] as const;
/** SweepVault.Stage on the wire (`Announced` and `Empty` are last so the earlier stages keep their numbers). */
export const DRAW_STAGE = ["none", "open", "pinned", "resolved", "claimed", "cancelled", "voided", "announced", "empty"] as const;

/** R3 (fixes-R3-contracts §1.2, §2): the Floor Module's burn-at-the-backing switch and the queue's refund hold. Everything else the
 *  site shows of the module comes from lib/floorModule.ts (web/'s copy) and the indexer's report. */
export const FLOOR_MODULE_R3 = [
  { type: "function", name: "exitAtBacking", stateMutability: "view", inputs: [], outputs: [{ type: "bool" }] },
  { type: "function", name: "holdsRefunds", stateMutability: "view", inputs: [], outputs: [{ type: "bool" }] },
  { type: "function", name: "burnCapacity", stateMutability: "view", inputs: [], outputs: [{ name: "unitPrice", type: "uint256" }, { name: "maxUnits", type: "uint256" }] },
] as const;
/** r4-contracts (A3-M1, A3-L1): `exitAtBacking()` is now `escaped()` alone (false while paused); `burnsOpen()` says a burn is priced
 *  at all, `pauseFloor()` is the frozen floor a paused burn pays (at most the backing per unit), and refunds stay held `RESUME_GRACE`
 *  after an unpause. */
export const FLOOR_MODULE_R4 = [
  { type: "function", name: "burnsOpen", stateMutability: "view", inputs: [], outputs: [{ type: "bool" }] },
  { type: "function", name: "pauseFloor", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "RESUME_GRACE", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
] as const;
/** r4-contracts A3-L1: the registry stamps a real resume on both clocks. */
export const LISTING_REGISTRY_R4 = [
  { type: "function", name: "keeperResumedAt", stateMutability: "view", inputs: [], outputs: [{ type: "uint64" }] },
  { type: "function", name: "keeperResumedAtBlock", stateMutability: "view", inputs: [], outputs: [{ type: "uint64" }] },
] as const;
/** ItemQueue (the pre-module queue) answers the same hold, always false (so `pure` there). */
export const ITEM_QUEUE_R3 = [
  { type: "function", name: "holdsRefunds", stateMutability: "pure", inputs: [], outputs: [{ type: "bool" }] },
] as const;
/** R3 N-8: ListingRegistry refuses a guardian that is (or is waiting to be) any other role. The site sends nothing to the registry;
 *  named so a pasted revert decodes. */
export const LISTING_REGISTRY_ERRORS = [{ type: "error", name: "SameKey", inputs: [] }] as const;

/** The drand evmnet verifier (src/rand/DrandEvmnet.sol): a round already proven needs no signature. */
export const BEACON = [
  { type: "function", name: "randomnessOf", stateMutability: "view", inputs: [{ name: "round", type: "uint64" }], outputs: [{ type: "bytes32" }] },
] as const;

/** docs/v6/offer-handins.md §2: the parameters suggested for 4663, shown labelled for a nest whose board is not deployed. */
export const OFFER_BOARD_SUGGESTED = { bondWei: 10n ** 16n, rewardWei: 10n ** 16n, slashDelayS: 3_600, expiryS: 86_400, rewardDayCapWei: 2n * 10n ** 17n, maxOpen: 64, maxOpenPerSubmitter: 4 };
