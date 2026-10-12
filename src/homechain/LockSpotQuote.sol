// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IPoolManager} from "@uniswap/v4-core/src/interfaces/IPoolManager.sol";
import {PoolKey} from "@uniswap/v4-core/src/types/PoolKey.sol";
import {PoolId, PoolIdLibrary} from "@uniswap/v4-core/src/types/PoolId.sol";
import {Currency} from "@uniswap/v4-core/src/types/Currency.sol";
import {StateLibrary} from "@uniswap/v4-core/src/libraries/StateLibrary.sol";
import {FullMath} from "@uniswap/v4-core/src/libraries/FullMath.sol";
import {LiquidityLock} from "../pools/LiquidityLock.sol";
import {IEthQuote} from "./HomeReceiver.sol";

/// @title LockSpotQuote
/// @notice The pools' spot price as a quote: how much of `coin` `ethIn` buys at the current sqrt price, ignoring fee and impact.
/// For a wX-quoted coin it composes the shared ETH/wX pool and the coin's wX pool, as `EtchRouter.buy` routes. A bound for a
/// buyback's `minOut`, not an oracle: the spot can be moved within a block, and `HomeReceiver` allows 5% under it per tranche.
contract LockSpotQuote is IEthQuote {
    using StateLibrary for IPoolManager;
    using PoolIdLibrary for PoolKey;

    LiquidityLock public immutable lock;
    IPoolManager public immutable poolManager;

    error NotAPool();

    constructor(LiquidityLock lock_) {
        lock = lock_;
        poolManager = lock_.poolManager();
    }

    function coinOutForEth(address coin, uint256 ethIn) external view returns (uint256) {
        (address curve, bool quoteIsEth,) = lock.coins(coin);
        if (curve == address(0)) revert NotAPool();
        uint256 quoteIn = ethIn;
        if (!quoteIsEth) quoteIn = _out(lock.sharedKey(), address(0), ethIn);
        return _out(lock.coinKey(coin), quoteIsEth ? address(0) : address(lock.wrapped()), quoteIn);
    }

    function _out(PoolKey memory key, address tokenIn, uint256 amountIn) private view returns (uint256) {
        (uint160 sqrtP,,,) = poolManager.getSlot0(key.toId());
        if (sqrtP == 0) revert NotAPool();
        uint256 q96 = 2 ** 96;
        if (Currency.unwrap(key.currency0) == tokenIn) {
            // token1 per token0 = (sqrtP / 2^96)^2
            return FullMath.mulDiv(FullMath.mulDiv(amountIn, sqrtP, q96), sqrtP, q96);
        }
        return FullMath.mulDiv(FullMath.mulDiv(amountIn, q96, sqrtP), q96, sqrtP);
    }
}
