// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {AddressAlias, IHomeReceiverCredit} from "./IHomeChain.sol";

/// @notice What the receiver reads from its escrow: which coin a ticket was for, and where its wX goes.
interface IFeeEscrowTickets {
    function coinOf(uint256 id) external view returns (address);
    function l2RecipientOf(uint256 id) external view returns (address);
}

/// @notice `EtchRouter.buy`: ETH in, coin out to `to`. Nothing else of the router is used.
interface IBuybackRouter {
    function buy(address coin, uint256 minOut, address to, uint256 deadline) external payable returns (uint256 out);
}

/// @notice How many units of `coin` `ethIn` should buy right now, before slippage. `LockSpotQuote` answers it from the pools.
interface IEthQuote {
    function coinOutForEth(address coin, uint256 ethIn) external view returns (uint256);
}

/// @title HomeReceiver
/// @notice The 4663 landing of everything a home-chain vault sends down (docs/v6/home-chain-vault.md §5.3, §5.4): YOYO sale proceeds,
/// credited to the coin whose ticket funded the sold item and spent by ANYONE's `buyback` — bought through the router straight to
/// 0x…dEaD, in tranches, with a spot-derived minimum out — and wX from Wrap purchases, released to the address the escrow's ticket
/// named on 4663. Both credits are accepted only from the home vault's aliased address: a retryable from the vault, and nothing else.
/// @dev What this does NOT protect against: a sandwich inside the slippage bound (5% of a tranche at most), and a dead retryable's
/// refund, which ArbOS credits to this address without a call: `absorb` lets anyone attribute such unaccounted ETH to a ticket's coin.
contract HomeReceiver is IHomeReceiverCredit, ReentrancyGuard {
    using SafeERC20 for IERC20;

    address public immutable homeVaultAlias;
    IFeeEscrowTickets public immutable escrow;
    IERC20 public immutable wx;             // the StandardArbERC20 twin of the home chain's HomeWrapper
    IBuybackRouter public immutable router;
    IEthQuote public immutable quote;
    uint256 public immutable reward;
    uint256 public immutable maxTranche;
    uint256 public immutable cooldown;

    uint256 public constant MAX_SLIPPAGE_BPS = 500;
    address public constant DEAD = 0x000000000000000000000000000000000000dEaD;

    mapping(address coin => uint256) public proceedsOf;
    mapping(uint256 ticket => uint256) public wrappedOwed;
    mapping(address coin => uint64) public lastBuybackAt;
    uint256 public proceedsTotal;
    uint256 public wrappedOwedTotal;

    event ProceedsCredited(uint256 indexed ticket, address indexed coin, uint256 amount);
    event WrappedCredited(uint256 indexed ticket, uint256 units);
    event WrappedClaimed(uint256 indexed ticket, address indexed to, uint256 units);
    event BoughtBack(address indexed coin, uint256 ethIn, uint256 burned, address indexed by);
    event Absorbed(uint256 indexed ticket, address indexed coin, uint256 amount);

    error BadInput();
    error NotHomeVault();
    error ZeroAmount();
    error Cooldown(uint64 until);
    error NothingOwed();
    error TransferFailed();

    constructor(address homeVault, IFeeEscrowTickets escrow_, IERC20 wx_, IBuybackRouter router_, IEthQuote quote_, uint256 reward_,
        uint256 maxTranche_, uint256 cooldown_)
    {
        if (homeVault == address(0) || address(escrow_) == address(0) || address(router_) == address(0) || address(quote_) == address(0)
            || reward_ == 0 || maxTranche_ <= reward_) revert BadInput();
        homeVaultAlias = AddressAlias.l2(homeVault);
        escrow = escrow_;
        wx = wx_;
        router = router_;
        quote = quote_;
        reward = reward_;
        maxTranche = maxTranche_;
        cooldown = cooldown_;
    }

    modifier onlyHomeVault() { if (msg.sender != homeVaultAlias) revert NotHomeVault(); _; }

    // ---- credits: the home vault's retryables ---------------------------------------------------------------------------------------

    /// @notice A settled YOYO sale's proceeds, for the coin whose ticket funded the item.
    function creditProceeds(uint256 ticket) external payable onlyHomeVault {
        if (msg.value == 0) revert ZeroAmount();
        address coin = escrow.coinOf(ticket);
        proceedsOf[coin] += msg.value;
        proceedsTotal += msg.value;
        emit ProceedsCredited(ticket, coin, msg.value);
    }

    /// @notice The report that `units` of wX bridged down belong to `ticket`.
    function creditWrapped(uint256 ticket, uint256 units) external onlyHomeVault {
        if (units == 0) revert ZeroAmount();
        wrappedOwed[ticket] += units;
        wrappedOwedTotal += units;
        emit WrappedCredited(ticket, units);
    }

    /// @notice Anyone: whatever of a ticket's wX has arrived goes to the address the escrow's ticket named, and nowhere else.
    function claimWrapped(uint256 ticket) external nonReentrant {
        uint256 owed = wrappedOwed[ticket];
        if (owed == 0) revert NothingOwed();
        uint256 have = wx.balanceOf(address(this));
        uint256 units = owed < have ? owed : have;
        if (units == 0) revert NothingOwed();
        wrappedOwed[ticket] = owed - units;
        wrappedOwedTotal -= units;
        address to = escrow.l2RecipientOf(ticket);
        wx.safeTransfer(to, units);
        emit WrappedClaimed(ticket, to, units);
    }

    // ---- the buyback: anyone, in tranches, burned ----------------------------------------------------------------------------------

    /// @notice Spends up to `maxTranche` of `coin`'s proceeds through the router to 0x…dEaD, at most once per `cooldown` per coin,
    /// refusing worse than MAX_SLIPPAGE_BPS under the pools' spot. Pays the caller `reward` from the tranche.
    function buyback(address coin) external nonReentrant returns (uint256 burned) {
        uint256 have = proceedsOf[coin];
        uint256 tranche = have < maxTranche ? have : maxTranche;
        if (tranche <= reward) revert ZeroAmount();
        uint64 until = uint64(lastBuybackAt[coin] + cooldown);
        if (lastBuybackAt[coin] != 0 && block.timestamp < until) revert Cooldown(until);
        lastBuybackAt[coin] = uint64(block.timestamp);
        proceedsOf[coin] = have - tranche;
        proceedsTotal -= tranche;
        uint256 ethIn = tranche - reward;
        uint256 minOut = quote.coinOutForEth(coin, ethIn) * (10_000 - MAX_SLIPPAGE_BPS) / 10_000;
        burned = router.buy{value: ethIn}(coin, minOut, DEAD, block.timestamp);
        (bool ok,) = msg.sender.call{value: reward}("");
        if (!ok) revert TransferFailed();
        emit BoughtBack(coin, ethIn, burned, msg.sender);
    }

    /// @notice ETH here that no credit accounts for (a dead retryable's refund) may be attributed by anyone to a ticket's coin.
    function absorb(uint256 ticket) external {
        uint256 free = address(this).balance - proceedsTotal;
        if (free == 0) revert ZeroAmount();
        address coin = escrow.coinOf(ticket);
        proceedsOf[coin] += free;
        proceedsTotal += free;
        emit Absorbed(ticket, coin, free);
    }

    /// @notice The router's refund of what a buy did not take (it spends everything: none expected), and nothing else by plain transfer.
    receive() external payable {
        if (msg.sender != address(router)) revert NotHomeVault();
        proceedsTotal += msg.value;
        proceedsOf[address(0)] += msg.value;
    }
}
