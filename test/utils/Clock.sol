// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Vm} from "forge-std/Vm.sol";
import {Wait} from "../../src/lib/Wait.sol";

/// @notice Advance time the way the chain actually does: the sequencer's seconds AND the parent chain's blocks.
/// @dev `vm.warp` alone moves only one of the two clocks the contracts now read, and a test that moves only one is testing
/// a chain that does not exist. That every waiting period in the system needed this helper is the point of `src/lib/Wait.sol`:
/// a timestamp on its own was never a delay anybody had to respect.
library Clock {
    Vm private constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(block.number + Wait.blocksFor(secs, 12) + 1);
    }

    /// @notice Jump to an absolute timestamp, moving the parent chain with it.
    function warpTo(uint256 ts) internal {
        uint256 at = vm.getBlockTimestamp();
        vm.warp(ts);
        if (ts > at) vm.roll(block.number + Wait.blocksFor(ts - at, 12) + 1);
    }

    /// @notice Move only the sequencer's clock, leaving the parent chain where it is — exactly what a sequencer stamping a
    /// block in the future can do, and nothing more.
    function skipTimeOnly(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
    }
}
