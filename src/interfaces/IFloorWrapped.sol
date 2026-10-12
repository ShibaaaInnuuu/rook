// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IWrappedCollection} from "./IWrappedCollection.sol";

/// @title IFloorWrapped
/// @notice What a collection's floor module needs from its wrapper, beyond `IWrappedCollection` (docs/v5/floor-module.md §3, §9).
/// The wrapper mints ETH-backed units for exactly one module, wired once before anything was ever minted, and admits and returns
/// rented items. Its ledger: `totalSupply == (held − openTickets) × ONE_ITEM + ethBacked`, and a redemption needs
/// an item nobody is promised: `held − openTickets − openReturns ≥ 1`.
interface IFloorWrapped is IWrappedCollection {
    /// @notice Where a rented item is (the wrapper alone knows): docs/v5/floor-module.md §9.3.
    enum RentalState { None, Active, ActiveDrawn, Leaving, LeavingDrawn, Ready, Returned, Taken }

    /// @notice The one address allowed to mint and burn ETH-backed units and to admit and release rentals; zero until wired.
    function floorModule() external view returns (address);

    /// @notice Units in supply backed by the module's ETH rather than by an item.
    function ethBacked() external view returns (uint256);

    /// @notice Rented items promised back to their renters (leaving, drawn while leaving, or ready to return).
    function openReturns() external view returns (uint256);

    /// @notice Rented items in custody: never counted as owned.
    function rentedHeld() external view returns (uint256);

    /// @notice The module: mint `amount` ETH-backed units to `to`.
    function moduleMint(address to, uint256 amount) external;

    /// @notice The module: burn `amount` of its own units, turning that many ETH-backed units into none (a burn for ETH, or an
    /// item recorded against them).
    function moduleBurn(uint256 amount) external;

    /// @notice The module, for the custodian: admit one rented item through the deposit path's own checks. One unit is minted to
    /// the module; the item is in the random set like any other, stamped; `returnTo` is where it goes back at the end.
    /// @return itemId the wrapper's id for it
    /// @return itemKey the key the keeper's eligibility list names it by
    function admitRental(bytes calldata attestation, bytes calldata returnTo) external returns (uint256 itemId, bytes32 itemKey);

    /// @notice The module, at a rental's end: the item is promised back and leaves the random set after every ticket burned before
    /// this call has resolved. The custodian's `recordReturn` later makes the unit it backed ETH-backed (`ethBacked += ONE_ITEM`)
    /// and, in the same call, tells the module (`IFloorRentals.rentalReturned`) so the rental's escrow backs that unit at once.
    function returnRental(uint256 itemId) external;

    function rentalState(uint256 itemId) external view returns (RentalState);
}

/// @title IFloorRentals
/// @notice What a wrapper with rentals calls on its floor module (deep-fuzz 2026-09-28): the moment a rented item's return makes the
/// unit it backed ETH-backed again, the rental's escrowed ETH must be that unit's backing again, in the same transaction.
interface IFloorRentals {
    /// @notice The wrapper, from `recordReturn`: the rented item `itemId` went home; its unit is ETH-backed from now on.
    function rentalReturned(uint256 itemId) external;
}
