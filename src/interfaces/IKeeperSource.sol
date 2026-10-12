// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice Where a contract finds its keeper (the ListingRegistry): the one key rotated by the owner in seven public days, and
/// the incident switch that zeroes it at once (docs/audit/2026-09-26/fixes-B-C-keeper-draws.md §0).
interface IKeeperSource {
    /// @notice The keeper every contract obeys now; zero while the keeper is paused.
    function keeper() external view returns (address);
    /// @notice Whether the owner or the guardian has paused every keeper power (and the inflows priced by the keeper's word).
    function keeperPaused() external view returns (bool);
    /// @notice When the owner last resumed a paused keeper (0: never), and the parent chain's block then (R3 check A3-L1).
    function keeperResumedAt() external view returns (uint64);
    function keeperResumedAtBlock() external view returns (uint64);
    /// @notice The owner (the Safe).
    function owner() external view returns (address);
    /// @notice A key that can only stop things (zero: none).
    function guardian() external view returns (address);
}
