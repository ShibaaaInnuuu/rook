// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase} from "../utils/SweepBase.sol";
import {Wrapper} from "../../src/Wrapper.sol";

/// The canister review's N-2 on the EVM side (docs/audit/2026-09-26/review2/fixes-R3-contracts.md §4): a redeemer who pins
/// custody's delivery below the market (so neither the bump nor the conflict-out can replace it) used to refund at
/// `deliverBy + REFUND_GRACE`, then confirm the delivery and keep both the frog and the token. A resolved ticket is now refunded
/// only once the custodian attests its delivery can no longer confirm, or `LATE_DELIVERY_WINDOW` after `deliverBy`; until then a
/// late delivery is recorded. The race, both ways.
contract R3LateDelivery is SweepBase {
    address alice = makeAddr("alice");

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    function _resolvedTicket() internal returns (uint256 id) {
        _deposit(alice);
        _deposit(alice);
        vm.prank(alice);
        id = wrapper.redeem(TAPROOT);
        _skip(2 minutes);
        _resolveTicket(id, keccak256("r"));
    }

    /// The pin attack: at deliverBy + REFUND_GRACE the redeemer's refund is refused; the pinned delivery confirms on day 10 and is
    /// recorded; the refund never happens, the unit stays backed.
    function test_N2c_aPinnedDeliveryConfirmingLateIsRecordedAndNeverRefunded() public {
        uint256 id = _resolvedTicket();
        uint256 supply = wrapper.totalSupply();
        _skip(7 days + 72 hours + 1 minutes);                        // past deliverBy + REFUND_GRACE
        vm.prank(alice);
        vm.expectRevert(Wrapper.NotUndeliverable.selector);
        wrapper.refund(id);
        _skip(10 days);                                            // the pinned delivery finally mines
        vm.prank(custodian);
        wrapper.recordDelivery(id, keccak256("the pinned delivery"));
        _skip(30 days);
        vm.prank(alice);
        vm.expectRevert(Wrapper.WrongStage.selector);
        wrapper.refund(id);
        assertEq(wrapper.totalSupply(), supply, "no token minted back");
        assertEq(wrapper.balanceOf(alice), ONE, "alice has the frog and only the token she did not redeem");
        _assertLedger();
    }

    /// Honest non-delivery: the custodian's conflict-out is final, it marks the ticket, and the refund comes at deliverBy + 72 h
    /// as before. A delivery recorded after that refund is refused (the custodian would never send one).
    function test_N2c_anAttestedUndeliverableTicketRefundsAtTheGrace() public {
        uint256 id = _resolvedTicket();
        _skip(7 days + 1 hours);
        vm.prank(custodian);
        wrapper.markUndeliverable(id);
        assertTrue(wrapper.undeliverable(id));
        vm.expectRevert(Wrapper.TooEarly.selector);
        wrapper.refund(id);                                        // still the grace first
        _skip(72 hours);
        wrapper.refund(id);
        assertEq(wrapper.balanceOf(alice), 2 * ONE);
        vm.prank(custodian);
        vm.expectRevert(Wrapper.WrongStage.selector);
        wrapper.recordDelivery(id, keccak256("too late"));
        _assertLedger();
    }

    /// A delivery recorded first, then an attestation attempt: nothing to mark, no refund.
    function test_N2c_deliveryFirstLeavesNothingToRefund() public {
        uint256 id = _resolvedTicket();
        _skip(8 days);
        vm.prank(custodian);
        wrapper.recordDelivery(id, keccak256("delivered"));
        vm.prank(custodian);
        vm.expectRevert(Wrapper.WrongStage.selector);
        wrapper.markUndeliverable(id);
        _skip(40 days);
        vm.expectRevert(Wrapper.WrongStage.selector);
        wrapper.refund(id);
        _assertLedger();
    }

    /// A custodian that is gone: no attestation ever, the refund comes LATE_DELIVERY_WINDOW after deliverBy, on both clocks.
    function test_N2c_aDeadCustodianStillRefundsAfterTheLateWindow() public {
        uint256 id = _resolvedTicket();
        _skip(7 days + 30 days - 1 hours);
        vm.expectRevert(Wrapper.NotUndeliverable.selector);
        wrapper.refund(id);
        vm.warp(vm.getBlockTimestamp() + 2 hours);                 // the sequencer's clock alone is not enough
        vm.expectRevert(Wrapper.NotUndeliverable.selector);
        wrapper.refund(id);
        _skip(2 hours);
        wrapper.refund(id);
        assertEq(wrapper.balanceOf(alice), 2 * ONE);
        _assertLedger();
    }

    /// Only the custodian attests, and only a resolved ticket.
    function test_N2c_onlyTheCustodianMarksAResolvedTicket() public {
        _deposit(alice);
        vm.prank(alice);
        uint256 id = wrapper.redeem(TAPROOT);
        vm.prank(custodian);
        vm.expectRevert(Wrapper.WrongStage.selector);
        wrapper.markUndeliverable(id);                             // pinned, not resolved
        _skip(2 minutes);
        _resolveTicket(id, keccak256("r"));
        vm.prank(keeper);
        vm.expectRevert(Wrapper.NotCustodian.selector);
        wrapper.markUndeliverable(id);
        vm.prank(alice);
        vm.expectRevert(Wrapper.NotCustodian.selector);
        wrapper.markUndeliverable(id);
    }

    /// Pinned-ticket refunds (no frog drawn, nothing to deliver) are unchanged: PIN_TIMEOUT after the pin, no attestation.
    function test_N2c_pinnedRefundsAreUnchanged() public {
        _deposit(alice);
        vm.prank(alice);
        uint256 id = wrapper.redeem(TAPROOT);
        _skip(7 days + 1 minutes);
        wrapper.refund(id);
        assertEq(wrapper.balanceOf(alice), ONE);
        _assertLedger();
    }
}
