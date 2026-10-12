// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {FloorBase} from "./FloorBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {IFloorWrapped} from "../../src/interfaces/IFloorWrapped.sol";

/// docs/v5/floor-module.md §6.3: every manipulation, at the tight base spread (15 bps). The spread is not what defends the
/// escrow; the move limit, the caps and the backing cap are, and each test here says which bound it is checking.
contract FloorAttacksTest is FloorBase {
    address mallory = makeAddr("mallory");

    /// Honest minters put `n` hours × 4 units of ETH-backed wX in at the calm floor.
    function _honest(uint256 hours_) internal {
        for (uint256 i = 0; i < hours_; ++i) {
            _mint(i % 2 == 0 ? alice : bob, 4 * module.mintPrice());
            _skip(1 hours);
            _post(module.accepted());
        }
    }

    /// Mallory holds `n` item-backed wX, as if bought in the pool at the floor.
    function _malloryBuysInThePool(uint256 n) internal {
        for (uint256 i = 0; i < n; ++i) _deposit(mallory);
    }

    /// The keeper (compromised, or fed a pushed median) posts `target` each hour for `hours_` hours; Mallory burns what the
    /// cap allows each hour. Returns what she received.
    function _pushAndBurn(uint256 target, uint256 hours_) internal returns (uint256 got, uint256 burnt) {
        vm.prank(mallory);
        wrapper.approve(address(module), type(uint256).max);
        for (uint256 h = 0; h < hours_; ++h) {
            _skip(1 hours);
            _post(target);
            (, uint256 room) = module.burnCapacity();
            uint256 have = wrapper.balanceOf(mallory);
            uint256 u = room < have ? room : have;
            if (u == 0) continue;
            uint256 before = mallory.balance;
            vm.prank(mallory);
            module.burn(u, 0, mallory);
            got += mallory.balance - before;
            burnt += u;
            _books();
        }
    }

    // ---- push up, then burn ----

    /// The coordinator's requirement: a tight spread must not open the push-up-then-burn drain. Honest minters leave 8 units of
    /// backing at F × 1.0015; Mallory holds 5 wX bought at F, gets the floor pushed +30% (as fast as the limit allows), and burns.
    /// (Her burns take ETH-backed units out whoever minted them: wX is fungible. The honest holders' wX is then backed by the
    /// items her pool purchases had been backed by.)
    function test_pushUpThenBurn_aTightSpreadOpensNoDrain() public {
        _honest(2);
        _malloryBuysInThePool(5);
        uint256 beta0 = module.beta();
        uint256 b0 = module.backing();
        uint256 e0 = wrapper.ethBacked();
        (uint256 got, uint256 burnt) = _pushAndBurn(F * 13 / 10, 6);
        assertEq(burnt, 5e18, "she burnt everything she had");
        // the escrow: never paid a unit more than its backing, and the units left are backed as well as before
        assertLe(got, burnt * beta0 / 1e18, "paid at most the backing per unit");
        assertGe(module.beta(), beta0, "the backing per unit of what is left did not fall");
        assertEq(module.backing() + got + module.revenue(), b0, "every wei out of the backing is accounted: paid or kept");
        assertEq(wrapper.ethBacked(), e0 - burnt);
        // her gain against the pool: at most the spread honest minters left behind (0.15% of F per unit)
        uint256 cost = 5 * F;
        assertLe(got, cost + 5 * (beta0 - F), "no more than the minters' spread over F, whatever the floor did");
        _books();
    }

    /// The same drain with the volatility weight at ZERO: the backing cap alone must hold it.
    function test_pushUpThenBurn_theBackingCapAloneHolds_withNoVolatilityWeight() public {
        _redeployWith(0);
        _honest(2);
        _malloryBuysInThePool(5);
        uint256 beta0 = module.beta();
        (uint256 got, uint256 burnt) = _pushAndBurn(F * 2, 6);
        assertLe(got, burnt * beta0 / 1e18, "the backing per unit is the ceiling");
        assertGe(module.beta(), beta0);
        _books();
    }

    /// Within the push's hour, the volatility term cancels the push: burning after a +5% push pays no more than before it.
    function test_pushUpThenBurn_theVolatilityTermCancelsThePushWithinTheHour() public {
        _honest(1);
        _walkTo(F * 9 / 10);                       // the real floor fell 10%: backing (≈ F) is above it, so the floor binds
        _skip(1 hours);
        _post(module.accepted());
        _skip(1 hours);
        _post(module.accepted());                  // calm at 0.9 F
        uint256 before = module.burnPrice();
        _skip(1 hours);
        _post(F);                                  // pushed: +5%
        uint256 after_ = module.burnPrice();
        assertLe(after_, before, "F(1+d)(1 - s - d) <= F(1 - s)");
    }

    /// Fuzzed: a trader alone can never take ETH out of the module, whatever the keeper posts and whenever they mint and burn.
    function testFuzz_aLoneTraderNeverProfits(uint256[6] memory targets, uint256[6] memory mints, uint256[6] memory burns) public {
        uint256 start = 1_000 ether;
        vm.deal(mallory, start);
        vm.prank(mallory);
        wrapper.approve(address(module), type(uint256).max);
        for (uint256 i = 0; i < 6; ++i) {
            _skip(bound(targets[i] % 7, 1, 6) * 10 minutes);
            _post(bound(targets[i], F / 3, F * 3));
            (uint256 price, uint256 maxIn) = module.mintCapacity();
            uint256 m = bound(mints[i], 0, maxIn);
            if (price != 0 && m * 1e18 >= price) {
                vm.prank(mallory);
                module.mint{value: m}(mallory, 0);
            }
            (uint256 bp, uint256 room) = module.burnCapacity();
            uint256 have = wrapper.balanceOf(mallory);
            uint256 b = bound(burns[i], 0, room < have ? room : have);
            if (bp != 0 && b * bp >= 1e18) {
                vm.prank(mallory);
                module.burn(b, 0, mallory);
            }
            _books();
        }
        // what she still holds is worth at most its backing at the module
        uint256 left = wrapper.balanceOf(mallory);
        uint256 value = mallory.balance + left * module.beta() / 1e18;
        assertLe(value, start, "no sequence of posts, mints and burns returns more than was put in");
    }

    // ---- push down, then mint ----

    /// A 5% push-down within the hour gains at most about d²: the volatility term widens the mint side by the move.
    function test_pushDownThenMint_gainsAtMostDeltaSquaredWithinTheHour() public {
        uint256 calmPrice = module.mintPrice();              // F × 1.0015
        _skip(1 hours);
        _post(F / 2);                                        // pushed down: accepted −5%
        uint256 pushed = module.mintPrice();
        // F(1 − d)(1 + s + d) against F(1 + s): d(d + s) = 0.2575% cheaper at d = 5%, s = 0.15%
        assertGe(pushed * 10_000, calmPrice * (10_000 - 27), "at most d(d + s) cheaper than before the push");
    }

    /// Sustained (a median held down for hours by a compromised keeper): the loss is bounded by the public ETH-backed cap.
    function test_pushDownThenMint_sustained_isBoundedByThePublicCap() public {
        uint256 units;
        uint256 paid;
        vm.deal(mallory, 100 ether);
        for (uint256 h = 0; h < 12; ++h) {
            _skip(1 hours);
            _post(F / 10);
            (uint256 price, uint256 maxIn) = module.mintCapacity();
            if (maxIn == 0) continue;
            vm.prank(mallory);
            units += module.mint{value: maxIn}(mallory, 0);
            paid += maxIn;
        }
        assertLe(units, 10e18, "never more than maxPublicEthBacked");
        // the stated bound: maxPublicEthBacked × F × (1 − 0.95^h) against the true floor
        uint256 fairValue = units * F / 1e18;
        uint256 loss = fairValue > paid ? fairValue - paid : 0;
        assertLe(loss, 10 * F * (10_000 - 5_404) / 10_000, "0.95^12 = 0.5404: at most 10 x F x 46%");
        assertGt(paid, 0);
        // and nothing leaves the escrow: burning it back pays at most what she paid
        vm.startPrank(mallory);
        wrapper.approve(address(module), type(uint256).max);
        vm.stopPrank();
        _skip(1 hours);
        _post(F);                                            // the keeper recovers: +5% an hour only
        (uint256 bp,) = module.burnCapacity();
        assertLe(bp, module.beta(), "her burns are capped at what she paid in");
        _books();
    }

    // ---- stale prices and the sequencer ----

    function test_stale_nothingTradesAfterThreeHours_onEitherClock() public {
        _mint(alice, 1 ether);
        _skip(module.FLOOR_TTL());
        (uint256 p, uint256 maxIn) = module.mintCapacity();
        assertEq(p, 0);
        assertEq(maxIn, 0);
        (uint256 bp, uint256 room) = module.burnCapacity();
        assertEq(bp, 0);
        assertEq(room, 0);
        assertEq(module.itemCap(), 0);
        assertEq(module.quoteMint(1 ether), 0);
        // a parent chain that stalls while time passes: stale by the timestamp
        _post(F);
        vm.warp(vm.getBlockTimestamp() + module.FLOOR_TTL());
        assertFalse(module.fresh());
    }

    function test_theSequencerCannotRefillACapOrDecayTheSpreadEarly() public {
        _skip(1 hours);
        _post(F * 2);                                        // +5%: vol 500
        _mint(alice, 5 * module.mintPrice());                // the hour's mints used up
        (, uint256 s0) = module.spreads();
        _skipTimeOnly(1 hours);
        _post(module.accepted());                            // fresh again by timestamp? no parent blocks: stale spread aside,
        (, uint256 s1) = module.spreads();                   // the volatility has not decayed
        assertGe(s1, s0, "decay counts the slower clock");
        (, uint256 room) = module.mintCapacity();
        assertEq(room, 0, "no refill on the sequencer's word");
    }

    // ---- a compromised keeper, both directions ----

    function test_compromisedKeeper_cannotDrainThroughBurns_evenOverADay() public {
        _honest(2);
        _malloryBuysInThePool(10);
        uint256 beta0 = module.beta();
        uint256 b0 = module.backing();
        (uint256 got, uint256 burnt) = _pushAndBurn(F * 100, 24);
        assertLe(got, burnt * beta0 / 1e18, "a day of +5%/h posts: still the backing per unit, at most");
        assertLe(got, b0, "and never more than the backing there was");
        _books();
    }

    function test_graduationsCannotBeMadeCheapByALowTarget() public {
        uint256 g0 = module.gradPrice();
        _skip(10 minutes);
        _post(F / 100);                                      // target far below; accepted moves 5% x 10/60 only
        assertGe(module.gradPrice() * 10_000, g0 * (10_000 - 84), "at most the limit's step cheaper");
    }

    // ---- helpers ----

    function _redeployWith(uint16 vw) internal {
        FloorModule.Params memory p = _params();
        p.volWeightBps = vw;
        _redeploy(p);
    }
}

