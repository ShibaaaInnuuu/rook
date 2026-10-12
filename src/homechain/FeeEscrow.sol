// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {Wait} from "../lib/Wait.sol";
import {INftLauncher, INftVault} from "../nft/INft.sol";
import {IArbSys, IFeeEscrowConfirm, AddressAlias} from "./IHomeChain.sol";

/// @notice What the home chain's `fund(ticket)` looks like from here.
interface IHomeVaultFund {
    function fund(uint256 ticket) external payable;
}

/// @title FeeEscrow
/// @notice The 4663 side of a home-chain vault (docs/v6/home-chain-vault.md §5): a coin's routers pay the sweep share and
/// graduation budgets in, earmarked per coin; when a coin has enough, ANYONE releases it as a numbered ticket through the rollup's
/// own bridge (`ArbSys.sendTxToL1` → the Outbox on Ethereum calls `HomeVault.fund(ticket)` ≈ 7 days later) and is paid a fixed
/// reward. The keeper may instead release through an allow-listed fast bridge, bounded: every fast ticket must be CONFIRMED by the
/// home vault itself within FAST_DEADLINE, or fast releases stop. The confirmation is a retryable from the home vault, so its
/// sender here is the vault's aliased address — nothing else can produce it.
/// @dev There is no other way out. No withdraw, no owner path; the owner only rotates the fast-bridge allow-list with a delay.
contract FeeEscrow is INftVault, IFeeEscrowConfirm, ReentrancyGuard {
    enum Mode { Native, Fast }
    enum Status { None, Sent, Confirmed }

    struct Ticket {
        address coin;
        uint256 amount;         // what left this contract (the reward included)
        address l2Recipient;    // where wX bridged back for this ticket goes (the coin's curve or queue)
        Mode mode;
        Status status;
        bool overdue;
        uint64 at;
        uint64 atBlock;
        uint256 amountSeen;     // what the home vault said it received
    }

    IArbSys public immutable arbSys;
    address public immutable homeVault;
    address public immutable homeVaultAlias;
    INftLauncher public immutable launcher;
    uint256 public immutable listingId;
    address public immutable owner;
    address public immutable keeper;
    uint256 public immutable reward;
    uint256 public immutable minRelease;
    uint256 public immutable parentBlockSeconds;

    uint256 public constant FAST_DEADLINE = 3 days;
    uint256 public constant CHANGE_DELAY = 7 days;

    mapping(address coin => uint256) public unspentOf;
    mapping(address coin => address) public recipientOf;
    mapping(address coin => uint256) public contributedBy;
    uint256 public accountedTotal;
    Ticket[] private _tickets;
    uint256 public fastOpen;            // fast tickets sent and not yet confirmed
    uint256 public fastOverdue;         // ... of which flagged past the deadline
    mapping(address bridge => bool) public isBridge;
    mapping(address bridge => uint64) public bridgeReadyAt;
    mapping(address bridge => uint64) public bridgeReadyAtBlock;

    event Deposited(address indexed coin, uint256 amount, address l2Recipient);
    event Released(uint256 indexed ticket, address indexed coin, uint256 amount, Mode mode, address indexed by);
    event Confirmed(uint256 indexed ticket, uint256 amountSeen);
    event Overdue(uint256 indexed ticket);
    event BridgeProposed(address bridge, uint64 readyAt);
    event BridgeAccepted(address bridge);
    event BridgeRemoved(address bridge);

    error BadInput();
    error NotRouter();
    error NotKeeper();
    error NotOwner();
    error NotHomeVault();
    error ZeroAmount();
    error TooSmall(uint256 amount, uint256 min);
    error BridgeNotAllowed();
    error FastBlocked();
    error WrongStage();
    error TooEarly();
    error BridgeCallFailed(bytes reason);
    error TransferFailed();

    constructor(IArbSys arbSys_, address homeVault_, INftLauncher launcher_, uint256 listingId_, address owner_, address keeper_,
        uint256 reward_, uint256 minRelease_, uint256 parentBlockSeconds_)
    {
        if (address(arbSys_) == address(0) || homeVault_ == address(0) || address(launcher_) == address(0) || owner_ == address(0)
            || keeper_ == address(0) || reward_ == 0 || minRelease_ <= reward_) revert BadInput();
        Wait.checkBlockSeconds(parentBlockSeconds_);
        arbSys = arbSys_;
        homeVault = homeVault_;
        homeVaultAlias = AddressAlias.l2(homeVault_);
        launcher = launcher_;
        listingId = listingId_;
        owner = owner_;
        keeper = keeper_;
        reward = reward_;
        minRelease = minRelease_;
        parentBlockSeconds = parentBlockSeconds_;
    }

    // ---- money in: a coin's own router, for a coin of this listing -------------------------------------------------------------

    /// @notice The sweep share, from the coin's router. wX that comes back for it goes to the router.
    function deposit(address coin) external payable {
        _deposit(coin, msg.sender);
    }

    /// @notice A graduation budget, from the coin's router, naming where wX bought with it should land on 4663.
    function depositFor(address coin, address l2Recipient) external payable {
        if (l2Recipient == address(0)) revert BadInput();
        _deposit(coin, l2Recipient);
    }

    function _deposit(address coin, address l2Recipient) private {
        if (launcher.routerOf(coin) != msg.sender || launcher.listingOf(coin) != listingId) revert NotRouter();
        if (msg.value == 0) revert ZeroAmount();
        unspentOf[coin] += msg.value;
        contributedBy[coin] += msg.value;
        accountedTotal += msg.value;
        recipientOf[coin] = l2Recipient;
        emit Deposited(coin, msg.value, l2Recipient);
    }

    // ---- release: anyone, through the rollup's bridge ----------------------------------------------------------------------------

    /// @notice Sends everything `coin` has earmarked (at least `minRelease`) to the home vault as ticket `id`, less the caller's
    /// reward. Lands there after the rollup's confirmation period, by anyone's `Outbox.executeTransaction`.
    function release(address coin) external nonReentrant returns (uint256 id) {
        uint256 amount = unspentOf[coin];
        if (amount < minRelease) revert TooSmall(amount, minRelease);
        id = _open(coin, amount, Mode.Native);
        arbSys.sendTxToL1{value: amount - reward}(homeVault, abi.encodeCall(IHomeVaultFund.fund, (id)));
        (bool ok,) = msg.sender.call{value: reward}("");
        if (!ok) revert TransferFailed();
        emit Released(id, coin, amount, Mode.Native, msg.sender);
    }

    /// @notice The keeper, through an allow-listed fast bridge, whose quote it chose off chain. No reward (it is the keeper's job),
    /// and refused while any fast ticket is overdue. The home vault must confirm it (§5.2) or fast releases stop.
    function releaseFast(address coin, address bridge, bytes calldata data) external nonReentrant returns (uint256 id) {
        if (msg.sender != keeper) revert NotKeeper();
        if (!isBridge[bridge]) revert BridgeNotAllowed();
        if (fastOverdue != 0) revert FastBlocked();
        uint256 amount = unspentOf[coin];
        if (amount < minRelease) revert TooSmall(amount, minRelease);
        id = _open(coin, amount, Mode.Fast);
        fastOpen++;
        (bool ok, bytes memory ret) = bridge.call{value: amount}(data);
        if (!ok) revert BridgeCallFailed(ret);
        emit Released(id, coin, amount, Mode.Fast, msg.sender);
    }

    function _open(address coin, uint256 amount, Mode mode) private returns (uint256 id) {
        unspentOf[coin] = 0;
        accountedTotal -= amount;
        id = _tickets.length + 1;      // ticket 0 is "unattributed" on the home chain; never issued here
        _tickets.push(Ticket(coin, amount, recipientOf[coin], mode, Status.Sent, false, uint64(block.timestamp), uint64(block.number), 0));
    }

    // ---- receipt: the home vault, through the inbox ----------------------------------------------------------------------------

    /// @notice Only the home vault's aliased address: `HomeVault.acknowledge(ticket)` arrived as a retryable.
    function confirm(uint256 id, uint256 amountSeen) external {
        if (msg.sender != homeVaultAlias) revert NotHomeVault();
        Ticket storage t = _ticket(id);
        if (t.status != Status.Sent) revert WrongStage();
        t.status = Status.Confirmed;
        t.amountSeen = amountSeen;
        if (t.mode == Mode.Fast) {
            fastOpen--;
            if (t.overdue) { t.overdue = false; fastOverdue--; }
        }
        emit Confirmed(id, amountSeen);
    }

    /// @notice Anyone: a fast ticket unconfirmed past FAST_DEADLINE on both clocks blocks further fast releases until it is confirmed.
    function flagOverdue(uint256 id) external {
        Ticket storage t = _ticket(id);
        if (t.mode != Mode.Fast || t.status != Status.Sent || t.overdue) revert WrongStage();
        if (!Wait.elapsed(uint64(t.at + FAST_DEADLINE), uint64(t.atBlock + Wait.blocksFor(FAST_DEADLINE, parentBlockSeconds)))) revert TooEarly();
        t.overdue = true;
        fastOverdue++;
        emit Overdue(id);
    }

    // ---- the fast-bridge allow-list: the owner, with a delay to add, none to remove ---------------------------------------------

    function proposeBridge(address bridge) external {
        if (msg.sender != owner) revert NotOwner();
        if (bridge == address(0) || bridge.code.length == 0) revert BadInput();
        bridgeReadyAt[bridge] = uint64(block.timestamp + CHANGE_DELAY);
        bridgeReadyAtBlock[bridge] = uint64(block.number + Wait.blocksFor(CHANGE_DELAY, parentBlockSeconds));
        emit BridgeProposed(bridge, bridgeReadyAt[bridge]);
    }

    function acceptBridge(address bridge) external {
        if (bridgeReadyAt[bridge] == 0) revert WrongStage();
        if (!Wait.elapsed(bridgeReadyAt[bridge], bridgeReadyAtBlock[bridge])) revert TooEarly();
        isBridge[bridge] = true;
        delete bridgeReadyAt[bridge];
        delete bridgeReadyAtBlock[bridge];
        emit BridgeAccepted(bridge);
    }

    function removeBridge(address bridge) external {
        if (msg.sender != owner) revert NotOwner();
        isBridge[bridge] = false;
        delete bridgeReadyAt[bridge];
        delete bridgeReadyAtBlock[bridge];
        emit BridgeRemoved(bridge);
    }

    // ---- views ----------------------------------------------------------------------------------------------------------------

    function ticket(uint256 id) external view returns (Ticket memory) { return _ticket(id); }
    function ticketCount() external view returns (uint256) { return _tickets.length; }
    function coinOf(uint256 id) external view returns (address) { return _ticket(id).coin; }
    function l2RecipientOf(uint256 id) external view returns (address) { return _ticket(id).l2Recipient; }

    function _ticket(uint256 id) private view returns (Ticket storage) {
        if (id == 0 || id > _tickets.length) revert BadInput();
        return _tickets[id - 1];
    }
}
