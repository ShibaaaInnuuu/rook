// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ILiquiditySeeder} from "./ILiquiditySeeder.sol";

/// @title ICapSource
/// @notice The collection's keeper-posted price per whole wrapped item (the sweep vaults' cap: floor, all-in, one-hour TTL on both
/// clocks), and the one address allowed to act for each coin. `CurveFactory` is it for its collection.
interface ICapSource {
    /// @return cap wei per whole wrapped item, from the freshest of the collection's vault caps
    /// @return fresh false if no cap is fresh on both clocks
    function freshCap() external view returns (uint256 cap, bool fresh);
    function routerOf(address coin) external view returns (address);
    function wrapped() external view returns (address);
    /// @notice The collection's liquidity lock (docs/v5/floor-module.md §13.2): where protocol-owned liquidity is added.
    function seeder() external view returns (ILiquiditySeeder);
}
