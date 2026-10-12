// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {HomeBase} from "./HomeBase.sol";
import {Clock} from "../utils/Clock.sol";
import {Merkle} from "../utils/Merkle.sol";
import {MockInbox, MockGatewayRouter, TriggerBot, RefusingCaller} from "./HomeMocks.sol";
import {HomeVaultBase} from "../../src/homechain/HomeVaultBase.sol";
import {HomeVaultYoyo} from "../../src/homechain/HomeVaultYoyo.sol";
import {HomeVaultRaffle} from "../../src/homechain/HomeVaultRaffle.sol";
import {HomeVaultBurn} from "../../src/homechain/HomeVaultBurn.sol";
import {HomeVaultWrap} from "../../src/homechain/HomeVaultWrap.sol";
import {HomeGuard} from "../../src/homechain/HomeGuard.sol";
import {HomePolicy, IHomeReceiverCredit, IFeeEscrowConfirm, IL1GatewayRouter} from "../../src/homechain/IHomeChain.sol";
import {SeaportGuard} from "../../src/nft/SeaportGuard.sol";
import {ISeaport16Sweep, BasicOrderParameters} from "../../src/nft/Seaport16.sol";

/// docs/v6/home-chain-vault.md §3: money in by ticket, the two buys, the fixed reward, and each policy's fate.
contract HomeVaultTest is HomeBase {
    // ---- construction ----------------------------------------------------------------------------------------------------------

    function test_constructorRefusals() public {
        HomeVaultBase.Config memory c = _config(address(nft), false);
        c.collection = makeAddr("no code");
        vm.expectRevert(HomeVaultBase.BadInput.selector);
        new HomeVaultBurn(c);
        c = _config(address(nft), false); c.seaport = address(0);
        vm.expectRevert(HomeVaultBase.BadInput.selector);
        new HomeVaultBurn(c);
        c = _config(address(nft), false); c.reward = 0;
        vm.expectRevert(HomeVaultBase.BadInput.selector);
        new HomeVaultBurn(c);
        c = _config(address(nft), false); c.parentBlockSeconds = 0;
        vm.expectRevert();
        new HomeVaultBurn(c);
        c = _config(address(nft), false);
        vm.expectRevert(HomeVaultBase.BadInput.selector);
        new HomeVaultYoyo(c, 10_999, TTL);
        vm.expectRevert(HomeVaultBase.BadInput.selector);
        new HomeVaultYoyo(c, 12_001, TTL);
        vm.expectRevert(HomeVaultBase.BadInput.selector);
        new HomeVaultWrap(c, address(0), IL1GatewayRouter(address(gateway)));
        assertEq(uint8(raffle.policy()), uint8(HomePolicy.Raffle));
        assertEq(uint8(yoyo.policy()), uint8(HomePolicy.Yoyo));
        assertEq(uint8(wrapVault.policy()), uint8(HomePolicy.Wrap));
        assertEq(uint8(burnVault.policy()), uint8(HomePolicy.Burn));
        // the one approval, only in a Yoyo vault of an ERC-721
        assertTrue(nft.isApprovedForAll(address(yoyo), address(seaport)));
        assertFalse(nft.isApprovedForAll(address(raffle), address(seaport)));
        assertFalse(nft.isApprovedForAll(address(burnVault), address(seaport)));
        assertFalse(nft.isApprovedForAll(address(wrapVault), address(seaport)));
    }

    // ---- money in --------------------------------------------------------------------------------------------------------------

    function test_fundByTicket_nativeAndFast() public {
        _fundNative(raffle, 1, 1 ether);
        _fundFast(raffle, 2, 0.5 ether);
        assertEq(raffle.fundedNative(1), 1 ether);
        assertEq(raffle.fundedFast(2), 0.5 ether);
        assertEq(raffle.attributed(), 1.5 ether);
        assertEq(raffle.queueLength(), 2);
        assertEq(raffle.queueAt(0), 1);
        // the outbox with another L2 sender is not the escrow: counted as fast
        outbox.execute{value: 0.1 ether}(stranger, address(raffle), abi.encodeCall(HomeVaultBase.fund, (3)));
        assertEq(raffle.fundedFast(3), 0.1 ether);
        assertEq(raffle.fundedNative(3), 0);
    }

    function test_fundRefusesZeroAndTicketZero() public {
        vm.expectRevert(HomeVaultBase.ZeroAmount.selector);
        raffle.fund{value: 0}(1);
        vm.expectRevert(HomeVaultBase.BadInput.selector);
        raffle.fund{value: 1 ether}(0);
    }

    function test_plainTransferIsBudgetUnderTicketZero() public {
        vm.prank(solver);
        (bool ok,) = address(raffle).call{value: 1 ether}("");
        assertTrue(ok);
        assertEq(raffle.unspentOf(0), 1 ether);
        assertEq(raffle.attributed(), 1 ether);
    }

    function test_smallFundingsWaitUntilMinEntry() public {
        _fundFast(raffle, 1, 0.004 ether);
        assertEq(raffle.waiting(), 0.004 ether);
        assertEq(raffle.attributed(), 0);
        _fundFast(raffle, 1, 0.006 ether);
        assertEq(raffle.waiting(), 0);
        assertEq(raffle.attributed(), 0.01 ether);
    }

    // ---- the cap and the keeper --------------------------------------------------------------------------------------------------

    function test_capExpiresOnBothClocks() public {
        _cap(raffle, CAP);
        assertTrue(raffle.capFresh());
        Clock.skipTimeOnly(2 hours);            // the sequencer's clock alone does not expire it... on an L2. On Ethereum both move.
        assertFalse(raffle.capFresh());         // timestamp past: stale (either clock past means stale)
        _cap(raffle, CAP);
        vm.roll(block.number + 400);            // 400 × 12 s > 1 h by blocks alone
        assertFalse(raffle.capFresh());
        vm.expectRevert(HomeVaultBase.NotKeeper.selector);
        raffle.postCap(1, "x");
        vm.prank(keeper);
        vm.expectRevert(HomeVaultBase.ZeroAmount.selector);
        raffle.postCap(0, "x");
    }

    function test_keeperRotationTakesSevenDaysOnBothClocks() public {
        address k2 = makeAddr("keeper 2");
        vm.expectRevert(HomeVaultBase.NotOwner.selector);
        raffle.proposeKeeper(k2);
        vm.prank(timelock);
        raffle.proposeKeeper(k2);
        vm.expectRevert(HomeVaultBase.TooEarly.selector);
        raffle.acceptKeeper();
        Clock.skipTimeOnly(7 days);
        vm.expectRevert(HomeVaultBase.TooEarly.selector);
        raffle.acceptKeeper();
        Clock.skip(1);
        vm.roll(block.number + 7 days / 12);
        raffle.acceptKeeper();
        assertEq(raffle.keeper(), k2);
        vm.prank(keeper);
        vm.expectRevert(HomeVaultBase.NotKeeper.selector);
        raffle.postCap(1, "x");
    }

    // ---- buying through Seaport ------------------------------------------------------------------------------------------------

    function test_buySeaport_anyoneTriggersAndGetsTheReward() public {
        _fundNative(raffle, 1, 2 ether);
        _cap(raffle, CAP);
        uint256 before = trigger.balance;
        uint256 aid = _buy(raffle, _ids(1), 1 ether);
        assertEq(nft.ownerOf(1), address(raffle));
        assertEq(trigger.balance - before, REWARD, "the fixed reward, nothing more");
        assertEq(seller.balance, 1 ether);
        assertEq(raffle.attributed(), 2 ether - 1 ether - REWARD, "price and reward charged to the ticket");
        HomeVaultBase.Acquisition memory a = raffle.acquisition(aid);
        assertEq(a.tokenId, 1);
        assertEq(a.fundedBy, 1);
        assertEq(a.pricePaid, 1 ether);
        assertEq(uint8(a.stage), uint8(HomeVaultBase.Stage.Held));
        HomeGuard.Cap memory k = guard.capOf(address(nft)); uint256 items = k.items; uint256 value = k.valueWei;
        assertEq(items, 1);
        assertEq(value, 1 ether);
    }

    function test_buySeaport_severalAndBothBasicSelectors() public {
        _fundNative(raffle, 1, 10 ether);
        _cap(raffle, CAP);
        _buy(raffle, _ids(1, 2, 3), 1 ether);
        assertEq(raffle.acquisitionCount(), 3);
        _ask(_ids(4, 5), 0.5 ether);
        vm.prank(trigger, trigger);
        raffle.buySeaport(abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (_basicParams(4))), _ids(4), 1);
        vm.prank(trigger, trigger);
        raffle.buySeaport(abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder_efficient_6GL6yc, (_basicParams(5))), _ids(5), 1);
        assertEq(nft.ownerOf(5), address(raffle));
        assertEq(raffle.acquisitionCount(), 5);
    }

    function test_buySeaport_refusals() public {
        _fundNative(raffle, 1, 2 ether);
        // no cap
        _ask(_ids(1), 1 ether);
        vm.prank(trigger, trigger);
        vm.expectRevert(HomeVaultBase.CapExpired.selector);
        raffle.buySeaport(_buyCall(_ids(1), address(raffle)), _ids(1), 1);
        _cap(raffle, CAP);
        // over the cap
        _ask(_ids(2), 6 ether);
        vm.prank(trigger, trigger);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.OrderOverCap.selector, 0, 6 ether, CAP));
        raffle.buySeaport(_buyCall(_ids(2), address(raffle)), _ids(2), 1);
        // budget short of price + reward + buffer
        _ask(_ids(3), 1.99 ether);
        vm.prank(trigger, trigger);
        vm.expectRevert(abi.encodeWithSelector(HomeVaultBase.Insufficient.selector, 1.99 ether + REWARD + BUFFER, 2 ether));
        raffle.buySeaport(_buyCall(_ids(3), address(raffle)), _ids(3), 1);
        // candidates rules
        vm.prank(trigger, trigger);
        vm.expectRevert(HomeVaultBase.BadCandidates.selector);
        raffle.buySeaport(_buyCall(_ids(1), address(raffle)), new uint256[](0), 1);
        vm.prank(trigger, trigger);
        vm.expectRevert(abi.encodeWithSelector(HomeVaultBase.DuplicateCandidate.selector, 1));
        raffle.buySeaport(_buyCall(_ids(1), address(raffle)), _ids(1, 1), 1);
        // a punks entry on an ERC-721 vault
        vm.expectRevert(HomeVaultBase.WrongKind.selector);
        raffle.buyPunk(1);
        vm.expectRevert(HomeVaultBase.WrongKind.selector);
        punkYoyo.buySeaport("", _ids(1), 1);
    }

    function test_buySeaport_aSwallowingMarketIsRefusedByTheOutcomeCheck() public {
        _fundNative(raffle, 1, 2 ether);
        _cap(raffle, CAP);
        _ask(_ids(1), 1 ether);
        seaport.setSwallow(true);
        vm.prank(trigger, trigger);
        vm.expectRevert(abi.encodeWithSelector(HomeVaultBase.NotEnoughBought.selector, 0, 1));
        raffle.buySeaport(_buyCall(_ids(1), address(raffle)), _ids(1), 1);
        assertEq(raffle.attributed(), 2 ether, "nothing charged");
    }

    function test_buySeaport_pausedGuardStopsBuying() public {
        _fundNative(raffle, 1, 2 ether);
        _cap(raffle, CAP);
        vm.prank(timelock);
        guard.setPaused(true);
        _ask(_ids(1), 1 ether);
        vm.prank(trigger, trigger);
        vm.expectRevert(HomeVaultBase.WrongStage.selector);
        raffle.buySeaport(_buyCall(_ids(1), address(raffle)), _ids(1), 1);
        _fundNative(raffle, 2, 1 ether);     // money in still works while paused
        assertEq(raffle.attributed(), 3 ether);
    }

    function test_guardCapsBindABuy() public {
        _fundNative(raffle, 1, 30 ether);
        _cap(raffle, 20 ether);
        _ask(_ids(1), 11 ether);              // over the per-item cap of 10 ETH
        vm.prank(trigger, trigger);
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.OverCap.selector, "item price", 11 ether, 10 ether));
        raffle.buySeaport(_buyCall(_ids(1), address(raffle)), _ids(1), 1);
    }

    function test_rewardCannotBeRefusedIntoAStuckVault_refusingCallerRevertsItself() public {
        _fundNative(raffle, 1, 2 ether);
        _cap(raffle, CAP);
        _ask(_ids(1), 1 ether);
        RefusingCaller rc = new RefusingCaller();
        vm.expectRevert(HomeVaultBase.TransferFailed.selector);
        rc.call(address(raffle), abi.encodeCall(HomeVaultBase.buySeaport, (_buyCall(_ids(1), address(raffle)), _ids(1), 1)));
    }

    function test_nothingArrivesOutsideABuy() public {
        nft.mint(seller, 9);
        vm.prank(seller);
        vm.expectRevert(HomeVaultBase.NotDuringBuy.selector);
        nft.safeTransferFrom(seller, address(raffle), 9);
        // a plain transferFrom lands (and is no acquisition): safe, wasteful, as on 4663
        vm.prank(seller);
        nft.transferFrom(seller, address(raffle), 9);
        assertEq(raffle.acquisitionCount(), 0);
    }

    // ---- buying a punk -------------------------------------------------------------------------------------------------------------

    function test_buyPunk_fromTheOrderBook_yoyoRelistsInTheSameCall() public {
        _fundNative(punkYoyo, 1, 5 ether);
        _cap(punkYoyo, CAP);
        uint256 before = trigger.balance;
        uint256 aid = _buyPunk(punkYoyo, 7, 2 ether);
        assertEq(punks.punkIndexToAddress(7), address(punkYoyo));
        assertEq(trigger.balance - before, REWARD);
        assertEq(punks.pendingWithdrawals(seller), 2 ether, "the seller is credited, as the real market does");
        (bool forSale,, address s, uint256 minValue, address only) = punks.punksOfferedForSale(7);
        assertTrue(forSale);
        assertEq(s, address(punkYoyo));
        assertEq(minValue, 2.4 ether, "relisted at 1.2x in the buying call");
        assertEq(only, address(0));
        assertEq(uint8(punkYoyo.acquisition(aid).stage), uint8(HomeVaultBase.Stage.Listed));
        assertEq(punkYoyo.listing(aid).ask, 2.4 ether);
    }

    function test_buyPunk_refusals() public {
        _fundNative(punkYoyo, 1, 5 ether);
        _cap(punkYoyo, CAP);
        punks.mint(seller, 1);
        vm.expectRevert(HomeVaultBase.NotForSale.selector);
        punkYoyo.buyPunk(1);
        vm.prank(seller);
        punks.offerPunkForSaleToAddress(1, 1 ether, stranger);      // to one address: not for anyone
        vm.expectRevert(HomeVaultBase.NotForSale.selector);
        punkYoyo.buyPunk(1);
        vm.prank(seller);
        punks.offerPunkForSale(1, 6 ether);
        vm.expectRevert(abi.encodeWithSelector(HomeVaultBase.OverCap.selector, 6 ether, CAP));
        punkYoyo.buyPunk(1);
        vm.prank(seller);
        punks.offerPunkForSale(1, 4.99 ether);
        vm.expectRevert(abi.encodeWithSelector(HomeVaultBase.Insufficient.selector, 4.99 ether + REWARD + BUFFER, 5 ether));
        punkYoyo.buyPunk(1);
    }

    // ---- the cheapest-ask hint (docs/v6/home-chain-vault.md §3.2) -------------------------------------------------------------------

    /// Anyone names a cheaper offered punk; a trigger that then picks a dearer one is refused, the named one buys, and once the
    /// named one is the vault's (relisted by the vault, so not "offered by somebody else") the hint is stale and blocks nothing.
    function test_hint_aCheaperHintedPunkBlocksADearerBuy() public {
        _fundNative(punkYoyo, 1, 10 ether);
        _cap(punkYoyo, CAP);
        _offerPunk(7, 2 ether);
        _offerPunk(8, 1.5 ether);
        (uint256 hid, uint256 hask) = punkYoyo.cheaperHint(7, 2 ether);
        assertEq(hask, 0, "nothing hinted yet: nothing blocks");
        vm.prank(stranger);
        punkYoyo.hintAsk(8);
        (hid, hask) = punkYoyo.cheaperHint(7, 2 ether);
        assertEq(hid, 8);
        assertEq(hask, 1.5 ether);
        vm.prank(trigger, trigger);
        vm.expectRevert(abi.encodeWithSelector(HomeVaultBase.CheaperOffered.selector, 8, 1.5 ether));
        punkYoyo.buyPunk(7);
        vm.prank(trigger, trigger);
        punkYoyo.buyPunk(8);                                        // the hinted one is what buys
        assertEq(punks.punkIndexToAddress(8), address(punkYoyo));
        (, hask) = punkYoyo.cheaperHint(7, 2 ether);
        assertEq(hask, 0, "relisted by the vault itself: no longer an offer by somebody else");
        vm.prank(trigger, trigger);
        punkYoyo.buyPunk(7);
        assertEq(punks.punkIndexToAddress(7), address(punkYoyo));
        assertEq(punkYoyo.hints().length, 1, "a stale hint stays until somebody takes its slot");
    }

    /// The list is bounded and only ever holds punks offered to anyone: a full list takes a cheaper hint in the dearest's slot, a
    /// withdrawn hint's slot goes first, a dearer hint into a full list is refused, and an equal ask blocks nothing.
    function test_hint_refusalsAndTheBoundedList() public {
        vm.expectRevert(HomeVaultBase.WrongKind.selector);
        raffle.hintAsk(1);                                          // an ERC-721 vault has no order book
        punks.mint(seller, 1);
        vm.expectRevert(HomeVaultBase.NotForSale.selector);
        punkYoyo.hintAsk(1);
        vm.prank(seller);
        punks.offerPunkForSaleToAddress(1, 1 ether, stranger);
        vm.expectRevert(HomeVaultBase.NotForSale.selector);         // offered to one address: not to anyone
        punkYoyo.hintAsk(1);
        vm.prank(seller);
        punks.offerPunkForSale(1, 0);
        vm.expectRevert(HomeVaultBase.NotForSale.selector);         // a zero ask is not an ask
        punkYoyo.hintAsk(1);
        for (uint256 i = 0; i < punkYoyo.MAX_HINTS(); ++i) {
            _offerPunk(10 + i, (10 + i) * 1 ether);
            punkYoyo.hintAsk(10 + i);
        }
        assertEq(punkYoyo.hints().length, 8);
        vm.expectRevert(HomeVaultBase.AlreadyHinted.selector);
        punkYoyo.hintAsk(10);
        _offerPunk(20, 20 ether);
        vm.expectRevert(HomeVaultBase.HintNotCheaper.selector);    // full, and dearer than the dearest (17 ETH)
        punkYoyo.hintAsk(20);
        _offerPunk(21, 5 ether);
        punkYoyo.hintAsk(21);                                       // full: takes the dearest's slot
        uint256[] memory h = punkYoyo.hints();
        assertEq(h.length, 8);
        assertEq(h[7], 21, "punk 17's slot (the dearest) is now punk 21's");
        vm.prank(seller);
        punks.punkNoLongerForSale(12);                              // a hint withdrawn from the book: its slot goes first
        _offerPunk(30, 100 ether);
        punkYoyo.hintAsk(30);
        h = punkYoyo.hints();
        assertEq(h[2], 30, "the stale slot, even for a dearer ask");
        // an equal ask does not block, and the check reads the book now: the cheapest live hint is 5 ETH (punk 21)
        (uint256 hid, uint256 hask) = punkYoyo.cheaperHint(99, 5 ether);
        assertEq(hask, 0, "5 ETH is not cheaper than 5 ETH");
        (hid, hask) = punkYoyo.cheaperHint(99, 5 ether + 1);
        assertEq(hid, 21);
        assertEq(hask, 5 ether);
        (, hask) = punkYoyo.cheaperHint(21, 5 ether + 1);
        assertEq(hask, 0, "the punk being bought is never its own competitor");
        _fundNative(punkYoyo, 1, 10 ether);
        _cap(punkYoyo, CAP);
        _offerPunk(40, 3 ether);
        vm.prank(trigger, trigger);
        punkYoyo.buyPunk(40);                                       // 3 ETH: nothing hinted is offered for less
        assertEq(punks.punkIndexToAddress(40), address(punkYoyo));
    }

    // ---- Yoyo: settle -----------------------------------------------------------------------------------------------------------------

    function test_yoyoPunk_settleWithdrawsAndBridgesProceedsDown() public {
        _fundNative(punkYoyo, 1, 5 ether);
        _cap(punkYoyo, CAP);
        uint256 aid = _buyPunk(punkYoyo, 7, 2 ether);
        vm.expectRevert(HomeVaultYoyo.NotSold.selector);
        punkYoyo.settle(aid);
        // a buyer takes the ask on the order book
        address buyer = makeAddr("punk buyer");
        vm.deal(buyer, 3 ether);
        vm.prank(buyer);
        punks.buyPunk{value: 2.4 ether}(7);
        assertEq(punks.pendingWithdrawals(address(punkYoyo)), 2.4 ether);
        uint256 before = stranger.balance;
        uint256 n = inbox.count();
        vm.prank(stranger);
        punkYoyo.settle(aid);
        assertEq(stranger.balance - before, REWARD);
        assertEq(inbox.count(), n + 1);
        MockInbox.Retryable memory r = inbox.last();
        assertEq(r.to, receiver);
        assertEq(r.callValueRefund, receiver);
        assertEq(r.excessFeeRefund, receiver);
        uint256 fee = punkYoyo.bridgeFee(36);
        assertEq(r.l2CallValue, 2.4 ether - REWARD - fee);
        assertEq(r.value, 2.4 ether - REWARD);
        assertEq(r.data, abi.encodeCall(IHomeReceiverCredit.creditProceeds, (1)));
        assertEq(punkYoyo.proceedsHeld(), 0);
        assertEq(uint8(punkYoyo.acquisition(aid).stage), uint8(HomeVaultBase.Stage.Sold));
        assertEq(punkYoyo.attributed(), 5 ether - 2 ether - REWARD, "the budget untouched by the sale");
        assertEq(address(punkYoyo).balance, punkYoyo.attributed());
    }

    function test_yoyoErc721_relistValidatesAnOrderAndSettleBridges() public {
        _fundNative(yoyo, 1, 5 ether);
        _cap(yoyo, CAP);
        uint256 aid = _buy(yoyo, _ids(1), 1 ether);
        assertEq(uint8(yoyo.acquisition(aid).stage), uint8(HomeVaultBase.Stage.Held));
        uint256 before = stranger.balance;
        vm.prank(stranger);
        yoyo.relist(aid);
        assertEq(stranger.balance - before, REWARD);
        HomeVaultYoyo.Listing memory l = yoyo.listing(aid);
        assertEq(l.ask, 1.2 ether);
        assertTrue(seaport.isValidated(l.orderHash), "validated by the offerer, no signature");
        assertEq(l.listedUntil, block.timestamp + TTL);
        vm.prank(stranger);
        vm.expectRevert(HomeVaultYoyo.StillListed.selector);
        yoyo.relist(aid);
        // a buyer fills it
        address buyer = makeAddr("buyer");
        vm.deal(buyer, 2 ether);
        vm.prank(buyer);
        seaport.fill{value: 1.2 ether}(l.orderHash);
        assertEq(nft.ownerOf(1), buyer);
        assertEq(yoyo.proceedsHeld(), 1.2 ether, "Seaport paid the vault: proceeds, not budget");
        vm.prank(stranger);
        vm.expectRevert(HomeVaultYoyo.NotSold.selector);
        yoyo.relist(aid);
        vm.prank(stranger);
        yoyo.settle(aid);
        assertEq(inbox.last().l2CallValue, 1.2 ether - REWARD - yoyo.bridgeFee(36));
        assertEq(yoyo.proceedsHeld(), 0);
    }

    function test_yoyoErc721_expiredListingCanBeRelisted() public {
        _fundNative(yoyo, 1, 5 ether);
        _cap(yoyo, CAP);
        uint256 aid = _buy(yoyo, _ids(1), 1 ether);
        yoyo.relist(aid);
        bytes32 h1 = yoyo.listing(aid).orderHash;
        Clock.skip(TTL);
        yoyo.relist(aid);
        bytes32 h2 = yoyo.listing(aid).orderHash;
        assertTrue(h1 != h2, "a new salt, a new order");
        assertEq(yoyo.listing(aid).relistCount, 2);
        assertEq(yoyo.listing(aid).ask, 1.2 ether, "the same ask");
    }

    function test_yoyoRelistAndSettleRefusedOnOtherPolicies_byAbi() public {
        // there is no relist/settle on a raffle vault at all: the selector does not exist
        (bool ok,) = address(raffle).call(abi.encodeWithSignature("relist(uint256)", 0));
        assertFalse(ok);
        (ok,) = address(raffle).call(abi.encodeWithSignature("settle(uint256)", 0));
        assertFalse(ok);
    }

    function test_yoyoSettleRewardComesFromProceedsNotBudget() public {
        _fundNative(yoyo, 1, 5 ether);
        _cap(yoyo, CAP);
        uint256 aid = _buy(yoyo, _ids(1), 1 ether);
        yoyo.relist(aid);
        uint256 budget = yoyo.attributed();
        address buyer = makeAddr("buyer");
        vm.deal(buyer, 2 ether);
        bytes32 h = yoyo.listing(aid).orderHash;
        vm.prank(buyer);
        seaport.fill{value: 1.2 ether}(h);
        yoyo.settle(aid);
        assertEq(yoyo.attributed(), budget);
    }

    // ---- Burn --------------------------------------------------------------------------------------------------------------------------

    function test_burnVault_anyoneBurnsWhatItBought() public {
        _fundNative(burnVault, 1, 2 ether);
        _cap(burnVault, CAP);
        uint256 aid = _buy(burnVault, _ids(1), 1 ether);
        vm.prank(stranger);
        burnVault.burn(aid);
        assertEq(nft.ownerOf(1), burnVault.DEAD());
        vm.expectRevert(HomeVaultBase.WrongStage.selector);
        burnVault.burn(aid);
        HomeGuard.Cap memory k = guard.capOf(address(nft)); uint256 items = k.items; uint256 value = k.valueWei;
        assertEq(items, 0);
        assertEq(value, 0);
    }

    // ---- Raffle ----------------------------------------------------------------------------------------------------------------------

    function test_raffle_theWinnerGetsTheNftInTheClaim() public {
        _fundNative(raffle, 1, 2 ether);
        _cap(raffle, CAP);
        uint256 aid = _buy(raffle, _ids(1), 1 ether);
        address who = _raffle(raffle, aid, keccak256("r"));
        assertEq(nft.ownerOf(1), who);
        assertEq(uint8(raffle.acquisition(aid).stage), uint8(HomeVaultBase.Stage.Delivered));
        assertEq(raffle.draw(aid).winner, who);
    }

    function test_raffle_punks() public {
        _fundNative(punkRaffle, 1, 5 ether);
        _cap(punkRaffle, CAP);
        uint256 aid = _buyPunk(punkRaffle, 7, 2 ether);
        address who = _raffle(punkRaffle, aid, keccak256("r"));
        assertEq(punks.punkIndexToAddress(7), who);
    }

    function test_raffle_onlyTheProvenWinnerAndOnlyOnce() public {
        _fundNative(raffle, 1, 2 ether);
        _cap(raffle, CAP);
        uint256 aid = _buy(raffle, _ids(1), 1 ether);
        bytes32[] memory leaves = _tickets(raffle);
        vm.prank(keeper);
        raffle.openDraw(aid, Merkle.root(leaves), 100, "ipfs://tickets");
        vm.expectRevert(abi.encodeWithSelector(HomeVaultRaffle.ChallengeWindow.selector, uint64(block.timestamp + 15 minutes)));
        raffle.requestDraw(aid);
        Clock.skip(15 minutes);
        raffle.requestDraw(aid);
        raffle.resolve(aid, abi.encode(keccak256("r")));
        (address who, uint256 first, uint256 end, uint256 index) = _holderOf(raffle.draw(aid).winningTicket);
        // a stranger claiming for themselves with the winner's range: bad proof
        vm.expectRevert(HomeVaultRaffle.BadProof.selector);
        raffle.claimWin(aid, stranger, first, end, Merkle.proof(leaves, index));
        raffle.claimWin(aid, who, first, end, Merkle.proof(leaves, index));
        vm.expectRevert(HomeVaultBase.WrongStage.selector);
        raffle.claimWin(aid, who, first, end, Merkle.proof(leaves, index));
    }

    function test_raffle_ownerCancelsOnlyInsideTheWindow_andVoidsAfterSevenDays() public {
        _fundNative(raffle, 1, 2 ether);
        _cap(raffle, CAP);
        uint256 aid = _buy(raffle, _ids(1), 1 ether);
        bytes32[] memory leaves = _tickets(raffle);
        vm.prank(keeper);
        raffle.openDraw(aid, Merkle.root(leaves), 100, "ipfs://tickets");
        vm.prank(timelock);
        raffle.cancelDraw(aid);
        assertEq(uint8(raffle.acquisition(aid).stage), uint8(HomeVaultBase.Stage.Held));
        vm.prank(keeper);
        raffle.openDraw(aid, Merkle.root(leaves), 100, "ipfs://tickets");
        Clock.skip(15 minutes);
        vm.prank(timelock);
        vm.expectRevert(HomeVaultRaffle.WindowOver.selector);
        raffle.cancelDraw(aid);
        raffle.requestDraw(aid);
        raffle.resolve(aid, abi.encode(keccak256("r")));
        vm.prank(timelock);
        vm.expectRevert(HomeVaultBase.TooEarly.selector);
        raffle.voidDraw(aid);
        Clock.skip(7 days);
        vm.prank(timelock);
        raffle.voidDraw(aid);
        assertEq(uint8(raffle.acquisition(aid).stage), uint8(HomeVaultBase.Stage.Held));
    }

    function test_raffle_redrawAfterADayIfDrandSkipped() public {
        _fundNative(raffle, 1, 2 ether);
        _cap(raffle, CAP);
        uint256 aid = _buy(raffle, _ids(1), 1 ether);
        bytes32 root = Merkle.root(_tickets(raffle));
        vm.prank(keeper);
        raffle.openDraw(aid, root, 100, "ipfs://tickets");
        Clock.skip(15 minutes);
        raffle.requestDraw(aid);
        uint64 r1 = raffle.draw(aid).round;
        vm.expectRevert(HomeVaultBase.TooEarly.selector);
        raffle.redraw(aid);
        Clock.skip(24 hours);
        raffle.redraw(aid);
        assertGt(raffle.draw(aid).round, r1);
    }

    // ---- Wrap --------------------------------------------------------------------------------------------------------------------------

    function test_wrapVault_wrapsAndBridgesTheWxDownWithAReport() public {
        _fundNative(wrapVault, 1, 3 ether);
        _cap(wrapVault, CAP);
        uint256 extra = wrapVault.bridgeFee(0) + wrapVault.bridgeFee(68);
        uint256 aid = _buy(wrapVault, _ids(1), 1 ether);
        assertEq(nft.ownerOf(1), address(wNft), "the wrapper holds it");
        assertEq(wNft.held(), 1);
        assertEq(wNft.balanceOf(address(wrapVault)), 0, "the wX left through the gateway");
        assertEq(wNft.balanceOf(address(gateway)), 1e18);
        MockGatewayRouter.Deposit memory d = gateway.last();
        assertEq(d.token, address(wNft));
        assertEq(d.to, receiver);
        assertEq(d.amount, 1e18);
        MockInbox.Retryable memory r = inbox.last();
        assertEq(r.to, receiver);
        assertEq(r.data, abi.encodeCall(IHomeReceiverCredit.creditWrapped, (1, 1e18)));
        assertEq(uint8(wrapVault.acquisition(aid).stage), uint8(HomeVaultBase.Stage.Wrapped));
        assertEq(wrapVault.attributed(), 3 ether - 1 ether - REWARD - extra, "price, reward and both bridge fees charged");
        assertEq(address(wrapVault).balance, wrapVault.attributed());
        HomeGuard.Cap memory k = guard.capOf(address(nft)); uint256 items = k.items; uint256 value = k.valueWei;
        assertEq(items, 1, "still one item in custody (the wrapper's)");
        assertEq(value, 0, "its value released");
    }

    function test_wrapVault_punks() public {
        _fundNative(punkWrap, 1, 5 ether);
        _cap(punkWrap, CAP);
        _buyPunk(punkWrap, 7, 2 ether);
        assertEq(punks.punkIndexToAddress(7), address(wPunk));
        assertEq(wPunk.held(), 1);
        assertEq(wPunk.balanceOf(address(gateway)), 1e18);
    }

    function test_wrapVault_budgetMustCoverTheBridge() public {
        _fundNative(wrapVault, 1, 1 ether + REWARD + BUFFER);
        _cap(wrapVault, CAP);
        _ask(_ids(1), 1 ether);
        uint256 extra = wrapVault.bridgeFee(0) + wrapVault.bridgeFee(68);
        vm.prank(trigger, trigger);
        vm.expectRevert(abi.encodeWithSelector(HomeVaultBase.Insufficient.selector, 1 ether + extra + REWARD + BUFFER, 1 ether + REWARD + BUFFER));
        wrapVault.buySeaport(_buyCall(_ids(1), address(wrapVault)), _ids(1), 1);
    }

    // ---- the receipt back to 4663 ----------------------------------------------------------------------------------------------------

    function test_acknowledgeSendsConfirmToTheEscrowOnce() public {
        vm.expectRevert(HomeVaultBase.NothingToAcknowledge.selector);
        raffle.acknowledge(1);
        _fundNative(raffle, 1, 1 ether);
        _fundFast(raffle, 1, 0.5 ether);
        uint256 need = inbox.calculateRetryableSubmissionFee(68, block.basefee) + raffle.L2_GAS_LIMIT() * raffle.L2_MAX_FEE();
        vm.expectRevert(abi.encodeWithSelector(HomeVaultBase.Insufficient.selector, need, need - 1));
        raffle.acknowledge{value: need - 1}(1);
        raffle.acknowledge{value: need}(1);
        MockInbox.Retryable memory r = inbox.last();
        assertEq(r.to, escrow);
        assertEq(r.l2CallValue, 0);
        assertEq(r.data, abi.encodeCall(IFeeEscrowConfirm.confirm, (1, 1.5 ether)));
        assertEq(r.callValueRefund, receiver);
        vm.expectRevert(HomeVaultBase.NothingToAcknowledge.selector);
        raffle.acknowledge{value: need}(1);
        assertEq(raffle.attributed(), 1.5 ether, "an ack costs the budget nothing");
    }

    // ---- attribution -----------------------------------------------------------------------------------------------------------------

    function test_attributionFifoAndByMostPaid() public {
        _fundNative(raffle, 1, 0.6 ether);
        _fundNative(raffle, 2, 2 ether);
        _cap(raffle, CAP);
        uint256 aid = _buy(raffle, _ids(1), 1 ether);
        // ticket 1 paid 0.6, ticket 2 paid 0.4 + reward: attributed to ticket 1
        assertEq(raffle.acquisition(aid).fundedBy, 1);
        assertEq(raffle.unspentOf(1), 0);
        assertEq(raffle.unspentOf(2), 2 ether - 0.4 ether - REWARD);
        assertEq(raffle.queueHead(), 1);
    }

    // ---- fuzz: the caller's net gain is the reward, the ledger never exceeds the balance -------------------------------------------

    function testFuzz_callerGainsExactlyTheRewardPerTrigger(uint96 fundWei, uint96 priceWei, uint8 count) public {
        uint256 n = bound(count, 1, 5);
        uint256 price = bound(priceWei, 0.01 ether, 5 ether);
        uint256 f = bound(fundWei, n * (price + REWARD) + BUFFER, 40 ether);
        _fundNative(raffle, 1, f);
        _cap(raffle, CAP);
        uint256 before = trigger.balance;
        for (uint256 i = 1; i <= n; ++i) _buy(raffle, _ids(i), price);
        assertEq(trigger.balance - before, n * REWARD);
        assertEq(raffle.attributed() + raffle.waiting(), f - n * (price + REWARD));
        assertEq(address(raffle).balance, raffle.attributed() + raffle.waiting() + raffle.proceedsHeld());
    }

    function testFuzz_markupWithinBounds(uint16 bps) public {
        uint256 m = bound(bps, 11_000, 12_000);
        HomeVaultYoyo v = new HomeVaultYoyo(_config(address(punks), true), m, TTL);
        vm.prank(timelock);
        guard.register(address(punks), address(v));
        _fundNative(v, 1, 5 ether);
        _cap(v, CAP);
        uint256 aid = _buyPunk(v, 3, 1 ether);
        uint256 ask = v.listing(aid).ask;
        assertGe(ask, 1.1 ether);
        assertLe(ask, 1.2 ether);
        assertEq(ask, 1 ether * m / 10_000);
    }
}
