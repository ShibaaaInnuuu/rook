// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase} from "../utils/SweepBase.sol";
import {Merkle} from "../utils/Merkle.sol";
import {SweepVault} from "../../src/SweepVault.sol";

/// Second review (R2). N-2: the C-04 fix picks the snapshot time from a drand round nobody saw at the announcement, but the
/// keeper (by not opening) and the owner/guardian (by cancelling an Open draw, whose snapshot time is public) can discard a
/// snapshot time after seeing it and re-roll. The fix doc says "a cancel steers nothing" and "nobody can see an outcome and pick
/// another round".
contract R2Draws is SweepBase {
    address guardian = makeAddr("guardian");

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

    /// A deterministic stand-in for drand: the round's randomness is fixed, nobody chooses it.
    function _drand(uint64 round) internal pure returns (bytes32) { return keccak256(abi.encode("drand", round)); }

    /// The keeper wants the snapshot in the hour before the announcement (when a friend held). It announces, reads the seed round
    /// once public, and simply does not open unless the snapshot falls there; anyone voids the lapsed draw after OPEN_TTL; repeat.
    function test_N2a_theKeeperRerollsTheSnapshotTimeByNotOpening() public {
        uint256 tries;
        uint256 id;
        uint64 at;
        while (true) {
            tries++;
            vm.prank(keeper);
            id = raffle.announceDraw(coinR);
            SweepVault.Draw memory d = raffle.draw(id);
            _skip(2 minutes);                                      // the seed round is public
            at = raffle.snapshotTimeOf(id, _drand(d.seedRound));
            if (at + 1 hours >= d.announcedAt) break;             // in the friend's hour: open it
            _skip(6 hours);
            vm.prank(stranger);
            raffle.voidDraw(id);                                  // looks like keeper downtime; the unit returns
            assertLt(tries, 200, "bounded");
        }
        bytes32[] memory tree = new bytes32[](2);
        tree[0] = raffle.leaf(stranger, 0, 10);
        tree[1] = raffle.leaf(address(0), 10, at);
        bytes memory sig = abi.encodePacked(_drand(raffle.draw(id).seedRound));
        vm.prank(keeper);
        raffle.openDraw(id, Merkle.root(tree), 10, "ipfs://x", Merkle.proof(tree, 1), sig);
        assertEq(raffle.draw(id).snapshotAt, at);
        emit log_named_uint("announcements discarded before the snapshot landed in the chosen hour", tries - 1);
        assertGt(tries, 1, "at least one re-roll was needed (and allowed)");
    }

    /// The guardian (an automated hot key) or the Safe sees an Open draw's snapshot time and ticket list, and cancels it before
    /// the pin; the next announcement draws a fresh snapshot time.
    function test_N2b_theGuardianCancelsAnOpenDrawAfterSeeingItsSnapshot() public {
        (uint256 id,) = _openDrawOne(raffle, coinR, stranger, 10);
        assertEq(uint8(raffle.draw(id).stage), uint8(SweepVault.Stage.Open));
        uint64 first = raffle.draw(id).snapshotAt;
        _skip(20 minutes);                                        // past the 15-minute challenge window, still unpinned
        vm.prank(guardian);
        raffle.cancelDraw(id);
        assertEq(uint8(raffle.draw(id).stage), uint8(SweepVault.Stage.Cancelled));
        (uint256 id2,) = _openDrawOne(raffle, coinR, stranger, 10);
        assertTrue(raffle.draw(id2).snapshotAt != first, "a new snapshot time");
    }
}
