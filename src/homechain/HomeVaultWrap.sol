// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC721} from "@openzeppelin/contracts/token/ERC721/IERC721.sol";
import {HomePolicy, ICryptoPunksMarket, IL1GatewayRouter, IHomeWrapperMint, IHomeReceiverCredit} from "./IHomeChain.sol";
import {HomeVaultBase} from "./HomeVaultBase.sol";

/// @title HomeVaultWrap
/// @notice A home-chain vault whose every purchase is wrapped into the collection's `HomeWrapper` in the buying call, the `1e18` wX
/// bridged down to the 4663 receiver through the standard token gateway, with a second retryable reporting which ticket it belongs
/// to. docs/v6/home-chain-vault.md §3.3, §5.4. The item stays counted in the guard (the wrapper holds it); its value is released.
contract HomeVaultWrap is HomeVaultBase {
    uint256 public constant ONE_ITEM = 1e18;
    address public immutable wrapper;
    IL1GatewayRouter public immutable gatewayRouter;

    event Wrapped(uint256 indexed acquisitionId, uint256 indexed tokenId, uint256 indexed ticket);

    constructor(Config memory c, address wrapper_, IL1GatewayRouter gatewayRouter_) HomeVaultBase(c, HomePolicy.Wrap) {
        if (wrapper_ == address(0) || address(gatewayRouter_) == address(0)) revert BadInput();
        wrapper = wrapper_;
        gatewayRouter = gatewayRouter_;
    }

    /// @dev The two retryables that carry the wX and its report down: `outboundTransfer` with no data, and `creditWrapped(uint256,uint256)`.
    function _extraCost() internal view override returns (uint256) {
        return bridgeFee(0) + bridgeFee(68);
    }

    function _afterBuy(uint256 acquisitionId) internal override {
        Acquisition storage a = _acquisitions[acquisitionId];
        if (isPunks) ICryptoPunksMarket(collection).offerPunkForSaleToAddress(a.tokenId, 0, wrapper);
        else IERC721(collection).approve(wrapper, a.tokenId);
        IHomeWrapperMint(wrapper).wrap(a.tokenId, address(this));
        _stage(a, Stage.Wrapped);
        guard.release(collection, 0, a.pricePaid);
        uint256 fee1 = bridgeFee(0);
        IERC20(wrapper).approve(gatewayRouter.getGateway(wrapper), ONE_ITEM);
        gatewayRouter.outboundTransfer{value: fee1}(wrapper, receiver, ONE_ITEM, L2_GAS_LIMIT, L2_MAX_FEE,
            abi.encode(2 * inbox.calculateRetryableSubmissionFee(0, block.basefee), ""));
        bytes memory data = abi.encodeCall(IHomeReceiverCredit.creditWrapped, (a.fundedBy, ONE_ITEM));
        uint256 fee2 = bridgeFee(data.length);
        inbox.createRetryableTicket{value: fee2}(receiver, 0, 2 * inbox.calculateRetryableSubmissionFee(data.length, block.basefee),
            receiver, receiver, L2_GAS_LIMIT, L2_MAX_FEE, data);
        guard.out(collection, 0, fee1 + fee2);
        emit Wrapped(acquisitionId, a.tokenId, a.fundedBy);
    }
}
