// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title StandInSafe
/// @notice REHEARSAL ONLY. `scripts/golive-dryrun.sh stack` plants this runtime with `anvil_setCode` at the rehearsal's owner (an
/// anvil dev account) so that `script/golive-check.sh --phase1` can read a Safe's `getThreshold()` and `getOwners()` on a local fork,
/// and so a 1-of-2 owner can be shown to FAIL. It is never deployed and holds nothing. Storage: slot 0 the threshold, slot 1 the
/// number of owners (set with `anvil_setStorageAt`; both 0 after `anvil_setCode`).
contract StandInSafe {
    uint256 internal threshold;
    uint256 internal ownerCount;

    function getThreshold() external view returns (uint256) {
        return threshold;
    }

    function getOwners() external view returns (address[] memory owners) {
        owners = new address[](ownerCount);
        for (uint256 i; i < ownerCount; ++i) owners[i] = address(uint160(0x5AFE0000 + i + 1));
    }

    receive() external payable {}
}
