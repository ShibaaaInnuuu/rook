// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IPoolManager} from "@uniswap/v4-core/src/interfaces/IPoolManager.sol";
import {IUnlockCallback} from "@uniswap/v4-core/src/interfaces/callback/IUnlockCallback.sol";
import {PoolKey} from "@uniswap/v4-core/src/types/PoolKey.sol";
import {Currency} from "@uniswap/v4-core/src/types/Currency.sol";
import {BalanceDelta} from "@uniswap/v4-core/src/types/BalanceDelta.sol";
import {ModifyLiquidityParams} from "@uniswap/v4-core/src/types/PoolOperation.sol";
import {PoolBase, Swapper} from "../../utils/PoolBase.sol";
import {FloorModule} from "../../../src/floor/FloorModule.sol";
import {IFloorWrapped} from "../../../src/interfaces/IFloorWrapped.sol";
import {IKeeperSource} from "../../../src/interfaces/IKeeperSource.sol";
import {IItemQueue} from "../../../src/interfaces/IItemQueue.sol";
import {SweepVault} from "../../../src/SweepVault.sol";
import {FrogCurve} from "../../../src/curve/FrogCurve.sol";
import {CurveFactory} from "../../../src/curve/CurveFactory.sol";
import {CurveToken} from "../../../src/curve/CurveToken.sol";
import {LiquidityLock} from "../../../src/pools/LiquidityLock.sol";
import {EtchRouter} from "../../../src/pools/EtchRouter.sol";

/// A third-party LP that can add AND remove its own position (the audit's attacker, who may take the saturation down again once
/// paid, and the tests' way to show a deferred add going through after that). Test-only.
contract LP is IUnlockCallback {
    IPoolManager immutable pm;
    constructor(IPoolManager pm_) { pm = pm_; }
    receive() external payable {}

    /// Returns what the change cost in each currency (positive: paid in; a removal returns 0, 0).
    function modify(PoolKey memory key, int24 lo, int24 hi, int256 delta) external returns (uint256 paid0, uint256 paid1) {
        return abi.decode(pm.unlock(abi.encode(key, lo, hi, delta)), (uint256, uint256));
    }

    function unlockCallback(bytes calldata data) external returns (bytes memory) {
        require(msg.sender == address(pm));
        (PoolKey memory key, int24 lo, int24 hi, int256 delta) = abi.decode(data, (PoolKey, int24, int24, int256));
        (BalanceDelta d,) = pm.modifyLiquidity(key, ModifyLiquidityParams(lo, hi, delta, bytes32(0)), "");
        _settle(key.currency0, d.amount0());
        _settle(key.currency1, d.amount1());
        return abi.encode(d.amount0() < 0 ? uint256(-int256(d.amount0())) : 0, d.amount1() < 0 ? uint256(-int256(d.amount1())) : 0);
    }

    function _settle(Currency c, int128 delta) internal {
        if (delta < 0) {
            uint256 amount = uint256(-int256(delta));
            if (Currency.unwrap(c) == address(0)) { pm.settle{value: amount}(); }
            else { pm.sync(c); IERC20(Currency.unwrap(c)).transfer(address(pm), amount); pm.settle(); }
        } else if (delta > 0) {
            pm.take(c, address(this), uint256(int256(delta)));
        }
    }
}

/// Audit A harness (copied from docs/audit/2026-09-26/poc/A/AuditABase.sol for the regression tests): the Phase-1 collection stack as DeployStack.s.sol builds it — the factory's queue is the FLOOR MODULE (not
/// ItemQueue), the curve template is DeployStack's `_template()` (40/30/5/5 split, 1% fee, 50% launch fee decaying over 10 min),
/// the targets are `_targets()`, the module params are `StackChecks.defaultParams(1e18, 0)` (stage 0: 3 items a day, 25% reserve),
/// against the real 4663 PoolManager runtime (etched by PoolBase).
abstract contract AuditABase is PoolBase {
    FloorModule module;
    address mallory = makeAddr("mallory");

    function _p1Template() internal pure returns (FrogCurve.Config memory c) {
        c.supply = 1_000_000_000e18; c.reserved = 200_000_000e18;
        c.platformFeeCap = 0.25 ether; c.platformBps = 500; c.poolEthBps = 1_260; c.feeBps = 100;
        c.vaultShareBps = 4_000; c.creatorShareBps = 3_000; c.reserveShareBps = 500; c.liquidityShareBps = 500;
        c.window = 3 days; c.launchFeeBps = 5_000; c.launchFeeDecay = 10 minutes;
    }

    function _p1Targets() internal pure returns (CurveFactory.Targets memory t) {
        t.normalRaise = 5 ether; t.earlyRaise = 8 ether; t.earlyUntilEth = 2 ether; t.maxRaise = 30 ether;
        t.minFeeBps = 100; t.maxFeeBps = 800;
    }

    function _p1Params() internal pure returns (FloorModule.Params memory p) {
        p.baseSpreadBps = 15; p.volWeightBps = 10_000; p.staleSpreadBps = 100; p.stressSpreadBps = 300; p.maxSpreadBps = 1_000;
        p.maxMoveBps = 500; p.convBps = 300; p.bufferBps = 500; p.mintPerHour = 5e18; p.burnPerHour = 5e18;
        p.maxPublicEthBacked = 10e18; p.maxEthBacked = 40e18; p.revenueTarget = 1 ether; p.rentTerm = 30 days;
        p.premiumBps = 1_000; p.rentShareBps = 5_000; p.maxRentals = 20; p.lowConfidenceSpreadBps = 100;
        p.fullConfidenceBps = 8_000; p.minConfidenceBps = 3_000; p.maxItemsPerDay = 3; p.reserveBps = 2_500; p.fillBandBps = 300;
    }

    function setUp() public virtual override {
        super.setUp();
        module = new FloorModule(IFloorWrapped(address(wrapper)), IKeeperSource(address(registry)), payee, platform, address(0), _p1Params(), PBS);
        wrapper.wireFloorModule(address(module));
        LiquidityLock lock2 = new LiquidityLock(PM, wrapper, _layout(), PBS);
        CurveFactory f2 = new CurveFactory(wrapper, lock2, IItemQueue(address(module)), platform, _p1Template(), _p1Targets(), PBS);
        EtchRouter r2 = new EtchRouter(PM, lock2, module);
        SweepVault v2r = new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(r2), f2, desk, PBS);
        SweepVault v2b = new SweepVault(registry, BURN_LISTING, wrapper, SweepVault.Policy.BURN, address(r2), f2, desk, PBS);
        _ceiling(v2r);                                        // C-01: no cap without the owner's ceiling
        _ceiling(v2b);
        f2.wire(v2r, v2b);
        module.wire(f2);
        lock2.wire(f2);
        lock = lock2;
        factory = f2;
        etchRouter = r2;
        curveRaffle = v2r;
        curveBurn = v2b;
        _postCap(curveRaffle, CAP);
        _floor(CAP);
    }

    function _floor(uint256 p) internal {
        vm.prank(keeper);
        module.postFloor(p, 10_000, 50);
    }

    function _skipBoth(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    /// Everything the curve owes out of its ETH balance.
    function _owed(FrogCurve c) internal view returns (uint256) {
        return c.reserveEth() + c.feesForVault() + c.creatorOwed() + c.platformOwed() + c.reserveOwed() + c.liquidityOwed();
    }
}
