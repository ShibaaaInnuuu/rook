// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {OfferItem, ConsiderationItem, OrderParameters, OrderType} from "../nft/Seaport16.sol";

/// @notice What happens to everything a home-chain vault buys. Fixed per vault at construction. docs/v6/home-chain-vault.md §3.3.
enum HomePolicy { Raffle, Burn, Yoyo, Wrap }

/// @notice CryptoPunksMarket (Ethereum, 0xb47e3cd837dDF8e4c57F05d70Ab865de6e193BBB): not an ERC-721. Its own order book, and a
/// seller who is paid by `pendingWithdrawals` and must `withdraw()`. docs/v6/home-chain-vault.md §1.
interface ICryptoPunksMarket {
    function punksOfferedForSale(uint256 punkIndex) external view
        returns (bool isForSale, uint256 punkIndex_, address seller, uint256 minValue, address onlySellTo);
    function punkIndexToAddress(uint256 punkIndex) external view returns (address);
    function pendingWithdrawals(address who) external view returns (uint256);
    function buyPunk(uint256 punkIndex) external payable;
    function offerPunkForSale(uint256 punkIndex, uint256 minSalePriceInWei) external;
    function offerPunkForSaleToAddress(uint256 punkIndex, uint256 minSalePriceInWei, address toAddress) external;
    function transferPunk(address to, uint256 punkIndex) external;
    function withdraw() external;
}

/// @notice Arbitrum Nitro's parent-chain Inbox (Robinhood Chain: 0x1A07cc4BD17E0118BdB54D70990D2158AbAD7a2D on Ethereum).
interface IArbInbox {
    function createRetryableTicket(address to, uint256 l2CallValue, uint256 maxSubmissionCost, address excessFeeRefundAddress,
        address callValueRefundAddress, uint256 gasLimit, uint256 maxFeePerGas, bytes calldata data) external payable
        returns (uint256);
    function calculateRetryableSubmissionFee(uint256 dataLength, uint256 baseFee) external view returns (uint256);
}

/// @notice The parent-chain Outbox that executes L2→L1 messages (Robinhood Chain: 0xf0ce991ea4A0d2400A4AB49b20ae333f6Dce3DE9).
interface IArbOutbox {
    /// @notice The L2 sender of the message being executed right now; zero outside an execution.
    function l2ToL1Sender() external view returns (address);
}

/// @notice Nitro's ArbSys precompile at 0x64 on the child chain.
interface IArbSys {
    function sendTxToL1(address destination, bytes calldata data) external payable returns (uint256);
}

/// @notice The parent-chain gateway router of the standard token bridge (Robinhood Chain: 0x6a2E3a1e16FC29f27Ce61429746D558d656975bB).
interface IL1GatewayRouter {
    function outboundTransfer(address token, address to, uint256 amount, uint256 maxGas, uint256 gasPriceBid, bytes calldata data)
        external payable returns (bytes memory);
    function getGateway(address token) external view returns (address);
    function calculateL2TokenAddress(address l1Token) external view returns (address);
}

/// @notice Seaport 1.6's own types that the sweep subset (Seaport16.sol) does not carry: what an order hash is over, and a plain order.
struct OrderComponents {
    address offerer; address zone; OfferItem[] offer; ConsiderationItem[] consideration; OrderType orderType; uint256 startTime;
    uint256 endTime; bytes32 zoneHash; uint256 salt; bytes32 conduitKey; uint256 counter;
}
struct Order { OrderParameters parameters; bytes signature; }

/// @notice What a YOYO vault needs from Seaport 1.6 beyond the fills: a contract-validated order needs no signature when the offerer
/// is the caller. docs/v6/home-chain-vault.md §3.4.
interface ISeaportValidate {
    function validate(Order[] calldata orders) external returns (bool validated);
    function getOrderHash(OrderComponents calldata order) external view returns (bytes32);
    function getCounter(address offerer) external view returns (uint256);
    function getOrderStatus(bytes32 orderHash) external view
        returns (bool isValidated, bool isCancelled, uint256 totalFilled, uint256 totalSize);
}

/// @notice What the vault and the wrapper ask the guard. docs/v6/home-chain-vault.md §6.
interface IHomeGuard {
    function admit(address collection, uint256 items, uint256 valueWei) external;
    function release(address collection, uint256 items, uint256 valueWei) external;
    function out(address collection, uint256 items, uint256 valueWei) external;
    function paused() external view returns (bool);
}

/// @notice What the vault asks its wrapper under the Wrap policy.
interface IHomeWrapperMint {
    function wrap(uint256 tokenId, address to) external;
}

/// @notice The 4663 side, as the home chain calls it through retryables (docs/v6/home-chain-vault.md §5).
interface IFeeEscrowConfirm {
    function confirm(uint256 ticket, uint256 amountSeen) external;
}
interface IHomeReceiverCredit {
    function creditProceeds(uint256 ticket) external payable;
    function creditWrapped(uint256 ticket, uint256 units) external;
}

/// @notice Nitro's L1→L2 address aliasing: a parent-chain contract's message executes on the child chain from its alias.
library AddressAlias {
    uint160 internal constant OFFSET = uint160(0x1111000000000000000000000000000000001111);
    function l2(address l1) internal pure returns (address) {
        unchecked { return address(uint160(l1) + OFFSET); }
    }
}

/// @notice The child-chain gateway router of the standard token bridge (Robinhood Chain: 0x1E324B9316138CA9a73F960213621AD1aaf01B89).
/// `outboundTransfer` burns the caller's twin through the standard gateway (`bridgeBurn`, the gateway's own right: no approval)
/// and the parent-chain gateway releases the L1 token to `to` once the message is executed. docs/v6/home-chain-vault.md §2.3.
interface IL2GatewayRouter {
    function outboundTransfer(address l1Token, address to, uint256 amount, bytes calldata data) external payable returns (bytes memory);
}

/// @notice The home chain's `HomeRedeemDesk`, as the 4663 adapter's L2→L1 message calls it (docs/v6/home-chain-vault.md §10.3).
interface IHomeRedeemDeskIntent {
    function intent(uint256 id, uint8 kind, address owner) external;
}

/// @notice The 4663 `HomeWrappedAdapter`, as the desk's retryable calls it (docs/v6/home-chain-vault.md §10.5).
interface IHomeAdapterAck {
    function acknowledge(uint256 id) external;
}
