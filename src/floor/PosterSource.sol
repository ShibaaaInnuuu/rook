// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IKeeperSource} from "../interfaces/IKeeperSource.sol";

/// @title PosterSource
/// @notice Where an Ethereum-NFT collection's `FloorModule` finds its keeper (docs/v7/phase1-nft.md §3.2): the canister, which reads
/// the floor itself from verified Seaport listings and on-chain sales, instead of the registry's keeper EOA. The registry's incident
/// switch zeroes the poster at once (the Safe or the guardian pausing the keeper stops every inflow priced by the poster's word, and
/// burns go on at the pause floor), and its owner and guardian are read through.
///
/// This collection's own floor switch (audit 2026-09-27-nft, D-M-1 / A-L-3): the registry's owner or guardian may `pause()` this
/// poster alone, and only the owner `unpause()`s it. While paused, `keeper()` is zero and `keeperPaused()` true for this module only:
/// the monitor's floor guard (D8) stops the collection whose floor it doubts, not every collection at once. The registry's pause
/// stays the canister-wide switch. A resume of either (the later of the two) starts the module's RESUME_GRACE.
/// @dev No setter besides the switch. The poster is the canister's EVM address, which does not change across its upgrades (one
/// threshold key, one derivation path); a different poster is a different module, which a different collection deployment is anyway.
contract PosterSource is IKeeperSource {
    IKeeperSource public immutable registry;
    address public immutable poster;

    /// @notice This collection's floor switch: set by the registry's owner or guardian at once, cleared by the owner only.
    bool public paused;
    /// @notice When the owner last resumed this poster from a pause (0: never), on both clocks.
    uint64 public resumedAt;
    uint64 public resumedAtBlock;

    event PosterPaused(address indexed by);
    event PosterUnpaused(address indexed by);

    error BadInput();
    error NotGuardian();
    error NotOwner();

    constructor(IKeeperSource registry_, address poster_) {
        if (address(registry_) == address(0) || poster_ == address(0) || poster_.code.length != 0) revert BadInput();
        registry = registry_;
        poster = poster_;
    }

    /// @notice The registry's owner or guardian, at once: this module's poster is zeroed until the owner resumes it.
    function pause() external {
        address g = registry.guardian();
        if (msg.sender != registry.owner() && (g == address(0) || msg.sender != g)) revert NotGuardian();
        paused = true;
        emit PosterPaused(msg.sender);
    }

    /// @notice The owner only. A real resume (from paused) is stamped on both clocks.
    function unpause() external {
        if (msg.sender != registry.owner()) revert NotOwner();
        if (paused) {
            resumedAt = uint64(block.timestamp);
            resumedAtBlock = uint64(block.number);
        }
        paused = false;
        emit PosterUnpaused(msg.sender);
    }

    /// @notice The poster, or zero while this poster or the registry's keeper is paused.
    function keeper() external view returns (address) {
        return paused || registry.keeperPaused() ? address(0) : poster;
    }

    function keeperPaused() external view returns (bool) { return paused || registry.keeperPaused(); }

    /// @notice The later resume of the two switches (so the module's RESUME_GRACE runs from whichever resumed last).
    function keeperResumedAt() external view returns (uint64) {
        uint64 r = registry.keeperResumedAt();
        return resumedAt > r ? resumedAt : r;
    }

    function keeperResumedAtBlock() external view returns (uint64) {
        uint64 r = registry.keeperResumedAtBlock();
        return resumedAtBlock > r ? resumedAtBlock : r;
    }

    function owner() external view returns (address) { return registry.owner(); }
    function guardian() external view returns (address) { return registry.guardian(); }
}
