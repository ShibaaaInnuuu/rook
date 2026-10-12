// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";

/// @title LiquidityAmounts
/// @notice The most liquidity a pair of amounts can add to a range at a price. Rounds DOWN, so the pool never asks for more than
/// the amounts given (the lock also keeps two wei back for the pool's own round-ups).
library LiquidityAmounts {
    uint256 internal constant Q96 = 2 ** 96;

    function forAmount0(uint160 sqrtA, uint160 sqrtB, uint256 amount0) internal pure returns (uint256) {
        if (sqrtA > sqrtB) (sqrtA, sqrtB) = (sqrtB, sqrtA);
        return Math.mulDiv(amount0, Math.mulDiv(sqrtA, sqrtB, Q96), sqrtB - sqrtA);
    }

    function forAmount1(uint160 sqrtA, uint160 sqrtB, uint256 amount1) internal pure returns (uint256) {
        if (sqrtA > sqrtB) (sqrtA, sqrtB) = (sqrtB, sqrtA);
        return Math.mulDiv(amount1, Q96, sqrtB - sqrtA);
    }

    function forAmounts(uint160 sqrtP, uint160 sqrtA, uint160 sqrtB, uint256 amount0, uint256 amount1)
        internal pure returns (uint128)
    {
        if (sqrtA > sqrtB) (sqrtA, sqrtB) = (sqrtB, sqrtA);
        uint256 l;
        if (sqrtP <= sqrtA) {
            l = forAmount0(sqrtA, sqrtB, amount0);
        } else if (sqrtP < sqrtB) {
            l = Math.min(forAmount0(sqrtP, sqrtB, amount0), forAmount1(sqrtA, sqrtP, amount1));
        } else {
            l = forAmount1(sqrtA, sqrtB, amount1);
        }
        return l > type(uint128).max / 2 ? type(uint128).max / 2 : uint128(l);
    }
}
