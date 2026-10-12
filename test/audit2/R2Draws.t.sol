// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase} from "../utils/SweepBase.sol";
import {Merkle} from "../utils/Merkle.sol";
import {SweepVault} from "../../src/SweepVault.sol";

/// Second review (R2) N-2, now a regression (docs/audit/2026-09-26/review2/fixes-R3-contracts.md §3): a draw voided or cancelled
/// before its pin hands its seed round and snapshot window to the coin's next draw, so neither the keeper (by not opening) nor the
/// owner or guardian (by cancelling an Open draw) can discard a snapshot time after seeing it. A pin consumes the seed.
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

    /// A deterministic stand-in for drand: the round's randomness is fixed, nobody chooses it (one valid signature per round).
    function _drand(uint64 round) internal pure returns (bytes32) { return keccak256(abi.encode("drand", round)); }

    function _open(uint256 id, uint64 at) internal {
        bytes32[] memory tree = new bytes32[](2);
        tree[0] = raffle.leaf(stranger, 0, 10);
        tree[1] = raffle.leaf(address(0), 10, at);
        bytes memory sig = abi.encodePacked(_drand(raffle.draw(id).seedRound));
        bytes32 root = Merkle.root(tree);
        bytes32[] memory proof = Merkle.proof(tree, 1);
        vm.prank(keeper);
        raffle.openDraw(id, root, 10, "ipfs://x", proof, sig);
    }

    /// The PoC's loop: the keeper announces, reads the snapshot time once the seed is public, declines to open, and anyone voids
    /// after OPEN_TTL. Every re-announcement now carries the first seed round and gets the same snapshot time: 30 tries, one time.
    function test_N2a_notOpeningNoLongerRerollsTheSnapshotTime() public {
        vm.prank(keeper);
        uint256 first = raffle.announceDraw(coinR);
        uint64 round0 = raffle.draw(first).seedRound;
        _skip(2 minutes);
        uint64 at0 = raffle.snapshotTimeOf(first, _drand(round0));
        uint256 id = first;
        for (uint256 i = 0; i < 30; ++i) {
            _skip(6 hours);
            vm.prank(stranger);
            raffle.voidDraw(id);
            assertEq(raffle.carriedSeed(coinR), first + 1, "the void leaves the seed to the coin");
            vm.prank(keeper);
            id = raffle.announceDraw(coinR);
            assertEq(raffle.draw(id).seedRound, round0, "the same seed round");
            assertEq(raffle.seedOrigin(id), first);
            _skip(2 minutes);
            assertEq(raffle.snapshotTimeOf(id, _drand(round0)), at0, "the same snapshot time, however often it is discarded");
        }
        _open(id, at0);
        assertEq(raffle.draw(id).snapshotAt, at0);
    }

    /// The guardian (or the Safe) cancels an Open draw after its snapshot time and list are public: the next draw carries the seed
    /// and must open the same snapshot time (an honest list, if the first was forged).
    function test_N2b_aCancelAfterTheSnapshotIsPublicPicksTheSameSnapshot() public {
        vm.prank(keeper);
        uint256 id = raffle.announceDraw(coinR);
        _skip(2 minutes);
        uint64 first = raffle.snapshotTimeOf(id, _drand(raffle.draw(id).seedRound));
        _open(id, first);
        _skip(20 minutes);                                        // past the challenge window, still unpinned
        vm.prank(guardian);
        raffle.cancelDraw(id);
        assertEq(uint8(raffle.draw(id).stage), uint8(SweepVault.Stage.Cancelled));

        vm.prank(keeper);
        uint256 id2 = raffle.announceDraw(coinR);
        assertEq(raffle.draw(id2).seedRound, raffle.draw(id).seedRound);
        _open(id2, first);
        assertEq(raffle.draw(id2).snapshotAt, first, "the same snapshot time");

        // the owner's cancel carries too, and an Announced draw's
        _skip(1 minutes);
        vm.prank(owner);
        raffle.cancelDraw(id2);
        vm.prank(keeper);
        uint256 id3 = raffle.announceDraw(coinR);
        assertEq(raffle.seedOrigin(id3), id, "the origin, not the carrier");
        vm.prank(guardian);
        raffle.cancelDraw(id3);
        vm.prank(keeper);
        uint256 id4 = raffle.announceDraw(coinR);
        _open(id4, first);
        assertEq(raffle.draw(id4).snapshotAt, first);
    }

    /// A pin consumes the seed: the coin's next draw gets a new round (and, by drand, a new time).
    function test_N2_aPinConsumesTheSeed() public {
        vm.prank(keeper);
        uint256 id = raffle.announceDraw(coinR);
        _skip(2 minutes);
        _open(id, raffle.snapshotTimeOf(id, _drand(raffle.draw(id).seedRound)));
        vm.prank(owner);
        raffle.cancelDraw(id);
        vm.prank(keeper);
        uint256 id2 = raffle.announceDraw(coinR);
        _skip(2 minutes);
        _open(id2, raffle.snapshotTimeOf(id2, _drand(raffle.draw(id2).seedRound)));
        _skip(16 minutes);
        raffle.requestDraw(id2);
        assertEq(raffle.carriedSeed(coinR), 0, "used by the pin");
        _skip(1 minutes);
        vm.prank(keeper);
        uint256 id3 = raffle.announceDraw(coinR);
        assertEq(raffle.seedOrigin(id3), id3);
        assertGt(raffle.draw(id3).seedRound, raffle.draw(id2).seedRound, "a fresh round");
    }

    /// The monitor's safety net (r3-services) pauses the keeper ahead of the pin of a draw whose root it could not verify: nobody
    /// can pin while paused, however long past the challenge window, and after the unpause (or a cancel, which carries the seed)
    /// the draw proceeds.
    function test_requestDrawRefusesWhileTheKeeperIsPaused() public {
        vm.prank(keeper);
        uint256 id = raffle.announceDraw(coinR);
        _skip(2 minutes);
        _open(id, raffle.snapshotTimeOf(id, _drand(raffle.draw(id).seedRound)));
        _skip(9 minutes);
        vm.prank(guardian);
        registry.pauseKeeper();
        _skip(1 days);
        vm.prank(stranger);
        vm.expectRevert(SweepVault.KeeperPaused.selector);
        raffle.requestDraw(id);
        assertEq(uint8(raffle.draw(id).stage), uint8(SweepVault.Stage.Open), "still unpinned");
        vm.prank(owner);
        registry.unpauseKeeper();
        vm.prank(stranger);
        raffle.requestDraw(id);
        assertEq(uint8(raffle.draw(id).stage), uint8(SweepVault.Stage.Pinned));
    }

    /// Carries are per coin: another coin's void does not hand its seed across.
    function test_N2_carriesArePerCoin() public {
        vm.prank(keeper);
        uint256 id = raffle.announceDraw(coinR);
        _skip(6 hours + 1 minutes);
        raffle.voidDraw(id);
        assertEq(raffle.carriedSeed(coinR), id + 1);
        assertEq(raffle.carriedSeed(address(0xBEEF)), 0);
    }
}
