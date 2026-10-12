// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {FloorBase} from "./FloorBase.sol";
import {Merkle} from "../utils/Merkle.sol";
import {Wait} from "../../src/lib/Wait.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {IKeeperSource} from "../../src/interfaces/IKeeperSource.sol";
import {Wrapper} from "../../src/Wrapper.sol";
import {IFloorWrapped} from "../../src/interfaces/IFloorWrapped.sol";

/// docs/v5/floor-module.md §4–§7: the floor, the spread, mint and burn, the caps, conversions, and the wrapper's side.
contract FloorModuleTest is FloorBase {
    // ---- the floor: posting and the move limit (§5) ----

    function test_theFirstPostIsTheAnchor() public view {
        (uint256 acc, uint64 at, uint64 ttl) = module.floor();
        assertEq(acc, F);
        assertEq(module.target(), F);
        assertEq(at, block.timestamp);
        assertEq(ttl, module.FLOOR_TTL());
        assertEq(ttl, 3 hours, "a post prices for three hours");
        assertTrue(module.fresh());
        assertEq(module.volBps(), 0, "no movement yet");
    }

    function test_onlyTheKeeperPosts_andNeverZero() public {
        vm.prank(stranger);
        vm.expectRevert(FloorModule.NotKeeper.selector);
        module.postFloor(F, 10_000, 1);
        vm.prank(keeper);
        vm.expectRevert(FloorModule.BadInput.selector);
        module.postFloor(0, 10_000, 1);
        vm.prank(keeper);
        vm.expectRevert(FloorModule.BadInput.selector);
        module.postFloor(F, 10_001, 1);
        vm.prank(stranger);
        vm.expectRevert(FloorModule.NotKeeper.selector);
        module.postRentalRoot(bytes32(uint256(1)));
    }

    function test_theFloorMovesAtMostFivePercentAnHour() public {
        _skip(1 hours);
        _post(F * 2);
        assertEq(module.accepted(), F * 10_500 / 10_000, "+5% after an hour, whatever the keeper posts");
        assertEq(module.target(), F * 2, "the keeper's figure is kept beside it");
        _skip(1 hours);
        _post(F / 2);
        uint256 a = F * 10_500 / 10_000;
        assertEq(module.accepted(), a - a * 500 / 10_000, "-5% of the accepted floor the next hour");
    }

    function test_aHalfHourMovesHalfAsFar() public {
        _skip(30 minutes);
        _post(F * 2);
        assertApproxEqAbs(module.accepted(), F + F * 250 / 10_000, 1, "2.5% after half an hour");
    }

    function test_aSilentKeeperStillMovesOnlyOneHoursWorth() public {
        _skip(5 hours);
        _post(F * 2);
        assertEq(module.accepted(), F * 10_500 / 10_000, "five silent hours are worth one");
    }

    function test_theSequencerAloneCannotBuyABiggerStep() public {
        _skipTimeOnly(1 hours);   // the timestamp jumps, the parent chain does not
        _post(F * 2);
        assertEq(module.accepted(), F, "no parent blocks passed: no step at all");
    }

    function test_postingEveryFiveMinutesCompoundsToAtMostFivePointOneThreePercent() public {
        for (uint256 i = 0; i < 12; ++i) {
            _skip(5 minutes);
            _post(F * 2);
        }
        assertLe(module.accepted(), F * 10_513 / 10_000, "e^0.05 - 1 at most");
        assertGe(module.accepted(), F * 10_500 / 10_000, "and at least the plain hour");
    }

    function testFuzz_everyPostIsWithinTheLimit(uint256[8] memory targets, uint16[8] memory gaps) public {
        for (uint256 i = 0; i < 8; ++i) {
            uint256 gap = bound(gaps[i], 1, 3 hours);
            _skip(gap);
            uint256 prev = module.accepted();
            uint256 t = bound(targets[i], 1, 100 ether);
            (uint256 lo, uint256 hi) = module.floorBounds();
            _post(t);
            uint256 next = module.accepted();
            uint256 el = gap > 1 hours ? 1 hours : gap;
            uint256 step = prev * 500 * el / (10_000 * 1 hours);
            assertLe(next > prev ? next - prev : prev - next, step, "never beyond maxMove x elapsed");
            // and within ±20% of the day's anchor (B-02: a leaked keeper cannot walk the floor 5%/h for ever)
            uint256 anchor = module.dayAnchor();
            assertLe(next, anchor * 12_000 / 10_000, "never above the day's band");
            assertGe(next, anchor * 8_000 / 10_000, "never below the day's band");
            if (t >= lo && t <= hi) assertEq(next, t, "within the limits, the keeper's figure is taken");
        }
    }

    function test_aPostGoesStaleAfterThreeHoursOnEitherClock() public {
        uint256 ttl = module.FLOOR_TTL();
        _skip(ttl - 13);
        assertTrue(module.fresh());
        _skip(13);
        assertFalse(module.fresh());
        _post(F);
        _skipTimeOnly(ttl);
        assertFalse(module.fresh(), "a timestamp a TTL ahead makes it stale, never fresh");
        vm.deal(alice, 1 ether);
        vm.prank(alice);
        vm.expectRevert(FloorModule.Stale.selector);
        module.mint{value: 1 ether}(alice, 0);
        vm.prank(alice);
        vm.expectRevert(FloorModule.Stale.selector);
        module.burn(1, 0, alice);
    }

    // ---- the spread (§6.1) ----

    function test_aCalmMarketPaysTheBaseSpread() public {
        (uint256 sMint, uint256 sBurn) = module.spreads();
        assertEq(sMint, 15);
        assertEq(sBurn, 15);
        assertEq(module.mintPrice(), 0.6339495 ether);
        assertEq(module.burnPrice(), 0, "nothing ETH-backed yet: nothing to pay a burn with");
        _mint(alice, 1 ether);
        assertEq(module.burnPrice(), 0.6320505 ether, "F x (1 - 0.15%)");
    }

    function test_movementWidensBothSidesAndDecaysOverAnHour() public {
        _skip(1 hours);
        _post(F * 2);                                         // +5%: vol 500 bps
        (uint256 sMint, uint256 sBurn) = module.spreads();
        assertEq(sBurn, 15 + 500);
        assertEq(sMint, 15 + 500);
        _skip(30 minutes);
        _post(module.accepted());                             // no move: vol decays to half
        (, sBurn) = module.spreads();
        assertApproxEqAbs(sBurn, 15 + 250, 1);
        _skip(1 hours);
        _post(module.accepted());
        (, sBurn) = module.spreads();
        assertEq(sBurn, 15, "an hour later, calm again");
    }

    function test_stalenessWidensFromTenMinutesToThreeHours() public {
        uint256 grace = module.STALE_GRACE();
        uint256 ttl = module.FLOOR_TTL();
        uint256 mid = (grace + ttl) / 2;                      // 95 minutes: halfway along the ramp
        _skip(grace - 13);
        (, uint256 s) = module.spreads();
        assertEq(s, 15, "under ten minutes: nothing");
        _skip(mid - grace);
        (, s) = module.spreads();
        assertApproxEqAbs(s, 15 + 50, 1, "95 minutes: half of the +1%");
        _skip(ttl - mid - 1 minutes);
        (, s) = module.spreads();
        assertApproxEqAbs(s, 15 + 100, 1, "at three hours: the whole +1%");
    }

    function test_stressWidensMintingOnly() public {
        _mint(alice, 5 * 0.6339495 ether);                    // ≈ 5 units: half of the public cap
        (uint256 sMint, uint256 sBurn) = module.spreads();
        assertApproxEqAbs(sMint, 15 + 150, 1, "+3% x E / maxPublic");
        assertEq(sBurn, 15, "burning reduces E: never taxed for it");
    }

    function test_theSpreadNeverExceedsTheMaximum() public {
        for (uint256 i = 0; i < 6; ++i) {                     // alternate ±5% every hour: movement piles up
            _skip(1 hours);
            _post(i % 2 == 0 ? F * 2 : F / 2);
        }
        _skip(50 minutes);                               // and let it go stale too
        (uint256 sMint, uint256 sBurn) = module.spreads();
        assertLe(sMint, 1_000);
        assertLe(sBurn, 1_000);
    }

    function test_theLagIsAlwaysPricedAgainstTheTrader() public {
        _mint(alice, 1 ether);
        _calm();
        _skip(1 hours);
        _post(F * 12 / 10);                                   // the real floor jumped 20%; accepted moves 5%
        uint256 acc = module.accepted();
        (uint256 sMint, uint256 sBurn) = module.spreads();
        assertEq(module.mintPrice(), _ceil(F * 12 / 10 * (10_000 + sMint), 10_000), "mint at the keeper's (higher) figure");
        uint256 b = acc * (10_000 - sBurn) / 10_000;
        uint256 beta = module.beta();
        assertEq(module.burnPrice(), b < beta ? b : beta, "burn at the accepted (lower) one, capped at the backing");
        _skip(1 hours);
        _post(F * 8 / 10);                                    // now it fell 20% below
        (sMint, sBurn) = module.spreads();
        assertEq(module.mintPrice(), _ceil(module.accepted() * (10_000 + sMint), 10_000), "mint at the accepted (higher) floor");
        // burn at the keeper's (lower) figure, but never below what the accepted floor may legally reach within the hour
        // (review2 N-1a): a post further down than that says nothing the module would act on
        uint256 lo = _ceil(module.accepted() * 9_500, 10_000);
        assertGt(lo, F * 8 / 10);
        assertEq(module.burnPrice(), lo * (10_000 - sBurn) / 10_000, "burn at the lowest legal floor within the hour");
    }

    // ---- confidence, depth and the bounds the keeper reads (docs/v5/floor-feed.md) ----

    function test_lowConfidenceWidensBothSides() public {
        conf = 4_900;                                          // the real frog feed's confidence
        _post(F);
        (uint256 sMint, uint256 sBurn) = module.spreads();
        assertEq(sBurn, 15 + 39, "+1% x (8,000 - 4,900) / 8,000, rounded up");
        assertEq(sMint, 15 + 39);
        conf = 8_746;                                          // BAYC's
        _post(F);
        (, sBurn) = module.spreads();
        assertEq(sBurn, 15, "at or above 80%: nothing added");
    }

    function test_aPostUnderTheMinimumConfidenceLeavesTheModuleStale() public {
        conf = 2_999;
        _post(F);
        assertFalse(module.fresh());
        vm.deal(alice, 1 ether);
        vm.prank(alice);
        vm.expectRevert(FloorModule.Stale.selector);
        module.mint{value: 1 ether}(alice, 0);
        conf = 3_000;
        _post(F);
        assertTrue(module.fresh());
    }

    function test_theBooksDepthBoundsPublicMinting() public {
        depth = 1;                                             // the real frog book: one listing within +10%
        _post(F);
        assertEq(module.publicCap(), 1e18);
        (, uint256 maxIn) = module.mintCapacity();
        assertEq(maxIn, module.mintPrice(), "one item's worth, whatever the constant cap says");
        uint256 p = module.mintPrice();
        vm.deal(alice, 2 * p);
        vm.prank(alice);
        vm.expectRevert(FloorModule.OverBacked.selector);
        module.mint{value: 2 * p}(alice, 0);
        depth = 0;
        _post(F);
        (, maxIn) = module.mintCapacity();
        assertEq(maxIn, 0, "no book: no public minting");
    }

    function test_floorBoundsIsExactlyWhatAPostCanBecome() public {
        _skip(30 minutes);
        (uint256 lo, uint256 hi) = module.floorBounds();
        assertEq(hi - F, F - lo);
        assertApproxEqAbs(hi, F + F * 250 / 10_000, 1e12, "plus or minus 2.5% after half an hour (plus one parent block)");
        _post(F * 3);
        assertEq(module.accepted(), hi, "outside the bounds: clamped to them, not refused");
        assertEq(module.target(), F * 3, "and the keeper's figure is kept for pricing the lag");
        assertEq(module.maxMoveBps(), 500);
    }

    function test_itemArrivalsAreLimitedPerDay_onBothClocks() public {
        FloorModule.Params memory p = _params();
        p.maxItemsPerDay = 2;
        _redeploy(p);
        _mint(alice, 3 ether);
        _convert(module.beta());
        _convert(module.beta());
        assertFalse(module.arrivalRoom());
        assertEq(module.maxItemCost(), 0, "the canister reads that it cannot be paid today");
        _deposit(address(module));
        uint256 beta = module.beta();
        vm.prank(custodian);
        vm.expectRevert(FloorModule.ArrivalRateLimited.selector);
        module.itemArrived(beta);
        _skipTimeOnly(1 days);                                 // the sequencer alone: not a day
        _post(F);
        vm.prank(custodian);
        vm.expectRevert(FloorModule.ArrivalRateLimited.selector);
        module.itemArrived(beta);
        _skip(1 days);
        _post(F);
        vm.prank(custodian);
        module.itemArrived(beta);
        _books();
    }

    // ---- mint and burn ----

    function test_mintIsInstantAndETHBacked() public {
        uint256 units = _mint(alice, 1 ether);
        assertEq(units, uint256(1e36) / 0.6339495 ether);
        assertEq(wrapper.balanceOf(alice), units);
        assertEq(wrapper.ethBacked(), units);
        assertEq(module.backing(), 1 ether, "every wei paid backs the units");
        assertEq(wrapper.held(), 0, "no item yet");
        _books();
    }

    function test_burnPaysTheFloorLessTheSpread_andKeepsTheDifference() public {
        uint256 units = _mint(alice, 2 ether);
        _calm();
        uint256 beta = module.beta();
        uint256 eth = _burn(alice, 1e18);
        assertEq(eth, module.accepted() * 9_985 / 10_000, "F x (1 - 0.15%)");
        assertEq(module.backing(), 2 ether - beta, "one unit's backing released");
        assertEq(module.revenue(), beta - eth, "the rest is revenue");
        assertGe(module.beta(), beta, "the backing per unit did not fall");
        assertEq(wrapper.ethBacked(), units - 1e18);
        _books();
    }

    function test_theBurnPriceNeverExceedsTheBackingPerUnit() public {
        _mint(alice, 3 ether);
        uint256 beta = module.beta();
        _walkTo(F * 13 / 10);                                 // the floor really rose 30%
        _calm();
        assertEq(module.burnPrice(), beta, "capped at what the units paid in");
        uint256 eth = _burn(alice, 1e18);
        assertEq(eth, beta);
        assertEq(module.revenue(), 0, "nothing left over: a burn at the backing");
        _books();
    }

    function test_theLastBurnTakesTheRoundingDust() public {
        uint256 units = _mint(alice, 1 ether + 7);
        _burn(alice, units);
        assertEq(wrapper.ethBacked(), 0);
        assertEq(module.backing(), 0, "no dust left behind in the backing");
        _books();
    }

    function test_mintsAreCappedPerHour_onTheSlowerClock() public {
        uint256 p = module.mintPrice();
        _mint(alice, 4 * p);
        vm.deal(alice, 10 ether);
        vm.prank(alice);
        vm.expectRevert(FloorModule.OverCap.selector);
        module.mint{value: 2 * p}(alice, 0);                  // 6 > 5 an hour
        _skipTimeOnly(1 hours);
        _post(F);                                             // fresh again, but the parent chain has not moved
        vm.prank(alice);
        vm.expectRevert(FloorModule.OverCap.selector);
        module.mint{value: 2 * p}(alice, 0);
        _skip(30 minutes);                               // half an hour on both clocks: 2.5 units drained
        _post(F);
        (, uint256 room) = module.mintCapacity();
        assertApproxEqRel(room, module.mintPrice() * 35 / 10, 1e16, "5 - 4 + 2.5 units of room (one extra parent block)");
    }

    function test_burnsAreCappedPerHour() public {
        _mint(alice, 5 * module.mintPrice());
        _skip(2 hours);
        _post(F);
        _mint(alice, 5 * module.mintPrice());
        _burn(alice, 5e18);
        vm.startPrank(alice);
        wrapper.approve(address(module), 1e18);
        vm.expectRevert(FloorModule.OverCap.selector);
        module.burn(1e18, 0, alice);
        vm.stopPrank();
    }

    function test_publicMintsStopAtTheETHBackedCap() public {
        for (uint256 i = 0; i < 2; ++i) {
            _mint(alice, 5 * module.mintPrice() - 1);
            _skip(1 hours);
            _post(F);
        }
        uint256 p = module.mintPrice();
        vm.deal(alice, 1 ether);
        vm.prank(alice);
        vm.expectRevert(FloorModule.OverBacked.selector);
        module.mint{value: p}(alice, 0);
        assertLe(wrapper.ethBacked(), 10e18);
    }

    function test_slippageAndBadInputs() public {
        vm.deal(alice, 2 ether);
        vm.prank(alice);
        vm.expectRevert(FloorModule.Slippage.selector);
        module.mint{value: 1 ether}(alice, 2e18);
        vm.prank(alice);
        vm.expectRevert(FloorModule.BadInput.selector);
        module.mint{value: 1 ether}(address(0), 0);
        _mint(alice, 1 ether);
        vm.startPrank(alice);
        wrapper.approve(address(module), 1e18);
        vm.expectRevert(FloorModule.Slippage.selector);
        module.burn(1e18, 1 ether, alice);
        vm.expectRevert(FloorModule.NotEnoughBacked.selector);
        module.burn(2e18, 0, alice);
        vm.stopPrank();
    }

    function test_churnFillsTheReserve() public {
        for (uint256 i = 0; i < 4; ++i) {
            uint256 u = _mint(alice, 1 ether);
            _burn(alice, u);
            _skip(1 hours);
            _post(F);
        }
        assertApproxEqRel(module.revenue(), 4 * (uint256(1 ether) * 30 / 10_015), 1e16, "about 0.3% of each round trip");
        assertEq(wrapper.ethBacked(), 0);
        _books();
    }

    // ---- the canister: the queue's interface (§7) ----

    function test_theSelectorsAreTheKeepersAndTheCanistersOwn() public pure {
        // docs/v5/floor-feed.md §9 (keeper/src/floorpost.ts, autobuyer/canister-rs/src/floorsink.rs)
        assertEq(bytes4(keccak256("floor()")), bytes4(0x40695363));
        assertEq(FloorModule.floor.selector, bytes4(0x40695363));
        assertEq(FloorModule.netUnits.selector, bytes4(0x273c589e));
        assertEq(FloorModule.escrow.selector, bytes4(0xe2fdcc17));
        assertEq(FloorModule.buffer.selector, bytes4(0xedaafe20));
        assertEq(FloorModule.maxItemCost.selector, bytes4(0x9b04dfbf));
        assertEq(bytes4(keccak256("itemsArrived()")), bytes4(0xff40b1af));   // a public variable: its getter's selector
        assertEq(FloorModule.itemArrived.selector, bytes4(0xd996759a));
        assertEq(FloorModule.postFloor.selector, bytes4(keccak256("postFloor(uint256,uint16,uint32)")));
        assertEq(FloorModule.floorBounds.selector, bytes4(keccak256("floorBounds()")));
    }

    function test_theNettingSignal() public {
        assertEq(module.netUnits(), 0);
        assertEq(module.buffer(), 0.05e18, "5% of an item, in units");
        uint256 cap = F * 10_300 / 10_000;
        assertEq(module.itemCap(), cap);
        assertEq(module.maxItemCost(), 0, "nothing to convert yet");
        _mint(alice, 1.04e18 * module.mintPrice() / 1e18);
        assertLt(module.netUnits(), 1e18 + module.buffer(), "under one item plus the buffer: the canister waits");
        _burn(bob, _mint(bob, 0.5e18 * module.mintPrice() / 1e18));
        assertLt(module.netUnits(), 1e18 + module.buffer(), "bob's mint and burn net to nothing");
        _mint(alice, 0.6e18 * module.mintPrice() / 1e18);
        uint256 net = module.netUnits();
        assertEq((net - module.buffer()) / 1e18, 1, "floorsink.rs wanted(): one item");
        assertEq(module.escrow(), module.backing());
        uint256 canPay = module.beta() + module.revenue();
        assertEq(module.maxItemCost(), cap < canPay ? cap : canPay, "never more than one unit's backing plus the reserve");
        _skip(module.FLOOR_TTL());
        assertEq(module.itemCap(), 0, "stale: no cap, nothing to buy");
        assertEq(module.maxItemCost(), 0);
    }

    function test_itemArrivedTurnsOneUnitIntoAnItem() public {
        _mint(alice, 2 ether);
        uint256 e = wrapper.ethBacked();
        uint256 beta = module.beta();
        uint256 supply = wrapper.totalSupply();
        uint256 cost = beta - 0.001 ether;
        _convert(cost);
        assertEq(wrapper.held(), 1);
        assertEq(wrapper.ethBacked(), e - 1e18, "one ETH-backed unit is item-backed now");
        assertEq(wrapper.totalSupply(), supply, "nobody's balance changed");
        assertEq(payee.balance, cost, "the canister is paid from the unit's backing");
        assertEq(module.revenue(), 0.001 ether, "what the item cost less than the backing is revenue");
        assertEq(module.itemsArrived(), 1);
        assertGe(module.beta(), beta);
        _books();
    }

    function test_aConversionAboveTheBackingIsPaidByTheReserve_orWaits() public {
        _mint(alice, 2 ether);
        uint256 beta = module.beta();
        uint256 cost = module.itemCap();                      // ≈ 3% over the floor: more than the backing
        assertGt(cost, beta);
        _deposit(address(module));
        vm.prank(custodian);
        vm.expectRevert(abi.encodeWithSelector(FloorModule.OverBasis.selector, cost - beta, 0));
        module.itemArrived(cost);
        // churn fills the reserve; then it pays
        for (uint256 i = 0; i < 8; ++i) {
            _skip(1 hours);
            _post(F);
            _burn(bob, _mint(bob, 2 ether));
        }
        uint256 r = module.revenue();
        assertGe(r, cost - module.beta());
        uint256 b2 = module.beta();
        vm.prank(custodian);
        module.itemArrived(cost);
        assertEq(module.revenue(), r - (cost - b2));
        _books();
    }

    function test_itemArrivedRefusals() public {
        _mint(alice, 2 ether);
        vm.prank(stranger);
        vm.expectRevert(FloorModule.NotCustodian.selector);
        module.itemArrived(0.5 ether);
        vm.prank(custodian);
        vm.expectRevert(FloorModule.NoWrapped.selector);
        module.itemArrived(0.5 ether);                         // nothing was wrapped here
        _deposit(address(module));
        uint256 max = module.itemCap();
        vm.prank(custodian);
        vm.expectRevert(abi.encodeWithSelector(FloorModule.OverItemCap.selector, max + 1, max));
        module.itemArrived(max + 1);
        uint256 beta = module.beta();
        vm.prank(custodian);
        vm.expectRevert(abi.encodeWithSelector(FloorModule.OverBasis.selector, max - beta, 0));
        module.itemArrived(max);                               // under the cap, over the backing, and no reserve
        vm.prank(custodian);
        vm.expectRevert(FloorModule.BadInput.selector);
        module.itemArrived(0);
    }

    function test_aSnapshotCoversACapDropBetweenPurchaseAndRecord() public {
        _mint(alice, 3 ether);
        uint256 beta = module.beta();
        module.snapshotCap();                                  // the canister, before it buys: cap F x 1.03
        _skip(1 hours);
        _post(F / 2);                                          // the floor falls 5% (the limit) while it buys
        assertLt(module.itemCap(), beta, "the fresh cap is now under what the item cost");
        assertEq(module.maxItemCost(), beta, "the snapshot still bounds it for two hours (and the backing does)");
        _deposit(address(module));
        vm.prank(custodian);
        module.itemArrived(beta);
        _books();
        _skip(2 hours);
        _post(module.accepted());
        _mint(alice, 1 ether);
        assertLt(module.maxItemCost(), beta, "the snapshot has lapsed");
    }

    function test_nothingToConvertWithoutAnETHBackedUnit() public {
        _deposit(address(module));
        vm.prank(custodian);
        vm.expectRevert(FloorModule.NotEnoughBacked.selector);
        module.itemArrived(0.5 ether);
        vm.prank(custodian);
        assertEq(module.reclaimStray(), 1e18, "the canister takes back a unit it could not record");
        assertEq(wrapper.balanceOf(custodian), 1e18);
    }

    // ---- the wrapper's side (§3) ----

    function test_theModuleIsWiredOnce_byTheDeployer_beforeAnythingExists() public {
        Wrapper w2 = new Wrapper(registry, distributor, beacon, FROGS, distribution, custodian, CUSTODY_BTC, "w2", "w2", PBS);
        vm.prank(stranger);
        vm.expectRevert(Wrapper.NotDeployer.selector);
        w2.wireFloorModule(address(module));
        vm.expectRevert(Wrapper.BadInput.selector);
        w2.wireFloorModule(stranger);                         // not a contract
        vm.expectRevert(Wrapper.NotItsModule.selector);
        w2.wireFloorModule(address(module));                  // B-05: a module built for another wrapper
        FloorModule m2 = new FloorModule(IFloorWrapped(address(w2)), IKeeperSource(address(registry)), payee, platform, address(0), _params(), PBS);
        // anything minted first: never
        vm.prank(custodian);
        w2.wrapDeposit(frogIds[63], _proof(63), keccak256("t"), 0, alice);
        vm.expectRevert(Wrapper.NotFresh.selector);
        w2.wireFloorModule(address(m2));
        vm.expectRevert(Wrapper.AlreadyWired.selector);
        wrapper.wireFloorModule(address(module));
        assertEq(wrapper.floorModule(), address(module));
    }

    function test_onlyTheModuleMintsOrBurnsWithoutAnItem() public {
        vm.startPrank(stranger);
        vm.expectRevert(Wrapper.NotModule.selector);
        wrapper.moduleMint(stranger, 1e18);
        vm.expectRevert(Wrapper.NotModule.selector);
        wrapper.moduleBurn(1);
        vm.expectRevert(Wrapper.NotModule.selector);
        wrapper.admitRental("", "");
        vm.expectRevert(Wrapper.NotModule.selector);
        wrapper.returnRental(0);
        vm.stopPrank();
    }

    function test_ETHBackedUnitsCannotRedeemAnItemThatIsNotThere() public {
        _mint(alice, 2 ether);
        vm.prank(alice);
        vm.expectRevert(Wrapper.NoItem.selector);
        wrapper.redeem(TAPROOT);
        _convert(module.beta());                               // the canister brings one
        vm.prank(alice);
        uint256 id = wrapper.redeem(TAPROOT);                  // now one ticket has one item
        vm.prank(alice);
        vm.expectRevert(Wrapper.NoItem.selector);
        wrapper.redeem(TAPROOT);                               // and a second has none
        _resolveTicket(id, bytes32(uint256(7)));
        _books();
    }

    function _proof(uint256 i) internal view returns (bytes32[] memory) {
        return Merkle.proof(frogLeaves, i);
    }

    function _ceil(uint256 a, uint256 b) internal pure returns (uint256) { return (a + b - 1) / b; }
}
