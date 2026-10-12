// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {CollectionDirectory} from "../curve/CollectionDirectory.sol";
import {CurveFactory} from "../curve/CurveFactory.sol";
import {Wait} from "../lib/Wait.sol";

/// @dev What the desk reads of a coin's curve (`FrogCurve`): its state (0 Trading, 1 Graduating, 2 Graduated, 3 Refunding) and
/// whether it is floor-buys-only.
interface IWakeCurve {
    function state() external view returns (uint8);
    function sweepOnly() external view returns (bool);
}

/// @dev What the desk reads of a collection's floor module: when its floor was last posted, and whether it is fresh now (the
/// collection is awake: there is nothing to ask for).
interface IWakePosted {
    function postedAt() external view returns (uint64);
    function fresh() external view returns (bool);
}

/// @title WakeDesk
/// @notice docs/v7/dormant-collections.md §3.3. Anyone can ask the canister to wake a dormant collection, which starts a paid scan.
/// The request carries a bond. The canister reads the requests here itself (`requestCount`, `requestAt`), and its own caps decide
/// when a wake runs.
///
/// The bond goes back to the requester once a coin they launched on that collection's factory reached its raise (`reclaim`): a
/// coin created by the requester at an index at or past the factory's `coinCount()` when they asked, not floor-buys-only, whose
/// curve is Graduating or Graduated, and which has not paid back a bond before (dormant-review DR-H-1, DR-L-5: a bare launch costs
/// gas alone and would make every wake free). With no such coin by the window's end plus `RECLAIM_GRACE`, anyone may close the
/// request (`forfeit`): the bond pays the protocol treasury for the cycles the wake burned if the wake was served INSIDE the window
/// (the collection's module was posted at or after the request and no later than its window's end), and goes back to the requester if
/// it was not (dormant-review DR-H-3: a request the canister never served, queued behind its caps or timed out, costs its requester
/// nothing; independent review DI-M-1: nor does one served only after its window, when no reclaim was possible any more).
/// Whether a post landed in the window is recorded while it can still be seen (`markServed`, anyone: the canister sends it after the
/// post that serves the request), since the module keeps only its latest post; unmarked, a request counts as served only when the
/// module's latest post itself falls inside the window.
/// A collection has at most one open request at a time: a second requester can see the wake already asked for, and launch when it
/// is warm. A request is refused while the collection's module is fresh (DI-M-2): the collection is awake and there is nothing to
/// ask for, so no request can hold the collection's one open slot while asking the canister for nothing.
/// A requester whose address does not take the refund or the reclaim (a contract wallet needing more gas than the stipend, or one
/// refusing ETH) is credited instead (DI-L-2), and withdraws with `withdraw`; its bond never goes to the treasury for that.
/// @dev No owner, no pause, no setter, no upgrade: it only holds bonds. Which collections exist is the `CollectionDirectory`'s
/// (admin-listed, append-only), and whether a request is served is the canister's (its install argument names this desk, so a
/// different desk is a timelocked upgrade). Windows run on both clocks (`Wait`). What this does NOT protect against: a requester
/// whose coin really graduates only to get the bond back. That graduation is what the wake was for. Nor a requester asking for
/// wakes the canister cannot serve (a collection too thin to confirm): those bonds come back, and the canister's own caps
/// (`wakes_per_day`, the cooldown) bound what such wakes cost.
contract WakeDesk is ReentrancyGuard {
    struct Request {
        address wrapper;
        address requester;
        uint64 at;
        uint64 atBlock;
        uint64 coinsAt;     // the factory's coinCount() at the request: a coin at this index or later is a launch after it
        uint128 bond;
        bool closed;
        uint64 servedAt;    // DI-M-1: the module's post inside the window that served it (`markServed`), 0: none recorded
    }

    /// @notice The bond is within these bounds at deploy (0.0005 ETH to 1 ETH).
    uint256 public constant MIN_BOND = 0.0005 ether;
    uint256 public constant MAX_BOND = 1 ether;
    /// @notice The window is within these bounds at deploy.
    uint256 public constant MIN_WINDOW = 1 days;
    uint256 public constant MAX_WINDOW = 30 days;
    /// @notice A reclaim may still land this long after the window, before anyone may forfeit the bond.
    uint256 public constant RECLAIM_GRACE = 1 days;

    CollectionDirectory public immutable directory;
    address payable public immutable treasury;
    uint256 public immutable bond;
    uint256 public immutable window;
    uint256 public immutable parentBlockSeconds;

    Request[] private _requests;
    /// @notice 1-based id of the last request for a wrapper (0: none).
    mapping(address wrapper => uint256) public lastOf;
    /// @notice Whether a coin already paid back a bond (one launch, one bond).
    mapping(address coin => bool) public spent;
    /// @notice DI-L-2: what a requester is owed after a refund or a reclaim its address did not take (`withdraw`).
    mapping(address requester => uint256) public owed;

    event Requested(uint256 indexed id, address indexed wrapper, address indexed requester, uint256 bond, uint256 coinsAt);
    event Reclaimed(uint256 indexed id, address indexed coin, address indexed requester, uint256 bond);
    event Forfeited(uint256 indexed id, uint256 bond);
    event Refunded(uint256 indexed id, address indexed requester, uint256 bond);
    event Served(uint256 indexed id, uint64 postedAt);
    event Owed(uint256 indexed id, address indexed requester, uint256 bond);
    event Withdrawn(address indexed requester, address indexed to, uint256 amount);

    error BadInput();
    error WrongBond(uint256 sent, uint256 bond);
    error NotListed();
    error AlreadyRequested(uint256 id);
    error Closed();
    error NotALaunch();
    error NotGraduated();
    error AlreadySpent();
    error TooLate();
    error TooEarly();
    error TransferFailed();
    error Awake();
    error NotServed();
    error NothingOwed();

    constructor(CollectionDirectory directory_, address payable treasury_, uint256 bond_, uint256 window_, uint256 parentBlockSeconds_) {
        if (address(directory_) == address(0) || treasury_ == address(0)) revert BadInput();
        if (bond_ < MIN_BOND || bond_ > MAX_BOND || window_ < MIN_WINDOW || window_ > MAX_WINDOW) revert BadInput();
        Wait.checkBlockSeconds(parentBlockSeconds_);
        directory = directory_;
        treasury = treasury_;
        bond = bond_;
        window = window_;
        parentBlockSeconds = parentBlockSeconds_;
    }

    /// @notice Ask for `wrapper`'s collection to be woken, with exactly `bond` wei. Refused while another request for it is open, and
    /// while its module's floor is fresh (DI-M-2: the collection is awake; a module that cannot say counts as not fresh).
    function request(address wrapper) external payable nonReentrant returns (uint256 id) {
        if (msg.value != bond) revert WrongBond(msg.value, bond);
        CurveFactory f = _factoryOf(wrapper);
        uint256 last = lastOf[wrapper];
        if (last != 0 && _open(last - 1)) revert AlreadyRequested(last - 1);
        if (_fresh(wrapper)) revert Awake();
        id = _requests.length;
        uint256 coins = f.coinCount();
        _requests.push(Request(wrapper, msg.sender, uint64(block.timestamp), uint64(block.number), uint64(coins), uint128(msg.value), false, 0));
        lastOf[wrapper] = id + 1;
        emit Requested(id, wrapper, msg.sender, msg.value, coins);
    }

    /// @notice Anyone: request `id`'s requester launched a coin on the collection since asking (`coinIndex` names it), it is not
    /// floor-buys-only, its curve reached its raise, and it paid back no bond before: the bond goes back to the requester. Allowed
    /// until the window's end plus `RECLAIM_GRACE`, on both clocks.
    function reclaim(uint256 id, uint256 coinIndex) external nonReentrant {
        Request storage r = _requests[id];
        if (r.closed) revert Closed();
        if (_elapsed(r, window + RECLAIM_GRACE)) revert TooLate();
        CurveFactory f = _factoryOf(r.wrapper);
        if (coinIndex < r.coinsAt || coinIndex >= f.coinCount()) revert NotALaunch();
        address coin = f.coins(coinIndex);
        if (f.creatorOf(coin) != r.requester) revert NotALaunch();
        if (spent[coin]) revert AlreadySpent();
        IWakeCurve curve = IWakeCurve(f.routerOf(coin));
        if (curve.sweepOnly()) revert NotALaunch();
        uint8 st = curve.state();
        if (st != 1 && st != 2) revert NotGraduated();
        spent[coin] = true;
        r.closed = true;
        uint256 b = r.bond;
        address to = r.requester;
        emit Reclaimed(id, coin, to, b);
        (bool ok,) = payable(to).call{value: b}("");
        if (!ok) _owe(id, to, b);
    }

    /// @notice Anyone, once the window and `RECLAIM_GRACE` are over on both clocks, closes an unreclaimed request: the bond goes to
    /// the treasury if the wake was served inside the window (`served`), else back to the requester (credited to it, for `withdraw`,
    /// if its address does not take it within the stipend). A module that cannot say when it was posted counts as served.
    function forfeit(uint256 id) external nonReentrant {
        Request storage r = _requests[id];
        if (r.closed) revert Closed();
        if (!_elapsed(r, window + RECLAIM_GRACE)) revert TooEarly();
        r.closed = true;
        uint256 b = r.bond;
        if (!_served(r)) {
            // a bounded stipend; an address that needs more gas, or refuses ETH, is credited instead (DI-L-2): the bond is still its own
            (bool back,) = payable(r.requester).call{value: b, gas: 50_000}("");
            if (back) emit Refunded(id, r.requester, b);
            else _owe(id, r.requester, b);
            return;
        }
        emit Forfeited(id, b);
        (bool ok,) = treasury.call{value: b}("");
        if (!ok) revert TransferFailed();
    }

    /// @notice Anyone (the canister sends it after the post that served the request): records that request `id`'s collection was
    /// posted inside its window, at or after the request. Taken while the module's latest post shows it: the latest post itself
    /// inside `[at, at + window]`, on the timestamp alone, the same bound as the unrecorded `served` (independent review 2, D2-L-1: a
    /// post after `at + window` is never inside the window, even while the block clock's half of it is still running). Once
    /// recorded, a later post can neither add nor take it away.
    function markServed(uint256 id) external nonReentrant {
        Request storage r = _requests[id];
        if (r.closed) revert Closed();
        if (r.servedAt != 0) return;
        (bool known, uint64 p) = _postedAt(r.wrapper);
        if (!known || p < r.at || p > r.at + window) revert NotServed();
        r.servedAt = p;
        emit Served(id, p);
    }

    /// @notice DI-L-2: the caller's credited refunds and reclaims, sent to `to`.
    function withdraw(address payable to) external nonReentrant {
        uint256 a = owed[msg.sender];
        if (a == 0) revert NothingOwed();
        owed[msg.sender] = 0;
        emit Withdrawn(msg.sender, to, a);
        (bool ok,) = to.call{value: a}("");
        if (!ok) revert TransferFailed();
    }

    /// @notice Whether request `id`'s wake was served inside its window: recorded by `markServed`, or the module's latest post falls
    /// in `[at, at + window]` (a module that cannot say counts as served).
    function served(uint256 id) external view returns (bool) {
        return _served(_requests[id]);
    }

    /// @notice The post recorded as serving request `id` (0: none recorded).
    function servedAt(uint256 id) external view returns (uint64) {
        return _requests[id].servedAt;
    }

    function _served(Request storage r) private view returns (bool) {
        if (r.servedAt != 0) return true;
        (bool known, uint64 p) = _postedAt(r.wrapper);
        if (!known) return true;
        return p >= r.at && p <= r.at + window;
    }

    function _postedAt(address wrapper) private view returns (bool, uint64) {
        try IWakePosted(_moduleOf(wrapper)).postedAt() returns (uint64 p) {
            return (true, p);
        } catch {
            return (false, 0);
        }
    }

    function _fresh(address wrapper) private view returns (bool) {
        try IWakePosted(_moduleOf(wrapper)).fresh() returns (bool f) {
            return f;
        } catch {
            return false;
        }
    }

    function _owe(uint256 id, address to, uint256 b) private {
        owed[to] += b;
        emit Owed(id, to, b);
    }

    /// @notice How many requests were ever made (the canister's cursor runs over `[0, requestCount())`).
    function requestCount() external view returns (uint256) {
        return _requests.length;
    }

    /// @notice One request, as the canister reads it.
    function requestAt(uint256 id) external view returns (address wrapper, address requester, uint64 at, uint128 bond_, bool closed) {
        Request storage r = _requests[id];
        return (r.wrapper, r.requester, r.at, r.bond, r.closed);
    }

    /// @notice The whole record, for the site.
    function requestOf(uint256 id) external view returns (Request memory) {
        return _requests[id];
    }

    /// @notice Whether request `id` is open: not closed and inside its window on either clock.
    function isOpen(uint256 id) external view returns (bool) {
        return id < _requests.length && _open(id);
    }

    /// @notice The open request for `wrapper`, as `(true, id)`, or `(false, 0)`.
    function openOf(address wrapper) external view returns (bool open, uint256 id) {
        uint256 last = lastOf[wrapper];
        if (last != 0 && _open(last - 1)) return (true, last - 1);
        return (false, 0);
    }

    function _open(uint256 id) private view returns (bool) {
        Request storage r = _requests[id];
        return !r.closed && !_elapsed(r, window);
    }

    function _elapsed(Request storage r, uint256 period) private view returns (bool) {
        return Wait.elapsed(uint64(r.at + period), uint64(r.atBlock + Wait.blocksFor(period, parentBlockSeconds)));
    }

    function _moduleOf(address wrapper) private view returns (address) {
        return address(directory.at(directory.indexOf(wrapper) - 1).queue);
    }

    function _factoryOf(address wrapper) private view returns (CurveFactory) {
        uint256 i = directory.indexOf(wrapper);
        if (i == 0) revert NotListed();
        return directory.at(i - 1).factory;
    }
}
