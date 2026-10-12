// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice What happens to everything an NFT listing buys. Fixed when the listing is made; one vault per fate, so the two kinds of
/// money never share a balance. docs/v4/nft.md §2, §7.
enum Policy { Raffle, Burn }

/// @notice What a vault reads from the NFT registry. Declared here so the vaults and the registry that builds them do not import
/// each other.
interface INftRegistry {
    function owner() external view returns (address);
    function keeper() external view returns (address);
    function treasury() external view returns (address);
    function launcher() external view returns (address);
    function isPaused(uint256 id) external view returns (bool);
    function isSweepTarget(address target) external view returns (bool);
    function vaultOf(uint256 id) external view returns (address);
}

/// @notice What a vault reads from the NFT launcher.
interface INftLauncher {
    function routerOf(address coin) external view returns (address);
    function creatorOf(address coin) external view returns (address);
    function listingOf(address coin) external view returns (uint256);
}

/// @notice What a coin's router needs from its vault.
interface INftVault {
    function deposit(address coin) external payable;
}
