// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC721} from "@openzeppelin/contracts/token/ERC721/IERC721.sol";
import {ItemType, OrderType, OfferItem, ConsiderationItem, OrderParameters} from "../nft/Seaport16.sol";
import {HomePolicy, ICryptoPunksMarket, ISeaportValidate, Order, OrderComponents, IHomeReceiverCredit} from "./IHomeChain.sol";
import {HomeVaultBase} from "./HomeVaultBase.sol";

/// @title HomeVaultYoyo
/// @notice PunkStrategy's loop with nobody's hand on it (docs/v6/home-chain-vault.md §3.4, §3.5): every purchase is relisted at
/// `pricePaid × markupBps / 10,000` — a punk through the order book in the buying call, an ERC-721 through a Seaport order the vault
/// VALIDATES itself (a contract cannot sign; Seaport skips the signature for an order its offerer validated) — and once a buyer
/// takes the ask, anyone's `settle` collects the proceeds and bridges them down to the 4663 receiver for the paying coin's buyback.
/// @dev The one approval this system ever gives: a Yoyo vault of an ERC-721 sets Seaport as operator at construction. It is safe
/// because Seaport honours the vault as offerer only under an order the vault validated (it has no key and no `isValidSignature`),
/// and `relist` validates only orders that pay the ask to the vault. `incrementCounter` is never called.
contract HomeVaultYoyo is HomeVaultBase {
    uint256 public constant MIN_MARKUP_BPS = 11_000;
    uint256 public constant MAX_MARKUP_BPS = 12_000;
    uint256 public immutable markupBps;
    uint256 public immutable listingTtl;

    struct Listing { uint256 ask; bytes32 orderHash; uint64 listedUntil; uint256 relistCount; }
    mapping(uint256 acquisitionId => Listing) private _listings;

    event Listed(uint256 indexed acquisitionId, uint256 ask, bytes32 orderHash, uint64 until);
    event Settled(uint256 indexed acquisitionId, uint256 proceeds, uint256 bridged);

    error StillListed();
    error NotSold();

    constructor(Config memory c, uint256 markupBps_, uint256 listingTtl_) HomeVaultBase(c, HomePolicy.Yoyo) {
        if (markupBps_ < MIN_MARKUP_BPS || markupBps_ > MAX_MARKUP_BPS || listingTtl_ == 0) revert BadInput();
        markupBps = markupBps_;
        listingTtl = listingTtl_;
        if (!c.isPunks) IERC721(c.collection).setApprovalForAll(c.seaport, true);
    }

    /// @dev A punk is relisted in the buying call, as PunkStrategy does. An ERC-721 waits for `relist`, whose caller pays the gas of
    /// Seaport's validation and gets the reward for it.
    function _afterBuy(uint256 acquisitionId) internal override {
        if (isPunks) _listPunk(acquisitionId);
    }

    /// @notice Anyone, reward: (re)lists a held or expired lot at the markup. Refused while an ask stands, and once the token is sold.
    function relist(uint256 acquisitionId) external nonReentrant {
        if (guard.paused()) revert WrongStage();
        Acquisition storage a = _acquisitions[acquisitionId];
        Listing storage l = _listings[acquisitionId];
        if (a.stage == Stage.Listed) {
            if (!_owned(a.tokenId)) revert NotSold();
            if (isPunks) {
                (bool forSale,,,, address onlySellTo) = ICryptoPunksMarket(collection).punksOfferedForSale(a.tokenId);
                if (forSale && onlySellTo == address(0)) revert StillListed();
            } else if (block.timestamp < l.listedUntil) {
                revert StillListed();
            }
        } else if (a.stage != Stage.Held) {
            revert WrongStage();
        }
        if (attributed < reward) revert Insufficient(reward, attributed);
        _chargeFifo(purchaseCount++, reward);
        if (isPunks) _listPunk(acquisitionId); else _listSeaport(acquisitionId);
        _payReward(this.relist.selector);
    }

    function _listPunk(uint256 acquisitionId) private {
        Acquisition storage a = _acquisitions[acquisitionId];
        Listing storage l = _listings[acquisitionId];
        uint256 ask = a.pricePaid * markupBps / 10_000;
        ICryptoPunksMarket(collection).offerPunkForSale(a.tokenId, ask);
        _stage(a, Stage.Listed);
        l.ask = ask;
        l.listedUntil = type(uint64).max;
        l.relistCount++;
        emit Listed(acquisitionId, ask, bytes32(0), l.listedUntil);
    }

    function _listSeaport(uint256 acquisitionId) private {
        Acquisition storage a = _acquisitions[acquisitionId];
        Listing storage l = _listings[acquisitionId];
        uint256 ask = a.pricePaid * markupBps / 10_000;
        OfferItem[] memory offer = new OfferItem[](1);
        offer[0] = OfferItem(ItemType.ERC721, collection, a.tokenId, 1, 1);
        ConsiderationItem[] memory cons = new ConsiderationItem[](1);
        cons[0] = ConsiderationItem(ItemType.NATIVE, address(0), 0, ask, ask, payable(address(this)));
        uint256 until = block.timestamp + listingTtl;
        uint256 salt = uint256(keccak256(abi.encode(acquisitionId, l.relistCount)));
        bytes32 orderHash = ISeaportValidate(seaport).getOrderHash(OrderComponents(address(this), address(0), offer, cons,
            OrderType.FULL_OPEN, block.timestamp, until, bytes32(0), salt, bytes32(0), ISeaportValidate(seaport).getCounter(address(this))));
        Order[] memory orders = new Order[](1);
        orders[0].parameters = OrderParameters(address(this), address(0), offer, cons, OrderType.FULL_OPEN, block.timestamp, until,
            bytes32(0), salt, bytes32(0), 1);
        if (!ISeaportValidate(seaport).validate(orders)) revert BuyFailed("validate");
        _stage(a, Stage.Listed);
        l.ask = ask;
        l.orderHash = orderHash;
        l.listedUntil = uint64(until);
        l.relistCount++;
        emit Listed(acquisitionId, ask, orderHash, uint64(until));
    }

    /// @notice Anyone, reward, once a listed lot's token has left: collects the ask (punks: `withdraw()` first, measured by balance)
    /// and bridges `ask − reward − bridgeFee` down to the 4663 receiver for the paying coin's buyback. The reward comes out of the
    /// proceeds, not the budget.
    function settle(uint256 acquisitionId) external nonReentrant {
        if (guard.paused()) revert WrongStage();
        Acquisition storage a = _acquisitions[acquisitionId];
        Listing storage l = _listings[acquisitionId];
        if (a.stage != Stage.Listed) revert WrongStage();
        if (_owned(a.tokenId)) revert NotSold();
        if (isPunks && ICryptoPunksMarket(collection).pendingWithdrawals(address(this)) != 0) {
            uint256 before = address(this).balance;
            ICryptoPunksMarket(collection).withdraw();
            uint256 got = address(this).balance - before;
            proceedsHeld += got;
            emit ProceedsIn(collection, got);
        }
        uint256 ask = l.ask;
        if (proceedsHeld < ask) revert Insufficient(ask, proceedsHeld);
        if (ask <= reward) revert Insufficient(reward, ask);
        proceedsHeld -= ask;
        _stage(a, Stage.Sold);
        guard.release(collection, 1, a.pricePaid);
        guard.out(collection, 1, ask);
        uint256 down = ask - reward;
        uint256 fee = bridgeFee(36);
        uint256 bridged = down > fee ? down - fee : 0;
        if (bridged != 0) {
            bytes memory data = abi.encodeCall(IHomeReceiverCredit.creditProceeds, (a.fundedBy));
            inbox.createRetryableTicket{value: down}(receiver, bridged, 2 * inbox.calculateRetryableSubmissionFee(data.length,
                block.basefee), receiver, receiver, L2_GAS_LIMIT, L2_MAX_FEE, data);
        } else {
            proceedsHeld += down;       // too small to bridge alone: it rides with the next settle
        }
        (bool ok,) = msg.sender.call{value: reward}("");
        if (!ok) revert TransferFailed();
        emit Rewarded(msg.sender, reward, this.settle.selector);
        emit Settled(acquisitionId, ask, bridged);
    }

    function listing(uint256 acquisitionId) external view returns (Listing memory) { return _listings[acquisitionId]; }
}
