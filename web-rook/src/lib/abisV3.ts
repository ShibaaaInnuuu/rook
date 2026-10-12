/**
 * docs/v5/ux-and-reserves.md. The v3/v4 contracts the one-click ticket and the redeem flow call, by signature, copied from src/.
 * FrogCurve (src/curve/FrogCurve.sol), EtchRouter (src/pools/EtchRouter.sol), LiquidityLock's keys (src/pools/LiquidityLock.sol),
 * Wrapper / NftWrapper's redemption (src/Wrapper.sol, src/nft/NftWrapper.sol), and Uniswap v4's V4Quoter and StateView.
 */
export const FROG_CURVE = [
  { type: "function", name: "buy", stateMutability: "payable", inputs: [{ name: "minOut", type: "uint256" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "sell", stateMutability: "nonpayable", inputs: [{ name: "amountIn", type: "uint256" }, { name: "minEth", type: "uint256" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "refund", stateMutability: "nonpayable", inputs: [{ name: "amount", type: "uint256" }], outputs: [{ type: "uint256" }, { type: "uint256" }] },
  { type: "function", name: "state", stateMutability: "view", inputs: [], outputs: [{ type: "uint8" }] },
  { type: "function", name: "v", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "t", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "vEnd", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "tEnd", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "feeBps", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "launchExtraBps", stateMutability: "view", inputs: [], outputs: [{ type: "uint256" }] },
  { type: "function", name: "sweepOnly", stateMutability: "view", inputs: [], outputs: [{ type: "bool" }] },
] as const;

export const ETCH_ROUTER = [
  { type: "function", name: "buy", stateMutability: "payable", inputs: [{ name: "coin", type: "address" }, { name: "minOut", type: "uint256" }, { name: "to", type: "address" }, { name: "deadline", type: "uint256" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "sell", stateMutability: "nonpayable", inputs: [{ name: "coin", type: "address" }, { name: "amountIn", type: "uint256" }, { name: "minEthOut", type: "uint256" }, { name: "to", type: "address" }, { name: "deadline", type: "uint256" }], outputs: [{ type: "uint256" }] },
] as const;

const POOL_KEY = { type: "tuple", components: [{ name: "currency0", type: "address" }, { name: "currency1", type: "address" }, { name: "fee", type: "uint24" }, { name: "tickSpacing", type: "int24" }, { name: "hooks", type: "address" }] } as const;
export const LOCK = [
  { type: "function", name: "sharedKey", stateMutability: "view", inputs: [], outputs: [POOL_KEY] },
  { type: "function", name: "coinKey", stateMutability: "view", inputs: [{ name: "coin", type: "address" }], outputs: [POOL_KEY] },
  { type: "function", name: "coins", stateMutability: "view", inputs: [{ name: "coin", type: "address" }], outputs: [{ name: "curve", type: "address" }, { name: "quoteIsEth", type: "bool" }, { name: "liquidity", type: "uint128" }] },
] as const;

export const V4_QUOTER = [
  { type: "function", name: "quoteExactInputSingle", stateMutability: "nonpayable",
    inputs: [{ name: "params", type: "tuple", components: [{ name: "poolKey", ...POOL_KEY }, { name: "zeroForOne", type: "bool" }, { name: "exactAmount", type: "uint128" }, { name: "hookData", type: "bytes" }] }],
    outputs: [{ name: "amountOut", type: "uint256" }, { name: "gasEstimate", type: "uint256" }] },
] as const;
export const STATE_VIEW = [
  { type: "function", name: "getSlot0", stateMutability: "view", inputs: [{ name: "poolId", type: "bytes32" }],
    outputs: [{ name: "sqrtPriceX96", type: "uint160" }, { name: "tick", type: "int24" }, { name: "protocolFee", type: "uint24" }, { name: "lpFee", type: "uint24" }] },
] as const;

export const WRAPPER_REDEEM = [
  { type: "function", name: "redeem", stateMutability: "nonpayable", inputs: [{ name: "taprootAddress", type: "string" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "refund", stateMutability: "nonpayable", inputs: [{ name: "id", type: "uint256" }], outputs: [] },
  { type: "function", name: "redraw", stateMutability: "nonpayable", inputs: [{ name: "id", type: "uint256" }], outputs: [] },
  { type: "function", name: "balanceOf", stateMutability: "view", inputs: [{ name: "a", type: "address" }], outputs: [{ type: "uint256" }] },
] as const;
export const NFT_WRAPPER_REDEEM = [
  { type: "function", name: "redeem", stateMutability: "nonpayable", inputs: [{ name: "destination", type: "address" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "refund", stateMutability: "nonpayable", inputs: [{ name: "id", type: "uint256" }], outputs: [] },
  { type: "function", name: "redraw", stateMutability: "nonpayable", inputs: [{ name: "id", type: "uint256" }], outputs: [] },
  { type: "function", name: "balanceOf", stateMutability: "view", inputs: [{ name: "a", type: "address" }], outputs: [{ type: "uint256" }] },
] as const;
export const ERC20_APPROVE = [
  { type: "function", name: "approve", stateMutability: "nonpayable", inputs: [{ name: "spender", type: "address" }, { name: "amount", type: "uint256" }], outputs: [{ type: "bool" }] },
  { type: "function", name: "allowance", stateMutability: "view", inputs: [{ name: "owner", type: "address" }, { name: "spender", type: "address" }], outputs: [{ type: "uint256" }] },
  { type: "function", name: "balanceOf", stateMutability: "view", inputs: [{ name: "a", type: "address" }], outputs: [{ type: "uint256" }] },
] as const;
