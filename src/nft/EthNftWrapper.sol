// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {IRandomnessBeacon} from "../interfaces/IRandomnessBeacon.sol";
import {IFloorWrapped} from "../interfaces/IFloorWrapped.sol";
import {Wait} from "../lib/Wait.sol";

/// @notice The one read the wrapper makes of the listing it names: whether it is paused (ListingRegistry).
interface IListingPause {
    function isPaused(uint256 id) external view returns (bool);
}

/// @notice The one read `wireFloorModule` makes of the module it is handed (B-05).
interface IModuleWrapped {
    function wrapped() external view returns (address);
}

/// @title EthNftWrapper
/// @notice One whole token is one random ERC-721 of one collection on another EVM chain (Ethereum for Phase 1), held at the
/// canister's own threshold-ECDSA address there (docs/v7/phase1-nft.md §3.1). The canister mints `ONE_ITEM` per NFT that entered
/// its custody past that chain's finality (a Seaport purchase, or a holder's claimed deposit), recording the token id and the
/// transaction that brought it in. A whole token burns to a ticket pinned to a future drand round, which is final; the round picks
/// which NFT leaves; the canister records the delivering transaction; a resolved ticket nobody delivers is refunded only once
/// the canister attests its delivery can no longer confirm (`markUndeliverable`) and `REFUND_GRACE` has passed, or
/// `LATE_DELIVERY_WINDOW` after its deadline if the canister never noted a delivery of it mined (`noteSent`). `Wrapper`'s
/// redemption machine after review rounds 1–4 (C-02: no redraw; E-03: a pinned ticket refunds after `PIN_TIMEOUT`; D M-1 /
/// review2 N-2: the gated refund; B-05: the module must name this wrapper), re-keyed from inscriptions to token ids. Rentals are
/// not supported.
/// @dev What this does NOT protect against, because nothing on this chain can:
///  - an NFT that leaves custody without this chain knowing (a key compromise, a collection whose admin can move tokens), or a
///    mint against an NFT the canister does not hold. Anyone can reconcile `ownerOf(item(i).tokenId) == custodian` on `chainId`.
///  - a delivery that confirms after the custodian's false `markUndeliverable`, or one kept unmined for `LATE_DELIVERY_WINDOW`.
///    A delivery the custodian noted mined (`noteSent`) is never refunded by the late window, only after `markUndeliverable`
///    (audit 2026-09-27-nft D3-H-1: a delivery mined but never recorded, the redeemer holding the NFT). The late window is for a
///    ticket whose delivery was never noted mined: a custodian that is gone before it sent. A custodian gone after `noteSent`
///    leaves that ticket open for good: its NFT was sent, its unit is not minted back.
/// @dev The floor module (docs/v5/floor-module.md): one address, wired once before anything was ever minted, may mint and burn
/// ETH-backed units. The ledger: `totalSupply == (held − openTickets) × ONE_ITEM + ethBacked`, and a redemption needs an
/// unpromised NFT: `held ≥ openTickets + 1`.
contract EthNftWrapper is ERC20, IFloorWrapped {
    /// @notice The ListingRegistry whose listing `listingId` pauses mints here (the same incident switch as every vault's).
    IListingPause public immutable registry;
    uint256 public immutable listingId;
    IRandomnessBeacon public immutable beacon;
    /// @notice The EIP-155 chain the collection lives on. Never this chain.
    uint64 public immutable chainId;
    /// @notice The ERC-721 contract on `chainId`.
    address public immutable collection;
    /// @notice The canister's EVM address: the only minter, the only recorder of deliveries, and (the same 20 bytes, one threshold
    /// key) the holder of every wrapped NFT on `chainId`. Code-less; no setter.
    address public immutable custodian;
    /// @notice The parent chain's block time on this deployment's chain (lib/Wait.sol).
    uint256 public immutable parentBlockSeconds;
    /// @notice Who deployed this wrapper: the only address that may wire the floor module and set the first mint limit.
    address public immutable deployer;

    uint256 public constant ONE_ITEM = 1e18;
    uint64 public constant PIN_AHEAD = 20;
    /// @notice A ticket still unresolved this long after its pin, on both clocks, may be refunded. The round is never replaced.
    uint256 public constant PIN_TIMEOUT = 7 days;
    uint256 public constant DELIVERY_DEADLINE = 7 days;
    /// @notice With the custodian's `markUndeliverable`, a resolved ticket is refundable this long after `deliverBy`.
    uint256 public constant REFUND_GRACE = 72 hours;
    /// @notice Without it, this long after `deliverBy`.
    uint256 public constant LATE_DELIVERY_WINDOW = 30 days;
    /// @notice Where a burn is delivered on `chainId`: nobody has its key.
    address public constant DEAD = 0x000000000000000000000000000000000000dEaD;

    /// @dev `Wrapper.Stage`, in the same order, so a consumer reading either as a uint8 reads the same thing.
    enum Stage { None, Pinned, Resolved, Delivered, Refunded }

    struct Ticket {
        Stage stage;
        address owner;
        uint64 round;
        uint64 pinnedAt;
        uint64 pinnedAtBlock;
        uint64 deliverBy;
        uint64 deliverByBlock;
        uint256 itemId;           // the NFT, once resolved
        bytes32 deliveryTx;
        address destination;      // on `chainId`; fixed at the burn
    }

    /// @notice One NFT's stay in custody: which token, and the transaction on `chainId` that brought it in.
    struct Item {
        uint256 tokenId;
        bytes32 sourceTx;
        bool deposit;             // a holder's deposit, not a purchase
    }

    /// @dev An item waiting to become drawable, stamped with how many tickets existed when it became free.
    struct Pending { uint256 itemId; uint256 stamp; }

    /// @notice `itemId + 1` of the token id while it is held here; zero when it is not. One token id, one held item, at once.
    mapping(uint256 tokenId => uint256) public itemOf;
    /// @notice `keccak256(tokenId, txHash)` pairs that have minted: once, ever.
    mapping(bytes32 source => bool) public sourceUsed;
    /// @notice The custodian's word that no version of a resolved ticket's delivery can confirm any more.
    mapping(uint256 ticketId => bool) public undeliverable;
    /// @notice The custodian's word that a transaction delivering this resolved ticket's NFT mined on `chainId` (before its
    /// finality): that transaction's hash, the last one noted. Nonzero, the ticket is refundable only with `markUndeliverable`.
    mapping(uint256 ticketId => bytes32) public sentTx;
    /// @notice Items minted and not delivered: in the free set, pending, or on a resolved ticket.
    uint256 public held;
    /// @notice Burns neither delivered nor refunded.
    uint256 public openTickets;
    /// @notice Items delivered to `DEAD`.
    uint256 public burned;
    uint256 public nextToResolve;
    uint256 public pendingHead;

    Item[] private _items;
    uint256[] private _free;
    Pending[] private _pending;
    Ticket[] private _tickets;

    // ---- the floor module ----
    address public floorModule;
    uint256 public ethBacked;

    // ---- H11: the custodian's mint rate (docs/v5/custody-hardening.md) ----
    uint256 public constant MINT_WINDOW = 24 hours;
    uint256 public constant DEFAULT_MINTS_PER_DAY = 20;
    uint256 public constant LIMIT_DELAY = 7 days;
    uint256 public maxMintsPerDay = DEFAULT_MINTS_PER_DAY;
    uint256 public itemMints;
    mapping(uint256 => uint256) private _mintedAt;
    uint256 public pendingMintLimit;
    uint64 public pendingLimitAt;
    uint64 public pendingLimitAtBlock;

    event Wrapped(uint256 indexed itemId, uint256 indexed tokenId, bytes32 purchaseTx, address indexed to);
    event Deposited(uint256 indexed itemId, uint256 indexed tokenId, bytes32 depositTx, address indexed to);
    event Redeemed(uint256 indexed ticketId, address indexed owner, uint64 round, address destination);
    event Resolved(uint256 indexed ticketId, uint256 indexed itemId, uint256 indexed tokenId, uint64 deliverBy);
    event Delivered(uint256 indexed ticketId, uint256 indexed tokenId, address destination, bytes32 txHash);
    event DeliverySent(uint256 indexed ticketId, uint256 indexed tokenId, bytes32 txHash);
    event Undeliverable(uint256 indexed ticketId);
    event Refunded(uint256 indexed ticketId, uint256 indexed itemId, address indexed owner);
    event FloorModuleWired(address module);
    event MintLimitProposed(uint256 limit, uint64 readyAt);
    event MintLimitSet(uint256 limit);

    error NotCustodian();
    error BadInput();
    error BadDestination();
    error ListingPaused();
    error AlreadyHeld();
    error AlreadyMinted();
    error WrongStage();
    error OutOfOrder(uint256 next);
    error TooEarly();
    error NothingToDraw();
    error NotDeployer();
    error AlreadyWired();
    error NotFresh();
    error NotModule();
    error NotItsModule();
    error NoItem();
    error MintRateLimited();
    error NotUndeliverable();
    error NoRentals();
    error NotSupported();
    error DeliverySentNotRecorded();

    constructor(IListingPause registry_, uint256 listingId_, IRandomnessBeacon beacon_, uint64 chainId_, address collection_,
        address custodian_, string memory name_, string memory symbol_, uint256 parentBlockSeconds_) ERC20(name_, symbol_)
    {
        Wait.checkBlockSeconds(parentBlockSeconds_);
        if (address(registry_) == address(0) || address(beacon_) == address(0) || collection_ == address(0) || custodian_ == address(0)) {
            revert BadInput();
        }
        if (chainId_ == 0 || chainId_ == block.chainid) revert BadInput();
        if (custodian_.code.length != 0) revert BadInput();
        registry = registry_;
        listingId = listingId_;
        beacon = beacon_;
        chainId = chainId_;
        collection = collection_;
        custodian = custodian_;
        parentBlockSeconds = parentBlockSeconds_;
        deployer = msg.sender;
    }

    // ---- H11 -------------------------------------------------------------------------------------------------------------------

    /// @notice Once, by the deployer, before anything was ever minted: the deployment's daily limit.
    function initMintLimit(uint256 limit) external {
        if (msg.sender != deployer) revert NotDeployer();
        if (limit == 0) revert BadInput();
        if (itemMints != 0 || totalSupply() != 0 || held != 0) revert NotFresh();
        maxMintsPerDay = limit;
        emit MintLimitSet(limit);
    }

    /// @notice The deployer: lower the daily limit at once, or propose a raise anyone may accept after `LIMIT_DELAY` on both clocks.
    function proposeMintLimit(uint256 limit) external {
        if (msg.sender != deployer) revert NotDeployer();
        if (limit == 0) revert BadInput();
        if (limit <= maxMintsPerDay) {
            maxMintsPerDay = limit;
            pendingMintLimit = 0;
            emit MintLimitSet(limit);
            return;
        }
        pendingMintLimit = limit;
        pendingLimitAt = uint64(block.timestamp + LIMIT_DELAY);
        pendingLimitAtBlock = uint64(block.number + Wait.blocksFor(LIMIT_DELAY, parentBlockSeconds));
        emit MintLimitProposed(limit, pendingLimitAt);
    }

    function acceptMintLimit() external {
        uint256 limit = pendingMintLimit;
        if (limit == 0) revert WrongStage();
        if (!Wait.elapsed(pendingLimitAt, pendingLimitAtBlock)) revert TooEarly();
        pendingMintLimit = 0;
        maxMintsPerDay = limit;
        emit MintLimitSet(limit);
    }

    /// @notice Whether an item may be minted now: the `maxMintsPerDay`-th last mint is a day old on both clocks.
    function mintRoom() public view returns (bool) {
        uint256 n = itemMints;
        uint256 m = maxMintsPerDay;
        if (n < m) return true;
        uint256 st = _mintedAt[n - m];
        return Wait.elapsed(uint64(st >> 64) + uint64(MINT_WINDOW), uint64(st) + uint64(Wait.blocksFor(MINT_WINDOW, parentBlockSeconds)));
    }

    function _countMint() private {
        if (!mintRoom()) revert MintRateLimited();
        _mintedAt[itemMints++] = (block.timestamp << 64) | block.number;
    }

    // ---- the floor module ------------------------------------------------------------------------------------------------------

    /// @notice Once, by the deployer, before this wrapper has ever minted or burned anything: the one floor module, which must be
    /// a module built for this wrapper (B-05).
    function wireFloorModule(address module) external {
        if (msg.sender != deployer) revert NotDeployer();
        if (floorModule != address(0)) revert AlreadyWired();
        if (module == address(0) || module.code.length == 0) revert BadInput();
        if (IModuleWrapped(module).wrapped() != address(this)) revert NotItsModule();
        if (totalSupply() != 0 || held != 0 || _tickets.length != 0) revert NotFresh();
        floorModule = module;
        emit FloorModuleWired(module);
    }

    modifier onlyModule() {
        if (msg.sender != floorModule || msg.sender == address(0)) revert NotModule();
        _;
    }

    function moduleMint(address to, uint256 amount) external onlyModule {
        if (to == address(0) || amount == 0) revert BadInput();
        ethBacked += amount;
        _mint(to, amount);
    }

    function moduleBurn(uint256 amount) external onlyModule {
        if (amount == 0 || amount > ethBacked) revert BadInput();
        ethBacked -= amount;
        _burn(msg.sender, amount);
    }

    /// @notice Rentals are not supported for an Ethereum collection in Phase 1 (docs/v7/phase1-nft.md §3.1).
    function admitRental(bytes calldata, bytes calldata) external pure returns (uint256, bytes32) {
        revert NoRentals();
    }

    function returnRental(uint256) external pure {
        revert NoRentals();
    }

    function rentalState(uint256) external pure returns (RentalState) { return RentalState.None; }
    function openReturns() external pure returns (uint256) { return 0; }
    function rentedHeld() external pure returns (uint256) { return 0; }

    // ---- minting: the custodian only ---------------------------------------------------------------------------------------------

    /// @notice The canister, for an NFT it bought into its own address on `chainId` by `purchaseTx` and saw there past that chain's
    /// finality: one `ONE_ITEM` to `to` (the floor module whose advance paid for it).
    function mint(uint256 tokenId, bytes32 purchaseTx, address to) external returns (uint256 itemId) {
        itemId = _admit(tokenId, purchaseTx, to, false);
        emit Wrapped(itemId, tokenId, purchaseTx, to);
    }

    /// @notice The canister, for an NFT its holder sent it by `depositTx` with the 4663 recipient `to` in the transfer's own data,
    /// seen past finality (docs/v7/phase1-nft.md §2.4).
    function mintDeposit(uint256 tokenId, bytes32 depositTx, address to) external returns (uint256 itemId) {
        itemId = _admit(tokenId, depositTx, to, true);
        emit Deposited(itemId, tokenId, depositTx, to);
    }

    function _admit(uint256 tokenId, bytes32 txHash, address to, bool deposit) private returns (uint256 itemId) {
        if (msg.sender != custodian) revert NotCustodian();
        if (registry.isPaused(listingId)) revert ListingPaused();
        if (to == address(0) || txHash == bytes32(0)) revert BadInput();
        if (itemOf[tokenId] != 0) revert AlreadyHeld();
        bytes32 source = keccak256(abi.encode(tokenId, txHash));
        if (sourceUsed[source]) revert AlreadyMinted();
        _countMint();
        sourceUsed[source] = true;
        itemId = _items.length;
        _items.push(Item(tokenId, txHash, deposit));
        itemOf[tokenId] = itemId + 1;
        held++;
        _pending.push(Pending(itemId, _tickets.length));
        _mint(to, ONE_ITEM);
    }

    // ---- redeeming: nobody chooses the NFT ---------------------------------------------------------------------------------------

    /// @notice Burn one whole token for one random NFT, delivered to `destination` on `chainId`. The round that decides which NFT
    /// is pinned now and does not exist yet; the destination cannot be changed afterwards.
    function redeem(address destination) external returns (uint256) {
        if (destination == address(0) || destination == custodian) revert BadDestination();
        return _redeem(destination);
    }

    /// @notice Burn one whole token for one random NFT, delivered to `DEAD` on `chainId`: out of circulation for good.
    function redeemToBurn() external returns (uint256) {
        return _redeem(DEAD);
    }

    /// @notice IWrappedCollection's YOYO exit, refused (audit 2026-09-27-nft, A-M-2). A relist ticket names the custodian, which
    /// never delivers it, so it only ever refunded: a free, repeatable option that kept every held NFT promised to its burner's own
    /// tickets and closed redemption to everyone else. YOYO is OFF for Ethereum collections and the canister never lists an NFT;
    /// if a relist exit is ever wanted, the custodian lists, and the ticket is not refundable to its burner.
    function redeemToRelist() external pure returns (uint256) {
        revert NotSupported();
    }

    function _redeem(address destination) private returns (uint256 id) {
        if (held < openTickets + 1) revert NoItem();
        _burn(msg.sender, ONE_ITEM);
        id = _tickets.length;
        Ticket storage t = _tickets.push();
        t.owner = msg.sender;
        t.destination = destination;
        openTickets++;
        t.stage = Stage.Pinned;
        t.round = beacon.roundAt(block.timestamp) + PIN_AHEAD;
        t.pinnedAt = uint64(block.timestamp);
        t.pinnedAtBlock = uint64(block.number);
        emit Redeemed(id, msg.sender, t.round, destination);
    }

    /// @notice Anyone, strictly in ticket order: proves the round (or reads it, if already proven) and draws the NFT among those
    /// that were already free when this ticket was burned.
    function resolve(uint256 id, bytes calldata signature) external {
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
        if (size == 0) revert NothingToDraw();   // unreachable while the ledger holds

        uint256 i = uint256(keccak256(abi.encode(randomness, address(this), id))) % size;
        uint256 itemId = _free[i];
        _free[i] = _free[size - 1];
        _free.pop();

        t.itemId = itemId;
        t.stage = Stage.Resolved;
        t.deliverBy = uint64(block.timestamp + DELIVERY_DEADLINE);
        t.deliverByBlock = uint64(block.number + Wait.blocksFor(DELIVERY_DEADLINE, parentBlockSeconds));
        nextToResolve = id + 1;
        _skipRefunded();
        emit Resolved(id, itemId, _items[itemId].tokenId, t.deliverBy);
    }

    function _skipRefunded() private {
        uint256 n = nextToResolve;
        uint256 len = _tickets.length;
        while (n < len && _tickets[n].stage == Stage.Refunded) ++n;
        nextToResolve = n;
    }

    /// @notice The canister records the transaction on `chainId` that sent a resolved ticket's NFT to its destination. Allowed past
    /// the deadline for as long as nobody has refunded.
    function recordDelivery(uint256 id, bytes32 txHash) external {
        if (msg.sender != custodian) revert NotCustodian();
        Ticket storage t = _tickets[id];
        if (t.stage != Stage.Resolved) revert WrongStage();
        if (txHash == bytes32(0)) revert BadInput();
        t.stage = Stage.Delivered;
        t.deliveryTx = txHash;
        uint256 tokenId = _items[t.itemId].tokenId;
        delete itemOf[tokenId];
        held--;
        openTickets--;
        if (t.destination == DEAD) burned++;
        emit Delivered(id, tokenId, t.destination, txHash);
    }

    /// @notice The custodian: the transaction `txHash` delivering this resolved ticket's NFT mined on `chainId` (not yet final).
    /// From then on the late window never refunds the ticket; only `recordDelivery` or `markUndeliverable` settle it. A later
    /// call replaces the hash (a replacement transaction). Changes no stage and no ledger number.
    function noteSent(uint256 id, bytes32 txHash) external {
        if (msg.sender != custodian) revert NotCustodian();
        Ticket storage t = _tickets[id];
        if (t.stage != Stage.Resolved) revert WrongStage();
        if (txHash == bytes32(0)) revert BadInput();
        sentTx[id] = txHash;
        emit DeliverySent(id, _items[t.itemId].tokenId, txHash);
    }

    /// @notice The custodian: no version of this resolved ticket's delivery can confirm any more (nothing was ever signed, or the
    /// transfer reverted and is final, or its nonce went to our own final cancel). Only then is the ticket refundable at
    /// `deliverBy + REFUND_GRACE`.
    function markUndeliverable(uint256 id) external {
        if (msg.sender != custodian) revert NotCustodian();
        if (_tickets[id].stage != Stage.Resolved) revert WrongStage();
        undeliverable[id] = true;
        emit Undeliverable(id);
    }

    /// @notice Anyone. A resolved ticket past `deliverBy + REFUND_GRACE` with `markUndeliverable`, or past
    /// `deliverBy + LATE_DELIVERY_WINDOW` without it and without `noteSent`, on both clocks, with no delivery recorded: the NFT
    /// goes back to the set and one whole token is minted back to the ticket's owner. A ticket noted sent and not marked never
    /// refunds (`DeliverySentNotRecorded`): its NFT may be at its destination. Also a ticket still pinned `PIN_TIMEOUT` after its
    /// pin: no NFT was drawn.
    function refund(uint256 id) external {
        Ticket storage t = _tickets[id];
        if (t.stage == Stage.Pinned) {
            if (!Wait.elapsed(uint64(t.pinnedAt + PIN_TIMEOUT), uint64(t.pinnedAtBlock + Wait.blocksFor(PIN_TIMEOUT, parentBlockSeconds)))) {
                revert TooEarly();
            }
            t.stage = Stage.Refunded;
            openTickets--;
            _skipRefunded();
            _mint(t.owner, ONE_ITEM);
            emit Refunded(id, type(uint256).max, t.owner);
            return;
        }
        if (t.stage != Stage.Resolved) revert WrongStage();
        if (!Wait.elapsed(uint64(t.deliverBy + REFUND_GRACE),
            uint64(t.deliverByBlock + Wait.blocksFor(REFUND_GRACE, parentBlockSeconds)))) revert TooEarly();
        if (!undeliverable[id]) {
            if (sentTx[id] != bytes32(0)) revert DeliverySentNotRecorded();
            if (!Wait.elapsed(uint64(t.deliverBy + LATE_DELIVERY_WINDOW),
                uint64(t.deliverByBlock + Wait.blocksFor(LATE_DELIVERY_WINDOW, parentBlockSeconds)))) revert NotUndeliverable();
        }
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
