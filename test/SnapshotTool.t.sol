// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "./utils/Base.sol";
import {Merkle} from "./utils/Merkle.sol";
import {Distributor} from "../src/Distributor.sol";
import {Clock} from "./utils/Clock.sol";

/// docs/snapshot-v1.md: the snapshots in test/fixtures/ were written by tools/snapshot (npm run fixtures), not by Solidity.
/// These tests hand the tool's root, ranges, amounts and proofs to the real Distributor. If the tool's leaf or tree format
/// drifted from what the contract verifies, nothing here could be claimed.
contract SnapshotToolTest is Base {
    bytes32 constant TXID = bytes32(uint256(0xB17C));

    function _json(string memory name) internal view returns (string memory) {
        return vm.readFile(string.concat("test/fixtures/", name, ".json"));
    }
    function _u(string memory json, string memory path) internal pure returns (uint256) {
        return vm.parseUint(vm.parseJsonString(json, path));
    }
    function _entry(uint256 i, string memory field) internal pure returns (string memory) {
        return string.concat(".entries[", vm.toString(i), "].", field);
    }

    function _inscription() internal returns (uint256 aid) {
        _fund(ordVault, coinA, 1 ether);
        vm.prank(keeper); ordVault.postCap(1 ether);
        vm.prank(keeper); uint256 id = ordVault.declarePurchase(1 ether, 1, "f");
        Clock.skip(1 hours);
        ordVault.release(id);
        vm.prank(attestor);
        aid = ordVault.settleInscription(id, ids[0], TXID, 1, Merkle.proof(idLeaves, 0));
    }

    function _draw(string memory fixture, Distributor.Mode mode) internal {
        string memory json = _json(fixture);
        uint256 n = vm.parseJsonUint(json, ".holders");
        uint256 total = _u(json, ".totalTickets");
        bytes32 root = vm.parseJsonBytes32(json, ".root");
        uint256 aid = _inscription();
        vm.prank(creatorA); distributor.choose(0, aid, mode, address(0));
        vm.prank(keeper); distributor.openDraw(0, aid, root, total, "ipfs://from-the-tool");
        Clock.skip(15 minutes + 1);
        distributor.requestDraw(0, aid);
        Clock.skip(30);
        distributor.resolve(0, aid, abi.encodePacked(keccak256("a round's signature")));
        uint256 winning = distributor.lot(0, aid).winningTicket;
        assertLt(winning, total);

        // the tool's ranges tile [0, total) with no gap and no overlap, and the entry that holds the winning ticket claims with the tool's proof
        uint256 expectFirst;
        bool claimed;
        for (uint256 i = 0; i < n; ++i) {
            address holder = vm.parseJsonAddress(json, _entry(i, "holder"));
            uint256 first = _u(json, _entry(i, "first"));
            uint256 end = _u(json, _entry(i, "end"));
            assertEq(first, expectFirst, "contiguous");
            assertGt(end, first);
            expectFirst = end;
            assertEq(vm.parseJsonBytes32(json, _entry(i, "leaf")), distributor.ticketLeaf(holder, first, end), "the tool's leaf IS the contract's leaf");
            if (winning >= first && winning < end) {
                distributor.claimWin(0, aid, holder, first, end, vm.parseJsonBytes32Array(json, _entry(i, "proof")));
                assertEq(distributor.lot(0, aid).winner, holder);
                claimed = true;
            }
        }
        assertEq(expectFirst, total, "the ranges end exactly at totalTickets");
        assertTrue(claimed, "whoever holds the winning ticket could prove it");
    }

    function test_aWeightedDrawSettlesOnTheToolsRootAndProof() public { _draw("snapshot-weighted", Distributor.Mode.DrawWeighted); }
    function test_aPerWalletDrawSettlesOnTheToolsRootAndProof() public { _draw("snapshot-per-wallet", Distributor.Mode.DrawPerWallet); }

    function test_everyHolderClaimsTheirProRataShareWithTheToolsProof_andTheRemainderIsNobodys() public {
        string memory json = _json("snapshot-pro-rata");
        uint256 n = vm.parseJsonUint(json, ".holders");
        uint256 total = _u(json, ".inputs.total");
        launcher.set(coinA, router, creatorA, 1);
        uint256 aid = runeVault.acquisitionCount();
        _fund(runeVault, coinA, 1 ether);
        vm.prank(keeper); runeVault.postCap(1 ether);
        vm.prank(keeper); uint256 id = runeVault.declarePurchase(1 ether, total, "f");
        Clock.skip(1 hours);
        runeVault.release(id);
        vm.prank(attestor); runeVault.settleRune(id, total, TXID, 1);
        vm.prank(creatorA); distributor.choose(1, aid, Distributor.Mode.ProRata, address(0));
        vm.prank(keeper); distributor.openDistribution(1, aid, vm.parseJsonBytes32(json, ".root"), "ipfs://from-the-tool");
        Clock.skip(15 minutes + 1);

        uint256 sum;
        for (uint256 i = 0; i < n; ++i) {
            address holder = vm.parseJsonAddress(json, _entry(i, "holder"));
            uint256 units = _u(json, _entry(i, "units"));
            distributor.claimShare(1, aid, holder, units, vm.parseJsonBytes32Array(json, _entry(i, "proof")));
            assertEq(distributor.accrued(1, holder), units);
            sum += units;
        }
        assertEq(sum, _u(json, ".allocated"));
        assertEq(sum + _u(json, ".remainder"), total, "allocated + remainder is the whole acquisition; nobody is given the rounding");
        assertLe(sum, total);
    }
}
