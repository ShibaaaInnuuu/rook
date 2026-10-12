// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "./utils/Base.sol";
import {AssetVault} from "../src/AssetVault.sol";
import {Clock} from "./utils/Clock.sol";

contract AssetVaultTest is Base {
    bytes32 constant TXID = keccak256("a bitcoin txid");

    function _cap(AssetVault v, uint256 c) internal { vm.prank(keeper); v.postCap(c); }
    function _declare(AssetVault v, uint256 maxEth, uint256 q) internal returns (uint256 id) {
        vm.prank(keeper);
        id = v.declarePurchase(maxEth, q, "floor: test");
    }
    function _released(AssetVault v, uint256 maxEth, uint256 q) internal returns (uint256 id) {
        id = _declare(v, maxEth, q);
        Clock.skip(1 hours);
        v.release(id);
    }

    // ---- money in ----

    function test_onlyACoinsOwnRouterMayAttributeMoneyToIt() public {
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        vm.expectRevert(AssetVault.NotRouter.selector);
        ordVault.deposit{value: 1 ether}(coinA);
        vm.prank(router);
        vm.expectRevert(AssetVault.NotRouter.selector);
        ordVault.deposit{value: 1 ether}(makeAddr("a coin nobody launched"));
        _fund(ordVault, coinA, 1 ether);
        assertEq(ordVault.contributedBy(coinA), 1 ether);
        assertEq(ordVault.attributed(), 1 ether);
    }

    function test_thereIsNoWithdraw_forAnyone() public {
        _fund(ordVault, coinA, 5 ether);
        bytes4[5] memory sels = [bytes4(keccak256("withdraw()")), bytes4(keccak256("withdraw(uint256)")), bytes4(keccak256("withdraw(address,uint256)")),
            bytes4(keccak256("sweep(address)")), bytes4(keccak256("rescue(address,uint256)"))];
        address[4] memory who = [owner, keeper, purchaser, attestor];
        for (uint256 w = 0; w < who.length; ++w) {
            for (uint256 i = 0; i < sels.length; ++i) {
                vm.prank(who[w]);
                (bool ok,) = address(ordVault).call(abi.encodeWithSelector(sels[i], who[w], 1 ether));
                assertFalse(ok);
            }
        }
        assertEq(address(ordVault).balance, 5 ether);
    }

    // ---- declaring ----

    /// docs/phase11.md §2. The catalogue carries no rune floor, so on the staging chain every real rune's vault is left with no
    /// cap AT ALL, deliberately. This is what the contracts do then: they refuse, and they refuse before anything is reserved.
    function test_aRuneVaultWithNoCapRefusesToBuy_ratherThanBuyingAtAnyPrice() public {
        _fund(runeVault, coinA, 5 ether);
        assertEq(runeVault.capWei(), 0, "no cap was ever posted");
        assertEq(runeVault.capExpiry(), 0);
        assertEq(runeVault.boundFor(1), 0, "nothing is bounded by a cap that does not exist");
        assertEq(runeVault.boundFor(type(uint128).max), 0, "and no quantity makes an unbounded spend out of it");
        for (uint256 q = 1; q < 4; ++q) {
            vm.prank(keeper);
            vm.expectRevert(AssetVault.CapExpired.selector);     // the name is about expiry; expiry 0 is also "never posted"
            runeVault.declarePurchase(1 ether, q * 10 ** 6, "no floor exists for this rune");
        }
        assertEq(runeVault.purchaseCount(), 0, "nothing was declared");
        assertEq(runeVault.reserved(), 0, "and nothing was reserved: the money stays attributed to its coins");
        assertEq(runeVault.attributed(), 5 ether);
    }

    function test_onlyTheKeeperDeclares_andOnlyUnderAFreshCap() public {
        _fund(ordVault, coinA, 5 ether);
        vm.prank(owner);
        vm.expectRevert(AssetVault.NotKeeper.selector);
        ordVault.declarePurchase(1 ether, 1, "x");
        vm.prank(keeper);
        vm.expectRevert(AssetVault.CapExpired.selector);          // no cap was ever posted
        ordVault.declarePurchase(1 ether, 1, "x");
        _cap(ordVault, 1 ether);
        Clock.skip(1 hours + 1);
        vm.prank(keeper);
        vm.expectRevert(AssetVault.CapExpired.selector);          // an hour-old cap is no cap
        ordVault.declarePurchase(1 ether, 1, "x");
        vm.prank(stranger);
        vm.expectRevert(AssetVault.NotKeeper.selector);
        ordVault.postCap(100 ether);
    }

    function test_spendIsBoundedByCapTimesCount() public {
        _fund(ordVault, coinA, 10 ether);
        _cap(ordVault, 1 ether);
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.OverCap.selector, 2 ether + 1, 2 ether));
        ordVault.declarePurchase(2 ether + 1, 2, "x");
        _declare(ordVault, 2 ether, 2);
    }

    function test_aRunesCapIsPerWholeUnit_timesTheAmount() public {
        _fund(runeVault, coinA, 10 ether);
        _cap(runeVault, 0.001 ether);                              // per whole rune; divisibility 2
        assertEq(runeVault.boundFor(150_000), 1.5 ether);          // 1,500.00 runes
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.OverCap.selector, 1.5 ether + 1, 1.5 ether));
        runeVault.declarePurchase(1.5 ether + 1, 150_000, "x");
        _declare(runeVault, 1.5 ether, 150_000);
    }

    function test_cannotDeclareMoreThanIsAttributedAndUnreserved() public {
        _fund(ordVault, coinA, 1 ether);
        _cap(ordVault, 1 ether);
        _declare(ordVault, 0.7 ether, 1);
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.Insufficient.selector, 0.4 ether, 0.3 ether));
        ordVault.declarePurchase(0.4 ether, 1, "x");
        // ETH forced in without attribution is never spendable
        vm.deal(address(ordVault), address(ordVault).balance + 5 ether);
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.Insufficient.selector, 0.4 ether, 0.3 ether));
        ordVault.declarePurchase(0.4 ether, 1, "x");
    }

    function test_atMostThreeOpenPurchases() public {
        _fund(ordVault, coinA, 10 ether);
        _cap(ordVault, 1 ether);
        _declare(ordVault, 1 ether, 1); _declare(ordVault, 1 ether, 1); _declare(ordVault, 1 ether, 1);
        vm.prank(keeper);
        vm.expectRevert(AssetVault.TooManyOpen.selector);
        ordVault.declarePurchase(1 ether, 1, "x");
    }

    function test_aPausedListingDeclaresAndReleasesNothing() public {
        _fund(ordVault, coinA, 5 ether);
        _cap(ordVault, 1 ether);
        uint256 id = _declare(ordVault, 1 ether, 1);
        vm.prank(owner);
        registry.setPaused(0, true);
        vm.prank(keeper);
        vm.expectRevert(AssetVault.ListingPaused.selector);
        ordVault.declarePurchase(1 ether, 1, "x");
        Clock.skip(1 hours);
        vm.expectRevert(AssetVault.ListingPaused.selector);
        ordVault.release(id);
    }

    // ---- releasing ----

    function test_ethLeavesOnlyAfterThePublicHour_toThePurchaserOfRecord_byAnyone() public {
        _fund(ordVault, coinA, 5 ether);
        _cap(ordVault, 1 ether);
        uint256 id = _declare(ordVault, 1 ether, 1);
        vm.expectRevert();
        ordVault.release(id);
        Clock.skip(1 hours - 1);
        vm.expectRevert();
        ordVault.release(id);
        Clock.skip(1);
        vm.prank(stranger);
        ordVault.release(id);
        assertEq(purchaser.balance, 1 ether);
        assertEq(address(ordVault).balance, 4 ether);
        vm.expectRevert(AssetVault.WrongStatus.selector);         // and only once
        ordVault.release(id);
    }

    function test_theOwnerCanCancelBeforeRelease_notAfter_andNobodyElseCan() public {
        _fund(ordVault, coinA, 5 ether);
        _cap(ordVault, 1 ether);
        uint256 id = _declare(ordVault, 1 ether, 1);
        vm.prank(keeper);
        vm.expectRevert(AssetVault.NotOwner.selector);
        ordVault.cancel(id);
        vm.prank(owner);
        ordVault.cancel(id);
        assertEq(ordVault.reserved(), 0);
        Clock.skip(1 hours);
        vm.expectRevert(AssetVault.WrongStatus.selector);
        ordVault.release(id);

        uint256 id2 = _released(ordVault, 1 ether, 1);
        vm.prank(owner);
        vm.expectRevert(AssetVault.WrongStatus.selector);
        ordVault.cancel(id2);
    }

    function test_coinsAreChargedFirstInFirstOut() public {
        _fund(ordVault, coinA, 0.6 ether);
        _fund(ordVault, coinB, 2 ether);
        _cap(ordVault, 1 ether);
        uint256 id = _released(ordVault, 1 ether, 1);
        AssetVault.Charge[] memory cs = ordVault.chargesOf(id);
        assertEq(cs.length, 2);
        assertEq(cs[0].coin, coinA); assertEq(cs[0].weiCharged, 0.6 ether);
        assertEq(cs[1].coin, coinB); assertEq(cs[1].weiCharged, 0.4 ether);
        assertEq(ordVault.chargedTo(coinA), 0.6 ether);
        assertEq(ordVault.attributed(), 1.6 ether);
    }

    // ---- settling ----

    function test_onlyTheAttestorSettles_neverThePurchaserOrTheKeeper() public {
        _fund(ordVault, coinA, 5 ether);
        _cap(ordVault, 1 ether);
        uint256 id = _released(ordVault, 1 ether, 1);
        address[4] memory who = [purchaser, keeper, owner, stranger];
        for (uint256 i = 0; i < who.length; ++i) {
            vm.prank(who[i]);
            vm.expectRevert(AssetVault.NotAttestor.selector);
            ordVault.settleInscription(id, ids[0], TXID, 50_000, _proofOf(0));
        }
    }

    function test_anInscriptionMustBeInTheCommittedCollection_andHeldOnce() public {
        _fund(ordVault, coinA, 5 ether);
        _cap(ordVault, 1 ether);
        uint256 id = _released(ordVault, 2 ether, 2);
        vm.startPrank(attestor);
        vm.expectRevert(AssetVault.NotInCollection.selector);
        ordVault.settleInscription(id, "ffff9999i0", TXID, 1, _proofOf(0));
        uint256 aid = ordVault.settleInscription(id, ids[0], TXID, 50_000, _proofOf(0));
        vm.expectRevert(AssetVault.AlreadyHeld.selector);
        ordVault.settleInscription(id, ids[0], TXID, 50_000, _proofOf(0));
        assertEq(uint8(ordVault.purchase(id).status), uint8(AssetVault.Status.Released), "one of two: still open");
        ordVault.settleInscription(id, ids[1], TXID, 50_000, _proofOf(1));
        vm.stopPrank();
        assertEq(uint8(ordVault.purchase(id).status), uint8(AssetVault.Status.Settled));
        AssetVault.Acquisition memory a = ordVault.acquisition(aid);
        assertEq(a.coin, coinA); assertEq(a.amount, 1); assertEq(a.assetRef, ids[0]); assertEq(a.btcTxid, TXID);
        assertEq(ordVault.openPurchases().length, 0);
    }

    function test_twoCoinsPayForTwoInscriptions_eachIsChargedOne() public {
        _fund(ordVault, coinA, 1 ether);
        _fund(ordVault, coinB, 1 ether);
        _cap(ordVault, 1 ether);
        uint256 id = _released(ordVault, 2 ether, 2);
        vm.startPrank(attestor);
        uint256 a0 = ordVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0));
        uint256 a1 = ordVault.settleInscription(id, ids[1], TXID, 1, _proofOf(1));
        vm.stopPrank();
        assertEq(ordVault.acquisition(a0).coin, coinA);
        assertEq(ordVault.acquisition(a1).coin, coinB);
    }

    function test_aRuneReceiptIsSplitBetweenThePayingCoinsInProportion() public {
        _fund(runeVault, coinA, 1 ether);
        _fund(runeVault, coinB, 3 ether);
        _cap(runeVault, 0.001 ether);
        uint256 id = _released(runeVault, 4 ether, 400_000);
        // docs/phase3.md H2: a little less arrived than was declared, so the ETH declared for the missing unit comes back first
        vm.prank(attestor);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.ReturnUnspentFirst.selector, 0.00001 ether));
        runeVault.settleRune(id, 399_999, TXID, 7_000_000);
        vm.deal(purchaser, 1 ether);
        vm.prank(purchaser); runeVault.returnUnspent{value: 0.00001 ether}(id);
        vm.prank(attestor);
        runeVault.settleRune(id, 399_999, TXID, 7_000_000);
        assertEq(runeVault.acquisitionCount(), 2);
        assertEq(runeVault.acquisition(0).coin, coinA); assertEq(runeVault.acquisition(0).amount, 99_999);
        assertEq(runeVault.acquisition(1).coin, coinB); assertEq(runeVault.acquisition(1).amount, 300_000);
        assertEq(runeVault.acquisition(1).assetRef, "840000:1");
        assertEq(uint8(runeVault.purchase(id).status), uint8(AssetVault.Status.Settled));
    }

    function test_kindsDoNotCross() public {
        _fund(runeVault, coinA, 1 ether);
        _cap(runeVault, 1 ether);
        uint256 id = _released(runeVault, 1 ether, 100);
        vm.startPrank(attestor);
        vm.expectRevert(AssetVault.WrongKind.selector);
        runeVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0));
        vm.expectRevert(AssetVault.BadQuantity.selector);
        runeVault.settleRune(id, 0, TXID, 1);
        vm.stopPrank();
    }

    // ---- the receipt that never comes ----

    function test_oneOverdueReceiptStopsEveryFurtherDeclaration_untilSettledOrWrittenOff() public {
        _fund(ordVault, coinA, 10 ether);
        _cap(ordVault, 1 ether);
        uint256 id = _released(ordVault, 1 ether, 1);
        Clock.skip(3 days + 1);
        _cap(ordVault, 1 ether);
        (bool blocked, uint256 by) = ordVault.isBlocked();
        assertTrue(blocked); assertEq(by, id);
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.BlockedByOverdue.selector, id));
        ordVault.declarePurchase(1 ether, 1, "x");

        // a late receipt unblocks it…
        vm.prank(attestor);
        ordVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0));
        _declare(ordVault, 1 ether, 1);
    }

    function test_aWriteOffIsTheOwners_onlyWhenOverdue_andIsRecordedForEver() public {
        _fund(ordVault, coinA, 10 ether);
        _cap(ordVault, 1 ether);
        uint256 id = _released(ordVault, 1 ether, 1);
        vm.prank(owner);
        vm.expectRevert(AssetVault.NotOverdue.selector);
        ordVault.writeOff(id, "too early");
        Clock.skip(3 days + 1);
        vm.prank(keeper);
        vm.expectRevert(AssetVault.NotOwner.selector);
        ordVault.writeOff(id, "not yours");
        vm.prank(owner);
        ordVault.writeOff(id, keccak256("incident report 1"));
        assertEq(ordVault.totalWrittenOff(), 1 ether);
        assertEq(uint8(ordVault.purchase(id).status), uint8(AssetVault.Status.WrittenOff));
        (bool blocked,) = ordVault.isBlocked();
        assertFalse(blocked);
        vm.prank(attestor);
        vm.expectRevert(AssetVault.WrongStatus.selector);         // a written-off purchase cannot be quietly settled afterwards
        ordVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0));
    }

    function test_unspentEthComesBackToTheCoinsThatPaid() public {
        _fund(ordVault, coinA, 1 ether);
        _fund(ordVault, coinB, 3 ether);
        _cap(ordVault, 4 ether);
        uint256 id = _released(ordVault, 4 ether, 1);
        vm.deal(purchaser, 10 ether);
        vm.prank(purchaser);
        ordVault.returnUnspent{value: 0.4 ether}(id);
        assertEq(ordVault.attributed(), 0.4 ether);
        assertEq(ordVault.chargedTo(coinA), 0.9 ether);
        assertEq(ordVault.chargedTo(coinB), 2.7 ether);
        vm.prank(purchaser);
        vm.expectRevert(AssetVault.BadQuantity.selector);         // never more than was released
        ordVault.returnUnspent{value: 3.7 ether}(id);
    }

    function test_aShortInscriptionPurchaseIsClosedByTheAttestor_onceSomethingArrived() public {
        _fund(ordVault, coinA, 5 ether);
        _cap(ordVault, 1 ether);
        uint256 id = _released(ordVault, 2 ether, 2);
        vm.startPrank(attestor);
        vm.expectRevert(AssetVault.WrongStatus.selector);
        ordVault.settleShort(id);
        ordVault.settleInscription(id, ids[2], TXID, 1, _proofOf(2));
        // docs/phase3.md H2: one of two arrived, so half of what was released has to be back before the purchase can be closed
        vm.expectRevert(abi.encodeWithSelector(AssetVault.ReturnUnspentFirst.selector, 1 ether));
        ordVault.settleShort(id);
        vm.stopPrank();
        vm.deal(purchaser, 1 ether);
        vm.prank(purchaser); ordVault.returnUnspent{value: 1 ether}(id);
        vm.prank(attestor); ordVault.settleShort(id);
        assertEq(uint8(ordVault.purchase(id).status), uint8(AssetVault.Status.Settled));
    }
}
