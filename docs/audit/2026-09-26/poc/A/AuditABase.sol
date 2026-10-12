// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {PoolBase, Swapper} from "../../utils/PoolBase.sol";
import {FloorModule} from "../../../src/floor/FloorModule.sol";
import {IFloorWrapped} from "../../../src/interfaces/IFloorWrapped.sol";
import {IItemQueue} from "../../../src/interfaces/IItemQueue.sol";
import {SweepVault} from "../../../src/SweepVault.sol";
import {FrogCurve} from "../../../src/curve/FrogCurve.sol";
import {CurveFactory} from "../../../src/curve/CurveFactory.sol";
import {CurveToken} from "../../../src/curve/CurveToken.sol";
import {LiquidityLock} from "../../../src/pools/LiquidityLock.sol";
import {EtchRouter} from "../../../src/pools/EtchRouter.sol";

/// Audit A harness: the Phase-1 collection stack as DeployStack.s.sol builds it — the factory's queue is the FLOOR MODULE (not
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
        module = new FloorModule(IFloorWrapped(address(wrapper)), keeper, payee, platform, address(0), _p1Params(), PBS);
        wrapper.wireFloorModule(address(module));
        LiquidityLock lock2 = new LiquidityLock(PM, wrapper, _layout(), PBS);
        CurveFactory f2 = new CurveFactory(wrapper, lock2, IItemQueue(address(module)), platform, _p1Template(), _p1Targets(), PBS);
        EtchRouter r2 = new EtchRouter(PM, lock2, module);
        SweepVault v2r = new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(r2), f2, desk, PBS);
        SweepVault v2b = new SweepVault(registry, BURN_LISTING, wrapper, SweepVault.Policy.BURN, address(r2), f2, desk, PBS);
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
