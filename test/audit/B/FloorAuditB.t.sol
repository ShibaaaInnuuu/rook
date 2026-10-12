// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {FloorBase} from "../../floor/FloorBase.sol";
import {FloorModule} from "../../../src/floor/FloorModule.sol";
import {Wrapper} from "../../../src/Wrapper.sol";
import {ListingRegistry} from "../../../src/ListingRegistry.sol";

/// Audit B regressions (docs/audit/2026-09-26/B-floor.md; the PoCs are in docs/audit/2026-09-26/poc/B). Each test replays a PoC's
/// attack against the fixed contracts and asserts it now FAILS. The deployed defaults: a 25% reserve share, stage 0's 3 arrivals
/// a day. Design: docs/audit/2026-09-26/fixes-B-C-keeper-draws.md.
contract FloorAuditB is FloorBase {
    address mallory = makeAddr("mallory");
    address guardian = makeAddr("guardian");

    function _params() internal pure override returns (FloorModule.Params memory p) {
        p = super._params();
        p.reserveBps = 2_500;
        p.maxItemsPerDay = 3; // stage 0
    }

    // ---- B-01: a dust reserve tier no longer freezes burns (a burn spans the tiers) -------------------------------------------

    function test_B01_aDustMintNoLongerFreezesBurns() public {
        _mint(alice, 4 * module.mintPrice());                 // 1 reserve unit + 3 convertible
        _burn(alice, module.reserveUnits());
        assertEq(module.reserveUnits(), 0, "reserve tier drained");
        vm.deal(mallory, 4);
        vm.prank(mallory);
        module.mint{value: 4}(mallory, 0);                    // the grief: a 1-wei reserve tier
        assertEq(module.reserveUnits(), 1);
        (uint256 p, uint256 room) = module.burnCapacity();
        assertGt(p, 0, "a price");
        assertGe(room, 1e18, "the module takes whole units again");
        assertGt(module.quoteBurn(1e18), 0.6 ether, "worth a unit's backing");
        uint256 got = _burn(alice, 1e18);                    // the dust tier and a convertible unit, in one call
        assertGt(got, 0.6 ether);
        assertEq(module.reserveUnits(), 0);
        _books();
    }

    function test_B01b_theGriefCannotBeReArmed_aBankRunDrainsTheModule() public {
        _mint(alice, 4 * module.mintPrice());
        vm.deal(mallory, 1 ether);
        uint256 left = wrapper.balanceOf(alice);
        while (left != 0) {
            vm.prank(mallory);
            module.mint{value: 4}(mallory, 0);                // re-armed before every burn
            uint256 u = left < 1e18 ? left : 1e18;
            assertGt(_burn(alice, u), 0, "every burn pays");
            left = wrapper.balanceOf(alice);
            _calm();
        }
        _books();
    }

    // ---- B-02: a compromised keeper cannot drain the surplus through burns, nor walk the floor past the day's band -------------

    function test_B02_aCompromisedKeeperCannotDrainTheReserveSurplusThroughBurns() public {
        _mint(alice, 4 * module.mintPrice());                 // an honest minter: 1 reserve unit at the mint price
        address curveLike = makeAddr("a curve paying its reserve slice");
        vm.deal(curveLike, 3 ether);
        vm.prank(curveLike);
        module.fundReserve{value: 3 ether}();
        assertEq(module.surplus(), 3 ether, "fee money is surplus, not the reserve units' backing");
        uint256 basisPerReserveUnit = module.betaReserve();
        for (uint256 i = 0; i < 2; ++i) _deposit(mallory);   // Mallory's 2 wX, "bought in the pool" at ~F

        // the stolen keeper posts a far-off target every hour, as in the PoC
        for (uint256 h = 0; h < 15; ++h) { _skip(1 hours); _post(F * 100); }
        _calm();
        assertLe(module.accepted(), F * 12_000 / 10_000, "the day's band stops the walk at +20% (the PoC reached 2x)");

        uint256 before = mallory.balance;
        _burn(mallory, module.reserveUnits());                // the reserve unit at the pushed floor
        uint256 got = mallory.balance - before;
        assertLe(got, basisPerReserveUnit, "never more than the unit paid in (the PoC paid 2x its basis)");
        assertGe(module.surplus(), 3 ether, "the surplus is untouched; it only keeps the spread");
        _books();
    }

    function test_B02b_overDaysTheWalkIsBoundedPerDay() public {
        for (uint256 h = 0; h < 47; ++h) { _skip(1 hours); _post(F * 100); }
        // two day windows: at most 1.2^2 = 1.44x (the 5%/h walk alone would have been 1.05^47 ≈ 9.9x)
        assertLe(module.accepted(), F * 14_400 / 10_000 + 1);
        uint256 top = module.accepted();
        for (uint256 h = 0; h < 23; ++h) { _skip(1 hours); _post(1); }
        assertGe(module.accepted(), top * 6_400 / 10_000 - 1, "and down the same way: two windows at most in a day");
    }

    /// B-09 / E-02: the guardian (or the Safe) stops the keeper, and every inflow priced by its word, at once. Burns continue.
    function test_B09_theGuardianStopsTheKeeperAndTheInflowsAtOnce_burnsContinue() public {
        vm.prank(owner);
        registry.setGuardian(guardian);
        _mint(alice, 2 * module.mintPrice());
        vm.prank(guardian);
        registry.pauseKeeper();
        assertEq(module.keeper(), address(0));
        vm.prank(keeper);
        vm.expectRevert(FloorModule.NotKeeper.selector);
        module.postFloor(F, 10_000, 50);
        vm.deal(bob, 1 ether);
        vm.prank(bob);
        vm.expectRevert(FloorModule.Paused.selector);
        module.mint{value: 1 ether}(bob, 0);
        (uint256 mp, uint256 maxIn) = module.mintCapacity();
        assertEq(mp + maxIn, 0, "no mint is quoted");
        assertGt(_burn(alice, 1e18), 0, "a holder still exits");
        vm.prank(guardian);
        vm.expectRevert();
        registry.unpauseKeeper();                             // only the owner resumes
        vm.prank(owner);
        registry.unpauseKeeper();
        _post(F);
        _books();
    }

    /// E-02 / C-11: the module no longer binds a key. The owner's rotation (seven public days) moves the module's keeper too.
    function test_E02_theModulesKeeperRotatesWithTheRegistry() public {
        address fresh = makeAddr("the new keeper");
        vm.prank(owner);
        registry.proposeRoles(fresh, purchaser, attestor);
        _skip(7 days + 1 hours);
        vm.prank(owner);
        registry.acceptRoles();
        assertEq(module.keeper(), fresh);
        vm.prank(keeper);
        vm.expectRevert(FloorModule.NotKeeper.selector);
        module.postFloor(F, 10_000, 50);
        vm.prank(fresh);
        module.postFloor(F, 10_000, 50);
        assertTrue(module.fresh());
    }

    // ---- B-03: the keeper cannot record a delivery ----------------------------------------------------------------------------

    function test_B03_theKeeperCannotForgeADelivery_theRedeemerIsRefunded() public {
        _deposit(alice);
        vm.prank(alice);
        uint256 id = wrapper.redeem(TAPROOT);
        vm.warp(vm.getBlockTimestamp() + 2 minutes);
        _resolveTicket(id, keccak256("r"));
        vm.prank(keeper);
        vm.expectRevert(Wrapper.NotCustodian.selector);
        wrapper.recordDelivery(id, keccak256("made-up txid"));
        _skip(10 days + 1 hours);                             // the deadline and the refund grace
        vm.prank(custodian);
        wrapper.markUndeliverable(id);                        // the custodian's attestation (review2 R3 §4)
        wrapper.refund(id);
        assertEq(wrapper.balanceOf(alice), ONE, "Alice has her token back");
        assertEq(wrapper.held(), 1, "the frog is still on the books");
    }

    // ---- B-04: a lost keeper no longer locks the backing ----------------------------------------------------------------------

    function test_B04_aLostKeeperNoLongerLocksTheBacking() public {
        _mint(alice, 4 * module.mintPrice());
        vm.deal(address(this), 1 ether);
        module.fundReserve{value: 1 ether}();
        uint256 backs = module.backing() + module.reserve();
        _skip(module.FLOOR_TTL());
        vm.startPrank(alice);
        wrapper.approve(address(module), type(uint256).max);
        vm.expectRevert(FloorModule.Stale.selector);
        module.burn(1e18, 0, alice);                          // stale, not yet escaped
        vm.stopPrank();
        _skip(7 days);
        assertTrue(module.escaped(), "a week without a good post");
        uint256 got = _burn(alice, wrapper.balanceOf(alice));
        assertGe(got, backs * 9_984 / 10_000, "every unit paid its backing less the base spread");
        assertLe(got, backs);
        module.skim();                                        // the rest (spread, surplus) to the platform
        assertEq(address(module).balance, 0, "nothing is left locked");
        assertEq(platform.balance, 1 ether + backs - got);
    }

    /// E-02 (a): a thief posting confidence 0 every 50 minutes keeps the module stale, but not past the fallback: the week counts
    /// from the last GOOD post.
    function test_E02a_confidenceZeroPostsCannotHoldTheModuleFrozen() public {
        _mint(alice, 2 * module.mintPrice());
        conf = 0;
        uint256 n = (module.FLOOR_TTL() + module.STALE_ESCAPE()) / 50 minutes + 2;              // 207 posts: 7 days and 4.5 h
        for (uint256 i = 0; i < n; ++i) { _skip(50 minutes); _post(module.accepted()); }
        assertFalse(module.fresh());
        assertTrue(module.escaped(), "the recent bad posts do not count");
        assertGt(_burn(alice, 1e18), 0);
    }

    /// Fuzz: in a calm market with a fee-funded surplus, a mint -> burn round trip never profits (the PoC's fuzz, kept).
    function testFuzz_B_roundTripNeverProfitsWithSurplus(uint256 eth, uint256 surplus, uint256 prior) public {
        eth = bound(eth, 4, 1.5 ether);
        surplus = bound(surplus, 0, 10 ether);
        prior = bound(prior, 0, 2);
        if (prior != 0) _mint(bob, prior * module.mintPrice());
        if (surplus != 0) { vm.deal(address(this), surplus); module.fundReserve{value: surplus}(); }
        uint256 units = _mint(mallory, eth);
        (, uint256 room) = module.burnCapacity();
        uint256 out;
        uint256 bal = wrapper.balanceOf(mallory);
        while (bal != 0 && room != 0) {
            uint256 u = bal < room ? bal : room;
            if (module.quoteBurn(u) == 0) break;
            out += _burn(mallory, u);
            bal = wrapper.balanceOf(mallory);
            (, room) = module.burnCapacity();
        }
        assertLe(out, eth, "no round-trip profit");
        assertLe(units, eth * 1e18 / F + 1, "never more than a unit per floor");
        _books();
    }

    /// A-10 (deferred to this branch): a graduation's price counts the keeper's raw target only up to the top of the day's band,
    /// so a far-off post cannot shrink a graduation's units by more than the band allows.
    function test_A10_aFarOffTargetDoesNotPriceAGraduation() public {
        uint256 before = module.gradPrice();
        _skip(1 minutes);
        _post(F * 100);                                        // accepted barely moves; the raw target is 100x
        assertEq(module.target(), F * 100);
        uint256 cap = module.dayAnchor() * 12_000 / 10_000 * 10_300 / 10_000 + 1;
        assertLe(module.gradPrice(), cap, "at most the top of the day's band, plus the conversion allowance");
        assertGe(module.gradPrice(), before, "and never below the price before the post");
    }

    receive() external payable {}
}
