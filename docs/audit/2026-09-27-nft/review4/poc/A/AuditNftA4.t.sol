// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

// Review round 4, part A. Run in a private copy of the forge project at test/audit_nft_a4/ (imports as the repo's test/ethnft files).
import {EthNftBase} from "../ethnft/EthNftBase.sol";
import {EthNftInvariantTest} from "../ethnft/EthNft.invariant.t.sol";
import {EthNftWrapper} from "../../src/nft/EthNftWrapper.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// What holds (a control, passing = holds): the round-3 invariant suite, handler unchanged (noteSent, the late refund, marks,
/// deliveries, advances, returns), at BAYC's launch parameters: convBps 1,000 and reserveBps 0 (J1).
contract AuditNftA4LaunchParamsInvariant is EthNftInvariantTest {
    function _params() internal view override returns (FloorModule.Params memory p) {
        p = super._params();
        p.convBps = 1_000;
        p.reserveBps = 0;
    }

    function test_A4_theParamsAreTheLaunchOnes() public view {
        assertEq(module.convBps(), 1_000);
        assertEq(module.reserveBps(), 0);
    }
}

contract AuditNftA4 is EthNftBase {
    bytes32 constant SENT = keccak256("a transfer the custodian saw mined");

    function _resolvedTicket(uint256 tokenId) internal returns (uint256 t) {
        _mintItem(tokenId, alice);
        t = _redeem(alice, alice);
        _resolve(t, 0);
    }

    /// A4-L-2's contract half (passing = the weakness is present): noteSent is the custodian's bare word, taken before finality, with
    /// nothing on this chain to tell a delivery that later dropped out of Ethereum from one that landed. Once noted, the ticket's only
    /// ways out are recordDelivery and markUndeliverable, both the custodian's: a year on, anyone's refund still reverts.
    function test_A4_L2_aNotedTicketHasNoExitButTheCustodians() public {
        uint256 t = _resolvedTicket(77);
        vm.prank(custodian);
        w.noteSent(t, SENT);
        _skip(365 days);
        vm.expectRevert(EthNftWrapper.DeliverySentNotRecorded.selector);
        w.refund(t);
        assertEq(uint8(w.ticket(t).stage), uint8(EthNftWrapper.Stage.Resolved));
        assertEq(w.balanceOf(alice), 0, "the redeemer burned the unit and has nothing on this chain");
        // the custodian's mark still opens it (what an upgraded canister would do)
        vm.prank(custodian);
        w.markUndeliverable(t);
        w.refund(t);
        assertEq(w.balanceOf(alice), ONE);
        _books();
    }

    /// What holds: noteSent grants the custodian nothing it did not have. recordDelivery already closes a ticket on its word alone
    /// (and removes the item from the ledger); noteSent only withholds the late window, changes no number, and cannot touch a ticket
    /// outside Resolved. A re-note replaces the hash; a mark after a note refunds at the grace.
    function test_A4_whatHolds_noteSentIsWeakerThanRecordDelivery() public {
        uint256 t = _resolvedTicket(77);
        uint256 supply = w.totalSupply();
        vm.startPrank(custodian);
        w.noteSent(t, SENT);
        w.noteSent(t, keccak256("another"));
        vm.stopPrank();
        assertEq(w.sentTx(t), keccak256("another"));
        assertEq(w.totalSupply(), supply);
        vm.prank(custodian);
        w.recordDelivery(t, bytes32(uint256(1)));        // the stronger power, unchanged by round 3
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.WrongStage.selector);
        w.noteSent(t, SENT);
        _books();
    }

    /// What holds: noteSent does not depend on the listing's pause (a delivery that already left is noted while minting is paused).
    function test_A4_whatHolds_noteSentAndMarkIgnoreTheListingPause() public {
        uint256 t = _resolvedTicket(77);
        vm.prank(owner);
        registry.setPaused(0, true);
        assertTrue(registry.isPaused(0));
        vm.prank(custodian);
        w.noteSent(t, SENT);
        vm.prank(custodian);
        w.markUndeliverable(t);
        _skip(7 days + 72 hours + 1);
        w.refund(t);
        assertTrue(w.refunded(t));
    }
}
