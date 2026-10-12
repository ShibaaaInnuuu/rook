// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IWrappedCollection} from "../interfaces/IWrappedCollection.sol";
import {Wait} from "../lib/Wait.sol";

/// @title MintDesk
/// @notice Public mint on demand (docs/v3/curve.md, "Public mint"). Anyone escrows ETH for one frog; the canister buys a floor
/// frog on Bitcoin with its own float, brings it into custody, mints the wFROG here, and `fulfill` swaps it for the ETH in one
/// step: the canister is paid its cost, the requester collects one wFROG and whatever it did not cost. A request nobody
/// fills within `ttl` (both clocks) is collected back in full.
/// @dev The canister fronts the purchase, so the requester's ETH never leaves before its wFROG exists. What this does NOT
/// protect against: the canister's reported `cost` is its word, bounded by the requester's own maximum (the amount escrowed).
/// A requester who escrows more than the floor may pay up to what they escrowed. The page should quote floor + fees + buffer.
contract MintDesk is ReentrancyGuard {
    enum Status { None, Open, Filled, Collected, Expired }

    struct Request {
        Status status;
        address requester;
        uint64 at;
        uint64 atBlock;
        uint256 amount;       // the most the requester will pay, escrowed
        uint256 cost;         // what the canister charged, once filled
    }

    IWrappedCollection public immutable wrapper;
    /// @notice Where the canister is paid (its EVM address for ETH, which it bridges back to its Bitcoin float).
    address public immutable payee;
    uint256 public immutable ttl;
    /// @notice The parent chain's block time on this chain (lib/Wait.sol): deployment config, fixed here.
    uint256 public immutable parentBlockSeconds;
    uint256 public constant ONE = 1e18;

    Request[] private _requests;
    /// @notice wFROG held here for filled requests not yet collected.
    uint256 public assigned;
    /// @notice ETH held here for open requests and uncollected change.
    uint256 public escrowed;

    event Requested(uint256 indexed id, address indexed requester, uint256 amount);
    event Filled(uint256 indexed id, uint256 cost);
    event Collected(uint256 indexed id, address indexed requester, uint256 wrapped, uint256 eth);
    event StrayReclaimed(uint256 amount);

    error NotCustodian();
    error NotRequester();
    error WrongStatus();
    error BadInput();
    error Expired();
    error NoWrapped();
    error TransferFailed();

    constructor(IWrappedCollection wrapper_, address payee_, uint256 ttl_, uint256 parentBlockSeconds_) {
        Wait.checkBlockSeconds(parentBlockSeconds_);
        parentBlockSeconds = parentBlockSeconds_;
        if (address(wrapper_) == address(0) || payee_ == address(0) || ttl_ < 1 hours) revert BadInput();
        wrapper = wrapper_;
        payee = payee_;
        ttl = ttl_;
    }

    /// @notice Escrow `msg.value` as the most you will pay for one wFROG.
    function request() external payable nonReentrant returns (uint256 id) {
        if (msg.value == 0) revert BadInput();
        id = _requests.length;
        _requests.push(Request(Status.Open, msg.sender, uint64(block.timestamp), uint64(block.number), msg.value, 0));
        escrowed += msg.value;
        emit Requested(id, msg.sender, msg.value);
    }

    function _expired(Request storage r) private view returns (bool) {
        // expired as soon as EITHER clock says so: the canister must not fill a request its requester may already be leaving
        return block.timestamp >= uint256(r.at) + ttl || block.number >= uint256(r.atBlock) + Wait.blocksFor(ttl, parentBlockSeconds);
    }

    /// @notice The custodian, after minting one wFROG to this desk for request `id`: pays itself `cost` and assigns the wFROG.
    function fulfill(uint256 id, uint256 cost) external nonReentrant {
        if (msg.sender != wrapper.custodian()) revert NotCustodian();
        Request storage r = _requests[id];
        if (r.status != Status.Open) revert WrongStatus();
        if (_expired(r)) revert Expired();
        if (cost > r.amount) revert BadInput();
        if (wrapper.balanceOf(address(this)) < assigned + ONE) revert NoWrapped();
        r.status = Status.Filled;
        r.cost = cost;
        assigned += ONE;
        escrowed -= cost;
        emit Filled(id, cost);
        if (cost != 0) {
            (bool ok,) = payee.call{value: cost}("");
            if (!ok) revert TransferFailed();
        }
    }

    /// @notice The requester: a filled request's wFROG and change, or an expired one's whole escrow.
    function collect(uint256 id) external nonReentrant returns (uint256 wrapped, uint256 eth) {
        Request storage r = _requests[id];
        if (msg.sender != r.requester) revert NotRequester();
        if (r.status == Status.Filled) {
            r.status = Status.Collected;
            wrapped = ONE;
            eth = r.amount - r.cost;
            assigned -= ONE;
        } else if (r.status == Status.Open && _expired(r)) {
            r.status = Status.Expired;
            eth = r.amount;
        } else {
            revert WrongStatus();
        }
        escrowed -= eth;
        emit Collected(id, msg.sender, wrapped, eth);
        if (wrapped != 0 && !wrapper.transfer(msg.sender, wrapped)) revert TransferFailed();
        if (eth != 0) {
            (bool ok,) = msg.sender.call{value: eth}("");
            if (!ok) revert TransferFailed();
        }
    }

    /// @notice The custodian: take back wrapped units here that no filled request is owed — one minted for a request that
    /// expired (or was collected) before `fulfill` landed, or anything sent here by mistake. Units owed to filled, uncollected
    /// requests (`assigned`) cannot be taken.
    function reclaimStray() external nonReentrant returns (uint256 amount) {
        address c = wrapper.custodian();
        if (msg.sender != c) revert NotCustodian();
        amount = wrapper.balanceOf(address(this)) - assigned;
        if (amount != 0 && !wrapper.transfer(c, amount)) revert TransferFailed();
        emit StrayReclaimed(amount);
    }

    function requestOf(uint256 id) external view returns (Request memory) { return _requests[id]; }
    function requestCount() external view returns (uint256) { return _requests.length; }
    function isExpired(uint256 id) external view returns (bool) { return _expired(_requests[id]); }
}
