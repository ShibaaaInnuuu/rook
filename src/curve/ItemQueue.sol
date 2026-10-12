// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IWrappedCollection} from "../interfaces/IWrappedCollection.sol";
import {ICapSource} from "../interfaces/ICapSource.sol";
import {IItemQueue} from "../interfaces/IItemQueue.sol";
import {Wait} from "../lib/Wait.sol";

interface IRefundable {
    function openRefunds() external;
}

/// @title ItemQueue
/// @notice One collection's batched buy queue (docs/v3/pools.md, section 5). Every graduating coin's item budget waits here in
/// graduation order. The canister buys one item whenever the queue holds enough, wraps it here, and `recordItem(cost)` pays it
/// and splits the item's `1e18` units FIFO: the oldest unfinished coin's ETH pays first, then the next, each coin getting units
/// pro rata to the ETH it put into this item. A coin is filled once its remaining ETH is at most its TAIL — the larger of `dust`
/// and `tailBps` of what it deposited — so a coin whose budget is not a whole number of items does not wait for another coin to
/// fund a last item it needs only a sliver of. Its curve then `release`s its units and its tail, which goes to the coin's own
/// shared-pool ETH side. A coin not filled by its deadline `cancel`s: its ETH and units go back to its curve
/// for refunds.
/// @dev Nothing is socialized: a coin's remainder stays its own until it is spent on its own share of a later item, returned, or
/// released as dust to its own curve. A later coin's ETH is never spent while an earlier coin has more than dust left.
/// The canister's side (autobuyer/canister-rs/src/buyqueue.rs) reads `queued`, `itemCap`, `buffer`, `itemsRecorded` and
/// `headDeadline`, buys one item whenever `queued ≥ itemCap + buffer`, mints it here and calls `recordItem`; it drives
/// `refundExpired` once the oldest coin's deadline has passed on both clocks.
/// What this does NOT protect against: the canister's reported `cost` is its word, bounded by the keeper's cap (the fresh one,
/// or one snapshotted within `SNAPSHOT_TTL`, so a cap that drops between a purchase and its record does not strand the unit);
/// a stale keeper stops purchases (not refunds).
contract ItemQueue is IItemQueue, ReentrancyGuard {
    using SafeERC20 for IERC20;

    enum Status { None, Queued, Filled, Released, Cancelled }

    struct Entry {
        Status status;
        uint64 position;       // index in `order`
        uint256 deposited;     // the item budget it brought
        uint256 remaining;     // ETH not yet spent (after Filled: its dust)
        uint256 spent;         // ETH spent on its shares of items
        uint256 units;         // wrapped units assigned to it
        address curve;         // who queued it: where its units, tail or refund go
        uint64 deadlineAt;     // its curve's refund deadline, on both clocks
        uint64 deadlineAtBlock;
    }

    uint256 public constant BPS = 10_000;
    uint256 public constant ONE = 1e18;

    /// @notice How long a snapshotted cap still bounds `recordItem` after it was taken (both clocks).
    uint256 public constant SNAPSHOT_TTL = 2 hours;

    IWrappedCollection public immutable wrapper;
    address public immutable payee;         // the canister's ETH address
    uint256 public immutable bufferBps;     // the queue must hold cap × (1 + bufferBps) before an item is bought
    uint256 public immutable parentBlockSeconds;
    uint256 public immutable dust;
    uint256 public immutable tailBps;       // a coin is filled once it has at most this share of its deposit left (and ≤ dust always)
    address public immutable deployer;
    ICapSource public factory;

    address[] public order;
    mapping(address coin => Entry) private _entries;
    uint256 public head;
    /// @notice ETH of coins still Queued: what items can be bought with. `address(this).balance == queued + heldDust` (the
    /// tails of filled coins waiting for their curves to release them are held, not queued).
    uint256 public queued;
    uint256 public heldDust;
    /// @notice Units assigned to coins and still here. `wrapped.balanceOf(this) >= unitsHeld`; the excess is stray.
    uint256 public unitsHeld;
    uint256 public itemsRecorded;
    uint256 public snapCap;
    uint64 public snapAt;
    uint64 public snapAtBlock;
    // the books: totalIn == totalSpent + queued + heldDust + totalReturned + totalDustReleased
    uint256 public totalIn;
    uint256 public totalSpent;
    uint256 public totalReturned;
    uint256 public totalDustReleased;

    event Wired(address factory);
    event Enqueued(address indexed coin, uint256 indexed position, uint256 amount);
    event ItemRecorded(uint256 indexed item, uint256 cost);
    event Assigned(uint256 indexed item, address indexed coin, uint256 eth, uint256 units);
    event CoinFilled(address indexed coin, uint256 units, uint256 spent, uint256 dust);
    event Released(address indexed coin, uint256 units, uint256 dust);
    event Cancelled(address indexed coin, uint256 eth, uint256 units);
    event CapSnapshot(uint256 cap);
    event StrayReclaimed(uint256 amount);

    error NotDeployer();
    error AlreadyWired();
    error NotWired();
    error NotCurve();
    error NotCustodian();
    error WrongStatus();
    error BadInput();
    error CapStale();
    error OverCap(uint256 cost, uint256 max);
    error NotEnough();
    error NoWrapped();
    error TransferFailed();
    error NotExpired();
    error Empty();

    constructor(IWrappedCollection wrapper_, address payee_, uint256 bufferBps_, uint256 dust_, uint256 tailBps_,
        uint256 parentBlockSeconds_)
    {
        if (address(wrapper_) == address(0) || payee_ == address(0) || bufferBps_ > 2_000 || dust_ > 0.1 ether || tailBps_ > 2_500) {
            revert BadInput();
        }
        Wait.checkBlockSeconds(parentBlockSeconds_);
        parentBlockSeconds = parentBlockSeconds_;
        tailBps = tailBps_;
        wrapper = wrapper_;
        payee = payee_;
        bufferBps = bufferBps_;
        dust = dust_;
        deployer = msg.sender;
    }

    /// @notice Once, by the deployer: the collection's factory (who each coin's curve is, and the keeper's cap).
    function wire(ICapSource factory_) external {
        if (msg.sender != deployer) revert NotDeployer();
        if (address(factory) != address(0)) revert AlreadyWired();
        if (factory_.wrapped() != address(wrapper)) revert BadInput();
        factory = factory_;
        emit Wired(address(factory_));
    }

    function _onlyCurve(address coin) private view {
        if (address(factory) == address(0)) revert NotWired();
        if (coin == address(0) || msg.sender != factory.routerOf(coin)) revert NotCurve();
    }

    /// @notice The coin's curve, at the close of its raise: queue its item budget behind every coin that closed before it, with
    /// the curve's refund deadline.
    function enqueue(address coin, uint64 deadlineAt, uint64 deadlineAtBlock) external payable nonReentrant {
        _onlyCurve(coin);
        Entry storage e = _entries[coin];
        if (e.status != Status.None || msg.value <= dust) revert BadInput();
        if (msg.value <= tailOf(msg.value)) revert BadInput();
        e.status = Status.Queued;
        e.position = uint64(order.length);
        e.deposited = msg.value;
        e.remaining = msg.value;
        e.curve = msg.sender;
        e.deadlineAt = deadlineAt;
        e.deadlineAtBlock = deadlineAtBlock;
        order.push(coin);
        queued += msg.value;
        totalIn += msg.value;
        emit Enqueued(coin, e.position, msg.value);
    }

    /// @notice Anyone (the canister, before it buys): remember the keeper's fresh cap, so that `recordItem` may still pay up to
    /// it for `SNAPSHOT_TTL` even if the keeper lowers the cap between the purchase and its record.
    function snapshotCap() external {
        uint256 cap = itemCap();
        if (cap == 0) revert CapStale();
        snapCap = cap;
        snapAt = uint64(block.timestamp);
        snapAtBlock = uint64(block.number);
        emit CapSnapshot(cap);
    }

    /// @notice The most `recordItem` accepts now: the fresh cap, or a snapshot still inside its TTL on both clocks, whichever is
    /// higher.
    function maxItemCost() public view returns (uint256 max) {
        max = itemCap();
        if (snapCap > max && !Wait.elapsed(uint64(snapAt + SNAPSHOT_TTL),
            uint64(snapAtBlock + Wait.blocksFor(SNAPSHOT_TTL, parentBlockSeconds)))) max = snapCap;
    }

    /// @notice The custodian, having bought one item and wrapped it to this queue: be paid `cost`, split the item FIFO.
    /// @dev Requires `balance ≥ unitsHeld + 1e18`: units released to curves or returned on refunds are no longer here, so the
    /// check counts what is still owed, not `itemsRecorded` (which the canister's assumed interface used).
    function recordItem(uint256 cost) external nonReentrant {
        if (msg.sender != wrapper.custodian()) revert NotCustodian();
        if (cost == 0 || cost > queued) revert NotEnough();
        uint256 max = maxItemCost();
        if (max == 0) revert CapStale();
        if (cost > max) revert OverCap(cost, max);
        if (wrapper.balanceOf(address(this)) < unitsHeld + ONE) revert NoWrapped();

        uint256 item = itemsRecorded++;
        uint256 left = cost;
        uint256 unitsLeft = ONE;
        uint256 h = head;
        while (left != 0) {
            address coin = order[h];
            Entry storage e = _entries[coin];
            if (e.status != Status.Queued) { ++h; continue; }
            uint256 take = Math.min(e.remaining, left);
            left -= take;
            uint256 u = left == 0 ? unitsLeft : Math.mulDiv(ONE, take, cost);
            unitsLeft -= u;
            e.remaining -= take;
            e.spent += take;
            e.units += u;
            queued -= take;
            emit Assigned(item, coin, take, u);
            if (e.remaining <= tailOf(e.deposited)) {
                e.status = Status.Filled;
                queued -= e.remaining;
                heldDust += e.remaining;
                ++h;
                emit CoinFilled(coin, e.units, e.spent, e.remaining);
            }
        }
        while (h < order.length && _entries[order[h]].status != Status.Queued) ++h;
        head = h;
        unitsHeld += ONE;
        totalSpent += cost;
        emit ItemRecorded(item, cost);
        (bool ok,) = payee.call{value: cost}("");
        if (!ok) revert TransferFailed();
    }

    /// @notice The coin's curve, once filled: take its units and its dust.
    function release(address coin) external nonReentrant returns (uint256 units, uint256 spent, uint256 dustOut) {
        _onlyCurve(coin);
        Entry storage e = _entries[coin];
        if (e.status != Status.Filled) revert WrongStatus();
        e.status = Status.Released;
        units = e.units;
        spent = e.spent;
        dustOut = e.remaining;
        e.remaining = 0;
        unitsHeld -= units;
        heldDust -= dustOut;
        totalDustReleased += dustOut;
        emit Released(coin, units, dustOut);
        if (units != 0) IERC20(address(wrapper)).safeTransfer(msg.sender, units);
        if (dustOut != 0) {
            (bool ok,) = msg.sender.call{value: dustOut}("");
            if (!ok) revert TransferFailed();
        }
    }

    /// @notice The coin's curve, when its window passed unfilled: take back its unspent ETH and the units already assigned.
    function cancel(address coin) external nonReentrant returns (uint256 eth, uint256 units) {
        _onlyCurve(coin);
        Entry storage e = _entries[coin];
        if (e.status != Status.Queued) revert WrongStatus();
        e.status = Status.Cancelled;
        eth = e.remaining;
        units = e.units;
        e.remaining = 0;
        queued -= eth;
        unitsHeld -= units;
        totalReturned += eth;
        uint256 h = head;
        while (h < order.length && _entries[order[h]].status != Status.Queued) ++h;
        head = h;
        emit Cancelled(coin, eth, units);
        if (units != 0) IERC20(address(wrapper)).safeTransfer(msg.sender, units);
        if (eth != 0) {
            (bool ok,) = msg.sender.call{value: eth}("");
            if (!ok) revert TransferFailed();
        }
    }

    /// @notice Anyone, once the oldest queued coin's deadline has passed on both clocks: open its curve's refunds (which cancels
    /// it here). The canister drives this; any holder may.
    function refundExpired() external {
        uint256 h = head;
        if (h >= order.length) revert Empty();
        Entry storage e = _entries[order[h]];
        if (!Wait.elapsed(e.deadlineAt, e.deadlineAtBlock)) revert NotExpired();
        IRefundable(e.curve).openRefunds();
    }

    /// @notice The custodian: take back units here that nobody is owed — a unit minted for a purchase that could no longer be
    /// recorded (its coin refunded first, or the queue ran short), or anything sent here by mistake.
    /// @inheritdoc IItemQueue
    /// @dev A queue collection has no standing reserve (docs/v5/floor-module.md §13.2): the slice is protocol liquidity.
    function fundReserve() external payable nonReentrant { _toLiquidity(); }

    /// @inheritdoc IItemQueue
    function fundLiquidity() external payable nonReentrant { _toLiquidity(); }

    function _toLiquidity() private {
        if (address(factory) == address(0)) revert NotWired();
        if (msg.value != 0) factory.seeder().addProtocolLiquidity{value: msg.value}();
    }

    function reclaimStray() external nonReentrant returns (uint256 amount) {
        address c = wrapper.custodian();
        if (msg.sender != c) revert NotCustodian();
        amount = wrapper.balanceOf(address(this)) - unitsHeld;
        if (amount != 0) IERC20(address(wrapper)).safeTransfer(c, amount);
        emit StrayReclaimed(amount);
    }

    // ---- views -----------------------------------------------------------------------------------------------------------

    /// @notice The most one item may cost now, wei: the keeper's fresh cap; 0 when none is fresh.
    function itemCap() public view returns (uint256) {
        if (address(factory) == address(0)) return 0;
        (uint256 cap, bool fresh) = factory.freshCap();
        return fresh ? cap : 0;
    }

    /// @notice Held beyond the cap before an item is bought, wei: `bufferBps` of the cap.
    function buffer() public view returns (uint256) { return (itemCap() * bufferBps) / BPS; }

    /// @notice The oldest queued coin's refund deadline; (0, 0) when nothing is queued.
    function headDeadline() external view returns (uint64 at, uint64 atBlock) {
        uint256 h = head;
        if (h >= order.length) return (0, 0);
        Entry storage e = _entries[order[h]];
        return (e.deadlineAt, e.deadlineAtBlock);
    }

    function entry(address coin) external view returns (Entry memory) { return _entries[coin]; }
    /// @notice The most a coin that deposited `deposited` may have left and still count as filled.
    function tailOf(uint256 deposited) public view returns (uint256) { return Math.max(dust, (deposited * tailBps) / BPS); }
    function isFilled(address coin) external view returns (bool) { return _entries[coin].status == Status.Filled; }
    /// @inheritdoc IItemQueue
    /// @dev Always: the queue has no cap, it buys over time (FIFO) and refunds what it could not buy by the deadline.
    function hasRoomFor(uint256) external pure returns (bool) { return true; }
    /// @inheritdoc IItemQueue
    /// @dev The canister's queue has no keeper pause of its own: never held.
    function holdsRefunds() external pure returns (bool) { return false; }
    function length() external view returns (uint256) { return order.length; }
    /// @notice Whether the canister should buy now: the queue holds one item at the keeper's cap plus the buffer.
    function canBuy() external view returns (bool) {
        uint256 cap = itemCap();
        return cap != 0 && queued >= cap + buffer();
    }
}
