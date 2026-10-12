// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {FloorBase} from "./FloorBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// docs/v6/relay-conversion.md §2: the advance. One unit's item budget leaves BEFORE its item arrives (to be converted into what
/// the item's chain is paid in); that unit is in transit — outside every burn and conversion — until the item is wrapped here
/// (`settleAdvance`) or the ETH comes back (`returnAdvance`). One at a time, within `maxItemCost()` and the daily item count.
contract AdvanceTest is FloorBase {
    event Advanced(uint256 amount, uint256 released);
    event AdvanceSettled(uint256 indexed item, uint256 advanced, uint256 cost);
    event AdvanceReturned(uint256 returned, uint256 basis, uint256 loss);

    function _advance(uint256 amount) internal {
        vm.prank(custodian);
        module.advance(amount);
    }

    function _settle(uint256 cost) internal {
        _deposit(address(module));
        vm.prank(custodian);
        module.settleAdvance(cost);
    }

    function _return(uint256 v) internal {
        vm.deal(custodian, custodian.balance + v);
        vm.prank(custodian);
        module.returnAdvance{value: v}();
    }

    /// The books with a unit in transit: every wei owed is held, and the tiers add up to the wrapper's ETH-backed units.
    function _transitBooks() internal view {
        _books();
        assertEq(module.netUnits() + module.reserveUnits() + module.transitUnits(), wrapper.ethBacked(), "C + R + T = E");
    }

    // ---- the ABI the canister calls ----

    function test_theSelectorsTheCanisterEncodes() public view {
        assertEq(FloorModule.advance.selector, bytes4(keccak256("advance(uint256)")));
        assertEq(FloorModule.settleAdvance.selector, bytes4(keccak256("settleAdvance(uint256)")));
        assertEq(FloorModule.returnAdvance.selector, bytes4(keccak256("returnAdvance()")));
        assertEq(module.transitUnits.selector, bytes4(keccak256("transitUnits()")));
    }

    // ---- advance ----

    function test_advancePaysThePayeeNowAndPutsOneUnitInTransit() public {
        _mint(alice, 2 ether);
        uint256 e = wrapper.ethBacked();
        uint256 net = module.netUnits();
        uint256 beta = module.betaConvertible();
        uint256 amount = beta - 0.001 ether;
        uint256 bal = address(module).balance;
        vm.expectEmit(address(module));
        emit Advanced(amount, beta);
        _advance(amount);
        assertEq(payee.balance, amount, "the payee (the canister's EVM address) has exactly the advance");
        assertEq(address(module).balance, bal - amount);
        assertEq(module.transitUnits(), 1e18);
        assertEq(module.advanced(), amount);
        assertEq(module.advanceBasis(), beta);
        assertEq(module.advancedAt(), vm.getBlockTimestamp());
        assertEq(wrapper.ethBacked(), e, "the wrapper still counts the unit: its wX is in somebody's wallet");
        assertEq(module.netUnits(), net - 1e18, "the transit unit is no convertible unit");
        assertEq(module.revenue(), 0.001 ether, "what the advance took under the unit's backing is revenue, as at an arrival");
        assertGe(module.betaConvertible(), beta, "no other unit's backing per unit fell");
        assertEq(module.arrivals(), 1, "H11 counts the advance: the ETH has left");
        _transitBooks();
    }

    function test_advanceRefusals() public {
        _mint(alice, 2 ether);
        vm.prank(stranger);
        vm.expectRevert(FloorModule.NotCustodian.selector);
        module.advance(0.5 ether);
        vm.prank(custodian);
        vm.expectRevert(FloorModule.BadInput.selector);
        module.advance(0);
        uint256 max = module.maxItemCost();
        vm.prank(custodian);
        vm.expectRevert(abi.encodeWithSelector(FloorModule.OverItemCap.selector, max + 1, max));
        module.advance(max + 1);
        _advance(0.3 ether);
        vm.prank(custodian);
        vm.expectRevert(FloorModule.AdvanceOpen.selector);            // one in flight at a time
        module.advance(0.1 ether);
    }

    function test_noAdvanceWhenStale_orWhileTheKeeperIsPaused_orWithoutAWholeUnit() public {
        _advanceRefusedWith0();                                      // nothing ETH-backed: no whole unit
        _mint(alice, 2 ether);
        _skip(module.FLOOR_TTL());                                   // the floor is stale
        assertEq(module.maxItemCost(), 0);
        _advanceRefusedWith0();
        _post(F);
        vm.prank(owner);
        registry.pauseKeeper();
        assertEq(module.maxItemCost(), 0, "no conversion is priced by the keeper's word during an incident");
        _advanceRefusedWith0();
    }

    function _advanceRefusedWith0() internal {
        vm.prank(custodian);
        vm.expectRevert(abi.encodeWithSelector(FloorModule.OverItemCap.selector, 1, 0));
        module.advance(1);
    }

    function test_anAdvanceAboveTheBackingIsToppedUpFromRevenueThenSurplus_neverTheReserve() public {
        _mint(alice, 2 ether);
        module.fundReserve{value: 0.05 ether}();                     // surplus
        uint256 beta = module.betaConvertible();
        uint256 amount = module.maxItemCost();
        assertGt(amount, beta, "the cap (floor + 3%) is over the unit's backing");
        uint256 s = module.surplus();
        uint256 res = module.reserve();
        _advance(amount);
        assertEq(module.surplus(), s - (amount - beta), "the shortfall came from the surplus (no revenue yet)");
        assertEq(module.reserve(), res, "never the reserve");
        assertEq(payee.balance, amount);
        _transitBooks();
    }

    function test_theDailyCountIsTakenAtTheAdvance_andNotAgainAtSettlement() public {
        FloorModule.Params memory p = _params();
        p.maxItemsPerDay = 2;
        _redeploy(p);
        _mint(alice, 3 ether);
        _convert(module.betaConvertible());
        uint256 amount = module.betaConvertible();
        _advance(amount);
        assertFalse(module.arrivalRoom(), "two ETH outflows today: the limit");
        assertEq(module.maxItemCost(), 0);
        _settle(amount);
        assertEq(module.arrivals(), 2, "the settlement is not a second arrival");
        vm.prank(custodian);
        vm.expectRevert(abi.encodeWithSelector(FloorModule.OverItemCap.selector, 1, 0));
        module.advance(1);
        _deposit(address(module));
        uint256 beta = module.betaConvertible();
        vm.prank(custodian);
        vm.expectRevert(FloorModule.ArrivalRateLimited.selector);
        module.itemArrived(beta);
    }

    // ---- burns cannot reach the transit unit ----

    function test_burnsNeverTakeTheTransitUnit_andPayTheSamePerUnit() public {
        _mint(alice, 0.7 ether);
        _mint(bob, 1.4 ether);
        _calm();
        (uint256 priceBefore,) = module.burnCapacity();
        uint256 betaBefore = module.beta();
        _advance(module.betaConvertible());
        (uint256 priceAfter, uint256 room) = module.burnCapacity();
        assertEq(priceAfter, priceBefore, "the others' exit price is unchanged");
        assertGe(module.beta(), betaBefore, "and so is their backing per unit");
        uint256 live = wrapper.ethBacked() - 1e18;
        assertLe(room, live, "the burn room never includes the transit unit");
        // everyone else exits: the last live unit takes the pot's dust, the transit unit's wX cannot be burned here
        uint256 a = wrapper.balanceOf(alice);
        uint256 b = wrapper.balanceOf(bob);
        _skip(1 hours);
        _post(F);
        uint256 burnB = b < live ? b : live;
        _burn(bob, burnB);
        uint256 left = wrapper.ethBacked() - module.transitUnits();
        if (left != 0) {
            vm.startPrank(alice);
            wrapper.approve(address(module), a);
            vm.expectRevert(FloorModule.NotEnoughBacked.selector);
            module.burn(left + 1, 0, alice);                         // one unit more than live: the transit unit
            vm.stopPrank();
            _burn(alice, left);
        }
        assertEq(wrapper.ethBacked(), 1e18, "only the transit unit is left ETH-backed");
        assertEq(module.netUnits(), 0);
        assertEq(module.backing(), 0, "every live unit took exactly its backing");
        assertEq(module.beta(), 0);
        _transitBooks();
    }

    // ---- settle ----

    function test_settleTurnsTheTransitUnitIntoTheItem_noEthMoves() public {
        _mint(alice, 2 ether);
        uint256 amount = module.betaConvertible();
        _advance(amount);
        uint256 supply = wrapper.totalSupply();
        uint256 e = wrapper.ethBacked();
        uint256 bal = address(module).balance;
        uint256 fills = module.fillCount();
        uint256 cost = amount - 0.002 ether;                        // the item used less than the whole advance
        _deposit(address(module));
        vm.expectEmit(address(module));
        emit AdvanceSettled(0, amount, cost);
        vm.prank(custodian);
        module.settleAdvance(cost);
        assertEq(module.transitUnits(), 0);
        assertEq(module.advanced(), 0);
        assertEq(module.advanceBasis(), 0);
        assertEq(module.advancedAt(), 0);
        assertEq(wrapper.ethBacked(), e - 1e18, "one ETH-backed unit is item-backed now");
        assertEq(wrapper.totalSupply(), supply, "nobody's balance changed");
        assertEq(wrapper.held(), 1);
        assertEq(address(module).balance, bal, "no ETH moved");
        assertEq(module.itemsArrived(), 1);
        assertEq(module.fillCount(), fills + 1, "the cost is a fill");
        _transitBooks();
    }

    function test_settleRefusals() public {
        vm.prank(custodian);
        vm.expectRevert(FloorModule.NoAdvance.selector);
        module.settleAdvance(1);
        _mint(alice, 2 ether);
        uint256 amount = 0.5 ether;
        _advance(amount);
        vm.prank(stranger);
        vm.expectRevert(FloorModule.NotCustodian.selector);
        module.settleAdvance(amount);
        vm.prank(custodian);
        vm.expectRevert(FloorModule.NoWrapped.selector);            // the item is not wrapped here yet
        module.settleAdvance(amount);
        _deposit(address(module));
        vm.prank(custodian);
        vm.expectRevert(FloorModule.BadInput.selector);
        module.settleAdvance(0);
        vm.prank(custodian);
        vm.expectRevert(FloorModule.BadInput.selector);             // never more than was advanced
        module.settleAdvance(amount + 1);
        vm.prank(custodian);
        module.settleAdvance(amount);
        _transitBooks();
    }

    function test_settleWhileTheKeeperIsPausedRecordsNoFill() public {
        _mint(alice, 2 ether);
        _advance(0.5 ether);
        vm.prank(owner);
        registry.pauseKeeper();
        uint256 fills = module.fillCount();
        _settle(0.5 ether);
        assertEq(module.fillCount(), fills, "no fill is priced during an incident");
        assertEq(module.transitUnits(), 0, "but the item is here");
        _transitBooks();
    }

    function test_aSecondAdvanceAfterSettlement() public {
        _mint(alice, 3 ether);
        uint256 a1 = module.betaConvertible();
        _advance(a1);
        _settle(a1);
        uint256 a2 = module.betaConvertible();
        _advance(a2);
        assertEq(payee.balance, a1 + a2);
        _settle(a2);
        assertEq(module.itemsArrived(), 2);
        _transitBooks();
    }

    // ---- return ----

    function test_returnAtOrAboveTheBasisRestoresTheUnit_theRestIsRevenue() public {
        _mint(alice, 2 ether);
        uint256 net = module.netUnits();
        uint256 backing = module.backing();
        uint256 beta = module.betaConvertible();
        uint256 amount = beta;
        _advance(amount);
        uint256 r = module.revenue();
        vm.expectEmit(address(module));
        emit AdvanceReturned(beta + 0.01 ether, beta, 0);
        _return(beta + 0.01 ether);
        assertEq(module.transitUnits(), 0);
        assertEq(module.netUnits(), net, "the unit is convertible again");
        assertEq(module.backing(), backing, "its backing is back");
        assertEq(module.revenue(), r + 0.01 ether, "anything above it is revenue");
        assertEq(module.betaConvertible(), beta);
        _transitBooks();
    }

    function test_aShortReturnIsCoveredByRevenueThenSurplus() public {
        _mint(alice, 2 ether);
        uint256 beta = module.betaConvertible();
        uint256 backing = module.backing();
        _advance(beta - 0.02 ether);                                  // 0.02 ETH of revenue
        module.fundReserve{value: 0.01 ether}();                      // 0.01 ETH of surplus
        vm.expectEmit(address(module));
        emit AdvanceReturned(beta - 0.025 ether, beta, 0);
        _return(beta - 0.025 ether);                                  // Relay's refund gas: 0.005 short of what it took
        assertEq(module.backing(), backing, "the unit's backing is whole again");
        assertEq(module.revenue(), 0, "revenue went first");
        assertEq(module.surplus(), 0.005 ether, "then the surplus");
        _transitBooks();
    }

    function test_aReturnTheyCannotCoverIsALossSharedByTheConvertibleUnits() public {
        _mint(alice, 2 ether);
        uint256 beta = module.betaConvertible();
        uint256 backing = module.backing();
        _advance(beta);
        vm.expectEmit(address(module));
        emit AdvanceReturned(beta / 2, beta, beta - beta / 2);
        _return(beta / 2);
        assertEq(module.backing(), backing - (beta - beta / 2), "the loss is the pot's");
        assertLt(module.betaConvertible(), beta, "and every convertible unit's share of it");
        _transitBooks();
    }

    function test_returnRefusals() public {
        vm.prank(custodian);
        vm.expectRevert(FloorModule.NoAdvance.selector);
        module.returnAdvance();
        _mint(alice, 2 ether);
        _advance(0.5 ether);
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        vm.expectRevert(FloorModule.NotCustodian.selector);
        module.returnAdvance{value: 0.5 ether}();
        _return(0);                                                   // everything lost: still closes the advance
        assertEq(module.transitUnits(), 0);
        _transitBooks();
        _advance(0.1 ether);                                          // the next one may start (the day's count permitting)
    }

    // ---- fuzz: whatever happens in between, the books hold ----

    function testFuzz_advanceThenSettleOrReturn(uint256 aSeed, uint256 cSeed, uint256 vSeed, bool settle, uint256 burnSeed) public {
        _mint(alice, 1 ether);
        _mint(bob, 2 ether);
        uint256 max = module.maxItemCost();
        uint256 amount = bound(aSeed, 1, max);
        uint256 betaBefore = module.betaConvertible();
        _advance(amount);
        assertGe(module.betaConvertible() + 1, betaBefore, "an advance never lowers the others' backing per unit");
        _transitBooks();
        uint256 live = wrapper.ethBacked() - 1e18;
        uint256 u = bound(burnSeed, 0, live < 3e18 ? live : 3e18);
        uint256 have = wrapper.balanceOf(bob);
        if (u > have) u = have;
        if (u != 0 && module.quoteBurn(u) != 0) _burn(bob, u);
        _transitBooks();
        if (settle) {
            _settle(bound(cSeed, 1, amount));
        } else {
            _return(bound(vSeed, 0, 2 * amount));
        }
        assertEq(module.transitUnits(), 0);
        _transitBooks();
    }
}
