import { parseAbi } from "viem";

/** Our contracts' events, written from src/*.sol. Enums are uint8 on the wire. test/abis.test.ts checks each against forge's artifacts when they exist. */
export const REGISTRY_EVENTS = parseAbi([
  "event Listed(uint256 indexed id, uint8 kind, address vault, string name, string ref)",
  "event AutoBuyerNamed(uint256 indexed id, address buyer, uint256 maxInFlight)",
  "event Paused(uint256 indexed id, bool paused)",
  "event RolesProposed(address keeper, address purchaser, address attestor, uint64 readyAt)",
  "event RolesSet(address keeper, address purchaser, address attestor)",
  "event RootProposed(uint256 indexed id, bytes32 root, uint64 readyAt)",
  "event RootSet(uint256 indexed id, bytes32 root)",
  "event BtcVaultProposed(uint256 indexed id, string btcVault, uint64 readyAt)",
  "event BtcVaultSet(uint256 indexed id, string btcVault)",
  "event TreasuryProposed(address treasury, uint64 readyAt)",
  "event TreasurySet(address treasury)",
  "event BondBpsProposed(uint16 bps, uint64 readyAt)",
  "event BondBpsSet(uint16 bps)",
  "event Wired(address launcher, address distributor)",
  "event NftRegistryWired(address nftRegistry)",
  "event BuyerReserved(address indexed buyer)",
  // docs/audit/2026-09-26/fixes-B-C-keeper-draws.md §0: the instant keeper switch (owner or guardian pauses, only the owner
  // unpauses) and the guardian's appointment; folded into the roles history with the other role events
  "event KeeperPaused(address indexed by, bool paused)",
  "event GuardianSet(address guardian)",
]);
export const LAUNCHER_EVENTS = parseAbi([
  "event Launched(address indexed coin, address indexed creator, uint256 indexed listingId, address curve, address router)",
]);
/**
 * docs/phase10.md: a coin's logo is in no event — not Pons's and not ours. It is an argument of the call, so the only way to
 * learn it is to decode the transaction that emitted `Launched`.
 */
export const LAUNCHER_CALLS = parseAbi([
  "struct Socials { string twitter; string telegram; string discord; string website; string farcaster; }",
  "struct LaunchParams { string name; string symbol; string logo; string description; Socials socials; uint16 creatorTaxBps; bytes32 expectedEconomics; bytes32 salt; uint256 launchConfigId; uint256 listingId; }",
  "function launch(LaunchParams p) payable returns (address coin, address curve, address router)",
]);
/**
 * docs/phase10.md §chart. Pons's own bonding curve, one per coin, read on the staging fork: `CurveBuy` and `CurveSell` are the
 * trades, `CurveCompleted` is the moment the curve ends and the coin graduates. Topics matched against live logs, not docs.
 */
export const CURVE_EVENTS = parseAbi([
  "event CurveBuy(address indexed buyer, address indexed recipient, uint256 quoteIn, uint256 tokensOut, uint256 fee, uint256 tax)",
  "event CurveSell(address indexed seller, address indexed recipient, uint256 tokensIn, uint256 quoteOut, uint256 fee, uint256 tax)",
  "event CurveCompleted(address recipient, uint256 quoteOut, uint256 tokenOut)",
]);
/**
 * After graduation a coin trades in a Uniswap v4 pool, so the venue is the one PoolManager the factory itself names
 * (`poolManager()`). `Initialize` says which pool is which coin's; `Swap` is the trade. Both are read by topic, never by
 * fetching everything that contract emits.
 */
export const POOL_EVENTS = parseAbi([
  "event Initialize(bytes32 indexed id, address indexed currency0, address indexed currency1, uint24 fee, int24 tickSpacing, address hooks, uint160 sqrtPriceX96, int24 tick)",
  "event Swap(bytes32 indexed id, address indexed sender, int128 amount0, int128 amount1, uint160 sqrtPriceX96, uint128 liquidity, int24 tick, uint24 fee)",
]);
/**
 * docs/phase13.md §2. Every Pons coin is an ERC-20, and its `Transfer` log is the only place a balance change is written
 * down: there is no event for "holders", and no way to derive one from trades, because a transfer is not a trade. Balances
 * are folded from these logs alone — the mint (`from` = the zero address) and a burn (`to` = it) included — so a reindex
 * reproduces every one of them and `balanceOf` is never called for a holder.
 */
export const COIN_EVENTS = parseAbi([
  "event Transfer(address indexed from, address indexed to, uint256 value)",
]);
export const ROUTER_EVENTS = parseAbi([
  "event Harvested(uint256 claimed, uint256 toVault, uint256 toTreasury)",
  "event TreasuryDeferred(uint256 owed)",
  "event TokenForwarded(address indexed token, address indexed treasury, uint256 amount)",
]);
export const VAULT_EVENTS = parseAbi([
  "event Deposited(address indexed coin, uint256 amount)",
  "event CapPosted(uint256 capWei, uint64 expiry)",
  "event Declared(uint256 indexed id, uint256 maxEth, uint256 quantity, uint64 releasableAt, string capRef)",
  "event Cancelled(uint256 indexed id)",
  "event Released(uint256 indexed id, address indexed purchaser, uint256 amount, uint64 deadline)",
  "event Charged(uint256 indexed id, address indexed coin, uint256 amount)",
  "event Receipted(uint256 indexed id, uint256 indexed acquisitionId, address indexed coin, string assetRef, uint256 amount, bytes32 btcTxid, uint64 satsPaid)",
  "event Settled(uint256 indexed id)",
  "event Returned(uint256 indexed id, uint256 amount)",
  "event WrittenOff(uint256 indexed id, uint256 amount, bytes32 reasonRef)",
  "event BondPosted(address indexed from, uint256 amount)",
  "event BondWithdrawn(address indexed to, uint256 amount)",
  "event BondSlashed(uint256 indexed id, uint256 amount)",
]);
export const DISTRIBUTOR_EVENTS = parseAbi([
  "event Chosen(uint256 indexed listingId, uint256 indexed acquisitionId, uint8 mode, address named)",
  "event DrawOpened(uint256 indexed listingId, uint256 indexed acquisitionId, bytes32 root, uint256 totalTickets, string uri, uint64 challengeEnds)",
  "event DrawCancelled(uint256 indexed listingId, uint256 indexed acquisitionId)",
  "event DrawPinned(uint256 indexed listingId, uint256 indexed acquisitionId, uint64 round)",
  "event DrawResolved(uint256 indexed listingId, uint256 indexed acquisitionId, uint256 winningTicket)",
  "event Won(uint256 indexed listingId, uint256 indexed acquisitionId, address indexed winner)",
  "event DrawVoided(uint256 indexed listingId, uint256 indexed acquisitionId, uint256 unclaimedTicket)",
  "event DestinationSet(uint256 indexed listingId, uint256 indexed acquisitionId, string destination)",
  "event Delivered(uint256 indexed listingId, uint256 indexed acquisitionId, bytes32 txid)",
  "event DeliveryOverdue(uint256 indexed listingId, uint256 indexed acquisitionId)",
  "event DistributionOpened(uint256 indexed listingId, uint256 indexed acquisitionId, bytes32 root, uint256 total, string uri, uint64 challengeEnds)",
  "event DistributionCancelled(uint256 indexed listingId, uint256 indexed acquisitionId)",
  "event ShareClaimed(uint256 indexed listingId, uint256 indexed acquisitionId, address indexed holder, uint256 amount)",
  "event RolledOver(uint256 indexed listingId, uint256 indexed acquisitionId, address indexed coin, uint256 amount)",
  "event MinimumPosted(uint256 indexed listingId, uint256 units, uint64 expiry)",
  "event PayoutRequested(uint256 indexed payoutId, uint256 indexed listingId, address indexed holder, uint256 amount, string destination)",
  "event PayoutRecorded(uint256 indexed payoutId, bytes32 txid)",
  "event PayoutOverdue(uint256 indexed payoutId)",
]);
export const EVENTS = { registry: REGISTRY_EVENTS, launcher: LAUNCHER_EVENTS, router: ROUTER_EVENTS, vault: VAULT_EVENTS, distributor: DISTRIBUTOR_EVENTS,
  curve: CURVE_EVENTS, pool: POOL_EVENTS, coin: COIN_EVENTS } as const;
export type ContractKind = keyof typeof EVENTS;

export const READS = parseAbi([
  // ERC-20
  "function name() view returns (string)",
  "function symbol() view returns (string)",
  // docs/phase12.md §3: a coin's whole supply, minted by Pons at launch and constant thereafter. It is in no event, so it is
  // read once per coin and kept on coin_meta; market cap is supply × price and cannot be computed without it.
  "function totalSupply() view returns (uint256)",
  // ListingRegistry
  "function listing(uint256 id) view returns ((uint8 kind, bool paused, uint8 divisibility, address vault, bytes32 membershipRoot, string name, string ref, string btcVault))",
  // AssetVault
  "function attributed() view returns (uint256)", "function waiting() view returns (uint256)", "function reserved() view returns (uint256)",
  "function bond() view returns (uint256)", "function outstanding() view returns (uint256)", "function requiredBond() view returns (uint256)",
  "function capWei() view returns (uint256)", "function capExpiry() view returns (uint64)", "function isBlocked() view returns (bool, uint256)",
  "function unspentOf(address coin) view returns (uint256)", "function queued(address coin) view returns (bool)",
  // Pons: the factory's record of a launch, the escrow, the curve, the hook's pending buckets (docs/phase1.md)
  "function getLaunchedToken(address token) view returns (address token_, address curve, address deployer, address creatorFeeRecipient, address pairToken, uint256 graduationThreshold, uint24 poolFee, int24 tickSpacing, uint16 creatorTaxBps, bool buybackEnabled, uint8 phase, uint256 sweptQuote, uint256 sweptTokens, uint256 sweptAt, bool exists)",
  "function memeHook() view returns (address)", "function feeEscrow() view returns (address)",
  // docs/phase10.md: the v4 PoolManager a graduated coin trades in, asked of the factory rather than written down per chain
  "function poolManager() view returns (address)",
  "function balanceOf(address recipient) view returns (uint256)",
  "function graduated() view returns (bool)",
  "function pendingFees(bytes32 poolId, address currency) view returns (uint256)",
  "function pendingCreatorTax(bytes32 poolId, address currency) view returns (uint256)",
  // docs/phase12.md §3: a Uniswap v3 pool's packed price word. Read on the WETH/USDG pool it is the chain's own dollar rate.
  "function slot0() view returns (uint160 sqrtPriceX96, int24 tick, uint16 observationIndex, uint16 observationCardinality, uint16 observationCardinalityNext, uint8 feeProtocol, bool unlocked)",
]);
