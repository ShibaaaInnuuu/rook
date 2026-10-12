// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {AuditABase} from "../audit/A/AuditABase.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// R3 check: the N-3 hold (`holdsRefunds`) ends the instant the Safe unpauses, but the floor is then stale (a pause always outlasts
/// FLOOR_TTL, and the keeper cannot post while paused), so `fill()` cannot run until the keeper's first post. Anyone calling
/// `openRefunds()` in that gap binds a curve whose grad was held through the pause to refund — the outcome N-3 meant to prevent.
contract Audit3Curve is AuditABase {
    address guardian = makeAddr("guardian");

    function setUp() public override {
        super.setUp();
        vm.prank(owner);
        registry.setGuardian(guardian);
    }

    function test_audit3_unpauseRace_refundBeatsFill() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(2 hours);                                        // stale at the close: the grad queues
        _fillCurve(c);
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Queued));

        vm.prank(guardian);
        registry.pauseKeeper();
        _skipBoth(3 days + 1 hours);                              // the window passes during the pause: held
        vm.prank(mallory);
        vm.expectRevert(FrogCurve.TooEarly.selector);
        c.openRefunds();

        // the Safe unpauses; the keeper has not posted yet (it could not while paused), so the floor is stale
        vm.prank(owner);
        registry.unpauseKeeper();
        assertFalse(module.holdsRefunds(), "the hold ends at the unpause");
        assertFalse(module.fresh(), "but the floor is stale until the keeper's next post");
        vm.expectRevert(FloorModule.NoRoom.selector);
        module.fill(address(tok));                                 // the runner cannot fill yet

        // mallory, in the same block as the unpause (or any block before the keeper's post + fill), opens refunds
        vm.prank(mallory);
        c.openRefunds();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Refunding), "bound to refund: the hold bought nothing");
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Cancelled), "the queued grad is cancelled");
    }
}
