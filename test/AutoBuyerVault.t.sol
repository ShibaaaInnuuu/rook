// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {AutoBase} from "./utils/AutoBase.sol";
import {AutoBuyerVault} from "../src/AutoBuyerVault.sol";
import {ListingRegistry} from "../src/ListingRegistry.sol";
import {Treasury} from "../src/Treasury.sol";
import {Distributor} from "../src/Distributor.sol";
import {stdError} from "forge-std/StdError.sol";
import {Clock} from "./utils/Clock.sol";

/// The v2 vault: what it lets out, to whom, under what bound, and every way the ETH comes back.
contract AutoBuyerVaultTest is AutoBase {
    // ---- the one address ------------------------------------------------------------------------------------------------

    function test_theBuyerIsImmutableAndIsTheOnlyAddressEthCanReach() public view {
        assertEq(autoOrd.buyer(), autoBuyer);
        assertEq(autoRune.buyer(), autoBuyer);
        assertEq(autoOrd.maxInFlight(), MAX_IN_FLIGHT);
        // there is no setter for either, and no withdraw of any kind
        assertEq(autoOrd.headroom(), MAX_IN_FLIGHT);
    }

    function test_onlyTheBuyerMayAnnounceOrPullOrReceipt() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        vm.prank(keeper);
        autoOrd.postCap(5 ether);

        vm.prank(keeper);
        vm.expectRevert(AutoBuyerVault.NotBuyer.selector);
        autoOrd.announce(1 ether, 1, "cap-ref");
        vm.prank(owner);
        vm.expectRevert(AutoBuyerVault.NotBuyer.selector);
        autoOrd.announce(1 ether, 1, "cap-ref");
        vm.prank(stranger);
        vm.expectRevert(AutoBuyerVault.NotBuyer.selector);
        autoOrd.announce(1 ether, 1, "cap-ref");

        vm.prank(autoBuyer);
        uint256 id = autoOrd.announce(1 ether, 1, "cap-ref");
        Clock.skip(1 hours);
        vm.prank(stranger);
        vm.expectRevert(AutoBuyerVault.NotBuyer.selector);
        autoOrd.pull(id);
        vm.prank(autoBuyer);
        autoOrd.pull(id);
        assertEq(autoBuyer.balance, 1 ether);

        vm.prank(attestor);
        vm.expectRevert(AutoBuyerVault.NotBuyer.selector);
        autoOrd.settleInscription(id, ids[0], bytes32("tx"), 1, _proofOf(0));
    }

    // ---- the cap and the bounds -----------------------------------------------------------------------------------------

    function test_announceIsBoundedByTheCapItsTtlAndMaxInFlight() public {
        _fundAuto(autoOrd, coinA, 30 ether);

        vm.prank(autoBuyer);
        vm.expectRevert(AutoBuyerVault.CapExpired.selector);          // no cap posted at all
        autoOrd.announce(1 ether, 1, "cap-ref");

        vm.prank(keeper);
        autoOrd.postCap(2 ether);
        vm.prank(autoBuyer);
        vm.expectRevert(abi.encodeWithSelector(AutoBuyerVault.OverCap.selector, 3 ether, 2 ether));
        autoOrd.announce(3 ether, 1, "cap-ref");

        // the bound is per unit: two inscriptions may cost twice the cap
        vm.prank(autoBuyer);
        uint256 id = autoOrd.announce(4 ether, 2, "cap-ref");
        assertEq(autoOrd.inFlight(), 4 ether);

        // maxInFlight is the ceiling on everything open at once
        vm.prank(keeper);
        autoOrd.postCap(30 ether);
        vm.prank(autoBuyer);
        vm.expectRevert(abi.encodeWithSelector(AutoBuyerVault.OverInFlight.selector, 17 ether, 16 ether));
        autoOrd.announce(17 ether, 1, "cap-ref");

        // and the cap dies after an hour
        Clock.skip(1 hours + 1);
        vm.prank(autoBuyer);
        vm.expectRevert(AutoBuyerVault.CapExpired.selector);
        autoOrd.announce(1 ether, 1, "cap-ref");
        id;
    }

    function test_announceNeedsSpendableFeesAndRespectsMaxOpen() public {
        _fundAuto(autoOrd, coinA, 1 ether);
        vm.prank(keeper);
        autoOrd.postCap(10 ether);

        vm.prank(autoBuyer);
        vm.expectRevert(abi.encodeWithSelector(AutoBuyerVault.Insufficient.selector, 2 ether, 1 ether));
        autoOrd.announce(2 ether, 1, "cap-ref");

        vm.startPrank(autoBuyer);
        autoOrd.announce(0.25 ether, 1, "cap-ref");
        autoOrd.announce(0.25 ether, 1, "cap-ref");
        autoOrd.announce(0.25 ether, 1, "cap-ref");
        vm.expectRevert(AutoBuyerVault.TooManyOpen.selector);
        autoOrd.announce(0.25 ether, 1, "cap-ref");
        vm.stopPrank();
    }

    function test_aPausedListingAnnouncesNothingAndPullsNothing() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        vm.prank(keeper);
        autoOrd.postCap(5 ether);
        vm.prank(autoBuyer);
        uint256 id = autoOrd.announce(1 ether, 1, "cap-ref");

        vm.prank(owner);
        registry.setPaused(2, true);
        Clock.skip(1 hours);
        vm.prank(autoBuyer);
        vm.expectRevert(AutoBuyerVault.ListingPaused.selector);
        autoOrd.pull(id);
        vm.prank(autoBuyer);
        vm.expectRevert(AutoBuyerVault.ListingPaused.selector);
        autoOrd.announce(1 ether, 1, "cap-ref");

        // and the pause lifts, because H2 is closed: somebody is always the owner
        vm.prank(owner);
        registry.setPaused(2, false);
        vm.prank(autoBuyer);
        autoOrd.pull(id);
        assertEq(autoBuyer.balance, 1 ether);
    }

    // ---- H1: the charge list is fixed at announcement ---------------------------------------------------------------------

    /// v1's H1 was that two matured declarations raced for the head of the queue and the loser could need more payers than
    /// MAX_PAYERS — permanently, since the queue only moves forward. Here each announcement already owns the coins that will
    /// pay for it, so the pull order cannot matter. This is the same shape as `test_repro_H1`, run both ways round.
    function test_H1_isStructurallyGoneBecauseCoinsAreChargedAtAnnounceNotAtPull() public {
        _fundAuto(autoOrd, coinA, 2 ether);                     // one big coin at the head
        for (uint256 i = 0; i < 70; ++i) {                      // then a long run of dust
            address dust = address(uint160(0xD05700 + i));
            launcher.set(dust, router, creatorA, 0);
            _fundAuto(autoOrd, dust, autoOrd.MIN_ENTRY());
        }
        vm.prank(keeper);
        autoOrd.postCap(10 ether);

        vm.startPrank(autoBuyer);
        uint256 first = autoOrd.announce(1 ether, 1, "cap-ref");    // one payer: coinA
        uint256 second = autoOrd.announce(1 ether, 1, "cap-ref");   // the rest of coinA, also one payer
        vm.stopPrank();
        assertEq(autoOrd.chargesOf(first).length, 1);
        assertEq(autoOrd.chargesOf(second).length, 1);
        assertEq(autoOrd.chargesOf(first)[0].coin, coinA);
        assertEq(autoOrd.chargesOf(second)[0].coin, coinA);

        // pull them in the OTHER order from the one they were announced in: both work
        Clock.skip(1 hours);
        vm.startPrank(autoBuyer);
        autoOrd.pull(second);
        autoOrd.pull(first);
        vm.stopPrank();
        assertEq(autoBuyer.balance, 2 ether, "neither order bricks the other");
    }

    function test_tooManyPayersIsRefusedAtAnnounceWhereTheBuyerCanSimplyAskForLess() public {
        for (uint256 i = 0; i < 70; ++i) {
            address dust = address(uint160(0xD06700 + i));
            launcher.set(dust, router, creatorA, 0);
            _fundAuto(autoOrd, dust, autoOrd.MIN_ENTRY());
        }
        vm.prank(keeper);
        autoOrd.postCap(10 ether);
        vm.prank(autoBuyer);
        vm.expectRevert(AutoBuyerVault.TooManyPayers.selector);
        autoOrd.announce(0.65 ether, 1, "cap-ref");             // 65 dust coins
        vm.prank(autoBuyer);
        uint256 id = autoOrd.announce(0.64 ether, 1, "cap-ref"); // 64 is the ceiling, and it goes through
        assertEq(autoOrd.chargesOf(id).length, 64);
    }

    // ---- the public hour and the pull --------------------------------------------------------------------------------------

    function test_aPullWaitsItsPublicHourAndGoesOnlyToTheBuyer() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        vm.prank(keeper);
        autoOrd.postCap(5 ether);
        vm.prank(autoBuyer);
        uint256 id = autoOrd.announce(1 ether, 1, "cap-ref");

        AutoBuyerVault.Purchase memory p = autoOrd.purchase(id);
        assertEq(uint8(p.status), uint8(AutoBuyerVault.Status.Announced));
        vm.prank(autoBuyer);
        vm.expectRevert(abi.encodeWithSelector(AutoBuyerVault.TooEarly.selector, p.readyAt));
        autoOrd.pull(id);

        Clock.warpTo(uint256(p.readyAt));
        uint256 vaultBefore = address(autoOrd).balance;
        vm.prank(autoBuyer);
        autoOrd.pull(id);
        assertEq(autoBuyer.balance, 1 ether);
        assertEq(address(autoOrd).balance, vaultBefore - 1 ether);
        assertEq(autoOrd.purchase(id).deadline, uint64(vm.getBlockTimestamp() + 3 days));

        vm.prank(autoBuyer);
        vm.expectRevert(AutoBuyerVault.WrongStatus.selector);
        autoOrd.pull(id);                                       // once, and only once
    }

    // ---- receipts ------------------------------------------------------------------------------------------------------------

    function test_anInscriptionReceiptMustBeInTheCollectionAndNotAlreadyHeld() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        uint256 id = _pulled(autoOrd, 2 ether, 2);

        bytes32[] memory empty = new bytes32[](0);
        vm.prank(autoBuyer);
        vm.expectRevert(AutoBuyerVault.NotInCollection.selector);
        autoOrd.settleInscription(id, "not-in-the-collection", bytes32("tx"), 1, empty);

        vm.startPrank(autoBuyer);
        autoOrd.settleInscription(id, ids[0], bytes32("tx"), 1000, _proofOf(0));
        vm.expectRevert(AutoBuyerVault.AlreadyHeld.selector);
        autoOrd.settleInscription(id, ids[0], bytes32("tx"), 1000, _proofOf(0));
        autoOrd.settleInscription(id, ids[1], bytes32("tx"), 1000, _proofOf(1));
        vm.stopPrank();

        assertEq(uint8(autoOrd.purchase(id).status), uint8(AutoBuyerVault.Status.Settled));
        assertEq(autoOrd.inFlight(), 0, "settling frees the headroom");
        assertEq(autoOrd.acquisitionCount(), 2);
        assertEq(autoOrd.acquisition(0).coin, coinA);
    }

    function test_aRuneReceiptSplitsWhatArrivedAcrossThePayingCoins() public {
        _fundAuto(autoRune, coinA, 0.6 ether);
        _fundAuto(autoRune, coinB, 0.4 ether);
        uint256 id = _pulled(autoRune, 1 ether, 100);           // 1 whole rune unit = 100 smallest units

        vm.prank(autoBuyer);
        autoRune.settleRune(id, 100, bytes32("tx"), 5000);
        assertEq(autoRune.acquisitionCount(), 2);
        assertEq(autoRune.acquisition(0).amount, 60);
        assertEq(autoRune.acquisition(1).amount, 40);
        assertEq(uint8(autoRune.purchase(id).status), uint8(AutoBuyerVault.Status.Settled));
    }

    function test_aShortDeliveryMustReturnTheMoneyForWhatNeverCame() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        uint256 id = _pulled(autoOrd, 2 ether, 2);              // 1 ETH each
        vm.prank(autoBuyer);
        autoOrd.settleInscription(id, ids[0], bytes32("tx"), 1000, _proofOf(0));

        vm.prank(autoBuyer);
        vm.expectRevert(abi.encodeWithSelector(AutoBuyerVault.ReturnUnspentFirst.selector, 1 ether));
        autoOrd.settleShort(id);

        vm.deal(autoBuyer, 1 ether);
        vm.prank(autoBuyer);
        autoOrd.returnFunds{value: 1 ether}(id);
        vm.prank(autoBuyer);
        autoOrd.settleShort(id);
        assertEq(uint8(autoOrd.purchase(id).status), uint8(AutoBuyerVault.Status.Settled));
        assertEq(autoOrd.inFlight(), 0);
    }

    function test_aReceiptAfterTheDeadlineIsAcceptedAndFlaggedLate() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        uint256 id = _pulled(autoOrd, 1 ether, 1);
        Clock.skip(3 days + 1);
        (bool blocked,) = autoOrd.isBlocked();
        assertTrue(blocked);

        vm.prank(autoBuyer);
        autoOrd.settleInscription(id, ids[0], bytes32("tx"), 1000, _proofOf(0));
        assertTrue(autoOrd.purchase(id).late, "a delivery that happened belongs on the record, marked for what it was");
        (blocked,) = autoOrd.isBlocked();
        assertFalse(blocked);
    }

    // ---- money back ---------------------------------------------------------------------------------------------------------

    function test_anAnnouncementThatWasNeverPulledIsCancelledAndTheCoinsAreWholeAgain() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        vm.prank(keeper);
        autoOrd.postCap(5 ether);
        vm.prank(autoBuyer);
        uint256 id = autoOrd.announce(2 ether, 1, "cap-ref");
        assertEq(autoOrd.attributed(), 3 ether);
        assertEq(autoOrd.chargedTo(coinA), 2 ether);

        vm.prank(stranger);
        vm.expectRevert(AutoBuyerVault.NotBuyer.selector);
        autoOrd.returnFunds(id);
        vm.deal(autoBuyer, 1 ether);
        vm.prank(autoBuyer);
        vm.expectRevert(AutoBuyerVault.BadQuantity.selector);      // and it takes no money: nothing ever left
        autoOrd.returnFunds{value: 1}(id);

        vm.prank(owner);
        autoOrd.returnFunds(id);
        assertEq(uint8(autoOrd.purchase(id).status), uint8(AutoBuyerVault.Status.Returned));
        assertEq(autoOrd.attributed(), 5 ether, "back where it was");
        assertEq(autoOrd.chargedTo(coinA), 0);
        assertEq(autoOrd.inFlight(), 0);
        assertEq(autoOrd.openPurchases().length, 0);
    }

    /// A buyer that stops between announce and pull, with an owner who never acts, used to be a state only two keys could
    /// leave. After PUBLIC_CANCEL_AFTER anyone may tidy it up; nothing has left the vault, so it costs nothing.
    function test_aStrandedAnnouncementCanBeCancelledByAnyoneAfterTheWindow() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        vm.prank(keeper);
        autoOrd.postCap(5 ether);
        vm.prank(autoBuyer);
        uint256 id = autoOrd.announce(2 ether, 1, "cap-ref");

        Clock.warpTo(uint256(autoOrd.purchase(id).readyAt) + 7 days);
        vm.prank(stranger);
        vm.expectRevert(AutoBuyerVault.NotBuyer.selector);          // one second too early
        autoOrd.returnFunds(id);

        Clock.skip(1);
        vm.prank(stranger);
        autoOrd.returnFunds(id);
        assertEq(autoOrd.attributed(), 5 ether);
        assertEq(autoOrd.headroom(), MAX_IN_FLIGHT);
    }

    function test_anyoneMayReturnUnspentEthAndAFullReturnClosesThePurchase() public {
        _fundAuto(autoOrd, coinA, 0.6 ether);
        _fundAuto(autoOrd, coinB, 0.4 ether);
        uint256 id = _pulled(autoOrd, 1 ether, 1);
        assertEq(autoOrd.inFlight(), 1 ether);

        // permission is deliberately absent: making the coins whole must need nobody's key
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        autoOrd.returnFunds{value: 0.4 ether}(id);
        assertEq(autoOrd.inFlight(), 0.6 ether);
        assertEq(uint8(autoOrd.purchase(id).status), uint8(AutoBuyerVault.Status.Pulled));

        vm.deal(autoBuyer, 1 ether);
        vm.prank(autoBuyer);
        autoOrd.returnFunds{value: 0.6 ether}(id);
        assertEq(uint8(autoOrd.purchase(id).status), uint8(AutoBuyerVault.Status.Returned));
        assertEq(autoOrd.inFlight(), 0);
        assertEq(autoOrd.chargedTo(coinA), 0);
        assertEq(autoOrd.chargedTo(coinB), 0);
        assertEq(autoOrd.attributed(), 1 ether);
    }

    function test_changeThatComesBackAfterTheReceiptStillReachesTheCoins() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        uint256 id = _pulled(autoOrd, 2 ether, 1);
        vm.prank(autoBuyer);
        autoOrd.settleInscription(id, ids[0], bytes32("tx"), 1000, _proofOf(0));
        assertEq(autoOrd.inFlight(), 0);

        vm.deal(autoBuyer, 1 ether);
        vm.prank(autoBuyer);
        autoOrd.returnFunds{value: 0.5 ether}(id);
        assertEq(autoOrd.inFlight(), 0, "a settled purchase already left the books; a late refund must not move them twice");
        assertEq(autoOrd.chargedTo(coinA), 1.5 ether);
        assertEq(autoOrd.attributed(), 3.5 ether);
    }

    /// v1's C1, restated against v2's code: the clamp is here too, and this is the split that used to break it.
    function test_C1_theClampIsHereToo_aTruncatingSplitRefundCompletes() public {
        _fundAuto(autoOrd, coinA, 0.6 ether);
        _fundAuto(autoOrd, coinB, 0.4 ether);
        uint256 id = _pulled(autoOrd, 1 ether, 1);

        vm.deal(autoBuyer, 2 ether);
        vm.startPrank(autoBuyer);
        autoOrd.returnFunds{value: 333_333_333_333_333_333}(id);
        assertEq(autoOrd.chargedTo(coinA), 400_000_000_000_000_000);
        assertEq(autoOrd.chargedTo(coinB), 266_666_666_666_666_667);
        autoOrd.returnFunds{value: 666_666_666_666_666_667}(id);
        vm.stopPrank();
        assertEq(autoOrd.chargedTo(coinA), 0);
        assertEq(autoOrd.chargedTo(coinB), 0);
    }

    function testFuzz_C1_anySplitOfARefundCompletes(uint256 first, uint256 second) public {
        first = bound(first, 1, 1 ether - 2);
        second = bound(second, 1, 1 ether - first - 1);
        _fundAuto(autoOrd, coinA, 0.6 ether);
        _fundAuto(autoOrd, coinB, 0.4 ether);
        uint256 id = _pulled(autoOrd, 1 ether, 1);

        vm.deal(autoBuyer, 2 ether);
        vm.startPrank(autoBuyer);
        autoOrd.returnFunds{value: first}(id);
        autoOrd.returnFunds{value: second}(id);
        autoOrd.returnFunds{value: 1 ether - first - second}(id);
        vm.stopPrank();
        assertEq(autoOrd.chargedTo(coinA), 0);
        assertEq(autoOrd.chargedTo(coinB), 0);
        assertEq(autoOrd.attributed(), 1 ether);
    }

    /// A stranger may always make the coins whole — but must not be able to close the purchase by doing it. If they could,
    /// anyone could spend `maxEth` to race a receipt: the buyer really bought the asset, the purchase closes as Returned, the
    /// acquisition is never recorded, and the asset can never be distributed. Expensive, permanent, and no theft — which is
    /// exactly the kind of griefing that is worth one extra condition.
    function test_aStrangerMayRepayAPurchaseButMayNotCloseIt() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        uint256 id = _pulled(autoOrd, 1 ether, 1);

        vm.deal(stranger, 2 ether);
        vm.prank(stranger);
        autoOrd.returnFunds{value: 1 ether}(id);
        assertEq(uint8(autoOrd.purchase(id).status), uint8(AutoBuyerVault.Status.Pulled), "still open, so the receipt can land");
        assertEq(autoOrd.inFlight(), 0);
        assertEq(autoOrd.chargedTo(coinA), 0, "and the coins are whole either way");

        // the receipt the stranger tried to erase still lands, and the acquisition is recorded
        vm.prank(autoBuyer);
        autoOrd.settleInscription(id, ids[0], bytes32("tx"), 1000, _proofOf(0));
        assertEq(uint8(autoOrd.purchase(id).status), uint8(AutoBuyerVault.Status.Settled));
        assertEq(autoOrd.acquisitionCount(), 1);
    }

    /// Money that finds its way home after a write-off un-writes exactly that much of it. Without this there was nowhere for
    /// a late bridge refund to go: the vault has no `receive()`, so the ETH would have stayed with the buyer for ever.
    function test_aLateRefundAfterAWriteOffComesBackAndUnwritesTheLoss() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        uint256 id = _pulled(autoOrd, 2 ether, 1);
        Clock.skip(3 days + 1);
        vm.prank(owner);
        autoOrd.writeOff(id, bytes32("the bridge never paid"));
        assertEq(autoOrd.totalWrittenOff(), 2 ether);

        // there is no other door: the vault takes ETH only through deposit, depositFrom and returnFunds
        (bool ok,) = address(autoOrd).call{value: 0}("");
        assertFalse(ok, "no receive(), by design");

        vm.deal(stranger, 2 ether);
        vm.prank(stranger);
        autoOrd.returnFunds{value: 1.2 ether}(id);
        assertEq(autoOrd.totalWrittenOff(), 0.8 ether, "the record says what was actually lost, not what was feared");
        assertEq(autoOrd.chargedTo(coinA), 0.8 ether);
        assertEq(autoOrd.attributed(), 4.2 ether, "and the coins have it back to spend: the 3 never charged, plus the 1.2 home");
        assertEq(autoOrd.inFlight(), 0, "a closed purchase does not re-enter the books");
    }

    // ---- what happens when Bitcoin never delivers ---------------------------------------------------------------------------

    function test_anOverdueBuyFreezesTheVaultAndIsWrittenOffWithNoBondToPayForIt() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        uint256 id = _pulled(autoOrd, 2 ether, 1);

        vm.prank(owner);
        vm.expectRevert(AutoBuyerVault.NotOverdue.selector);
        autoOrd.writeOff(id, bytes32("too soon"));

        Clock.skip(3 days + 1);
        (bool blocked, uint256 by) = autoOrd.isBlocked();
        assertTrue(blocked);
        assertEq(by, id);
        vm.prank(keeper);
        autoOrd.postCap(5 ether);
        vm.prank(autoBuyer);
        vm.expectRevert(abi.encodeWithSelector(AutoBuyerVault.BlockedByOverdue.selector, id));
        autoOrd.announce(1 ether, 1, "cap-ref");

        vm.prank(stranger);
        vm.expectRevert(AutoBuyerVault.NotOwner.selector);          // not yet: seven more days
        autoOrd.writeOff(id, bytes32("anyone"));

        vm.prank(owner);
        autoOrd.writeOff(id, bytes32("the bridge never paid"));
        assertEq(autoOrd.totalWrittenOff(), 2 ether);
        assertEq(autoOrd.chargedTo(coinA), 2 ether, "the coins paid and got nothing, and that stays on the record");
        assertEq(autoOrd.inFlight(), 0);
        (blocked,) = autoOrd.isBlocked();
        assertFalse(blocked);

        vm.prank(autoBuyer);
        autoOrd.announce(1 ether, 1, "cap-ref");                    // the vault works again
    }

    function test_sevenDaysLaterAnybodyMayWriteOffSoAnOwnerCannotFreezeTheVaultByDoingNothing() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        uint256 id = _pulled(autoOrd, 2 ether, 1);
        Clock.skip(3 days + 7 days + 2);
        vm.prank(stranger);
        autoOrd.writeOff(id, bytes32("nobody else would"));
        assertEq(autoOrd.totalWrittenOff(), 2 ether);
    }

    function test_aPartlyReturnedPurchaseWritesOffOnlyWhatIsActuallyGone() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        uint256 id = _pulled(autoOrd, 2 ether, 1);
        vm.deal(autoBuyer, 2 ether);
        vm.prank(autoBuyer);
        autoOrd.returnFunds{value: 1.5 ether}(id);
        Clock.skip(3 days + 1);
        vm.prank(owner);
        autoOrd.writeOff(id, bytes32("half of it came home"));
        assertEq(autoOrd.totalWrittenOff(), 0.5 ether);
        assertEq(autoOrd.chargedTo(coinA), 0.5 ether);
        assertEq(autoOrd.inFlight(), 0);
    }

    // ---- the money never goes anywhere else -----------------------------------------------------------------------------------

    function test_noKeyCanMoveEthAnywhereButTheBuyer() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        // the whole external surface, tried by every key that exists
        address[4] memory keys = [owner, keeper, attestor, stranger];
        for (uint256 i = 0; i < keys.length; ++i) {
            vm.startPrank(keys[i]);
            vm.expectRevert(AutoBuyerVault.NotBuyer.selector);
            autoOrd.announce(1 ether, 1, "cap-ref");
            vm.stopPrank();
        }
        assertEq(address(autoOrd).balance, 5 ether);
        // a pull sends to `buyer` and to nothing the caller names: there is no destination argument anywhere in the ABI
        uint256 id = _pulled(autoOrd, 1 ether, 1);
        assertEq(autoBuyer.balance, 1 ether);
        assertEq(address(autoOrd).balance, 4 ether);
        id;
    }

    function test_depositsStillComeOnlyFromACoinsOwnRouterOrTheTreasury() public {
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        vm.expectRevert(AutoBuyerVault.NotRouter.selector);
        autoOrd.deposit{value: 1 ether}(coinA);

        vm.deal(treasury, 2 ether);
        vm.prank(treasury);
        autoOrd.depositFrom{value: 1 ether}(coinA);
        assertEq(autoOrd.contributedBy(coinA), 1 ether);
        vm.prank(treasury);
        vm.expectRevert(AutoBuyerVault.UnknownCoin.selector);
        autoOrd.depositFrom{value: 1 ether}(makeAddr("a coin no launcher knows"));
    }

    /// The Treasury and the Distributor each take a vault address from the registry. Before `IEtchVault` they cast it to
    /// `AssetVault` and called it, which worked on a v2 vault only because the two contracts happened to share a field order.
    /// This drives both, end to end, against a v2 vault — so the compatibility is tested rather than assumed.
    function test_theTreasuryAndTheDistributorBothWorkAgainstAV2Vault() public {
        Treasury till = new Treasury(registry);
        vm.startPrank(owner);
        registry.proposeTreasury(address(till));
        Clock.skip(7 days);
        registry.acceptTreasury();
        till.setToken(coinA);
        vm.stopPrank();

        // the treasury's share arrives in a v2 vault, attributed to the protocol token
        vm.deal(address(till), 5 ether);
        vm.prank(keeper);
        till.fund(2, 5 ether);
        assertEq(autoOrd.contributedBy(coinA), 5 ether);
        assertEq(address(autoOrd).balance, 5 ether);

        // it buys, and the receipt is an acquisition the Distributor can read
        uint256 id = _pulled(autoOrd, 1 ether, 1);
        vm.prank(autoBuyer);
        uint256 aid = autoOrd.settleInscription(id, ids[0], bytes32("tx"), 1000, _proofOf(0));

        vm.prank(creatorA);
        distributor.choose(2, aid, Distributor.Mode.DrawWeighted, address(0));
        assertEq(uint8(distributor.lot(2, aid).mode), uint8(Distributor.Mode.DrawWeighted));

        // and a rune vault's ProRata mode, which reads `isRune()` off the same interface
        _fundAuto(autoRune, coinA, 2 ether);
        uint256 rid = _pulled(autoRune, 1 ether, 100);
        vm.prank(autoBuyer);
        autoRune.settleRune(rid, 100, bytes32("tx"), 5000);
        vm.prank(creatorA);
        distributor.choose(3, 0, Distributor.Mode.ProRata, address(0));
        assertEq(uint8(distributor.lot(3, 0).mode), uint8(Distributor.Mode.ProRata));
    }

    function test_feesUnderMinEntryWaitAndAreNotSpendable() public {
        _fundAuto(autoOrd, coinA, autoOrd.MIN_ENTRY() - 1);
        assertEq(autoOrd.waiting(), autoOrd.MIN_ENTRY() - 1);
        assertEq(autoOrd.attributed(), 0);
        assertEq(autoOrd.queueLength(), 0);
        _fundAuto(autoOrd, coinA, 1);
        assertEq(autoOrd.waiting(), 0);
        assertEq(autoOrd.attributed(), autoOrd.MIN_ENTRY());
        assertEq(autoOrd.queueLength(), 1);
    }
}
