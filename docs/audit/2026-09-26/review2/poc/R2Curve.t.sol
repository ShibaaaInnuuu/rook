// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {AuditABase} from "../audit/A/AuditABase.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// Second review (R2). N-3: the A-03 close gate (`hasRoomFor`) ignores freshness and the keeper pause, while `_tryFill` requires
/// both. A curve that closes during a keeper incident (the runbook's pause lasts the 7-day rotation) closes into a Queued grad,
/// cannot be filled while paused, and after its 3-day window anyone opens refunds: the average-price refund with fees kept that
/// A-03's gate was meant to prevent.
contract R2Curve is AuditABase {
    address guardian = makeAddr("guardian");

    function test_N3_aCurveThatClosesDuringAKeeperPauseIsForcedIntoRefunds() public {
        vm.prank(owner);
        registry.setGuardian(guardian);
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        _postCap(curveRaffle, CAP);
        _floor(CAP);

        vm.prank(guardian);
        registry.pauseKeeper();                                   // an incident: pause, then rotate (7 days), then unpause
        assertTrue(c.canClose(), "the close gate still says yes while paused");
        _fillCurve(c);                                            // the closing buy goes through
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduating));
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Queued), "not filled: keeper paused");
        vm.expectRevert(FloorModule.NoRoom.selector);
        module.fill(address(tok));                                // nobody can fill it while paused
        assertFalse(c.canGraduate());

        _skipBoth(3 days + 1 hours);                              // the curve's window; the rotation is still 4 days away
        uint256 fees = c.creatorOwed() + c.platformOwed() + c.feesForVault() + c.reserveOwed() + c.liquidityOwed();
        vm.prank(mallory);
        c.openRefunds();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Refunding), "bound to refund");
        emit log_named_decimal_uint("refund pool (ETH)", c.refundEthPool(), 18);
        emit log_named_decimal_uint("raise", c.raise(), 18);
        emit log_named_decimal_uint("fees kept (not refunded)", fees, 18);
        assertGt(fees, 0, "trade and launch fees are not refunded");
    }
}
