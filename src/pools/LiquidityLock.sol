// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IPoolManager} from "@uniswap/v4-core/src/interfaces/IPoolManager.sol";
import {IUnlockCallback} from "@uniswap/v4-core/src/interfaces/callback/IUnlockCallback.sol";
import {IHooks} from "@uniswap/v4-core/src/interfaces/IHooks.sol";
import {PoolKey} from "@uniswap/v4-core/src/types/PoolKey.sol";
import {Currency} from "@uniswap/v4-core/src/types/Currency.sol";
import {BalanceDelta} from "@uniswap/v4-core/src/types/BalanceDelta.sol";
import {ModifyLiquidityParams, SwapParams} from "@uniswap/v4-core/src/types/PoolOperation.sol";
import {StateLibrary} from "@uniswap/v4-core/src/libraries/StateLibrary.sol";
import {TickMath} from "@uniswap/v4-core/src/libraries/TickMath.sol";
import {ILiquiditySeeder} from "../interfaces/ILiquiditySeeder.sol";
import {ICapSource} from "../interfaces/ICapSource.sol";
import {IWrappedCollection} from "../interfaces/IWrappedCollection.sol";
import {LiquidityAmounts} from "./LiquidityAmounts.sol";
import {Wait} from "../lib/Wait.sol";

/// @notice What the lock calls on a coin's curve: fees in, nothing out.
interface IPoolFeeSink {
    function onPoolFees() external payable;
    function depositPoolFees(uint256 wrappedAmount) external payable;
    function forwardWrappedFees() external;
    function forwardTokenFees() external;
    function feeBps() external view returns (uint256);
    function sweepOnly() external view returns (bool);
}

/// @notice The collection's floor module, when the factory's queue is one: the reference a shared add is priced against (A-04).
interface IFloorReference {
    function fresh() external view returns (bool);
    function price() external view returns (uint256);
    function gradPrice() external view returns (uint256);
}

/// @notice What the lock reads of a `CurveFactory` beyond `ICapSource`: its queue, probed once for a floor module.
interface IFactoryQueue {
    function queue() external view returns (address);
}

/// @title LiquidityLock
/// @notice One collection's protocol-owned Uniswap v4 liquidity, and the seeder its graduating curves hand it to
/// (docs/v3/pools.md). It opens each coin's own hookless pool with one wide position (±`FAR_HALF` ticks around its seed price),
/// adds each coin's contribution to the collection's one shared wX/ETH pool as LAYERS around a center (near band, wide band, the
/// far layer of ±`FAR_HALF`, and an ETH-only bid), and
/// pays the fees those positions earn: a coin pool's to its curve (split vault / creator / platform there), the shared pool's
/// pro rata to the coins that seeded it (seeder shares), to their sweep vaults.
///
/// **Nothing ever leaves.** The positions are the lock's own in the PoolManager (no NFT, nothing to transfer), and there is no
/// code path that passes a negative liquidity delta: `modifyLiquidity` is called only with a positive delta (an add) or zero (a
/// fee poke). There is no owner. The only addresses tokens are sent to are the PoolManager (to pay for an add or an alignment),
/// the curve of the coin whose fees they are, and — for the protocol's own liquidity (`PROTOCOL`, docs/v5/floor-module.md
/// §13.2: fee slices anyone may add, never withdrawable) — the treasury, for its ETH fees only.
///
/// @dev What this does NOT protect against:
///  - the keeper's cap: it gates when a shared-pool add may happen (the pool price must be within `maxDeviation` ticks of it) and
///    steers the center for NEW liquidity by at most `maxShift` ticks per `moveEvery`. It cannot move liquidity already added.
///  - third-party LPs in either pool: they earn their share of the LP fee, as anyone may.
///  - Uniswap's protocol fee on 4663 (measured: 0.1% on a 1% pool, 0.05% on 0.3%), taken before LP fees.
///  - rounding: seeder-share payouts round down; at most a few wei per position per claim stay here, owed to nobody.
///
/// Audit 2026-09-26 A-01: no position's bound is on a tick that is cheap to fill to v4's `maxLiquidityPerTick` (the widest
/// layer is ±`FAR_HALF`, not the usable-tick extremes), and a failed add never reverts a graduation: the coin's seed and its
/// shared add are attempted in self-calls, and what fails waits here for anyone's retry (`seedCoin`, `deploy`).
/// A-04: in a stack whose queue is a floor module, shared adds are priced against the module's fresh floor within
/// `REF_DEVIATION` (docs/audit/2026-09-26/fixes-A-graduation.md).
contract LiquidityLock is ILiquiditySeeder, IUnlockCallback, ReentrancyGuard {
    using SafeERC20 for IERC20;
    using SafeERC20 for IWrappedCollection;
    using StateLibrary for IPoolManager;

    struct Params {
        uint24 sharedFee;      // the shared pool's LP fee, pips (3000 = 0.30%)
        int24 sharedSpacing;   // 60
        int24 coinSpacing;     // coin pools' tick spacing (200); their fee is the curve's feeBps
        int24 nearHalf;        // ±960 ticks ≈ ±10%
        int24 wideHalf;        // ±2640 ticks ≈ ±30%
        int24 bidWidth;        // the ETH-only bid above the near band (tick up = wX cheaper): 1920 ticks
        uint16 nearBps;        // share of each contribution offered to the near band (5000)
        uint16 wideBps;        // ... to the wide band (3000); full range gets the rest
        int24 maxDeviation;    // a shared add waits unless the pool tick is within this of the cap's tick (1000 ≈ 10%)
        int24 maxShift;        // the center moves at most this far per move (960)
        int24 minMove;         // ... and only when the cap is at least this far from it (480)
        uint64 moveEvery;      // ... and at most once per this period, on both clocks (1 day)
    }

    struct Position {
        int24 lower;
        int24 upper;
        uint128 liquidity;          // == the sum of every coin's units in it: no liquidity is ever removed
        uint256 accEthX128;         // fees per unit of liquidity, ever, Q128
        uint256 accWrappedX128;
    }

    struct Share {
        uint128 units;              // liquidity this coin added to the position
        uint256 ckEth;              // the accumulators when this coin's owed amounts were last brought up to date
        uint256 ckWrapped;
    }

    struct Coin {
        address curve;              // who seeded it, and where its fees go
        bool quoteIsEth;            // SWEEP_ONLY: COIN/ETH
        uint128 liquidity;          // its own pool's full-range position
    }

    enum Op { SeedCoin, AddShared, Align, Poke, PokeCoin }

    uint256 public constant BPS = 10_000;
    /// @notice The pseudo-coin protocol-owned liquidity is held under: layers like any coin's, no curve, nothing to pay out.
    address public constant PROTOCOL = address(1);
    uint256 private constant Q128 = 2 ** 128;
    uint256 private constant ONE = 1e18;
    /// @dev Kept back from each add so the pool's own round-ups never ask for more than the lock was given for it.
    uint256 private constant MARGIN = 2;
    /// @notice The widest layer's half-width, ticks, around its reference (the shared pool's first center, or a coin's seed
    /// price). At this
    /// distance from the price, filling v4's per-tick liquidity cap costs ~1e6–1e9 ETH (fixes-A-graduation.md, A-01), where the
    /// usable-tick extremes cost ~1e-6 ETH. A multiple of both spacings (60, 200); ×/÷ 6.6e7 in price.
    int24 public constant FAR_HALF = 180_000;
    /// @notice With a floor module as reference: a shared add waits unless the pool tick is within this of the module's price
    /// (≈ 2%). The pool is arbitraged against the module, so a fair pool sits within its spread plus the pool fee.
    int24 public constant REF_DEVIATION = 200;

    IPoolManager public immutable poolManager;
    IWrappedCollection public immutable wrapped;
    address public immutable deployer;
    uint256 public immutable parentBlockSeconds;
    uint24 public immutable sharedFee;
    int24 public immutable sharedSpacing;
    int24 public immutable coinSpacing;
    int24 public immutable nearHalf;
    int24 public immutable wideHalf;
    int24 public immutable bidWidth;
    uint256 public immutable nearBps;
    uint256 public immutable wideBps;
    int24 public immutable maxDeviation;
    int24 public immutable maxShift;
    int24 public immutable minMove;
    uint64 public immutable moveEvery;

    ICapSource public factory;
    /// @notice The factory's queue when it is a floor module (found at `wire`), or zero: then the keeper's cap is the reference.
    IFloorReference public floorRef;
    /// @notice Where the protocol positions' ETH fees go (§13.2); zero: they compound as more protocol liquidity.
    address public treasury;

    // ---- the shared pool ----
    bool public centered;
    int24 public center;
    /// @notice The far layer's ticks: ±`FAR_HALF` around the FIRST center, fixed for good (like the full range it replaces, one
    /// position shared by every graduation; a later price move of ×/÷ 4.8e8 still leaves its ticks ~1e5 ETH to saturate).
    int24 public farLower;
    int24 public farUpper;
    uint64 public centerMovedAt;
    uint64 public centerMovedAtBlock;
    /// @notice ETH and wX that graduations contributed to the shared pool, ever (added or still pending).
    uint256 public sharedEthIn;
    uint256 public sharedWrappedIn;
    bytes32[] public positionIds;
    mapping(bytes32 id => Position) private _positions;
    mapping(bytes32 id => mapping(address coin => Share)) private _shares;
    mapping(address coin => bytes32[]) private _coinPositions;
    mapping(bytes32 id => mapping(address coin => bool)) private _holds;
    /// @notice A coin's contribution not yet added (the price was out of the guard, or the amounts could not be paired).
    mapping(address coin => uint256) public pendingEth;
    mapping(address coin => uint256) public pendingWrapped;
    /// @notice Seeder fees credited to a coin and not yet paid to it.
    mapping(address coin => uint256) public owedEth;
    mapping(address coin => uint256) public owedWrapped;
    /// @notice Totals, for the books: every fee the shared positions collected, and every fee paid out.
    uint256 public feesInEth;
    uint256 public feesInWrapped;
    uint256 public feesPaidEth;
    uint256 public feesPaidWrapped;

    mapping(address coin => Coin) public coins;
    /// @notice A coin's own pool position, [lower, upper], once seeded.
    mapping(address coin => int24[2]) private _coinRange;
    /// @notice A coin whose pool seed failed (A-01 fail soft): its reserved coins and its pool's quote (wX, or ETH for
    /// SWEEP_ONLY), held here until `seedCoin` succeeds.
    mapping(address coin => uint256) public pendingCoinToken;
    mapping(address coin => uint256) public pendingCoinQuote;

    event Wired(address factory);
    event TreasuryWired(address treasury);
    event ProtocolLiquidityAdded(address indexed from, uint256 eth);
    event ProtocolFeesPaid(uint256 eth, uint256 wrappedKept);
    event CoinPoolSeeded(address indexed coin, uint128 liquidity, uint256 tokenUsed, uint256 quoteUsed);
    event CoinSeedDeferred(address indexed coin, bytes reason);
    event SharedAdded(address indexed coin, bytes32 indexed position, uint128 liquidity, uint256 ethUsed, uint256 wrappedUsed);
    event SharedDeferred(address indexed coin, uint8 reason);
    event CenterSet(int24 center);
    event CenterMoved(int24 from, int24 to);
    event SharedFeesPaid(address indexed coin, uint256 eth, uint256 wrappedAmount);
    event CoinFeesPaid(address indexed coin, uint256 tokenFees, uint256 quoteFees);

    error NotDeployer();
    error AlreadyWired();
    error NotWired();
    error NotCurve();
    error AlreadySeeded();
    error NotSeeded();
    error NotPoolManager();
    error BadParams();
    error BadInput();
    error NoCap();
    error TooSoon();
    error NoMove();
    error Overspent();
    error TransferFailed();
    error NotSelf();
    error NothingPending();
    error OutOfGas();

    constructor(IPoolManager poolManager_, IWrappedCollection wrapped_, Params memory p, uint256 parentBlockSeconds_) {
        if (address(poolManager_) == address(0) || address(wrapped_) == address(0)) revert BadParams();
        Wait.checkBlockSeconds(parentBlockSeconds_);
        if (p.sharedSpacing <= 0 || p.coinSpacing <= 0 || p.sharedFee > 100_000) revert BadParams();
        if (p.nearHalf <= 0 || p.wideHalf <= p.nearHalf || p.bidWidth <= 0) revert BadParams();
        if (p.nearHalf % p.sharedSpacing != 0 || p.wideHalf % p.sharedSpacing != 0 || p.bidWidth % p.sharedSpacing != 0) revert BadParams();
        if (p.maxShift % p.sharedSpacing != 0 || p.maxShift <= 0 || p.minMove <= 0 || p.maxDeviation <= 0) revert BadParams();
        if (uint256(p.nearBps) + p.wideBps > BPS || p.moveEvery < 1 hours) revert BadParams();
        if (p.wideHalf + p.nearHalf + p.bidWidth > 200_000 || p.wideHalf >= FAR_HALF) revert BadParams();
        poolManager = poolManager_;
        wrapped = wrapped_;
        deployer = msg.sender;
        parentBlockSeconds = parentBlockSeconds_;
        sharedFee = p.sharedFee;
        sharedSpacing = p.sharedSpacing;
        coinSpacing = p.coinSpacing;
        nearHalf = p.nearHalf;
        wideHalf = p.wideHalf;
        bidWidth = p.bidWidth;
        nearBps = p.nearBps;
        wideBps = p.wideBps;
        maxDeviation = p.maxDeviation;
        maxShift = p.maxShift;
        minMove = p.minMove;
        moveEvery = p.moveEvery;
    }

    /// @notice Once, by the deployer: the collection's factory, which names each coin's curve and serves the keeper's cap.
    function wire(ICapSource factory_) external {
        if (msg.sender != deployer) revert NotDeployer();
        if (address(factory) != address(0)) revert AlreadyWired();
        if (factory_.wrapped() != address(wrapped)) revert BadParams();
        factory = factory_;
        // A-04: a factory whose queue is a floor module gives the shared adds a reference near the pool's fair price
        try IFactoryQueue(address(factory_)).queue() returns (address q) {
            if (q.code.length != 0) {
                try IFloorReference(q).gradPrice() returns (uint256) { floorRef = IFloorReference(q); } catch {}
            }
        } catch {}
        emit Wired(address(factory_));
    }

    /// @notice Once, by the deployer: the protocol treasury, which receives the protocol positions' ETH fees from then on.
    function wireTreasury(address treasury_) external {
        if (msg.sender != deployer) revert NotDeployer();
        if (treasury != address(0)) revert AlreadyWired();
        if (treasury_ == address(0)) revert BadParams();
        treasury = treasury_;
        emit TreasuryWired(treasury_);
    }

    /// @dev The PoolManager pays out ETH by `take`; nobody else sends ETH here except through `seed`/`seedEth`/`addProtocolLiquidity`.
    receive() external payable {
        if (msg.sender != address(poolManager)) revert NotPoolManager();
    }

    /// @inheritdoc ILiquiditySeeder
    /// @dev Protocol-owned liquidity (§13.2): ETH only, paired with the wX the protocol positions have earned, the rest in the
    /// ETH-only bid layer; what the price guard defers waits as `pendingEth[PROTOCOL]` for `deploy(PROTOCOL)`.
    function addProtocolLiquidity() external payable nonReentrant {
        if (msg.value == 0) revert BadInput();
        pendingEth[PROTOCOL] += msg.value;
        emit ProtocolLiquidityAdded(msg.sender, msg.value);
        if (address(factory) != address(0)) _tryDeploy(PROTOCOL);
    }

    // ---- seeding ---------------------------------------------------------------------------------------------------------

    function _onlyCurveOf(address token) private view {
        if (address(factory) == address(0)) revert NotWired();
        if (token == address(0) || msg.sender != factory.routerOf(token)) revert NotCurve();
        if (coins[token].curve != address(0)) revert AlreadySeeded();
    }

    /// @inheritdoc ILiquiditySeeder
    function seed(address token, uint256 tokenAmount, uint256 wrappedForToken, uint256 wrappedForShared) external payable nonReentrant {
        _onlyCurveOf(token);
        if (tokenAmount == 0 || wrappedForToken == 0) revert BadInput();
        coins[token].curve = msg.sender;
        IERC20(token).safeTransferFrom(msg.sender, address(this), tokenAmount);
        wrapped.safeTransferFrom(msg.sender, address(this), wrappedForToken + wrappedForShared);
        sharedEthIn += msg.value;
        sharedWrappedIn += wrappedForShared;

        // a deferred coin seed (quoteLeft = 0, its quote kept in pendingCoinQuote) does not hold back the shared add
        (, uint256 quoteLeft) = _trySeedCoin(token, tokenAmount, wrappedForToken);
        pendingEth[token] += msg.value;
        pendingWrapped[token] += wrappedForShared + quoteLeft;
        _tryDeploy(token);
    }

    /// @inheritdoc ILiquiditySeeder
    function seedEth(address token, uint256 tokenAmount) external payable nonReentrant {
        _onlyCurveOf(token);
        if (tokenAmount == 0 || msg.value == 0) revert BadInput();
        coins[token].curve = msg.sender;
        coins[token].quoteIsEth = true;
        IERC20(token).safeTransferFrom(msg.sender, address(this), tokenAmount);
        (bool ok, uint256 ethLeft) = _trySeedCoin(token, tokenAmount, msg.value);
        // what could not be paired goes back as the coin's pool fees: it was its raise
        if (ok && ethLeft != 0) IPoolFeeSink(msg.sender).onPoolFees{value: ethLeft}();
    }

    /// @notice Anyone: retry the pool seed of a coin whose seed failed at graduation. Reverts with the reason if it fails again.
    /// The quote left over goes where it would have: wX to the coin's shared contribution, ETH back to its curve as pool fees.
    function seedCoin(address coin) external nonReentrant {
        uint256 tokenAmount = pendingCoinToken[coin];
        uint256 quoteAmount = pendingCoinQuote[coin];
        if (tokenAmount == 0) revert NothingPending();
        pendingCoinToken[coin] = 0;
        pendingCoinQuote[coin] = 0;
        uint256 left = _seedCoinPool(coin, tokenAmount, quoteAmount);
        Coin storage c = coins[coin];
        if (c.quoteIsEth) {
            if (left != 0) IPoolFeeSink(c.curve).onPoolFees{value: left}();
        } else {
            pendingWrapped[coin] += left;
            _tryDeploy(coin);
        }
    }

    /// @dev A-01 fail soft: the seed runs in a self-call, so a revert anywhere in it (the PoolManager's side included) is undone
    /// and the amounts wait in `pendingCoin*` for `seedCoin`. A call starved of gas does not count as a failure: see `_caught`.
    function _trySeedCoin(address token, uint256 tokenAmount, uint256 quoteAmount) private returns (bool ok, uint256 left) {
        uint256 g = gasleft();
        try this.seedCoinPoolSelf(token, tokenAmount, quoteAmount) returns (uint256 l) {
            return (true, l);
        } catch (bytes memory reason) {
            _caught(g);
            pendingCoinToken[token] = tokenAmount;
            pendingCoinQuote[token] = quoteAmount;
            emit CoinSeedDeferred(token, reason);
            return (false, 0);
        }
    }

    /// @dev The lock only. External so that `_trySeedCoin` can catch it.
    function seedCoinPoolSelf(address token, uint256 tokenAmount, uint256 quoteAmount) external returns (uint256) {
        if (msg.sender != address(this)) revert NotSelf();
        return _seedCoinPool(token, tokenAmount, quoteAmount);
    }

    /// @dev After a caught failure: refuse it if the attempt used almost all the gas it had, so nobody can defer a seed or an
    /// add by calling with just too little gas (and a gas estimate never lands on a deferral). A callee that ran out of gas
    /// leaves the caller ~1/64 of what it had (EIP-150); one whose nested call ran out and bubbled the revert up leaves ~1/64
    /// per level, a few levels deep here — under 1/8 either way. A genuine revert (a full tick, a bad state) happens early and
    /// leaves far more; one that happens late with gas this tight reverts the whole call, and a retry with more gas defers it.
    function _caught(uint256 gasBefore) private view {
        if (gasleft() < gasBefore / 8) revert OutOfGas();
    }

    /// @dev Opens the coin's own pool at the seed price (or aligns a pool somebody opened first), adds one position of ±`FAR_HALF`
    /// ticks around the seed price (A-01: never on the cheaply saturable usable-tick extremes).
    /// @return quoteLeft the quote (wX or ETH) the add could not pair. Coin left over stays here: burned in effect.
    function _seedCoinPool(address token, uint256 tokenAmount, uint256 quoteAmount) private returns (uint256 quoteLeft) {
        PoolKey memory key = coinKey(token);
        bool tokenIs0 = Currency.unwrap(key.currency0) == token;
        (uint256 a0, uint256 a1) = tokenIs0 ? (tokenAmount, quoteAmount) : (quoteAmount, tokenAmount);
        uint160 target = _sqrtRatio(a0, a1);
        (int24 lo, int24 hi) = _farRange(TickMath.getTickAtSqrtPrice(target), key.tickSpacing);
        (uint160 sqrtP,,,) = poolManager.getSlot0(key.toId());
        if (sqrtP == 0) poolManager.initialize(key, target);
        (uint128 liq, uint256 used0, uint256 used1, uint256 left) = abi.decode(
            poolManager.unlock(abi.encode(Op.SeedCoin, abi.encode(key, target, a0, a1, lo, hi))), (uint128, uint256, uint256, uint256));
        coins[token].liquidity = liq;
        _coinRange[token] = [lo, hi];
        (uint256 tokenUsed, uint256 quoteUsed) = tokenIs0 ? (used0, used1) : (used1, used0);
        quoteLeft = left;
        emit CoinPoolSeeded(token, liq, tokenUsed, quoteUsed);
    }

    // ---- the shared pool -------------------------------------------------------------------------------------------------

    /// @notice Anyone: add `coin`'s pending contribution to the shared pool's layers, if the pool price is within the guard.
    function deploy(address coin) external nonReentrant {
        if (coin != PROTOCOL && (coins[coin].curve == address(0) || coins[coin].quoteIsEth)) revert NotSeeded();
        if (address(factory) == address(0)) revert NotWired();
        _deploy(coin);
    }

    /// @dev A-01 fail soft: the add runs in a self-call; a revert leaves the amounts pending (reason 4) for `deploy`.
    function _tryDeploy(address coin) private {
        if (pendingEth[coin] == 0 && pendingWrapped[coin] == 0) return;
        uint256 g = gasleft();
        try this.deploySelf(coin) {} catch {
            _caught(g);
            emit SharedDeferred(coin, 4);
        }
    }

    /// @dev The lock only. External so that `_tryDeploy` can catch it.
    function deploySelf(address coin) external {
        if (msg.sender != address(this)) revert NotSelf();
        _deploy(coin);
    }

    /// @notice The price shared adds are checked against, wei per whole wX, and how far (ticks) the pool may be from it: the floor
    /// module's while fresh (A-04), else the keeper's fresh cap within `maxDeviation`. `ok` false: no fresh reference.
    function priceReference() public view returns (uint256 priceWei, int24 deviation, bool ok) {
        IFloorReference m = floorRef;
        if (address(m) != address(0)) {
            if (!m.fresh()) return (0, REF_DEVIATION, false);
            priceWei = m.price();
            return (priceWei, REF_DEVIATION, priceWei != 0);
        }
        (uint256 cap, bool fresh) = factory.freshCap();
        return (cap, maxDeviation, fresh && cap != 0);
    }

    function _deploy(address coin) private {
        if (pendingEth[coin] == 0 && pendingWrapped[coin] == 0) return;
        (uint256 ref, int24 dev, bool ok) = priceReference();
        if (!ok) { emit SharedDeferred(coin, 1); return; }
        uint160 sqrtRef = _sqrtRatio(ref, ONE);
        int24 refTick = TickMath.getTickAtSqrtPrice(sqrtRef);
        PoolKey memory key = sharedKey();
        (uint160 sqrtP, int24 tick,,) = poolManager.getSlot0(key.toId());
        if (sqrtP == 0) {
            tick = poolManager.initialize(key, sqrtRef);
        }
        if (_far(tick, refTick, dev)) {
            // nobody's liquidity is in range: moving the price costs nothing, so move it to the reference's before adding
            if (poolManager.getLiquidity(key.toId()) != 0) { emit SharedDeferred(coin, 2); return; }
            if (!_align(coin, key, sqrtRef, tick < refTick)) { emit SharedDeferred(coin, 3); return; }
            (, tick,,) = poolManager.getSlot0(key.toId());
            if (_far(tick, refTick, dev)) { emit SharedDeferred(coin, 2); return; }
        }
        if (!centered) {
            centered = true;
            center = _clampCenter(_floor(refTick, sharedSpacing));
            (farLower, farUpper) = _farRange(center, sharedSpacing);
            centerMovedAt = uint64(block.timestamp);
            centerMovedAtBlock = uint64(block.number);
            emit CenterSet(center);
        }
        poolManager.unlock(abi.encode(Op.AddShared, abi.encode(coin)));
    }

    function _far(int24 tick, int24 refTick, int24 dev) private pure returns (bool) {
        int256 d = int256(tick) - int256(refTick);
        return d > int256(dev) || d < -int256(dev);
    }

    /// @dev One wei in, price limit at the cap's price: crosses empty ticks for free. Paid from the coin's pending amounts.
    function _align(address coin, PoolKey memory key, uint160 sqrtRef, bool up) private returns (bool) {
        // tick up (wX cheaper) = selling wX (currency1) into the pool; tick down = selling ETH (currency0)
        if (up ? pendingWrapped[coin] == 0 : pendingEth[coin] == 0) return false;
        (uint256 paid0, uint256 paid1) =
            abi.decode(poolManager.unlock(abi.encode(Op.Align, abi.encode(key, sqrtRef, !up))), (uint256, uint256));
        pendingEth[coin] -= paid0;
        pendingWrapped[coin] -= paid1;
        return true;
    }

    /// @notice Anyone: move the center for NEW shared liquidity toward the reference (the module's fresh floor, else the keeper's
    /// fresh cap), by at most `maxShift` ticks, at most once per `moveEvery` on both clocks. Positions already added never move.
    function moveCenter() external nonReentrant {
        if (!centered) revert NoMove();
        (uint256 cap,, bool fresh) = priceReference();
        if (!fresh) revert NoCap();
        if (!Wait.elapsed(uint64(centerMovedAt + moveEvery), uint64(centerMovedAtBlock + Wait.blocksFor(moveEvery, parentBlockSeconds)))) {
            revert TooSoon();
        }
        int24 target = _floor(TickMath.getTickAtSqrtPrice(_sqrtRatio(cap, ONE)), sharedSpacing);
        int256 diff = int256(target) - int256(center);
        if (diff < int256(minMove) && diff > -int256(minMove)) revert NoMove();
        if (diff > int256(maxShift)) diff = int256(maxShift);
        if (diff < -int256(maxShift)) diff = -int256(maxShift);
        int24 from = center;
        int24 to = _clampCenter(_floor(int24(int256(center) + diff), sharedSpacing));
        if (to == from) revert NoMove();
        center = to;
        centerMovedAt = uint64(block.timestamp);
        centerMovedAtBlock = uint64(block.number);
        emit CenterMoved(from, to);
    }

    // ---- fees ------------------------------------------------------------------------------------------------------------

    /// @notice Anyone: collect the fees of every shared position `coin` holds units in, and pay the coin's share of all fees
    /// credited so far to its curve, which deposits them into the coin's sweep vault.
    function claimShared(address coin) external nonReentrant returns (uint256 eth, uint256 wrappedAmount) {
        Coin storage c = coins[coin];
        if (coin != PROTOCOL && c.curve == address(0)) revert NotSeeded();
        bytes32[] storage ids = _coinPositions[coin];
        if (ids.length != 0) poolManager.unlock(abi.encode(Op.Poke, abi.encode(ids)));
        for (uint256 i = 0; i < ids.length; ++i) _settleShare(ids[i], coin);
        eth = owedEth[coin];
        wrappedAmount = owedWrapped[coin];
        if (eth == 0 && wrappedAmount == 0) return (0, 0);
        owedEth[coin] = 0;
        owedWrapped[coin] = 0;
        feesPaidEth += eth;
        feesPaidWrapped += wrappedAmount;
        if (coin == PROTOCOL) {
            // the protocol's positions: wX fees compound as more protocol liquidity; ETH fees go to the treasury, or compound too
            pendingWrapped[PROTOCOL] += wrappedAmount;
            address to = treasury;
            if (to == address(0)) pendingEth[PROTOCOL] += eth;
            emit ProtocolFeesPaid(to == address(0) ? 0 : eth, wrappedAmount);
            if (to != address(0) && eth != 0) {
                (bool ok,) = to.call{value: eth}("");
                if (!ok) revert TransferFailed();
            }
            if (address(factory) != address(0)) _tryDeploy(PROTOCOL);
            return (eth, wrappedAmount);
        }
        if (wrappedAmount != 0) wrapped.forceApprove(c.curve, wrappedAmount);
        IPoolFeeSink(c.curve).depositPoolFees{value: eth}(wrappedAmount);
        if (wrapped.allowance(address(this), c.curve) != 0) revert TransferFailed();
        emit SharedFeesPaid(coin, eth, wrappedAmount);
    }

    /// @notice Anyone: collect the coin's own pool's fees and hand them to its curve, which splits them (docs/v3/pools.md 4b).
    function collectCoinFees(address coin) external nonReentrant returns (uint256 tokenFees, uint256 quoteFees) {
        Coin storage c = coins[coin];
        if (c.curve == address(0) || c.liquidity == 0) revert NotSeeded();
        PoolKey memory key = coinKey(coin);
        int24[2] memory r = _coinRange[coin];
        (uint256 f0, uint256 f1) =
            abi.decode(poolManager.unlock(abi.encode(Op.PokeCoin, abi.encode(key, r[0], r[1]))), (uint256, uint256));
        bool tokenIs0 = Currency.unwrap(key.currency0) == coin;
        (tokenFees, quoteFees) = tokenIs0 ? (f0, f1) : (f1, f0);
        if (tokenFees != 0) {
            IERC20(coin).safeTransfer(c.curve, tokenFees);
            IPoolFeeSink(c.curve).forwardTokenFees();
        }
        if (quoteFees != 0) {
            if (c.quoteIsEth) {
                IPoolFeeSink(c.curve).onPoolFees{value: quoteFees}();
            } else {
                wrapped.safeTransfer(c.curve, quoteFees);
                IPoolFeeSink(c.curve).forwardWrappedFees();
            }
        }
        emit CoinFeesPaid(coin, tokenFees, quoteFees);
    }

    // ---- the PoolManager callback ----------------------------------------------------------------------------------------

    function unlockCallback(bytes calldata data) external returns (bytes memory) {
        if (msg.sender != address(poolManager)) revert NotPoolManager();
        (Op op, bytes memory args) = abi.decode(data, (Op, bytes));
        if (op == Op.SeedCoin) return _cbSeedCoin(args);
        if (op == Op.AddShared) return _cbAddShared(abi.decode(args, (address)));
        if (op == Op.Align) return _cbAlign(args);
        if (op == Op.Poke) return _cbPoke(abi.decode(args, (bytes32[])));
        return _cbPokeCoin(args);
    }

    function _cbSeedCoin(bytes memory args) private returns (bytes memory) {
        (PoolKey memory key, uint160 target, uint256 a0, uint256 a1, int24 lo, int24 hi) =
            abi.decode(args, (PoolKey, uint160, uint256, uint256, int24, int24));
        (uint160 sqrtP,,,) = poolManager.getSlot0(key.toId());
        if (sqrtP != target) {
            // somebody opened this pool first, at another price: swap toward the seed price with the limit AT it, spending at
            // most half of one side. Everything bought on the way is bought below our own valuation.
            bool zeroForOne = sqrtP > target;
            uint256 spend = (zeroForOne ? a0 : a1) / 2;
            if (spend != 0) {
                BalanceDelta d = poolManager.swap(key, SwapParams(zeroForOne, -int256(spend), target), "");
                _settle(key.currency0, d.amount0());
                _settle(key.currency1, d.amount1());
                a0 = uint256(int256(a0) + d.amount0());
                a1 = uint256(int256(a1) + d.amount1());
                (sqrtP,,,) = poolManager.getSlot0(key.toId());
            }
        }
        uint128 liq = LiquidityAmounts.forAmounts(sqrtP, TickMath.getSqrtPriceAtTick(lo), TickMath.getSqrtPriceAtTick(hi),
            _less(a0), _less(a1));
        uint256 used0;
        uint256 used1;
        if (liq != 0) {
            (BalanceDelta d,) =
                poolManager.modifyLiquidity(key, ModifyLiquidityParams(lo, hi, int256(uint256(liq)), bytes32(0)), "");
            _settle(key.currency0, d.amount0());
            _settle(key.currency1, d.amount1());
            used0 = uint256(-int256(d.amount0()));
            used1 = uint256(-int256(d.amount1()));
            if (used0 > a0 || used1 > a1) revert Overspent();
        }
        // the quote side left over; the coin side left over stays here
        address token0 = Currency.unwrap(key.currency0);
        bool quoteIs0 = token0 == address(0) || token0 == address(wrapped);
        return abi.encode(liq, used0, used1, quoteIs0 ? a0 - used0 : a1 - used1);
    }

    function _cbAlign(bytes memory args) private returns (bytes memory) {
        (PoolKey memory key, uint160 limit, bool zeroForOne) = abi.decode(args, (PoolKey, uint160, bool));
        BalanceDelta d = poolManager.swap(key, SwapParams(zeroForOne, -1, limit), "");
        _settle(key.currency0, d.amount0());
        _settle(key.currency1, d.amount1());
        uint256 paid0 = d.amount0() < 0 ? uint256(-int256(d.amount0())) : 0;
        uint256 paid1 = d.amount1() < 0 ? uint256(-int256(d.amount1())) : 0;
        return abi.encode(paid0, paid1);
    }

    function _cbAddShared(address coin) private returns (bytes memory) {
        PoolKey memory key = sharedKey();
        (uint160 sqrtP, int24 tick,,) = poolManager.getSlot0(key.toId());
        uint256 e = pendingEth[coin];
        uint256 w = pendingWrapped[coin];
        int24 c = center;
        (int24 minT, int24 maxT) = (farLower, farUpper);
        // near, wide, far: each takes its weight of what is left and passes on what it could not pair
        uint256 weightLeft = BPS;
        for (uint256 i = 0; i < 3; ++i) {
            (int24 lo, int24 hi, uint256 wgt) = i == 0 ? (c - nearHalf, c + nearHalf, nearBps)
                : i == 1 ? (c - wideHalf, c + wideHalf, wideBps) : (minT, maxT, BPS - nearBps - wideBps);
            if (i != 2 && wgt == 0) continue;
            uint256 ae = i == 2 ? e : Math.mulDiv(e, wgt, weightLeft);
            uint256 aw = i == 2 ? w : Math.mulDiv(w, wgt, weightLeft);
            weightLeft -= wgt;
            (uint256 se, uint256 sw) = _addShared(coin, key, lo, hi, sqrtP, ae, aw);
            e -= se;
            w -= sw;
        }
        // the bid: ETH only, just past the near band on the cheap-wX side, while the price is below it
        int24 bidLo = c + nearHalf;
        if (tick < bidLo && e > MARGIN) {
            (uint256 se,) = _addShared(coin, key, bidLo, bidLo + bidWidth, sqrtP, e, 0);
            e -= se;
        }
        pendingEth[coin] = e;
        pendingWrapped[coin] = w;
        return "";
    }

    /// @dev Add the most liquidity `ae`/`aw` allow to [lo, hi]. The add itself collects the position's fees so far, credited to
    /// the units that existed before it, so a new coin never shares in fees earned before it arrived.
    function _addShared(address coin, PoolKey memory key, int24 lo, int24 hi, uint160 sqrtP, uint256 ae, uint256 aw)
        private returns (uint256 spentE, uint256 spentW)
    {
        uint128 liq = LiquidityAmounts.forAmounts(sqrtP, TickMath.getSqrtPriceAtTick(lo), TickMath.getSqrtPriceAtTick(hi),
            _less(ae), _less(aw));
        if (liq == 0) return (0, 0);
        bytes32 id = keccak256(abi.encode(lo, hi));
        Position storage p = _positions[id];
        if (p.lower == 0 && p.upper == 0) {
            p.lower = lo;
            p.upper = hi;
            positionIds.push(id);
        }
        (BalanceDelta d, BalanceDelta fees) =
            poolManager.modifyLiquidity(key, ModifyLiquidityParams(lo, hi, int256(uint256(liq)), bytes32(0)), "");
        _accrue(p, fees);
        _settle(key.currency0, d.amount0());
        _settle(key.currency1, d.amount1());
        // principal = what the add cost, net of the fees it collected
        spentE = uint256(int256(fees.amount0()) - int256(d.amount0()));
        spentW = uint256(int256(fees.amount1()) - int256(d.amount1()));
        if (spentE > ae || spentW > aw) revert Overspent();
        _settleShare(id, coin);
        if (!_holds[id][coin]) {
            _holds[id][coin] = true;
            _coinPositions[coin].push(id);
        }
        _shares[id][coin].units += liq;
        p.liquidity += liq;
        emit SharedAdded(coin, id, liq, spentE, spentW);
    }

    function _cbPoke(bytes32[] memory ids) private returns (bytes memory) {
        PoolKey memory key = sharedKey();
        for (uint256 i = 0; i < ids.length; ++i) {
            Position storage p = _positions[ids[i]];
            (BalanceDelta d, BalanceDelta fees) =
                poolManager.modifyLiquidity(key, ModifyLiquidityParams(p.lower, p.upper, 0, bytes32(0)), "");
            _accrue(p, fees);
            _settle(key.currency0, d.amount0());
            _settle(key.currency1, d.amount1());
        }
        return "";
    }

    function _cbPokeCoin(bytes memory args) private returns (bytes memory) {
        (PoolKey memory key, int24 lo, int24 hi) = abi.decode(args, (PoolKey, int24, int24));
        (BalanceDelta d,) = poolManager.modifyLiquidity(key, ModifyLiquidityParams(lo, hi, 0, bytes32(0)), "");
        _settle(key.currency0, d.amount0());
        _settle(key.currency1, d.amount1());
        return abi.encode(uint256(int256(d.amount0())), uint256(int256(d.amount1())));
    }

    function _accrue(Position storage p, BalanceDelta fees) private {
        uint256 f0 = uint256(int256(fees.amount0()));
        uint256 f1 = uint256(int256(fees.amount1()));
        if (f0 == 0 && f1 == 0) return;
        // fees accrue only to existing liquidity, so p.liquidity > 0 here
        p.accEthX128 += Math.mulDiv(f0, Q128, p.liquidity);
        p.accWrappedX128 += Math.mulDiv(f1, Q128, p.liquidity);
        feesInEth += f0;
        feesInWrapped += f1;
    }

    function _settleShare(bytes32 id, address coin) private {
        Position storage p = _positions[id];
        Share storage s = _shares[id][coin];
        if (s.units != 0) {
            owedEth[coin] += Math.mulDiv(s.units, p.accEthX128 - s.ckEth, Q128);
            owedWrapped[coin] += Math.mulDiv(s.units, p.accWrappedX128 - s.ckWrapped, Q128);
        }
        s.ckEth = p.accEthX128;
        s.ckWrapped = p.accWrappedX128;
    }

    /// @dev Pay what the lock owes the PoolManager (delta < 0) or take what it is owed (delta > 0).
    function _settle(Currency c, int128 delta) private {
        if (delta < 0) {
            uint256 amount = uint256(-int256(delta));
            if (Currency.unwrap(c) == address(0)) {
                poolManager.settle{value: amount}();
            } else {
                poolManager.sync(c);
                IERC20(Currency.unwrap(c)).safeTransfer(address(poolManager), amount);
                poolManager.settle();
            }
        } else if (delta > 0) {
            poolManager.take(c, address(this), uint256(int256(delta)));
        }
    }

    // ---- math ------------------------------------------------------------------------------------------------------------

    function _less(uint256 a) private pure returns (uint256) { return a > MARGIN ? a - MARGIN : 0; }

    function _floor(int24 tick, int24 spacing) private pure returns (int24) {
        int24 f = (tick / spacing) * spacing;
        if (tick < 0 && tick % spacing != 0) f -= spacing;
        return f;
    }

    /// @dev The far layer around `ref`: ±`FAR_HALF` on the spacing grid, inside the usable ticks. Clamping only brings a bound
    /// nearer the price, which makes filling its tick dearer, never cheaper.
    function _farRange(int24 ref, int24 spacing) private pure returns (int24 lo, int24 hi) {
        int24 minT = TickMath.minUsableTick(spacing);
        int24 maxT = TickMath.maxUsableTick(spacing);
        int256 l = int256(_floor(ref, spacing)) - FAR_HALF;
        int256 h = int256(_floor(ref, spacing)) + FAR_HALF;
        lo = l < int256(minT) ? minT : _floor(int24(l), spacing);
        hi = h > int256(maxT) ? maxT : _floor(int24(h), spacing);
    }

    /// @dev Keep every band inside the usable ticks wherever the center goes.
    function _clampCenter(int24 c) private view returns (int24) {
        int24 lim = _floor(TickMath.maxUsableTick(sharedSpacing) - wideHalf - bidWidth - nearHalf, sharedSpacing);
        if (c > lim) return lim;
        if (c < -lim) return -lim;
        return c;
    }

    /// @dev sqrt(amount1 / amount0) in Q64.96, clamped to the pool's valid range. For the shared pool the price is wX
    /// (currency1) per ETH (currency0), so the cap's price is `_sqrtRatio(cap, 1e18)`.
    function _sqrtRatio(uint256 amount0, uint256 amount1) private pure returns (uint160) {
        uint256 r = Math.sqrt(Math.mulDiv(amount1, 2 ** 192, amount0));
        if (r <= TickMath.MIN_SQRT_PRICE) return TickMath.MIN_SQRT_PRICE + 1;
        if (r >= TickMath.MAX_SQRT_PRICE) return TickMath.MAX_SQRT_PRICE - 1;
        return uint160(r);
    }

    // ---- views -----------------------------------------------------------------------------------------------------------

    /// @notice The collection's shared pool: native ETH / wX, hookless.
    function sharedKey() public view returns (PoolKey memory) {
        return PoolKey(Currency.wrap(address(0)), Currency.wrap(address(wrapped)), sharedFee, sharedSpacing, IHooks(address(0)));
    }

    /// @notice A coin's own pool: COIN / wX (or COIN / ETH for SWEEP_ONLY), hookless, LP fee = the curve's trade fee. Before the
    /// coin is seeded its curve says which (A-06).
    function coinKey(address coin) public view returns (PoolKey memory key) {
        Coin storage c = coins[coin];
        address curve = c.curve != address(0) ? c.curve : factory.routerOf(coin);
        bool quoteIsEth = c.curve != address(0) ? c.quoteIsEth : IPoolFeeSink(curve).sweepOnly();
        address quote = quoteIsEth ? address(0) : address(wrapped);
        uint24 fee = uint24(IPoolFeeSink(curve).feeBps() * 100);
        (address t0, address t1) = coin < quote ? (coin, quote) : (quote, coin);
        key = PoolKey(Currency.wrap(t0), Currency.wrap(t1), fee, coinSpacing, IHooks(address(0)));
    }

    /// @notice A coin's own pool position's ticks, [lower, upper] (zero until seeded).
    function coinRange(address coin) external view returns (int24 lower, int24 upper) {
        return (_coinRange[coin][0], _coinRange[coin][1]);
    }

    function positionCount() external view returns (uint256) { return positionIds.length; }
    function position(bytes32 id) external view returns (Position memory) { return _positions[id]; }
    function shareOf(bytes32 id, address coin) external view returns (Share memory) { return _shares[id][coin]; }
    function coinPositions(address coin) external view returns (bytes32[] memory) { return _coinPositions[coin]; }

    /// @notice What `coin` is owed from fees already collected (a claim collects more first).
    function owed(address coin) external view returns (uint256 eth, uint256 wrappedAmount) {
        eth = owedEth[coin];
        wrappedAmount = owedWrapped[coin];
        bytes32[] storage ids = _coinPositions[coin];
        for (uint256 i = 0; i < ids.length; ++i) {
            Position storage p = _positions[ids[i]];
            Share storage s = _shares[ids[i]][coin];
            eth += Math.mulDiv(s.units, p.accEthX128 - s.ckEth, Q128);
            wrappedAmount += Math.mulDiv(s.units, p.accWrappedX128 - s.ckWrapped, Q128);
        }
    }

    /// @notice The four ranges new shared liquidity goes to now: near, wide, far (±`FAR_HALF` around the first center; zero
    /// before the first add), bid.
    function layers() external view returns (int24[2][4] memory r) {
        r[0] = [center - nearHalf, center + nearHalf];
        r[1] = [center - wideHalf, center + wideHalf];
        r[2] = [farLower, farUpper];
        r[3] = [center + nearHalf, center + nearHalf + bidWidth];
    }
}
