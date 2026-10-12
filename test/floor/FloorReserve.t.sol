// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {FloorBase} from "./FloorBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// docs/v5/floor-module.md §13.1 (the reserve share: two pots, burns from the reserve first, never converted) and §13.4 (the
/// fills price inside the keeper's band; `itemSold`), at the 25% default and the frog terms of `FloorBase`.
contract FloorReserveTest is FloorBase {
    address mallory = makeAddr("mallory");

    function _params() internal pure override returns (FloorModule.Params memory p) {
        p = super._params();
        p.reserveBps = 2_500;
        p.fillBandBps = 300;
    }

    /// Mint `n` whole units for `who`, four an hour (the hourly cap is five).
    function _mintMany(address who, uint256 n) internal {
        while (n != 0) {
            uint256 k = n < 4 ? n : 4;
            _mint(who, k * module.mintPrice());
            n -= k;
            if (n != 0) { _skip(1 hours); _post(module.accepted()); }
        }
    }

    // ---- the reserve share ----

    function test_aMintSplitsIntoTheTwoPots() public {
        uint256 units = _mint(alice, 1 ether);
        assertEq(module.reserve(), 0.25 ether, "a quarter of the ETH is the standing reserve");
        assertEq(module.backing(), 0.75 ether, "the rest is convertible");
        assertEq(module.reserveUnits(), units / 4, "a quarter of the units are reserve units");
        assertEq(module.netUnits(), units - units / 4, "the canister sees only the convertible share");
        assertEq(wrapper.ethBacked(), units, "all of them are ETH-backed");
        assertEq(module.escrowBalance(), 1 ether, "every wei backs a unit");
        assertApproxEqAbs(module.betaReserve(), module.betaConvertible(), 1, "both pots hold the mint price per unit");
        _books();
    }

    function test_theCanisterConvertsOnlyTheConvertibleShare() public {
        _mint(alice, 4 * module.mintPrice());                  // 4 units: 3 convertible, 1 reserve
        assertEq(module.netUnits(), 3e18);
        uint256 s = module.reserve();
        for (uint256 i = 0; i < 3; ++i) _convert(module.betaConvertible());
        assertEq(module.netUnits(), 0, "three items bought");
        assertEq(module.backing(), 0, "the convertible pot is spent");
        assertEq(module.reserve(), s, "the reserve is untouched by conversions");
        assertEq(module.reserveUnits(), 1e18, "one unit stays ETH-backed, by the reserve");
        assertEq(module.maxItemCost(), 0, "nothing more to convert: the reserve is not for items");
        _books();
    }

    function test_theReserveIsNeverConverted_evenWhenTheRevenueIsShort() public {
        _mint(alice, 4 * module.mintPrice());
        uint256 s = module.reserve();
        uint256 beta = module.betaConvertible();
        _deposit(address(module));
        vm.prank(custodian);
        vm.expectRevert(abi.encodeWithSelector(FloorModule.OverBasis.selector, 0.01 ether, 0));
        module.itemArrived(beta + 0.01 ether);
        assertEq(module.reserve(), s);
    }

    /// B-01: a burn draws the reserve units first and spans both tiers in one call, each tier paying at most its own backing per
    /// unit (docs/audit/2026-09-26/fixes-B-C-keeper-draws.md). Before, a burn could take reserve units only while there were any.
    function test_burnsDrawTheReserveFirst_andSpanTheTiers() public {
        uint256 units = _mint(alice, 4 * module.mintPrice());  // 3 convertible, 1 reserve
        uint256 r = module.reserveUnits();
        (uint256 price, uint256 room) = module.burnCapacity();
        assertEq(room, units, "a burn may take every ETH-backed unit, across the tiers");
        assertLe(price, module.burnPrice(), "the lower tier price is what the router sees");
        assertEq(module.beta(), module.betaReserve(), "the next unit is the reserve's");
        uint256 betaC = module.betaConvertible();
        uint256 quoted = module.quoteBurn(r + 1e18);
        uint256 b0 = module.backing();
        vm.startPrank(alice);
        wrapper.approve(address(module), units);
        uint256 got = module.burn(r + 1e18, 0, alice);         // across the tiers in one call
        vm.stopPrank();
        assertEq(got, quoted, "the quote is exact");
        assertEq(module.reserveUnits(), 0);
        assertEq(module.reserve(), 0, "the reserve released all its basis");
        assertApproxEqAbs(module.backing(), b0 - betaC, 1, "the convertible pot released one unit's backing");
        assertEq(module.beta(), module.betaConvertible());
        _burn(alice, 1e18);
        _books();
    }

    /// B-02: the reserve tier releases exactly its basis per unit; what it keeps of that (the spread) goes to the surplus, which
    /// no unit claims. So the reserve's backing per unit never moves on a burn, and a later reserve unit is backed by what it paid.
    function test_theReserveKeepsItsBasisPerUnit_andTheSpreadGoesToTheSurplus() public {
        uint256 units = _mint(alice, 4 * module.mintPrice());
        uint256 r = module.reserveUnits();
        uint256 betaS = module.betaReserve();
        uint256 v = module.revenue();
        _burn(alice, r / 2);
        assertApproxEqAbs(module.betaReserve(), betaS, 1, "the reserve's basis per unit stands");
        assertEq(module.revenue(), v, "the reserve's spread is not revenue");
        uint256 sp = module.surplus();
        assertGt(sp, 0, "it is the surplus");
        uint256 left = module.reserveUnits();
        _burn(alice, left);
        assertEq(module.reserveUnits(), 0);
        assertEq(module.reserve(), 0, "no basis without units");
        assertGt(module.surplus(), sp);
        // the next mint's reserve units are backed by what they paid, not by the surplus
        uint256 mp = module.mintPrice();
        _mint(bob, mp);
        assertLe(module.betaReserve(), mp, "the surplus never backs units");
        units;
        _books();
    }

    function Math_min(uint256 a, uint256 b) internal pure returns (uint256) { return a < b ? a : b; }

    function test_aTinyMintLeavesNoReserveUnitWithoutBacking() public {
        // 3 wei buys 4 units at 0.634 ETH: a quarter is one unit and zero wei — all of it goes convertible instead
        vm.deal(alice, 3);
        vm.prank(alice);
        module.mint{value: 3}(alice, 0);
        assertEq(module.reserveUnits(), 0);
        assertEq(module.reserve(), 0);
        assertEq(module.backing(), 3);
    }

    function test_reserveUnitsDoNotStressTheMintSpread_norFillThePublicCap() public {
        _redeployWith(2_500, 8e18, 40e18);                    // 8 convertible units of public room
        uint256 mintP = module.mintPrice();
        _mint(alice, 4 * mintP);                                // 3 convertible + 1 reserve
        (uint256 sMint,) = module.spreads();
        uint256 c = module.netUnits();
        assertEq(sMint, uint256(15) + (300 * c + 8e18 - 1) / 8e18, "stress counts the convertible units, not the reserve one");
        (uint256 price, uint256 room) = module.mintCapacity();
        mintP;
        assertApproxEqRel(room * 1e18 / price, (8e18 - c) * 10_000 / 7_500, 1e15, "the public cap: 5 convertible units is 6.67 minted");
    }

    function _redeployWith(uint16 reserveBps, uint256 publicCap, uint256 hardCap) internal {
        FloorModule.Params memory p = _params();
        p.reserveBps = reserveBps;
        p.maxPublicEthBacked = publicCap;
        p.maxEthBacked = hardCap;
        p.mintPerHour = 20e18;
        _redeploy(p);
    }

    /// wX stays fully backed through any sequence: (W1) for the count, and every wei of the two pots in the module.
    function testFuzz_wXStaysFullyBacked(uint256[8] memory amounts, uint8[8] memory ops) public {
        for (uint256 i = 0; i < 8; ++i) {
            uint256 op = ops[i] % 4;
            if (op == 0) {
                (, uint256 maxIn) = module.mintCapacity();
                uint256 a = bound(amounts[i], 0, maxIn);
                if (a * 1e18 >= module.mintPrice()) _mint(i % 2 == 0 ? alice : bob, a);
            } else if (op == 1) {
                address who = i % 2 == 0 ? alice : bob;
                (uint256 price, uint256 room) = module.burnCapacity();
                uint256 have = wrapper.balanceOf(who);
                uint256 u = bound(amounts[i], 0, room < have ? room : have);
                // a burn pays each tier separately, floored: a unit count whose reserve part or convertible part rounds to
                // 0 wei is refused (`Slippage`), so each part that exists must pay at least one wei (nightly 2026-10-07)
                uint256 rU = u < module.reserveUnits() ? u : module.reserveUnits();
                uint256 cU = u - rU;
                if (u != 0 && price != 0 && (rU == 0 || rU * price >= 1e18) && (cU == 0 || cU * price >= 1e18)) _burn(who, u);
            } else if (op == 2) {
                if (module.netUnits() >= 1e18 && module.maxItemCost() != 0) {
                    uint256 max = module.maxItemCost();
                    uint256 can = module.betaConvertible() + module.revenue();
                    _convert(bound(amounts[i], 1, max < can ? max : can));
                }
            } else {
                _skip(20 minutes);
                _post(module.accepted());
            }
            _books();
            assertEq(module.netUnits() + module.reserveUnits(), wrapper.ethBacked(), "E = C + R");
            assertGe(module.beta(), 0);
            if (module.reserveUnits() == 0 && module.netUnits() == 0) {
                assertEq(module.backing(), 0, "no convertible backing without convertible units");
            }
        }
    }

    // ---- push up, then burn: with the reserve ----

    /// The reserve pays burns first; a pushed floor cannot make it pay more per unit than it holds per unit, and it holds
    /// the mint price. Mallory holds pool-bought wX, gets the floor pushed +30%, burns: every burn is capped at the reserve's
    /// backing per unit, and the reserve's backing per unit never falls.
    function test_pushUpThenBurn_theReservePaysAtMostWhatItHoldsPerUnit() public {
        _mintMany(alice, 8);                                    // 6 convertible + 2 reserve at F x 1.0015
        for (uint256 i = 0; i < 5; ++i) _deposit(mallory);     // Mallory's 5 wX, bought in the pool at F
        uint256 betaS0 = module.betaReserve();
        uint256 s0 = module.reserve();
        uint256 b0 = module.backing();
        vm.prank(mallory);
        wrapper.approve(address(module), type(uint256).max);
        uint256 got;
        for (uint256 h = 0; h < 6; ++h) {
            _skip(1 hours);
            _post(F * 13 / 10);
            (uint256 price, uint256 room) = module.burnCapacity();
            uint256 u = room < wrapper.balanceOf(mallory) ? room : wrapper.balanceOf(mallory);
            if (u == 0 || price == 0) continue;
            assertLe(price, module.beta(), "never above the tier's backing per unit");
            vm.prank(mallory);
            got += module.burn(u, 0, mallory);
            if (module.reserveUnits() != 0) assertGe(module.betaReserve(), betaS0, "the reserve's backing per unit never falls");
            _books();
        }
        // pushed, but only to the day's bound (±20% of the day's anchor): 1.05^6 would be +34% (B-02)
        assertGe(module.accepted(), F * 12 / 10 - 1, "the floor was pushed to the day's bound");
        assertLe(module.accepted(), F * 12 / 10, "and no further");
        assertLe(got, s0 + b0 - module.reserve() - module.backing(), "she got at most what left the pots");
        assertLe(got, 5 * betaS0 * 10_015 / 10_000, "and at most the mint price per unit, which is what the pots hold");
    }

    // ---- the fills price (§13.4) ----

    function test_beforeAnyFillTheKeeperFloorAlone() public view {
        assertEq(module.fillCount(), 0);
        assertEq(module.price(), module.accepted());
    }

    function test_aPurchaseAtTheCapMovesThePriceNowhere() public {
        _mint(alice, 4 * module.mintPrice());
        _convert(module.betaConvertible() - 0.03 ether);       // a cheap one: revenue for the next one's allowance
        _skip(25 hours);                                        // ... and out of the window
        _post(F);
        assertEq(module.price(), F);
        _convert(module.itemCap());                             // the canister paid the whole allowance
        assertEq(module.fillCount(), 2);
        assertEq(module.fill(0).price, F, "recorded net of the allowance: the floor it implies");
        _skip(6 hours);
        _post(F);
        assertEq(module.price(), F, "an honest canister at the cap: no drift");
    }

    function test_fillsMoveThePriceWithinTheBand_timeWeighted() public {
        _mint(alice, 4 * module.mintPrice());
        uint256 cheap = F * 97 / 100;                           // bought 3% under: implies a floor 3%/1.03 under
        _convert(cheap);
        uint256 implied = cheap * 10_000 / 10_300;
        assertEq(module.fill(0).price, implied);
        assertEq(module.price(), F, "at the fill: the 24 h average is still the floor (the fill has held for no time)");
        // half a window later the average is halfway between the floor (before the fill) and the fill
        _skip(12 hours);
        _post(F);
        uint256 expected = (F * 12 hours + implied * 12 hours) / 24 hours;
        assertApproxEqAbs(module.price(), expected, 1e9, "time-weighted: 12 h of the floor, 12 h of the fill");
        assertLt(module.price(), F);
        // a day later the fill is (nearly) the whole window: clamped to the band
        _skip(11 hours + 59 minutes);
        _post(F);
        uint256 lo = F * (10_000 - 300) / 10_000;
        assertEq(module.price(), lo, "clamped to minus 3%");
        (uint256 sMint, uint256 sBurn) = module.spreads();
        assertEq(module.mintPrice(), _ceil(F * (10_000 + sMint), 10_000), "the lag: mints price at the higher of the fills price and the target");
        assertEq(module.burnPrice(), lo * (10_000 - sBurn) / 10_000, "burns at the lower");
        // once the fill leaves the window the keeper floor alone again
        _skip(2 minutes);
        _post(F);
        assertEq(module.price(), F);
    }

    function test_theBandBoundsWhatFillsCanDo_andTheCapStaysOnTheKeeperFloor() public {
        _withTreasury();
        for (uint256 i = 0; i < 3; ++i) _convert(module.betaConvertible());
        // the treasury's inventory sells high: a custodian reporting proceeds far above the floor
        _fundFloat(2 * module.mintPrice());
        vm.deal(custodian, 10 ether);
        vm.prank(custodian);
        module.itemSold{value: F * 2}(F * 2);
        _skip(23 hours + 59 minutes);
        _post(F);
        assertEq(module.price(), _ceil(F * 10_300, 10_000), "at most +3% of the accepted floor, whatever the fills say");
        assertEq(module.itemCap(), F * 10_300 / 10_000, "the canister's cap is the keeper's floor, not our fills");
        assertEq(module.minSaleProceeds(), _ceil(F * 9_700, 10_000));
    }

    function test_theRingKeepsEightFills() public {
        _redeployWith(2_500, 40e18, 40e18);
        _mint(alice, 12 * module.mintPrice());
        for (uint256 i = 0; i < 7; ++i) _convert(module.betaConvertible());
        assertEq(module.fillCount(), 7);
        module.fill(6);
        vm.expectRevert(FloorModule.BadInput.selector);
        module.fill(7);
    }

    // ---- sales (§13.4) ----

    /// A treasury for this suite: any address the module was built with; the module here has none, so redeploy with one and
    /// give it eight units of public minting.
    function _withTreasury() internal {
        moduleTreasury = makeAddr("treasury2");
        _redeploy(_params());
        _mintMany(alice, 8);
    }

    function _fundFloat(uint256 eth) internal {
        if (module.treasury() == address(0)) _withTreasury();
        vm.deal(moduleTreasury, moduleTreasury.balance + eth);
        vm.prank(moduleTreasury);
        module.fundFloat{value: eth}();
    }

    function test_itemSoldTakesAFloatUnit_andPaysTheReserve() public {
        _withTreasury();
        _fundFloat(2 * module.mintPrice());
        assertEq(module.floatUnits(), 2e18);
        uint256 f = module.floatUnits();
        uint256 s = module.reserve();
        uint256 min = module.minSaleProceeds();
        vm.deal(custodian, 10 ether);
        vm.startPrank(custodian);
        vm.expectRevert(abi.encodeWithSelector(FloorModule.UnderSalePrice.selector, min - 1, min));
        module.itemSold{value: min - 1}(min - 1);
        vm.expectRevert(FloorModule.BadInput.selector);
        module.itemSold{value: min - 1}(min);
        module.itemSold{value: min}(min);
        vm.stopPrank();
        assertEq(module.floatUnits(), f - 1e18);
        assertEq(wrapper.balanceOf(custodian), 1e18, "the custodian holds the unit it bought");
        assertEq(module.reserve(), s, "the basis is untouched");
        assertEq(module.surplus(), min, "the proceeds are surplus");
        assertEq(module.fill(0).price, _ceil(min * 10_000, 9_700) > F ? F : min * 10_000 / 9_700, "a sale fill, gross of the allowance");
        _books();
    }

    function test_itemSoldRefusals() public {
        vm.deal(custodian, 10 ether);
        vm.deal(alice, 10 ether);
        uint256 min = module.minSaleProceeds();
        vm.prank(alice);
        vm.expectRevert(FloorModule.NotCustodian.selector);
        module.itemSold{value: min}(min);
        vm.prank(custodian);
        vm.expectRevert(FloorModule.NoInventory.selector);
        module.itemSold{value: min}(min);
        _withTreasury();
        min = module.minSaleProceeds();
        _fundFloat(2 * module.mintPrice());
        _skip(module.FLOOR_TTL() + 1 minutes);
        vm.prank(custodian);
        vm.expectRevert(FloorModule.Stale.selector);
        module.itemSold{value: min}(min);
    }

    function test_strayReclaimNeverTakesTheFloats() public {
        _withTreasury();
        _fundFloat(2 * module.mintPrice());
        uint256 f = module.floatUnits();
        vm.prank(custodian);
        assertEq(module.reclaimStray(), 0);
        assertEq(wrapper.balanceOf(address(module)), f);
        _deposit(address(module));                              // a stray unit
        vm.prank(custodian);
        assertEq(module.reclaimStray(), 1e18);
        assertEq(module.floatUnits(), f);
    }

    // ---- floats (§13.3): a mint to the module with no reserve share ----

    function test_aFloatIsAllConvertible_andOnlyTheTreasuryFundsIt() public {
        _withTreasury();
        vm.deal(alice, 10 ether);
        vm.prank(alice);
        vm.expectRevert(FloorModule.NotTreasury.selector);
        module.fundFloat{value: 1 ether}();
        uint256 c0 = module.netUnits();
        uint256 r0 = module.reserveUnits();
        uint256 b0 = module.backing();
        uint256 eth = 2 * module.mintPrice();
        _fundFloat(eth);
        assertEq(module.reserveUnits(), r0, "no reserve share on a float");
        assertEq(module.netUnits(), c0 + 2e18);
        assertEq(module.backing(), b0 + eth, "all of it convertible");
        assertGt(module.floatUnits(), 0);
        (uint256 sMint,) = module.spreads();
        uint256 stressUnits = module.netUnits() - module.floatUnits();
        assertEq(sMint, 15 + (300 * stressUnits + 10e18 - 1) / 10e18, "floats do not stress the public spread");
        // the canister converts float units as any convertible unit
        _convert(module.betaConvertible());
        assertGt(module.itemsArrived(), 0);
        _books();
    }

    function test_aFloatIsBoundedByTheHardCap_andNeedsAFreshFloor() public {
        _withTreasury();
        _fundFloat(2 * module.mintPrice());
        vm.deal(moduleTreasury, 100 ether);
        uint256 tooMuch = 40 * module.mintPrice();
        vm.startPrank(moduleTreasury);
        vm.expectRevert(FloorModule.OverBacked.selector);
        module.fundFloat{value: tooMuch}();
        _skip(module.FLOOR_TTL() + 1 minutes);
        vm.expectRevert(FloorModule.Stale.selector);
        module.fundFloat{value: 1 ether}();
        vm.stopPrank();
    }

    // ---- fundReserve without a treasury: all of it is reserve ----

    /// B-02: `fundReserve` (the coins' reserve slices, the treasury's allocation) joins the surplus, which no burn is paid from.
    function test_fundReserveJoinsTheSurplus_notTheUnitsBacking() public {
        vm.deal(alice, 1 ether);
        vm.prank(alice);
        module.fundReserve{value: 1 ether}();
        assertEq(module.surplus(), 1 ether);
        assertEq(module.reserve(), 0);
        assertEq(module.reserveUnits(), 0, "no units");
        assertEq(module.loanOutstanding(), 0);
        (uint256 price, uint256 room) = module.burnCapacity();
        assertEq(room, 0, "nothing to burn yet");
        price;
        uint256 mp = module.mintPrice();
        _mint(alice, 4 * mp);
        assertLe(module.betaReserve(), mp, "the surplus does not back the new reserve units");
        _books();
    }

    // ---- volume ----

    function test_volume7dRollsDaily() public {
        _mint(alice, 1 ether);
        assertEq(module.volume7d(), 1 ether);
        _burn(alice, 1e17);
        uint256 v = module.volume7d();
        assertGt(v, 1 ether);
        for (uint256 d = 0; d < 6; ++d) { _skip(1 days); _post(F); }
        assertEq(module.volume7d(), v, "still inside seven days");
        _skip(1 days); _post(F);
        assertEq(module.volume7d(), 0, "rolled off");
        _mint(alice, 0.5 ether);
        assertEq(module.volume7d(), 0.5 ether);
    }

    function _ceil(uint256 a, uint256 b) internal pure returns (uint256) { return (a + b - 1) / b; }
}
