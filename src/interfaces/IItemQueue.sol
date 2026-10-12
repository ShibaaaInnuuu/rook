// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IWrappedCollection} from "./IWrappedCollection.sol";
import {ICapSource} from "./ICapSource.sol";

/// @title IItemQueue
/// @notice Where a graduating curve converts its item budget into wrapped units: the collection's `ItemQueue` (docs/v3/pools.md
/// §5: the canister buys items over hours) or its `FloorModule` (docs/v5/floor-module.md: converted at once, at the floor). The
/// curve cannot tell which, and does not need to.
interface IItemQueue {
    /// @notice The coin's curve, at the close of its raise: its item budget, with the curve's refund deadline.
    function enqueue(address coin, uint64 deadlineAt, uint64 deadlineAtBlock) external payable;
    /// @notice Whether the coin's units are ready to be released to its curve.
    function isFilled(address coin) external view returns (bool);
    /// @notice Whether an item budget of `eth` closing now would be converted without waiting on room (audit 2026-09-26 A-03):
    /// the factory refuses a launch, and a curve the buy that would close it, while this is false.
    function hasRoomFor(uint256 eth) external view returns (bool);
    /// @notice Whether conversion is paused by an incident (the floor module: the keeper is paused, or was resumed less than
    /// `RESUME_GRACE` ago), so a curve whose window passed unfilled should wait — up to its escape hatch, never longer — before
    /// opening refunds (review2 N-3, R3 check A3-L1).
    function holdsRefunds() external view returns (bool);
    /// @notice The coin's curve, once filled: its units, what they cost, and the ETH left over.
    function release(address coin) external returns (uint256 units, uint256 spent, uint256 dust);
    /// @notice The coin's curve, its window passed unfilled: its unspent ETH and any units already assigned.
    function cancel(address coin) external returns (uint256 eth, uint256 units);
    function wrapper() external view returns (IWrappedCollection);
    function factory() external view returns (ICapSource);
    /// @notice The canister's margin over the per-item cap before it buys, bps.
    function bufferBps() external view returns (uint256);
    /// @notice A coin's fee slice for the collection's standing reserve (docs/v5/floor-module.md §13.2). A floor module repays
    /// its bootstrap loan first, then keeps the rest as reserve; an `ItemQueue` collection, which has no reserve, adds it to
    /// protocol liquidity. Anyone may call it with ETH.
    function fundReserve() external payable;
    /// @notice A coin's fee slice for protocol-owned liquidity: a floor module repays its bootstrap loan first, then hands the
    /// rest to the collection's `LiquidityLock`. Anyone may call it with ETH.
    function fundLiquidity() external payable;
}
