// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {IRandomnessBeacon} from "../interfaces/IRandomnessBeacon.sol";
import {IWrappedCollection} from "../interfaces/IWrappedCollection.sol";
import {IFloorWrapped, IFloorRentals} from "../interfaces/IFloorWrapped.sol";
import {Wait} from "../lib/Wait.sol";

/// @title NftWrapper
/// @notice One whole token is one random NFT of one collection on another EVM chain, held at the canister's own address there.
/// The canister mints `ONE_ITEM` per NFT that entered its custody (bought through Seaport, or deposited by a holder), recording the
/// token id and the transaction that brought it in; a whole token burns to a ticket pinned to a future drand round; the round
/// picks which NFT leaves; the canister records the delivering transaction; and a ticket nobody delivers in time is refunded,
/// NFT back to the set and token back to its owner. `Wrapper`'s machinery, re-keyed from inscriptions to token ids.
/// docs/v4/nft-wrapper.md.
/// @dev What this does NOT protect against, because nothing on this chain can:
///  - an NFT that leaves custody without this chain knowing (a key compromise, a collection whose admin can move tokens), or a
///    mint against an NFT the canister does not hold. Supply and backing diverge and the contract cannot see it. The custodian
///    is the whole question; anyone can reconcile `ownerOf(tokenId) == custodian` on `chainId` for every held item.
///  - a delivery sent near the deadline AND a refund here: the redeemer then has both. The canister stops sending a day before
///    `deliverBy` and re-reads the ticket immediately before signing (docs/v4/nft-wrapper.md §5.3).
///  - a beacon that never proves a round: the ticket on it, and every ticket behind it, waits. Refund is from `Resolved` only.
/// @dev The floor module (docs/v5/floor-module.md): one address, wired once before anything was ever minted, may mint and burn
/// ETH-backed units (`ethBacked`) and admit and release rented NFTs. Its ledger is then
/// `totalSupply == (held − openTickets) × ONE_ITEM + ethBacked`, and a redemption needs an unpromised NFT.
contract NftWrapper is ERC20, IFloorWrapped {
    IRandomnessBeacon public immutable beacon;
    /// @notice The EIP-155 chain the collection lives on. Never this chain: a 4663 collection is held by contract (NftSweepVault).
    uint64 public immutable chainId;
    /// @notice The ERC-721 contract on `chainId`.
    address public immutable collection;
    /// @notice The canister's EVM address: the only minter here, the only recorder of deliveries, and — the same 20 bytes, one
    /// threshold key — the address that holds every wrapped NFT on `chainId`. Code-less; no setter.
    address public immutable custodian;

    uint256 public constant ONE_ITEM = 1e18;
    uint64 public constant PIN_AHEAD = 20;
    uint256 public constant REDRAW_AFTER = 24 hours;
    uint256 public constant DELIVERY_DEADLINE = 7 days;
    /// @notice Where a burn is delivered on `chainId`: nobody has its key.
    address public constant DEAD = 0x000000000000000000000000000000000000dEaD;
    /// @notice The first four bytes of a holder deposit's `safeTransferFrom` data (docs/v4/nft-wrapper.md §4).
    bytes4 public constant DEPOSIT_MAGIC = "EWN1";

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

    /// @dev `Wrapper.Pending`: an item waiting to become drawable, stamped with how many tickets existed when it became free.
    struct Pending { uint256 itemId; uint256 stamp; }

    /// @notice `itemId + 1` of the token id while it is held here; zero when it is not. One token id, one held item, ever at once.
    mapping(uint256 tokenId => uint256) public itemOf;
    /// @notice `keccak256(tokenId, txHash)` pairs that have minted: once, ever, so a replayed attestation cannot mint again.
    mapping(bytes32 source => bool) public sourceUsed;
    /// @notice Items minted and not delivered: in the free set, pending, or on a resolved ticket.
    uint256 public held;
    /// @notice Burns neither delivered nor refunded. `totalSupply == (held − openTickets) × ONE_ITEM`.
    uint256 public openTickets;
    /// @notice Items delivered to `DEAD`.
    uint256 public burned;
    uint256 public nextToResolve;
    uint256 public pendingHead;

    Item[] private _items;
    uint256[] private _free;
    Pending[] private _pending;
    Ticket[] private _tickets;

    // ---- the floor module (docs/v5/floor-module.md) ----
    /// @notice Who deployed this wrapper: the only address that may wire the floor module, once, before anything is minted.
    address public immutable deployer;
    address public floorModule;
    /// @notice Units in supply backed by the module's ETH, not by an NFT.
    uint256 public ethBacked;
    /// @notice Rented NFTs promised back: leaving, drawn while leaving, or ready to return.
    uint256 public openReturns;
    /// @notice Rented NFTs in custody. Never owned.
    uint256 public rentedHeld;

    struct Rental {
        RentalState state;
        uint64 leaveAfter;       // the ticket count at release: tickets below it may still draw the NFT, none at or above
        bytes32 returnTx;
        bytes returnTo;          // abi.encode(address) on `chainId`, fixed at intake
    }

    mapping(uint256 itemId => Rental) private _rentals;
    uint256[] private _rentalIds;
    uint256[] private _leaving;
    uint256 public leavingHead;
    mapping(uint256 itemId => uint256) private _freePos;
    mapping(uint256 itemId => bool) private _pendingSkip;

    // ---- H11: the custodian's mint rate (docs/v5/custody-hardening.md) ----
    uint256 public constant MINT_WINDOW = 24 hours;
    uint256 public constant DEFAULT_MINTS_PER_DAY = 20;
    uint256 public constant LIMIT_DELAY = 7 days;
    /// @notice Items the custodian may bring in (purchases, deposits, rentals; the creator's `wrap` too) in any 24 h, both clocks.
    uint256 public maxMintsPerDay = DEFAULT_MINTS_PER_DAY;
    /// @notice Items ever minted, and when each was (packed `timestamp << 64 | block`): the sliding window's memory.
    uint256 public itemMints;
    mapping(uint256 => uint256) private _mintedAt;
    /// @notice A raise waiting out `LIMIT_DELAY` on both clocks. Lowering takes effect at once.
    uint256 public pendingMintLimit;
    uint64 public pendingLimitAt;
    uint64 public pendingLimitAtBlock;

    event RedeemedToRelist(uint256 indexed id);
    event Wrapped(uint256 indexed itemId, uint256 indexed tokenId, bytes32 purchaseTx, address indexed to);
    event Deposited(uint256 indexed itemId, uint256 indexed tokenId, bytes32 depositTx, address indexed to);
    event Redeemed(uint256 indexed ticketId, address indexed owner, uint64 round, address destination);
    event Repinned(uint256 indexed ticketId, uint64 round);
    event Resolved(uint256 indexed ticketId, uint256 indexed itemId, uint256 indexed tokenId, uint64 deliverBy);
    event Delivered(uint256 indexed ticketId, uint256 indexed tokenId, address destination, bytes32 txHash);
    event Refunded(uint256 indexed ticketId, uint256 indexed itemId, address indexed owner);
    event FloorModuleWired(address module);
    event MintLimitProposed(uint256 limit, uint64 readyAt);
    event MintLimitSet(uint256 limit);
    event RentalAdmitted(uint256 indexed itemId, uint256 indexed tokenId, bytes32 depositTx, address returnTo);
    event RentalLeaving(uint256 indexed itemId, uint64 leaveAfter);
    event RentalReady(uint256 indexed itemId);
    event RentalTaken(uint256 indexed itemId, uint256 indexed ticketId);
    event RentalReturned(uint256 indexed itemId, uint256 indexed tokenId, bytes32 txHash);

    error NotCustodian();
    error BadInput();
    error BadDestination();
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
    error NoItem();
    error MintRateLimited();

    constructor(IRandomnessBeacon beacon_, uint64 chainId_, address collection_, address custodian_, string memory name_,
        string memory symbol_) ERC20(name_, symbol_)
    {
        if (address(beacon_) == address(0) || collection_ == address(0) || custodian_ == address(0)) revert BadInput();
        if (chainId_ == 0 || chainId_ == block.chainid) revert BadInput();
        if (custodian_.code.length != 0) revert BadInput();
        beacon = beacon_;
        chainId = chainId_;
        collection = collection_;
        custodian = custodian_;
        deployer = msg.sender;
    }

    // ---- H11: the custodian's mint rate --------------------------------------------------------------------------------------

    /// @notice Once, by the deployer, before anything was ever minted: the deployment's daily limit.
    function initMintLimit(uint256 limit) external {
        if (msg.sender != deployer) revert NotDeployer();
        if (limit == 0) revert BadInput();
        if (itemMints != 0 || totalSupply() != 0 || held != 0) revert NotFresh();
        maxMintsPerDay = limit;
        emit MintLimitSet(limit);
    }

    /// @notice The deployer (the timelock): lower the daily limit at once, or propose a raise that anyone may accept after
    /// `LIMIT_DELAY` on both clocks. The only way the custodian's rate ever goes up, and it is public for a week first.
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
        pendingLimitAtBlock = uint64(block.number + Wait.blocksFor(LIMIT_DELAY));
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
        return Wait.elapsed(uint64(st >> 64) + uint64(MINT_WINDOW), uint64(st) + uint64(Wait.blocksFor(MINT_WINDOW)));
    }

    function _countMint() private {
        if (!mintRoom()) revert MintRateLimited();
        _mintedAt[itemMints++] = (block.timestamp << 64) | block.number;
    }

    // ---- the floor module ----------------------------------------------------------------------------------------------------

    /// @notice Once, by the deployer, before this wrapper has ever minted or burned anything: the one floor module.
    function wireFloorModule(address module) external {
        if (msg.sender != deployer) revert NotDeployer();
        if (floorModule != address(0)) revert AlreadyWired();
        if (module == address(0) || module.code.length == 0) revert BadInput();
        if (totalSupply() != 0 || held != 0 || _tickets.length != 0) revert NotFresh();
        floorModule = module;
        emit FloorModuleWired(module);
    }

    modifier onlyModule() {
        if (msg.sender != floorModule || msg.sender == address(0)) revert NotModule();
        _;
    }

    /// @notice The module: `amount` ETH-backed units to `to`.
    function moduleMint(address to, uint256 amount) external onlyModule {
        if (to == address(0) || amount == 0) revert BadInput();
        ethBacked += amount;
        _mint(to, amount);
    }

    /// @notice The module: burn `amount` of its own units against the ETH-backed count.
    function moduleBurn(uint256 amount) external onlyModule {
        if (amount == 0 || amount > ethBacked) revert BadInput();
        ethBacked -= amount;
        _burn(msg.sender, amount);
    }

    /// @notice The module, for the custodian: a holder's NFT lent to the module (docs/v5/floor-module.md §9.2). The same checks as
    /// `mintDeposit`; `ONE_ITEM` to the module; the NFT goes back to `returnTo` on `chainId` at the end.
    /// @param attestation `abi.encode(uint256 tokenId, bytes32 depositTx)`
    /// @param returnTo `abi.encode(address)`: not zero, not the custodian, not `DEAD`
    function admitRental(bytes calldata attestation, bytes calldata returnTo) external onlyModule
        returns (uint256 itemId, bytes32 itemKey)
    {
        (uint256 tokenId, bytes32 depositTx) = abi.decode(attestation, (uint256, bytes32));
        if (returnTo.length != 32) revert BadDestination();
        address dest = abi.decode(returnTo, (address));
        if (dest == address(0) || dest == custodian || dest == DEAD) revert BadDestination();
        itemId = _admitAs(tokenId, depositTx, msg.sender, true);
        itemKey = keccak256(abi.encode(tokenId));
        Rental storage r = _rentals[itemId];
        r.state = RentalState.Active;
        r.returnTo = returnTo;
        _rentalIds.push(itemId);
        rentedHeld++;
        emit RentalAdmitted(itemId, tokenId, depositTx, dest);
    }

    /// @notice The module, at a rental's end: the NFT is promised back and leaves the random set once every ticket burned before
    /// now has resolved (at once, if none is waiting). Only an Active rental (not on a ticket). Until it is returned, the unit it
    /// backed is backed by the rental's escrowed ETH at the module; on return that unit becomes ETH-backed (`recordReturn`).
    function returnRental(uint256 itemId) external onlyModule {
        Rental storage r = _rentals[itemId];
        if (r.state != RentalState.Active) revert WrongStage();
        r.state = RentalState.Leaving;
        r.leaveAfter = uint64(_tickets.length);
        _leaving.push(itemId);
        openReturns++;
        emit RentalLeaving(itemId, r.leaveAfter);
        _processLeaving(nextToResolve);
    }

    /// @notice Anyone: take out of the random set every leaving NFT whose earlier tickets have all resolved.
    function processLeaving() external {
        _processLeaving(nextToResolve);
    }

    /// @notice The canister, having sent a rented NFT back to its renter's `returnTo` on `chainId` (plain `transferFrom`).
    function recordReturn(uint256 itemId, bytes32 txHash) external {
        if (msg.sender != custodian) revert NotCustodian();
        Rental storage r = _rentals[itemId];
        if (r.state != RentalState.Ready) revert WrongStage();
        if (txHash == bytes32(0)) revert BadInput();
        r.state = RentalState.Returned;
        r.returnTx = txHash;
        uint256 tokenId = _items[itemId].tokenId;
        delete itemOf[tokenId];
        held--;
        openReturns--;
        rentedHeld--;
        // the unit this item backed is backed by its rental's escrowed ETH at the module from now on
        ethBacked += ONE_ITEM;
        emit RentalReturned(itemId, tokenId, txHash);
        IFloorRentals(floorModule).rentalReturned(itemId);        // and the module makes the rental's escrow its backing now
    }

    function _processLeaving(uint256 upTo) private {
        uint256 h = leavingHead;
        uint256 n = _leaving.length;
        while (h < n) {
            uint256 x = _leaving[h];
            Rental storage r = _rentals[x];
            if (r.leaveAfter > upTo) break;
            ++h;
            if (r.state == RentalState.Leaving) {
                uint256 pos = _freePos[x];
                if (pos != 0) {
                    uint256 last = _free[_free.length - 1];
                    _free[pos - 1] = last;
                    if (last != x && _rentals[last].state != RentalState.None) _freePos[last] = pos;
                    _free.pop();
                    delete _freePos[x];
                } else {
                    _pendingSkip[x] = true;
                }
                r.state = RentalState.Ready;
                emit RentalReady(x);
            }
        }
        leavingHead = h;
    }

    // ---- minting: the custodian only ------------------------------------------------------------------------------------------

    /// @notice The canister, for an NFT it bought into its own address on `chainId` by `purchaseTx` and saw there at a finalized
    /// block: one `ONE_ITEM` to `to` (the curve, desk or queue whose request this fills).
    function mint(uint256 tokenId, bytes32 purchaseTx, address to) external returns (uint256 itemId) {
        itemId = _admit(tokenId, purchaseTx, to, false);
        emit Wrapped(itemId, tokenId, purchaseTx, to);
    }

    /// @notice The canister, for an NFT its owner sent it by `depositTx` with the 4663 recipient `to` in the transfer's signed data
    /// (docs/v4/nft-wrapper.md §4). The canister attests the binding and finality; this checks what a contract can.
    function mintDeposit(uint256 tokenId, bytes32 depositTx, address to) external returns (uint256 itemId) {
        itemId = _admit(tokenId, depositTx, to, true);
        emit Deposited(itemId, tokenId, depositTx, to);
    }

    function _admit(uint256 tokenId, bytes32 txHash, address to, bool deposit) private returns (uint256 itemId) {
        if (msg.sender != custodian) revert NotCustodian();
        itemId = _admitAs(tokenId, txHash, to, deposit);
    }

    function _admitAs(uint256 tokenId, bytes32 txHash, address to, bool deposit) private returns (uint256 itemId) {
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

    // ---- redeeming: nobody chooses the NFT ------------------------------------------------------------------------------------

    /// @notice Burn one whole token for one random NFT, delivered to `destination` on `chainId` — your own address there, or
    /// another if yours has no twin on that chain. The round that decides which NFT is pinned now and does not exist yet; the
    /// destination cannot be changed afterwards.
    function redeem(address destination) external returns (uint256) {
        if (destination == address(0) || destination == custodian) revert BadDestination();
        return _redeem(destination);
    }

    /// @notice Burn one whole token for one random NFT, delivered to `DEAD` on `chainId`: out of circulation for good.
    function redeemToBurn() external returns (uint256) {
        return _redeem(DEAD);
    }

    /// @notice Burn one whole token for one random NFT that stays with the custodian and is listed for sale from there (the YOYO
    /// policy's exit, docs/v5/policies-and-fees.md §1.3): the one ticket whose destination is the custodian.
    function redeemToRelist() external returns (uint256 id) {
        id = _redeem(custodian);
        emit RedeemedToRelist(id);
    }

    function _redeem(address destination) private returns (uint256 id) {
        // a ticket needs an NFT nobody else is promised; ETH-backed units are redeemed for ETH at the floor module instead
        if (held < openTickets + openReturns + 1) revert NoItem();
        _burn(msg.sender, ONE_ITEM);
        id = _tickets.length;
        Ticket storage t = _tickets.push();
        t.owner = msg.sender;
        t.destination = destination;
        openTickets++;
        _pin(t);
        emit Redeemed(id, msg.sender, t.round, destination);
    }

    /// @notice Anyone, if a pinned round is still unproven a day later on both clocks (drand skipped it): pins a fresh one.
    function redraw(uint256 id) external {
        Ticket storage t = _tickets[id];
        if (t.stage != Stage.Pinned) revert WrongStage();
        if (!Wait.elapsed(uint64(t.pinnedAt + REDRAW_AFTER), uint64(t.pinnedAtBlock + Wait.blocksFor(REDRAW_AFTER)))) revert TooEarly();
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

    /// @notice Anyone, strictly in ticket order: proves the round (or reads it, if already proven) and draws the NFT among those
    /// that were already free when this ticket was burned. Nothing done after the round is public can move the result.
    function resolve(uint256 id, bytes calldata signature) external {
        if (id != nextToResolve) revert OutOfOrder(nextToResolve);
        Ticket storage t = _tickets[id];
        if (t.stage != Stage.Pinned) revert WrongStage();
        bytes32 randomness = beacon.randomnessOf(t.round);
        if (randomness == bytes32(0)) randomness = beacon.prove(t.round, signature);

        // every item that was free before this burn becomes drawable; the rest wait for a later ticket
        uint256 head = pendingHead;
        uint256 n = _pending.length;
        while (head < n && _pending[head].stamp <= id) {
            uint256 x = _pending[head++].itemId;
            if (_pendingSkip[x]) { delete _pendingSkip[x]; continue; }
            _free.push(x);
            if (_rentals[x].state != RentalState.None) _freePos[x] = _free.length;
        }
        pendingHead = head;
        _processLeaving(id);
        uint256 size = _free.length;
        if (size == 0) revert NothingToDraw();   // unreachable while the ledger holds (docs/v3/wrapper.md, "Built", 4)

        uint256 i = uint256(keccak256(abi.encode(randomness, address(this), id))) % size;
        uint256 itemId = _free[i];
        uint256 last = _free[size - 1];
        _free[i] = last;
        if (_freePos[last] != 0) _freePos[last] = i + 1;
        _free.pop();
        RentalState rs = _rentals[itemId].state;
        if (rs != RentalState.None) {
            delete _freePos[itemId];
            if (rs == RentalState.Active) _rentals[itemId].state = RentalState.ActiveDrawn;
            else if (rs == RentalState.Leaving) _rentals[itemId].state = RentalState.LeavingDrawn;
        }

        t.itemId = itemId;
        t.stage = Stage.Resolved;
        t.deliverBy = uint64(block.timestamp + DELIVERY_DEADLINE);
        t.deliverByBlock = uint64(block.number + Wait.blocksFor(DELIVERY_DEADLINE));
        nextToResolve = id + 1;
        emit Resolved(id, itemId, _items[itemId].tokenId, t.deliverBy);
    }

    /// @notice The canister records the transaction on `chainId` that sent a resolved ticket's NFT to its destination. Allowed
    /// past the deadline for as long as nobody has refunded.
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
        Rental storage r = _rentals[t.itemId];
        if (r.state == RentalState.ActiveDrawn || r.state == RentalState.LeavingDrawn) {
            if (r.state == RentalState.LeavingDrawn) openReturns--;
            r.state = RentalState.Taken;
            rentedHeld--;
            emit RentalTaken(t.itemId, id);
        }
    }

    /// @notice Anyone, once a resolved ticket is past its deadline on both clocks with no delivery: the NFT goes back to the set
    /// and one whole token is minted back to the ticket's owner.
    function refund(uint256 id) external {
        Ticket storage t = _tickets[id];
        if (t.stage != Stage.Resolved) revert WrongStage();
        if (!Wait.elapsed(t.deliverBy, t.deliverByBlock)) revert TooEarly();
        t.stage = Stage.Refunded;
        openTickets--;
        Rental storage r = _rentals[t.itemId];
        if (r.state == RentalState.LeavingDrawn) {
            r.state = RentalState.Ready;
            emit RentalReady(t.itemId);
        } else {
            if (r.state == RentalState.ActiveDrawn) r.state = RentalState.Active;
            _pending.push(Pending(t.itemId, _tickets.length));
        }
        _mint(t.owner, ONE_ITEM);
        emit Refunded(id, t.itemId, t.owner);
    }

    // ---- views ---------------------------------------------------------------------------------------------------------------

    function refunded(uint256 id) external view returns (bool) { return _tickets[id].stage == Stage.Refunded; }
    function ticket(uint256 id) external view returns (Ticket memory) { return _tickets[id]; }
    function ticketCount() external view returns (uint256) { return _tickets.length; }
    function rentalState(uint256 itemId) external view returns (RentalState) { return _rentals[itemId].state; }
    function rentalOf(uint256 itemId) external view returns (Rental memory) { return _rentals[itemId]; }
    function rentalCount() external view returns (uint256) { return _rentalIds.length; }
    function rentalAt(uint256 i) external view returns (uint256) { return _rentalIds[i]; }
    function leavingCount() external view returns (uint256) { return _leaving.length - leavingHead; }
    function item(uint256 itemId) external view returns (Item memory) { return _items[itemId]; }
    function itemCount() external view returns (uint256) { return _items.length; }
    /// @notice Items drawable by the next ticket to resolve, before it promotes anything pending.
    function freeCount() external view returns (uint256) { return _free.length; }
    function freeAt(uint256 i) external view returns (uint256) { return _free[i]; }
    function pendingCount() external view returns (uint256) { return _pending.length - pendingHead; }
    /// @notice The `i`-th item still waiting to become drawable, and its stamp: a ticket numbered below `stamp` cannot draw it.
    function pendingAt(uint256 i) external view returns (uint256 itemId, uint256 stamp) {
        Pending storage p = _pending[pendingHead + i];
        return (p.itemId, p.stamp);
    }
}
