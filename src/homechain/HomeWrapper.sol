// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {IERC721} from "@openzeppelin/contracts/token/ERC721/IERC721.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IRandomnessBeacon} from "../interfaces/IRandomnessBeacon.sol";
import {IWrappedCollection} from "../interfaces/IWrappedCollection.sol";
import {Wait} from "../lib/Wait.sol";
import {ICryptoPunksMarket, IHomeGuard} from "./IHomeChain.sol";

/// @title HomeWrapper
/// @notice wX on the collection's own chain: one whole token is one NFT of `collection` held BY THIS CONTRACT. Anyone wraps an NFT
/// they own and gets `ONE_ITEM`; a whole token burns to a ticket pinned to a drand round that does not exist yet; the round picks
/// which NFT leaves, and it leaves in the same call that resolves the ticket. There is no custodian's word anywhere: the NFT is
/// here (`ownerOf(tokenId) == this`) or the wrap reverted. `NftWrapper`'s draw (stamps, strict order, redraw), minus every step that
/// existed only because a key on another chain had to deliver. docs/v6/home-chain-vault.md §4.
/// @dev What this does NOT protect against:
///  - a collection that refuses a transfer to the ticket's destination (a block list, a transfer validator): the ticket stays
///    `Resolved`, anyone may retry `deliver`, and after DELIVERY_DEADLINE on both clocks anyone may `refund` it — a re-roll for that
///    redeemer only, as docs/v4/nft-wrapper.md §9.6;
///  - a collection whose admin can move tokens it does not own (an upgradeable proxy): choose plain ERC-721s;
///  - value spread inside the collection: a token is worth about the AVERAGE held item;
///  - a beacon that never proves a round: that ticket and every one behind it wait.
contract HomeWrapper is ERC20, IWrappedCollection, ReentrancyGuard {
    IRandomnessBeacon public immutable beacon;
    address public immutable collection;
    /// @notice The collection is CryptoPunksMarket: wrapped through `offerPunkForSaleToAddress(id, 0, this)` + `buyPunk`, delivered
    /// by `transferPunk`.
    bool public immutable isPunks;
    IHomeGuard public immutable guard;
    /// @notice The HomeVault of this collection (or zero): its wraps were already admitted by the guard as purchases, so they are
    /// counted once, not twice.
    address public immutable vault;
    uint256 public immutable parentBlockSeconds;

    uint256 public constant ONE_ITEM = 1e18;
    uint64 public constant PIN_AHEAD = 20;
    uint256 public constant REDRAW_AFTER = 24 hours;
    uint256 public constant DELIVERY_DEADLINE = 7 days;
    address public constant DEAD = 0x000000000000000000000000000000000000dEaD;

    /// @dev `NftWrapper.Stage`, same values, same order.
    enum Stage { None, Pinned, Resolved, Delivered, Refunded }

    struct Ticket {
        Stage stage;
        address owner;
        uint64 round;
        uint64 pinnedAt;
        uint64 pinnedAtBlock;
        uint64 deliverBy;
        uint64 deliverByBlock;
        uint256 itemId;
        address destination;
    }
    struct Item { uint256 tokenId; address from; }
    struct Pending { uint256 itemId; uint256 stamp; }

    /// @notice `itemId + 1` of the token id while it is held here; zero when it is not.
    mapping(uint256 tokenId => uint256) public itemOf;
    uint256 public held;
    uint256 public openTickets;
    uint256 public burned;
    uint256 public nextToResolve;
    uint256 public pendingHead;

    Item[] private _items;
    uint256[] private _free;
    Pending[] private _pending;
    Ticket[] private _tickets;

    event RedeemedToRelist(uint256 indexed ticketId);

    event Wrapped(uint256 indexed itemId, uint256 indexed tokenId, address indexed from, address to);
    event Redeemed(uint256 indexed ticketId, address indexed owner, uint64 round, address destination);
    event Repinned(uint256 indexed ticketId, uint64 round);
    event Resolved(uint256 indexed ticketId, uint256 indexed itemId, uint256 indexed tokenId, bool delivered);
    event Delivered(uint256 indexed ticketId, uint256 indexed tokenId, address destination);
    event Refunded(uint256 indexed ticketId, uint256 indexed itemId, address indexed owner);

    error BadInput();
    error BadDestination();
    error NotArrived();
    error WrongStage();
    error OutOfOrder(uint256 next);
    error TooEarly();
    error NothingToDraw();
    error DeliveryFailed();

    constructor(IRandomnessBeacon beacon_, address collection_, bool isPunks_, IHomeGuard guard_, address vault_,
        uint256 parentBlockSeconds_, string memory name_, string memory symbol_) ERC20(name_, symbol_)
    {
        if (address(beacon_) == address(0) || collection_ == address(0) || address(guard_) == address(0)) revert BadInput();
        if (collection_.code.length == 0) revert BadInput();
        Wait.checkBlockSeconds(parentBlockSeconds_);
        beacon = beacon_;
        collection = collection_;
        isPunks = isPunks_;
        guard = guard_;
        vault = vault_;
        parentBlockSeconds = parentBlockSeconds_;
    }

    /// @notice The NFTs sit here.
    function custodian() external view returns (address) { return address(this); }

    // ---- in: anyone, their own NFT ----------------------------------------------------------------------------------------------

    /// @notice Moves `tokenId` from the caller into this contract and mints `ONE_ITEM` to `to`. ERC-721: the caller approved this
    /// contract. Punks: the caller first `offerPunkForSaleToAddress(tokenId, 0, wrapper)`.
    function wrap(uint256 tokenId, address to) external nonReentrant {
        if (to == address(0)) revert BadInput();
        if (itemOf[tokenId] != 0) revert WrongStage();
        if (msg.sender != vault) guard.admit(collection, 1, 0);
        if (isPunks) {
            ICryptoPunksMarket p = ICryptoPunksMarket(collection);
            (bool forSale,, address seller, uint256 minValue, address onlySellTo) = p.punksOfferedForSale(tokenId);
            if (!forSale || seller != msg.sender || minValue != 0 || onlySellTo != address(this)) revert NotArrived();
            p.buyPunk{value: 0}(tokenId);
            if (p.punkIndexToAddress(tokenId) != address(this)) revert NotArrived();
        } else {
            IERC721 c = IERC721(collection);
            c.transferFrom(msg.sender, address(this), tokenId);
            if (c.ownerOf(tokenId) != address(this)) revert NotArrived();
        }
        uint256 itemId = _items.length;
        _items.push(Item(tokenId, msg.sender));
        itemOf[tokenId] = itemId + 1;
        held++;
        _pending.push(Pending(itemId, _tickets.length));
        _mint(to, ONE_ITEM);
        emit Wrapped(itemId, tokenId, msg.sender, to);
    }

    // ---- out: nobody chooses the NFT ---------------------------------------------------------------------------------------------

    /// @notice Burn one whole token for one random NFT, delivered to `destination` on this chain. Fixed at the burn.
    function redeem(address destination) external returns (uint256) {
        if (destination == address(0) || destination == address(this)) revert BadDestination();
        return _redeem(destination);
    }

    /// @notice Burn one whole token for one random NFT, sent to `DEAD`: out of circulation for good.
    function redeemToBurn() external returns (uint256) {
        return _redeem(DEAD);
    }

    /// @notice Burn one whole token for one random NFT that goes to the home-chain vault and is listed for sale from there (the
    /// YOYO policy's exit, docs/v5/policies-and-fees.md §1.3; docs/v6/home-chain-vault.md §5): its delivery is the listing.
    function redeemToRelist() external returns (uint256 id) {
        id = _redeem(vault);
        emit RedeemedToRelist(id);
    }

    function _redeem(address destination) private returns (uint256 id) {
        _burn(msg.sender, ONE_ITEM);
        id = _tickets.length;
        Ticket storage t = _tickets.push();
        t.owner = msg.sender;
        t.destination = destination;
        openTickets++;
        _pin(t);
        emit Redeemed(id, msg.sender, t.round, destination);
    }

    /// @notice Anyone, if a pinned round is still unproven a day later on both clocks.
    function redraw(uint256 id) external {
        Ticket storage t = _tickets[id];
        if (t.stage != Stage.Pinned) revert WrongStage();
        if (!Wait.elapsed(uint64(t.pinnedAt + REDRAW_AFTER), uint64(t.pinnedAtBlock + Wait.blocksFor(REDRAW_AFTER, parentBlockSeconds))))
            revert TooEarly();
        if (beacon.randomnessOf(t.round) != bytes32(0)) revert TooEarly();
        _pin(t);
        emit Repinned(id, t.round);
    }

    function _pin(Ticket storage t) private {
        t.stage = Stage.Pinned;
        t.round = beacon.roundAt(block.timestamp) + PIN_AHEAD;
        t.pinnedAt = uint64(block.timestamp);
        t.pinnedAtBlock = uint64(block.number);
    }

    /// @notice Anyone, strictly in ticket order: proves the round, draws among the items that were free before this burn, and moves
    /// the NFT to the ticket's destination in this same call. If the collection refuses the transfer the ticket stays `Resolved`
    /// for `deliver` or, after the deadline, `refund`.
    function resolve(uint256 id, bytes calldata signature) external nonReentrant {
        if (id != nextToResolve) revert OutOfOrder(nextToResolve);
        Ticket storage t = _tickets[id];
        if (t.stage != Stage.Pinned) revert WrongStage();
        bytes32 randomness = beacon.randomnessOf(t.round);
        if (randomness == bytes32(0)) randomness = beacon.prove(t.round, signature);

        uint256 head = pendingHead;
        uint256 n = _pending.length;
        while (head < n && _pending[head].stamp <= id) _free.push(_pending[head++].itemId);
        pendingHead = head;
        uint256 size = _free.length;
        if (size == 0) revert NothingToDraw();

        uint256 i = uint256(keccak256(abi.encode(randomness, address(this), id))) % size;
        uint256 itemId = _free[i];
        _free[i] = _free[size - 1];
        _free.pop();

        t.itemId = itemId;
        t.stage = Stage.Resolved;
        t.deliverBy = uint64(block.timestamp + DELIVERY_DEADLINE);
        t.deliverByBlock = uint64(block.number + Wait.blocksFor(DELIVERY_DEADLINE, parentBlockSeconds));
        nextToResolve = id + 1;
        bool delivered = _tryDeliver(id, t);
        emit Resolved(id, itemId, _items[itemId].tokenId, delivered);
    }

    /// @notice Anyone retries a delivery the collection refused at resolution. Allowed past the deadline while nobody has refunded.
    function deliver(uint256 id) external nonReentrant {
        Ticket storage t = _tickets[id];
        if (t.stage != Stage.Resolved) revert WrongStage();
        if (!_tryDeliver(id, t)) revert DeliveryFailed();
    }

    /// @dev The transfer, counted out by the guard first (over the day's limit it reverts `Wait`, and so does the whole resolve — the
    /// ticket keeps its place and is resolved later). `transferFrom`, never the safe variant: a destination cannot refuse.
    function _tryDeliver(uint256 id, Ticket storage t) private returns (bool ok) {
        uint256 tokenId = _items[t.itemId].tokenId;
        guard.out(collection, 1, 0);
        if (isPunks) {
            try ICryptoPunksMarket(collection).transferPunk(t.destination, tokenId) { ok = true; } catch {}
            if (ok && ICryptoPunksMarket(collection).punkIndexToAddress(tokenId) != t.destination) ok = false;
        } else {
            try IERC721(collection).transferFrom(address(this), t.destination, tokenId) { ok = true; } catch {}
            if (ok && IERC721(collection).ownerOf(tokenId) != t.destination) ok = false;
        }
        if (!ok) return false;
        t.stage = Stage.Delivered;
        delete itemOf[tokenId];
        held--;
        openTickets--;
        guard.release(collection, 1, 0);
        if (t.destination == DEAD) burned++;
        emit Delivered(id, tokenId, t.destination);
    }

    /// @notice Anyone, once a resolved ticket is past its deadline on both clocks with no delivery: the NFT goes back to the set and
    /// one whole token is minted back to the ticket's owner.
    function refund(uint256 id) external {
        Ticket storage t = _tickets[id];
        if (t.stage != Stage.Resolved) revert WrongStage();
        if (!Wait.elapsed(t.deliverBy, t.deliverByBlock)) revert TooEarly();
        t.stage = Stage.Refunded;
        openTickets--;
        _pending.push(Pending(t.itemId, _tickets.length));
        _mint(t.owner, ONE_ITEM);
        emit Refunded(id, t.itemId, t.owner);
    }

    // ---- views ---------------------------------------------------------------------------------------------------------------

    function refunded(uint256 id) external view returns (bool) { return _tickets[id].stage == Stage.Refunded; }
    function ticket(uint256 id) external view returns (Ticket memory) { return _tickets[id]; }
    function ticketCount() external view returns (uint256) { return _tickets.length; }
    function item(uint256 itemId) external view returns (Item memory) { return _items[itemId]; }
    function itemCount() external view returns (uint256) { return _items.length; }
    function freeCount() external view returns (uint256) { return _free.length; }
    function freeAt(uint256 i) external view returns (uint256) { return _free[i]; }
    function pendingCount() external view returns (uint256) { return _pending.length - pendingHead; }
    function pendingAt(uint256 i) external view returns (uint256 itemId, uint256 stamp) {
        Pending storage p = _pending[pendingHead + i];
        return (p.itemId, p.stamp);
    }
}
