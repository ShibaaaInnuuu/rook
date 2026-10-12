// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IRandomnessBeacon} from "../interfaces/IRandomnessBeacon.sol";
import {IWrappedCollection} from "../interfaces/IWrappedCollection.sol";
import {Wait} from "../lib/Wait.sol";
import {IArbSys, IL2GatewayRouter, IHomeRedeemDeskIntent, AddressAlias} from "./IHomeChain.sol";

/// @title HomeWrappedAdapter
/// @notice The 4663 face of a home-chain collection's wX (docs/v6/home-chain-vault.md §10.3). The bridged wX on 4663 is a plain
/// `StandardArbERC20` twin: it cannot say what it backs and cannot be burned for an item here. This contract mints its own units
/// 1:1 against twin deposited into it, so that a curve, a lock, a router and a sweep vault get the `IWrappedCollection` they were
/// built for, and turns `redeemToBurn` / `redeemToRelist` into a **redeem intent** the home chain fulfils: the unit is burned
/// and its twin escrowed here, the intent goes up through the rollup's own bridge, the home chain's `HomeRedeemDesk`
/// acknowledges it back through the inbox, anyone then dispatches the twin up to the desk, which burns it at `HomeWrapper`.
/// A ticket the home chain never acknowledges is refunded here after `ACK_DEADLINE` on both clocks: the unit comes back.
/// @dev What leaves this contract, and how — the whole of it: twin, by `withdraw` against burned units, or by `dispatch` of an
/// acknowledged ticket, to the desk and nowhere else; nothing else, ever. There is no custodian on this chain: `custodian()` is
/// this contract, as `HomeWrapper.custodian()` is the wrapper, so every custodian-only path of the vaults and the queue is
/// unreachable for a home-chain collection, on purpose. Why only burn and relist can cross the bridge: neither has an outcome for
/// the redeemer (the NFT goes to `DEAD` or to the home vault's listing), so which home-chain unit completes which ticket changes
/// nobody's payout. A redemption to a person stays `HomeWrapper.redeem`, reached by bridging the twin up oneself (§4).
contract HomeWrappedAdapter is ERC20, IWrappedCollection, ReentrancyGuard {
    using SafeERC20 for IERC20;

    /// @notice The `StandardArbERC20` twin of the home chain's `HomeWrapper` on this chain.
    IERC20 public immutable wx;
    /// @notice The home chain's `HomeWrapper`: the L1 token the gateway knows the twin by.
    address public immutable homeWrapper;
    /// @notice The home chain's `HomeRedeemDesk`: the intent's target, and the only destination of dispatched wX.
    address public immutable homeDesk;
    /// @notice Its aliased address (Nitro's L1→L2 aliasing): the only sender of `acknowledge`.
    address public immutable homeDeskAlias;
    IL2GatewayRouter public immutable l2Router;
    IArbSys public immutable arbSys;
    /// @notice This chain's beacon, for the sweep vaults' draws (`SweepVault` reads it from its wrapper).
    IRandomnessBeacon public immutable beacon;
    uint256 public immutable parentBlockSeconds;

    uint256 public constant ONE_ITEM = 1e18;
    /// @notice How long an intent may go unacknowledged before its ticket refunds: three times the bridge's ≈ 7 days.
    uint256 public constant ACK_DEADLINE = 21 days;

    enum Kind { Burn, Relist }
    enum Stage { None, Open, Acknowledged, Dispatched, Refunded }

    struct Ticket {
        Stage stage;
        Kind kind;
        address owner;
        uint64 openedAt;
        uint64 openedAtBlock;
        uint64 acknowledgedAt;
        uint64 dispatchedAt;
    }

    /// @notice Twin held for units and open tickets: `totalSupply + openTickets × ONE_ITEM`. Twin sent here otherwise is `stray`.
    uint256 public escrowed;
    /// @notice Tickets open or acknowledged: burned here, not yet dispatched to the home chain nor refunded.
    uint256 public openTickets;
    Ticket[] private _tickets;

    event Deposited(address indexed from, address indexed to, uint256 amount);
    event Withdrawn(address indexed from, address indexed to, uint256 amount);
    event RedeemIntent(uint256 indexed id, Kind kind, address indexed owner);
    event Acknowledged(uint256 indexed id);
    event Dispatched(uint256 indexed id);
    event Refunded(uint256 indexed id, address indexed owner);

    error BadInput();
    error ZeroAmount();
    error NotArrived();
    error NotHomeDesk();
    error WrongStage();
    error TooEarly();
    error BridgeMismatch();

    constructor(IERC20 wx_, address homeWrapper_, address homeDesk_, IL2GatewayRouter l2Router_, IArbSys arbSys_, IRandomnessBeacon beacon_,
        uint256 parentBlockSeconds_, string memory name_, string memory symbol_) ERC20(name_, symbol_)
    {
        if (address(wx_) == address(0) || address(wx_).code.length == 0 || homeWrapper_ == address(0) || homeDesk_ == address(0)
            || address(l2Router_) == address(0) || address(arbSys_) == address(0) || address(beacon_) == address(0)) revert BadInput();
        Wait.checkBlockSeconds(parentBlockSeconds_);
        wx = wx_;
        homeWrapper = homeWrapper_;
        homeDesk = homeDesk_;
        homeDeskAlias = AddressAlias.l2(homeDesk_);
        l2Router = l2Router_;
        arbSys = arbSys_;
        beacon = beacon_;
        parentBlockSeconds = parentBlockSeconds_;
    }

    /// @notice Custody is a contract on the home chain; here, nobody: this contract, so no custodian-only path is reachable.
    function custodian() external view returns (address) { return address(this); }

    /// @notice Whole items whose claim is in this contract — exact for these books, a lower bound of the home custody.
    function held() external view returns (uint256) { return escrowed / ONE_ITEM; }

    // ---- units against twin, 1:1, both ways --------------------------------------------------------------------------------------

    /// @notice Anyone: `amount` of twin in (measured), `amount` of units to `to`.
    function deposit(uint256 amount, address to) external nonReentrant {
        if (amount == 0) revert ZeroAmount();
        if (to == address(0)) revert BadInput();
        uint256 before = wx.balanceOf(address(this));
        wx.safeTransferFrom(msg.sender, address(this), amount);
        if (wx.balanceOf(address(this)) - before != amount) revert NotArrived();
        escrowed += amount;
        _mint(to, amount);
        emit Deposited(msg.sender, to, amount);
    }

    /// @notice The caller's units burned, the same amount of twin to `to`. The one way twin leaves but `dispatch`.
    function withdraw(uint256 amount, address to) external nonReentrant {
        if (amount == 0) revert ZeroAmount();
        if (to == address(0)) revert BadInput();
        _burn(msg.sender, amount);
        escrowed -= amount;
        wx.safeTransfer(to, amount);
        emit Withdrawn(msg.sender, to, amount);
    }

    // ---- the intent: burn here, fulfil there --------------------------------------------------------------------------------------

    /// @notice Burn `ONE_ITEM` of the caller's units for one random item burned on the home chain (IWrappedCollection).
    function redeemToBurn() external nonReentrant returns (uint256) { return _redeem(Kind.Burn); }

    /// @notice Burn `ONE_ITEM` of the caller's units for one random item relisted by the home vault (IWrappedCollection).
    function redeemToRelist() external nonReentrant returns (uint256) { return _redeem(Kind.Relist); }

    function _redeem(Kind kind) private returns (uint256 id) {
        _burn(msg.sender, ONE_ITEM);
        id = _tickets.length;
        _tickets.push(Ticket(Stage.Open, kind, msg.sender, uint64(block.timestamp), uint64(block.number), 0, 0));
        openTickets++;
        arbSys.sendTxToL1(homeDesk, abi.encodeCall(IHomeRedeemDeskIntent.intent, (id, uint8(kind), msg.sender)));
        emit RedeemIntent(id, kind, msg.sender);
    }

    /// @notice Only the desk's aliased address (a retryable from the home chain): the intent was received there. From now on the
    /// ticket is the home chain's to complete; it cannot be refunded.
    function acknowledge(uint256 id) external {
        if (msg.sender != homeDeskAlias) revert NotHomeDesk();
        Ticket storage t = _ticket(id);
        if (t.stage != Stage.Open) revert WrongStage();
        t.stage = Stage.Acknowledged;
        t.acknowledgedAt = uint64(block.timestamp);
        emit Acknowledged(id);
    }

    /// @notice Anyone, for an acknowledged ticket: its twin goes up to the desk through the standard gateway (the twin is burned
    /// here by the gateway; the L1 token is released to the desk ≈ 7 days later). Fulfilment, on this chain's side.
    function dispatch(uint256 id) external nonReentrant {
        Ticket storage t = _ticket(id);
        if (t.stage != Stage.Acknowledged) revert WrongStage();
        t.stage = Stage.Dispatched;
        t.dispatchedAt = uint64(block.timestamp);
        openTickets--;
        escrowed -= ONE_ITEM;
        uint256 before = wx.balanceOf(address(this));
        l2Router.outboundTransfer(homeWrapper, homeDesk, ONE_ITEM, "");
        if (before - wx.balanceOf(address(this)) != ONE_ITEM) revert BridgeMismatch();
        emit Dispatched(id);
    }

    /// @notice Anyone, once an open ticket has waited `ACK_DEADLINE` on both clocks with no acknowledgement: `ONE_ITEM` minted back
    /// to its owner. The twin never moved; the home chain's intent, if it ever lands, finds a ticket that is no longer open.
    function refund(uint256 id) external {
        Ticket storage t = _ticket(id);
        if (t.stage != Stage.Open) revert WrongStage();
        if (!Wait.elapsed(uint64(t.openedAt + ACK_DEADLINE), uint64(t.openedAtBlock + Wait.blocksFor(ACK_DEADLINE, parentBlockSeconds)))) revert TooEarly();
        t.stage = Stage.Refunded;
        openTickets--;
        _mint(t.owner, ONE_ITEM);
        emit Refunded(id, t.owner);
    }

    // ---- views ---------------------------------------------------------------------------------------------------------------

    /// @notice Whether `id` was refunded (IWrappedCollection): what `SweepVault.reclaim` reads.
    function refunded(uint256 id) external view returns (bool) { return _ticket(id).stage == Stage.Refunded; }
    function ticket(uint256 id) external view returns (Ticket memory) { return _ticket(id); }
    function ticketCount() external view returns (uint256) { return _tickets.length; }
    /// @notice Twin here that no unit and no ticket accounts for: sent by plain transfer. It stays.
    function stray() external view returns (uint256) { return wx.balanceOf(address(this)) - escrowed; }

    function _ticket(uint256 id) private view returns (Ticket storage) {
        if (id >= _tickets.length) revert BadInput();
        return _tickets[id];
    }
}
