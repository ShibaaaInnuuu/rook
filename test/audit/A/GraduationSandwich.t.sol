// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {PoolKey} from "@uniswap/v4-core/src/types/PoolKey.sol";
import {TickMath} from "@uniswap/v4-core/src/libraries/TickMath.sol";
import {StateLibrary} from "@uniswap/v4-core/src/libraries/StateLibrary.sol";
import {IPoolManager} from "@uniswap/v4-core/src/interfaces/IPoolManager.sol";
import {AuditABase} from "./AuditABase.sol";
import {Swapper} from "../../utils/PoolBase.sol";
import {SweepVault} from "../../../src/SweepVault.sol";
import {FrogCurve} from "../../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../../src/curve/CurveToken.sol";
import {LiquidityLock} from "../../../src/pools/LiquidityLock.sol";

/// Regression for A-04: the graduate() sandwich. The shared add used to be accepted anywhere within 1,000 ticks of the keeper's
/// sweep cap (which in production sits 10% above the floor). In a stack with a floor module it is now accepted only within
/// `REF_DEVIATION` (200) ticks of the module's fresh price, so the audit's ±990-tick push finds the add deferred and loses its
/// round-trip fees, and a push inside the new guard does not pay.
contract GraduationSandwichTest is AuditABase {
    using StateLibrary for IPoolManager;

    Swapper atk;

    function setUp() public override {
        super.setUp();
        atk = new Swapper(PM);
        vm.deal(address(atk), 100 ether);
        _wrappedFor(address(atk), 20);
        (FrogCurve c1,) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        _fillCurve(c1);
        c1.graduate();
    }

    function _value() internal view returns (uint256) {
        return address(atk).balance + Math.mulDiv(wrapper.balanceOf(address(atk)), CAP, 1e18);
    }

    function _refTick() internal pure returns (int24) {
        return TickMath.getTickAtSqrtPrice(uint160(Math.sqrt(Math.mulDiv(1e18, 2 ** 192, CAP))));
    }

    /// push the shared price `ticks` off the reference, graduate someone else's coin, swap back. Returns the P&L at the cap.
    function _run(bool pushWxCheap, int24 ticks, bool graduate) internal returns (int256 pnl, FrogCurve c2, CurveToken t2) {
        (c2, t2) = _launch(SweepVault.Policy.BURN, false);
        _skipBoth(20 minutes);
        _postCap(curveRaffle, CAP);
        _floor(CAP);
        _fillCurve(c2);
        PoolKey memory key = lock.sharedKey();
        (uint160 s0,,,) = PM.getSlot0(key.toId());
        uint256 v0 = _value();
        int24 edge = pushWxCheap ? _refTick() + ticks : _refTick() - ticks;
        atk.swap(key, !pushWxCheap, pushWxCheap ? 20e18 : 50 ether, TickMath.getSqrtPriceAtTick(edge));
        if (graduate) c2.graduate();
        atk.swap(key, pushWxCheap, pushWxCheap ? 50 ether : 20e18, s0);
        pnl = int256(_value()) - int256(v0);
        emit log_named_int("attacker P&L, wei at the cap", pnl);
    }

    function test_A04_theReferenceIsTheModulesFloor() public view {
        assertEq(address(lock.floorRef()), address(module), "found at wire: the factory's queue is a floor module");
        (uint256 p, int24 dev, bool ok) = lock.priceReference();
        assertTrue(ok);
        assertEq(p, module.price());
        assertEq(dev, lock.REF_DEVIATION());
    }

    /// The audit's push (990 ticks): the add is deferred whole, graduation completes, and the sandwich only pays fees.
    function test_A04_theAuditsPush_findsTheAddDeferred_wXCheap() public { _deferred(true); }
    function test_A04_theAuditsPush_findsTheAddDeferred_wXDear() public { _deferred(false); }

    function _deferred(bool pushWxCheap) internal {
        (int256 pnl, FrogCurve c2, CurveToken t2) = _run(pushWxCheap, 990, true);
        assertEq(uint8(c2.state()), uint8(FrogCurve.State.Graduated), "graduation does not wait");
        assertEq(lock.pendingEth(address(t2)), c2.poolEth(), "no shared add at the skewed price");
        assertLt(pnl, 0, "the sandwich loses its fees (before the fix: +0.011 to +0.013 ETH)");
        // back at a fair price, anyone adds it
        lock.deploy(address(t2));
        assertLt(lock.pendingEth(address(t2)), c2.poolEth() / 100);
    }

    /// Inside the new guard (190 ticks, ~1.9%): the add happens, and the sandwich does not beat its own control by more than
    /// dust. Measured against the same round trip with no graduation in between.
    function test_A04_aPushInsideTheGuard_doesNotPay_wXCheap() public { _inside(true); }
    function test_A04_aPushInsideTheGuard_doesNotPay_wXDear() public { _inside(false); }

    function _inside(bool pushWxCheap) internal {
        uint256 snap = vm.snapshotState();
        (int256 control,,) = _run(pushWxCheap, 190, false);
        vm.revertToState(snap);
        (int256 pnl, FrogCurve c2, CurveToken t2) = _run(pushWxCheap, 190, true);
        assertEq(uint8(c2.state()), uint8(FrogCurve.State.Graduated));
        assertLt(lock.pendingEth(address(t2)), c2.poolEth() / 100, "the add happened");
        emit log_named_int("sandwich P&L minus its control, wei", pnl - control);
        assertLt(pnl, 0.0005 ether, "no meaningful profit (before the fix: ~0.012 ETH at the 990-tick edge)");
    }

    /// Control, kept from the PoC: the same round trip with no graduation in the middle (fees only).
    function test_A04_control_roundTripOnly() public {
        PoolKey memory key = lock.sharedKey();
        (uint160 s0,,,) = PM.getSlot0(key.toId());
        uint256 v0 = _value();
        atk.swap(key, true, 50 ether, TickMath.getSqrtPriceAtTick(_refTick() - 990));
        atk.swap(key, false, 20e18, s0);
        int256 pnl = int256(_value()) - int256(v0);
        emit log_named_int("round trip P&L, wei at the cap", pnl);
        assertLt(pnl, 0);
    }
}

/// A-04, the production shape: the keeper posts the sweep cap at floor + 10% (docs/v6/p1-services.md §1), so the cap is ~953
/// ticks off a fair pool. The lock now takes the module's floor as its reference: the first add aligns the empty pool to the
/// floor, the center sits on the floor's tick, and the add is made.
contract ProductionCapMarginTest is AuditABase {
    function test_A04_aCapTenPercentAboveTheFloor_theLayersCenterOnTheFloor() public {
        uint256 floorWei = CAP;
        _postCap(curveRaffle, floorWei * 11 / 10);
        _floor(floorWei);
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        _postCap(curveRaffle, floorWei * 11 / 10);
        _floor(floorWei);
        _fillCurve(c);
        c.graduate();
        assertLt(lock.pendingEth(address(tok)), 1e15, "the shared add was made");
        int24 floorTick = TickMath.getTickAtSqrtPrice(uint160(Math.sqrt(Math.mulDiv(1e18, 2 ** 192, floorWei))));
        int24 capTick = TickMath.getTickAtSqrtPrice(uint160(Math.sqrt(Math.mulDiv(1e18, 2 ** 192, floorWei * 11 / 10))));
        int24 center = lock.center();
        assertTrue(center <= floorTick && center > floorTick - 60, "centered on the floor's tick");
        assertGt(center - capTick, 900, "not on the cap's, ~953 ticks away");
    }
}
