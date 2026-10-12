import { parseAbi } from "viem";

/** The views and events the v8 reads need, copied by signature from src/nft/EthNftWrapper.sol, src/floor/FloorModule.sol and Nitro. */
export const ETH_NFT_WRAPPER = parseAbi([
  "function totalSupply() view returns (uint256)",
  "function held() view returns (uint256)",
  "function openTickets() view returns (uint256)",
  "function ethBacked() view returns (uint256)",
  "function itemCount() view returns (uint256)",
  "function item(uint256 itemId) view returns ((uint256 tokenId, bytes32 sourceTx, bool deposit))",
  "function itemOf(uint256 tokenId) view returns (uint256)",
  "function ticketCount() view returns (uint256)",
  "function ticket(uint256 id) view returns ((uint8 stage, address owner, uint64 round, uint64 pinnedAt, uint64 pinnedAtBlock, uint64 deliverBy, uint64 deliverByBlock, uint256 itemId, bytes32 deliveryTx, address destination))",
  "function nextToResolve() view returns (uint256)",
  "function freeCount() view returns (uint256)",
  "function freeAt(uint256 i) view returns (uint256)",
  "function pendingCount() view returns (uint256)",
  "function pendingAt(uint256 i) view returns (uint256 itemId, uint256 stamp)",
  "function beacon() view returns (address)",
  "event Wrapped(uint256 indexed itemId, uint256 indexed tokenId, bytes32 purchaseTx, address indexed to)",
  "event Deposited(uint256 indexed itemId, uint256 indexed tokenId, bytes32 depositTx, address indexed to)",
  "event Redeemed(uint256 indexed ticketId, address indexed owner, uint64 round, address destination)",
  "event Resolved(uint256 indexed ticketId, uint256 indexed itemId, uint256 indexed tokenId, uint64 deliverBy)",
  "event Delivered(uint256 indexed ticketId, uint256 indexed tokenId, address destination, bytes32 txHash)",
  "event Undeliverable(uint256 indexed ticketId)",
  "event Refunded(uint256 indexed ticketId, uint256 indexed itemId, address indexed owner)",
]);
/** EthNftWrapper.Stage */
export const STAGES = ["none", "pinned", "resolved", "delivered", "refunded"] as const;

export const FLOOR_MODULE = parseAbi([
  "function ethBackedUnits() view returns (uint256)",
  "function reserveUnits() view returns (uint256)",
  "function netUnits() view returns (uint256)",
  "function transitUnits() view returns (uint256)",
  "function escrowBalance() view returns (uint256)",
  "function revenue() view returns (uint256)",
  "function advanced() view returns (uint256)",
  "function advancedAt() view returns (uint64)",
  "event AdvanceSettled(uint256 indexed item, uint256 advanced, uint256 cost)",
]);

export const ERC721 = parseAbi([
  "function ownerOf(uint256 tokenId) view returns (address)",
  "event Transfer(address indexed from, address indexed to, uint256 indexed tokenId)",
  "function safeTransferFrom(address from, address to, uint256 tokenId, bytes data)",
]);

/** Seaport 1.6 and 1.5 (docs/v7/phase1-nft.md: the canister fills OpenSea listings through Seaport). */
export const SEAPORTS = ["0x0000000000000068f116a894984e2db1123eb395", "0x00000000000000adc04c56bf30ac9d3c0aaf14dc"] as const;
export const SEAPORT = parseAbi([
  "event OrderFulfilled(bytes32 orderHash, address indexed offerer, address indexed zone, address recipient, (uint8 itemType, address token, uint256 identifier, uint256 amount)[] offer, (uint8 itemType, address token, uint256 identifier, uint256 amount, address recipient)[] consideration)",
]);

/** Arbitrum Nitro's NodeInterface (a virtual contract every Nitro node answers at 0xc8): which batch carried a block, and its depth. */
export const NODE_INTERFACE = "0x00000000000000000000000000000000000000C8" as const;
export const NITRO = parseAbi([
  "function findBatchContainingBlock(uint64 blockNum) view returns (uint64 batch)",
  "function getL1Confirmations(bytes32 blockHash) view returns (uint64 confirmations)",
]);
