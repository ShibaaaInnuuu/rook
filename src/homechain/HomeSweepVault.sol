// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ListingRegistry} from "../ListingRegistry.sol";
import {SweepVault} from "../SweepVault.sol";
import {MintDesk} from "../curve/MintDesk.sol";
import {CurveFactory} from "../curve/CurveFactory.sol";
import {ILauncher} from "../interfaces/ILauncher.sol";
import {IWrappedCollection} from "../interfaces/IWrappedCollection.sol";
import {INftVault} from "../nft/INft.sol";

/// @notice What the vault reads back from the escrow it is built with: that the escrow names this vault, for this listing.
interface IEscrowNames {
    function launcher() external view returns (address);
    function listingId() external view returns (uint256);
}

/// @title HomeSweepVault
/// @notice `SweepVault` for a home-chain collection (docs/v6/home-chain-vault.md §10.4): the whole vault, unchanged, plus one
/// road for a coin's credit — `forward(coin)`, anyone's call, to the listing's `FeeEscrow`, from where anyone's `release` sends it
/// up to the home vault as a numbered ticket (§5). The escrow accepts a deposit only from `launcher.routerOf(coin)`; its
/// `launcher` is THIS vault, which answers for itself: the escrow trusts exactly one contract, fixed at its construction, for
/// exactly the coins that contract carries (this factory's, of this policy).
/// @dev The keeper's `sweep` at the router and anyone's `forward` draw from the same `credit`: whichever comes first takes it.
/// Both spend the coin's fee on the coin's collection. Units here (a sweep's, a coin quoted in wX) are the adapter's, drawn or
/// retired through it as any wrapper's.
contract HomeSweepVault is SweepVault {
    INftVault public immutable escrow;

    event Forwarded(address indexed coin, uint256 amount, address indexed by);

    error NoEscrow();
    error EscrowNotMine();
    error NothingToForward();

    constructor(ListingRegistry registry_, uint256 listingId_, IWrappedCollection wrapper_, Policy policy_, address venue_,
        ILauncher launcher_, MintDesk desk_, uint256 parentBlockSeconds_, INftVault escrow_)
        SweepVault(registry_, listingId_, wrapper_, policy_, venue_, launcher_, desk_, parentBlockSeconds_)
    {
        if (address(escrow_) == address(0)) revert NoEscrow();
        IEscrowNames e = IEscrowNames(address(escrow_));
        if (e.launcher() != address(this) || e.listingId() != listingId_) revert EscrowNotMine();
        escrow = escrow_;
    }

    /// @notice Anyone, while the listing is not paused: everything `coin` has here in ETH goes to the escrow, earmarked to `coin`.
    function forward(address coin) external nonReentrant returns (uint256 amount) {
        if (registry.isPaused(listingId)) revert ListingPaused();
        amount = credit[coin];
        if (amount == 0) revert NothingToForward();
        credit[coin] = 0;
        totalCredit -= amount;
        escrow.deposit{value: amount}(coin);
        emit Forwarded(coin, amount, msg.sender);
    }

    // ---- what the escrow asks its `launcher`: this vault, for its own coins ---------------------------------------------------------

    /// @notice The address that may pay the escrow for `coin`: this vault, if the coin is this factory's and of this policy.
    function routerOf(address coin) external view returns (address) { return _mine(coin) ? address(this) : address(0); }
    function creatorOf(address coin) external view returns (address) { return launcher.creatorOf(coin); }
    /// @notice This listing for this vault's coins; an id no escrow has for any other.
    function listingOf(address coin) external view returns (uint256) { return _mine(coin) ? listingId : type(uint256).max; }

    function _mine(address coin) private view returns (bool) {
        return coin != address(0) && launcher.routerOf(coin) != address(0) && CurveFactory(address(launcher)).policyOf(coin) == policy;
    }
}
