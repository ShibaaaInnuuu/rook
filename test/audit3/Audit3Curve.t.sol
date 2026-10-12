// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {AuditABase} from "../audit/A/AuditABase.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// R3 check A3-L1, now a regression (docs/audit/2026-09-26/review3/fixes-R4-contracts.md §2): the N-3 hold used to end the instant
/// the Safe unpaused, while the floor was still stale (nobody can post during a pause), so `fill()` could not run and anyone calling
/// `openRefunds()` in that gap bound a graduation held through the pause to refund. The hold now lasts `RESUME_GRACE` (6 h, both
/// clocks) past the unpause, whatever the floor does meanwhile, and never past the curve's escape hatch.
contract Audit3Curve is AuditABase {
    address guardian = makeAddr("guardian");

    function setUp() public override {
        super.setUp();
        vm.prank(owner);
        registry.setGuardian(guardian);
    }

    /// A curve that closed on a stale floor (its grad Queued), then a pause that outlasts its window.
    function _heldThroughAPause() internal returns (FrogCurve c, CurveToken tok) {
        (c, tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(module.FLOOR_TTL());                             // stale at the close: the grad queues
        _fillCurve(c);
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Queued));
        vm.prank(guardian);
        registry.pauseKeeper();
        _skipBoth(3 days + 1 hours);                              // the window passes during the pause: held
        vm.prank(mallory);
        vm.expectRevert(FrogCurve.TooEarly.selector);
        c.openRefunds();
    }

    /// The reviewer's race: in the block of the unpause (the floor still stale, `fill` impossible) mallory's `openRefunds` is now
    /// refused; the keeper posts, anyone fills, the curve graduates.
    function test_audit3_unpauseRace_refundNoLongerBeatsTheFill() public {
        (FrogCurve c, CurveToken tok) = _heldThroughAPause();
        vm.prank(owner);
        registry.unpauseKeeper();
        assertEq(registry.keeperResumedAt(), uint64(block.timestamp), "the resume is stamped");
        assertEq(registry.keeperResumedAtBlock(), uint64(block.number));
        assertFalse(module.fresh(), "the floor is stale until the keeper's next post");
        assertTrue(module.holdsRefunds(), "the hold outlives the unpause");
        vm.expectRevert(FloorModule.NoRoom.selector);
        module.fill(address(tok));

        vm.prank(mallory);
        vm.expectRevert(FrogCurve.TooEarly.selector);
        c.openRefunds();                                           // the back-run of the unpause

        _skipBoth(1 minutes);
        _postCap(curveRaffle, CAP);
        _floor(CAP);                                               // the keeper's first post
        assertTrue(module.holdsRefunds(), "a fresh floor does not end the grace: no back-run of the first post either");
        vm.prank(mallory);
        vm.expectRevert(FrogCurve.TooEarly.selector);
        c.openRefunds();
        module.fill(address(tok));                                 // anyone
        assertTrue(c.canGraduate());
        c.graduate();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated), "the hold bought the graduation");
    }

    /// The grace is bounded: 6 hours on both clocks after the unpause. A curve nobody fills (no post) then refunds as before.
    function test_audit3_theGraceEndsAfterSixHoursOnBothClocks() public {
        (FrogCurve c,) = _heldThroughAPause();
        vm.prank(owner);
        registry.unpauseKeeper();
        vm.warp(vm.getBlockTimestamp() + module.RESUME_GRACE() + 1);      // the sequencer's clock alone
        assertTrue(module.holdsRefunds(), "held until the parent chain's clock has passed too");
        vm.prank(mallory);
        vm.expectRevert(FrogCurve.TooEarly.selector);
        c.openRefunds();
        vm.roll(vm.getBlockNumber() + module.RESUME_GRACE() / PBS + 1);
        assertFalse(module.holdsRefunds(), "the grace is over");
        vm.prank(mallory);
        c.openRefunds();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Refunding));
    }

    /// The escape hatch still bounds every hold: an unpause just before it gives no extra time beyond it.
    function test_audit3_theHatchStillBoundsTheGrace() public {
        (FrogCurve c,) = _heldThroughAPause();
        _skipBoth(4 days - 2 hours);                               // just before window + STUCK_GRACE
        vm.prank(owner);
        registry.unpauseKeeper();
        assertTrue(module.holdsRefunds());
        _skipBoth(2 hours + 1);                                    // past the hatch, inside the grace
        assertTrue(module.holdsRefunds());
        vm.prank(mallory);
        c.openRefunds();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Refunding), "the hatch wins over the grace");
    }

    /// No pause, no grace: an unpause of a keeper that is not paused stamps nothing, and an unfilled curve refunds at its window.
    function test_audit3_noPauseNoGrace() public {
        vm.prank(owner);
        registry.unpauseKeeper();
        assertEq(registry.keeperResumedAt(), 0, "nothing resumed");
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(module.FLOOR_TTL());
        _fillCurve(c);
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Queued));
        assertFalse(module.holdsRefunds());
        _skipBoth(3 days + 1 hours);
        c.openRefunds();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Refunding));
    }

    /// N-3 is not reopened: the close gate still refuses while paused, and a curve closing in the grace (the floor stale after the
    /// unpause) queues and is filled at the keeper's next post, like any close on a stale floor.
    function test_audit3_N3StillHoldsAroundTheGrace() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        _postCap(curveRaffle, CAP);
        _floor(CAP);
        vm.prank(guardian);
        registry.pauseKeeper();
        assertFalse(c.canClose(), "no close while paused");
        _skipBoth(module.FLOOR_TTL());
        vm.prank(owner);
        registry.unpauseKeeper();
        assertTrue(c.canClose(), "after the unpause a stale floor does not gate the close (A-03)");
        _fillCurve(c);
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Queued));
        _postCap(curveRaffle, CAP);
        _floor(CAP);
        module.fill(address(tok));
        c.graduate();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated));
    }
}
