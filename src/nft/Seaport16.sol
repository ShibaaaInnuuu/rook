// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice Seaport 1.6's own types, as its ABI encodes them (seaport-types/src/lib/ConsiderationStructs.sol and ConsiderationEnums.sol),
/// for the three entry points an NFT sweep vault forwards. docs/v4/nft.md §5.1.
enum ItemType { NATIVE, ERC20, ERC721, ERC1155, ERC721_WITH_CRITERIA, ERC1155_WITH_CRITERIA }
enum OrderType { FULL_OPEN, PARTIAL_OPEN, FULL_RESTRICTED, PARTIAL_RESTRICTED, CONTRACT }
enum Side { OFFER, CONSIDERATION }
enum BasicOrderType {
    ETH_TO_ERC721_FULL_OPEN, ETH_TO_ERC721_PARTIAL_OPEN, ETH_TO_ERC721_FULL_RESTRICTED, ETH_TO_ERC721_PARTIAL_RESTRICTED,
    ETH_TO_ERC1155_FULL_OPEN, ETH_TO_ERC1155_PARTIAL_OPEN, ETH_TO_ERC1155_FULL_RESTRICTED, ETH_TO_ERC1155_PARTIAL_RESTRICTED,
    ERC20_TO_ERC721_FULL_OPEN, ERC20_TO_ERC721_PARTIAL_OPEN, ERC20_TO_ERC721_FULL_RESTRICTED, ERC20_TO_ERC721_PARTIAL_RESTRICTED,
    ERC20_TO_ERC1155_FULL_OPEN, ERC20_TO_ERC1155_PARTIAL_OPEN, ERC20_TO_ERC1155_FULL_RESTRICTED, ERC20_TO_ERC1155_PARTIAL_RESTRICTED,
    ERC721_TO_ERC20_FULL_OPEN, ERC721_TO_ERC20_PARTIAL_OPEN, ERC721_TO_ERC20_FULL_RESTRICTED, ERC721_TO_ERC20_PARTIAL_RESTRICTED,
    ERC1155_TO_ERC20_FULL_OPEN, ERC1155_TO_ERC20_PARTIAL_OPEN, ERC1155_TO_ERC20_FULL_RESTRICTED, ERC1155_TO_ERC20_PARTIAL_RESTRICTED
}

struct OfferItem { ItemType itemType; address token; uint256 identifierOrCriteria; uint256 startAmount; uint256 endAmount; }
struct ConsiderationItem {
    ItemType itemType; address token; uint256 identifierOrCriteria; uint256 startAmount; uint256 endAmount; address payable recipient;
}
struct OrderParameters {
    address offerer; address zone; OfferItem[] offer; ConsiderationItem[] consideration; OrderType orderType; uint256 startTime;
    uint256 endTime; bytes32 zoneHash; uint256 salt; bytes32 conduitKey; uint256 totalOriginalConsiderationItems;
}
struct AdvancedOrder { OrderParameters parameters; uint120 numerator; uint120 denominator; bytes signature; bytes extraData; }
struct CriteriaResolver { uint256 orderIndex; Side side; uint256 index; uint256 identifier; bytes32[] criteriaProof; }
struct FulfillmentComponent { uint256 orderIndex; uint256 itemIndex; }
struct ReceivedItem { ItemType itemType; address token; uint256 identifier; uint256 amount; address payable recipient; }
struct Execution { ReceivedItem item; address offerer; bytes32 conduitKey; }
struct AdditionalRecipient { uint256 amount; address payable recipient; }
struct BasicOrderParameters {
    address considerationToken; uint256 considerationIdentifier; uint256 considerationAmount; address payable offerer; address zone;
    address offerToken; uint256 offerIdentifier; uint256 offerAmount; BasicOrderType basicOrderType; uint256 startTime;
    uint256 endTime; bytes32 zoneHash; uint256 salt; bytes32 offererConduitKey; bytes32 fulfillerConduitKey;
    uint256 totalOriginalAdditionalRecipients; AdditionalRecipient[] additionalRecipients; bytes signature;
}

/// @notice The subset of Seaport 1.6 an NFT sweep vault forwards. The selectors are the compiler's, from these signatures:
/// fulfillBasicOrder 0xfb0f3ee1, fulfillBasicOrder_efficient_6GL6yc 0x00000000, fulfillAvailableAdvancedOrders 0x87201b41.
interface ISeaport16Sweep {
    function fulfillBasicOrder(BasicOrderParameters calldata parameters) external payable returns (bool fulfilled);
    function fulfillBasicOrder_efficient_6GL6yc(BasicOrderParameters calldata parameters) external payable returns (bool fulfilled);
    function fulfillAvailableAdvancedOrders(AdvancedOrder[] calldata advancedOrders, CriteriaResolver[] calldata criteriaResolvers,
        FulfillmentComponent[][] calldata offerFulfillments, FulfillmentComponent[][] calldata considerationFulfillments,
        bytes32 fulfillerConduitKey, address recipient, uint256 maximumFulfilled) external payable
        returns (bool[] memory availableOrders, Execution[] memory executions);
}
