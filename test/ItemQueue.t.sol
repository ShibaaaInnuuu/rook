// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {QueueBase, MockCurve} from "./utils/QueueBase.sol";
import {Clock} from "./utils/Clock.sol";
import {ItemQueue} from "../src/curve/ItemQueue.sol";

/// docs/v3/pools.md §5: the per-collection buy queue.
contract ItemQueueTest is QueueBase {
    function _e(MockCurve c) internal view returns (ItemQueue.Entry memory) { return queue.entry(address(c)); }

    function test_theCanistersViewOfTheQueue() public {
        assertEq(address(queue.wrapper()), address(wrapper));
        assertEq(queue.payee(), payee);
        (uint64 at, uint64 atBlock) = queue.headDeadline();
        assertEq(at, 0);
        assertEq(atBlock, 0, "an empty queue has no deadline");
        assertEq(queue.itemCap(), CAP);
        assertEq(queue.buffer(), CAP * 500 / 10_000);
        assertFalse(queue.canBuy());
        MockCurve a = _coin(4.12 ether);
        (at, atBlock) = queue.headDeadline();
        assertEq(at, block.timestamp + 3 days);
        assertTrue(queue.canBuy());
        assertEq(queue.queued(), 4.12 ether);
        _item(0.64 ether);
        assertEq(queue.itemsRecorded(), 1);
        assertEq(_e(a).units, 1e18);
        Clock.skip(2 hours);
        assertEq(queue.itemCap(), 0, "a stale cap is no cap");
    }

    /// Two coins at 5 ETH on a Bored Ape collection (docs/v3/curve.md): 4.12 ETH each queued; one ape at 5.1 ETH is funded by
    /// the first coin's 4.12 and 0.98 of the second's, so the first gets 4.12/5.1 of it and is done; the second carries 3.14.
    function test_theApeExample() public {
        _postCap(raffle, 5.2 ether);
        MockCurve a = _coin(4.12 ether);
        MockCurve b = _coin(4.12 ether);
        _item(5.1 ether);
        assertEq(uint8(_e(a).status), uint8(ItemQueue.Status.Filled));
        assertEq(_e(a).units, uint256(1e18) * 4.12 ether / 5.1 ether);
        assertEq(_e(b).units, 1e18 - _e(a).units, "the last contributor takes the rounding: exactly one unit assigned");
        assertEq(_e(b).remaining, 4.12 ether - 0.98 ether, "3.14 ETH carried, still the second coin's");
        assertEq(uint8(_e(b).status), uint8(ItemQueue.Status.Queued));
        assertEq(queue.queued(), 3.14 ether);
        assertEq(payee.balance, 5.1 ether);
        _books();
    }

    function test_aCheapItemsCoinIsFilledAtItsTail() public {
        MockCurve a = _coin(4.12 ether);
        for (uint256 i = 0; i < 5; ++i) _item(0.64 ether);
        assertEq(uint8(_e(a).status), uint8(ItemQueue.Status.Queued), "0.92 left: more than the 0.412 tail");
        _item(0.64 ether);
        assertEq(uint8(_e(a).status), uint8(ItemQueue.Status.Filled), "0.28 left: under the tail");
        assertEq(queue.heldDust(), 0.28 ether);
        assertEq(queue.queued(), 0, "a tail is held for its coin, not spent on anyone's item");
        (uint256 units, uint256 spent, uint256 tail) = a.release();
        assertEq(units, 6e18);
        assertEq(spent, 3.84 ether);
        assertEq(tail, 0.28 ether);
        assertEq(address(a).balance, 0.28 ether);
        assertEq(wrapper.balanceOf(address(a)), 6e18);
        _books();
    }

    function test_fifoALaterCoinIsNeverSpentBeforeAnEarlierOne() public {
        _postCap(raffle, 5.2 ether);
        MockCurve a = _coin(4 ether);
        MockCurve b = _coin(4 ether);
        MockCurve c = _coin(4 ether);
        _item(5 ether);                                          // a 4, b 1
        _item(5 ether);                                          // b 3, c 2
        assertEq(_e(a).spent, 4 ether);
        assertEq(_e(b).spent, 4 ether);
        assertEq(_e(c).spent, 2 ether);
        assertEq(uint8(_e(c).status), uint8(ItemQueue.Status.Queued));
        assertEq(queue.head(), 2);
        _books();
    }

    function test_onlyTheCustodianRecordsOnlyWhatIsHereAndOnlyUpToTheCap() public {
        _coin(4.12 ether);
        vm.prank(keeper);
        vm.expectRevert(ItemQueue.NotCustodian.selector);
        queue.recordItem(0.64 ether);
        vm.prank(custodian);
        vm.expectRevert(ItemQueue.NoWrapped.selector);
        queue.recordItem(0.64 ether);
        _deposit(address(queue));
        vm.prank(custodian);
        vm.expectRevert(abi.encodeWithSelector(ItemQueue.OverCap.selector, CAP + 1, CAP));
        queue.recordItem(CAP + 1);
        vm.prank(custodian);
        vm.expectRevert(ItemQueue.NotEnough.selector);
        queue.recordItem(4.12 ether + 1);
        vm.prank(custodian);
        queue.recordItem(CAP);
    }

    function test_aCapDropBetweenMintAndRecordDoesNotStrandTheUnit() public {
        _coin(4.12 ether);
        queue.snapshotCap();                                     // the canister, before it buys
        _deposit(address(queue));
        _postCap(raffle, 0.5 ether);                             // the keeper lowers the cap mid-purchase
        assertEq(queue.itemCap(), 0.5 ether);
        assertEq(queue.maxItemCost(), CAP, "the snapshot still bounds it");
        vm.prank(custodian);
        queue.recordItem(0.64 ether);
        Clock.skip(3 hours);
        _postCap(raffle, 0.5 ether);
        assertEq(queue.maxItemCost(), 0.5 ether, "and only for its TTL");
    }

    function test_refundExpiredCancelsTheHeadOnceItsDeadlinePassesOnBothClocks() public {
        MockCurve a = _coin(4.12 ether);
        MockCurve b = _coin(4.12 ether);
        _item(0.64 ether);
        vm.expectRevert(ItemQueue.NotExpired.selector);
        queue.refundExpired();
        Clock.skipTimeOnly(4 days);
        vm.expectRevert(ItemQueue.NotExpired.selector);
        queue.refundExpired();
        Clock.skip(3 days);
        queue.refundExpired();
        assertEq(uint8(_e(a).status), uint8(ItemQueue.Status.Cancelled));
        assertEq(address(a).balance, 4.12 ether - 0.64 ether);
        assertEq(wrapper.balanceOf(address(a)), 1e18, "the units already bought go back with it");
        assertEq(queue.head(), 1, "the next coin is the head");
        queue.refundExpired();
        assertEq(uint8(_e(b).status), uint8(ItemQueue.Status.Cancelled));
        vm.expectRevert(ItemQueue.Empty.selector);
        queue.refundExpired();
        _books();
    }

    function test_aUnitMintedForACoinThatRefundedFirstIsReclaimedNotLost() public {
        MockCurve a = _coin(1 ether);
        Clock.skip(3 days + 1);
        _deposit(address(queue));                                // the canister's mint landed...
        a.openRefunds();                                         // ...after the refund
        vm.prank(custodian);
        vm.expectRevert(ItemQueue.NotEnough.selector);
        queue.recordItem(0.64 ether);
        vm.prank(stranger);
        vm.expectRevert(ItemQueue.NotCustodian.selector);
        queue.reclaimStray();
        vm.prank(custodian);
        assertEq(queue.reclaimStray(), 1e18);
        assertEq(wrapper.balanceOf(custodian), 1e18);
    }

    function test_reclaimStrayNeverTakesUnitsOwedToCoins() public {
        _coin(4.12 ether);
        _item(0.64 ether);
        vm.prank(custodian);
        assertEq(queue.reclaimStray(), 0);
        assertEq(wrapper.balanceOf(address(queue)), 1e18);
    }

    function test_onlyACoinsOwnCurveQueuesReleasesOrCancels() public {
        MockCurve a = _coin(1 ether);
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        vm.expectRevert(ItemQueue.NotCurve.selector);
        queue.enqueue{value: 1 ether}(address(a), 0, 0);
        vm.prank(stranger);
        vm.expectRevert(ItemQueue.NotCurve.selector);
        queue.cancel(address(a));
        vm.expectRevert(ItemQueue.BadInput.selector);
        a.enqueue{value: 0}(0, 0);
        vm.expectRevert(ItemQueue.WrongStatus.selector);
        a.release();
    }

    function testFuzz_unitsAreExactlyOnePerItemAndFifoHolds(uint96[5] memory budgets, uint96[8] memory costs) public {
        _postCap(raffle, 3 ether);
        MockCurve[5] memory cs;
        for (uint256 i = 0; i < 5; ++i) cs[i] = _coin(bound(budgets[i], 0.2 ether, 6 ether));
        uint256 items;
        for (uint256 i = 0; i < 8; ++i) {
            uint256 cost = bound(costs[i], 0.1 ether, 3 ether);
            if (cost > queue.queued()) continue;
            _item(cost);
            ++items;
        }
        uint256 assigned;
        bool open;
        for (uint256 i = 0; i < 5; ++i) {
            ItemQueue.Entry memory e = queue.entry(address(cs[i]));
            assigned += e.units;
            if (e.spent != 0) assertFalse(open, "a later coin was spent while an earlier one still queued");
            if (e.status == ItemQueue.Status.Queued) open = true;
            assertEq(e.spent + e.remaining, e.deposited);
        }
        assertEq(assigned, items * 1e18, "units assigned == items x 1e18, exactly");
        assertEq(queue.itemsRecorded(), items);
        _books();
    }
}
