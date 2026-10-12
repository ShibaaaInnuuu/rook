// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IRandomnessBeacon} from "../interfaces/IRandomnessBeacon.sol";
import {Wait} from "../lib/Wait.sol";
import {Policy, INftRegistry} from "./INft.sol";
import {NftVaultBase} from "./NftVaultBase.sol";

/// @title NftRemoteVault
/// @notice One NFT listing whose collection lives on ANOTHER EVM chain. This vault is its accounting on 4663: fees in, attributed per
/// coin; ETH out to one address only, the canister's (`buyer`, immutable), after a public hour and within a fresh cap; a receipt per
/// NFT with its purchase transaction; and the fate — raffle or burn — settled here and delivered there, by the canister, with the
/// delivering transaction's hash on the record. docs/v4/nft.md §6, §7.
///
/// The canister buys INTO its own address on the collection's chain — the same EVM address as `buyer` here — and holds there until
/// a raffle is won (it delivers to the winner's same address, or where the winner redirected it in the first hour) or it burns to
/// 0x…dEaD. AutoBuyerVault's shape, with an EVM receipt in place of a Bitcoin one. There is no withdraw function.
///
/// @dev What this does NOT protect against:
///  - a FALSE receipt or a false delivery record. This chain cannot read the other one. What is checkable here is: a token id is
///    receipted once, never more than the announced count, and ETH for NFTs that never came is back first. Whether a tx hash did what
///    it says is checkable by anyone with that chain's RPC. See docs/v4/nft.md §10.4.
///  - the canister being lost: `buyer` is immutable, so what is pulled and unsettled is written off, and what it holds on the other
///    chain stays there.
///  - a cap posted too high: the keeper's word, expiring after an hour on both clocks.
contract NftRemoteVault is NftVaultBase {
    address payable public immutable buyer;
    uint256 public immutable maxInFlight;

    uint256 public constant PUBLIC_WAIT = 1 hours;
    uint256 public constant SETTLE_DEADLINE = 3 days;
    uint256 public constant PUBLIC_WRITEOFF_AFTER = 7 days;
    uint256 public constant PUBLIC_CANCEL_AFTER = 7 days;
    uint256 public constant DELIVERY_DEADLINE = 7 days;
    uint256 public constant DESTINATION_CORRECTION = 1 hours;
    uint256 public constant MAX_OPEN = 3;
    uint256 public constant MAX_QUANTITY = 50;

    enum Status { None, Announced, Pulled, Settled, Cancelled, Returned, WrittenOff }

    struct Purchase {
        Status status;
        uint64 readyAt;
        uint64 readyAtBlock;
        uint64 deadline;
        uint64 deadlineBlock;
        uint256 maxEth;
        uint256 quantity;
        uint256 received;
        uint256 returned;
        string capRef;
    }

    Purchase[] private _purchases;
    uint256[] private _open;
    uint256 public reserved;            // announced, not yet pulled
    uint256 public inFlight;            // announced or pulled, and not yet settled, returned or written off
    uint256 public totalWrittenOff;
    mapping(uint256 tokenId => bool) public receipted;

    event Announced(uint256 indexed id, uint256 maxEth, uint256 quantity, uint64 readyAt, string capRef);
    event AnnouncementCancelled(uint256 indexed id);
    event Pulled(uint256 indexed id, uint256 amount, uint64 deadline);
    event Receipted(uint256 indexed id, uint256 indexed acquisitionId, uint256 indexed tokenId, bytes32 txHash);
    event Settled(uint256 indexed id, uint256 received);
    event Returned(uint256 indexed id, uint256 amount);
    event WrittenOff(uint256 indexed id, uint256 amount);
    event DestinationSet(uint256 indexed acquisitionId, address destination);
    event Delivered(uint256 indexed acquisitionId, address indexed to, bytes32 txHash);
    event Overdue(uint256 indexed acquisitionId);

    error NotBuyer();
    error NotWinner();
    error OverCap(uint256 maxEth, uint256 bound);
    error Insufficient(uint256 maxEth, uint256 available);
    error OverInFlight(uint256 maxEth, uint256 headroom);
    error TooManyOpen();
    error BlockedByOverdue(uint256 purchaseId);
    error WrongStatus();
    error NotOverdue();
    error BadQuantity();
    error AlreadyReceipted(uint256 tokenId);
    error ReturnFirst(uint256 atLeast);
    error DestinationLocked();
    error SendFailed();

    constructor(INftRegistry registry_, uint256 listingId_, uint64 chainId_, address collection_, Policy policy_,
        IRandomnessBeacon beacon_, address payable buyer_, uint256 maxInFlight_)
        NftVaultBase(registry_, listingId_, chainId_, collection_, policy_, beacon_)
    {
        buyer = buyer_;
        maxInFlight = maxInFlight_;
    }

    modifier onlyBuyer() { if (msg.sender != buyer) revert NotBuyer(); _; }

    // ---- buying ---------------------------------------------------------------------------------------------------------------

    function announce(uint256 maxEth, uint256 quantity, string calldata capRef) external onlyBuyer returns (uint256 id) {
        if (registry.isPaused(listingId)) revert ListingPaused();
        if (maxEth == 0 || quantity == 0 || quantity > MAX_QUANTITY) revert BadQuantity();
        if (!capFresh()) revert CapExpired();
        if (maxEth > capWei * quantity) revert OverCap(maxEth, capWei * quantity);
        (bool blocked, uint256 by) = isBlocked();
        if (blocked) revert BlockedByOverdue(by);
        if (_open.length >= MAX_OPEN) revert TooManyOpen();
        if (maxEth > attributed - reserved) revert Insufficient(maxEth, attributed - reserved);
        if (inFlight + maxEth > maxInFlight) revert OverInFlight(maxEth, maxInFlight - inFlight);
        id = _purchases.length;
        Purchase storage p = _purchases.push();
        p.status = Status.Announced;
        p.readyAt = uint64(block.timestamp + PUBLIC_WAIT);
        p.readyAtBlock = uint64(block.number + Wait.blocksFor(PUBLIC_WAIT));
        p.maxEth = maxEth;
        p.quantity = quantity;
        p.capRef = capRef;
        _open.push(id);
        reserved += maxEth;
        inFlight += maxEth;
        emit Announced(id, maxEth, quantity, p.readyAt, capRef);
    }

    /// @notice The owner may cancel an announcement at any time before it is pulled; anyone may, once it has sat unpulled for seven
    /// days past its ready hour. Nothing has left the vault either way.
    function cancelAnnouncement(uint256 id) external {
        Purchase storage p = _purchases[id];
        if (p.status != Status.Announced) revert WrongStatus();
        if (msg.sender != registry.owner() && !Wait.elapsed(uint64(p.readyAt + PUBLIC_CANCEL_AFTER),
            uint64(p.readyAtBlock + Wait.blocksFor(PUBLIC_CANCEL_AFTER)))) revert NotOwner();
        p.status = Status.Cancelled;
        reserved -= p.maxEth;
        inFlight -= p.maxEth;
        _close(id);
        emit AnnouncementCancelled(id);
    }

    /// @notice The ONLY function that sends ETH out, and only to `buyer`, an hour (both clocks) after the announcement.
    function pull(uint256 id) external onlyBuyer nonReentrant {
        Purchase storage p = _purchases[id];
        if (p.status != Status.Announced) revert WrongStatus();
        if (!Wait.elapsed(p.readyAt, p.readyAtBlock)) revert TooEarly();
        if (registry.isPaused(listingId)) revert ListingPaused();
        p.status = Status.Pulled;
        p.deadline = uint64(block.timestamp + SETTLE_DEADLINE);
        p.deadlineBlock = uint64(block.number + Wait.blocksFor(SETTLE_DEADLINE));
        reserved -= p.maxEth;
        _chargeFifo(id, p.maxEth);
        (bool ok,) = buyer.call{value: p.maxEth}("");
        if (!ok) revert SendFailed();
        emit Pulled(id, p.maxEth, p.deadline);
    }

    /// @notice One NFT bought on the collection's chain into `buyer`'s address there, by transaction `txHash`.
    function receipt(uint256 id, uint256 tokenId, bytes32 txHash) external onlyBuyer returns (uint256 acquisitionId) {
        Purchase storage p = _purchases[id];
        if (p.status != Status.Pulled) revert WrongStatus();
        if (p.received == p.quantity) revert BadQuantity();
        if (txHash == bytes32(0)) revert BadInput();
        if (receipted[tokenId]) revert AlreadyReceipted(tokenId);
        receipted[tokenId] = true;
        uint256 price = p.maxEth / p.quantity;
        acquisitionId = _record(tokenId, _attribute(id, price), price, id, txHash);
        emit Receipted(id, acquisitionId, tokenId, txHash);
        if (++p.received == p.quantity) _settled(id, p);
    }

    /// @notice Closes a purchase that bought fewer than announced, once the ETH for the missing ones is back (docs/phase3.md H2).
    function settleShort(uint256 id) external onlyBuyer {
        Purchase storage p = _purchases[id];
        if (p.status != Status.Pulled || p.received == 0) revert WrongStatus();
        uint256 mayKeep = (p.maxEth * p.received) / p.quantity;
        uint256 kept = p.maxEth - p.returned;
        if (kept > mayKeep) revert ReturnFirst(kept - mayKeep);
        _settled(id, p);
    }

    /// @notice Anyone returns ETH a purchase did not use; it goes back to the coins that paid, in proportion, as fresh money.
    function returnFunds(uint256 id) external payable nonReentrant {
        Purchase storage p = _purchases[id];
        if (p.status != Status.Pulled && p.status != Status.Settled && p.status != Status.WrittenOff) revert WrongStatus();
        if (msg.value == 0 || p.returned + msg.value > p.maxEth) revert BadQuantity();
        if (p.status == Status.Pulled) inFlight -= msg.value;
        p.returned += msg.value;
        _giveBack(id, p.maxEth, msg.value);
        emit Returned(id, msg.value);
        if (p.status == Status.Pulled && p.returned == p.maxEth && p.received == 0) {
            p.status = Status.Returned;
            _close(id);
        }
    }

    /// @notice A pulled purchase past its deadline with no settlement: the owner may write it off; seven days later, anyone may.
    function writeOff(uint256 id) external {
        Purchase storage p = _purchases[id];
        if (p.status != Status.Pulled) revert WrongStatus();
        if (!_overdue(id, 0)) revert NotOverdue();
        if (msg.sender != registry.owner() && !_overdue(id, PUBLIC_WRITEOFF_AFTER)) revert NotOwner();
        p.status = Status.WrittenOff;
        uint256 lost = p.maxEth - p.returned;
        inFlight -= lost;
        totalWrittenOff += lost;
        _close(id);
        emit WrittenOff(id, lost);
    }

    function _settled(uint256 id, Purchase storage p) private {
        inFlight -= p.maxEth - p.returned;
        p.status = Status.Settled;
        _close(id);
        emit Settled(id, p.received);
    }

    /// @dev AssetVault's clamped give-back: never gives a coin back more than it was charged, and always places the whole amount.
    function _giveBack(uint256 id, uint256 maxEth, uint256 amount) private {
        Charge[] storage cs = _charges[id];
        uint256 n = cs.length;
        uint256 given;
        for (uint256 i = 0; i < n; ++i) {
            uint256 room = cs[i].weiCharged - cs[i].givenBackWei;
            uint256 share = (amount * cs[i].weiCharged) / maxEth;
            if (share > room) share = room;
            if (share > amount - given) share = amount - given;
            if (share == 0) continue;
            cs[i].givenBackWei += share;
            given += share;
            _credit(cs[i].coin, share);
            chargedTo[cs[i].coin] -= share;
        }
        for (uint256 i = 0; i < n && given < amount; ++i) {
            uint256 room = cs[i].weiCharged - cs[i].givenBackWei;
            if (room == 0) continue;
            uint256 share = amount - given;
            if (share > room) share = room;
            cs[i].givenBackWei += share;
            given += share;
            _credit(cs[i].coin, share);
            chargedTo[cs[i].coin] -= share;
        }
    }

    function _close(uint256 id) private {
        uint256 n = _open.length;
        for (uint256 i = 0; i < n; ++i) {
            if (_open[i] == id) { _open[i] = _open[n - 1]; _open.pop(); return; }
        }
    }

    function _overdue(uint256 id, uint256 extra) private view returns (bool) {
        Purchase storage p = _purchases[id];
        return Wait.elapsed(uint64(uint256(p.deadline) + extra + 1), uint64(p.deadlineBlock + Wait.blocksFor(extra)));
    }

    // ---- the fate, delivered on the collection's chain ----------------------------------------------------------------------------

    /// @dev A win is recorded here; the NFT is on another chain. The canister delivers after the winner's hour to redirect.
    function _onWin(uint256, Lot storage, address) internal override {}

    /// @notice The winner may send the NFT somewhere other than their own address on the collection's chain — for a 4663 account
    /// that is a contract with no twin there — for one hour after the win, on both clocks. After that the address is what it is.
    function setDestination(uint256 acquisitionId, address to) external {
        Lot storage lot = _lot(acquisitionId);
        if (lot.stage != Stage.Won) revert WrongStage();
        if (msg.sender != lot.winner) revert NotWinner();
        if (to == address(0)) revert BadInput();
        if (_destinationLocked(lot)) revert DestinationLocked();
        lot.destination = to;
        emit DestinationSet(acquisitionId, to);
    }

    /// @notice The canister's record that it sent a won NFT to its destination, by `txHash` on the collection's chain.
    function recordDelivery(uint256 acquisitionId, bytes32 txHash) external onlyBuyer {
        Lot storage lot = _lot(acquisitionId);
        if (lot.stage != Stage.Won) revert WrongStage();
        if (!_destinationLocked(lot)) revert TooEarly();
        if (txHash == bytes32(0)) revert BadInput();
        lot.stage = Stage.Delivered;
        lot.settleTx = txHash;
        emit Delivered(acquisitionId, lot.destination, txHash);
    }

    /// @notice The canister's record that it burned a Burn listing's NFT to 0x…dEaD, by `txHash` on the collection's chain.
    function recordBurn(uint256 acquisitionId, bytes32 txHash) external onlyBuyer {
        if (txHash == bytes32(0)) revert BadInput();
        _markBurned(acquisitionId, txHash);
    }

    /// @notice Anyone: a won NFT not delivered, or a Burn listing's NFT not burned, seven days on (both clocks). On the record for ever.
    function flagOverdue(uint256 acquisitionId) external {
        _acq(acquisitionId);
        Lot storage lot = _lot(acquisitionId);
        bool pending = lot.stage == Stage.Won || (policy == Policy.Burn && lot.stage == Stage.Held);
        if (!pending || !Wait.elapsed(uint64(lot.at + DELIVERY_DEADLINE + 1), uint64(lot.atBlock + Wait.blocksFor(DELIVERY_DEADLINE)))) {
            revert NotOverdue();
        }
        lot.overdue = true;
        emit Overdue(acquisitionId);
    }

    function _destinationLocked(Lot storage lot) private view returns (bool) {
        return Wait.elapsed(uint64(lot.at + DESTINATION_CORRECTION), uint64(lot.atBlock + Wait.blocksFor(DESTINATION_CORRECTION)));
    }

    // ---- views ----------------------------------------------------------------------------------------------------------------

    function isBlocked() public view returns (bool, uint256) {
        for (uint256 i = 0; i < _open.length; ++i) {
            if (_purchases[_open[i]].status == Status.Pulled && _overdue(_open[i], 0)) return (true, _open[i]);
        }
        return (false, 0);
    }

    function purchase(uint256 id) external view returns (Purchase memory) { return _purchases[id]; }
    function purchaseCount() external view returns (uint256) { return _purchases.length; }
    function openPurchases() external view returns (uint256[] memory) { return _open; }
    function headroom() external view returns (uint256) { return maxInFlight - inFlight; }
}
