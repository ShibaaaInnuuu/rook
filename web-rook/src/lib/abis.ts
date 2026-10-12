import { parseAbi } from "viem";

export const REGISTRY = parseAbi([
  "function count() view returns (uint256)",
  "function listing(uint256 id) view returns ((uint8 kind, bool paused, bool autonomous, uint8 divisibility, address vault, bytes32 membershipRoot, string name, string ref, string btcVault))",
  "function isAutoBuyer(address who) view returns (bool)",
  "function bondBps() view returns (uint16)", "function owner() view returns (address)", "function treasury() view returns (address)",
  "function keeper() view returns (address)", "function purchaser() view returns (address)", "function attestor() view returns (address)",
]);
export const LAUNCHER = parseAbi([
  "struct Socials { string twitter; string telegram; string discord; string website; string farcaster; }",
  "struct LaunchParams { string name; string symbol; string logo; string description; Socials socials; uint16 creatorTaxBps; bytes32 expectedEconomics; bytes32 salt; uint256 launchConfigId; uint256 listingId; }",
  "function launch(LaunchParams p) payable returns (address coin, address curve, address router)",
  "function coinCount() view returns (uint256)", "function coins(uint256 i) view returns (address)",
  "function routerOf(address coin) view returns (address)", "function creatorOf(address coin) view returns (address)", "function listingOf(address coin) view returns (uint256)",
  "event Launched(address indexed coin, address indexed creator, uint256 indexed listingId, address curve, address router)",
]);
export const FACTORY = parseAbi([
  "function launchFee() view returns (uint256)", "function feeEscrow() view returns (address)",
  "function getLaunchedToken(address token) view returns (address token_, address curve, address deployer, address creatorFeeRecipient, address pairToken, uint256 graduationThreshold, uint24 poolFee, int24 tickSpacing, uint16 creatorTaxBps, bool buybackEnabled, uint8 phase, uint256 sweptQuote, uint256 sweptTokens, uint256 sweptAt, bool exists)",
]);
export const ESCROW = parseAbi(["function balanceOf(address recipient) view returns (uint256)"]);
export const ERC20 = parseAbi(["function name() view returns (string)", "function symbol() view returns (string)"]);
export const ROUTER = parseAbi(["function harvest() returns (uint256 toVault, uint256 toTreasury)", "function treasuryOwed() view returns (uint256)"]);
export const VAULT = parseAbi([
  "function attributed() view returns (uint256)", "function reserved() view returns (uint256)", "function bond() view returns (uint256)",
  "function outstanding() view returns (uint256)", "function requiredBond() view returns (uint256)", "function totalWrittenOff() view returns (uint256)",
  "function capWei() view returns (uint256)", "function capExpiry() view returns (uint64)", "function isBlocked() view returns (bool, uint256)",
  "function contributedBy(address coin) view returns (uint256)", "function chargedTo(address coin) view returns (uint256)",
  "function openPurchases() view returns (uint256[])", "function purchaseCount() view returns (uint256)", "function acquisitionCount() view returns (uint256)",
  "function purchase(uint256 id) view returns ((uint8 status, uint64 releasableAt, uint64 deadline, address purchaser, uint256 maxEth, uint256 quantity, uint256 settledQuantity, uint256 returned, string capRef))",
  "function acquisition(uint256 id) view returns ((uint256 purchaseId, address coin, uint256 amount, bytes32 btcTxid, uint64 satsPaid, string assetRef))",
  "function release(uint256 id)",
]);
export const DISTRIBUTOR = parseAbi([
  "function choose(uint256 listingId, uint256 acquisitionId, uint8 mode, address named)",
  "function lot(uint256 listingId, uint256 acquisitionId) view returns ((uint8 mode, uint8 stage, bool overdue, uint64 challengeEnds, uint64 round, uint64 pinnedAt, uint64 destinationAt, address winner, bytes32 root, uint256 totalTickets, uint256 winningTicket, bytes32 deliveryTxid, string uri, string destination))",
  "function distribution(uint256 listingId, uint256 acquisitionId) view returns ((bool open, bool rolled, uint64 challengeEnds, uint64 claimsEnd, bytes32 root, uint256 total, uint256 claimed, string uri))",
  "function accrued(uint256 listingId, address holder) view returns (uint256)", "function minimumUnits(uint256 listingId) view returns (uint256)", "function minimumExpiry(uint256 listingId) view returns (uint64)",
  "function shareClaimed(uint256 listingId, uint256 acquisitionId, address holder) view returns (bool)",
  "function requestDraw(uint256 listingId, uint256 acquisitionId)", "function resolve(uint256 listingId, uint256 acquisitionId, bytes signature)",
  "function claimWin(uint256 listingId, uint256 acquisitionId, address winner, uint256 firstTicket, uint256 endTicket, bytes32[] proof)",
  "function setDestination(uint256 listingId, uint256 acquisitionId, string taprootAddress)",
  "function claimShare(uint256 listingId, uint256 acquisitionId, address holder, uint256 units, bytes32[] proof)",
  "function requestPayout(uint256 listingId, string taprootAddress) returns (uint256)",
]);
export const BEACON = parseAbi(["function timeOf(uint64 round) view returns (uint256)", "function randomnessOf(uint64 round) view returns (bytes32)"]);

/** docs/phase6.md: Pons's curve, as the STAGING trade box calls it. The real site sends nobody to trade; Pons's own site does that. */
export const PONS_CURVE = parseAbi([
  "function buy(uint256 quoteIn, uint256 minTokensOut, address recipient) payable returns (uint256)",
  "function sell(uint256 tokensIn, uint256 minQuoteOut, address recipient) returns (uint256)",
]);
export const ERC20_TRADE = parseAbi(["function approve(address spender, uint256 amount) returns (bool)", "function allowance(address owner, address spender) view returns (uint256)", "function balanceOf(address owner) view returns (uint256)"]);
