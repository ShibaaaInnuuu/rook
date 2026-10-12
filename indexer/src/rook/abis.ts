import { parseAbi } from "viem";

/** The v3 contracts' events and views the board reads, copied from src/ by signature (FrogCurve, CurveFactory, LiquidityLock, the coin as an ERC-20, Uniswap v4's PoolManager). */
export const CURVE_TRADE_EVENTS = parseAbi([
  "event Bought(address indexed buyer, uint256 ethIn, uint256 fee, uint256 tokensOut)",
  "event Sold(address indexed seller, uint256 tokensIn, uint256 ethOut, uint256 fee)",
  "event Graduated(uint256 unitsToCoinPool, uint256 unitsToShared, uint256 ethToShared)",
]);
export const CURVE_VIEWS = parseAbi([
  "function feeBps() view returns (uint256)",
  "function launchedAt() view returns (uint64)",
  "function totalSupply() view returns (uint256)",
]);
export const FACTORY_VIEWS = parseAbi(["function feeOf(address coin) view returns (uint256)", "function policyOf(address coin) view returns (uint8)"]);
export const ERC20_EVENTS = parseAbi(["event Transfer(address indexed from, address indexed to, uint256 value)"]);
export const ERC20_VIEWS = parseAbi(["function totalSupply() view returns (uint256)"]);
export const LOCK_VIEWS = parseAbi(["function coinKey(address coin) view returns ((address currency0, address currency1, uint24 fee, int24 tickSpacing, address hooks))"]);
export const POOL_SWAP = parseAbi(["event Swap(bytes32 indexed id, address indexed sender, int128 amount0, int128 amount1, uint160 sqrtPriceX96, uint128 liquidity, int24 tick, uint24 fee)"]);
