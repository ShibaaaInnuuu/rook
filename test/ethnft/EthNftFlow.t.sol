// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {EthNftBase} from "./EthNftBase.sol";
import {EthNftWrapper} from "../../src/nft/EthNftWrapper.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {PosterSource} from "../../src/floor/PosterSource.sol";
import {IKeeperSource} from "../../src/interfaces/IKeeperSource.sol";

/// docs/v7/phase1-nft.md §2–3: the poster, and the whole buy / redeem path on the 4663 side, against the real FloorModule.
contract PosterSourceTest is EthNftBase {
    function test_thePosterIsTheCanister_andTheRegistrySwitchZeroesIt() public {
        assertEq(poster.keeper(), custodian);
        assertEq(module.keeper(), custodian);
        assertEq(poster.owner(), owner);
        assertEq(poster.guardian(), guardian);
        assertEq(address(poster.registry()), address(registry));
        vm.prank(guardian);
        registry.pauseKeeper();
        assertTrue(poster.keeperPaused());
        assertEq(poster.keeper(), address(0));
        vm.prank(custodian);
        vm.expectRevert(FloorModule.NotKeeper.selector);
        module.postFloor(F, 10_000, 50);
        vm.prank(owner);
        registry.unpauseKeeper();
        assertEq(poster.keeperResumedAt(), vm.getBlockTimestamp());
        assertEq(poster.keeperResumedAtBlock(), vm.getBlockNumber());
        assertEq(poster.keeper(), custodian);
    }

    function test_theKeeperEoaHasNoFloorRoleHere() public {
        vm.prank(keeper);
        vm.expectRevert(FloorModule.NotKeeper.selector);
        module.postFloor(F, 10_000, 50);
        vm.prank(keeper);
        vm.expectRevert(FloorModule.NotKeeper.selector);
        module.postRentalRoot(bytes32(uint256(1)));
    }

    function test_constructorRefusals() public {
        vm.expectRevert(PosterSource.BadInput.selector);
        new PosterSource(IKeeperSource(address(0)), custodian);
        vm.expectRevert(PosterSource.BadInput.selector);
        new PosterSource(IKeeperSource(address(registry)), address(0));
        vm.expectRevert(PosterSource.BadInput.selector);
        new PosterSource(IKeeperSource(address(registry)), address(module));      // a poster with code
    }

    /// Audit 2026-09-27-nft D-M-1 / A-L-3: this collection's own floor switch. The guardian (D8) or the Safe pauses this poster
    /// alone; the module's keeper is zero and its posts are refused, while the registry (and every other collection's poster over
    /// it) is untouched. Only the Safe resumes it, and the resume starts the module's RESUME_GRACE (the later of the two stamps).
    function test_D_M_1_thePosterHasItsOwnSwitch_theGuardianPausesOneCollection_theSafeResumesIt() public {
        PosterSource other = new PosterSource(IKeeperSource(address(registry)), custodian);   // another collection's, same registry
        vm.expectRevert(PosterSource.NotGuardian.selector);
        poster.pause();                                                                      // a stranger cannot
        vm.prank(guardian);
        poster.pause();
        assertTrue(poster.paused());
        assertTrue(poster.keeperPaused());
        assertEq(poster.keeper(), address(0));
        assertEq(module.keeper(), address(0));
        vm.prank(custodian);
        vm.expectRevert(FloorModule.NotKeeper.selector);
        module.postFloor(F, 10_000, 50);
        assertFalse(registry.keeperPaused(), "the registry is not paused: every other collection goes on");
        assertEq(other.keeper(), custodian);
        assertFalse(other.keeperPaused());
        vm.prank(guardian);
        vm.expectRevert(PosterSource.NotOwner.selector);
        poster.unpause();                                                                    // the guardian only stops things
        _skip(1 hours);
        vm.prank(owner);
        poster.unpause();
        assertEq(poster.resumedAt(), vm.getBlockTimestamp());
        assertEq(poster.keeperResumedAt(), vm.getBlockTimestamp(), "the later of the poster's and the registry's resume");
        assertEq(poster.keeperResumedAtBlock(), vm.getBlockNumber());
        assertEq(poster.keeper(), custodian);
        vm.prank(custodian);
        module.postFloor(F, 10_000, 50);
        // the owner may pause it too; the registry's pause still zeroes it, whatever its own switch says
        vm.prank(owner);
        poster.pause();
        vm.prank(owner);
        poster.unpause();
        vm.prank(guardian);
        registry.pauseKeeper();
        assertEq(poster.keeper(), address(0));
        assertEq(other.keeper(), address(0), "the registry's pause is the canister-wide switch");
    }

    function test_theModulesBoundsStillFenceThePoster() public {
        _skip(1 hours);
        _post(F * 2);
        assertEq(module.accepted(), F * 105 / 100, "+5 % an hour at most");
        assertEq(module.target(), F * 2);
    }
}

contract EthNftFlowTest is EthNftBase {
    event AdvanceSettled(uint256 indexed item, uint256 advanced, uint256 cost);

    function test_aGraduationsBudgetBuysAnNft_andTheUnitBecomesItemBacked() public {
        _mintAtModule(alice, 3 ether);                        // stands in for a graduation's ETH-backed units
        uint256 supply = w.totalSupply();
        uint256 amount = module.maxItemCost();
        assertGt(amount, 0);
        assertLe(amount, F * 103 / 100 + 1, "at most floor + 3 %");
        vm.prank(custodian);
        module.advance(amount);
        assertEq(payee.balance, amount, "the item budget is at the canister's address, to go to Ethereum through Relay");
        assertEq(module.transitUnits(), ONE);
        _books();
        // (Relay → Ethereum, a Seaport fill, finality) … then the canister mints the unit to the module and settles
        _mintItem(9180, address(module));
        uint256 cost = amount * 9_950 / 10_000;
        vm.prank(custodian);
        module.settleAdvance(cost);
        assertEq(module.transitUnits(), 0);
        assertEq(w.totalSupply(), supply, "the supply is unchanged: one ETH-backed unit became item-backed");
        assertEq(w.ethBacked(), supply - ONE);
        _books();
        vm.prank(custodian);
        vm.expectRevert(FloorModule.NoAdvance.selector);
        module.settleAdvance(cost);
    }

    function test_settleNeedsTheUnitAtTheModule() public {
        _mintAtModule(alice, 3 ether);
        uint256 amount = module.maxItemCost();
        vm.prank(custodian);
        module.advance(amount);
        vm.prank(custodian);
        vm.expectRevert(FloorModule.NoWrapped.selector);
        module.settleAdvance(amount);
        _mintItem(9180, address(module));
        vm.expectEmit(address(module));
        emit AdvanceSettled(0, amount, amount - 1);
        vm.prank(custodian);
        module.settleAdvance(amount - 1);
        assertEq(w.held(), 1);
        assertEq(module.itemsArrived(), 1);
        assertEq(w.balanceOf(address(module)), 0, "the module burned the unit it was minted: it backs alice's units now");
        _books();
    }

    function test_theWholeLoop_buyRedeemDeliver() public {
        _mintAtModule(alice, 3 ether);
        uint256 units = w.balanceOf(alice);
        _buyOne(9180, module.maxItemCost());
        assertEq(w.held(), 1);
        uint256 t = _redeem(alice, alice);
        assertEq(w.balanceOf(alice), units - ONE);
        _resolve(t, 0);
        assertEq(_tokenOf(t), 9180);
        _deliver(t);
        assertEq(w.held(), 0);
        _books();
        // the rest of alice's units are ETH-backed and exit at the module
        uint256 eth = _burnAtModule(alice, w.balanceOf(alice));
        assertGt(eth, 0);
        _books();
    }

    function test_aFailedConversionComesBack() public {
        _mintAtModule(alice, 3 ether);
        uint256 amount = module.maxItemCost();
        uint256 backing = module.backing();
        vm.prank(custodian);
        module.advance(amount);
        vm.deal(custodian, amount);
        vm.prank(custodian);
        module.returnAdvance{value: amount}();
        assertEq(module.transitUnits(), 0);
        assertGe(module.backing(), backing - 1, "the backing is restored");
        _books();
    }

    function test_thePauseStopsEveryInflowPricedByThePoster_butNotBurns() public {
        _mintAtModule(alice, 2 ether);
        vm.prank(guardian);
        registry.pauseKeeper();
        vm.deal(bob, 1 ether);
        vm.prank(bob);
        vm.expectRevert(FloorModule.Paused.selector);
        module.mint{value: 1 ether}(bob, 0);
        assertEq(module.maxItemCost(), 0, "no advance while paused");
        vm.prank(custodian);
        vm.expectRevert();
        module.advance(1);
        uint256 eth = _burnAtModule(alice, w.balanceOf(alice) / 2);
        assertGt(eth, 0, "burns go on at the pause floor");
        _books();
    }

    function test_oneAdvanceAtATime() public {
        _mintAtModule(alice, 4 ether);
        uint256 amount = module.maxItemCost();
        vm.prank(custodian);
        module.advance(amount);
        uint256 max = module.maxItemCost();
        vm.prank(custodian);
        vm.expectRevert(FloorModule.AdvanceOpen.selector);
        module.advance(max == 0 ? 1 : max);
    }
}
