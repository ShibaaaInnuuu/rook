// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {ListingRegistry} from "./ListingRegistry.sol";
import {Distributor} from "./Distributor.sol";
import {ILauncher} from "./interfaces/ILauncher.sol";
import {IEtchVault} from "./interfaces/IEtchVault.sol";
import {IRandomnessBeacon} from "./interfaces/IRandomnessBeacon.sol";
import {Wait} from "./lib/Wait.sol";
import {BtcAddress} from "./lib/BtcAddress.sol";
import {IWrappedCollection} from "./interfaces/IWrappedCollection.sol";
import {IFloorWrapped, IFloorRentals} from "./interfaces/IFloorWrapped.sol";

/// @title Wrapper
/// @notice One whole token is one random inscription from this listing's vault, and the way to find out what that is worth is
/// to take one. A receipted frog whose paying coin's creator has ruled out a draw is minted as `ONE_ITEM`; a whole token burns
/// to a ticket pinned to a future drand round, which is final (nobody can re-pin it); the round picks which frog leaves; the
/// custodian records the Bitcoin delivery; a ticket nobody delivers in time is refunded, frog back to the set and token back to
/// its owner, and so is a ticket nobody resolves within `PIN_TIMEOUT`. docs/v3/wrapper.md;
/// docs/audit/2026-09-26/fixes-B-C-keeper-draws.md.
/// @dev What this does NOT protect against, because nothing on this chain can:
///  - a frog that leaves Bitcoin without this chain knowing (a key compromise, a lost canister). Supply and backing diverge
///    and the contract cannot see it. The custodian is the whole question; this contract only keeps the books honest.
///  - a delivery that confirms after a refund on this chain: the redeemer then has both. A resolved ticket is refunded only once
///    the custodian attests its delivery can no longer confirm (`markUndeliverable`: its conflict-out is deep, or nothing was ever
///    sent) or `LATE_DELIVERY_WINDOW` after `deliverBy` (a custodian that is gone); until then a late delivery is simply recorded.
///    So a redeemer who pins the delivery (canister review2 N-2) must keep it unmined for a month. A delivery confirming after
///    that, or after a false attestation, is still both.
///  - a vault holding frogs nobody wants. The token then trades below one frog and nobody redeems. That is the mechanism
///    reporting honestly, not a failure of it.
///  - a round nobody proves for `PIN_TIMEOUT`: anyone may then refund the ticket (no frog moves), and resolution skips it. A
///    holder who reads the round's outcome and dislikes it can get that refund only if nobody else resolves for a week.
/// @dev The floor module (docs/v5/floor-module.md): one address, wired once before anything was ever minted, may mint and burn
/// ETH-backed units (`ethBacked`) and admit and release rented frogs. Its ledger is then
/// `totalSupply == (held − openTickets) × ONE_ITEM + ethBacked`, and a redemption needs an unpromised frog.
/// @dev The one read `wireFloorModule` makes of the module it is handed.
interface IModuleOf {
    function wrapped() external view returns (address);
}

contract Wrapper is ERC20, IFloorWrapped {
    ListingRegistry public immutable registry;
    Distributor public immutable distributor;
    IRandomnessBeacon public immutable beacon;
    uint256 public immutable listingId;
    /// @notice Where every wrapped frog's `ONE_ITEM` is minted: the pro-rata claim contract (docs/v3/wrapper.md, "Built", 3).
    address public immutable distribution;
    /// @notice The canister that holds the frogs and sends them on Bitcoin: the only party that records a delivery (B-03, C-05),
    /// since it alone signs the Bitcoin spend (docs/v3/wrapper.md, "Custody on its own key").
    address public immutable custodian;
    /// @notice The parent chain's block time on this deployment's chain (lib/Wait.sol): deployment config, fixed here.
    uint256 public immutable parentBlockSeconds;
    /// @notice The Bitcoin (taproot) address wrapped frogs are held at. Written once, here; there is no setter. `wrap` refuses
    /// while the listing's delivery address (`btcVault`) differs from it.
    string public custodyAddress;

    uint256 public constant ONE_ITEM = 1e18;
    /// @notice BIP341's NUMS point H as a key-path-only taproot output: nobody knows its key and there is no script path.
    /// H.x = 50929b74…3ac0 = SHA-256 of the uncompressed secp256k1 generator; anyone can recompute it. docs/v3/sweep.md.
    /// A frog redeemed here (`redeemToBurn`) leaves circulation for good.
    string public constant BURN_ADDRESS = "bc1p2zffkaxp5py4fdutfdsrt6t6tcrc5ks09rkfd428hlhf4n5q8tqqwv7ahk";
    /// @notice The Distributor's own margin: a minute of drand rounds, beyond any sequencer lag short of an outage.
    uint64 public constant PIN_AHEAD = 20;
    /// @notice A ticket still unresolved this long after its pin, on both clocks, may be refunded. The round is never replaced.
    uint256 public constant PIN_TIMEOUT = 7 days;
    uint256 public constant DELIVERY_DEADLINE = 7 days;
    /// @notice A resolved ticket is refundable only this long after `deliverBy` (both clocks): the canister stops sending a day
    /// before `deliverBy`, re-prices a stuck delivery and conflicts it out, but a delivery broadcast in time can still confirm
    /// late; the grace lets it be recorded before anyone can refund, so a redeemer never keeps both the frog and the token
    /// (docs/audit/2026-09-26/fixes-D-canister.md M-1).
    uint256 public constant REFUND_GRACE = 72 hours;
    /// @notice Without the custodian's `markUndeliverable`, a resolved ticket is refundable only this long after `deliverBy` (both
    /// clocks): a delivery kept unmined below the market (a pinned child, canister review2 N-2) must then stay so for a month.
    uint256 public constant LATE_DELIVERY_WINDOW = 30 days;

    enum Stage { None, Pinned, Resolved, Delivered, Refunded }

    struct Ticket {
        Stage stage;
        address owner;
        uint64 round;
        uint64 pinnedAt;
        uint64 pinnedAtBlock;     // the parent chain's half of the pin timeout. See lib/Wait.sol.
        uint64 deliverBy;
        uint64 deliverByBlock;    // ...and of the delivery deadline
        uint256 acquisitionId;    // the frog, once resolved
        bytes32 deliveryTxid;
        string destination;
    }

    /// @dev A frog waiting to become drawable: `stamp` is how many tickets existed when it became free. Stamps only grow, so
    /// this is a queue, and ticket `n` promotes from its head every frog stamped `≤ n` before it draws.
    struct Pending { uint256 acquisitionId; uint256 stamp; }

    mapping(uint256 acquisitionId => bool) public wrapped;
    /// @notice Frogs wrapped and not delivered: in the free set, pending, or on a resolved ticket.
    uint256 public held;
    /// @notice Burns neither delivered nor refunded, resolved or not. `totalSupply == (held − openTickets) × ONE_ITEM`.
    uint256 public openTickets;
    uint256 public nextToResolve;
    uint256 public pendingHead;

    uint256[] private _free;
    Pending[] private _pending;
    Ticket[] private _tickets;

    /// @notice A frog deposited by its holder (docs/v3/sweep.md, 1a) is numbered `DEPOSIT_FLAG | n`, so its id in the free set and
    /// on a ticket can never collide with an acquisition id.
    uint256 public constant DEPOSIT_FLAG = 1 << 255;
    uint256 public depositCount;
    /// @notice A Bitcoin outpoint `keccak256(txid, vout)` that has minted: once, ever, so a re-included deposit cannot mint again.
    mapping(bytes32 outpoint => bool) public depositMinted;
    /// @notice Inscriptions this wrapper counts as held (by either path), cleared on delivery: nothing held is minted twice.
    mapping(bytes32 inscriptionLeaf => bool) public inscriptionHeld;
    mapping(uint256 frogId => bytes32 inscriptionLeaf) public leafOfFrog;
    /// @notice The custodian's word that no version of a resolved ticket's delivery can confirm any more.
    mapping(uint256 ticketId => bool) public undeliverable;

    // ---- the floor module (docs/v5/floor-module.md) ----
    /// @notice Who deployed this wrapper: the only address that may wire the floor module, once, before anything is minted.
    address public immutable deployer;
    address public floorModule;
    /// @notice Units in supply backed by the module's ETH, not by a frog.
    uint256 public ethBacked;
    /// @notice Rented frogs promised back: leaving, drawn while leaving, or ready to return.
    uint256 public openReturns;
    /// @notice Rented frogs in custody. Never owned.
    uint256 public rentedHeld;

    struct Rental {
        RentalState state;
        uint64 leaveAfter;       // the ticket count at release: tickets below it may still draw the frog, none at or above
        bytes32 returnTx;
        bytes returnTo;          // a taproot address, fixed at intake
    }

    mapping(uint256 frogId => Rental) private _rentals;
    uint256[] private _rentalIds;
    uint256[] private _leaving;
    uint256 public leavingHead;
    /// @dev A rented frog's index + 1 in `_free` while it is there; a leaving frog still pending is tombstoned instead.
    mapping(uint256 frogId => uint256) private _freePos;
    mapping(uint256 frogId => bool) private _pendingSkip;

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

    event Wrapped(uint256 indexed acquisitionId, string inscriptionId, address indexed to);
    event Redeemed(uint256 indexed ticketId, address indexed owner, uint64 round, string destination);
    event Resolved(uint256 indexed ticketId, uint256 indexed acquisitionId, uint64 deliverBy);
    event Delivered(uint256 indexed ticketId, uint256 indexed acquisitionId, bytes32 txid);
    event Refunded(uint256 indexed ticketId, uint256 indexed acquisitionId, address indexed owner);
    event Undeliverable(uint256 indexed ticketId);
    event Deposited(uint256 indexed frogId, string inscriptionId, bytes32 txid, uint32 vout, address indexed to);
    event FloorModuleWired(address module);
    event MintLimitProposed(uint256 limit, uint64 readyAt);
    event MintLimitSet(uint256 limit);
    event RentalAdmitted(uint256 indexed frogId, string inscriptionId, bytes returnTo);
    event RentalLeaving(uint256 indexed frogId, uint64 leaveAfter);
    event RentalReady(uint256 indexed frogId);
    event RentalTaken(uint256 indexed frogId, uint256 indexed ticketId);
    event RentalReturned(uint256 indexed frogId, bytes32 txid);

    error WrongKind();
    event RedeemedToRelist(uint256 indexed id);
    error ListingPaused();
    error AlreadyWrapped();
    error NotAnInscription();
    error NotCreator();
    error NotNever();
    error BadAddress();
    error BadInput();
    error NotKeeper();
    error NotCustody();
    error WrongStage();
    error OutOfOrder(uint256 next);
    error TooEarly();
    error NothingToDraw();
    error NotCustodian();
    error NotInCollection();
    error AlreadyHeld();
    error AlreadyMinted();
    error NotDeployer();
    error AlreadyWired();
    error NotFresh();
    error NotModule();
    error NoItem();
    error MintRateLimited();
    error NotItsModule();
    /// @notice A resolved ticket's delivery may still confirm: no refund until the custodian says it cannot, or the late window.
    error NotUndeliverable();

    constructor(ListingRegistry registry_, Distributor distributor_, IRandomnessBeacon beacon_, uint256 listingId_,
        address distribution_, address custodian_, string memory custodyAddress_, string memory name_, string memory symbol_,
        uint256 parentBlockSeconds_)
        ERC20(name_, symbol_)
    {
        Wait.checkBlockSeconds(parentBlockSeconds_);
        parentBlockSeconds = parentBlockSeconds_;
        if (distribution_ == address(0) || custodian_ == address(0)) revert BadInput();
        if (!BtcAddress.isTaproot(custodyAddress_)) revert BadAddress();
        if (IEtchVault(registry_.vaultOf(listingId_)).isRune()) revert WrongKind();
        registry = registry_;
        distributor = distributor_;
        beacon = beacon_;
        listingId = listingId_;
        distribution = distribution_;
        custodian = custodian_;
        custodyAddress = custodyAddress_;
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

    // ---- the floor module ----------------------------------------------------------------------------------------------------

    /// @notice Once, by the deployer, before this wrapper has ever minted or burned anything: the one floor module.
    function wireFloorModule(address module) external {
        if (msg.sender != deployer) revert NotDeployer();
        if (floorModule != address(0)) revert AlreadyWired();
        if (module == address(0) || module.code.length == 0) revert BadInput();
        if (IModuleOf(module).wrapped() != address(this)) revert NotItsModule();                // B-05: a module built for this wrapper
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

    /// @notice The module, for the custodian: a holder's frog lent to the module (docs/v5/floor-module.md §9.2). The same checks
    /// as `wrapDeposit`; `ONE_ITEM` to the module; the frog goes back to `returnTo` (a taproot address) at the end.
    /// @param attestation `abi.encode(string inscriptionId, bytes32[] proof, bytes32 txid, uint32 vout)`
    function admitRental(bytes calldata attestation, bytes calldata returnTo) external onlyModule
        returns (uint256 frogId, bytes32 itemKey)
    {
        (string memory inscriptionId, bytes32[] memory proof, bytes32 txid, uint32 vout) =
            abi.decode(attestation, (string, bytes32[], bytes32, uint32));
        if (!BtcAddress.isTaproot(string(returnTo))) revert BadAddress();
        frogId = _admitDeposit(inscriptionId, proof, txid, vout, msg.sender);
        itemKey = leafOfFrog[frogId];
        Rental storage r = _rentals[frogId];
        r.state = RentalState.Active;
        r.returnTo = returnTo;
        _rentalIds.push(frogId);
        rentedHeld++;
        emit RentalAdmitted(frogId, inscriptionId, returnTo);
    }

    /// @notice The module, at a rental's end: the frog is promised back and leaves the random set once every ticket burned before
    /// now has resolved (at once, if none is waiting). Only an Active rental (not on a ticket). Until it is returned, the unit it
    /// backed is backed by the rental's escrowed ETH at the module; on return that unit becomes ETH-backed (`recordReturn`).
    function returnRental(uint256 frogId) external onlyModule {
        Rental storage r = _rentals[frogId];
        if (r.state != RentalState.Active) revert WrongStage();
        r.state = RentalState.Leaving;
        r.leaveAfter = uint64(_tickets.length);
        _leaving.push(frogId);
        openReturns++;
        emit RentalLeaving(frogId, r.leaveAfter);
        _processLeaving(nextToResolve);
    }

    /// @notice Anyone: take out of the random set every leaving frog whose earlier tickets have all resolved.
    function processLeaving() external {
        _processLeaving(nextToResolve);
    }

    /// @notice The custodian, having sent a rented frog back to its renter's `returnTo`.
    function recordReturn(uint256 frogId, bytes32 txid) external {
        if (msg.sender != custodian) revert NotCustodian();
        Rental storage r = _rentals[frogId];
        if (r.state != RentalState.Ready) revert WrongStage();
        if (txid == bytes32(0)) revert BadInput();
        r.state = RentalState.Returned;
        r.returnTx = txid;
        inscriptionHeld[leafOfFrog[frogId]] = false;
        held--;
        openReturns--;
        rentedHeld--;
        // the unit this item backed is backed by its rental's escrowed ETH at the module from now on
        ethBacked += ONE_ITEM;
        emit RentalReturned(frogId, txid);
        IFloorRentals(floorModule).rentalReturned(frogId);        // and the module makes the rental's escrow its backing now
    }

    /// @dev Leaving frogs whose `leaveAfter ≤ upTo` leave the set: every ticket that could still draw them has resolved.
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
                    _pendingSkip[x] = true;       // still pending: dropped when its turn to be promoted comes
                }
                r.state = RentalState.Ready;
                emit RentalReady(x);
            }
            // LeavingDrawn: the ticket that drew it decides (delivered: Taken; refunded: Ready)
        }
        leavingHead = h;
    }

    // ---- minting -------------------------------------------------------------------------------------------------------------

    /// @notice The paying coin's creator wraps one receipted frog they have already ruled out of any draw (`Mode.Never`, which
    /// the Distributor makes final). One `ONE_ITEM` is minted to `distribution`, never to a person.
    function wrap(uint256 acquisitionId) external {
        if (registry.isPaused(listingId)) revert ListingPaused();
        if (wrapped[acquisitionId]) revert AlreadyWrapped();
        // a frog is wrapped only if the listing delivers to custody (the owner can move btcVault only after 7 public days)
        if (keccak256(bytes(registry.listing(listingId).btcVault)) != keccak256(bytes(custodyAddress))) revert NotCustody();
        IEtchVault.Acquisition memory a = IEtchVault(registry.vaultOf(listingId)).acquisition(acquisitionId);
        if (a.amount != 1) revert NotAnInscription();
        if (ILauncher(registry.launcher()).creatorOf(a.coin) != msg.sender) revert NotCreator();
        Distributor.Lot memory lot = distributor.lot(listingId, acquisitionId);
        if (lot.mode != Distributor.Mode.Never || lot.stage != Distributor.Stage.None) revert NotNever();

        bytes32 leaf = registry.leafOf(a.assetRef);
        if (inscriptionHeld[leaf]) revert AlreadyHeld();
        _countMint();
        inscriptionHeld[leaf] = true;
        leafOfFrog[acquisitionId] = leaf;
        wrapped[acquisitionId] = true;
        held++;
        _pending.push(Pending(acquisitionId, _tickets.length));
        _mint(distribution, ONE_ITEM);
        emit Wrapped(acquisitionId, a.assetRef, distribution);
    }

    /// @notice The custodian, for a frog its holder sent to `custodyAddress` (docs/v3/sweep.md, 1a): one `ONE_ITEM` to the EVM
    /// address the depositor wrote into that Bitcoin transaction's OP_RETURN. The canister attests confirmation depth, that
    /// the output carries this inscription and nothing else, and the OP_RETURN; this checks what a contract can: membership,
    /// one mint per outpoint, and that the frog is not already held here.
    function wrapDeposit(string calldata inscriptionId, bytes32[] calldata proof, bytes32 txid, uint32 vout, address to)
        external returns (uint256 frogId)
    {
        if (msg.sender != custodian) revert NotCustodian();
        frogId = _admitDeposit(inscriptionId, proof, txid, vout, to);
    }

    function _admitDeposit(string memory inscriptionId, bytes32[] memory proof, bytes32 txid, uint32 vout, address to)
        private returns (uint256 frogId)
    {
        if (registry.isPaused(listingId)) revert ListingPaused();
        if (to == address(0) || txid == bytes32(0)) revert BadInput();
        if (!registry.isMember(listingId, inscriptionId, proof)) revert NotInCollection();
        bytes32 outpoint = keccak256(abi.encode(txid, vout));
        if (depositMinted[outpoint]) revert AlreadyMinted();
        bytes32 leaf = registry.leafOf(inscriptionId);
        if (inscriptionHeld[leaf]) revert AlreadyHeld();

        _countMint();
        depositMinted[outpoint] = true;
        inscriptionHeld[leaf] = true;
        frogId = DEPOSIT_FLAG | depositCount++;
        leafOfFrog[frogId] = leaf;
        held++;
        _pending.push(Pending(frogId, _tickets.length));
        _mint(to, ONE_ITEM);
        emit Deposited(frogId, inscriptionId, txid, vout, to);
    }

    // ---- redeeming -----------------------------------------------------------------------------------------------------------

    /// @notice Burn one whole token for one random frog, delivered to `taprootAddress`. The round that decides which frog is
    /// pinned now and does not exist yet; the address cannot be changed afterwards.
    function redeem(string calldata taprootAddress) external returns (uint256 id) {
        if (!BtcAddress.isTaproot(taprootAddress)) revert BadAddress();
        return _redeem(taprootAddress);
    }

    /// @notice Burn one whole token for one random frog, delivered to `BURN_ADDRESS`, where nobody can ever move it again
    /// (IWrappedCollection: the BURN policy's exit, whatever the collection).
    function redeemToBurn() external returns (uint256 id) {
        return _redeem(BURN_ADDRESS);
    }

    /// @notice Burn one whole token for one random frog that stays at `custodyAddress` and is listed for sale from there
    /// (IWrappedCollection: the YOYO policy's exit, docs/v5/policies-and-fees.md §1.3). The custodian delivers by listing.
    function redeemToRelist() external returns (uint256 id) {
        id = _redeem(custodyAddress);
        emit RedeemedToRelist(id);
    }

    function _redeem(string memory taprootAddress) private returns (uint256 id) {
        // a ticket needs a frog nobody else is promised; ETH-backed units are redeemed for ETH at the floor module instead
        if (held < openTickets + openReturns + 1) revert NoItem();
        _burn(msg.sender, ONE_ITEM);
        id = _tickets.length;
        Ticket storage t = _tickets.push();
        t.owner = msg.sender;
        t.destination = taprootAddress;
        openTickets++;
        _pin(t);
        emit Redeemed(id, msg.sender, t.round, taprootAddress);
    }

    function _pin(Ticket storage t) private {
        t.stage = Stage.Pinned;
        t.round = beacon.roundAt(block.timestamp) + PIN_AHEAD;
        t.pinnedAt = uint64(block.timestamp);
        t.pinnedAtBlock = uint64(block.number);
    }

    /// @notice Anyone, strictly in ticket order: proves the round (or reads it, if already proven) and draws the frog among
    /// those that were already free when this ticket was burned. Nothing done after the round is public can move the result.
    function resolve(uint256 id, bytes calldata signature) external {
        if (id != nextToResolve) revert OutOfOrder(nextToResolve);
        Ticket storage t = _tickets[id];
        if (t.stage != Stage.Pinned) revert WrongStage();
        bytes32 randomness = beacon.randomnessOf(t.round);
        if (randomness == bytes32(0)) randomness = beacon.prove(t.round, signature);

        // every frog that was free before this burn becomes drawable; the rest wait for a later ticket
        uint256 head = pendingHead;
        uint256 n = _pending.length;
        while (head < n && _pending[head].stamp <= id) {
            uint256 x = _pending[head++].acquisitionId;
            if (_pendingSkip[x]) { delete _pendingSkip[x]; continue; }     // a rented frog that left while pending
            _free.push(x);
            if (_rentals[x].state != RentalState.None) _freePos[x] = _free.length;
        }
        pendingHead = head;
        // rented frogs released before this ticket was burned are not in its set
        _processLeaving(id);
        uint256 size = _free.length;
        if (size == 0) revert NothingToDraw();   // unreachable while the ledger holds: see docs/v3/wrapper.md, "Built", 4

        uint256 i = uint256(keccak256(abi.encode(randomness, address(this), id))) % size;
        uint256 frog = _free[i];
        uint256 last = _free[size - 1];
        _free[i] = last;
        if (_freePos[last] != 0) _freePos[last] = i + 1;
        _free.pop();
        RentalState rs = _rentals[frog].state;
        if (rs != RentalState.None) {
            delete _freePos[frog];
            if (rs == RentalState.Active) _rentals[frog].state = RentalState.ActiveDrawn;
            else if (rs == RentalState.Leaving) _rentals[frog].state = RentalState.LeavingDrawn;
        }

        t.acquisitionId = frog;
        t.stage = Stage.Resolved;
        t.deliverBy = uint64(block.timestamp + DELIVERY_DEADLINE);
        t.deliverByBlock = uint64(block.number + Wait.blocksFor(DELIVERY_DEADLINE, parentBlockSeconds));
        nextToResolve = id + 1;
        _skipRefunded();
        emit Resolved(id, frog, t.deliverBy);
    }

    /// @dev Resolution is in ticket order; a ticket refunded while still pinned never draws, so the order moves past it.
    function _skipRefunded() private {
        uint256 n = nextToResolve;
        uint256 len = _tickets.length;
        while (n < len && _tickets[n].stage == Stage.Refunded) ++n;
        nextToResolve = n;
    }

    /// @notice The custodian records the Bitcoin transaction that sent a resolved ticket's frog. Allowed past the deadline for as
    /// long as nobody has refunded. Only the custodian: it alone signs the spend (B-03, C-05).
    function recordDelivery(uint256 id, bytes32 txid) external {
        if (msg.sender != custodian) revert NotCustodian();
        Ticket storage t = _tickets[id];
        if (t.stage != Stage.Resolved) revert WrongStage();
        if (txid == bytes32(0)) revert BadInput();
        t.stage = Stage.Delivered;
        t.deliveryTxid = txid;
        inscriptionHeld[leafOfFrog[t.acquisitionId]] = false;
        held--;
        openTickets--;
        emit Delivered(id, t.acquisitionId, txid);
        Rental storage r = _rentals[t.acquisitionId];
        if (r.state == RentalState.ActiveDrawn || r.state == RentalState.LeavingDrawn) {
            // promised back, but left through a ticket instead: the promise is void; the module pays the renter (§9.5)
            if (r.state == RentalState.LeavingDrawn) openReturns--;
            r.state = RentalState.Taken;
            rentedHeld--;
            emit RentalTaken(t.acquisitionId, id);
        }
    }

    /// @notice The custodian: no version of this resolved ticket's delivery can confirm any more — its conflict-out spent the
    /// delivery's inputs and is final, or nothing was ever sent. Only then is the ticket refundable at `deliverBy + REFUND_GRACE`;
    /// without it, at `deliverBy + LATE_DELIVERY_WINDOW` (canister review2 N-2). The custodian alone signs every spend, so this is
    /// no new trust.
    function markUndeliverable(uint256 id) external {
        if (msg.sender != custodian) revert NotCustodian();
        if (_tickets[id].stage != Stage.Resolved) revert WrongStage();
        undeliverable[id] = true;
        emit Undeliverable(id);
    }

    /// @notice Anyone, once a resolved ticket's delivery can no longer confirm — past its deadline plus `REFUND_GRACE` with the
    /// custodian's `markUndeliverable`, or past its deadline plus `LATE_DELIVERY_WINDOW` without — on both clocks, with no delivery
    /// recorded: the frog goes back to the set and one whole token is minted back to the ticket's owner. Without this, a custodian
    /// that stops would destroy tokens for nothing and the supply would silently stop matching the backing. Until then a late
    /// delivery is recorded as usual, so there is no refund for it to race. Also a ticket still pinned `PIN_TIMEOUT` after its
    /// pin (E-03): nobody resolved it, no frog was drawn, the token comes back and resolution moves past it.
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
        if (!undeliverable[id] && !Wait.elapsed(uint64(t.deliverBy + LATE_DELIVERY_WINDOW),
            uint64(t.deliverByBlock + Wait.blocksFor(LATE_DELIVERY_WINDOW, parentBlockSeconds)))) revert NotUndeliverable();
        t.stage = Stage.Refunded;
        openTickets--;
        Rental storage r = _rentals[t.acquisitionId];
        if (r.state == RentalState.LeavingDrawn) {
            r.state = RentalState.Ready;                 // released before: it goes home, not back into the set
            emit RentalReady(t.acquisitionId);
        } else {
            if (r.state == RentalState.ActiveDrawn) r.state = RentalState.Active;
            _pending.push(Pending(t.acquisitionId, _tickets.length));
        }
        _mint(t.owner, ONE_ITEM);
        emit Refunded(id, t.acquisitionId, t.owner);
    }

    // ---- views ---------------------------------------------------------------------------------------------------------------

    function ticket(uint256 id) external view returns (Ticket memory) { return _tickets[id]; }
    /// @notice Whether `id` was refunded (IWrappedCollection).
    function refunded(uint256 id) external view returns (bool) { return _tickets[id].stage == Stage.Refunded; }
    function ticketCount() external view returns (uint256) { return _tickets.length; }
    function rentalState(uint256 frogId) external view returns (RentalState) { return _rentals[frogId].state; }
    function rentalOf(uint256 frogId) external view returns (Rental memory) { return _rentals[frogId]; }
    function rentalCount() external view returns (uint256) { return _rentalIds.length; }
    function rentalAt(uint256 i) external view returns (uint256) { return _rentalIds[i]; }
    function leavingCount() external view returns (uint256) { return _leaving.length - leavingHead; }
    /// @notice Frogs drawable by the next ticket to resolve, before it promotes anything pending.
    function freeCount() external view returns (uint256) { return _free.length; }
    function freeAt(uint256 i) external view returns (uint256) { return _free[i]; }
    function pendingCount() external view returns (uint256) { return _pending.length - pendingHead; }
    /// @notice The `i`-th frog still waiting to become drawable, and its stamp: a ticket numbered below `stamp` cannot draw it.
    function pendingAt(uint256 i) external view returns (uint256 acquisitionId, uint256 stamp) {
        Pending storage p = _pending[pendingHead + i];
        return (p.acquisitionId, p.stamp);
    }
}
