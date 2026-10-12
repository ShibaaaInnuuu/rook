// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title ILiquiditySeeder
/// @notice What a graduating curve hands its liquidity to: the collection's `LiquidityLock` (docs/v3/pools.md). It opens the coin's
/// own pool, adds the coin's contribution to the collection's shared wX/ETH pool, and can never give either back.
/// @dev The curve approves exactly the amounts below and sends the ETH as value; the seeder must take all of it or revert.
interface ILiquiditySeeder {
    /// @notice A coin whose raise bought items: COIN/wX gets `tokenAmount` and `wrappedForToken`; the shared pool gets
    /// `wrappedForShared` and `msg.value` ETH.
    function seed(address token, uint256 tokenAmount, uint256 wrappedForToken, uint256 wrappedForShared) external payable;

    /// @notice A SWEEP_ONLY coin: COIN/ETH gets `tokenAmount` and `msg.value`. No shared pool.
    function seedEth(address token, uint256 tokenAmount) external payable;

    /// @notice ETH that graduations have contributed to this collection's shared pool, ever (the bootstrap's depth measure).
    function sharedEthIn() external view returns (uint256);

    /// @notice ETH for protocol-owned liquidity in the shared pool (docs/v5/floor-module.md §13.2): anyone may add, nothing
    /// ever leaves.
    function addProtocolLiquidity() external payable;
}
