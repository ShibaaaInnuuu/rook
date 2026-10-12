// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IPoolManager} from "@uniswap/v4-core/src/interfaces/IPoolManager.sol";
import {IUnlockCallback} from "@uniswap/v4-core/src/interfaces/callback/IUnlockCallback.sol";
import {PoolKey} from "@uniswap/v4-core/src/types/PoolKey.sol";
import {Currency} from "@uniswap/v4-core/src/types/Currency.sol";
import {BalanceDelta} from "@uniswap/v4-core/src/types/BalanceDelta.sol";
import {SwapParams, ModifyLiquidityParams} from "@uniswap/v4-core/src/types/PoolOperation.sol";
import {StateLibrary} from "@uniswap/v4-core/src/libraries/StateLibrary.sol";
import {TickMath} from "@uniswap/v4-core/src/libraries/TickMath.sol";
import {SweepBase} from "./SweepBase.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveFactory} from "../../src/curve/CurveFactory.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {ItemQueue} from "../../src/curve/ItemQueue.sol";
import {LiquidityLock} from "../../src/pools/LiquidityLock.sol";
import {EtchRouter} from "../../src/pools/EtchRouter.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// Swaps any pool directly against the PoolManager, exact input, and can add its own liquidity: a trader, an arbitrageur, or
/// somebody front-running a pool's initialization. Test-only.
contract Swapper is IUnlockCallback {
    IPoolManager immutable pm;
    constructor(IPoolManager pm_) { pm = pm_; }
    receive() external payable {}

    function swap(PoolKey memory key, bool zeroForOne, uint256 amountIn, uint160 limit) external returns (uint256 paid, uint256 got) {
        return abi.decode(pm.unlock(abi.encode(uint8(0), abi.encode(key, zeroForOne, amountIn, limit))), (uint256, uint256));
    }

    function addLiquidity(PoolKey memory key, int24 lo, int24 hi, uint128 liq) external {
        pm.unlock(abi.encode(uint8(1), abi.encode(key, lo, hi, liq)));
    }

    function unlockCallback(bytes calldata data) external returns (bytes memory) {
        require(msg.sender == address(pm));
        (uint8 op, bytes memory args) = abi.decode(data, (uint8, bytes));
        BalanceDelta d;
        PoolKey memory key;
        if (op == 0) {
            bool zeroForOne; uint256 amountIn; uint160 limit;
            (key, zeroForOne, amountIn, limit) = abi.decode(args, (PoolKey, bool, uint256, uint160));
            if (limit == 0) limit = zeroForOne ? TickMath.MIN_SQRT_PRICE + 1 : TickMath.MAX_SQRT_PRICE - 1;
            d = pm.swap(key, SwapParams(zeroForOne, -int256(amountIn), limit), "");
        } else {
            int24 lo; int24 hi; uint128 liq;
            (key, lo, hi, liq) = abi.decode(args, (PoolKey, int24, int24, uint128));
            (d,) = pm.modifyLiquidity(key, ModifyLiquidityParams(lo, hi, int256(uint256(liq)), bytes32(0)), "");
        }
        _settle(key.currency0, d.amount0());
        _settle(key.currency1, d.amount1());
        (int128 a0, int128 a1) = (d.amount0(), d.amount1());
        uint256 paid = uint256(int256(a0 < 0 ? -a0 : int128(0))) + uint256(int256(a1 < 0 ? -a1 : int128(0)));
        uint256 got = uint256(int256(a0 > 0 ? a0 : int128(0))) + uint256(int256(a1 > 0 ? a1 : int128(0)));
        return abi.encode(paid, got);
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

/// `SweepBase`, plus the whole graduation stack for the frog collection against the REAL Uniswap v4 PoolManager: its runtime
/// code as deployed on 4663 (test/fixtures/v4-pool-manager-4663.hex, docs/v3/pools.md §1) etched at its own address, so no
/// network is needed. A LiquidityLock with the documented layout, the collection's queue, factory and vaults, a router.
/// In the etched copy the protocol fee controller is unset, so there is no protocol fee here (the fork test has the real one).
abstract contract PoolBase is SweepBase {
    using StateLibrary for IPoolManager;

    IPoolManager constant PM = IPoolManager(0x8366a39CC670B4001A1121B8F6A443A643e40951);

    CurveFactory factory;
    ItemQueue queue;
    LiquidityLock lock;
    EtchRouter etchRouter;
    Swapper swapper;
    SweepVault curveRaffle;
    SweepVault curveBurn;
    address platform = makeAddr("platform");
    address creator = makeAddr("creator");
    address buyer = makeAddr("buyer");

    uint256 constant SUPPLY = 1_000_000_000e18;
    uint256 constant RESERVED = 200_000_000e18;
    uint256 constant CAP = 0.646 ether;
    uint256 constant FROG = 0.64 ether;

    function _template() internal pure returns (FrogCurve.Config memory c) {
        c.supply = SUPPLY;
        c.reserved = RESERVED;
        c.platformFeeCap = 0.25 ether;
        c.platformBps = 500;
        c.poolEthBps = 1_260;
        c.feeBps = 100;
        c.vaultShareBps = 5_000;
        c.creatorShareBps = 3_000;
        c.window = 3 days;
    }

    function _targets() internal pure returns (CurveFactory.Targets memory t) {
        t.normalRaise = 5 ether;
        t.earlyRaise = 8 ether;
        t.earlyUntilEth = 2 ether;
        t.maxRaise = 30 ether;
    }

    function _layout() internal pure returns (LiquidityLock.Params memory p) {
        p.sharedFee = 3000;
        p.sharedSpacing = 60;
        p.coinSpacing = 200;
        p.nearHalf = 960;
        p.wideHalf = 2640;
        p.bidWidth = 1920;
        p.nearBps = 5000;
        p.wideBps = 3000;
        p.maxDeviation = 1000;
        p.maxShift = 960;
        p.minMove = 480;
        p.moveEvery = 1 days;
    }

    function _etchPoolManager() internal {
        if (address(PM).code.length == 0) {
            vm.etch(address(PM), vm.parseBytes(vm.readFile("test/fixtures/v4-pool-manager-4663.hex")));
        }
    }

    function setUp() public virtual override {
        super.setUp();
        _etchPoolManager();
        lock = new LiquidityLock(PM, wrapper, _layout(), PBS);
        queue = new ItemQueue(wrapper, payee, 500, 0.001 ether, 1_000, PBS);
        factory = new CurveFactory(wrapper, lock, queue, platform, _template(), _targets(), PBS);
        curveRaffle = new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(venue), factory, desk, PBS);
        curveBurn = new SweepVault(registry, BURN_LISTING, wrapper, SweepVault.Policy.BURN, address(venue), factory, desk, PBS);
        _ceiling(curveRaffle);
        _ceiling(curveBurn);
        factory.wire(curveRaffle, curveBurn);
        queue.wire(factory);
        lock.wire(factory);
        etchRouter = new EtchRouter(PM, lock, FloorModule(address(0)));
        swapper = new Swapper(PM);
        vm.deal(address(swapper), 1_000 ether);
        _postCap(curveRaffle, CAP);
    }

    // ---- the graduation path ----

    function _launch(SweepVault.Policy policy, bool sweepOnly) internal returns (FrogCurve c, CurveToken tok) {
        vm.prank(creator);
        (address coin, address curve) = factory.launch("Alien", "ALIEN", policy, sweepOnly);
        c = FrogCurve(payable(curve));
        tok = CurveToken(coin);
    }

    function _fillCurve(FrogCurve c) internal {
        uint256 amount = c.raise() * 2;
        vm.deal(buyer, buyer.balance + amount);
        vm.prank(buyer);
        c.buy{value: amount}(0);
    }

    function _freshCap() internal {
        if (!curveRaffle.capFresh()) _postCap(curveRaffle, CAP);
    }

    function _item(uint256 cost) internal {
        _deposit(address(queue));
        _freshCap();
        vm.prank(custodian);
        queue.recordItem(cost);
    }

    /// Launch, sell out the curve, buy its frogs through the queue, graduate into the real PoolManager.
    function _graduate(SweepVault.Policy policy) internal returns (FrogCurve c, CurveToken tok) {
        (c, tok) = _launch(policy, false);
        _fillCurve(c);
        while (!queue.isFilled(address(tok))) _item(FROG);
        _freshCap();
        c.graduate();
    }

    function _graduateSweepOnly() internal returns (FrogCurve c, CurveToken tok) {
        (c, tok) = _launch(SweepVault.Policy.RAFFLE, true);
        _fillCurve(c);
        c.graduate();
    }

    // ---- reading the pools ----

    function _tick(PoolKey memory key) internal view returns (int24 tick) {
        (, tick,,) = PM.getSlot0(key.toId());
    }

    function _sqrtP(PoolKey memory key) internal view returns (uint160 s) {
        (s,,,) = PM.getSlot0(key.toId());
    }

    /// The liquidity of the lock's own position [lo, hi] (salt 0) in `key`, as the PoolManager holds it.
    function _pmLiquidity(PoolKey memory key, int24 lo, int24 hi) internal view returns (uint128 l) {
        (l,,) = PM.getPositionInfo(key.toId(), address(lock), lo, hi, bytes32(0));
    }

    /// ETH per whole token, from a pool's sqrt price, for a pair where the token is currency1 per currency0 = ETH.
    function _ethPerWrapped(PoolKey memory key) internal view returns (uint256) {
        uint256 s = _sqrtP(key);
        // price = (s / 2^96)^2 wX per ETH; ETH per wX = 2^192 / s^2
        return (uint256(2 ** 192) / s) * 1e18 / s;
    }

    function _wrappedFor(address to, uint256 n) internal {
        for (uint256 i = 0; i < n; ++i) _deposit(to);
    }
}
