// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase} from "../utils/SweepBase.sol";
import {Merkle} from "../utils/Merkle.sol";
import {SweepVault} from "../../src/SweepVault.sol";

/// R3 check of the seed carry and `Stage.Empty`.
contract Audit3Draws is SweepBase {
    address guardian = makeAddr("guardian");
    address holder = makeAddr("holder");

    function setUp() public override {
        super.setUp();
        _stockVenue(40);
        _sweepWhole(raffle, coinR, routerR, 30);
        vm.prank(owner);
        registry.setGuardian(guardian);
    }

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    function _drand(uint64 round) internal pure returns (bytes32) { return keccak256(abi.encode("drand", round)); }
    function _sig(uint256 id) internal view returns (bytes memory) { return abi.encodePacked(_drand(raffle.draw(id).seedRound)); }
    function _at(uint256 id) internal view returns (uint64) { return raffle.snapshotTimeOf(id, _drand(raffle.draw(id).seedRound)); }

    function _announce() internal returns (uint256 id) {
        vm.prank(keeper);
        id = raffle.announceDraw(coinR);
        _skip(2 minutes);
    }

    /// Original N-2 idea, re-run: the keeper declines to open 30 times (and the guardian cancels open draws); the snapshot time
    /// never changes.
    function test_audit3_N2_reroll_by_void_and_cancel_is_dead() public {
        uint256 first = _announce();
        uint64 at0 = _at(first);
        for (uint256 i = 0; i < 30; ++i) {
            _skip(6 hours);
            raffle.voidDraw(first + i);
            uint256 id = _announce();
            assertEq(_at(id), at0, "same time after a void");
        }
        // guardian cancels an Open draw after seeing its list
        uint256 cur = first + 30;
        bytes32[] memory tree = new bytes32[](2);
        tree[0] = raffle.leaf(holder, 0, 5);
        tree[1] = raffle.leaf(address(0), 5, _at(cur));
        bytes memory sig = _sig(cur);
        vm.prank(keeper);
        raffle.openDraw(cur, Merkle.root(tree), 5, "u", Merkle.proof(tree, 1), sig);
        vm.prank(guardian);
        raffle.cancelDraw(cur);
        uint256 next = _announce();
        assertEq(_at(next), at0, "same time after a cancel of an Open draw");
    }

    /// A3-I2, now a regression: a keeper-only re-roll that did not need the Empty stage, opening with totalTickets = N > 0 and root =
    /// the sentinel leaf alone (`leaf(0, N, at)`, empty proof; `MerkleProof.verify([], root, root)` is true). `openDraw` now refuses
    /// an empty sentinel proof when there are tickets (a list with tickets has a holder leaf beside its sentinel), so the carry
    /// cannot be consumed by a draw nobody can win. The honest list of the same snapshot then opens and draws.
    function test_audit3_sentinelOnlyNonEmptyRootIsRefused() public {
        uint256 first = _announce();
        _skip(6 hours);
        raffle.voidDraw(first);
        uint256 id = _announce();
        assertEq(raffle.seedOrigin(id), first);
        uint64 at = _at(id);
        bytes32 root = raffle.leaf(address(0), 7, at);
        bytes memory sig = _sig(id);
        vm.prank(keeper);
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.openDraw(id, root, 7, "u", new bytes32[](0), sig);
        assertEq(raffle.carriedSeed(coinR), first + 1, "the carry is untouched");

        // the honest list of the same snapshot time opens and pins
        bytes32[] memory tree = new bytes32[](2);
        tree[0] = raffle.leaf(holder, 0, 7);
        tree[1] = raffle.leaf(address(0), 7, at);
        vm.prank(keeper);
        raffle.openDraw(id, Merkle.root(tree), 7, "u", Merkle.proof(tree, 1), sig);
        _skip(16 minutes);
        raffle.requestDraw(id);
        assertEq(raffle.carriedSeed(coinR), 0, "consumed by a draw the holder can win");
    }
}
