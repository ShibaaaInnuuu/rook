import { parseAbi } from "viem";

/** The functions the local run calls, by their Solidity signatures. Nothing here can send; it only encodes and decodes. */
export const REGISTRY = parseAbi([
  "function count() view returns (uint256)",
  "function vaultOf(uint256 id) view returns (address)",
  "function leafOf(string inscriptionId) pure returns (bytes32)",
  "function keeper() view returns (address)", "function purchaser() view returns (address)", "function attestor() view returns (address)",
]);
export const FACTORY = parseAbi([
  "function launchFee() view returns (uint256)",
  "function getLaunchedToken(address token) view returns (address token_, address curve, address deployer, address creatorFeeRecipient, address pairToken, uint256 graduationThreshold, uint24 poolFee, int24 tickSpacing, uint16 creatorTaxBps, bool buybackEnabled, uint8 phase, uint256 sweptQuote, uint256 sweptTokens, uint256 sweptAt, bool exists)",
]);
export const LAUNCHER = parseAbi([
  "struct Socials { string twitter; string telegram; string discord; string website; string farcaster; }",
  "struct LaunchParams { string name; string symbol; string logo; string description; Socials socials; uint16 creatorTaxBps; bytes32 expectedEconomics; bytes32 salt; uint256 launchConfigId; uint256 listingId; }",
  "function launch(LaunchParams p) payable returns (address coin, address curve, address router)",
  "function routerOf(address coin) view returns (address)",
  "event Launched(address indexed coin, address indexed creator, uint256 indexed listingId, address curve, address router)",
]);
export const CURVE = parseAbi(["function buy(uint256 quoteIn, uint256 minTokensOut, address recipient) payable returns (uint256)"]);
export const ROUTER = parseAbi(["function harvest() returns (uint256 toVault, uint256 toTreasury)"]);
export const VAULT = parseAbi([
  "function attributed() view returns (uint256)", "function reserved() view returns (uint256)", "function bond() view returns (uint256)",
  "function capWei() view returns (uint256)", "function capExpiry() view returns (uint64)", "function requiredBond() view returns (uint256)",
  "function postCap(uint256 capWei)", "function declarePurchase(uint256 maxEth, uint256 quantity, string capRef) returns (uint256 id)",
  "function postBond() payable", "function release(uint256 id)",
  "function settleInscription(uint256 id, string inscriptionId, bytes32 btcTxid, uint64 satsPaid, bytes32[] proof) returns (uint256)",
  "function purchase(uint256 id) view returns ((uint8 status, uint64 releasableAt, uint64 deadline, address purchaser, uint256 maxEth, uint256 quantity, uint256 settledQuantity, uint256 returned, string capRef))",
  "function purchaseCount() view returns (uint256)", "function acquisitionCount() view returns (uint256)",
  "function acquisition(uint256 id) view returns ((uint256 purchaseId, address coin, uint256 amount, bytes32 btcTxid, uint64 satsPaid, string assetRef))",
]);
export const DISTRIBUTOR = parseAbi([
  "function choose(uint256 listingId, uint256 acquisitionId, uint8 mode, address named)",
  "function openDraw(uint256 listingId, uint256 acquisitionId, bytes32 root, uint256 totalTickets, string uri)",
  "function requestDraw(uint256 listingId, uint256 acquisitionId)",
  "function resolve(uint256 listingId, uint256 acquisitionId, bytes signature)",
  "function claimWin(uint256 listingId, uint256 acquisitionId, address winner, uint256 firstTicket, uint256 endTicket, bytes32[] proof)",
  "function setDestination(uint256 listingId, uint256 acquisitionId, string taprootAddress)",
  "function recordDelivery(uint256 listingId, uint256 acquisitionId, bytes32 txid)",
  "function lot(uint256 listingId, uint256 acquisitionId) view returns ((uint8 mode, uint8 stage, bool overdue, uint64 challengeEnds, uint64 round, uint64 pinnedAt, uint64 destinationAt, address winner, bytes32 root, uint256 totalTickets, uint256 winningTicket, bytes32 deliveryTxid, string uri, string destination))",
]);
export const BEACON = parseAbi([
  "function timeOf(uint64 round) view returns (uint256)", "function randomnessOf(uint64 round) view returns (bytes32)",
  "function prove(uint64 round, bytes signature) returns (bytes32)", "function roundAt(uint256 timestamp) view returns (uint64)",
]);
