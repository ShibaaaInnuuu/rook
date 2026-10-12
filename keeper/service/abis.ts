import { parseAbi } from "viem";
import { FLOOR_MODULE_ABI } from "../src/floorpost.js";

/**
 * The functions and errors the production keeper and the resolver read and call, by their Solidity signatures
 * (src/floor/FloorModule.sol, src/SweepVault.sol, src/Wrapper.sol, src/ListingRegistry.sol, src/rand/DrandEvmnet.sol).
 *
 * What each may WRITE is closed and split by role (test/service-guards.test.ts):
 *   the keeper (the registry's keeper key): MODULE.postFloor, VAULT.postCap / announceDraw / openDraw;
 *   the resolver (any funded key, no role): VAULT_PUBLIC.requestDraw / resolve / claim / voidDraw, WRAPPER_PUBLIC.resolve.
 * Nothing here records a delivery: `Wrapper.recordDelivery` is the custodian's alone (fixes-B-C-keeper-draws.md, B-03/C-05).
 */
export const MODULE = [...FLOOR_MODULE_ABI, ...parseAbi([
  "function fresh() view returns (bool)",
  "function accepted() view returns (uint256)",
  "function minConfidenceBps() view returns (uint256)",
  "function confidenceBps() view returns (uint16)",
  "function wrapped() view returns (address)",
  "function registry() view returns (address)",
])] as const;

const DRAW_STRUCT = "struct Draw { uint8 stage; address coin; uint64 challengeEnds; uint64 challengeEndsBlock; uint64 round; uint64 pinnedAt; uint64 pinnedAtBlock; uint64 resolvedAt; uint64 resolvedAtBlock; bytes32 root; uint256 totalTickets; uint256 winningTicket; address winner; string uri; uint64 seedRound; uint64 announcedAt; uint64 announcedAtBlock; uint64 snapshotAt; }";

export const VAULT = parseAbi([
  DRAW_STRUCT,
  "function registry() view returns (address)",
  "function policy() view returns (uint8)",
  "function cap() view returns (uint256)",
  "function capAt() view returns (uint64)",
  "function capAtBlock() view returns (uint64)",
  "function capFresh() view returns (bool)",
  "function capCeiling() view returns (uint256)",
  "function parentBlockSeconds() view returns (uint256)",
  "function units(address coin) view returns (uint256)",
  "function drawCount() view returns (uint256)",
  "function drawInFlight(address coin) view returns (uint256)",
  "function draw(uint256 id) view returns (Draw)",
  "function snapshotTimeOf(uint256 id, bytes32 seedRandomness) view returns (uint64)",
  // review2 N-2 (r3-contracts): a draw voided or cancelled before its pin leaves its seed to the coin's next announcement, which
  // gets the same seed round and snapshot time; the keeper never computes that time itself, it reads snapshotTimeOf
  "function seedOrigin(uint256 id) view returns (uint256)",
  "function carriedSeed(address coin) view returns (uint256)",
  "function beacon() view returns (address)",
  "function postCap(uint256 weiPerWrapped)",
  "function announceDraw(address coin) returns (uint256 id)",
  "function openDraw(uint256 id, bytes32 root, uint256 totalTickets, string uri, bytes32[] sentinelProof, bytes seedSignature)",
  "event DrawAnnounced(uint256 indexed drawId, address indexed coin, uint64 seedRound)",
  "event SnapshotFixed(uint256 indexed drawId, uint64 snapshotAt)",
  "event SeedCarried(uint256 indexed drawId, uint256 indexed originId, uint64 seedRound)",
  "event DrawOpened(uint256 indexed drawId, address indexed coin, bytes32 root, uint256 totalTickets, string uri, uint64 challengeEnds)",
  // fixes-R3-carry: a draw opened over a snapshot with no eligible holder, closed by requestDraw with no winner
  "event DrawEmpty(uint256 indexed drawId, address indexed coin, uint64 snapshotAt)",
]);

/** The vault's permissionless draw calls: anyone may send them, the keeper key is not needed (the resolver, E-03). */
export const VAULT_PUBLIC = parseAbi([
  "function requestDraw(uint256 id)",
  "function resolve(uint256 id, bytes signature)",
  "function claim(uint256 id, address holder, uint256 firstTicket, uint256 endTicket, bytes32[] proof)",
  "function voidDraw(uint256 id)",
]);

/** SweepVault.Stage, as numbered on chain (`Announced` and `Empty` are last so the earlier stages kept their numbers). `Empty`
 *  (fixes-R3-carry): opened over a snapshot with no eligible holder and closed with no winner; final. */
export const STAGE = { None: 0, Open: 1, Pinned: 2, Resolved: 3, Claimed: 4, Cancelled: 5, Voided: 6, Announced: 7, Empty: 8 } as const;
/** SweepVault's draw timing constants (src/SweepVault.sol), fixed in the contract and repeated here to plan against. */
export const DRAW_RULES = { openTtlS: 6 * 3_600, challengeS: 900, pinTimeoutS: 7 * 86_400, voidAfterS: 7 * 86_400, snapshotWindowS: 86_400 } as const;

export type DrawView = { stage: number; coin: `0x${string}`; challengeEnds: bigint; challengeEndsBlock: bigint; round: bigint; pinnedAt: bigint; pinnedAtBlock: bigint;
  resolvedAt: bigint; resolvedAtBlock: bigint; root: `0x${string}`; totalTickets: bigint; winningTicket: bigint; winner: `0x${string}`; uri: string; seedRound: bigint;
  announcedAt: bigint; announcedAtBlock: bigint; snapshotAt: bigint };

export const WRAPPER = parseAbi([
  "struct Ticket { uint8 stage; address owner; uint64 round; uint64 pinnedAt; uint64 pinnedAtBlock; uint64 deliverBy; uint64 deliverByBlock; uint256 acquisitionId; bytes32 deliveryTxid; string destination; }",
  "function nextToResolve() view returns (uint256)",
  "function ticketCount() view returns (uint256)",
  "function ticket(uint256 id) view returns (Ticket)",
  "function beacon() view returns (address)",
]);
/** EthNftWrapper (docs/v7/phase1-nft.md §3.1): the same machine, its ticket holds an item id and an EVM destination. */
export const NFT_WRAPPER = parseAbi([
  "struct Ticket { uint8 stage; address owner; uint64 round; uint64 pinnedAt; uint64 pinnedAtBlock; uint64 deliverBy; uint64 deliverByBlock; uint256 itemId; bytes32 deliveryTx; address destination; }",
  "function nextToResolve() view returns (uint256)",
  "function ticketCount() view returns (uint256)",
  "function ticket(uint256 id) view returns (Ticket)",
  "function beacon() view returns (address)",
]);
/** Wrapper.Stage */
export const TICKET = { None: 0, Pinned: 1, Resolved: 2, Delivered: 3, Refunded: 4 } as const;
export type TicketView = { stage: number; owner: `0x${string}`; round: bigint; pinnedAt: bigint; pinnedAtBlock: bigint; deliverBy: bigint; deliverByBlock: bigint;
  acquisitionId: bigint; deliveryTxid: `0x${string}`; destination: string };

/** The Wrapper's one permissionless call the resolver makes: resolve the next ticket, in order. */
export const WRAPPER_PUBLIC = parseAbi(["function resolve(uint256 id, bytes signature)"]);

export const BEACON = parseAbi([
  "function roundAt(uint256 timestamp) view returns (uint64)",
  "function timeOf(uint64 round) view returns (uint256)",
  "function randomnessOf(uint64 round) view returns (bytes32)",
  "function verify(uint64 round, bytes signature) view returns (bytes32)",
]);

export const REGISTRY = parseAbi([
  "function keeper() view returns (address)",
  "function namedKeeper() view returns (address)",
  "function keeperPaused() view returns (bool)",
  "function guardian() view returns (address)",
  "function owner() view returns (address)",
]);

export const FACTORY = parseAbi([
  "function coinCount() view returns (uint256)",
  "function coins(uint256 i) view returns (address)",
  "function policyOf(address coin) view returns (uint8)",
  "function routerOf(address coin) view returns (address)",
]);

/** Every custom error the contracts the keeper and the resolver call can revert with: a failed simulation is reported by name. */
export const ERRORS = parseAbi([
  "error BadParams()", "error NotKeeper()", "error NotCustodian()", "error BadInput()", "error Stale()", "error OverCap()",
  "error CapStale()", "error CapStep(uint256 cap, uint256 lo, uint256 hi)", "error CapTooSoon()", "error OverCeiling(uint256 cap, uint256 ceiling)",
  "error WrongPolicy()", "error ListingPaused()", "error StaleSnapshot()", "error Insufficient()", "error WrongStage()", "error NotOwner()",
  "error NotRouter()", "error NotAccepted()", "error TooEarly()", "error BadProof()", "error ReentrancyGuardReentrantCall()",
  "error NoCeiling()", "error DrawInFlight()", "error KeeperPaused()", "error ChallengeWindow()", "error WindowOver()",
  "error OutOfOrder(uint256 next)", "error NothingToDraw()", "error InvalidSignature()", "error BadSignatureLength()", "error SignatureNotOnCurve()",
  "error PairingFailed()",
  // r3-contracts: the Wrapper's refund of a resolved ticket before the custodian's attestation or the late window; the registry's
  // role separation for the guardian. Neither is a call the keeper makes; named so a simulation that meets one says which.
  "error NotUndeliverable()", "error SameKey()",
]);

/** SweepVault.Policy */
export const POLICY = { RAFFLE: 0, BURN: 1, YOYO: 2 } as const;
