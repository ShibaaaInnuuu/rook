// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {AuditABase} from "../audit/A/AuditABase.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// Second review (R2) N-3, now a regression (docs/audit/2026-09-26/review2/fixes-R3-contracts.md §2): the close gate honours the
/// keeper pause, so a curve does not close into a graduation nobody can fill; and a graduation already queued when a pause starts
/// waits for the unpause (up to the escape hatch, never longer) instead of being bound to refund after its window.
contract R2Curve is AuditABase {
    address guardian = makeAddr("guardian");

    function setUp() public override {
        super.setUp();
        vm.prank(owner);
        registry.setGuardian(guardian);
    }

    function _refresh() internal {
        _postCap(curveRaffle, CAP);
        _floor(CAP);
    }

    /// The PoC's scenario: the guardian pauses while a curve is trading. The closing buy is now refused (`QueueFull`) and the curve
    /// keeps trading; after the unpause it closes and is filled at once.
    function test_N3_aCloseDuringAPauseIsRefusedAndTheCurveKeepsTrading() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        _refresh();

        vm.prank(guardian);
        registry.pauseKeeper();
        assertFalse(module.hasRoomFor(c.itemBudget()), "no room while paused");
        assertFalse(c.canClose(), "the close gate says no while paused");
        uint256 amount = c.raise() * 2;
        vm.deal(buyer, amount);
        vm.prank(buyer);
        vm.expectRevert(FrogCurve.QueueFull.selector);
        c.buy{value: amount}(0);
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Trading), "keeps trading");

        _skipBoth(3 days + 1 hours);                              // longer than the window the PoC's refund needed
        vm.prank(owner);
        registry.unpauseKeeper();
        _refresh();
        assertTrue(c.canClose());
        _fillCurve(c);
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduating));
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Filled), "filled at the close");
        assertTrue(c.canGraduate());
        c.graduate();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated));
    }

    /// A curve that closed on a stale floor (Queued), then a pause: its window passes during the pause, and refunds wait; the
    /// unpause and a post let anyone fill it, and it graduates.
    function test_N3_aGradQueuedBeforeAPauseWaitsForTheUnpause() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(module.FLOOR_TTL());                             // the floor is stale: the close queues, it does not fill
        assertFalse(module.fresh());
        _fillCurve(c);
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Queued));

        vm.prank(guardian);
        registry.pauseKeeper();
        assertTrue(module.holdsRefunds());
        (uint64 at,) = c.refundsOpenAt();
        assertEq(at, c.gradAt() + c.window() + c.STUCK_GRACE(), "held to the hatch while paused");
        _skipBoth(3 days + 1 hours);                              // past the window
        vm.prank(mallory);
        vm.expectRevert(FrogCurve.TooEarly.selector);
        c.openRefunds();

        vm.prank(owner);
        registry.unpauseKeeper();
        _refresh();
        module.fill(address(tok));
        assertTrue(c.canGraduate());
        c.graduate();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated));
    }

    /// The hold is bounded: a pause that is never lifted lets refunds open at the escape hatch's time, not later.
    function test_N3_theHoldNeverOutlastsTheHatch() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(module.FLOOR_TTL());
        _fillCurve(c);
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Queued));
        vm.prank(guardian);
        registry.pauseKeeper();
        _skipBoth(3 days + 4 days + 1 hours);                     // window + STUCK_GRACE
        vm.prank(mallory);
        c.openRefunds();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Refunding));
        assertEq(c.refundEthPool(), c.reserveEth());
    }

    /// Without a pause nothing changes: an unfilled curve opens refunds at its window, as before.
    function test_N3_noPauseNoHold() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(module.FLOOR_TTL());
        _fillCurve(c);
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Queued));
        assertFalse(module.holdsRefunds());
        _skipBoth(3 days + 1 hours);
        c.openRefunds();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Refunding));
    }
}
