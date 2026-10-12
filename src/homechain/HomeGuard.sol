// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Wait} from "../lib/Wait.sol";
import {IHomeGuard} from "./IHomeChain.sol";

/// @title HomeGuard
/// @notice One per home chain, shared by every HomeVault and HomeWrapper there: docs/v5/custody-hardening.md H2 (custody caps) and
/// H3 (outbound rate limits with a tripwire), in Solidity, with the canister replaced by contracts. docs/v6/home-chain-vault.md §6.
///
/// Caps: per collection {items, value, per-item price} and global {items, value}. A collection is registered at STAGE 0 (3 items,
/// 10 ETH) and raised by the owner — the timelock — by at most ×4 per call on each number and at most once per RAISE_INTERVAL on
/// both clocks. Lowering is instant. Only the contracts registered for a collection may count against it.
///
/// Outflow: a 24-hour sliding window of items out (per collection and global) and wei out (global). Over a limit, the act reverts
/// `Wait(until)`. The tripwire: an act that CONSUMES a whole day's limit within TRIP_SPAN of the window's first act pauses the guard
/// (a bank run, or a bug, or a forged stream — all cost time, not items); only the owner unpauses. The canister's tripwire counted
/// "distinct asks over the limit"; a reverted call leaves no trace on a chain, so the on-chain tripwire fires on consumption speed.
///
/// @dev What this does NOT protect against: the owner. It is meant to be a timelock (docs/v5/launch-gates.md) whose delay is
/// `timelockDelay`; the bank-run check refuses caps that could not leave within that delay less a day.
contract HomeGuard is IHomeGuard {
    struct Cap {
        bool listed;
        uint256 maxItems;
        uint256 maxValueWei;
        uint256 maxItemWei;
        uint256 items;
        uint256 valueWei;
        uint64 raisedAt;
        uint64 raisedAtBlock;
    }
    struct Limits { uint256 perCollectionItems; uint256 globalItems; uint256 globalValueWei; uint64 raisedAt; uint64 raisedAtBlock; }
    struct Act { uint64 at; uint64 atBlock; uint256 items; uint256 valueWei; }
    struct Window { Act[] acts; uint256 head; uint256 items; uint256 valueWei; }

    uint256 public constant STAGE0_ITEMS = 3;
    uint256 public constant STAGE0_VALUE = 10 ether;
    uint256 public constant STAGE0_OUT_PER_COLLECTION = 6;
    uint256 public constant STAGE0_OUT_GLOBAL = 12;
    uint256 public constant STAGE0_OUT_VALUE = 10 ether;
    uint256 public constant MAX_RAISE = 4;
    uint256 public constant RAISE_INTERVAL = 3 days;
    uint256 public constant WINDOW = 24 hours;
    uint256 public constant TRIP_SPAN = 6 hours;

    address public immutable owner;
    uint256 public immutable timelockDelay;
    uint256 public immutable parentBlockSeconds;

    bool public paused;
    Limits public limits;
    Cap public global;
    mapping(address collection => Cap) private _caps;
    mapping(address caller => address collection) public collectionOf;
    mapping(address collection => Window) private _windows;
    Window private _globalWindow;

    event Registered(address indexed collection, address indexed caller);
    event Raised(address indexed collection, uint256 maxItems, uint256 maxValueWei, uint256 maxItemWei);
    event GlobalRaised(uint256 maxItems, uint256 maxValueWei);
    event LimitsSet(uint256 perCollectionItems, uint256 globalItems, uint256 globalValueWei);
    event Admitted(address indexed collection, uint256 items, uint256 valueWei);
    event Released(address indexed collection, uint256 items, uint256 valueWei);
    event Out(address indexed collection, uint256 items, uint256 valueWei);
    event Tripped(address indexed collection, uint256 windowItems, uint256 windowValue);
    event Paused(bool paused);

    error NotOwner();
    error NotRegistered();
    error AlreadyRegistered();
    error BadInput();
    error OverCap(string which, uint256 would, uint256 max);
    error RaiseTooLarge(uint256 would, uint256 max);
    error RaiseTooSoon();
    error BankRun(uint256 maxItems, uint256 perDay, uint256 delay);
    error OutflowWait(string which, uint64 until);
    error IsPaused();

    modifier onlyOwner() { if (msg.sender != owner) revert NotOwner(); _; }

    constructor(address owner_, uint256 timelockDelay_, uint256 parentBlockSeconds_) {
        if (owner_ == address(0) || timelockDelay_ < 1 days) revert BadInput();
        Wait.checkBlockSeconds(parentBlockSeconds_);
        owner = owner_;
        timelockDelay = timelockDelay_;
        parentBlockSeconds = parentBlockSeconds_;
        global = Cap(true, STAGE0_ITEMS * 4, STAGE0_VALUE, 0, 0, 0, 0, 0);
        limits = Limits(STAGE0_OUT_PER_COLLECTION, STAGE0_OUT_GLOBAL, STAGE0_OUT_VALUE, 0, 0);
    }

    // ---- registration and raises: the owner (timelock) --------------------------------------------------------------------------

    /// @notice Lists `collection` at stage 0 (once) and lets `caller` (a vault or a wrapper of it) count against it.
    function register(address collection, address caller) external onlyOwner {
        if (collection == address(0) || caller == address(0)) revert BadInput();
        if (collectionOf[caller] != address(0)) revert AlreadyRegistered();
        Cap storage c = _caps[collection];
        if (!c.listed) {
            c.listed = true;
            c.maxItems = STAGE0_ITEMS;
            c.maxValueWei = STAGE0_VALUE;
            c.maxItemWei = STAGE0_VALUE;
            _checkBankRun(STAGE0_ITEMS, limits.perCollectionItems);
        }
        collectionOf[caller] = collection;
        emit Registered(collection, caller);
    }

    /// @notice At most ×4 on each number, at most once per RAISE_INTERVAL on both clocks; lowering is always fine.
    function raise(address collection, uint256 maxItems, uint256 maxValueWei, uint256 maxItemWei) external onlyOwner {
        Cap storage c = _caps[collection];
        if (!c.listed) revert NotRegistered();
        if (maxItemWei > maxValueWei || maxItems == 0) revert BadInput();
        bool up = maxItems > c.maxItems || maxValueWei > c.maxValueWei || maxItemWei > c.maxItemWei;
        if (up) {
            _checkRaise(maxItems, c.maxItems);
            _checkRaise(maxValueWei, c.maxValueWei);
            _checkRaise(maxItemWei, c.maxItemWei);
            if (c.raisedAt != 0 && !Wait.elapsed(uint64(c.raisedAt + RAISE_INTERVAL),
                uint64(c.raisedAtBlock + Wait.blocksFor(RAISE_INTERVAL, parentBlockSeconds)))) revert RaiseTooSoon();
            c.raisedAt = uint64(block.timestamp);
            c.raisedAtBlock = uint64(block.number);
            _checkBankRun(maxItems, limits.perCollectionItems);
        }
        c.maxItems = maxItems;
        c.maxValueWei = maxValueWei;
        c.maxItemWei = maxItemWei;
        emit Raised(collection, maxItems, maxValueWei, maxItemWei);
    }

    function raiseGlobal(uint256 maxItems, uint256 maxValueWei) external onlyOwner {
        if (maxItems == 0) revert BadInput();
        bool up = maxItems > global.maxItems || maxValueWei > global.maxValueWei;
        if (up) {
            _checkRaise(maxItems, global.maxItems);
            _checkRaise(maxValueWei, global.maxValueWei);
            if (global.raisedAt != 0 && !Wait.elapsed(uint64(global.raisedAt + RAISE_INTERVAL),
                uint64(global.raisedAtBlock + Wait.blocksFor(RAISE_INTERVAL, parentBlockSeconds)))) revert RaiseTooSoon();
            global.raisedAt = uint64(block.timestamp);
            global.raisedAtBlock = uint64(block.number);
        }
        global.maxItems = maxItems;
        global.maxValueWei = maxValueWei;
        emit GlobalRaised(maxItems, maxValueWei);
    }

    /// @notice The outbound limits, under the same ×4 and interval rules.
    function setLimits(uint256 perCollectionItems, uint256 globalItems, uint256 globalValueWei) external onlyOwner {
        if (perCollectionItems == 0 || globalItems < perCollectionItems) revert BadInput();
        Limits storage l = limits;
        bool up = perCollectionItems > l.perCollectionItems || globalItems > l.globalItems || globalValueWei > l.globalValueWei;
        if (up) {
            _checkRaise(perCollectionItems, l.perCollectionItems);
            _checkRaise(globalItems, l.globalItems);
            _checkRaise(globalValueWei, l.globalValueWei);
            if (l.raisedAt != 0 && !Wait.elapsed(uint64(l.raisedAt + RAISE_INTERVAL),
                uint64(l.raisedAtBlock + Wait.blocksFor(RAISE_INTERVAL, parentBlockSeconds)))) revert RaiseTooSoon();
            l.raisedAt = uint64(block.timestamp);
            l.raisedAtBlock = uint64(block.number);
        }
        l.perCollectionItems = perCollectionItems;
        l.globalItems = globalItems;
        l.globalValueWei = globalValueWei;
        emit LimitsSet(perCollectionItems, globalItems, globalValueWei);
    }

    function setPaused(bool p) external onlyOwner {
        paused = p;
        emit Paused(p);
    }

    // ---- what the vaults and wrappers call ---------------------------------------------------------------------------------------

    /// @notice One more purchase or deposit of `items` at `valueWei` fits under every cap. Registered callers only.
    function admit(address collection, uint256 items, uint256 valueWei) external {
        Cap storage c = _registered(collection);
        if (paused) revert IsPaused();
        if (c.items + items > c.maxItems) revert OverCap("collection items", c.items + items, c.maxItems);
        if (c.valueWei + valueWei > c.maxValueWei) revert OverCap("collection value", c.valueWei + valueWei, c.maxValueWei);
        if (items != 0 && valueWei > c.maxItemWei * items) revert OverCap("item price", valueWei, c.maxItemWei * items);
        if (global.items + items > global.maxItems) revert OverCap("global items", global.items + items, global.maxItems);
        if (global.valueWei + valueWei > global.maxValueWei) revert OverCap("global value", global.valueWei + valueWei, global.maxValueWei);
        c.items += items;
        c.valueWei += valueWei;
        global.items += items;
        global.valueWei += valueWei;
        emit Admitted(collection, items, valueWei);
    }

    /// @notice Items and value that left custody (a win, a burn, a sale, a redemption, a wrap). Clamped: never underflows.
    function release(address collection, uint256 items, uint256 valueWei) external {
        Cap storage c = _registered(collection);
        c.items = items > c.items ? 0 : c.items - items;
        c.valueWei = valueWei > c.valueWei ? 0 : c.valueWei - valueWei;
        global.items = items > global.items ? 0 : global.items - items;
        global.valueWei = valueWei > global.valueWei ? 0 : global.valueWei - valueWei;
        emit Released(collection, items, valueWei);
    }

    /// @notice An outbound act: `items` leaving and/or `valueWei` paid out. Reverts `Wait` over a window's limit; trips the pause when
    /// a whole day's limit is consumed within TRIP_SPAN.
    function out(address collection, uint256 items, uint256 valueWei) external {
        _registered(collection);
        if (paused) revert IsPaused();
        Window storage w = _windows[collection];
        _prune(w);
        _prune(_globalWindow);
        Limits memory l = limits;
        if (w.items + items > l.perCollectionItems) revert OutflowWait("collection items", _until(w));
        if (_globalWindow.items + items > l.globalItems) revert OutflowWait("global items", _until(_globalWindow));
        if (_globalWindow.valueWei + valueWei > l.globalValueWei) revert OutflowWait("global value", _until(_globalWindow));
        _push(w, items, valueWei);
        _push(_globalWindow, items, valueWei);
        emit Out(collection, items, valueWei);
        bool hit = (items != 0 && (w.items == l.perCollectionItems || _globalWindow.items == l.globalItems))
            || (valueWei != 0 && _globalWindow.valueWei == l.globalValueWei);
        if (hit && _young(w) && _young(_globalWindow)) {
            paused = true;
            emit Tripped(collection, w.items, _globalWindow.valueWei);
            emit Paused(true);
        }
    }

    // ---- views ----------------------------------------------------------------------------------------------------------------

    function capOf(address collection) external view returns (Cap memory) { return _caps[collection]; }
    function windowOf(address collection) external view returns (uint256 items, uint256 valueWei, uint256 acts) {
        Window storage w = _windows[collection];
        return (w.items, w.valueWei, w.acts.length - w.head);
    }
    function globalWindow() external view returns (uint256 items, uint256 valueWei, uint256 acts) {
        return (_globalWindow.items, _globalWindow.valueWei, _globalWindow.acts.length - _globalWindow.head);
    }

    // ---- internals -------------------------------------------------------------------------------------------------------------

    function _registered(address collection) private view returns (Cap storage c) {
        if (collectionOf[msg.sender] != collection || collection == address(0)) revert NotRegistered();
        c = _caps[collection];
    }

    function _checkRaise(uint256 would, uint256 current) private pure {
        if (would > current * MAX_RAISE) revert RaiseTooLarge(would, current * MAX_RAISE);
    }

    /// @dev H3's bank-run constraint: a collection's whole cap must be able to leave within the timelock's delay less one day.
    function _checkBankRun(uint256 maxItems, uint256 perDay) private view {
        uint256 days_ = (timelockDelay - 1 days) / 1 days;
        if (days_ == 0 || maxItems > perDay * days_) revert BankRun(maxItems, perDay, timelockDelay);
    }

    /// @dev Drops acts older than WINDOW on BOTH clocks: an act leaves the window only once the timestamp AND the parent block say so.
    function _prune(Window storage w) private {
        uint256 blocks = Wait.blocksFor(WINDOW, parentBlockSeconds);
        while (w.head < w.acts.length) {
            Act storage a = w.acts[w.head];
            if (!Wait.elapsed(uint64(a.at + WINDOW), uint64(a.atBlock + blocks))) break;
            w.items -= a.items;
            w.valueWei -= a.valueWei;
            delete w.acts[w.head];
            w.head++;
        }
    }

    function _push(Window storage w, uint256 items, uint256 valueWei) private {
        w.acts.push(Act(uint64(block.timestamp), uint64(block.number), items, valueWei));
        w.items += items;
        w.valueWei += valueWei;
    }

    function _until(Window storage w) private view returns (uint64) {
        return w.head < w.acts.length ? uint64(w.acts[w.head].at + WINDOW) : uint64(block.timestamp);
    }

    /// @dev Whether the window's first act is younger than TRIP_SPAN: the whole limit went in a rush.
    function _young(Window storage w) private view returns (bool) {
        if (w.head >= w.acts.length) return false;
        return block.timestamp < w.acts[w.head].at + TRIP_SPAN;
    }
}
