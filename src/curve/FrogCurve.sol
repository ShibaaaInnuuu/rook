// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {CurveToken} from "./CurveToken.sol";
import {IItemQueue} from "../interfaces/IItemQueue.sol";
import {SweepVault} from "../SweepVault.sol";
import {IWrappedCollection} from "../interfaces/IWrappedCollection.sol";
import {ILiquiditySeeder} from "../interfaces/ILiquiditySeeder.sol";
import {Wait} from "../lib/Wait.sol";

/// @title FrogCurve
/// @notice One coin's own bonding curve, priced in ETH, whose graduation buys items of its collection through the collection's
/// buy queue (docs/v3/curve.md, docs/v3/pools.md). Generic over the wrapped item (wFROG, wAPE, ...).
///
///   Trading ──(the raise is reached)──▶ Graduating ──(the queue filled this coin's share)──▶ Graduated
///                                            └──────(the window passes, on both clocks, unfilled)──▶ Refunding
///
/// Trading: a constant-product curve over virtual reserves, solved at construction so that its LAST price equals the price the
/// coin's own pool opens at, valued at what its items cost — no jump at graduation. At the close the raise is split: the
/// platform's fee (capped in ETH), the shared pool's ETH side, and the item budget, which goes into the collection's queue behind
/// every coin that closed before it. Graduated: the coin's units come back from the queue; `w_s = poolEth · units / spent` of them
/// pair with the ETH side in the collection's shared pool, the rest open COIN/wX with the reserved supply. Refunding: every coin
/// sold returns for its pro-rata share of the ETH left AND of the units already bought (kept, not sold back).
///
/// SWEEP_ONLY (a creator's choice at launch): no items. The raise, less the platform's fee, opens COIN/ETH at once.
///
/// Liveness (audit 2026-09-26 A-01, docs/audit/2026-09-26/fixes-A-graduation.md): a curve that has not graduated
/// `window + STUCK_GRACE` after its close may open refunds even if the queue filled it (its units are released and refunded pro
/// rata with the ETH) or if it is SWEEP_ONLY (the whole raise). A-03: the buy that would close the curve is refused while the
/// queue has no room to convert the item budget; the curve keeps trading, so nobody is bound to a refund.
///
/// Fees: `feeBps` of every trade in ETH, split vault / creator / the collection's reserve / protocol liquidity / platform
/// (docs/v5/floor-module.md §13.2: the reserve and liquidity slices reach the collection's queue or module, which repays a
/// bootstrap loan first; a SWEEP_ONLY coin sends both to the lock). Once graduated, the coin's pool's LP fees reach here through
/// the lock (`onPoolFees` for ETH, split five ways; `forwardWrappedFees` and `forwardTokenFees` for balances, split three ways,
/// the platform taking the protocol's slices in kind); the shared pool's seeder fees reach here through `depositPoolFees` and go
/// entirely to the vault.
/// @dev What this does NOT protect against:
///  - the canister's cost per item is its word, bounded per item by the keeper's cap plus the queue's buffer.
///  - a slow queue: a coin whose remainder is less than an item waits for later coins, or refunds at its deadline.
///  - the lock misbehaving: it is immutable and ours; the curve checks it took exactly what it was given.
contract FrogCurve is ReentrancyGuard {
    using SafeERC20 for IERC20;

    enum State { Trading, Graduating, Graduated, Refunding }

    struct Config {
        uint256 raise;           // E: ETH in the curve's reserve at the close, fees excluded (set per coin by the factory)
        uint256 supply;          // the coin's whole supply
        uint256 reserved;        // R: kept back for the coin's own pool
        uint256 platformFeeCap;  // the platform's graduation fee never exceeds this, whatever the raise
        uint16 platformBps;      // of the raise: the platform's graduation fee, before the cap
        uint16 poolEthBps;       // of the raise: the shared pool's ETH side
        uint16 feeBps;           // of every trade (and the coin pool's LP fee)
        uint16 vaultShareBps;    // of the fee: to the sweep vault (raffle or burn, per this coin's policy)
        uint16 creatorShareBps;  // of the fee: to the creator
        uint16 reserveShareBps;  // of the fee: to the collection's standing reserve (§13.2)
        uint16 liquidityShareBps; // of the fee: to protocol-owned liquidity. The platform gets the rest.
        uint64 window;           // how long Graduating may last before anyone may start refunds
        bool sweepOnly;          // no items: graduate straight into COIN/ETH
        uint16 launchFeeBps;     // the extra fee on a buy at launch, decaying to nothing (docs/v5/policies-and-fees.md §2); 0: none
        uint64 launchFeeDecay;   // ... over this long, on the slower clock (10 minutes)
    }

    uint256 public constant BPS = 10_000;
    uint256 public constant ONE = 1e18;
    /// @notice How long past the refund deadline a filled (or SWEEP_ONLY) curve that has still not graduated waits before anyone
    /// may open refunds instead (A-01's escape hatch).
    uint256 public constant STUCK_GRACE = 4 days;
    address public constant DEAD = 0x000000000000000000000000000000000000dEaD;

    address public immutable factory;
    CurveToken public immutable token;
    IWrappedCollection public immutable wrapped;
    SweepVault public immutable vault;
    ILiquiditySeeder public immutable seeder;
    /// @notice Where the item budget is converted: the collection's `ItemQueue` or its `FloorModule` (docs/v5/floor-module.md).
    IItemQueue public immutable queue;
    address public immutable creator;
    address public immutable platform;
    uint256 public immutable parentBlockSeconds;
    bool public immutable sweepOnly;

    uint256 public immutable raise;
    uint256 public immutable reserved;
    uint256 public immutable forSale;        // S = supply − reserved
    uint256 public immutable platformFee;    // min(raise · platformBps, platformFeeCap)
    uint256 public immutable poolEth;        // SWEEP_ONLY: the COIN/ETH pool's ETH; else the shared pool's ETH side
    uint256 public immutable itemBudget;     // into the queue at the close (0 for SWEEP_ONLY)
    uint256 public immutable feeBps;
    uint256 public immutable vaultShareBps;
    uint256 public immutable creatorShareBps;
    uint256 public immutable reserveShareBps;
    uint256 public immutable liquidityShareBps;
    uint256 public immutable window;
    uint256 public immutable launchFeeBps;
    uint256 public immutable launchFeeDecay;
    uint64 public immutable launchedAt;
    uint64 public immutable launchedAtBlock;
    /// @notice Virtual reserves: V0 at launch, A = V0 + raise at graduation, where the token reserve is B.
    uint256 public immutable v0;
    uint256 public immutable t0;
    uint256 public immutable vEnd;
    uint256 public immutable tEnd;

    State public state;
    uint256 public v;                        // virtual ETH reserve
    uint256 public t;                        // virtual token reserve
    uint256 public reserveEth;               // ETH that belongs to the raise (not fees)
    uint256 public feesForVault;
    uint256 public creatorOwed;
    uint256 public platformOwed;
    uint256 public reserveOwed;              // the reserve slice, until flushed to the queue (or the lock)
    uint256 public liquidityOwed;            // the liquidity slice, until flushed
    uint64 public gradAt;
    uint64 public gradAtBlock;
    uint256 public refundEthPool;
    uint256 public refundWrappedPool;
    uint256 public returned;                 // coins returned in Refunding
    /// @notice At graduation: the units the queue bought for this coin and what they cost, and how they were split.
    uint256 public unitsBought;
    uint256 public unitsCost;
    uint256 public unitsToShared;

    event Bought(address indexed buyer, uint256 ethIn, uint256 fee, uint256 tokensOut);
    event Sold(address indexed seller, uint256 tokensIn, uint256 ethOut, uint256 fee);
    event GraduationStarted(uint64 deadline, uint256 itemBudget);
    event Graduated(uint256 unitsToCoinPool, uint256 unitsToShared, uint256 ethToShared);
    event RefundsOpened(uint256 ethPool, uint256 wrappedPool);
    event Refunded(address indexed holder, uint256 tokensIn, uint256 ethOut, uint256 wrappedOut);
    event PoolFees(uint256 eth);
    event WrappedFeesForwarded(uint256 toVault, uint256 toCreator, uint256 toPlatform);
    event TokenFeesForwarded(uint256 burned, uint256 toCreator, uint256 toPlatform);
    event SeederFeesDeposited(uint256 eth, uint256 wrappedAmount);
    event ProtocolFeesFlushed(uint256 toReserve, uint256 toLiquidity);

    error WrongState();
    error BadConfig();
    error Slippage();
    error NotSeeder();
    error NotFilled();
    error TooEarly();
    error NotCreator();
    error SeederShort();
    error TransferFailed();
    error QueueFull();
    error Expired();

    constructor(Config memory c, string memory name_, string memory symbol_, IWrappedCollection wrapped_, SweepVault vault_,
        ILiquiditySeeder seeder_, IItemQueue queue_, address creator_, address platform_, uint256 parentBlockSeconds_)
    {
        Wait.checkBlockSeconds(parentBlockSeconds_);
        if (c.raise == 0 || c.reserved == 0 || c.reserved >= c.supply) revert BadConfig();
        if (uint256(c.platformBps) + c.poolEthBps >= BPS) revert BadConfig();
        if (c.feeBps > 1_000 || c.window < 1 hours) revert BadConfig();
        if (c.launchFeeBps > 9_000 || uint256(c.launchFeeBps) + c.feeBps >= BPS) revert BadConfig();
        if (c.launchFeeBps != 0 && c.launchFeeDecay < 1 minutes) revert BadConfig();
        if (uint256(c.vaultShareBps) + c.creatorShareBps + c.reserveShareBps + c.liquidityShareBps > BPS) revert BadConfig();
        if (address(wrapped_) == address(0) || address(vault_) == address(0) || address(seeder_) == address(0)) revert BadConfig();
        if (!c.sweepOnly && address(queue_) == address(0)) revert BadConfig();
        if (creator_ == address(0) || platform_ == address(0)) revert BadConfig();
        if (address(vault_.wrapper()) != address(wrapped_)) revert BadConfig();

        factory = msg.sender;
        wrapped = wrapped_;
        vault = vault_;
        seeder = seeder_;
        queue = queue_;
        creator = creator_;
        platform = platform_;
        parentBlockSeconds = parentBlockSeconds_;
        sweepOnly = c.sweepOnly;
        raise = c.raise;
        feeBps = c.feeBps;
        vaultShareBps = c.vaultShareBps;
        creatorShareBps = c.creatorShareBps;
        reserveShareBps = c.reserveShareBps;
        liquidityShareBps = c.liquidityShareBps;
        window = c.window;
        launchFeeBps = c.launchFeeBps;
        launchFeeDecay = c.launchFeeDecay;
        launchedAt = uint64(block.timestamp);
        launchedAtBlock = uint64(block.number);

        uint256 fee = Math.min((c.raise * c.platformBps) / BPS, c.platformFeeCap);
        platformFee = fee;
        uint256 pe;
        uint256 budget;
        uint256 value;                                              // the coin pool's quote side, in ETH at acquisition cost
        if (c.sweepOnly) {
            pe = c.raise - fee;
            value = pe;
        } else {
            pe = (c.raise * c.poolEthBps) / BPS;
            budget = c.raise - fee - pe;
            if (budget <= 2 * pe) revert BadConfig();               // the coin's own pool must get more than the shared pool
            value = budget - pe;                                    // pe's worth of units go to the shared pool
        }
        poolEth = pe;
        itemBudget = budget;

        // Solve the curve. The coin's pool opens at P = value / R ETH per coin unit. A constant-product curve that takes E in
        // while selling S out and ends at P: A/B = P and (A − E)(B + S) = A·B, so B = E·S / (P·S − E), A = P·B.
        uint256 s_ = c.supply - c.reserved;
        uint256 pS = Math.mulDiv(value, s_, c.reserved);             // P·S
        if (pS <= c.raise) revert BadConfig();                      // the end price must be above the average price
        uint256 b = Math.mulDiv(c.raise, s_, pS - c.raise);
        uint256 a = Math.mulDiv(value, b, c.reserved);
        if (a <= c.raise || b == 0) revert BadConfig();
        uint256 v0_ = a - c.raise;
        uint256 t0_ = b + s_;
        // The token reserve at graduation, rounded UP: every rounding on the curve is in its own favour (k only grows), so the
        // reserve can never fall below this before the raise is reached, and the last buy takes exactly what is left. The
        // integer solution sells S ± a few base units; the reserved supply absorbs the difference.
        uint256 tEnd_ = Math.mulDiv(v0_, t0_, a, Math.Rounding.Ceil);
        if (t0_ - tEnd_ >= c.supply) revert BadConfig();
        v0 = v0_;
        t0 = t0_;
        vEnd = a;
        tEnd = tEnd_;
        forSale = t0_ - tEnd_;
        reserved = c.supply - (t0_ - tEnd_);
        v = v0_;
        t = t0_;

        token = new CurveToken(name_, symbol_, c.supply, address(this));
    }

    receive() external payable {
        // ETH arrives only from the queue (a release's dust or a cancel's refund) and through the payable functions below
        if (msg.sender != address(queue)) revert TransferFailed();
    }

    // ---- Trading -------------------------------------------------------------------------------------------------------------

    /// @notice Buy with `msg.value`; at least `minOut` coins. The buy that reaches the raise is filled up to it, the rest of the
    /// ETH is returned, and the curve closes — unless the queue has no room for the item budget (`QueueFull`, A-03).
    function buy(uint256 minOut) external payable nonReentrant returns (uint256 out) {
        return _buy(minOut);
    }

    /// @notice `buy`, refused after `deadline` (A-07).
    function buy(uint256 minOut, uint256 deadline_) external payable nonReentrant returns (uint256 out) {
        if (block.timestamp > deadline_) revert Expired();
        return _buy(minOut);
    }

    function _buy(uint256 minOut) private returns (uint256 out) {
        if (state != State.Trading) revert WrongState();
        uint256 gross = msg.value;
        uint256 extraBps = launchExtraBps();
        uint256 rate = feeBps + extraBps;
        uint256 fee = (gross * rate) / BPS;
        uint256 net = gross - fee;
        uint256 room = vEnd - v;
        bool last;
        if (net >= room) {
            last = true;
            net = room;
            gross = Math.mulDiv(net, BPS, BPS - rate, Math.Rounding.Ceil);
            // A-02: never book more than was paid (net = room ≤ msg.value − ⌊msg.value·rate/BPS⌋, so fee stays ≥ 0)
            if (gross > msg.value) gross = msg.value;
            fee = gross - net;
            out = t - tEnd;                                           // everything left on the curve
            // A-03: do not close into a queue that cannot convert the budget; the curve keeps trading until it can
            if (!sweepOnly && !queue.hasRoomFor(itemBudget)) revert QueueFull();
        } else {
            out = t - Math.mulDiv(v, t, v + net, Math.Rounding.Ceil); // rounds against the buyer
        }
        if (out < minOut || out == 0) revert Slippage();
        v += net;
        t -= out;
        reserveEth += net;
        // the launch extra goes to the coin's sweep vault, entirely; the base fee is split as always
        uint256 extra = rate == 0 ? 0 : Math.mulDiv(fee, extraBps, rate);
        feesForVault += extra;
        _takeFee(fee - extra);
        emit Bought(msg.sender, gross, fee, out);
        IERC20(address(token)).safeTransfer(msg.sender, out);
        if (last) {
            state = State.Graduating;
            gradAt = uint64(block.timestamp);
            gradAtBlock = uint64(block.number);
            emit GraduationStarted(uint64(block.timestamp + window), itemBudget);
            if (!sweepOnly) {
                reserveEth -= itemBudget;
                queue.enqueue{value: itemBudget}(address(token), uint64(block.timestamp + window),
                    uint64(block.number + Wait.blocksFor(window, parentBlockSeconds)));
            }
        }
        if (msg.value > gross) _send(msg.sender, msg.value - gross);
    }

    /// @notice Sell `amountIn` coins back to the curve for at least `minEth`.
    function sell(uint256 amountIn, uint256 minEth) external nonReentrant returns (uint256 ethOut) {
        return _sell(amountIn, minEth);
    }

    /// @notice `sell`, refused after `deadline` (A-07).
    function sell(uint256 amountIn, uint256 minEth, uint256 deadline_) external nonReentrant returns (uint256 ethOut) {
        if (block.timestamp > deadline_) revert Expired();
        return _sell(amountIn, minEth);
    }

    function _sell(uint256 amountIn, uint256 minEth) private returns (uint256 ethOut) {
        if (state != State.Trading) revert WrongState();
        if (amountIn == 0) revert Slippage();
        uint256 gross = v - Math.mulDiv(v, t, t + amountIn, Math.Rounding.Ceil);   // rounds against the seller
        uint256 fee = (gross * feeBps) / BPS;
        ethOut = gross - fee;
        if (ethOut < minEth || gross == 0) revert Slippage();
        v -= gross;
        t += amountIn;
        reserveEth -= gross;
        _takeFee(fee);
        IERC20(address(token)).safeTransferFrom(msg.sender, address(this), amountIn);
        emit Sold(msg.sender, amountIn, ethOut, fee);
        _send(msg.sender, ethOut);
    }

    /// @notice The extra fee a buy pays now, bps (docs/v5/policies-and-fees.md §2): `launchFeeBps` at launch, falling linearly
    /// to nothing at `launchFeeDecay` on the slower of the two clocks.
    function launchExtraBps() public view returns (uint256) {
        uint256 d = launchFeeDecay;
        if (launchFeeBps == 0 || d == 0) return 0;
        uint256 dt = block.timestamp > launchedAt ? block.timestamp - launchedAt : 0;
        uint256 db = block.number > launchedAtBlock ? (block.number - launchedAtBlock) * parentBlockSeconds : 0;
        uint256 el = Math.min(Math.min(dt, db), d);
        return Math.mulDiv(launchFeeBps, d - el, d);
    }

    function _takeFee(uint256 fee) private {
        uint256 toVault = (fee * vaultShareBps) / BPS;
        uint256 toCreator = (fee * creatorShareBps) / BPS;
        uint256 toReserve = (fee * reserveShareBps) / BPS;
        uint256 toLiquidity = (fee * liquidityShareBps) / BPS;
        feesForVault += toVault;
        creatorOwed += toCreator;
        reserveOwed += toReserve;
        liquidityOwed += toLiquidity;
        platformOwed += fee - toVault - toCreator - toReserve - toLiquidity;
    }

    // ---- Graduating ----------------------------------------------------------------------------------------------------------

    /// @notice Anyone, once the queue has filled this coin (SWEEP_ONLY: at once): open the coin's pool, add its share to the
    /// collection's shared pool, pay the platform its capped graduation fee.
    function graduate() external nonReentrant {
        if (state != State.Graduating) revert WrongState();
        state = State.Graduated;
        reserveEth = 0;
        platformOwed += platformFee;
        IERC20(address(token)).forceApprove(address(seeder), reserved);
        if (sweepOnly) {
            seeder.seedEth{value: poolEth}(address(token), reserved);
            emit Graduated(0, 0, poolEth);
        } else {
            if (!queue.isFilled(address(token))) revert NotFilled();
            (uint256 units, uint256 spent, uint256 dust) = queue.release(address(token));
            // the coin's pool gets exactly the units that cost `itemBudget − poolEth` (its declared value, so it opens at the
            // curve's last price); the shared pool gets the rest of the units with poolEth and the tail the queue did not spend
            uint256 toCoin = Math.min(Math.mulDiv(units, itemBudget - poolEth, spent), units);
            uint256 toShared = units - toCoin;
            unitsBought = units;
            unitsCost = spent;
            unitsToShared = toShared;
            IERC20(address(wrapped)).forceApprove(address(seeder), units);
            seeder.seed{value: poolEth + dust}(address(token), reserved, toCoin, toShared);
            if (wrapped.allowance(address(this), address(seeder)) != 0) revert SeederShort();
            emit Graduated(toCoin, toShared, poolEth + dust);
        }
        if (token.allowance(address(this), address(seeder)) != 0) revert SeederShort();
    }

    /// @notice Anyone, once the window has passed on both clocks unfilled: take back from the queue what it holds for this coin,
    /// and open refunds. Also (A-01's escape hatch), once `window + STUCK_GRACE` has passed on both clocks with the curve still
    /// not graduated although the queue filled it (or it is SWEEP_ONLY): the queue releases the units bought for it, and holders
    /// refund pro rata the ETH here and those units. The item budget stays in the queue as those units' backing: nothing is paid
    /// twice. While the queue holds refunds (the floor module's keeper is paused, so nothing can be filled, review2 N-3; and for a
    /// grace after the unpause, R3 check A3-L1), an unfilled curve waits for the escape hatch's time instead of its window: a pause
    /// gets that long to be lifted and filled after, and never longer.
    function openRefunds() external nonReentrant {
        if (state != State.Graduating) revert WrongState();
        bool filled = !sweepOnly && queue.isFilled(address(token));
        if (sweepOnly || filled) {
            if (!_stuck()) revert WrongState();                     // before the hatch: graduate() is the only way on
        } else if (!Wait.elapsed(uint64(gradAt + window), uint64(gradAtBlock + Wait.blocksFor(window, parentBlockSeconds)))) {
            revert TooEarly();
        } else if (queue.holdsRefunds() && !_stuck()) {
            revert TooEarly();
        }
        state = State.Refunding;
        uint256 eth;
        uint256 units;
        if (filled) (units,, eth) = queue.release(address(token));
        else if (!sweepOnly) (eth, units) = queue.cancel(address(token));
        reserveEth += eth;
        refundEthPool = reserveEth;
        refundWrappedPool = units;
        emit RefundsOpened(refundEthPool, refundWrappedPool);
    }

    /// @dev Past `window + STUCK_GRACE` on both clocks since the close.
    function _stuck() private view returns (bool) {
        uint256 w = window + STUCK_GRACE;
        return Wait.elapsed(uint64(gradAt + w), uint64(gradAtBlock + Wait.blocksFor(w, parentBlockSeconds)));
    }

    // ---- Refunding -----------------------------------------------------------------------------------------------------------

    /// @notice Return `amount` coins for their pro-rata share of the ETH left and the units already bought.
    function refund(uint256 amount) external nonReentrant returns (uint256 ethOut, uint256 wrappedOut) {
        if (state != State.Refunding) revert WrongState();
        if (amount == 0) revert Slippage();
        ethOut = Math.mulDiv(refundEthPool, amount, forSale);
        wrappedOut = Math.mulDiv(refundWrappedPool, amount, forSale);
        returned += amount;
        reserveEth -= ethOut;
        IERC20(address(token)).safeTransferFrom(msg.sender, address(this), amount);
        emit Refunded(msg.sender, amount, ethOut, wrappedOut);
        if (wrappedOut != 0) IERC20(address(wrapped)).safeTransfer(msg.sender, wrappedOut);
        _send(msg.sender, ethOut);
    }

    // ---- fees ----------------------------------------------------------------------------------------------------------------

    /// @notice Anyone: the vault's share of the ETH fees, credited to this coin.
    function flushVaultFees() external nonReentrant {
        uint256 amount = feesForVault;
        feesForVault = 0;
        if (amount != 0) vault.deposit{value: amount}(address(token));
    }

    function claimCreatorFees() external nonReentrant {
        if (msg.sender != creator) revert NotCreator();
        uint256 amount = creatorOwed;
        creatorOwed = 0;
        _send(creator, amount);
    }

    /// @notice Anyone: the platform's trade fees and, once graduated, its graduation fee — to the platform, nowhere else.
    function claimPlatformFees() external nonReentrant {
        uint256 amount = platformOwed;
        platformOwed = 0;
        _send(platform, amount);
    }

    /// @notice Anyone: the collection's reserve slice to its queue or module (`fundReserve`), the liquidity slice to the same
    /// (`fundLiquidity`, which hands it to the lock) — or both straight to the lock for a SWEEP_ONLY coin, which has no queue.
    /// docs/v5/floor-module.md §13.2.
    function flushProtocolFees() external nonReentrant {
        uint256 toReserve = reserveOwed;
        uint256 toLiquidity = liquidityOwed;
        reserveOwed = 0;
        liquidityOwed = 0;
        emit ProtocolFeesFlushed(toReserve, toLiquidity);
        if (sweepOnly) {
            if (toReserve + toLiquidity != 0) seeder.addProtocolLiquidity{value: toReserve + toLiquidity}();
            return;
        }
        if (toReserve != 0) queue.fundReserve{value: toReserve}();
        if (toLiquidity != 0) queue.fundLiquidity{value: toLiquidity}();
    }

    /// @notice The lock: the coin's own COIN/ETH pool's LP fees (SWEEP_ONLY), or ETH it could not pair at seeding. Split as the
    /// trade fees are.
    /// @dev Not `nonReentrant`: the lock calls it from inside `graduate` (a SWEEP_ONLY seed that could not pair every wei). It
    /// only books `msg.value` as fees, from the seeder only, and calls nothing.
    function onPoolFees() external payable {
        if (msg.sender != address(seeder)) revert NotSeeder();
        _takeFee(msg.value);
        emit PoolFees(msg.value);
    }

    /// @notice The lock: this coin's seeder fees from the collection's shared pool. All of it to the coin's sweep vault.
    function depositPoolFees(uint256 wrappedAmount) external payable nonReentrant {
        if (msg.sender != address(seeder)) revert NotSeeder();
        if (wrappedAmount != 0) {
            IERC20(address(wrapped)).safeTransferFrom(msg.sender, address(this), wrappedAmount);
            IERC20(address(wrapped)).forceApprove(address(vault), wrappedAmount);
            vault.depositWrapped(address(token), wrappedAmount);
        }
        if (msg.value != 0) vault.deposit{value: msg.value}(address(token));
        emit SeederFeesDeposited(msg.value, wrappedAmount);
    }

    /// @notice Anyone, once graduated: the coin's pool's fees in wX arrive here; split them as the ETH fees are.
    function forwardWrappedFees() external nonReentrant {
        if (state != State.Graduated) revert WrongState();
        uint256 bal = wrapped.balanceOf(address(this));
        if (bal == 0) return;
        uint256 toVault = (bal * vaultShareBps) / BPS;
        uint256 toCreator = (bal * creatorShareBps) / BPS;
        uint256 toPlatform = bal - toVault - toCreator;
        if (toVault != 0) {
            IERC20(address(wrapped)).forceApprove(address(vault), toVault);
            vault.depositWrapped(address(token), toVault);
        }
        if (toCreator != 0) IERC20(address(wrapped)).safeTransfer(creator, toCreator);
        if (toPlatform != 0) IERC20(address(wrapped)).safeTransfer(platform, toPlatform);
        emit WrappedFeesForwarded(toVault, toCreator, toPlatform);
    }

    /// @notice Anyone, once graduated: the coin's pool's fees in this coin arrive here. The creator's and the platform's shares
    /// are paid in the coin; the vault's share is burned (the vault holds only wX and ETH, and swapping coins for wX without an
    /// oracle is a trade anyone can sandwich; burning returns the value to every holder pro rata).
    function forwardTokenFees() external nonReentrant {
        if (state != State.Graduated) revert WrongState();
        uint256 bal = token.balanceOf(address(this));
        if (bal == 0) return;
        uint256 toBurn = (bal * vaultShareBps) / BPS;
        uint256 toCreator = (bal * creatorShareBps) / BPS;
        uint256 toPlatform = bal - toBurn - toCreator;
        if (toBurn != 0) IERC20(address(token)).safeTransfer(DEAD, toBurn);
        if (toCreator != 0) IERC20(address(token)).safeTransfer(creator, toCreator);
        if (toPlatform != 0) IERC20(address(token)).safeTransfer(platform, toPlatform);
        emit TokenFeesForwarded(toBurn, toCreator, toPlatform);
    }

    // ---- views ---------------------------------------------------------------------------------------------------------------

    /// @notice The curve's current marginal price, ETH (wei) per whole coin.
    function price() external view returns (uint256) { return Math.mulDiv(v, ONE, t); }
    /// @notice The price the coin's own pool opens at, ETH (wei) per whole coin, valuing its units at what they cost.
    function poolOpenPrice() external view returns (uint256) {
        return Math.mulDiv(sweepOnly ? poolEth : itemBudget - poolEth, ONE, reserved);
    }
    function deadline() external view returns (uint64 at, uint64 atBlock) {
        return (uint64(gradAt + window), uint64(gradAtBlock + Wait.blocksFor(window, parentBlockSeconds)));
    }
    /// @notice Whether `graduate()` may be called now (Graduating, and filled or SWEEP_ONLY).
    function canGraduate() external view returns (bool) {
        return state == State.Graduating && (sweepOnly || queue.isFilled(address(token)));
    }
    /// @notice When `openRefunds()` becomes callable (both clocks), for a Graduating curve: the deadline if unfilled, the escape
    /// hatch's `window + STUCK_GRACE` if filled or SWEEP_ONLY, or unfilled while the queue holds refunds (a keeper pause, N-3, or the
    /// grace after it, A3-L1: the latest it can be; the hold may end sooner).
    /// (0, 0) in any other state.
    function refundsOpenAt() external view returns (uint64 at, uint64 atBlock) {
        if (state != State.Graduating) return (0, 0);
        uint256 w = (sweepOnly || queue.isFilled(address(token)) || queue.holdsRefunds()) ? window + STUCK_GRACE : window;
        return (uint64(gradAt + w), uint64(gradAtBlock + Wait.blocksFor(w, parentBlockSeconds)));
    }
    /// @notice Whether the buy that reaches the raise would be accepted now (A-03): the queue has room for the item budget (none
    /// while the floor module's keeper is paused, N-3).
    function canClose() external view returns (bool) {
        return state == State.Trading && (sweepOnly || queue.hasRoomFor(itemBudget));
    }
    /// @notice Coins sold and ETH raised, for the page (the queue's `entry(token)` has the item side).
    function progress() external view returns (uint256 sold, uint256 raised) {
        return (t0 - t, v - v0);
    }

    function _send(address to, uint256 amount) private {
        if (amount == 0) return;
        (bool ok,) = to.call{value: amount}("");
        if (!ok) revert TransferFailed();
    }
}
