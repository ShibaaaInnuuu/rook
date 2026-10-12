import { parseAbi } from "viem";

/** docs/v5/ux-and-reserves.md. The v3/v4 contracts' events and views this read side needs, copied from src/ by signature. */
export const WRAPPER_EVENTS = parseAbi([
  "event Wrapped(uint256 indexed acquisitionId, string inscriptionId, address indexed to)",
  "event Deposited(uint256 indexed frogId, string inscriptionId, bytes32 txid, uint32 vout, address indexed to)",
  "event Redeemed(uint256 indexed ticketId, address indexed owner, uint64 round, string destination)",
  "event Repinned(uint256 indexed ticketId, uint64 round)",
  "event Resolved(uint256 indexed ticketId, uint256 indexed acquisitionId, uint64 deliverBy)",
  "event Delivered(uint256 indexed ticketId, uint256 indexed acquisitionId, bytes32 txid)",
  "event Refunded(uint256 indexed ticketId, uint256 indexed acquisitionId, address indexed owner)",
  // review2 canister N-2 (fixes-R3-contracts §4): the custodian's word that a resolved ticket's delivery can no longer confirm
  "event Undeliverable(uint256 indexed ticketId)",
]);
export const NFT_WRAPPER_EVENTS = parseAbi([
  "event Wrapped(uint256 indexed itemId, uint256 indexed tokenId, bytes32 purchaseTx, address indexed to)",
  "event Deposited(uint256 indexed itemId, uint256 indexed tokenId, bytes32 depositTx, address indexed to)",
  "event Redeemed(uint256 indexed ticketId, address indexed owner, uint64 round, address destination)",
  "event Repinned(uint256 indexed ticketId, uint64 round)",
  "event Resolved(uint256 indexed ticketId, uint256 indexed itemId, uint256 indexed tokenId, uint64 deliverBy)",
  "event Delivered(uint256 indexed ticketId, uint256 indexed tokenId, address destination, bytes32 txHash)",
  "event Refunded(uint256 indexed ticketId, uint256 indexed itemId, address indexed owner)",
  // docs/v7/phase1-nft.md §3.1: EthNftWrapper, the Phase-1 NFT wrapper, has the gated refund (no Repinned: it has no redraw)
  "event Undeliverable(uint256 indexed ticketId)",
]);
export const WRAPPER_READS = parseAbi([
  "function totalSupply() view returns (uint256)",
  "function held() view returns (uint256)",
  "function openTickets() view returns (uint256)",
  "function symbol() view returns (string)",
  // fixes-R3-contracts §4: a resolved ticket refunds at deliverBy + REFUND_GRACE once attested, else deliverBy + LATE_DELIVERY_WINDOW
  "function undeliverable(uint256 ticketId) view returns (bool)",
  "function LATE_DELIVERY_WINDOW() view returns (uint256)",
]);
export const FACTORY_EVENTS = parseAbi([
  "event Launched(address indexed coin, address indexed curve, address indexed creator, uint8 policy, uint256 raise, bool sweepOnly, bool early, bool opening)",
]);
export const FACTORY_READS = parseAbi([
  "function freshCap() view returns (uint256 cap, bool fresh)",
  "function raffleVault() view returns (address)",
  "function burnVault() view returns (address)",
  "function yoyoVault() view returns (address)",
  "function feeOf(address coin) view returns (uint256)",
  // fixes-A-graduation A-03: false while the queue has no room for a new coin's item budget ("launches paused")
  "function launchesOpen() view returns (bool)",
  // docs/v7/phase1-nft-pooled.md §2: the collection's own graduation targets (immutables)
  "function normalRaise() view returns (uint256)",
  "function earlyRaise() view returns (uint256)",
  "function earlyUntilEth() view returns (uint256)",
  "function maxRaise() view returns (uint256)",
]);
/** docs/v5/policies-and-fees.md §2–§3: the tier and the decaying launch fee are the curve's immutables; `launchExtraBps()` is what a buy pays now. */
export const CURVE_READS = parseAbi([
  "function state() view returns (uint8)",
  "function progress() view returns (uint256 sold, uint256 raised)",
  "function feeBps() view returns (uint256)",
  "function launchFeeBps() view returns (uint256)",
  "function launchFeeDecay() view returns (uint256)",
  "function launchedAt() view returns (uint64)",
  "function launchExtraBps() view returns (uint256)",
  // fixes-A-graduation A-03 and A-01: the close gate and the escape hatch
  "function canClose() view returns (bool)",
  "function canGraduate() view returns (bool)",
  "function refundsOpenAt() view returns (uint64 at, uint64 atBlock)",
]);
export const TOKEN_READS = parseAbi(["function name() view returns (string)", "function symbol() view returns (string)"]);
export const SWEEP_EVENTS = parseAbi([
  "event Won(uint256 indexed drawId, address indexed holder)",
  "event Retired(address indexed coin, uint256 indexed ticketId)",
  // docs/v5/policies-and-fees.md §1: YOYO's listing, sale, buyback-and-burn, and return
  "event Relisted(address indexed coin, uint256 indexed ticketId, uint256 ask)",
  "event SaleRecorded(address indexed coin, uint256 indexed ticketId, uint256 proceeds)",
  "event BoughtBack(address indexed coin, address indexed caller, uint256 spent, uint256 burned)",
  "event ListingReturned(address indexed coin, uint256 indexed ticketId)",
]);
/** fixes-B-C C-02..C-04, E-02: a raffle vault's draw, announce → open → pin → resolve → claim, or cancel / void; or, with no
 *  eligible holder at the snapshot, announce → open (empty) → closed with no winner (fixes-R3-carry). */
export const DRAW_EVENTS = parseAbi([
  "event DrawAnnounced(uint256 indexed drawId, address indexed coin, uint64 seedRound)",
  "event SnapshotFixed(uint256 indexed drawId, uint64 snapshotAt)",
  "event DrawOpened(uint256 indexed drawId, address indexed coin, bytes32 root, uint256 totalTickets, string uri, uint64 challengeEnds)",
  "event DrawCancelled(uint256 indexed drawId)",
  "event DrawPinned(uint256 indexed drawId, uint64 round)",
  "event DrawResolved(uint256 indexed drawId, uint256 winningTicket)",
  "event Won(uint256 indexed drawId, address indexed holder)",
  "event DrawVoided(uint256 indexed drawId)",
  // review2 N-2 (fixes-R3-contracts §3): a draw voided or cancelled before its pin leaves its seed round and snapshot window to the
  // coin's next announcement, which emits this (before DrawAnnounced, same transaction)
  "event SeedCarried(uint256 indexed drawId, uint256 indexed originId, uint64 seedRound)",
  // fixes-R3-carry: an Open draw with no tickets (no eligible holder at the snapshot), closed by requestDraw with no winner
  "event DrawEmpty(uint256 indexed drawId, address indexed coin, uint64 snapshotAt)",
]);
/** fixes-R3-contracts §3: the seed a coin's next draw will carry (origin id + 1; 0 none), and the draw whose seed a draw uses. */
export const DRAW_SEED_READS = parseAbi([
  "function carriedSeed(address coin) view returns (uint256)",
  "function seedOrigin(uint256 id) view returns (uint256)",
]);
/** fixes-R3-contracts §2 (N-3): while the queue holds refunds (the module: keeper down), an unfilled curve's refunds wait up to STUCK_GRACE longer. */
export const QUEUE_READS = parseAbi(["function holdsRefunds() view returns (bool)"]);
/** fixes-B-C §0: the registry's keeper switch (FloorModule.registry() names it). */
export const REGISTRY_READS = parseAbi([
  "function keeperPaused() view returns (bool)",
  "function guardian() view returns (address)",
]);
export const REGISTRY_SWITCH_EVENTS = parseAbi(["event KeeperPaused(address indexed by, bool paused)"]);
export const SWEEP_READS = parseAbi([
  "function listedOpen(address coin) view returns (uint256)",
  "function proceeds(address coin) view returns (uint256)",
  "function markupBps() view returns (uint256)",
]);
/** docs/v6/offer-handins.md §2.1: the board's ABI as built; the canister pins the same selectors. */
export const OFFER_BOARD_EVENTS = parseAbi([
  "event Submitted(uint256 indexed id, address indexed submitter, bytes32 indexed offerHash, uint256 askSats, string inscriptionId)",
  "event Claimed(uint256 indexed id, address indexed submitter, uint256 reward)",
  "event Slashed(uint256 indexed id, address indexed submitter, uint256 bond)",
  "event Refunded(uint256 indexed id, address indexed submitter, uint256 bond)",
  "event Funded(address indexed from, uint256 amount)",
]);
export const OFFER_BOARD_READS = parseAbi([
  "function params() view returns (uint256 bond, uint256 reward, uint256 slashDelay, uint256 expiry, uint256 rewardDayCap, uint256 maxOpen)",
  "function maxOpenPerSubmitter() view returns (uint256)",
  "function custodian() view returns (address)",
  "function rewardPool() view returns (uint256)",
  "function paidToday() view returns (uint256)",
  "function openCount() view returns (uint256)",
  "function openIds(uint256 start, uint256 max) view returns (uint256[])",
  "function submissionCount() view returns (uint256)",
  "function submissionOf(uint256 id) view returns ((address submitter, uint64 at, uint64 atBlock, uint8 status, uint256 bond, uint256 askSats, bytes32 offerHash, string inscriptionId))",
  "function slashable(uint256 id) view returns (bool)",
  "function expired(uint256 id) view returns (bool)",
]);
export const ERC721_READS = parseAbi(["function ownerOf(uint256 tokenId) view returns (address)"]);
