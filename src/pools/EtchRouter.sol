// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IPoolManager} from "@uniswap/v4-core/src/interfaces/IPoolManager.sol";
import {IUnlockCallback} from "@uniswap/v4-core/src/interfaces/callback/IUnlockCallback.sol";
import {PoolKey} from "@uniswap/v4-core/src/types/PoolKey.sol";
import {Currency} from "@uniswap/v4-core/src/types/Currency.sol";
import {BalanceDelta} from "@uniswap/v4-core/src/types/BalanceDelta.sol";
import {SwapParams} from "@uniswap/v4-core/src/types/PoolOperation.sol";
import {TickMath} from "@uniswap/v4-core/src/libraries/TickMath.sol";
import {StateLibrary} from "@uniswap/v4-core/src/libraries/StateLibrary.sol";
import {LiquidityLock} from "./LiquidityLock.sol";
import {FloorModule} from "../floor/FloorModule.sol";

/// @title EtchRouter
/// @notice A public router for one collection's coins (docs/v3/pools.md, section 7): ETH → COIN and COIN → ETH in one transaction,
/// and ETH ↔ wX directly. Every ETH ↔ wX leg takes the cheaper venue per trade (docs/v5/floor-module.md §8): the shared pool is
/// filled only while its marginal price, fees included, beats the floor module's; the module takes the rest up to its room; the
/// pool again beyond that. For a coin whose pool is COIN/wX the coin leg happens in the same PoolManager `unlock`; for a SWEEP_ONLY
/// coin it is one swap. Exact input, a minimum out, a deadline.
/// @dev No owner, no fee, nothing held between transactions: every wei and unit a call brings is spent or returned in it. Anyone
/// may call it, and anyone may route the same pools and the module without it. It trusts the lock only for the pool keys and the
/// module (optional; zero routes the pool only) for its own quotes, which the module itself enforces.
contract EtchRouter is IUnlockCallback, ReentrancyGuard {
    using SafeERC20 for IERC20;
    using StateLibrary for IPoolManager;

    enum Op { BuyCoin, SellCoin, BuyWrapped, SellWrapped }

    uint256 private constant ONE = 1e18;
    uint256 private constant PIPS = 1_000_000;

    IPoolManager public immutable poolManager;
    LiquidityLock public immutable lock;
    /// @notice The collection's floor module, or zero (pool-only routing).
    FloorModule public immutable module;

    error NotPoolManager();
    error Expired();
    error TooLittle(uint256 out, uint256 minOut);
    error NotAPool();
    error BadInput();
    error TransferFailed();

    constructor(IPoolManager poolManager_, LiquidityLock lock_, FloorModule module_) {
        if (address(poolManager_) == address(0) || address(lock_.poolManager()) != address(poolManager_)) revert BadInput();
        if (address(module_) != address(0) && address(module_.wrapper()) != address(lock_.wrapped())) revert BadInput();
        poolManager = poolManager_;
        lock = lock_;
        module = module_;
    }

    /// @notice Spend all of `msg.value` on `coin`; at least `minOut` of it to `to`.
    function buy(address coin, uint256 minOut, address to, uint256 deadline) external payable nonReentrant returns (uint256 out) {
        if (block.timestamp > deadline) revert Expired();
        if (msg.value == 0 || to == address(0)) revert BadInput();
        uint256 spent;
        (spent, out) = abi.decode(poolManager.unlock(abi.encode(Op.BuyCoin, abi.encode(coin, msg.value, msg.sender, to))),
            (uint256, uint256));
        if (out < minOut) revert TooLittle(out, minOut);
        _refund(spent);
    }

    /// @notice Sell `amountIn` of `coin` (approved to this router); at least `minEthOut` ETH to `to`.
    function sell(address coin, uint256 amountIn, uint256 minEthOut, address to, uint256 deadline)
        external nonReentrant returns (uint256 ethOut)
    {
        if (block.timestamp > deadline) revert Expired();
        if (amountIn == 0 || to == address(0)) revert BadInput();
        (, ethOut) = abi.decode(poolManager.unlock(abi.encode(Op.SellCoin, abi.encode(coin, amountIn, msg.sender, to))),
            (uint256, uint256));
        if (ethOut < minEthOut) revert TooLittle(ethOut, minEthOut);
    }

    /// @notice Spend all of `msg.value` on wX at the best of the shared pool and the floor module; at least `minOut` to `to`.
    function buyWrapped(uint256 minOut, address to, uint256 deadline) external payable nonReentrant returns (uint256 out) {
        if (block.timestamp > deadline) revert Expired();
        if (msg.value == 0 || to == address(0)) revert BadInput();
        uint256 spent;
        (spent, out) = abi.decode(poolManager.unlock(abi.encode(Op.BuyWrapped, abi.encode(address(0), msg.value, msg.sender, to))),
            (uint256, uint256));
        if (out < minOut) revert TooLittle(out, minOut);
        _refund(spent);
    }

    /// @notice Sell `amountIn` wX (approved to this router) at the best of the shared pool and the floor module; at least `minEthOut`.
    function sellWrapped(uint256 amountIn, uint256 minEthOut, address to, uint256 deadline)
        external nonReentrant returns (uint256 ethOut)
    {
        if (block.timestamp > deadline) revert Expired();
        if (amountIn == 0 || to == address(0)) revert BadInput();
        IERC20(address(lock.wrapped())).safeTransferFrom(msg.sender, address(this), amountIn);
        (, ethOut) = abi.decode(poolManager.unlock(abi.encode(Op.SellWrapped, abi.encode(address(0), amountIn, msg.sender, to))),
            (uint256, uint256));
        if (ethOut < minEthOut) revert TooLittle(ethOut, minEthOut);
    }

    function unlockCallback(bytes calldata data) external returns (bytes memory) {
        if (msg.sender != address(poolManager)) revert NotPoolManager();
        (Op op, bytes memory args) = abi.decode(data, (Op, bytes));
        (address coin, uint256 amountIn, address payer, address to) = abi.decode(args, (address, uint256, address, address));
        address w = address(lock.wrapped());
        uint256 spent;
        uint256 out;
        if (op == Op.BuyWrapped) {
            uint256 poolEth;
            (poolEth, out, spent) = _ethToWrapped(amountIn, to, false);
            if (poolEth != 0) poolManager.settle{value: poolEth}();
        } else if (op == Op.SellWrapped) {
            uint256 wToPool;
            (wToPool, out) = _wrappedToEth(amountIn, to, false);
            if (wToPool != 0) {
                poolManager.sync(Currency.wrap(w));
                IERC20(w).safeTransfer(address(poolManager), wToPool);
                poolManager.settle();
            }
            spent = amountIn;
        } else {
            (address curve, bool quoteIsEth,) = lock.coins(coin);
            if (curve == address(0)) revert NotAPool();
            PoolKey memory coinPool = lock.coinKey(coin);
            if (op == Op.BuyCoin) {
                uint256 quoteIn = amountIn;
                uint256 poolEth = amountIn;
                spent = amountIn;
                if (!quoteIsEth) (poolEth, quoteIn, spent) = _ethToWrapped(amountIn, address(this), true);
                (uint256 qPaid, uint256 coinOut) = _swap(coinPool, quoteIsEth ? address(0) : w, quoteIn);
                if (quoteIsEth) spent = qPaid;
                else if (qPaid != quoteIn) revert BadInput();        // exact input: the coin leg took all of it
                if (quoteIsEth) poolEth = qPaid;
                if (poolEth != 0) poolManager.settle{value: poolEth}();
                poolManager.take(Currency.wrap(coin), to, coinOut);
                out = coinOut;
            } else {
                (uint256 coinPaid, uint256 qOut) = _swap(coinPool, coin, amountIn);
                if (quoteIsEth) {
                    poolManager.take(Currency.wrap(address(0)), to, qOut);
                    out = qOut;
                } else {
                    (, out) = _wrappedToEth(qOut, to, true);
                }
                poolManager.sync(Currency.wrap(coin));
                IERC20(coin).safeTransferFrom(payer, address(poolManager), coinPaid);
                poolManager.settle();
                spent = coinPaid;
            }
        }
        return abi.encode(spent, out);
    }

    /// @dev ETH → wX for `ethIn`: the pool while it beats the module's mint price, the module up to its room, the pool for the rest.
    /// The pool's wX is taken to `to`, or left as credit in the PoolManager when `inPool` (a coin leg follows); the module's wX goes
    /// to `to`, or is paid into the PoolManager as credit when `inPool`.
    /// @return poolEth the ETH owed to the PoolManager
    /// @return wOut the wX bought
    /// @return spent the ETH used (the pool's and the module's)
    function _ethToWrapped(uint256 ethIn, address to, bool inPool) private returns (uint256 poolEth, uint256 wOut, uint256 spent) {
        PoolKey memory key = lock.sharedKey();
        (uint256 price, uint256 room) = address(module) == address(0) ? (0, 0) : module.mintCapacity();
        uint256 left = ethIn;
        uint256 poolW;
        if (price != 0) {
            // ETH is currency0: buying wX lowers P (wX per ETH). The pool's marginal wX per ETH is P·(1 − fee); the module's is
            // 1e18 / price. Fill the pool down to P = 1e18 / (price · (1 − fee)).
            uint160 limit = _sqrtLimit(Math.mulDiv(ONE * PIPS, 1 << 192, price * (PIPS - _fee(key, true))));
            if (limit < _sqrtP(key)) {
                (uint256 paid, uint256 got) = _swapLimited(key, true, left, limit);
                left -= paid;
                poolEth += paid;
                poolW += got;
            }
            uint256 toModule = Math.min(left, room);
            if (toModule != 0 && toModule * ONE >= price) {
                uint256 units = module.mint{value: toModule}(inPool ? address(this) : to, 0);
                left -= toModule;
                wOut += units;
                if (inPool) {
                    poolManager.sync(key.currency1);
                    IERC20(Currency.unwrap(key.currency1)).safeTransfer(address(poolManager), units);
                    poolManager.settle();
                }
            }
        }
        if (left != 0) {
            (uint256 paid, uint256 got) = _swap(key, address(0), left);
            left -= paid;
            poolEth += paid;
            poolW += got;
        }
        wOut += poolW;
        spent = ethIn - left;
        if (!inPool && poolW != 0) poolManager.take(key.currency1, to, poolW);
    }

    /// @dev wX → ETH for `wIn`: the pool while it beats the module's burn price, the module up to its room, the pool for the rest.
    /// When `fromPool` the wX is credit in the PoolManager (a coin leg came first) and the module's share is taken out to burn;
    /// otherwise it is this router's balance and the returned `wToPool` must be paid in. ETH goes to `to`.
    function _wrappedToEth(uint256 wIn, address to, bool fromPool) private returns (uint256 wToPool, uint256 ethOut) {
        PoolKey memory key = lock.sharedKey();
        (uint256 price, uint256 room) = address(module) == address(0) ? (0, 0) : module.burnCapacity();
        uint256 left = wIn;
        uint256 poolEthOut;
        if (price != 0) {
            // selling wX raises P. The pool's marginal ETH per wX is (1 − fee) / P; the module's is price / 1e18. Fill the pool up to
            // P = 1e18 · (1 − fee) / price.
            uint160 limit = _sqrtLimit(Math.mulDiv(ONE * (PIPS - _fee(key, false)), 1 << 192, price * PIPS));
            if (limit > _sqrtP(key)) {
                (uint256 paid, uint256 got) = _swapLimited(key, false, left, limit);
                left -= paid;
                wToPool += paid;
                poolEthOut += got;
            }
            uint256 toModule = Math.min(left, room);
            if (toModule != 0 && Math.mulDiv(toModule, price, ONE) != 0) {
                if (fromPool) poolManager.take(key.currency1, address(this), toModule);
                IERC20(Currency.unwrap(key.currency1)).forceApprove(address(module), toModule);
                ethOut += module.burn(toModule, 0, to);
                left -= toModule;
            }
        }
        if (left != 0) {
            (uint256 paid, uint256 got) = _swap(key, Currency.unwrap(key.currency1), left);
            if (paid != left) revert BadInput();
            wToPool += paid;
            poolEthOut += got;
        }
        ethOut += poolEthOut;
        if (poolEthOut != 0) poolManager.take(Currency.wrap(address(0)), to, poolEthOut);
        if (fromPool) wToPool = 0;                                     // paid out of the coin leg's credit already
    }

    /// @dev The pool's total fee for a swap in this direction, pips: Uniswap's protocol fee, then the LP fee on the rest.
    function _fee(PoolKey memory key, bool zeroForOne) private view returns (uint256) {
        (,, uint24 protocolFee, uint24 lpFee) = poolManager.getSlot0(key.toId());
        uint256 pf = zeroForOne ? protocolFee % 4096 : protocolFee >> 12;
        return pf + lpFee - (pf * lpFee) / PIPS;
    }

    function _sqrtP(PoolKey memory key) private view returns (uint160 s) {
        (s,,,) = poolManager.getSlot0(key.toId());
    }

    function _sqrtLimit(uint256 priceX192) private pure returns (uint160) {
        uint256 r = Math.sqrt(priceX192);
        if (r <= TickMath.MIN_SQRT_PRICE) return TickMath.MIN_SQRT_PRICE + 1;
        if (r >= TickMath.MAX_SQRT_PRICE) return TickMath.MAX_SQRT_PRICE - 1;
        return uint160(r);
    }

    /// @dev Exact input of `amountIn` of `tokenIn` in `key`; returns what the pool took and what it gave.
    function _swap(PoolKey memory key, address tokenIn, uint256 amountIn) private returns (uint256 paid, uint256 got) {
        bool zeroForOne = Currency.unwrap(key.currency0) == tokenIn;
        return _swapLimited(key, zeroForOne, amountIn, zeroForOne ? TickMath.MIN_SQRT_PRICE + 1 : TickMath.MAX_SQRT_PRICE - 1);
    }

    function _swapLimited(PoolKey memory key, bool zeroForOne, uint256 amountIn, uint160 limit)
        private returns (uint256 paid, uint256 got)
    {
        BalanceDelta d = poolManager.swap(key, SwapParams(zeroForOne, -int256(amountIn), limit), "");
        (int128 a0, int128 a1) = (d.amount0(), d.amount1());
        (int128 inD, int128 outD) = zeroForOne ? (a0, a1) : (a1, a0);
        paid = uint256(-int256(inD));
        got = uint256(int256(outD));
    }

    function _refund(uint256 spent) private {
        if (msg.value > spent) {
            (bool ok,) = msg.sender.call{value: msg.value - spent}("");
            if (!ok) revert TransferFailed();
        }
    }
}
