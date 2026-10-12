// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase, MockVenue} from "../../utils/SweepBase.sol";
import {Merkle} from "../../utils/Merkle.sol";
import {Clock} from "../../utils/Clock.sol";
import {SweepVault} from "../../../src/SweepVault.sol";
import {Wrapper} from "../../../src/Wrapper.sol";

/// Audit C regressions for the sweep vault and the draws (docs/audit/2026-09-26/C-treasury-roles-deploy.md, E-services-workflow.md
/// E-02/E-03; the PoCs are in docs/audit/2026-09-26/poc/C). Each replays a PoC's attack against the fixed contracts and asserts it
/// now FAILS. Design: docs/audit/2026-09-26/fixes-B-C-keeper-draws.md. (C-10, the desk, is another branch's.)
contract C_SweepVaultRegression is SweepBase {
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address carol = makeAddr("carol (the grinder)");
    address guardian = makeAddr("guardian");
    bytes32[] leaves;
    uint256 constant TOTAL = 10; // alice [0,5), bob [5,8), carol [8,10)

    function setUp() public override {
        super.setUp();
        _stockVenue(40);
        leaves.push(raffle.leaf(alice, 0, 5));
        leaves.push(raffle.leaf(bob, 5, 8));
        leaves.push(raffle.leaf(carol, 8, 10));
    }

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    function _fresh() internal returns (SweepVault v) {
        v = new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(venue), launcher, desk, PBS);
    }

    // ---- C-01 ------------------------------------------------------------------------------------------------------------------

    /// The PoC: a ceiling proposed at deploy was never accepted, and a stale re-post at 1000x then spent a coin's whole credit.
    /// Now: no cap without a ceiling, the owner's first ceiling is active at once, and a stale re-post rises one step at most.
    function test_C01_theCeilingIsActiveFromTheOwnersStep_andAStaleRepostCannotJump() public {
        SweepVault v = _fresh();
        vm.prank(keeper);
        vm.expectRevert(SweepVault.NoCeiling.selector);
        v.postCap(PRICE);                                     // before the owner's step: no cap at all
        vm.prank(owner);
        v.proposeCapCeiling(2 * PRICE);                       // go-live step 7
        assertEq(v.capCeiling(), 2 * PRICE, "active at once: no accept step to forget");
        vm.prank(keeper);
        v.postCap(PRICE);
        Clock.skip(8 days);
        assertFalse(v.capFresh());
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(SweepVault.CapStep.selector, 1000 * PRICE, 0, PRICE + PRICE / 5));
        v.postCap(1000 * PRICE);                              // the compromised keeper's stale re-post
        vm.prank(keeper);
        v.postCap(PRICE + PRICE / 5);
        // fresh steps of +20%, ten minutes apart, climb until the next would pass the ceiling: refused there
        uint256 next = v.cap() + v.cap() / 5;
        while (next <= 2 * PRICE) {
            _skip(10 minutes);
            vm.prank(keeper);
            v.postCap(next);
            next = v.cap() + v.cap() / 5;
        }
        _skip(10 minutes);
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(SweepVault.OverCeiling.selector, next, 2 * PRICE));
        v.postCap(next);
        assertLe(v.cap(), 2 * PRICE);
    }

    /// C-01 (b): the fresh step compounds, but only up to the ceiling; and the Safe can lower the ceiling at once.
    function test_C01b_theFreshCompoundingStopsAtTheCeiling() public {
        vm.prank(owner);
        raffle.proposeCapCeiling(2 * PRICE);                  // lowering (from the suite's high one) is at once
        _postCap(raffle, PRICE);
        uint256 c = PRICE;
        for (uint256 i = 0; i < 6; ++i) {
            _skip(10 minutes);
            c = c + (c * 2_000) / 10_000;
            if (c > 2 * PRICE) {
                vm.prank(keeper);
                vm.expectRevert();
                raffle.postCap(c);
                break;
            }
            vm.prank(keeper);
            raffle.postCap(c);
        }
        assertLe(raffle.cap(), 2 * PRICE, "never above the ceiling (the PoC reached 2.99x)");
    }

    // ---- C-02 / E-03: no re-roll ------------------------------------------------------------------------------------------------

    function _published(uint64 round) internal pure returns (bytes32) {
        return keccak256(abi.encode("drand evmnet", round));
    }

    /// The PoC: a 20% holder declined each losing round and re-rolled with `redraw` a day later until they won. Now the pinned
    /// round is final: there is no redraw, anyone else resolves it to the same winner, and the grinder cannot stop them.
    function test_C02_theGrinderCannotReroll() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        (uint256 id,) = _openDraw(raffle, coinR, leaves, TOTAL);
        _skip(15 minutes);
        vm.prank(carol);
        raffle.requestDraw(id);
        uint64 round = raffle.draw(id).round;
        uint256 expected = uint256(keccak256(abi.encode(_published(round), address(raffle), id))) % TOTAL;
        _skip(beacon.timeOf(round) + 1 - vm.getBlockTimestamp());
        _skip(24 hours);                                      // carol read the outcome and waited a day, as in the PoC
        vm.prank(carol);
        (bool ok,) = address(raffle).call(abi.encodeWithSignature("redraw(uint256)", id));
        assertFalse(ok, "there is no redraw");
        assertEq(raffle.draw(id).round, round, "the pinned round is final");
        vm.prank(bob);                                        // anyone resolves, whenever: the same winner
        raffle.resolve(id, abi.encodePacked(_published(round)));
        assertEq(raffle.draw(id).winningTicket, expected);
    }

    // ---- C-03 ------------------------------------------------------------------------------------------------------------------

    /// The PoC: the honest root with a truncated `totalTickets` handed every draw to the lowest range. Now the root must commit
    /// the count (the sentinel leaf), so the honest root with a truncated count is refused.
    function test_C03_aTruncatedTotalIsRefused() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        vm.prank(keeper);
        uint256 id = raffle.announceDraw(coinR);
        bytes32 seed = keccak256("seed");
        uint64 at = raffle.snapshotTimeOf(id, seed);
        bytes32[] memory tree = new bytes32[](4);
        (tree[0], tree[1], tree[2]) = (leaves[0], leaves[1], leaves[2]);
        tree[3] = raffle.leaf(address(0), TOTAL, at);          // the honest published tree
        bytes32 honest = Merkle.root(tree);
        vm.prank(keeper);
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.openDraw(id, honest, 5, "https://keeper/snap", Merkle.proof(tree, 3), abi.encodePacked(seed));
        // a tree whose sentinel says 5 is a DIFFERENT root: any recomputation of the published list catches it
        tree[3] = raffle.leaf(address(0), 5, at);
        assertTrue(Merkle.root(tree) != honest, "the truncated count changes the root");
    }

    // ---- C-04 ------------------------------------------------------------------------------------------------------------------

    /// The PoC's economics relied on a snapshot at a published block. Now the snapshot time is a moment of the day before the
    /// announcement, picked by a drand round that did not exist when the draw was announced; the keeper does not choose it.
    function testFuzz_C04_theSnapshotTimeIsUnknownAtTheAnnouncementAndNotTheKeepers(bytes32 seed) public {
        vm.assume(seed != bytes32(0));
        _sweepWhole(raffle, coinR, routerR, 1);
        vm.prank(keeper);
        uint256 id = raffle.announceDraw(coinR);
        SweepVault.Draw memory d = raffle.draw(id);
        assertEq(beacon.randomnessOf(d.seedRound), bytes32(0), "the seed round does not exist yet");
        assertGe(beacon.timeOf(d.seedRound), vm.getBlockTimestamp() + 57, "about a minute ahead");
        uint64 at = raffle.snapshotTimeOf(id, seed);
        assertLe(at, d.announcedAt);
        assertGt(uint256(at) + raffle.SNAPSHOT_WINDOW(), d.announcedAt);
        // the keeper must open at exactly that time: a sentinel with any other time is refused
        bytes32[] memory tree = new bytes32[](2);
        tree[0] = raffle.leaf(alice, 0, TOTAL);
        tree[1] = raffle.leaf(address(0), TOTAL, at + 1);
        vm.prank(keeper);
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.openDraw(id, Merkle.root(tree), TOTAL, "x", Merkle.proof(tree, 1), abi.encodePacked(seed));
    }

    // ---- E-02: forged draws in bulk, and the incident switch -------------------------------------------------------------------

    /// The E-02 scenario: a stolen keeper opens forged draws over every unit in one block. Now: one draw per coin before its pin;
    /// the guardian pauses the keeper and cancels it; while paused nothing can be pinned; the unit goes back to the coin.
    function test_E02_oneForgedDrawPerCoin_theGuardianPausesAndCancels() public {
        vm.prank(owner);
        registry.setGuardian(guardian);
        _sweepWhole(raffle, coinR, routerR, 3);
        bytes32[] memory forged = new bytes32[](1);
        forged[0] = raffle.leaf(keeper, 0, 1);               // the thief holds every ticket
        (uint256 id,) = _openDraw(raffle, coinR, forged, 1);
        vm.prank(keeper);
        vm.expectRevert(SweepVault.DrawInFlight.selector);
        raffle.announceDraw(coinR);                           // not a second one
        vm.prank(guardian);
        registry.pauseKeeper();
        _skip(15 minutes);
        vm.expectRevert(SweepVault.KeeperPaused.selector);
        raffle.requestDraw(id);                               // nothing proceeds during the incident
        vm.prank(keeper);
        vm.expectRevert(SweepVault.NotKeeper.selector);
        raffle.announceDraw(coinR2);
        assertFalse(raffle.capFresh(), "and no sweep at the thief's cap");
        vm.prank(guardian);
        raffle.cancelDraw(id);                                // past the window, before the pin
        assertEq(raffle.units(coinR), 3 * ONE, "the unit went back to the coin");
    }

    // ---- E-03: a stuck redemption ----------------------------------------------------------------------------------------------

    /// The E-03 scenario: nothing resolves a ticket; it sat Pinned for ever with no refund, blocking every later one. Now it is
    /// refunded a week after its pin (anyone), and resolution moves past it.
    function test_E03_aStuckPinnedTicketIsRefunded_andNoLongerBlocksTheQueue() public {
        _deposit(alice);
        _deposit(bob);
        vm.prank(alice);
        uint256 a = wrapper.redeem(TAPROOT);
        vm.prank(bob);
        uint256 b = wrapper.redeem(TAPROOT);
        _skip(7 days);
        vm.expectRevert(abi.encodeWithSelector(Wrapper.OutOfOrder.selector, a));
        wrapper.resolve(b, abi.encodePacked(bytes32(uint256(3))));   // b waits behind a
        vm.prank(stranger);
        wrapper.refund(a);
        assertEq(wrapper.balanceOf(alice), ONE);
        wrapper.resolve(b, abi.encodePacked(bytes32(uint256(3))));
        assertEq(uint8(wrapper.ticket(b).stage), uint8(Wrapper.Stage.Resolved));
        _assertLedger();
    }

    /// Canister M-1 (docs/audit/2026-09-26/fixes-D-canister.md): a delivery broadcast before `deliverBy` can confirm late. The
    /// refund waits `REFUND_GRACE` past the deadline, so the custodian can still record it and the redeemer never keeps both the
    /// frog and the token.
    function test_M1_aLateDeliveryIsRecordedBeforeAnyRefund() public {
        _deposit(alice);
        vm.prank(alice);
        uint256 id = wrapper.redeem(TAPROOT);
        _skip(2 minutes);
        _resolveTicket(id, keccak256("r"));
        _skip(7 days + 1 hours);                              // past deliverBy: the delivery has not confirmed yet
        vm.expectRevert(Wrapper.TooEarly.selector);
        wrapper.refund(id);
        _skip(2 days);                                        // it confirms late, inside the grace
        vm.prank(custodian);
        wrapper.recordDelivery(id, keccak256("the late delivery"));
        _skip(2 days);
        vm.expectRevert(Wrapper.WrongStage.selector);
        wrapper.refund(id);                                   // no refund on top of the frog
        assertEq(wrapper.balanceOf(alice), 0);
        _assertLedger();
    }
}
