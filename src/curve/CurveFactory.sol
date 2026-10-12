// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {FrogCurve} from "./FrogCurve.sol";
import {IItemQueue} from "../interfaces/IItemQueue.sol";
import {SweepVault} from "../SweepVault.sol";
import {ILauncher} from "../interfaces/ILauncher.sol";
import {ICapSource} from "../interfaces/ICapSource.sol";
import {IWrappedCollection} from "../interfaces/IWrappedCollection.sol";
import {ILiquiditySeeder} from "../interfaces/ILiquiditySeeder.sol";
import {Wait} from "../lib/Wait.sol";

/// @title CurveFactory
/// @notice Launches coins on our own curve for ONE collection (one wrapped item: wFROG, wAPE, ...). docs/v3/curve.md,
/// docs/v3/pools.md. The creator picks a name, the coin's sweep policy (RAFFLE or BURN) and whether it buys items at all
/// (SWEEP_ONLY does not); the target is the factory's, fixed in the coin's curve at launch and public:
///
///  - normal: `normalRaise` (5 ETH);
///  - early, while this collection's shared pool has received less than `earlyUntilEth` of ETH side from graduations:
///    `earlyRaise` (8 ETH; the constructor refuses more than `HARD_MAX_EARLY_RAISE`, 10 ETH);
///  - opening, while it has received none at all: at least one item at the keeper's cap plus the queue's buffer, plus the
///    platform's fee cap, grossed up for the shared pool's ETH side — so a lone first coin can buy one item on its own.
///
/// @dev It is the `ILauncher` the two sweep vaults read (`routerOf(coin)` is the coin's curve) and the `ICapSource` the queue and
/// the lock read (the freshest of the two vaults' keeper caps). Everything else is fixed at construction. No setter, no owner.
/// Per collection by construction: another collection has its own factory, queue, lock and shared pool, and starts early.
contract CurveFactory is ILauncher, ICapSource {
    struct Targets {
        uint256 normalRaise;
        uint256 earlyRaise;
        uint256 earlyUntilEth;   // the early target applies while the lock's sharedEthIn is below this
        uint256 maxRaise;        // no target above this (an opening target on a very expensive collection): choose SWEEP_ONLY
        uint16 minFeeBps;        // the creator's fee tier (docs/v5/policies-and-fees.md §3): at least this (100)
        uint16 maxFeeBps;        // ... and at most this (800); 0 for both keeps the template's fee as the only tier
    }

    uint256 public constant BPS = 10_000;
    uint256 public constant HARD_MAX_EARLY_RAISE = 10 ether;

    address public immutable wrapped;
    ILiquiditySeeder public immutable seeder;
    /// @notice The collection's `ItemQueue`, or its `FloorModule` (docs/v5/floor-module.md): the curves cannot tell which.
    IItemQueue public immutable queue;
    address public immutable platform;
    address public immutable deployer;
    uint256 public immutable parentBlockSeconds;
    uint256 public immutable normalRaise;
    uint256 public immutable earlyRaise;
    uint256 public immutable earlyUntilEth;
    uint256 public immutable maxRaise;
    uint256 public immutable minFeeBps;
    uint256 public immutable maxFeeBps;

    SweepVault public raffleVault;
    SweepVault public burnVault;
    /// @notice The YOYO policy's vault (docs/v5/policies-and-fees.md §1), wired once after the other two; zero: no YOYO here.
    SweepVault public yoyoVault;
    FrogCurve.Config private _template;

    mapping(address coin => address) public override(ILauncher, ICapSource) routerOf;
    mapping(address coin => address) public creatorOf;
    mapping(address coin => uint256) public listingOf;
    mapping(address coin => SweepVault.Policy) public policyOf;
    /// @notice The coin's fee tier, bps of every trade, fixed in its curve at launch.
    mapping(address coin => uint256) public feeOf;
    address[] public coins;

    event Wired(address raffleVault, address burnVault);
    event YoyoWired(address yoyoVault);
    event Launched(address indexed coin, address indexed curve, address indexed creator, SweepVault.Policy policy, uint256 raise,
        bool sweepOnly, bool early, bool opening);
    event FeeTier(address indexed coin, uint256 feeBps);

    error NotDeployer();
    error AlreadyWired();
    error NotWired();
    error BadVault();
    error BadInput();
    error NoCap();
    error TooExpensive(uint256 raise, uint256 maxRaise);
    error BadFeeTier(uint256 feeBps, uint256 min, uint256 max);
    error NoYoyo();
    error QueueFull();

    /// @param template the launch terms; `raise` and `sweepOnly` in it are ignored (set per coin).
    constructor(IWrappedCollection wrapped_, ILiquiditySeeder seeder_, IItemQueue queue_, address platform_, FrogCurve.Config memory template,
        Targets memory t, uint256 parentBlockSeconds_)
    {
        if (address(wrapped_) == address(0) || address(seeder_) == address(0) || address(queue_) == address(0) || platform_ == address(0)) {
            revert BadInput();
        }
        if (address(queue_.wrapper()) != address(wrapped_)) revert BadInput();
        if (t.normalRaise == 0 || t.earlyRaise < t.normalRaise || t.earlyRaise > HARD_MAX_EARLY_RAISE || t.maxRaise < t.earlyRaise) {
            revert BadInput();
        }
        if (t.minFeeBps > t.maxFeeBps || t.maxFeeBps > 1_000) revert BadInput();
        if (t.maxFeeBps != 0 && (template.feeBps < t.minFeeBps || template.feeBps > t.maxFeeBps)) revert BadInput();
        Wait.checkBlockSeconds(parentBlockSeconds_);
        wrapped = address(wrapped_);
        seeder = seeder_;
        queue = queue_;
        platform = platform_;
        deployer = msg.sender;
        parentBlockSeconds = parentBlockSeconds_;
        normalRaise = t.normalRaise;
        earlyRaise = t.earlyRaise;
        earlyUntilEth = t.earlyUntilEth;
        maxRaise = t.maxRaise;
        minFeeBps = t.minFeeBps;
        maxFeeBps = t.maxFeeBps;
        _template = template;
    }

    /// @notice Once, by the deployer, after `wire`: the YOYO vault, which must name this factory and be of that policy.
    function wireYoyo(SweepVault yoyo_) external {
        if (msg.sender != deployer) revert NotDeployer();
        if (address(raffleVault) == address(0)) revert NotWired();
        if (address(yoyoVault) != address(0)) revert AlreadyWired();
        if (yoyo_.policy() != SweepVault.Policy.YOYO || address(yoyo_.launcher()) != address(this) || address(yoyo_.wrapper()) != wrapped) {
            revert BadVault();
        }
        yoyoVault = yoyo_;
        emit YoyoWired(address(yoyo_));
    }

    /// @notice Once, by the deployer: the two policy vaults, which must name this factory as their launcher.
    function wire(SweepVault raffle_, SweepVault burn_) external {
        if (msg.sender != deployer) revert NotDeployer();
        if (address(raffleVault) != address(0)) revert AlreadyWired();
        if (raffle_.policy() != SweepVault.Policy.RAFFLE || burn_.policy() != SweepVault.Policy.BURN) revert BadVault();
        if (address(raffle_.launcher()) != address(this) || address(burn_.launcher()) != address(this)) revert BadVault();
        if (address(raffle_.wrapper()) != wrapped || address(burn_.wrapper()) != wrapped) revert BadVault();
        raffleVault = raffle_;
        burnVault = burn_;
        emit Wired(address(raffle_), address(burn_));
    }

    /// @notice Launch a coin on this collection at the template's fee. Its target is `raiseFor(sweepOnly)` now, fixed in its curve.
    function launch(string calldata name, string calldata symbol, SweepVault.Policy policy, bool sweepOnly)
        external returns (address coin, address curve)
    {
        return _launch(name, symbol, policy, sweepOnly, _template.feeBps);
    }

    /// @notice Launch a coin at the creator's fee tier (docs/v5/policies-and-fees.md §3): `feeBps` of every trade, within
    /// `[minFeeBps, maxFeeBps]`, fixed in the curve for good and published by `feeOf` and the `FeeTier` event.
    function launch(string calldata name, string calldata symbol, SweepVault.Policy policy, bool sweepOnly, uint16 feeBps)
        external returns (address coin, address curve)
    {
        if (maxFeeBps == 0 ? feeBps != _template.feeBps : (feeBps < minFeeBps || feeBps > maxFeeBps)) {
            revert BadFeeTier(feeBps, minFeeBps, maxFeeBps);
        }
        return _launch(name, symbol, policy, sweepOnly, feeBps);
    }

    function _launch(string calldata name, string calldata symbol, SweepVault.Policy policy, bool sweepOnly, uint16 feeBps)
        private returns (address coin, address curve)
    {
        if (address(raffleVault) == address(0)) revert NotWired();
        (uint256 raise, bool early, bool opening) = raiseFor(sweepOnly);
        SweepVault vault = policy == SweepVault.Policy.RAFFLE ? raffleVault : policy == SweepVault.Policy.BURN ? burnVault : yoyoVault;
        if (address(vault) == address(0)) revert NoYoyo();
        // A-03: no coin that buys items is launched while the queue could not convert its budget: it would be bound to refund
        if (!sweepOnly && !queue.hasRoomFor(itemBudgetFor(raise))) revert QueueFull();
        FrogCurve.Config memory c = _template;
        c.raise = raise;
        c.sweepOnly = sweepOnly;
        c.feeBps = feeBps;
        FrogCurve fc = new FrogCurve(c, name, symbol, IWrappedCollection(wrapped), vault, seeder, queue, msg.sender, platform,
            parentBlockSeconds);
        coin = address(fc.token());
        curve = address(fc);
        routerOf[coin] = curve;
        creatorOf[coin] = msg.sender;
        listingOf[coin] = vault.listingId();
        policyOf[coin] = policy;
        feeOf[coin] = feeBps;
        coins.push(coin);
        emit Launched(coin, curve, msg.sender, policy, raise, sweepOnly, early, opening);
        emit FeeTier(coin, feeBps);
    }

    /// @notice The target a coin launched now would get, and why.
    function raiseFor(bool sweepOnly) public view returns (uint256 raise, bool early, bool opening) {
        if (sweepOnly) return (normalRaise, false, false);
        uint256 depth = seeder.sharedEthIn();
        early = depth < earlyUntilEth;
        opening = depth == 0;
        raise = early ? earlyRaise : normalRaise;
        if (opening) {
            (uint256 cap, bool fresh) = freshCap();
            if (!fresh || cap == 0) revert NoCap();
            uint256 item = Math.mulDiv(cap, BPS + queue.bufferBps(), BPS, Math.Rounding.Ceil);
            uint256 open = Math.mulDiv(item + _template.platformFeeCap, BPS, BPS - _template.poolEthBps, Math.Rounding.Ceil);
            if (open > raise) raise = open;
        }
        if (raise > maxRaise) revert TooExpensive(raise, maxRaise);
    }

    /// @notice The item budget a coin that buys items gets for `raise` (FrogCurve's constructor: the raise less the platform's
    /// capped fee and the shared pool's ETH side).
    function itemBudgetFor(uint256 raise) public view returns (uint256) {
        uint256 fee = Math.min((raise * _template.platformBps) / BPS, _template.platformFeeCap);
        uint256 pe = (raise * _template.poolEthBps) / BPS;
        return raise - fee - pe;
    }

    /// @notice Whether a coin that buys items can be launched now (A-03): a fresh enough target exists and the queue has room for
    /// its item budget. SWEEP_ONLY launches do not depend on it. The site shows "launches paused" when false.
    function launchesOpen() external view returns (bool) {
        if (address(raffleVault) == address(0)) return false;
        try this.raiseFor(false) returns (uint256 raise, bool, bool) {
            return queue.hasRoomFor(itemBudgetFor(raise));
        } catch {
            return false;
        }
    }

    /// @inheritdoc ICapSource
    function freshCap() public view returns (uint256 cap, bool fresh) {
        if (address(raffleVault) == address(0)) return (0, false);
        bool rf = raffleVault.capFresh();
        bool bf = burnVault.capFresh();
        if (rf && (!bf || raffleVault.capAt() >= burnVault.capAt())) return (raffleVault.cap(), true);
        if (bf) return (burnVault.cap(), true);
        return (0, false);
    }

    function template() external view returns (FrogCurve.Config memory) { return _template; }
    function coinCount() external view returns (uint256) { return coins.length; }
}
