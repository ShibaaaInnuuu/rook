// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice A source of public randomness that arrives in numbered rounds at known times and can be proven by anyone.
interface IRandomnessBeacon {
    /// @notice The round current at `timestamp` (0 before genesis).
    function roundAt(uint256 timestamp) external view returns (uint64);
    /// @notice When `round` is due.
    function timeOf(uint64 round) external view returns (uint256);
    /// @notice The proven randomness of `round`, or zero if nobody has proven it yet.
    function randomnessOf(uint64 round) external view returns (bytes32);
    /// @notice Verify and store `round`. Permissionless; idempotent.
    function prove(uint64 round, bytes calldata signature) external returns (bytes32);
}
