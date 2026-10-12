// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {MerkleProof} from "@openzeppelin/contracts/utils/cryptography/MerkleProof.sol";
import {SweepBase} from "./utils/SweepBase.sol";

/// docs/snapshot-v1.md "Version 2": test/fixtures/snapshot-weighted-v2.json was written by tools/snapshot (npm run fixtures), not by Solidity.
/// The SweepVault's own `leaf` and OpenZeppelin's MerkleProof (what `openDraw` and `claim` verify with) must accept the tool's
/// sentinel leaf and every holder's range under the tool's root. If the tool's sentinel drifted from `leaf(address(0), totalTickets,
/// snapshotAt)`, no draw built by the keeper could be opened.
contract SnapshotToolV2Test is SweepBase {
    function _entry(uint256 i, string memory field) internal pure returns (string memory) {
        return string.concat(".entries[", vm.toString(i), "].", field);
    }

    function test_theToolsSentinelAndRangesVerifyUnderItsRoot_withTheVaultsLeaf() public view {
        string memory json = vm.readFile("test/fixtures/snapshot-weighted-v2.json");
        assertEq(vm.parseJsonUint(json, ".version"), 2);
        bytes32 root = vm.parseJsonBytes32(json, ".root");
        uint256 total = vm.parseUint(vm.parseJsonString(json, ".totalTickets"));
        uint256 at = vm.parseUint(vm.parseJsonString(json, ".inputs.snapshotAt"));

        // the sentinel: holder zero over [totalTickets, snapshotAt), the proof openDraw requires
        bytes32 sentinel = raffle.leaf(address(0), total, at);
        assertEq(vm.parseJsonBytes32(json, ".sentinel.leaf"), sentinel, "the tool's sentinel IS the vault's leaf(address(0), total, snapshotAt)");
        assertTrue(MerkleProof.verify(vm.parseJsonBytes32Array(json, ".sentinel.proof"), root, sentinel), "openDraw's check passes on the tool's proof");
        // a truncated count or another time is another leaf, which the root does not hold
        assertFalse(MerkleProof.verify(vm.parseJsonBytes32Array(json, ".sentinel.proof"), root, raffle.leaf(address(0), total - 1, at)));
        assertFalse(MerkleProof.verify(vm.parseJsonBytes32Array(json, ".sentinel.proof"), root, raffle.leaf(address(0), total, at + 1)));

        // every holder's range: the vault's leaf, contiguous over [0, total), each proof verifies (claim's check)
        uint256 n = vm.parseJsonUint(json, ".holders");
        uint256 expectFirst;
        for (uint256 i = 0; i < n; ++i) {
            address holder = vm.parseJsonAddress(json, _entry(i, "holder"));
            uint256 first = vm.parseUint(vm.parseJsonString(json, _entry(i, "first")));
            uint256 end = vm.parseUint(vm.parseJsonString(json, _entry(i, "end")));
            assertEq(first, expectFirst, "contiguous");
            expectFirst = end;
            bytes32 l = raffle.leaf(holder, first, end);
            assertEq(vm.parseJsonBytes32(json, _entry(i, "leaf")), l);
            assertTrue(MerkleProof.verify(vm.parseJsonBytes32Array(json, _entry(i, "proof")), root, l));
        }
        assertEq(expectFirst, total, "the ranges end exactly at totalTickets, where the sentinel's range starts");
    }
}
