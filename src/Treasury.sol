// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {ListingRegistry} from "./ListingRegistry.sol";
import {IEtchVault} from "./interfaces/IEtchVault.sol";
import {ILauncher} from "./interfaces/ILauncher.sol";

/// @title Treasury
/// @notice Every coin's router sends twenty per cent of its fees here. This contract lets that ETH out ONE way: into a vault the
/// registry knows, attributed to the protocol token, where it can only ever buy that listing's own asset for that token's holders.
/// There is no withdraw function, for anyone, as there is none in either vault.
/// @dev docs/token/protocol-token-v1.md §2. What the keeper may choose here is a LISTING — that is, a collection or a rune — and
/// nothing else. It cannot name a vault (the address comes from the registry), cannot name a recipient, cannot take ETH out, and
/// cannot change which coin the money is attributed to. The worst it can do is buy for an unpopular collection.
/// What this does NOT protect against:
///  - the owner never setting a token, or setting it to a coin whose holders are all the owner's. The token is set once, in public,
///    and is visible before any fee has arrived.
///  - the owner pausing funding for ever. ETH then sits here, attributed to nobody, exactly as unspent fees sit in a vault.
///  - ETH forced in without a call (selfdestruct): it is fundable like any other balance, and belongs to the token's holders.
contract Treasury is ReentrancyGuard {
    ListingRegistry public immutable registry;

    /// @notice The coin whose holders every purchase funded from here is for. Set once, and never again.
    address public token;
    bool public paused;

    uint256 public totalReceived;
    uint256 public totalFunded;
    mapping(uint256 listingId => uint256) public fundedTo;

    event Received(address indexed from, uint256 amount);
    event TokenSet(address indexed token);
    event Funded(uint256 indexed listingId, address indexed vault, uint256 amount);
    event PausedSet(bool paused);

    error NotOwner();
    error NotKeeper();
    error TokenAlreadySet();
    error TokenNotSet();
    error UnknownCoin();
    error NoVault();
    error ListingPaused();
    error IsPaused();
    error ZeroAmount();
    error Insufficient(uint256 amount, uint256 available);

    constructor(ListingRegistry registry_) {
        registry = registry_;
    }

    modifier onlyOwner() { if (msg.sender != registry.owner()) revert NotOwner(); _; }
    modifier onlyKeeper() { if (msg.sender != registry.keeper()) revert NotKeeper(); _; }

    receive() external payable {
        totalReceived += msg.value;
        emit Received(msg.sender, msg.value);
    }

    /// @notice Names the protocol token, once. It must be a coin the launcher knows, so that the Distributor can find its creator
    /// and its holders, and so that a mistyped address cannot strand every fee this contract will ever receive.
    function setToken(address token_) external onlyOwner {
        if (token != address(0)) revert TokenAlreadySet();
        if (ILauncher(registry.launcher()).routerOf(token_) == address(0)) revert UnknownCoin();
        token = token_;
        emit TokenSet(token_);
    }

    /// @notice The owner can stop funding and cannot do anything else here. Stopping is not taking: paused or not, the ETH stays.
    function setPaused(bool paused_) external onlyOwner {
        paused = paused_;
        emit PausedSet(paused_);
    }

    /// @notice Moves ETH into one listing's vault, attributed to the protocol token. The vault address is read from the registry,
    /// never supplied, so this cannot send ETH anywhere the registry has not published.
    function fund(uint256 listingId, uint256 amount) external onlyKeeper nonReentrant {
        address token_ = token;
        if (token_ == address(0)) revert TokenNotSet();
        if (paused) revert IsPaused();
        if (registry.isPaused(listingId)) revert ListingPaused();
        if (amount == 0) revert ZeroAmount();
        uint256 available = address(this).balance;
        if (amount > available) revert Insufficient(amount, available);
        address vault = registry.vaultOf(listingId);
        if (vault == address(0)) revert NoVault();
        totalFunded += amount;
        fundedTo[listingId] += amount;
        emit Funded(listingId, vault, amount);
        // through the interface, not a cast to one concrete vault: a listing may be v1 or v2, and the compiler should be
        // the thing that knows this call is available on both, rather than a field order that happens to line up.
        IEtchVault(vault).depositFrom{value: amount}(token_);
    }
}
