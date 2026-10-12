// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {
    ItemType, OrderType, OfferItem, ConsiderationItem, OrderParameters, AdvancedOrder, CriteriaResolver, FulfillmentComponent,
    BasicOrderType, BasicOrderParameters, ISeaport16Sweep
} from "./Seaport16.sol";

/// @title SeaportGuard
/// @notice What an NFT sweep vault lets through to Seaport 1.6, and nothing else. docs/v4/nft.md §5.1, §10.1.
///
/// The caller's calldata is DECODED, every order in it is checked against the listing and the cap, and the checked values are
/// RE-ENCODED: Seaport executes exactly what was checked, never the caller's bytes. Per order:
///   - one offer item: an ERC-721 of the listing's collection, a candidate id, amount 1;
///   - consideration in native ETH only, no item past the signed count (no fulfiller tip), summing to at most the cap;
///   - the offerer and every recipient are neither the sweep's caller nor tx.origin (a speed bump, not a defence: §10.1);
///   - no contract orders, no partial fills, no criteria, no fulfiller conduit, and the vault receives what is bought.
/// @dev Accepted: fulfillBasicOrder, fulfillBasicOrder_efficient_6GL6yc (one NFT, the ETH-for-ERC-721 route) and
/// fulfillAvailableAdvancedOrders (up to one order per candidate). Everything else, matchAdvancedOrders included, is refused.
library SeaportGuard {
    error UnsupportedCall(bytes4 selector);
    error WrongOffer(uint256 order);
    error WrongConsideration(uint256 order);
    error Tip(uint256 order);
    error PaysTheCaller(uint256 order, address who);
    error OrderOverCap(uint256 order, uint256 price, uint256 cap);
    error BadFill();

    /// @return forward the re-encoded calldata to send to Seaport
    /// @return total the most native ETH the orders can take: the sum of each order's price, each at most `capWei`
    function check(bytes calldata data, address collection, uint256[] calldata candidates, uint256 capWei)
        internal view returns (bytes memory forward, uint256 total)
    {
        if (data.length < 4) revert UnsupportedCall(bytes4(data));
        bytes4 sel = bytes4(data[:4]);
        if (sel == ISeaport16Sweep.fulfillBasicOrder.selector || sel == ISeaport16Sweep.fulfillBasicOrder_efficient_6GL6yc.selector) {
            BasicOrderParameters memory p = abi.decode(data[4:], (BasicOrderParameters));
            total = _basic(p, collection, candidates, capWei);
            forward = abi.encodeWithSelector(sel, p);
        } else if (sel == ISeaport16Sweep.fulfillAvailableAdvancedOrders.selector) {
            (AdvancedOrder[] memory orders, CriteriaResolver[] memory resolvers, FulfillmentComponent[][] memory offerF,
                FulfillmentComponent[][] memory considerationF, bytes32 conduitKey, address recipient, uint256 maximumFulfilled) =
                abi.decode(data[4:], (AdvancedOrder[], CriteriaResolver[], FulfillmentComponent[][], FulfillmentComponent[][],
                    bytes32, address, uint256));
            if (resolvers.length != 0 || conduitKey != bytes32(0) || (recipient != address(0) && recipient != address(this))
                || orders.length == 0 || orders.length > candidates.length) revert BadFill();
            for (uint256 i = 0; i < orders.length; ++i) total += _advanced(i, orders[i], collection, candidates, capWei);
            forward = abi.encodeWithSelector(sel, orders, resolvers, offerF, considerationF, bytes32(0), address(this), maximumFulfilled);
        } else {
            revert UnsupportedCall(sel);
        }
    }

    function _basic(BasicOrderParameters memory p, address collection, uint256[] calldata candidates, uint256 capWei)
        private view returns (uint256 price)
    {
        // routes 0–3 are ETH for an ERC-721 (full or partial, open or restricted); every other route pays or receives something else
        if (uint8(p.basicOrderType) > uint8(BasicOrderType.ETH_TO_ERC721_PARTIAL_RESTRICTED) || p.fulfillerConduitKey != bytes32(0))
            revert BadFill();
        if (p.offerToken != collection || p.offerAmount != 1 || !_isCandidate(p.offerIdentifier, candidates)) revert WrongOffer(0);
        if (p.considerationToken != address(0) || p.considerationIdentifier != 0) revert WrongConsideration(0);
        if (p.additionalRecipients.length != p.totalOriginalAdditionalRecipients) revert Tip(0);
        _notCaller(0, p.offerer);
        price = p.considerationAmount;
        for (uint256 j = 0; j < p.additionalRecipients.length; ++j) {
            _notCaller(0, p.additionalRecipients[j].recipient);
            price += p.additionalRecipients[j].amount;
        }
        if (price > capWei) revert OrderOverCap(0, price, capWei);
    }

    function _advanced(uint256 i, AdvancedOrder memory a, address collection, uint256[] calldata candidates, uint256 capWei)
        private view returns (uint256 price)
    {
        OrderParameters memory o = a.parameters;
        if (a.numerator == 0 || a.numerator != a.denominator || o.orderType == OrderType.CONTRACT) revert BadFill();
        if (o.offer.length != 1) revert WrongOffer(i);
        OfferItem memory it = o.offer[0];
        if (it.itemType != ItemType.ERC721 || it.token != collection || it.startAmount != 1 || it.endAmount != 1
            || !_isCandidate(it.identifierOrCriteria, candidates)) revert WrongOffer(i);
        if (o.consideration.length != o.totalOriginalConsiderationItems) revert Tip(i);
        _notCaller(i, o.offerer);
        for (uint256 j = 0; j < o.consideration.length; ++j) {
            ConsiderationItem memory c = o.consideration[j];
            if (c.itemType != ItemType.NATIVE || c.token != address(0) || c.identifierOrCriteria != 0) revert WrongConsideration(i);
            _notCaller(i, c.recipient);
            price += c.startAmount > c.endAmount ? c.startAmount : c.endAmount;   // an ascending or descending price, at its highest
        }
        if (price > capWei) revert OrderOverCap(i, price, capWei);
    }

    /// @dev Neither the sweep's caller nor the transaction's signer may be paid, and the vault cannot be the offerer. Only catches
    /// the careless: a second address walks around it (docs/v4/nft.md §10.1).
    function _notCaller(uint256 i, address who) private view {
        if (who == msg.sender || who == tx.origin || who == address(this)) revert PaysTheCaller(i, who);
    }

    function _isCandidate(uint256 id, uint256[] calldata candidates) private pure returns (bool) {
        for (uint256 k = 0; k < candidates.length; ++k) if (candidates[k] == id) return true;
        return false;
    }
}
