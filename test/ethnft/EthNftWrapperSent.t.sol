// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {EthNftBase} from "./EthNftBase.sol";
import {EthNftWrapper} from "../../src/nft/EthNftWrapper.sol";

/// Audit 2026-09-27-nft D3-H-1: the canister notes a delivery mined (`noteSent`) right after it transfers; from then on the late
/// window can never refund that ticket, so a delivery that mined but was never recorded (the redeemer moved the NFT on before
/// finality) cannot mint the unit back while the redeemer keeps the NFT. Only `markUndeliverable` reopens the refund.
contract EthNftWrapperSentTest is EthNftBase {
    event DeliverySent(uint256 indexed ticketId, uint256 indexed tokenId, bytes32 txHash);

    bytes32 constant SENT = keccak256("transferFrom(custodian, destination, token) mined on Ethereum");

    function _resolvedTicket(uint256 tokenId) internal returns (uint256 t) {
        _mintItem(tokenId, alice);
        t = _redeem(alice, alice);
        _resolve(t, 0);
    }

    function _noteSent(uint256 t, bytes32 h) internal {
        vm.prank(custodian);
        w.noteSent(t, h);
    }

    /// Past `deliverBy + LATE_DELIVERY_WINDOW` on both clocks, as `test_withoutTheAttestationTheLateWindowIsThirtyDays_onBothClocks`.
    function _pastTheLateWindow(uint256 t) internal {
        uint64 by = w.ticket(t).deliverBy;
        vm.warp(by + 30 days + 1);
        vm.roll(vm.getBlockNumber() + (37 days) / PBS + 2);
    }

    function test_D3H1_theAbiIsWhatTheCanisterCodesAgainst() public pure {
        assertEq(EthNftWrapper.noteSent.selector, bytes4(0x6d65f720));
        assertEq(keccak256("DeliverySent(uint256,uint256,bytes32)"),
            bytes32(0x40ecca6312528fd11b6c79e8d7465570e4ff6c3b6df1e616ea8c093452b61654));
        assertEq(EthNftWrapper.DeliverySentNotRecorded.selector, bytes4(keccak256("DeliverySentNotRecorded()")));
    }

    function test_D3H1_aNotedDeliveryIsNeverRefundedByTheLateWindow() public {
        uint256 t = _resolvedTicket(77);
        _noteSent(t, SENT);
        // (the redeemer moves token 77 on before finality; the canister never records)
        _pastTheLateWindow(t);
        uint256 supply = w.totalSupply();
        uint256 held = w.held();
        uint256 open = w.openTickets();
        vm.prank(alice);
        vm.expectRevert(EthNftWrapper.DeliverySentNotRecorded.selector);
        w.refund(t);
        vm.expectRevert(EthNftWrapper.DeliverySentNotRecorded.selector);
        w.refund(t);                                          // anyone: the same
        _skip(365 days);                                      // and no later either
        vm.expectRevert(EthNftWrapper.DeliverySentNotRecorded.selector);
        w.refund(t);
        assertEq(w.totalSupply(), supply, "no unit minted back");
        assertEq(w.held(), held, "the NFT is not back in the set");
        assertEq(w.openTickets(), open);
        assertEq(w.balanceOf(alice), 0);
        assertEq(uint8(w.ticket(t).stage), uint8(EthNftWrapper.Stage.Resolved));
        // the fixed canister still records it
        _deliver(t);
        assertEq(uint8(w.ticket(t).stage), uint8(EthNftWrapper.Stage.Delivered));
        assertEq(w.held(), held - 1);
        assertEq(w.openTickets(), open - 1);
        vm.expectRevert(EthNftWrapper.WrongStage.selector);
        w.refund(t);
        _books();
    }

    /// The control: the same ticket never noted sent (a custodian gone before it sent) still refunds by the late window.
    function test_D3H1_control_theLateWindowStillRefundsATicketNeverNotedSent() public {
        uint256 t = _resolvedTicket(77);
        uint256 supply = w.totalSupply();
        _pastTheLateWindow(t);
        w.refund(t);
        assertTrue(w.refunded(t));
        assertEq(w.balanceOf(alice), ONE, "the unit is back");
        assertEq(w.totalSupply(), supply + ONE);
        assertEq(w.held(), 1, "the NFT is back in the set");
        _books();
    }

    function test_D3H1_markUndeliverableAfterNoteSent_refundsAtTheGrace() public {
        uint256 t = _resolvedTicket(77);
        _noteSent(t, SENT);
        uint64 by = w.ticket(t).deliverBy;
        _skip(by - vm.getBlockTimestamp() + 72 hours + 1);
        vm.expectRevert(EthNftWrapper.DeliverySentNotRecorded.selector);
        w.refund(t);
        // a reorg dropped the transfer for good: the canister's word that no version can confirm
        vm.prank(custodian);
        w.markUndeliverable(t);
        w.refund(t);
        assertTrue(w.refunded(t));
        assertEq(w.balanceOf(alice), ONE);
        assertEq(w.held(), 1);
        assertEq(w.openTickets(), 0);
        _books();
    }

    function test_D3H1_markUndeliverableAfterNoteSent_stillWaitsForTheGrace() public {
        uint256 t = _resolvedTicket(77);
        _noteSent(t, SENT);
        vm.prank(custodian);
        w.markUndeliverable(t);
        vm.expectRevert(EthNftWrapper.TooEarly.selector);
        w.refund(t);
        uint64 by = w.ticket(t).deliverBy;
        vm.warp(by + 72 hours + 1);                           // the sequencer alone
        vm.expectRevert(EthNftWrapper.TooEarly.selector);
        w.refund(t);
        vm.roll(vm.getBlockNumber() + (7 days + 72 hours) / PBS + 2);
        w.refund(t);
        assertTrue(w.refunded(t));
    }

    function test_noteSent_onlyTheCustodian() public {
        uint256 t = _resolvedTicket(77);
        vm.prank(stranger);
        vm.expectRevert(EthNftWrapper.NotCustodian.selector);
        w.noteSent(t, SENT);
        vm.prank(alice);
        vm.expectRevert(EthNftWrapper.NotCustodian.selector);
        w.noteSent(t, SENT);
        assertEq(w.sentTx(t), bytes32(0));
    }

    function test_noteSent_onlyAResolvedTicket() public {
        _mintItem(1, alice);
        _mintItem(2, alice);
        _mintItem(3, alice);
        uint256 a = _redeem(alice, alice);
        uint256 b = _redeem(alice, alice);
        uint256 c = _redeem(alice, alice);
        // Pinned
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.WrongStage.selector);
        w.noteSent(a, SENT);
        // Delivered
        _resolve(a, 0);
        _deliver(a);
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.WrongStage.selector);
        w.noteSent(a, SENT);
        // Refunded (resolved, marked, refunded)
        _resolve(b, 0);
        _skip(7 days + 72 hours + 1);
        vm.prank(custodian);
        w.markUndeliverable(b);
        w.refund(b);
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.WrongStage.selector);
        w.noteSent(b, SENT);
        // Refunded while pinned (the pin timeout)
        _skip(7 days + 1);
        w.refund(c);
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.WrongStage.selector);
        w.noteSent(c, SENT);
        // no such ticket
        vm.prank(custodian);
        vm.expectRevert();
        w.noteSent(99, SENT);
    }

    function test_noteSent_zeroHashIsRefused() public {
        uint256 t = _resolvedTicket(77);
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.BadInput.selector);
        w.noteSent(t, bytes32(0));
        assertEq(w.sentTx(t), bytes32(0));
    }

    function test_noteSent_emitsStoresAndChangesNoLedgerNumber() public {
        _mintItem(5, bob);
        uint256 t = _resolvedTicket(77);
        uint256 tokenId = _tokenOf(t);
        uint256 supply = w.totalSupply();
        uint256 held = w.held();
        uint256 open = w.openTickets();
        uint256 next = w.nextToResolve();
        uint256 free = w.freeCount();
        uint256 pending = w.pendingCount();
        EthNftWrapper.Ticket memory before = w.ticket(t);
        vm.expectEmit(true, true, false, true, address(w));
        emit DeliverySent(t, tokenId, SENT);
        _noteSent(t, SENT);
        assertEq(w.sentTx(t), SENT);
        assertEq(w.totalSupply(), supply);
        assertEq(w.held(), held);
        assertEq(w.openTickets(), open);
        assertEq(w.nextToResolve(), next);
        assertEq(w.freeCount(), free);
        assertEq(w.pendingCount(), pending);
        assertFalse(w.undeliverable(t));
        assertEq(w.itemOf(tokenId), w.ticket(t).itemId + 1, "still held");
        assertEq(keccak256(abi.encode(w.ticket(t))), keccak256(abi.encode(before)), "the ticket is untouched");
        // again: a replacement transaction overwrites, harmlessly
        bytes32 replacement = keccak256("the replacement, same nonce");
        vm.expectEmit(true, true, false, true, address(w));
        emit DeliverySent(t, tokenId, replacement);
        _noteSent(t, replacement);
        assertEq(w.sentTx(t), replacement);
        assertEq(w.totalSupply(), supply);
        assertEq(w.held(), held);
        assertEq(w.openTickets(), open);
        // recordDelivery takes its own hash, and markUndeliverable remains callable
        uint256 snap = vm.snapshotState();
        vm.prank(custodian);
        w.markUndeliverable(t);
        assertTrue(w.undeliverable(t));
        vm.revertToState(snap);
        vm.prank(custodian);
        w.recordDelivery(t, SENT);
        assertEq(w.ticket(t).deliveryTx, SENT);
        _books();
    }
}
