// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {PoolKey} from "@uniswap/v4-core/src/types/PoolKey.sol";
import {PoolBase} from "../utils/PoolBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {IKeeperSource} from "../../src/interfaces/IKeeperSource.sol";
import {IFloorWrapped} from "../../src/interfaces/IFloorWrapped.sol";
import {IItemQueue} from "../../src/interfaces/IItemQueue.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveFactory} from "../../src/curve/CurveFactory.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {LiquidityLock} from "../../src/pools/LiquidityLock.sol";
import {EtchRouter} from "../../src/pools/EtchRouter.sol";

/// docs/v5/floor-module.md §8 and §11, against the REAL Uniswap v4 PoolManager (its 4663 runtime code, etched; no protocol fee in
/// the etched copy). The collection's stack is wired to the floor module: graduations convert through it, the router takes the
/// pool or the module per trade, the sweep vaults' venue is the router.
contract FloorRouterTest is PoolBase {
    FloorModule module;
    address platform2 = makeAddr("platform2");
    address trader = makeAddr("trader");
    address mallory = makeAddr("mallory");

    function _params() internal pure returns (FloorModule.Params memory p) {
        p.baseSpreadBps = 15;
        p.volWeightBps = 10_000;
        p.staleSpreadBps = 100;
        p.stressSpreadBps = 300;
        p.maxSpreadBps = 1_000;
        p.maxMoveBps = 500;
        p.convBps = 300;
        p.bufferBps = 500;
        p.mintPerHour = 5e18;
        p.burnPerHour = 5e18;
        p.maxPublicEthBacked = 10e18;
        p.maxEthBacked = 40e18;
        p.revenueTarget = 1 ether;
        p.rentTerm = 30 days;
        p.premiumBps = 1_000;
        p.rentShareBps = 5_000;
        p.maxRentals = 20;
        p.lowConfidenceSpreadBps = 100;
        p.fullConfidenceBps = 8_000;
        p.minConfidenceBps = 3_000;
        p.maxItemsPerDay = 40;
        p.reserveBps = 2_500;
        p.fillBandBps = 300;
    }

    function setUp() public override {
        super.setUp();
        module = new FloorModule(IFloorWrapped(address(wrapper)), IKeeperSource(address(registry)), payee, platform2, address(0), _params(), PBS);
        wrapper.wireFloorModule(address(module));
        LiquidityLock lock2 = new LiquidityLock(PM, wrapper, _layout(), PBS);
        CurveFactory f2 = new CurveFactory(wrapper, lock2, IItemQueue(address(module)), platform, _template(), _targets(), PBS);
        EtchRouter r2 = new EtchRouter(PM, lock2, module);
        SweepVault v2r = new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(r2), f2, desk, PBS);
        SweepVault v2b = new SweepVault(registry, BURN_LISTING, wrapper, SweepVault.Policy.BURN, address(r2), f2, desk, PBS);
        _ceiling(v2r);
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
        _floor(CAP);                                           // the module's floor where the pool's center is
    }

    function _floor(uint256 p) internal {
        vm.prank(keeper);
        module.postFloor(p, 10_000, 50);
    }

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    /// Two graduations through the module, then the canister converts their ETH-backed units to frogs.
    function _twoCoins() internal returns (CurveToken tok) {
        for (uint256 i = 0; i < 2; ++i) {
            (FrogCurve c, CurveToken t) = _launch(SweepVault.Policy.RAFFLE, false);
            _fillCurve(c);
            c.graduate();
            tok = t;
        }
        while (module.netUnits() >= 1e18) {
            _deposit(address(module));
            uint256 cost = module.beta();
            vm.prank(custodian);
            module.itemArrived(cost);
        }
        _skip(1 hours);
        _postCap(curveRaffle, CAP);
        _floor(CAP);
    }

    function _buyW(address who, uint256 eth) internal returns (uint256 out) {
        vm.deal(who, who.balance + eth);
        vm.prank(who);
        out = etchRouter.buyWrapped{value: eth}(0, who, block.timestamp);
    }

    function _sellW(address who, uint256 units) internal returns (uint256 out) {
        vm.startPrank(who);
        wrapper.approve(address(etchRouter), units);
        out = etchRouter.sellWrapped(units, 0, who, block.timestamp);
        vm.stopPrank();
    }

    // ---- best execution ----

    function test_buyingTakesTheModuleWhenItIsCheaper() public {
        _twoCoins();
        PoolKey memory key = lock.sharedKey();
        uint160 before = _sqrtP(key);
        uint256 quote = module.quoteMint(0.5 ether);
        uint256 out = _buyW(trader, 0.5 ether);
        assertEq(out, quote, "all of it at the module: 15 bps against the pool's 30 bps and its impact");
        assertEq(_sqrtP(key), before, "the pool was not touched");
    }

    function test_buyingFillsACheapPoolFirst_thenTheModule_andBeatsEitherAlone() public {
        _twoCoins();
        // somebody dumps wX into the pool: it is now cheaper than the module's mint price
        _wrappedFor(address(swapper), 3);
        swapper.swap(lock.sharedKey(), false, 3e18, 0);
        uint256 amount = 1 ether;
        uint256 snap = vm.snapshotState();
        uint256 viaRouter = _buyW(trader, amount);
        vm.revertToState(snap);
        uint256 moduleOnly = module.quoteMint(amount);
        _skip(module.FLOOR_TTL() + 1 minutes);                 // the module stale: the router has only the pool
        uint256 poolOnly = _buyW(trader, amount);
        vm.revertToState(snap);
        assertGe(viaRouter, moduleOnly, "at least what the module alone gives");
        assertGe(viaRouter, poolOnly, "at least what the pool alone gives");
        assertGt(viaRouter, moduleOnly, "strictly better: the pool's cheap part first");
        emit log_named_decimal_uint("wX for 1 ETH, router", viaRouter, 18);
        emit log_named_decimal_uint("wX for 1 ETH, module only", moduleOnly, 18);
        emit log_named_decimal_uint("wX for 1 ETH, pool only", poolOnly, 18);
    }

    function test_sellingTakesTheBetterVenue_andSplits() public {
        _twoCoins();
        _wrappedFor(trader, 4);                                // item-backed wX, deposited
        _buyW(mallory, 1 ether);                               // some ETH-backed units for the module to buy back
        uint256 snap = vm.snapshotState();
        uint256 viaRouter = _sellW(trader, 2e18);
        vm.revertToState(snap);
        uint256 moduleOnly = module.quoteBurn(2e18);
        _skip(module.FLOOR_TTL() + 1 minutes);
        uint256 poolOnly = _sellW(trader, 2e18);
        vm.revertToState(snap);
        assertGe(viaRouter, moduleOnly);
        assertGe(viaRouter, poolOnly);
        emit log_named_decimal_uint("ETH for 2 wX, router", viaRouter, 18);
        emit log_named_decimal_uint("ETH for 2 wX, module only", moduleOnly, 18);
        emit log_named_decimal_uint("ETH for 2 wX, pool only", poolOnly, 18);
    }

    function test_aStaleModuleLeavesThePool_andAFullOneOverflowsIntoIt() public {
        _twoCoins();
        uint256 residual = module.netUnits();                  // what the graduations left below one item: not the trade's
        _skip(module.FLOOR_TTL() + 1 minutes);
        uint256 out = _buyW(trader, 0.2 ether);
        assertGt(out, 0, "the pool alone");
        assertEq(module.netUnits(), residual, "nothing at the module");
        _postCap(curveRaffle, CAP);
        _floor(CAP);
        (uint256 price, uint256 room) = module.mintCapacity();
        uint256 big = room + 0.5 ether;
        uint256 e0 = module.ethBackedUnits();
        _buyW(trader, big);
        assertApproxEqAbs(module.ethBackedUnits() - e0, room * 1e18 / price, 5e16, "the module to its room, the rest to the pool");
    }

    function test_coinTradesUseTheModuleForTheirWrappedLeg() public {
        CurveToken tok = _twoCoins();
        uint256 e0 = module.netUnits();
        vm.deal(trader, 0.3 ether);
        vm.prank(trader);
        uint256 coins = etchRouter.buy{value: 0.3 ether}(address(tok), 0, trader, block.timestamp);
        assertGt(coins, 0);
        assertGt(module.netUnits(), e0, "the ETH to wX leg was minted at the module");
        vm.startPrank(trader);
        IERC20(address(tok)).approve(address(etchRouter), coins);
        uint256 back = etchRouter.sell(address(tok), coins, 0, trader, block.timestamp);
        vm.stopPrank();
        assertGt(back, 0);
    }

    // ---- sandwiches ----

    /// Mallory buys wX ahead of a victim, hoping to sell into the victim's buy: through the router (a mint at the module) and
    /// straight into the pool (pushing wX dearer there, with her `swapper`). The router sends the victim to the module at the
    /// module's own price, which no trade moves: the only thing her front-run changes is the stress term (§6.1), which her own
    /// mint raised for everyone, herself included. Her back-run sells into a pool nobody else touched: both legs together lose.
    function test_aSandwichAroundTheRouterGetsNothing() public {
        _twoCoins();
        uint256 fair = module.quoteMint(0.5 ether);
        uint256 e0 = module.netUnits();
        vm.deal(mallory, 5 ether);
        vm.deal(address(swapper), 10 ether);
        uint256 m0 = mallory.balance + address(swapper).balance;
        vm.prank(mallory);
        uint256 got = etchRouter.buyWrapped{value: 2 ether}(0, mallory, block.timestamp);   // front-run, via the router
        (, uint256 gotPool) = swapper.swap(lock.sharedKey(), true, 2 ether, 0);              // front-run, in the pool: wX dearer
        uint256 quoteNow = module.quoteMint(0.5 ether);
        uint256 victim = _buyW(trader, 0.5 ether);
        assertEq(victim, quoteNow, "the victim's price is the module's, whatever the pool did");
        // the stress her mint added: stressSpreadBps x (units she minted) / publicCap, and nothing else
        uint256 stressAdded = (300 * (module.netUnits() - e0) + 10e18 - 1) / 10e18 + 1;
        assertGe(victim * 10_000, fair * (10_000 - stressAdded), "worse than before her only by the stress term her own mint raised");
        // her back-run: sell everything she bought, both ways
        vm.startPrank(mallory);
        wrapper.approve(address(etchRouter), got);
        etchRouter.sellWrapped(got, 0, mallory, block.timestamp);
        vm.stopPrank();
        swapper.swap(lock.sharedKey(), false, gotPool, 0);
        assertEq(wrapper.balanceOf(mallory) + wrapper.balanceOf(address(swapper)), 0, "she holds nothing");
        assertLt(mallory.balance + address(swapper).balance, m0, "the sandwich lost money");
    }

    // ---- the sweep vaults still route ----

    function test_aSweepVaultBuysThroughTheRouterAndTheModule() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _fillCurve(c);
        c.graduate();
        c.flushVaultFees();
        uint256 credit = curveRaffle.credit(address(tok));
        assertGt(credit, 0);
        _postCap(curveRaffle, CAP * 11 / 10);
        uint256 spend = credit;
        bytes memory data = abi.encodeCall(EtchRouter.buyWrapped, (1, address(curveRaffle), block.timestamp));
        (, uint256 bought) = curveRaffle.sweep(address(tok), spend, data, 1);
        assertGt(bought, 0);
        assertEq(curveRaffle.units(address(tok)), bought, "credited to the coin that paid");
    }

    // ---- trader economics: measured round trips (docs/v5/floor-module.md §11) ----

    function _roundTrip(CurveToken tok, uint256 eth) internal returns (uint256 costBps) {
        uint256 snap = vm.snapshotState();
        vm.deal(trader, eth);
        vm.prank(trader);
        uint256 coins = etchRouter.buy{value: eth}(address(tok), 0, trader, block.timestamp);
        vm.startPrank(trader);
        IERC20(address(tok)).approve(address(etchRouter), coins);
        uint256 back = etchRouter.sell(address(tok), coins, 0, trader, block.timestamp);
        vm.stopPrank();
        costBps = (eth - back) * 10_000 / eth;
        vm.revertToState(snap);
    }

    function test_roundTripCosts_calmAndStressed() public {
        CurveToken tok = _twoCoins();                          // the graduations' reserve units are the module's buy-back room
        assertGe(module.reserveUnits(), 1e18, "reserve units to burn into");
        (uint256 sMint,) = module.spreads();
        assertLe(sMint, 25, "calm: the base spread plus the stress of less than a unit awaiting conversion");
        uint256 calmSmall = _roundTrip(tok, 0.1 ether);
        uint256 calmBig = _roundTrip(tok, 1 ether);
        _skip(1 hours);
        _postCap(curveRaffle, CAP);
        _floor(CAP * 2);                                       // a 5% move this hour: the module is 5.15% a side, the pool wins
        uint256 stressSmall = _roundTrip(tok, 0.1 ether);
        uint256 stressBig = _roundTrip(tok, 1 ether);
        emit log_named_uint("round trip 0.1 ETH, calm (bps)", calmSmall);
        emit log_named_uint("round trip 1 ETH, calm (bps)", calmBig);
        emit log_named_uint("round trip 0.1 ETH, stressed (bps)", stressSmall);
        emit log_named_uint("round trip 1 ETH, stressed (bps)", stressBig);
        // calm: two 1% coin-pool legs and two 15 bps module legs = 2.29% before impact (no protocol fee in the etched copy)
        assertLe(calmSmall, 250, "calm, small: within 0.2 pp of the fee legs");
        assertLe(stressSmall, 320, "stressed: the router takes the pool, not the 5% module");
    }
}
