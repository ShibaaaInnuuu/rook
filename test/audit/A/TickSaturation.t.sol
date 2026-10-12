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
import {AuditABase, LP} from "./AuditABase.sol";
import {Swapper} from "../../utils/PoolBase.sol";
import {SweepVault} from "../../../src/SweepVault.sol";
import {FrogCurve} from "../../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../../src/curve/CurveToken.sol";
import {FloorModule} from "../../../src/floor/FloorModule.sol";
import {LiquidityLock} from "../../../src/pools/LiquidityLock.sol";

/// Regression for A-01 (docs/audit/2026-09-26/A-curve.md, fix: fixes-A-graduation.md). The audit's attack — filling v4's
/// per-tick liquidity cap at the full-range tick the lock always used — no longer freezes anything:
///  1. no lock position is bounded by the usable-tick extremes any more (the widest layer is ±FAR_HALF), so the audit's
///     position does not touch the lock, and saturating the new bounds costs billions of ETH;
///  2. even a saturated bound does not revert graduate(): the coin seed and the shared add fail soft, and anyone retries;
///  3. a curve that has not graduated window + STUCK_GRACE after its close can open refunds even when filled or SWEEP_ONLY.
contract TickSaturationTest is AuditABase {
    using StateLibrary for IPoolManager;

    Swapper attacker;
    LP lp;

    function setUp() public override {
        super.setUp();
        attacker = new Swapper(PM);
        vm.deal(address(attacker), 1 ether);
        lp = new LP(PM);
    }

    function _sqrtAtCap() internal pure returns (uint160) {
        return uint160(Math.sqrt(Math.mulDiv(1e18, 2 ** 192, CAP)));
    }

    /// The audit's attack, verbatim: one ETH-only position at the top usable tick of the shared pool, max per-tick liquidity.
    function _saturateSharedAtTheOldBound() internal returns (uint256 cost) {
        PoolKey memory key = lock.sharedKey();
        (uint160 s,,,) = PM.getSlot0(key.toId());
        if (s == 0) PM.initialize(key, _sqrtAtCap());
        int24 maxT = TickMath.maxUsableTick(key.tickSpacing);
        uint128 lmax = Pool.tickSpacingToMaxLiquidityPerTick(key.tickSpacing);
        uint256 b0 = address(attacker).balance;
        attacker.addLiquidity(key, maxT - key.tickSpacing, maxT, lmax);
        cost = b0 - address(attacker).balance;
    }

    function _graduateOne(SweepVault.Policy policy) internal returns (FrogCurve c, CurveToken tok) {
        (c, tok) = _launch(policy, false);
        _skipBoth(20 minutes);
        _postCap(curveRaffle, CAP);
        _floor(CAP);
        _fillCurve(c);
        c.graduate();
    }

    /// What the lock's share of `tick` is not: the room left under the per-tick cap there.
    function _room(PoolKey memory key, int24 tick) internal view returns (uint128) {
        (uint128 gross,) = PM.getTickLiquidity(key.toId(), tick);
        return Pool.tickSpacingToMaxLiquidityPerTick(key.tickSpacing) - gross;
    }

    // ---- 1. the audit's position no longer touches the lock -------------------------------------------------------------

    function test_A01_theOldBoundSaturation_noLongerBlocksGraduation() public {
        uint256 cost = _saturateSharedAtTheOldBound();
        assertLt(cost, 1e13, "the audit's attack is still as cheap ...");

        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        _fillCurve(c);
        assertTrue(module.isFilled(address(tok)));
        c.graduate();                                           // ... but it no longer blocks anything
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated));
        assertLt(lock.pendingEth(address(tok)), 1e15, "the shared add happened (a few wei of margin stay pending, as ever)");
        assertEq(lock.pendingCoinToken(address(tok)), 0, "the coin pool was seeded");
        int24[2][4] memory r = lock.layers();
        assertEq(r[2][1] - r[2][0], 2 * lock.FAR_HALF(), "the far layer is +-FAR_HALF, not full range");
        assertLt(r[2][1], TickMath.maxUsableTick(60));
        assertGt(r[2][0], TickMath.minUsableTick(60));
        (int24 lo, int24 hi) = lock.coinRange(address(tok));
        assertEq(hi - lo, 2 * lock.FAR_HALF());
        assertEq(module.gradUnits(), 0, "the units were released to the pools");
    }

    /// Before the fix: one position froze every later coin of the collection. Now each graduates.
    function test_A01_onePositionNoLongerBlocksLaterCoins() public {
        _saturateSharedAtTheOldBound();
        for (uint256 i = 0; i < 3; ++i) {
            (FrogCurve c,) = _graduateOne(SweepVault.Policy.BURN);
            assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated));
        }
    }

    /// Control, kept from the PoC: the same flow graduates with no attack at all.
    function test_A01_control_graduatesWithoutTheAttack() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        _fillCurve(c);
        c.graduate();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated));
    }

    /// The audit's SWEEP_ONLY variant: the COIN/ETH pool opened at another price and saturated at its top usable tick. A-06 as
    /// well: `coinKey` names the COIN/ETH pool before the seed.
    function test_A01_sweepOnlyOldBoundSaturation_noLongerFreezesTheRaise() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, true);
        PoolKey memory key = lock.coinKey(address(tok));
        assertEq(Currency.unwrap(key.currency0), address(0), "A-06: coinKey names COIN/ETH before the seed");
        assertEq(Currency.unwrap(key.currency1), address(tok));
        PM.initialize(key, TickMath.getSqrtPriceAtTick(0));
        int24 maxT = TickMath.maxUsableTick(key.tickSpacing);
        attacker.addLiquidity(key, maxT - key.tickSpacing, maxT, Pool.tickSpacingToMaxLiquidityPerTick(key.tickSpacing));

        _skipBoth(20 minutes);
        _fillCurve(c);
        c.graduate();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated));
        (,, uint128 liq) = lock.coins(address(tok));
        assertGt(liq, 0, "the coin's pool was seeded");
        assertEq(lock.pendingCoinToken(address(tok)), 0);
    }

    // ---- 2. saturating the NEW bounds: prohibitive, and still not a freeze ---------------------------------------------

    /// The shared pool's far layer, once placed, is bounded ~180,000 ticks from the price. Filling the tick there costs
    /// ~3.4e9 ETH (measured). Even an attacker who pays it does not block graduation: the add is deferred and retried.
    function test_A01_saturatingTheNewSharedBound_costsBillions_andOnlyDefersTheAdd() public {
        _graduateOne(SweepVault.Policy.RAFFLE);                 // places the layers
        PoolKey memory key = lock.sharedKey();
        int24 hi = lock.farUpper();
        uint128 room = _room(key, hi);
        vm.deal(address(lp), 1e32);
        (uint256 paid0, uint256 paid1) = lp.modify(key, hi, hi + key.tickSpacing, int256(uint256(room)));
        emit log_named_decimal_uint("ETH to fill the far layer's upper tick", paid0, 18);
        assertEq(paid1, 0, "above the price: ETH only");
        assertGt(paid0, 1e27, "more than a billion ETH (the audit's attack cost 1.9e12 wei)");

        (FrogCurve c2, CurveToken t2) = _launch(SweepVault.Policy.BURN, false);
        _skipBoth(20 minutes);
        _postCap(curveRaffle, CAP);
        _floor(CAP);
        _fillCurve(c2);
        vm.expectEmit(true, false, false, true, address(lock));
        emit LiquidityLock.SharedDeferred(address(t2), 4);
        c2.graduate();
        assertEq(uint8(c2.state()), uint8(FrogCurve.State.Graduated), "graduation did not wait for the shared add");
        assertEq(lock.pendingEth(address(t2)), c2.poolEth(), "the shared contribution waits, whole");
        assertEq(lock.pendingCoinToken(address(t2)), 0, "the coin's own pool is open");

        // anyone may retry; while the tick is full, the retry says why
        vm.expectRevert(abi.encodeWithSelector(Pool.TickLiquidityOverflow.selector, hi));
        lock.deploy(address(t2));
        // once the attacker takes the position down (the only party who can), the retry goes through
        lp.modify(key, hi, hi + key.tickSpacing, -int256(uint256(room)));
        lock.deploy(address(t2));
        assertLt(lock.pendingEth(address(t2)), 1e15, "added (what cannot be paired waits, as before)");
    }

    /// A SWEEP_ONLY coin's own pool, saturated at its NEW upper bound (the seed price + FAR_HALF, known at launch): ~7e6 ETH.
    /// The seed is deferred, graduation completes, the raise waits in the lock, and `seedCoin` (anyone) opens the pool later.
    function test_A01_coinSeedFailure_defersThenAnyoneRetries() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, true);
        PoolKey memory key = lock.coinKey(address(tok));
        // the lock's seed price and range, as it computes them (ETH is currency0: price = coins per ETH)
        uint160 target = uint160(Math.sqrt(Math.mulDiv(c.reserved(), 2 ** 192, c.poolEth())));
        int24 t = TickMath.getTickAtSqrtPrice(target);
        int24 center = (t / 200) * 200;
        if (t < 0 && t % 200 != 0) center -= 200;
        int24 hi = center + lock.FAR_HALF();
        PM.initialize(key, target);
        vm.deal(address(lp), 1e32);
        uint128 lmax = Pool.tickSpacingToMaxLiquidityPerTick(200);
        (uint256 paid0,) = lp.modify(key, hi, hi + 200, int256(uint256(lmax)));
        emit log_named_decimal_uint("ETH to fill the coin pool's upper tick", paid0, 18);
        assertGt(paid0, 1e24, "more than a million ETH (the audit's attack cost 2.1e13 wei)");

        _skipBoth(20 minutes);
        _fillCurve(c);
        c.graduate();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated), "graduation does not depend on the add");
        assertEq(lock.pendingCoinToken(address(tok)), c.reserved(), "the reserved coins wait in the lock");
        assertEq(lock.pendingCoinQuote(address(tok)), c.poolEth(), "and the pool's ETH");
        assertGe(address(lock).balance, c.poolEth());
        (,, uint128 liq) = lock.coins(address(tok));
        assertEq(liq, 0);

        vm.expectRevert(abi.encodeWithSelector(Pool.TickLiquidityOverflow.selector, hi));
        lock.seedCoin(address(tok));
        lp.modify(key, hi, hi + 200, -int256(uint256(lmax)));
        uint256 feesBefore = c.platformOwed() + c.feesForVault() + c.creatorOwed();
        vm.prank(mallory);
        lock.seedCoin(address(tok));
        (,, liq) = lock.coins(address(tok));
        assertGt(liq, 0, "seeded by the retry");
        assertEq(lock.pendingCoinToken(address(tok)), 0);
        assertEq(lock.pendingCoinQuote(address(tok)), 0);
        assertGe(c.platformOwed() + c.feesForVault() + c.creatorOwed(), feesBefore, "ETH it could not pair went back as pool fees");
        assertGe(address(c).balance, _owed(c));
        vm.expectRevert(LiquidityLock.NothingPending.selector);
        lock.seedCoin(address(tok));
    }

    /// The self-calls that make fail-soft possible are the lock's own: nobody else can reach them.
    function test_A01_theSelfCallsRefuseEveryoneElse() public {
        vm.expectRevert(LiquidityLock.NotSelf.selector);
        lock.seedCoinPoolSelf(address(1), 1, 1);
        vm.expectRevert(LiquidityLock.NotSelf.selector);
        lock.deploySelf(address(1));
    }

    /// Fail-soft cannot be forced by starving the call of gas: at every gas limit, graduate() either reverts as a whole or
    /// completes with nothing deferred.
    function test_A01_noGasLimitDefersTheSeed() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        _fillCurve(c);
        uint256 completed;
        for (uint256 g = 400_000; g <= 4_000_000; g += 100_000) {
            uint256 snap = vm.snapshotState();
            (bool ok,) = address(c).call{gas: g}(abi.encodeWithSignature("graduate()"));
            if (ok) {
                completed++;
                assertEq(lock.pendingCoinToken(address(tok)), 0, "a completed graduation seeded the coin pool");
                assertLt(lock.pendingEth(address(tok)), 1e15, "and made the shared add");
            } else {
                assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduating), "a failed one changed nothing");
            }
            vm.revertToState(snap);
        }
        assertGt(completed, 0, "enough gas graduates");
    }

    // ---- 3. the escape hatch ------------------------------------------------------------------------------------------

    /// A filled curve whose graduate() cannot complete (an unforeseen failure, here a lock that reverts) is no longer frozen:
    /// window + STUCK_GRACE after its close, anyone opens refunds; the module releases the units bought for it, holders get the
    /// ETH here and those units pro rata, and nothing is paid twice (the budget stays in the module as their backing).
    function test_A01_escapeHatch_aFilledCurveThatCannotGraduateRefunds() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        _fillCurve(c);
        FloorModule.Grad memory g = module.grad(address(tok));
        assertEq(uint8(g.status), uint8(FloorModule.GradStatus.Filled));
        uint256 backingBefore = module.backing() + module.reserve();
        vm.mockCallRevert(address(lock), abi.encodeWithSelector(LiquidityLock.seed.selector), "unforeseen");
        vm.expectRevert(bytes("unforeseen"));
        c.graduate();
        assertTrue(c.canGraduate());

        _skipBoth(3 days + 1 hours);                            // past the normal deadline: still graduate()'s turn
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.openRefunds();
        (uint64 at, uint64 atBlock) = c.refundsOpenAt();
        assertEq(at, c.gradAt() + 3 days + c.STUCK_GRACE());
        vm.warp(at);
        vm.roll(atBlock);
        c.openRefunds();
        vm.clearMockedCalls();

        assertEq(uint8(c.state()), uint8(FrogCurve.State.Refunding));
        assertEq(c.refundWrappedPool(), g.units, "the units the budget bought");
        assertEq(c.refundEthPool(), c.poolEth() + c.platformFee(), "the raise less the budget: no graduation fee taken");
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Released));
        assertEq(module.gradUnits(), 0);
        assertEq(module.backing() + module.reserve(), backingBefore, "the budget stays as the units' backing: not paid twice");
        assertEq(wrapper.balanceOf(address(c)), g.units);
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.graduate();

        uint256 held = tok.balanceOf(buyer);
        assertEq(held, c.forSale(), "the one buyer holds every coin sold");
        uint256 ethBefore = buyer.balance;
        vm.startPrank(buyer);
        tok.approve(address(c), held);
        (uint256 e, uint256 w) = c.refund(held);
        vm.stopPrank();
        assertEq(e, c.refundEthPool());
        assertEq(w, g.units);
        assertEq(buyer.balance - ethBefore, e);
        assertEq(wrapper.balanceOf(buyer), g.units);
        assertGe(address(c).balance, _owed(c), "the fees owed are still there");
        c.flushVaultFees();
        c.flushProtocolFees();
        vm.prank(creator);
        c.claimCreatorFees();
        c.claimPlatformFees();
    }

    /// SWEEP_ONLY: there was no refund path at all. Now, stuck past window + STUCK_GRACE, the whole raise refunds.
    function test_A01_escapeHatch_aSweepOnlyCurveThatCannotGraduateRefundsItsRaise() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, true);
        _skipBoth(20 minutes);
        _fillCurve(c);
        vm.mockCallRevert(address(lock), abi.encodeWithSelector(LiquidityLock.seedEth.selector), "unforeseen");
        vm.expectRevert(bytes("unforeseen"));
        c.graduate();
        _skipBoth(3 days + 1 hours);
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.openRefunds();
        _skipBoth(4 days);
        c.openRefunds();
        vm.clearMockedCalls();
        assertEq(c.refundEthPool(), c.raise(), "the whole raise");
        assertEq(c.refundWrappedPool(), 0);
        uint256 held = tok.balanceOf(buyer);
        uint256 ethBefore = buyer.balance;
        vm.startPrank(buyer);
        tok.approve(address(c), held);
        c.refund(held);
        vm.stopPrank();
        assertEq(buyer.balance - ethBefore, c.raise());
        assertGe(address(c).balance, _owed(c));
    }

    /// Control: a healthy curve graduates at once, and once graduated no refund can be opened, however late.
    function test_A01_escapeHatch_control_aGraduatedCurveNeverRefunds() public {
        (FrogCurve c,) = _graduateOne(SweepVault.Policy.RAFFLE);
        assertFalse(c.canGraduate());
        _skipBoth(30 days);
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.openRefunds();
    }

    // ---- fuzz: graduate never reverts because of the liquidity adds -------------------------------------------------

    /// Whatever a third party does to the pools first — the audit's saturation, a pool opened at any price with liquidity in
    /// any range, the new far bound filled — graduate() completes.
    function testFuzz_A01_graduateNeverRevertsBecauseOfTheLiquidityAdds(uint8 which, int24 tick, int24 lo, uint96 liq, bool sweepOnly)
        public
    {
        if (!sweepOnly) _graduateOne(SweepVault.Policy.RAFFLE);  // the shared layers exist (so its far bound is known)
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.BURN, sweepOnly);
        PoolKey memory key = which % 2 == 0 ? lock.coinKey(address(tok)) : lock.sharedKey();
        if (sweepOnly) key = lock.coinKey(address(tok));
        int24 s = key.tickSpacing;
        (uint160 sp,,,) = PM.getSlot0(key.toId());
        if (sp == 0) PM.initialize(key, TickMath.getSqrtPriceAtTick(int24(bound(tick, -600_000, 600_000))));
        vm.deal(address(lp), 1e32);
        // the attacker holds whatever it needs of either currency (dealt), so every position below can really be placed
        if (Currency.unwrap(key.currency0) != address(0)) deal(Currency.unwrap(key.currency0), address(lp), 1e45);
        deal(Currency.unwrap(key.currency1), address(lp), 1e45);
        uint8 w = which % 4;
        if (w == 0) {
            // the audit's: max per-tick liquidity at the top usable tick
            int24 maxT = TickMath.maxUsableTick(s);
            lp.modify(key, maxT - s, maxT, int256(uint256(_room(key, maxT))));
        } else if (w == 1) {
            // the bottom usable tick (wX or coin side): the attacker has none, so it takes a dust-sized version
            int24 minT = TickMath.minUsableTick(s);
            try lp.modify(key, minT, minT + s, int256(uint256(liq))) {} catch {}
        } else if (w == 2 && !sweepOnly && Currency.unwrap(key.currency0) == address(0) && lock.farUpper() != 0) {
            int24 hi = lock.farUpper();
            lp.modify(key, hi, hi + s, int256(uint256(_room(key, hi))));
        } else {
            // any range, any size the attacker can pay in ETH
            int24 l = int24(bound(lo, TickMath.minUsableTick(s) / s, TickMath.maxUsableTick(s) / s - 1)) * s;
            try lp.modify(key, l, l + s, int256(uint256(liq))) {} catch {}
        }
        _skipBoth(20 minutes);
        _postCap(curveRaffle, CAP);
        _floor(CAP);
        _fillCurve(c);
        c.graduate();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated));
        assertGe(address(c).balance, _owed(c));
    }
}
