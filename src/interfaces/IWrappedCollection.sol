// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @title IWrappedCollection
/// @notice What a curve, a mint desk, a sweep vault or a buy queue needs from a wrapped collection, whatever the collection is:
/// Bitcoin inscriptions (`Wrapper`, wFROG) or an ERC-721 on another EVM chain (`NftWrapper`, e.g. wAPE). One whole token is
/// one item in the custodian's keeping; a whole token burns for a random item. docs/v4/nft-wrapper.md §3.
/// @dev Deliberately left out, because it differs per chain or is nobody's business but the custodian's: redeeming to a person
/// (a taproot string or an EVM address), the ticket struct, minting, and which collection this is.
interface IWrappedCollection is IERC20 {
    /// @notice 1e18: one whole token is one item in custody.
    function ONE_ITEM() external view returns (uint256);

    /// @notice The canister: the only minter, and the only address a consumer should accept "I minted it to you" from.
    function custodian() external view returns (address);

    /// @notice Items in custody and on the books: drawable, waiting to be, or named by a resolved, undelivered ticket.
    function held() external view returns (uint256);

    /// @notice Burns neither delivered nor refunded. `totalSupply == (held − openTickets) × ONE_ITEM` (plus `ethBacked` for a wrapper
    /// with a floor module: docs/v5/floor-module.md §4).
    function openTickets() external view returns (uint256);

    /// @notice Burn `ONE_ITEM` of the caller's for a random item, delivered where nobody can ever move it again.
    function redeemToBurn() external returns (uint256 ticketId);

    /// @notice Burn `ONE_ITEM` of the caller's for a random item that stays in custody and is listed for sale there (the YOYO
    /// policy's exit, docs/v5/policies-and-fees.md §1): its delivery is the listing.
    function redeemToRelist() external returns (uint256 ticketId);

    /// @notice Whether `ticketId` was refunded: nobody delivered in time, its `ONE_ITEM` went back to the ticket's owner.
    function refunded(uint256 ticketId) external view returns (bool);
}
