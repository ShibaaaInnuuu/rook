// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase} from "../utils/SweepBase.sol";
import {Merkle} from "../utils/Merkle.sol";
import {SweepVault} from "../../src/SweepVault.sol";

/// The carried empty snapshot (docs/audit/2026-09-26/review2/fixes-R3-carry.md; integration-R3.md §8). A draw voided or cancelled
/// before its pin carries its seed round and snapshot time to the coin's next draw (N-2); when nobody eligible held a whole coin at
/// that time, no carrier could be opened and the coin's raffle was stuck for good. Now an empty snapshot is opened as the sentinel
/// alone (`totalTickets` 0) and, after the same challenge window, `requestDraw` closes it with no winner (`Empty`): the carry is
/// consumed, the unit goes back to the coin. A false empty is cancellable in the window, and the cancel carries the same time.
contract R3CarryEmpty is SweepBase {
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

    /// The keeper's empty open: the tree is the sentinel `leaf(0, 0, at)` alone, no proof.
    function _openEmpty(uint256 id) internal {
        bytes32 root = raffle.leaf(address(0), 0, _at(id));
        bytes memory sig = _sig(id);
        vm.prank(keeper);
        raffle.openDraw(id, root, 0, "ipfs://empty", new bytes32[](0), sig);
    }

    /// The keeper's honest open over one holder with `n` tickets.
    function _openHonest(uint256 id, uint256 n) internal returns (bytes32[] memory tree) {
        tree = new bytes32[](2);
        tree[0] = raffle.leaf(holder, 0, n);
        tree[1] = raffle.leaf(address(0), n, _at(id));
        bytes memory sig = _sig(id);
        vm.prank(keeper);
        raffle.openDraw(id, Merkle.root(tree), n, "ipfs://tickets", Merkle.proof(tree, 1), sig);
    }

    function _announce() internal returns (uint256 id) {
        vm.prank(keeper);
        id = raffle.announceDraw(coinR);
        _skip(2 minutes);
    }

    function _stage(uint256 id) internal view returns (SweepVault.Stage) { return raffle.draw(id).stage; }

    /// The stuck scenario of integration-R3 §8: the first draw's snapshot is empty, so it is not opened and is voided; the void
    /// carries the seed; the carrier gets the same (empty) time. It now opens empty, closes with no winner after the window, the
    /// carry is consumed, the unit is the coin's again, and the coin's next draw pins a new seed and is drawn normally.
    function test_carriedEmptySnapshotNowResolves() public {
        uint256 units0 = raffle.units(coinR);
        uint256 first = _announce();
        uint64 at0 = _at(first);
        _skip(6 hours);
        raffle.voidDraw(first);
        assertEq(raffle.carriedSeed(coinR), first + 1, "the void carries the empty time");

        uint256 id = _announce();
        assertEq(raffle.seedOrigin(id), first);
        assertEq(_at(id), at0, "the same empty snapshot time");
        _openEmpty(id);
        SweepVault.Draw memory d = raffle.draw(id);
        assertEq(uint8(d.stage), uint8(SweepVault.Stage.Open));
        assertEq(d.totalTickets, 0);
        assertEq(d.snapshotAt, at0);
        assertEq(d.root, raffle.leaf(address(0), 0, at0), "the sentinel alone");

        _skip(16 minutes);
        vm.expectEmit(true, true, false, true, address(raffle));
        emit SweepVault.DrawEmpty(id, coinR, at0);
        vm.prank(stranger);
        raffle.requestDraw(id);
        assertEq(uint8(_stage(id)), uint8(SweepVault.Stage.Empty));
        assertEq(raffle.carriedSeed(coinR), 0, "the carry is consumed");
        assertEq(raffle.drawInFlight(coinR), 0);
        assertEq(raffle.units(coinR), units0, "the unit is the coin's again");
        assertEq(raffle.totalReserved(), 0);
        assertEq(raffle.draw(id).round, 0, "no round pinned: nobody to pick");
        _assertVaultBooks(raffle);

        // the coin's next draw: a new seed, a new time, drawn to a winner
        _skip(1 minutes);
        uint256 next = _announce();
        assertEq(raffle.seedOrigin(next), next, "its own seed");
        assertGt(raffle.draw(next).seedRound, raffle.draw(id).seedRound, "a fresh round");
        bytes32[] memory tree = _openHonest(next, 5);
        _skip(16 minutes);
        raffle.requestDraw(next);
        assertEq(uint8(_stage(next)), uint8(SweepVault.Stage.Pinned));
        _skip(2 minutes);
        raffle.resolve(next, abi.encodePacked(keccak256("pin round")));
        raffle.claim(next, holder, 0, 5, Merkle.proof(tree, 0));
        assertEq(wrapper.balanceOf(holder), ONE, "the unit was drawn after all");
        _assertVaultBooks(raffle);
    }

    /// A draw with its own (uncarried) seed whose snapshot turns out empty closes the same way; the next one gets a new seed.
    function test_freshEmptyDrawClosesWithNoWinner() public {
        uint256 id = _announce();
        _openEmpty(id);
        _skip(16 minutes);
        raffle.requestDraw(id);
        assertEq(uint8(_stage(id)), uint8(SweepVault.Stage.Empty));
        uint256 next = _announce();
        assertEq(raffle.seedOrigin(next), next);
        assertTrue(raffle.draw(next).seedRound != raffle.draw(id).seedRound);
    }

    /// The empty open's tree is the sentinel alone: holder leaves beside a zero sentinel, any proof, a wrong time, a zero root.
    function test_emptyOpenIsTheSentinelAlone() public {
        uint256 id = _announce();
        uint64 at = _at(id);
        bytes32[] memory tree = new bytes32[](2);
        tree[0] = raffle.leaf(keeper, 0, 5);                       // a hidden holder beside the zero sentinel
        tree[1] = raffle.leaf(address(0), 0, at);
        bytes memory sig = _sig(id);
        bytes32 root = Merkle.root(tree);
        bytes32[] memory proof = Merkle.proof(tree, 1);
        bytes32 empty = raffle.leaf(address(0), 0, at);
        bytes32 early = raffle.leaf(address(0), 0, at - 1);
        bytes32 counted = raffle.leaf(address(0), 1, at);
        bytes32[] memory none = new bytes32[](0);
        bytes32[] memory one = new bytes32[](1);
        one[0] = bytes32(uint256(1));
        vm.startPrank(keeper);
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.openDraw(id, root, 0, "x", proof, sig);
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.openDraw(id, empty, 0, "x", one, sig);
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.openDraw(id, early, 0, "x", none, sig);
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.openDraw(id, counted, 0, "x", none, sig);
        vm.expectRevert(SweepVault.BadInput.selector);
        raffle.openDraw(id, bytes32(0), 0, "x", none, sig);
        // and a non-empty list cannot pass its sentinel off as empty: count and root must agree
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.openDraw(id, empty, 5, "x", none, sig);
        vm.stopPrank();
        _openEmpty(id);
    }

    /// The close is gated exactly like a pin: not before the window on both clocks, not while the keeper is paused or zero.
    function test_emptyCloseWaitsTheWindowAndThePause() public {
        uint256 id = _announce();
        _openEmpty(id);
        _skip(10 minutes);
        vm.expectRevert(SweepVault.ChallengeWindow.selector);
        raffle.requestDraw(id);
        // the wall clock past the window, the parent chain's not yet
        vm.warp(vm.getBlockTimestamp() + 10 minutes);
        vm.expectRevert(SweepVault.ChallengeWindow.selector);
        raffle.requestDraw(id);
        _skip(10 minutes);
        vm.prank(guardian);
        registry.pauseKeeper();
        _skip(1 days);
        vm.expectRevert(SweepVault.KeeperPaused.selector);
        raffle.requestDraw(id);
        assertEq(uint8(_stage(id)), uint8(SweepVault.Stage.Open), "held by the pause");
        vm.prank(owner);
        registry.unpauseKeeper();
        raffle.requestDraw(id);
        assertEq(uint8(_stage(id)), uint8(SweepVault.Stage.Empty));
    }

    /// A false empty (the snapshot had holders; the keeper says nobody, to throw the time away): the guardian cancels it in the
    /// window, or after it while the keeper is paused (the monitor's bad-root answer: pause, then cancel). The cancel carries the
    /// seed, so the next draw gets the same snapshot time and only the honest list opens it. Repeated, it is only a delay.
    function test_falseEmptyIsCancellableAndCannotReroll() public {
        uint256 first = _announce();
        uint64 at0 = _at(first);
        _openEmpty(first);
        _skip(5 minutes);
        vm.prank(guardian);
        raffle.cancelDraw(first);
        assertEq(raffle.carriedSeed(coinR), first + 1, "the cancel carries the seed");

        uint256 id;
        for (uint256 i = 0; i < 10; ++i) {
            id = _announce();
            assertEq(_at(id), at0, "one snapshot time, however often the keeper claims it empty");
            _openEmpty(id);
            _skip(20 minutes);                                      // past the window: the monitor's pause holds it
            vm.prank(guardian);
            registry.pauseKeeper();
            vm.expectRevert(SweepVault.KeeperPaused.selector);
            raffle.requestDraw(id);
            vm.prank(guardian);
            raffle.cancelDraw(id);
            vm.prank(owner);
            registry.unpauseKeeper();
            assertEq(raffle.carriedSeed(coinR), first + 1);
        }
        id = _announce();
        _openHonest(id, 7);
        assertEq(raffle.draw(id).snapshotAt, at0, "the honest list at the same time");
        _skip(16 minutes);
        raffle.requestDraw(id);
        assertEq(uint8(_stage(id)), uint8(SweepVault.Stage.Pinned));
        assertEq(raffle.carriedSeed(coinR), 0);
        _assertVaultBooks(raffle);
    }

    /// The owner's cancel of an empty draw carries too, and a cancel of a true empty does not recreate the stuck state: the next
    /// carrier opens empty again and closes.
    function test_cancelOfATrueEmptyIsOnlyADelay() public {
        uint256 id = _announce();
        uint64 at0 = _at(id);
        _openEmpty(id);
        vm.prank(owner);
        raffle.cancelDraw(id);
        uint256 id2 = _announce();
        assertEq(_at(id2), at0);
        _openEmpty(id2);
        _skip(16 minutes);
        raffle.requestDraw(id2);
        assertEq(uint8(_stage(id2)), uint8(SweepVault.Stage.Empty));
        assertEq(raffle.carriedSeed(coinR), 0);
    }

    /// `Empty` is final: nothing else applies to it.
    function test_emptyIsFinal() public {
        uint256 id = _announce();
        _openEmpty(id);
        _skip(16 minutes);
        raffle.requestDraw(id);
        vm.expectRevert(SweepVault.WrongStage.selector);
        raffle.requestDraw(id);
        vm.expectRevert(SweepVault.WrongStage.selector);
        raffle.resolve(id, abi.encodePacked(bytes32(uint256(1))));
        vm.expectRevert(SweepVault.WrongStage.selector);
        raffle.claim(id, holder, 0, 1, new bytes32[](0));
        _skip(30 days);
        vm.expectRevert(SweepVault.WrongStage.selector);
        raffle.voidDraw(id);
        vm.prank(owner);
        vm.expectRevert(SweepVault.WrongStage.selector);
        raffle.cancelDraw(id);
        bytes32 root = raffle.leaf(address(0), 0, _at(id));
        bytes memory sig = _sig(id);
        vm.prank(keeper);
        vm.expectRevert(SweepVault.WrongStage.selector);
        raffle.openDraw(id, root, 0, "x", new bytes32[](0), sig);
    }

    /// The stage numbers the services decode: `Empty` is appended, the others keep theirs.
    function test_stageNumbers() public pure {
        assertEq(uint8(SweepVault.Stage.Announced), 7);
        assertEq(uint8(SweepVault.Stage.Empty), 8);
        assertEq(uint8(SweepVault.Stage.Voided), 6);
    }
}
