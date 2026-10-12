// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {IPoolManager} from "@uniswap/v4-core/src/interfaces/IPoolManager.sol";
import {PoolKey} from "@uniswap/v4-core/src/types/PoolKey.sol";
import {Currency} from "@uniswap/v4-core/src/types/Currency.sol";
import {StateLibrary} from "@uniswap/v4-core/src/libraries/StateLibrary.sol";
import {TickMath} from "@uniswap/v4-core/src/libraries/TickMath.sol";
import {PoolBase, Swapper} from "../utils/PoolBase.sol";
import {Clock} from "../utils/Clock.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {LiquidityLock} from "../../src/pools/LiquidityLock.sol";
import {EtchRouter} from "../../src/pools/EtchRouter.sol";
import {ICapSource} from "../../src/interfaces/ICapSource.sol";

/// docs/v3/pools.md against the real v4 PoolManager (etched): the coin's pool, the layered shared pool, the add guard, the
/// center, seeder shares, coin-pool fees, the router, and that nothing can leave.
contract LiquidityLockTest is PoolBase {
    using StateLibrary for IPoolManager;

    // ---- the coin's own pool ----

    function test_graduationOpensTheCoinPoolAtTheCurvesLastPrice() public {
        (FrogCurve c, CurveToken tok) = _graduate(SweepVault.Policy.RAFFLE);
        PoolKey memory key = lock.coinKey(address(tok));
        assertEq(key.fee, 10_000, "the curve's 1% as the pool's LP fee");
        assertEq(address(key.hooks), address(0), "hookless");
        uint256 s = _sqrtP(key);
        bool coinIs0 = Currency.unwrap(key.currency0) == address(tok);
        // wFROG per whole coin, from the pool
        uint256 wPerCoin = coinIs0 ? Math.mulDiv(s * s, 1e18, 2 ** 192) : Math.mulDiv(2 ** 192, 1e18, s * s);
        uint256 ethPerCoin = Math.mulDiv(wPerCoin, c.unitsCost(), c.unitsBought());
        assertApproxEqRel(ethPerCoin, c.poolOpenPrice(), 1e12, "the pool opens where the curve ended, valued at cost");
        assertApproxEqRel(c.price(), c.poolOpenPrice(), 1e9);
        (,, uint128 liq) = lock.coins(address(tok));
        (int24 lo, int24 hi) = lock.coinRange(address(tok));
        assertEq(hi - lo, 2 * lock.FAR_HALF(), "one position of +-FAR_HALF around the seed price (audit A-01), not full range");
        assertTrue(lo > TickMath.minUsableTick(200) && hi < TickMath.maxUsableTick(200), "never on the saturable extremes");
        assertEq(_pmLiquidity(key, lo, hi), liq, "one wide position, the lock's own");
        assertGt(liq, 0);
        // audit A-01: the position is ±FAR_HALF on the 200-tick grid, so the seed price sits up to a spacing off its middle and
        // the two sides pair to within ~1e-6 (the full range paired to ~1e-12): at most ~2e-6 of the reserve is left unpaired
        assertLe(tok.balanceOf(address(lock)), c.reserved() / 1e5, "at most dust of the coin is left over (in effect burned)");
    }

    function test_somebodyWhoOpensTheCoinPoolFirstMovesNothing() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _fillCurve(c);
        while (!queue.isFilled(address(tok))) _item(FROG);
        PoolKey memory key = lock.coinKey(address(tok));
        PM.initialize(key, TickMath.getSqrtPriceAtTick(0));      // a price of their choosing, and no liquidity
        _freshCap();
        c.graduate();
        uint256 s = _sqrtP(key);
        bool coinIs0 = Currency.unwrap(key.currency0) == address(tok);
        uint256 wPerCoin = coinIs0 ? Math.mulDiv(s * s, 1e18, 2 ** 192) : Math.mulDiv(2 ** 192, 1e18, s * s);
        assertApproxEqRel(Math.mulDiv(wPerCoin, c.unitsCost(), c.unitsBought()), c.poolOpenPrice(), 1e12,
            "aligned to the seed price for free: the swap crossed empty ticks");
    }

    function test_somebodyWhoOpensTheCoinPoolFirstWithLiquiditySellsToTheLockBelowItsValuation() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _fillCurve(c);
        while (!queue.isFilled(address(tok))) _item(FROG);
        PoolKey memory key = lock.coinKey(address(tok));
        bool coinIs0 = Currency.unwrap(key.currency0) == address(tok);
        // the attacker opens it with the coin 100x too cheap in wFROG, and puts a little of both in, full range
        int24 wrongTick = coinIs0 ? int24(-230_000) : int24(230_000);
        PM.initialize(key, TickMath.getSqrtPriceAtTick(wrongTick));
        _wrappedFor(address(swapper), 1);
        vm.prank(buyer);
        tok.transfer(address(swapper), 1_000_000e18);
        swapper.addLiquidity(key, TickMath.minUsableTick(200), TickMath.maxUsableTick(200), 1e17);
        uint256 lockCoinBefore = tok.balanceOf(address(lock));
        _freshCap();
        c.graduate();
        (,, uint128 liq) = lock.coins(address(tok));
        assertGt(liq, 0, "the lock still seeded the pool");
        assertEq(lockCoinBefore, 0);
        // the price moved toward (to) the seed price: the attacker's liquidity was bought through at the attacker's prices
        int24 t = _tick(key);
        int24 seedTick = TickMath.getTickAtSqrtPrice(uint160(Math.sqrt(coinIs0
            ? Math.mulDiv(c.unitsBought() - c.unitsToShared(), 2 ** 192, c.reserved())
            : Math.mulDiv(c.reserved(), 2 ** 192, c.unitsBought() - c.unitsToShared()))));
        assertLt(_abs(int256(t) - int256(seedTick)), 200, "at the seed price");
    }

    // ---- the shared pool: layers, every graduation adds to the same positions ----

    function test_theFirstGraduationLaysOutTheLayersAroundTheCap() public {
        (, CurveToken tok) = _graduate(SweepVault.Policy.RAFFLE);
        assertTrue(lock.centered());
        int24 capTick = TickMath.getTickAtSqrtPrice(uint160(Math.sqrt(Math.mulDiv(1e18, 2 ** 192, CAP))));
        assertLe(_abs(int256(lock.center()) - int256(capTick)), 60, "centered on the keeper's cap");
        assertEq(lock.positionCount(), 4, "near, wide, full, bid");
        int24[2][4] memory r = lock.layers();
        PoolKey memory sk = lock.sharedKey();
        for (uint256 i = 0; i < 4; ++i) {
            bytes32 id = keccak256(abi.encode(r[i][0], r[i][1]));
            uint128 l = lock.position(id).liquidity;
            assertGt(l, 0);
            assertEq(_pmLiquidity(sk, r[i][0], r[i][1]), l, "the PoolManager holds exactly what the lock books");
            assertEq(lock.shareOf(id, address(tok)).units, l, "one coin: all the units");
        }
        assertLe(lock.pendingEth(address(tok)), 10, "everything placed, but rounding");
        assertLe(lock.pendingWrapped(address(tok)), 10);
        assertEq(uint8(0), uint8(0));
    }

    function test_everyGraduationAddsToTheSamePositions() public {
        (, CurveToken a) = _graduate(SweepVault.Policy.RAFFLE);
        int24[2][4] memory r = lock.layers();
        uint128[4] memory before;
        for (uint256 i = 0; i < 4; ++i) before[i] = lock.position(keccak256(abi.encode(r[i][0], r[i][1]))).liquidity;
        (, CurveToken b) = _graduate(SweepVault.Policy.BURN);
        assertEq(lock.positionCount(), 4, "no new position");
        for (uint256 i = 0; i < 4; ++i) {
            bytes32 id = keccak256(abi.encode(r[i][0], r[i][1]));
            uint128 l = lock.position(id).liquidity;
            assertGt(l, before[i], "deeper");
            assertEq(lock.shareOf(id, address(a)).units + lock.shareOf(id, address(b)).units, l, "shares sum to the whole");
            assertEq(lock.shareOf(id, address(a)).units, before[i], "the first coin's share is unchanged");
        }
    }

    function test_theSharedAddWaitsForAFreshCapAndAnyoneFinishesIt() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _fillCurve(c);
        while (!queue.isFilled(address(tok))) _item(FROG);
        Clock.skip(2 hours);                                     // the keeper's cap is stale
        c.graduate();                                            // graduation does not wait on the pool
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated));
        assertEq(lock.positionCount(), 0);
        assertEq(lock.pendingEth(address(tok)), c.poolEth() + (c.itemBudget() - c.unitsCost()));
        assertGe(lock.pendingWrapped(address(tok)), c.unitsToShared());
        assertLe(lock.pendingWrapped(address(tok)), c.unitsToShared() + 1e6, "plus the coin pool's unpaired wei");
        _freshCap();
        vm.prank(stranger);
        lock.deploy(address(tok));
        assertEq(lock.positionCount(), 4);
        assertLe(lock.pendingEth(address(tok)), 10);
    }

    function test_theSharedAddWaitsWhileThePriceIsPushedAway() public {
        _graduate(SweepVault.Policy.RAFFLE);
        PoolKey memory sk = lock.sharedKey();
        int24 ref = _tick(sk);
        // somebody buys wFROG hard: ETH in, the tick falls 1,500 (wFROG ~16% dearer), past the 1,000-tick guard
        swapper.swap(sk, true, 50 ether, TickMath.getSqrtPriceAtTick(ref - 1500));
        (FrogCurve c2, CurveToken t2) = _launch(SweepVault.Policy.BURN, false);
        _fillCurve(c2);
        while (!queue.isFilled(address(t2))) _item(FROG);
        _freshCap();
        c2.graduate();
        assertGt(lock.pendingEth(address(t2)), 0.5 ether, "not added at a pushed price");
        vm.expectEmit(true, false, false, true);
        emit LiquidityLock.SharedDeferred(address(t2), 2);
        lock.deploy(address(t2));
        // arbitrage brings it back; now anyone may finish the add
        uint256 w = IERC20(address(wrapper)).balanceOf(address(swapper));
        swapper.swap(sk, false, w, TickMath.getSqrtPriceAtTick(ref));
        lock.deploy(address(t2));
        assertLe(lock.pendingEth(address(t2)), 10);
    }

    function test_somebodyWhoOpensTheSharedPoolFirstMovesNothing() public {
        PoolKey memory sk = lock.sharedKey();
        PM.initialize(sk, TickMath.getSqrtPriceAtTick(50_000));  // wFROG at a silly price, no liquidity
        (, CurveToken tok) = _graduate(SweepVault.Policy.RAFFLE);
        int24 capTick = TickMath.getTickAtSqrtPrice(uint160(Math.sqrt(Math.mulDiv(1e18, 2 ** 192, CAP))));
        assertLe(_abs(int256(_tick(sk)) - int256(capTick)), 1, "moved to the cap's price by one wei across empty ticks");
        assertEq(lock.positionCount(), 4);
        assertLe(lock.pendingWrapped(address(tok)), 10);
    }

    // ---- the center ----

    function test_theCenterMovesTowardTheCapBoundedAndOldLiquidityStays() public {
        (, CurveToken a) = _graduate(SweepVault.Policy.RAFFLE);
        int24 c0 = lock.center();
        int24[2][4] memory r0 = lock.layers();
        uint128 near0 = lock.position(keccak256(abi.encode(r0[0][0], r0[0][1]))).liquidity;
        _postCap(curveRaffle, CAP * 13 / 10);                     // the floor rose 30%: tick down ≈ 2,624
        vm.expectRevert(LiquidityLock.TooSoon.selector);
        lock.moveCenter();
        Clock.skip(1 days);
        _postCap(curveRaffle, CAP * 13 / 10);
        vm.prank(stranger);
        lock.moveCenter();
        assertEq(lock.center(), c0 - 960, "one step of maxShift, toward the cap");
        vm.expectRevert(LiquidityLock.TooSoon.selector);
        lock.moveCenter();
        assertEq(lock.position(keccak256(abi.encode(r0[0][0], r0[0][1]))).liquidity, near0, "old positions never move");
        assertEq(lock.shareOf(keccak256(abi.encode(r0[0][0], r0[0][1])), address(a)).units, near0);
    }

    function test_theCenterDoesNotMoveForASmallChange() public {
        _graduate(SweepVault.Policy.RAFFLE);
        Clock.skip(1 days);
        _postCap(curveRaffle, CAP * 102 / 100);                   // 2%: ≈ 200 ticks, under the 480-tick minimum
        vm.expectRevert(LiquidityLock.NoMove.selector);
        lock.moveCenter();
    }

    function test_newLiquidityGoesAroundTheNewCenterWhileTheFullRangeIsShared() public {
        _graduate(SweepVault.Policy.RAFFLE);
        int24[2][4] memory r0 = lock.layers();
        Clock.skip(1 days);
        _postCap(curveRaffle, CAP * 11 / 10);                     // +10%: tick ≈ −953
        lock.moveCenter();
        PoolKey memory sk = lock.sharedKey();
        // arbitrage follows the floor: somebody buys wFROG until the pool is at the new cap
        int24 capTick = TickMath.getTickAtSqrtPrice(uint160(Math.sqrt(Math.mulDiv(1e18, 2 ** 192, CAP * 11 / 10))));
        swapper.swap(sk, true, 50 ether, TickMath.getSqrtPriceAtTick(capTick));
        (, CurveToken b) = _graduate(SweepVault.Policy.BURN);
        int24[2][4] memory r1 = lock.layers();
        assertTrue(r1[0][0] != r0[0][0], "a new near band");
        assertEq(r1[2][0], r0[2][0], "one full-range position for all time");
        assertGt(lock.shareOf(keccak256(abi.encode(r1[2][0], r1[2][1])), address(b)).units, 0);
        assertGt(lock.shareOf(keccak256(abi.encode(r1[0][0], r1[0][1])), address(b)).units, 0);
        assertEq(lock.positionCount(), 4 + 3, "new near, wide and bid; the full range is shared");
    }

    // ---- seeder shares ----

    function _churn(uint256 eth) internal {
        PoolKey memory sk = lock.sharedKey();
        (, uint256 w) = swapper.swap(sk, true, eth, 0);           // buy wFROG, then sell it all back: fees both ways
        swapper.swap(sk, false, w, 0);
    }

    function test_seederFeesArePaidToTheCoinsVault() public {
        (, CurveToken a) = _graduate(SweepVault.Policy.RAFFLE);
        _churn(0.5 ether);
        uint256 unitsBefore = curveRaffle.units(address(a));
        (uint256 eth, uint256 w) = lock.claimShared(address(a));
        assertGt(eth + w, 0);
        assertEq(curveRaffle.credit(address(a)), eth, "ETH fees credited to the coin");
        assertEq(curveRaffle.units(address(a)) - unitsBefore, w, "wFROG fees credited to the coin");
        assertEq(lock.feesPaidEth(), eth);
        assertLe(lock.feesInEth() - lock.feesPaidEth(), 10, "fees in == fees credited, but rounding");
        assertLe(lock.feesInWrapped() - lock.feesPaidWrapped(), 10);
    }

    function test_aLaterCoinCannotDiluteAnEarlierCoinsAccruedFees() public {
        (, CurveToken a) = _graduate(SweepVault.Policy.RAFFLE);
        _churn(0.5 ether);
        (uint256 aEth0, uint256 aW0) = _claimable(address(a));
        (, CurveToken b) = _graduate(SweepVault.Policy.BURN);     // its adds poke the positions first
        (uint256 bEth, uint256 bW) = lock.claimShared(address(b));
        assertEq(bEth + bW, 0, "the second coin gets nothing earned before it arrived");
        (uint256 aEth, uint256 aW) = lock.claimShared(address(a));
        assertGe(aEth + 2, aEth0);
        assertGe(aW + 2, aW0);
        assertGt(aEth + aW, 0);
    }

    function test_feesAfterBothArriveSplitByUnits() public {
        (, CurveToken a) = _graduate(SweepVault.Policy.RAFFLE);
        (, CurveToken b) = _graduate(SweepVault.Policy.BURN);
        lock.claimShared(address(a));
        lock.claimShared(address(b));
        _churn(0.2 ether);
        (uint256 aE, uint256 aW) = lock.claimShared(address(a));
        (uint256 bE, uint256 bW) = lock.claimShared(address(b));
        int24[2][4] memory r = lock.layers();
        bytes32 near = keccak256(abi.encode(r[0][0], r[0][1]));
        uint256 ua = lock.shareOf(near, address(a)).units;
        uint256 ub = lock.shareOf(near, address(b)).units;
        // a small churn stays inside the near band; there the split is exactly the units' split
        assertApproxEqRel((aE + aW * CAP / 1e18) * ub, (bE + bW * CAP / 1e18) * ua, 0.02e18);
        assertLe(lock.feesInEth() - lock.feesPaidEth(), 20);
    }

    function _claimable(address coin) internal returns (uint256 eth, uint256 w) {
        uint256 snap = vm.snapshotState();
        (eth, w) = lock.claimShared(coin);
        vm.revertToState(snap);
    }

    // ---- the coin's pool fees ----

    function test_coinPoolFeesAreSplitVaultCreatorPlatformAndTheCoinsVaultShareBurned() public {
        (FrogCurve c, CurveToken tok) = _graduate(SweepVault.Policy.RAFFLE);
        vm.deal(buyer, 2 ether);
        vm.prank(buyer);
        uint256 got = etchRouter.buy{value: 1 ether}(address(tok), 0, buyer, block.timestamp);
        vm.startPrank(buyer);
        tok.approve(address(etchRouter), got);
        etchRouter.sell(address(tok), got, 0, buyer, block.timestamp);
        vm.stopPrank();
        uint256 units0 = curveRaffle.units(address(tok));
        (uint256 tokenFees, uint256 quoteFees) = lock.collectCoinFees(address(tok));
        assertGt(tokenFees, 0);
        assertGt(quoteFees, 0);
        uint256 toVault = quoteFees * 5_000 / 10_000;
        uint256 toCreator = quoteFees * 3_000 / 10_000;
        assertEq(curveRaffle.units(address(tok)) - units0, toVault);
        assertEq(IERC20(address(wrapper)).balanceOf(creator), toCreator);
        assertEq(IERC20(address(wrapper)).balanceOf(platform), quoteFees - toVault - toCreator, "sums exactly");
        assertEq(tok.balanceOf(c.DEAD()), tokenFees * 5_000 / 10_000, "the vault's share of coin fees is burned");
        assertEq(tok.balanceOf(creator), tokenFees * 3_000 / 10_000);
    }

    // ---- the router, and routing without it ----

    function test_theRouterBuysAndSellsThroughWrappedInOneTransaction() public {
        (, CurveToken tok) = _graduate(SweepVault.Policy.RAFFLE);
        address buyer = makeAddr("a trader who never touched the curve");
        vm.deal(buyer, 1 ether);
        vm.prank(buyer);
        uint256 got = etchRouter.buy{value: 0.1 ether}(address(tok), 1, buyer, block.timestamp);
        assertEq(tok.balanceOf(buyer), got);
        assertEq(address(etchRouter).balance, 0);
        assertEq(IERC20(address(wrapper)).balanceOf(address(etchRouter)), 0, "the wFROG never left the PoolManager");
        uint256 ethBefore = buyer.balance;
        vm.startPrank(buyer);
        tok.approve(address(etchRouter), got);
        uint256 back = etchRouter.sell(address(tok), got, 1, buyer, block.timestamp);
        vm.stopPrank();
        assertEq(buyer.balance - ethBefore, back);
        assertLt(back, 0.1 ether, "fees both ways");
        assertGt(back, 0.095 ether, "0.3% + 1%, twice, and a little depth");
        assertEq(tok.balanceOf(address(etchRouter)), 0);
    }

    function test_theRouterRespectsMinOutAndDeadline() public {
        (, CurveToken tok) = _graduate(SweepVault.Policy.RAFFLE);
        vm.deal(buyer, 1 ether);
        vm.startPrank(buyer);
        vm.expectRevert(EtchRouter.Expired.selector);
        etchRouter.buy{value: 0.1 ether}(address(tok), 0, buyer, block.timestamp - 1);
        vm.expectRevert();
        etchRouter.buy{value: 0.1 ether}(address(tok), type(uint256).max, buyer, block.timestamp);
        vm.stopPrank();
        vm.expectRevert(EtchRouter.NotAPool.selector);
        etchRouter.buy{value: 1}(address(0xBEEF), 0, buyer, block.timestamp);
    }

    function test_anyoneCanRouteThePoolsWithoutOurRouter() public {
        (, CurveToken tok) = _graduate(SweepVault.Policy.RAFFLE);
        // an aggregator's two hops, done by hand: ETH → wFROG in the shared pool, wFROG → COIN in the coin's pool
        (, uint256 w) = swapper.swap(lock.sharedKey(), true, 0.1 ether, 0);
        PoolKey memory ck = lock.coinKey(address(tok));
        bool wIs0 = Currency.unwrap(ck.currency0) == address(wrapper);
        (, uint256 coins) = swapper.swap(ck, wIs0, w, 0);
        assertEq(tok.balanceOf(address(swapper)), coins);
        assertGt(coins, 0);
    }

    function test_aSweepOnlyCoinTradesAgainstEthInOneHop() public {
        (FrogCurve c, CurveToken tok) = _graduateSweepOnly();
        (, bool quoteIsEth, uint128 liq) = lock.coins(address(tok));
        assertTrue(quoteIsEth);
        assertGt(liq, 0);
        assertEq(lock.positionCount(), 0, "no shared pool for it");
        vm.deal(buyer, 1 ether);
        vm.prank(buyer);
        uint256 got = etchRouter.buy{value: 0.5 ether}(address(tok), 1, buyer, block.timestamp);
        vm.startPrank(buyer);
        tok.approve(address(etchRouter), got / 2);
        etchRouter.sell(address(tok), got / 2, 1, buyer, block.timestamp);
        vm.stopPrank();
        uint256 vaultBefore = c.feesForVault();
        (, uint256 ethFees) = lock.collectCoinFees(address(tok));
        assertGt(ethFees, 0);
        assertEq(c.feesForVault() - vaultBefore, ethFees * 5_000 / 10_000, "ETH pool fees split like trade fees");
    }

    // ---- nothing leaves, nobody else seeds ----

    function test_theLockHasNoWayOut() public view {
        string memory json = vm.readFile("out/LiquidityLock.sol/LiquidityLock.json");
        string[] memory sigs = vm.parseJsonKeys(json, ".methodIdentifiers");
        string[27] memory allowed = ["BPS()", "bidWidth()", "center()", "centerMovedAt()", "centerMovedAtBlock()", "centered()",
            "claimShared(address)", "coinKey(address)", "coinPositions(address)", "coinSpacing()", "coins(address)",
            "collectCoinFees(address)", "deploy(address)", "deployer()", "factory()", "feesInEth()", "feesInWrapped()",
            "feesPaidEth()", "feesPaidWrapped()", "layers()", "maxDeviation()", "maxShift()", "minMove()", "moveCenter()",
            "moveEvery()", "nearBps()", "nearHalf()"];
        string[37] memory allowed2 = ["owed(address)", "owedEth(address)", "owedWrapped(address)", "parentBlockSeconds()",
            "pendingEth(address)", "pendingWrapped(address)", "poolManager()", "position(bytes32)", "positionCount()",
            "positionIds(uint256)", "seed(address,uint256,uint256,uint256)", "seedEth(address,uint256)", "shareOf(bytes32,address)",
            "sharedEthIn()", "sharedFee()", "sharedKey()", "sharedSpacing()", "sharedWrappedIn()", "unlockCallback(bytes)",
            "wideBps()", "wideHalf()",
            // docs/v5/floor-module.md §13.2: protocol-owned liquidity, money in only; the treasury wired once for its ETH fees
            "PROTOCOL()", "addProtocolLiquidity()", "treasury()", "wireTreasury(address)",
            // audit 2026-09-26 A-01/A-04: bounded ranges, fail-soft seeding with anyone's retry (money in only: the self-calls
            // refuse any caller but the lock itself), the module reference for shared adds
            "FAR_HALF()", "REF_DEVIATION()", "farLower()", "farUpper()", "coinRange(address)", "floorRef()", "priceReference()",
            "pendingCoinToken(address)", "pendingCoinQuote(address)", "seedCoin(address)",
            "seedCoinPoolSelf(address,uint256,uint256)", "deploySelf(address)"];
        uint256 found;
        for (uint256 i = 0; i < sigs.length; ++i) {
            bool ok;
            for (uint256 j = 0; j < allowed.length && !ok; ++j) ok = keccak256(bytes(sigs[i])) == keccak256(bytes(allowed[j]));
            for (uint256 j = 0; j < allowed2.length && !ok; ++j) ok = keccak256(bytes(sigs[i])) == keccak256(bytes(allowed2[j]));
            if (keccak256(bytes(sigs[i])) == keccak256("wire(address)") || keccak256(bytes(sigs[i])) == keccak256("wrapped()")) ok = true;
            assertTrue(ok, string.concat("unexpected external function: ", sigs[i]));
            if (ok) found++;
        }
        assertEq(found, sigs.length);
        assertEq(sigs.length, allowed.length + allowed2.length + 2, "exactly the documented surface: no withdraw, no decrease");
    }

    function test_onlyTheCoinsOwnCurveSeeds() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        vm.prank(stranger);
        vm.expectRevert(LiquidityLock.NotCurve.selector);
        lock.seed(address(tok), 1, 1, 0);
        vm.prank(address(c));
        vm.expectRevert(LiquidityLock.NotCurve.selector);
        lock.seed(address(0xBAD), 1, 1, 0);
    }

    function test_wireOnceOnlyByTheDeployer() public {
        vm.prank(stranger);
        vm.expectRevert(LiquidityLock.NotDeployer.selector);
        lock.wire(ICapSource(address(factory)));
        vm.expectRevert(LiquidityLock.AlreadyWired.selector);
        lock.wire(ICapSource(address(factory)));
    }

    function test_nobodyButThePoolManagerSendsEthOrCallsBack() public {
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        (bool ok,) = address(lock).call{value: 1}("");
        assertFalse(ok);
        vm.prank(stranger);
        vm.expectRevert(LiquidityLock.NotPoolManager.selector);
        lock.unlockCallback(abi.encode(LiquidityLock.Op.Poke, abi.encode(new bytes32[](0))));
    }

    function test_theLayoutIsCheckedAtConstruction() public {
        LiquidityLock.Params memory p = _layout();
        p.nearHalf = 961;                                         // not on the tick spacing
        vm.expectRevert(LiquidityLock.BadParams.selector);
        new LiquidityLock(PM, wrapper, p, PBS);
        p = _layout();
        p.wideHalf = p.nearHalf;
        vm.expectRevert(LiquidityLock.BadParams.selector);
        new LiquidityLock(PM, wrapper, p, PBS);
        p = _layout();
        p.nearBps = 8_000;
        vm.expectRevert(LiquidityLock.BadParams.selector);
        new LiquidityLock(PM, wrapper, p, PBS);
    }

    // ---- fuzz ----

    /// However the keeper's cap wanders, each move of the center is at most `maxShift`, toward the cap, at most once a day; the
    /// bands keep their widths and stay inside the usable ticks.
    function testFuzz_theBandsMoveBoundedTowardTheCap(uint16[6] memory pcts) public {
        _graduate(SweepVault.Policy.RAFFLE);
        for (uint256 i = 0; i < 6; ++i) {
            Clock.skip(1 days);
            uint256 cap = CAP * bound(pcts[i], 25, 400) / 100;
            _postCap(curveRaffle, cap);
            int24 before = lock.center();
            int24 target = TickMath.getTickAtSqrtPrice(uint160(Math.sqrt(Math.mulDiv(1e18, 2 ** 192, cap))));
            try lock.moveCenter() {
                int256 step = int256(lock.center()) - int256(before);
                assertLe(_abs(step), 960, "at most maxShift per move");
                assertTrue(step != 0);
                assertTrue((step > 0) == (int256(target) > int256(before)), "toward the cap");
                assertLe(_abs(int256(target) - int256(lock.center())), _abs(int256(target) - int256(before)), "never past it");
            } catch (bytes memory err) {
                assertEq(bytes4(err), LiquidityLock.NoMove.selector, "the only refusal after a day is: already close enough");
                assertLt(_abs(int256(target) - int256(before)), 480 + 60);
            }
            int24[2][4] memory r = lock.layers();
            assertEq(r[0][1] - r[0][0], 2 * 960);
            assertEq(r[1][1] - r[1][0], 2 * 2640);
            assertEq(r[3][1] - r[3][0], 1920);
            assertGe(r[1][0], TickMath.minUsableTick(60));
            assertLe(r[3][1], TickMath.maxUsableTick(60));
        }
    }

    /// Any item cost the keeper allows, any cap: graduation opens the coin's pool at the curve's last price, the shared adds never
    /// take more than the coin brought, and the units are exactly the liquidity.
    function testFuzz_graduationIntoThePools(uint96 itemCost, uint16 capPct) public {
        uint256 cap = CAP * bound(capPct, 50, 300) / 100;
        _postCap(curveRaffle, cap);
        uint256 cost = bound(itemCost, cap / 3, cap);
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _fillCurve(c);
        while (!queue.isFilled(address(tok))) {
            _deposit(address(queue));
            if (!curveRaffle.capFresh()) _postCap(curveRaffle, cap);
            // a lone coin whose remainder is above its tail but below one item would wait for the next coin; here the last
            // item simply costs what is left (the property under test is the seeding, not the queue's patience)
            uint256 thisCost = Math.min(cost, queue.queued());
            vm.prank(custodian);
            queue.recordItem(thisCost);
        }
        if (!curveRaffle.capFresh()) _postCap(curveRaffle, cap);
        c.graduate();
        PoolKey memory key = lock.coinKey(address(tok));
        uint256 s = _sqrtP(key);
        bool coinIs0 = Currency.unwrap(key.currency0) == address(tok);
        uint256 wPerCoin = coinIs0 ? Math.mulDiv(s * s, 1e18, 2 ** 192) : Math.mulDiv(2 ** 192, 1e18, s * s);
        assertApproxEqRel(Math.mulDiv(wPerCoin, c.unitsCost(), c.unitsBought()), c.poolOpenPrice(), 1e12);
        uint256 used;
        for (uint256 i = 0; i < lock.positionCount(); ++i) {
            bytes32 id = lock.positionIds(i);
            assertEq(lock.shareOf(id, address(tok)).units, lock.position(id).liquidity);
            used++;
        }
        assertGe(used, 3, "near, wide and full at least");
        assertLe(lock.pendingWrapped(address(tok)), c.unitsToShared(), "never more than it brought");
        assertLe(lock.pendingEth(address(tok)), c.poolEth() + (c.itemBudget() - c.unitsCost()));
    }

    function _abs(int256 x) internal pure returns (uint256) { return uint256(x < 0 ? -x : x); }
}
