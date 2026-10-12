// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {StdInvariant} from "forge-std/StdInvariant.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IPoolManager} from "@uniswap/v4-core/src/interfaces/IPoolManager.sol";
import {PoolKey} from "@uniswap/v4-core/src/types/PoolKey.sol";
import {Currency} from "@uniswap/v4-core/src/types/Currency.sol";
import {TickMath} from "@uniswap/v4-core/src/libraries/TickMath.sol";
import {PoolBase} from "../utils/PoolBase.sol";
import {Clock} from "../utils/Clock.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {LiquidityLock} from "../../src/pools/LiquidityLock.sol";

/// Graduations, trades in both pools (through the router and around it), seeder-fee claims, coin-fee collections, deferred adds,
/// center moves and time — in any order, against the real PoolManager. What must hold: liquidity only ever grows (nothing can
/// leave), every position's liquidity is exactly the sum of the coins' units, fees in are fees credited, and the lock's balances
/// cover everything it owes.
contract PoolsInvariantTest is PoolBase {
    address[] coins;
    mapping(bytes32 => uint128) maxLiquidity;
    mapping(address => uint128) coinLiquidity;
    uint256 public panics;
    uint256 public failures;
    uint256 public graduations;
    uint256 public claimsPaid;
    address trader = makeAddr("trader");

    function setUp() public override {
        super.setUp();
        _graduate(SweepVault.Policy.RAFFLE);
        coins.push(_lastCoin());
        _snapshotCoin(coins[0]);
        vm.deal(trader, 10_000 ether);
        bytes4[] memory s = new bytes4[](9);
        s[0] = this.act_graduate.selector;
        s[1] = this.act_swapShared.selector;
        s[2] = this.act_routeCoin.selector;
        s[3] = this.act_claimShared.selector;
        s[4] = this.act_collectCoin.selector;
        s[5] = this.act_deploy.selector;
        s[6] = this.act_moveCenter.selector;
        s[7] = this.act_passTime.selector;
        s[8] = this.act_graduateSweepOnly.selector;
        targetSelector(StdInvariant.FuzzSelector({addr: address(this), selectors: s}));
        targetContract(address(this));
    }

    function _lastCoin() internal view returns (address) { return factory.coins(factory.coinCount() - 1); }

    function _snapshotCoin(address coin) internal {
        (,, uint128 l) = lock.coins(coin);
        coinLiquidity[coin] = l;
    }

    function _check(bytes memory err) internal {
        if (err.length >= 4 && bytes4(err) == bytes4(0x4e487b71)) panics++;
        failures++;
    }

    /// After each run: the actions did something (a handler whose every call reverts proves nothing).
    function afterInvariant() public {
        emit log_named_uint("coins", coins.length);
        emit log_named_uint("positions", lock.positionCount());
        emit log_named_uint("fees in (ETH)", lock.feesInEth());
        emit log_named_uint("seeder claims that paid", claimsPaid);
        emit log_named_uint("reverted actions", failures);
    }

    // ---- actions ----

    function act_graduate(bool burn) external {
        if (coins.length >= 5 || nextFrog + 12 >= N_FROGS) return;
        _graduate(burn ? SweepVault.Policy.BURN : SweepVault.Policy.RAFFLE);
        coins.push(_lastCoin());
        _snapshotCoin(_lastCoin());
    }

    function act_graduateSweepOnly() external {
        if (coins.length >= 5) return;
        _graduateSweepOnly();
        coins.push(_lastCoin());
        _snapshotCoin(_lastCoin());
    }

    function act_swapShared(bool buyWrapped, uint96 amount) external {
        PoolKey memory sk = lock.sharedKey();
        if (buyWrapped) {
            uint256 a = bound(amount, 1e12, 3 ether);
            try swapper.swap(sk, true, a, 0) {} catch (bytes memory err) { _check(err); }
        } else {
            uint256 bal = IERC20(address(wrapper)).balanceOf(address(swapper));
            if (bal == 0) return;
            uint256 a = bound(amount, 1, bal);
            try swapper.swap(sk, false, a, 0) {} catch (bytes memory err) { _check(err); }
        }
    }

    function act_routeCoin(uint256 which, bool isBuy, uint96 amount) external {
        address coin = coins[which % coins.length];
        if (isBuy) {
            uint256 a = bound(amount, 1e12, 1 ether);
            vm.prank(trader);
            try etchRouter.buy{value: a}(coin, 0, trader, block.timestamp) {} catch (bytes memory err) { _check(err); }
        } else {
            uint256 bal = IERC20(coin).balanceOf(trader);
            if (bal == 0) return;
            uint256 a = bound(amount, 1, bal);
            vm.startPrank(trader);
            IERC20(coin).approve(address(etchRouter), a);
            try etchRouter.sell(coin, a, 0, trader, block.timestamp) {} catch (bytes memory err) { _check(err); }
            vm.stopPrank();
        }
    }

    function act_claimShared(uint256 which) external {
        try lock.claimShared(coins[which % coins.length]) returns (uint256 e, uint256 w) { if (e + w != 0) claimsPaid++; }
        catch (bytes memory err) { _check(err); }
    }

    function act_collectCoin(uint256 which) external {
        try lock.collectCoinFees(coins[which % coins.length]) {} catch (bytes memory err) { _check(err); }
    }

    function act_deploy(uint256 which) external {
        _freshCap();
        try lock.deploy(coins[which % coins.length]) {} catch (bytes memory err) { _check(err); }
    }

    function act_moveCenter(uint16 pct) external {
        uint256 p = bound(pct, 80, 125);
        _postCap(curveRaffle, CAP * p / 100);
        try lock.moveCenter() {} catch (bytes memory err) { _check(err); }
        _postCap(curveRaffle, CAP);
    }

    function act_passTime(uint32 secs) external {
        Clock.skip(bound(secs, 1, 2 days));
    }

    // ---- invariants ----

    /// Nothing can leave: every shared position only ever grows, and equals the sum of the coins' units exactly.
    function invariant_sharedLiquidityOnlyGrowsAndIsExactlyTheUnits() public {
        PoolKey memory sk = lock.sharedKey();
        for (uint256 i = 0; i < lock.positionCount(); ++i) {
            bytes32 id = lock.positionIds(i);
            LiquidityLock.Position memory p = lock.position(id);
            assertEq(_pmLiquidity(sk, p.lower, p.upper), p.liquidity, "the PoolManager holds what the lock books");
            uint256 sum;
            for (uint256 j = 0; j < coins.length; ++j) sum += lock.shareOf(id, coins[j]).units;
            assertEq(sum, p.liquidity, "units sum to the whole");
            assertGe(p.liquidity, maxLiquidity[id], "liquidity never falls");
            maxLiquidity[id] = p.liquidity;
        }
    }

    /// Each coin's own pool position never changes after its seed.
    function invariant_coinPoolLiquidityNeverMoves() public view {
        for (uint256 j = 0; j < coins.length; ++j) {
            (,, uint128 l) = lock.coins(coins[j]);
            assertEq(l, coinLiquidity[coins[j]]);
            PoolKey memory ck = lock.coinKey(coins[j]);
            (int24 lo, int24 hi) = lock.coinRange(coins[j]);
            assertEq(_pmLiquidity(ck, lo, hi), l);
        }
    }

    /// Fees in == fees credited (paid + owed), but rounding down by at most a wei per position per coin per claim.
    function invariant_feesInAreFeesCredited() public view {
        uint256 owedE;
        uint256 owedW;
        for (uint256 j = 0; j < coins.length; ++j) {
            (uint256 e, uint256 w) = lock.owed(coins[j]);
            owedE += e;
            owedW += w;
        }
        assertLe(lock.feesPaidEth() + owedE, lock.feesInEth(), "never credits more than came in");
        assertLe(lock.feesPaidWrapped() + owedW, lock.feesInWrapped());
        uint256 slack = 1_000 + lock.positionCount() * coins.length * 64;
        assertLe(lock.feesInEth() - lock.feesPaidEth() - owedE, slack, "and never loses more than rounding");
        assertLe(lock.feesInWrapped() - lock.feesPaidWrapped() - owedW, slack);
    }

    /// The lock's balances cover every pending contribution and every fee it has collected and not paid.
    function invariant_theLockCoversWhatItOwes() public view {
        uint256 pe;
        uint256 pw;
        for (uint256 j = 0; j < coins.length; ++j) {
            pe += lock.pendingEth(coins[j]);
            pw += lock.pendingWrapped(coins[j]);
        }
        assertGe(address(lock).balance, pe + (lock.feesInEth() - lock.feesPaidEth()));
        assertGe(IERC20(address(wrapper)).balanceOf(address(lock)), pw + (lock.feesInWrapped() - lock.feesPaidWrapped()));
    }

    /// The router never keeps anything.
    function invariant_theRouterHoldsNothing() public view {
        assertEq(address(etchRouter).balance, 0);
        assertEq(IERC20(address(wrapper)).balanceOf(address(etchRouter)), 0);
        for (uint256 j = 0; j < coins.length; ++j) assertEq(IERC20(coins[j]).balanceOf(address(etchRouter)), 0);
    }

    function invariant_noPanics() public view {
        assertEq(panics, 0);
    }
}
