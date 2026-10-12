// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20Errors} from "@openzeppelin/contracts/interfaces/draft-IERC6093.sol";
import {WrapBase} from "./utils/WrapBase.sol";
import {Clock} from "./utils/Clock.sol";
import {Wrapper} from "../src/Wrapper.sol";
import {Distributor} from "../src/Distributor.sol";

/// docs/v3/wrapper.md, step 1: mint on receipt, burn to a ticket, resolve by beacon, deliver, refund on deadline.
contract WrapperTest is WrapBase {
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");

    // ---- what a wrapper is for ----

    function test_aRuneListingCannotBeWrapped_itIsAlreadyDivisible() public {
        vm.expectRevert(Wrapper.WrongKind.selector);
        new Wrapper(registry, distributor, beacon, 1, distribution, custodian, CUSTODY_BTC, "x", "x", PBS);
    }

    function test_theDistributionAddressCannotBeZero() public {
        vm.expectRevert(Wrapper.BadInput.selector);
        new Wrapper(registry, distributor, beacon, 0, address(0), custodian, CUSTODY_BTC, "x", "x", PBS);
    }

    function test_oneWholeTokenIsOneFrog() public view {
        assertEq(wrapper.decimals(), 18);
        assertEq(wrapper.ONE_ITEM(), 1e18);
        assertEq(wrapper.symbol(), "wFROG");
    }

    // ---- minting ----

    function test_aFrogIsMintedToTheDistributionContract_neverToAPerson() public {
        uint256 aid = _frog(0);
        assertEq(wrapper.balanceOf(distribution), ONE);
        assertEq(wrapper.balanceOf(creatorA), 0, "the creator who wrapped it gets nothing directly");
        assertEq(wrapper.totalSupply(), ONE);
        assertEq(wrapper.held(), 1);
        assertTrue(wrapper.wrapped(aid));
        assertEq(wrapper.pendingCount(), 1, "free for the next ticket, drawable once one resolves");
        _assertLedger();
    }

    function test_onlyThePayingCoinsCreatorWraps() public {
        uint256 aid = _receipt(0);
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.Never, address(0));
        for (uint256 k = 0; k < 3; ++k) {
            address who = [creatorB, keeper, stranger][k];
            vm.prank(who);
            vm.expectRevert(Wrapper.NotCreator.selector);
            wrapper.wrap(aid);
        }
    }

    function test_aFrogThatCouldStillBeDrawnIsNeverWrapped() public {
        uint256 aid = _receipt(0);
        // no choice yet: the creator could still draw it
        vm.prank(creatorA);
        vm.expectRevert(Wrapper.NotNever.selector);
        wrapper.wrap(aid);
        // Hold is the one choice the Distributor lets a creator revisit, so it is not enough either
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.Hold, address(0));
        vm.prank(creatorA);
        vm.expectRevert(Wrapper.NotNever.selector);
        wrapper.wrap(aid);
        // a draw
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.DrawWeighted, address(0));
        vm.prank(creatorA);
        vm.expectRevert(Wrapper.NotNever.selector);
        wrapper.wrap(aid);
    }

    function test_aNamedFrogIsNeverWrapped_itAlreadyBelongsToSomeone() public {
        uint256 aid = _receipt(0);
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.Named, alice);
        vm.prank(creatorA);
        vm.expectRevert(Wrapper.NotNever.selector);
        wrapper.wrap(aid);
    }

    function test_aWrappedFrogCanNeverBeDrawnAfterwards() public {
        uint256 aid = _frog(0);
        vm.prank(creatorA);
        vm.expectRevert(Distributor.WrongStage.selector);
        distributor.choose(0, aid, Distributor.Mode.DrawWeighted, address(0));
    }

    function test_aFrogIsWrappedOnce() public {
        uint256 aid = _frog(0);
        vm.prank(creatorA);
        vm.expectRevert(Wrapper.AlreadyWrapped.selector);
        wrapper.wrap(aid);
        assertEq(wrapper.totalSupply(), ONE);
    }

    function test_anAcquisitionThatDoesNotExistWrapsNothing() public {
        vm.prank(creatorA);
        vm.expectRevert();
        wrapper.wrap(7);
    }

    function test_aPausedListingWrapsNothingNew() public {
        uint256 aid = _receipt(0);
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.Never, address(0));
        vm.prank(owner);
        registry.setPaused(0, true);
        vm.prank(creatorA);
        vm.expectRevert(Wrapper.ListingPaused.selector);
        wrapper.wrap(aid);
    }

    // ---- redeeming ----

    function test_aBurnOpensATicketPinnedToARoundThatDoesNotExistYet() public {
        _frog(0);
        _give(alice, 1);
        uint64 expect = beacon.roundAt(block.timestamp) + wrapper.PIN_AHEAD();
        uint256 id = _redeem(alice);
        Wrapper.Ticket memory t = wrapper.ticket(id);
        assertEq(uint8(t.stage), uint8(Wrapper.Stage.Pinned));
        assertEq(t.owner, alice);
        assertEq(t.round, expect);
        assertEq(t.destination, TAPROOT);
        assertEq(wrapper.balanceOf(alice), 0);
        assertEq(wrapper.totalSupply(), 0, "the token is gone from supply at the burn");
        assertEq(wrapper.openTickets(), 1);
        _assertLedger();
    }

    function test_aBurnNeedsAWholeToken() public {
        _frog(0);
        vm.prank(distribution);
        wrapper.transfer(alice, ONE - 1);
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(IERC20Errors.ERC20InsufficientBalance.selector, alice, ONE - 1, ONE));
        wrapper.redeem(TAPROOT);
    }

    function test_aBurnNeedsATaprootAddress() public {
        _frog(0);
        _give(alice, 1);
        vm.prank(alice);
        vm.expectRevert(Wrapper.BadAddress.selector);
        wrapper.redeem("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq");
        assertEq(wrapper.balanceOf(alice), ONE, "and burns nothing when it refuses");
    }

    // ---- resolving ----

    function test_resolvingNamesAFrogAndStartsTheDeliveryClock() public {
        uint256 aid = _frog(0);
        _give(alice, 1);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        Wrapper.Ticket memory t = wrapper.ticket(id);
        assertEq(uint8(t.stage), uint8(Wrapper.Stage.Resolved));
        assertEq(t.acquisitionId, aid, "the only frog there is");
        assertEq(t.deliverBy, block.timestamp + 7 days);
        assertEq(wrapper.nextToResolve(), 1);
        assertEq(wrapper.freeCount(), 0, "and it is spoken for");
        _assertLedger();
    }

    function test_theResultIsTheSameWhoeverResolves() public {
        _frog(0); _frog(1); _frog(2);
        _give(alice, 1);
        uint256 id = _redeem(alice);
        bytes32 r = keccak256("the round");
        uint256 snap = vm.snapshotState();
        vm.prank(alice); wrapper.resolve(id, abi.encodePacked(r));
        uint256 byAlice = wrapper.ticket(id).acquisitionId;
        vm.revertToState(snap);
        vm.prank(stranger); wrapper.resolve(id, abi.encodePacked(r));
        assertEq(wrapper.ticket(id).acquisitionId, byAlice);
    }

    function test_ticketsResolveStrictlyInOrder() public {
        _frog(0); _frog(1);
        _give(alice, 2);
        _redeem(alice);
        uint256 second = _redeem(alice);
        vm.expectRevert(abi.encodeWithSelector(Wrapper.OutOfOrder.selector, 0));
        wrapper.resolve(second, abi.encodePacked(keccak256("r")));
        _resolve(0, keccak256("r"));
        _resolve(second, keccak256("r2"));
        assertTrue(wrapper.ticket(0).acquisitionId != wrapper.ticket(second).acquisitionId, "two tickets, two frogs");
    }

    function test_aTicketResolvesOnce() public {
        _frog(0);
        _give(alice, 1);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        vm.expectRevert(abi.encodeWithSelector(Wrapper.OutOfOrder.selector, 1));
        wrapper.resolve(id, abi.encodePacked(keccak256("r")));
    }

    /// Every frog free when a ticket was burned can be the one it draws, and only those.
    function testFuzz_aTicketDrawsOnlyAmongFrogsFreeWhenItWasBurned(bytes32 r) public {
        uint256 a0 = _frog(0);
        uint256 a1 = _frog(1);
        _give(alice, 1);
        uint256 id = _redeem(alice);
        _frog(2);                                  // wrapped AFTER the burn
        _resolve(id, r);
        uint256 got = wrapper.ticket(id).acquisitionId;
        assertTrue(got == a0 || got == a1, "a frog wrapped after the burn is not this ticket's to draw");
        assertEq(wrapper.pendingCount(), 1, "the late frog waits for a later ticket");
        _assertLedger();
    }

    // ---- delivering ----

    /// B-03 / C-05: only the custodian, which signs the Bitcoin spend, records a delivery. The keeper cannot.
    function test_onlyTheCustodianRecordsTheDelivery_andTheFrogLeavesTheBooks() public {
        _frog(0);
        _give(alice, 1);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        vm.prank(stranger);
        vm.expectRevert(Wrapper.NotCustodian.selector);
        wrapper.recordDelivery(id, TXID);
        vm.prank(keeper);
        vm.expectRevert(Wrapper.NotCustodian.selector);
        wrapper.recordDelivery(id, TXID);
        vm.prank(custodian);
        vm.expectRevert(Wrapper.BadInput.selector);
        wrapper.recordDelivery(id, bytes32(0));
        vm.prank(custodian);
        wrapper.recordDelivery(id, TXID);
        Wrapper.Ticket memory t = wrapper.ticket(id);
        assertEq(uint8(t.stage), uint8(Wrapper.Stage.Delivered));
        assertEq(t.deliveryTxid, TXID);
        assertEq(wrapper.held(), 0);
        assertEq(wrapper.openTickets(), 0);
        _assertLedger();
        vm.prank(custodian);
        vm.expectRevert(Wrapper.WrongStage.selector);
        wrapper.recordDelivery(id, TXID);
    }

    function test_theCustodianRecordsItsOwnDelivery_andCanDoNothingElse() public {
        _frog(0);
        _frog(1);
        _give(alice, 2);
        uint256 id = _redeem(alice);
        uint256 id2 = _redeem(alice);
        _resolve(id, keccak256("r"));
        vm.prank(custodian);
        wrapper.recordDelivery(id, TXID);
        assertEq(uint8(wrapper.ticket(id).stage), uint8(Wrapper.Stage.Delivered));
        _assertLedger();
        // nothing else is the custodian's: it cannot resolve out of order or refund early, and it holds no token to redeem
        vm.prank(custodian);
        vm.expectRevert(Wrapper.WrongStage.selector);
        wrapper.recordDelivery(id2, TXID);   // not resolved yet
        _resolve(id2, keccak256("s"));
        vm.prank(custodian);
        vm.expectRevert(Wrapper.TooEarly.selector);
        wrapper.refund(id2);
        vm.prank(custodian);
        vm.expectRevert();
        wrapper.redeem(CUSTODY_BTC);
    }

    function test_custodyIsFixedAtConstruction_aTaprootAddress_andAZeroCustodianIsRefused() public {
        assertEq(wrapper.custodian(), custodian);
        assertEq(wrapper.custodyAddress(), CUSTODY_BTC);
        vm.expectRevert(Wrapper.BadInput.selector);
        new Wrapper(registry, distributor, beacon, 0, distribution, address(0), CUSTODY_BTC, "x", "x", PBS);
        vm.expectRevert(Wrapper.BadAddress.selector);
        new Wrapper(registry, distributor, beacon, 0, distribution, custodian, "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", "x", "x", PBS);
    }

    function test_aFrogIsWrappedOnlyWhileTheListingDeliversToCustody() public {
        uint256 aid = _receipt(0);
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.Never, address(0));
        // the owner moves the listing's delivery address: seven public days, then wrapping stops
        vm.prank(owner);
        registry.proposeBtcVault(0, "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0");
        Clock.skip(7 days);
        vm.prank(owner);
        registry.acceptBtcVault(0);
        vm.prank(creatorA);
        vm.expectRevert(Wrapper.NotCustody.selector);
        wrapper.wrap(aid);
        // and back: wrapping resumes
        vm.prank(owner);
        registry.proposeBtcVault(0, CUSTODY_BTC);
        Clock.skip(7 days);
        vm.prank(owner);
        registry.acceptBtcVault(0);
        vm.prank(creatorA);
        wrapper.wrap(aid);
        assertTrue(wrapper.wrapped(aid));
    }

    function test_nothingIsDeliveredBeforeItIsResolved() public {
        _frog(0);
        _give(alice, 1);
        uint256 id = _redeem(alice);
        vm.prank(custodian);
        vm.expectRevert(Wrapper.WrongStage.selector);
        wrapper.recordDelivery(id, TXID);
    }

    function test_aLateDeliveryIsStillRecorded_untilSomebodyRefunds() public {
        _frog(0);
        _give(alice, 1);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        Clock.skip(8 days);
        vm.prank(custodian);
        wrapper.recordDelivery(id, TXID);
        vm.expectRevert(Wrapper.WrongStage.selector);
        wrapper.refund(id);
    }

    // ---- refunding ----

    function test_anUndeliveredTicketRefunds_theFrogReturnsAndTheTokenComesBack() public {
        uint256 aid = _frog(0);
        _give(alice, 1);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        vm.expectRevert(Wrapper.TooEarly.selector);
        wrapper.refund(id);
        Clock.skip(7 days);
        vm.expectRevert(Wrapper.TooEarly.selector);
        wrapper.refund(id);                        // the deadline has passed, the refund grace has not
        vm.prank(custodian);
        wrapper.markUndeliverable(id);             // the custodian's attestation (review2 R3 §4)
        Clock.skip(72 hours);
        vm.prank(stranger);                        // anyone may: it only ever pays the ticket's owner
        wrapper.refund(id);
        assertEq(wrapper.balanceOf(alice), ONE, "re-minted to the ticket's owner");
        assertEq(uint8(wrapper.ticket(id).stage), uint8(Wrapper.Stage.Refunded));
        assertEq(wrapper.held(), 1, "the frog is still the vault's");
        assertEq(wrapper.openTickets(), 0);
        assertEq(wrapper.pendingCount(), 1);
        _assertLedger();
        // and the same frog is redeemable again
        uint256 again = _redeem(alice);
        _resolve(again, keccak256("r"));
        assertEq(wrapper.ticket(again).acquisitionId, aid);
        vm.expectRevert(Wrapper.WrongStage.selector);
        wrapper.refund(id);
    }

    /// E-03: a ticket nobody resolves is refunded a week after its pin (both clocks), no frog moving; and resolution moves past it,
    /// so it blocks no later ticket.
    function test_aPinnedTicketIsRefundedAfterAWeek_andResolutionMovesPastIt() public {
        uint256 aid = _frog(0);
        _frog(1);
        _give(alice, 2);
        uint256 id = _redeem(alice);
        uint256 id2 = _redeem(alice);
        vm.expectRevert(Wrapper.TooEarly.selector);
        wrapper.refund(id);
        Clock.skipTimeOnly(8 days);
        vm.expectRevert(Wrapper.TooEarly.selector);
        wrapper.refund(id);                                  // the parent chain has not waited a week
        Clock.skip(7 days);
        vm.prank(stranger);
        wrapper.refund(id);
        assertEq(uint8(wrapper.ticket(id).stage), uint8(Wrapper.Stage.Refunded));
        assertEq(wrapper.balanceOf(alice), ONE, "the token is back");
        assertEq(wrapper.held(), 2, "no frog moved");
        assertEq(wrapper.nextToResolve(), id2, "the stuck ticket no longer blocks the next");
        _assertLedger();
        _resolve(id2, keccak256("s"));
        assertEq(uint8(wrapper.ticket(id2).stage), uint8(Wrapper.Stage.Resolved), "the next ticket draws");
        aid;
        vm.expectRevert(Wrapper.WrongStage.selector);
        wrapper.refund(id);
    }

    /// C-02: the pinned round is final. There is no redraw, at any age, proven or not.
    function test_thePinnedRoundIsNeverReplaced() public {
        _frog(0);
        _give(alice, 1);
        uint256 id = _redeem(alice);
        uint64 round = wrapper.ticket(id).round;
        Clock.skip(3 days);
        (bool ok,) = address(wrapper).call(abi.encodeWithSignature("redraw(uint256)", id));
        assertFalse(ok, "there is no redraw");
        assertEq(wrapper.ticket(id).round, round);
    }

    // ---- the owner cannot stop a redemption ----

    function test_pausingTheListingStopsNoRedemption() public {
        _frog(0); _frog(1);
        _give(alice, 2);
        uint256 id = _redeem(alice);
        vm.prank(owner);
        registry.setPaused(0, true);
        uint256 id2 = _redeem(alice);
        _resolve(id, keccak256("r"));
        _resolve(id2, keccak256("r2"));
        vm.prank(custodian);
        wrapper.recordDelivery(id, TXID);
        Clock.skip(7 days + 72 hours);
        vm.prank(custodian);
        wrapper.markUndeliverable(id2);            // the custodian's attestation (review2 R3 §4)
        wrapper.refund(id2);
        _assertLedger();
    }

    // ---- the whole path ----

    function test_threeFrogs_threeBurns_oneDelivered_oneRefunded_oneOpen_theBooksBalanceAtEveryStep() public {
        _frog(0); _assertLedger();
        _frog(1); _assertLedger();
        _frog(2); _assertLedger();
        _give(alice, 2);
        _give(bob, 1);
        uint256 a = _redeem(alice); _assertLedger();
        uint256 b = _redeem(bob); _assertLedger();
        uint256 c = _redeem(alice); _assertLedger();
        assertEq(wrapper.totalSupply(), 0);
        _resolve(a, keccak256("a")); _assertLedger();
        _resolve(b, keccak256("b")); _assertLedger();
        _resolve(c, keccak256("c")); _assertLedger();
        uint256 fa = wrapper.ticket(a).acquisitionId;
        uint256 fb = wrapper.ticket(b).acquisitionId;
        uint256 fc = wrapper.ticket(c).acquisitionId;
        assertTrue(fa != fb && fb != fc && fa != fc, "three tickets, three different frogs");
        vm.prank(custodian); wrapper.recordDelivery(a, TXID); _assertLedger();
        Clock.skip(7 days + 72 hours);
        vm.prank(custodian); wrapper.markUndeliverable(b);   // the custodian's attestation (review2 R3 §4)
        wrapper.refund(b); _assertLedger();
        assertEq(wrapper.held(), 2);
        assertEq(wrapper.openTickets(), 1);
        assertEq(wrapper.totalSupply(), ONE, "bob's token is back; alice's second ticket is still open");
    }
}
