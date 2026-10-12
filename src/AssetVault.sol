// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {ListingRegistry} from "./ListingRegistry.sol";
import {ILauncher} from "./interfaces/ILauncher.sol";
import {Wait} from "./lib/Wait.sol";
import {IEtchVault} from "./interfaces/IEtchVault.sol";

/// @title AssetVault
/// @notice Holds one listing's ETH and lets it out one way only: a purchase the keeper DECLARES in public, bounded by a fresh
/// price cap, which anyone may RELEASE to the purchaser an hour later unless the owner cancelled it, and which a DIFFERENT key must
/// SETTLE with a Bitcoin receipt inside a deadline — or the vault declares nothing more. There is no withdraw function.
/// One contract for both kinds of listing: `isRune` decides only how a quantity is priced and what a receipt carries.
/// @dev What this does NOT protect against, because no contract on this chain can:
///  - the purchaser keeping released ETH, or buying badly. The loss is bounded by MAX_OPEN declarations, each within cap × quantity,
///    and the machine stops at the first overdue receipt; it is not bounded to zero.
///  - a false receipt. The attestor's word is checkable by anyone against the listing's public Bitcoin address, not by this contract.
///  - a cap posted too high. The cap is the keeper's word about a market this chain cannot read; it expires after an hour.
///  - ETH forced in without attribution (selfdestruct): it is never spendable, which is safe and wasteful.
///  - a coin whose lifetime fees never reach MIN_ENTRY: they wait here, attributed to it, for ever. That is the price of a queue
///    nobody can stuff with dust (docs/phase3.md H1).
contract AssetVault is IEtchVault, ReentrancyGuard {
    ListingRegistry public immutable registry;
    uint256 public immutable listingId;
    bool public immutable isRune;
    uint8 public immutable divisibility;

    uint256 public constant CAP_TTL = 1 hours;
    uint256 public constant DECLARE_DELAY = 1 hours;
    uint256 public constant SETTLE_DEADLINE = 3 days;
    uint256 public constant MAX_OPEN = 3;

    enum Status { None, Declared, Cancelled, Released, Settled, WrittenOff }

    struct Purchase {
        Status status;
        uint64 releasableAt;       // by the sequencer's clock
        uint64 releasableAtBlock;  // ...and by the parent chain's, which the sequencer cannot run forward. See lib/Wait.sol.
        uint64 deadline;
        address purchaser;
        uint256 maxEth;
        uint256 quantity;          // inscriptions: a count. rune: an amount in the rune's smallest unit
        uint256 settledQuantity;   // inscriptions: items receipted so far. rune: amount received
        uint256 returned;
        string capRef;             // where the cap came from (a URL or a hash), for whoever checks later
    }

    struct Charge { address coin; uint256 weiCharged; uint256 assignedWei; uint256 givenBackWei; }

    // the cap: wei per inscription, or wei per WHOLE rune unit (10**divisibility smallest units)
    uint256 public capWei;
    uint64 public capExpiry;

    /// @notice docs/phase3.md H1. The queue holds COINS, each at most once, in the order their unspent fees first reached
    /// MIN_ENTRY. A coin's fees under that wait, attributed to it and not yet spendable, so that nobody can lengthen the queue
    /// with dust; and a release that would need more than MAX_PAYERS coins is refused, so every loop here has a ceiling.
    uint256 public constant MIN_ENTRY = 0.01 ether;
    uint256 public constant MAX_PAYERS = 64;        // the dearest loop (a rune receipt, one acquisition per payer) is then a quarter of a 32M block
    address[] private _queue;
    uint256 public queueHead;
    mapping(address coin => uint256) public unspentOf;   // a coin's fees not yet charged to a purchase, queued or waiting
    mapping(address coin => bool) public queued;
    uint256 public attributed;     // wei of queued coins, i.e. spendable and owed to a coin's purpose
    uint256 public waiting;        // wei of coins still under MIN_ENTRY: attributed to them, not yet spendable
    uint256 public reserved;       // wei promised to declarations not yet released
    uint256 public totalWrittenOff;   // what no bond covered
    /// @notice docs/phase1.md F4. ETH posted against the purchaser's honesty, and what is released and not yet settled.
    uint256 public bond;
    uint256 public outstanding;
    /// @notice docs/phase3.md M1. Both are kept per purchaser: a purchaser rotated in cannot be released ETH against the bond of
    /// the one rotated out, and the one rotated out can still take their own bond back.
    mapping(address purchaser => uint256) public bondOf;
    mapping(address purchaser => uint256) public outstandingOf;
    uint256 public constant PUBLIC_WRITEOFF_AFTER = 7 days;   // past the receipt deadline, anyone may write a purchase off
    mapping(address coin => uint256) public contributedBy;
    mapping(address coin => uint256) public chargedTo;

    Purchase[] private _purchases;
    mapping(uint256 purchaseId => Charge[]) private _charges;
    uint256[] private _open;
    IEtchVault.Acquisition[] private _acquisitions;
    mapping(bytes32 inscriptionLeaf => bool) public inscriptionHeld;
    /// @notice docs/audit/README.md "Single-clock waits": the parent chain's half of each purchase's settle `deadline`
    /// (lib/Wait.sol). Declared last and outside `Purchase`, so `purchase()` returns exactly what it did.
    mapping(uint256 purchaseId => uint64) public deadlineBlock;

    event Deposited(address indexed coin, uint256 amount);
    event CapPosted(uint256 capWei, uint64 expiry);
    event Declared(uint256 indexed id, uint256 maxEth, uint256 quantity, uint64 releasableAt, string capRef);
    event Cancelled(uint256 indexed id);
    event Released(uint256 indexed id, address indexed purchaser, uint256 amount, uint64 deadline);
    event Charged(uint256 indexed id, address indexed coin, uint256 amount);
    event Receipted(uint256 indexed id, uint256 indexed acquisitionId, address indexed coin, string assetRef, uint256 amount, bytes32 btcTxid, uint64 satsPaid);
    event Settled(uint256 indexed id);
    event Returned(uint256 indexed id, uint256 amount);
    event WrittenOff(uint256 indexed id, uint256 amount, bytes32 reasonRef);
    event BondPosted(address indexed from, uint256 amount);
    event BondWithdrawn(address indexed to, uint256 amount);
    event BondSlashed(uint256 indexed id, uint256 amount);

    error NotKeeper();
    error NotAttestor();
    error NotOwner();
    error NotRouter();
    error NotTreasury();
    error UnknownCoin();
    error ListingPaused();
    error CapExpired();
    error OverCap(uint256 maxEth, uint256 bound);
    error Insufficient(uint256 maxEth, uint256 available);
    error TooManyOpen();
    error BlockedByOverdue(uint256 purchaseId);
    error WrongStatus();
    error TooEarly(uint64 at);
    error NotOverdue();
    error BadQuantity();
    error NotInCollection();
    error AlreadyHeld();
    error WrongKind();
    error SendFailed();
    error ZeroAmount();
    error BondTooSmall(uint256 required, uint256 posted);
    error NotPurchaser();
    error TooManyPayers();
    error ReturnUnspentFirst(uint256 atLeast);

    constructor(ListingRegistry registry_, uint256 listingId_, bool isRune_, uint8 divisibility_) {
        registry = registry_;
        listingId = listingId_;
        isRune = isRune_;
        divisibility = divisibility_;
    }

    modifier onlyKeeper() { if (msg.sender != registry.keeper()) revert NotKeeper(); _; }
    modifier onlyAttestor() { if (msg.sender != registry.attestor()) revert NotAttestor(); _; }
    modifier onlyOwner() { if (msg.sender != registry.owner()) revert NotOwner(); _; }

    // ---- money in -------------------------------------------------------------------------------------------------------------

    /// @notice A coin's fees arrive. Only that coin's own router may attribute money to it.
    function deposit(address coin) external payable {
        if (ILauncher(registry.launcher()).routerOf(coin) != msg.sender) revert NotRouter();
        if (msg.value == 0) revert ZeroAmount();
        _credit(coin, msg.value);
        contributedBy[coin] += msg.value;
        emit Deposited(coin, msg.value);
    }

    /// @notice The treasury's share of every coin's fees arrives, attributed to the protocol token, so that what this vault buys
    /// with it belongs to that token's holders. docs/token/protocol-token-v1.md §2.
    /// @dev The ONLY difference from `deposit` above is who may call it and which coin is credited: the accounting, the queue, the
    /// cap, the public wait, the bond and the absence of a withdraw are the same code and the same properties. `registry.treasury()`
    /// is set behind the registry's seven-day public delay, and the coin must be one the launcher knows, so that ETH cannot be
    /// attributed to an address with no creator, no holders and no way out.
    function depositFrom(address coin) external payable {
        if (msg.sender != registry.treasury()) revert NotTreasury();
        if (ILauncher(registry.launcher()).routerOf(coin) == address(0)) revert UnknownCoin();
        if (msg.value == 0) revert ZeroAmount();
        _credit(coin, msg.value);
        contributedBy[coin] += msg.value;
        emit Deposited(coin, msg.value);
    }

    function _credit(address coin, uint256 amount) private {
        uint256 b = unspentOf[coin] + amount;
        unspentOf[coin] = b;
        if (queued[coin]) {
            attributed += amount;
        } else if (b >= MIN_ENTRY) {
            queued[coin] = true;
            _queue.push(coin);
            attributed += b;
            waiting -= b - amount;
        } else {
            waiting += amount;
        }
    }

    /// @notice How many coins a release of `amount` would charge right now (0: not enough is spendable). The keeper reads this
    /// before declaring; a declaration that would need more than MAX_PAYERS can be cancelled by the owner and declared smaller.
    function payersFor(uint256 amount) external view returns (uint256 payers) {
        uint256 left = amount;
        for (uint256 i = queueHead; left != 0; ++i) {
            if (i == _queue.length) return 0;
            uint256 b = unspentOf[_queue[i]];
            left -= b < left ? b : left;
            ++payers;
        }
    }

    // ---- the keeper -----------------------------------------------------------------------------------------------------------

    function postCap(uint256 capWei_) external onlyKeeper {
        if (capWei_ == 0) revert ZeroAmount();
        capWei = capWei_;
        capExpiry = uint64(block.timestamp + CAP_TTL);
        emit CapPosted(capWei_, capExpiry);
    }

    /// @notice The most a declaration for `quantity` may ask for under the posted cap.
    function boundFor(uint256 quantity) public view returns (uint256) {
        return isRune ? (capWei * quantity) / (10 ** uint256(divisibility)) : capWei * quantity;
    }

    function declarePurchase(uint256 maxEth, uint256 quantity, string calldata capRef) external onlyKeeper returns (uint256 id) {
        if (registry.isPaused(listingId)) revert ListingPaused();
        if (maxEth == 0 || quantity == 0) revert BadQuantity();
        if (block.timestamp > capExpiry) revert CapExpired();
        uint256 bound = boundFor(quantity);
        if (maxEth > bound) revert OverCap(maxEth, bound);
        (bool blocked, uint256 by) = isBlocked();
        if (blocked) revert BlockedByOverdue(by);
        if (_open.length >= MAX_OPEN) revert TooManyOpen();
        uint256 available = attributed - reserved;
        if (maxEth > available) revert Insufficient(maxEth, available);

        id = _purchases.length;
        uint64 releasableAt = uint64(block.timestamp + DECLARE_DELAY);
        uint64 releasableAtBlock = uint64(block.number + Wait.blocksFor(DECLARE_DELAY));
        _purchases.push(Purchase(Status.Declared, releasableAt, releasableAtBlock, 0, registry.purchaser(), maxEth, quantity, 0, 0, capRef));
        _open.push(id);
        reserved += maxEth;
        emit Declared(id, maxEth, quantity, releasableAt, capRef);
    }

    // ---- the owner: can stop, cannot take ---------------------------------------------------------------------------------------

    function cancel(uint256 id) external onlyOwner {
        Purchase storage p = _purchases[id];
        if (p.status != Status.Declared) revert WrongStatus();
        p.status = Status.Cancelled;
        reserved -= p.maxEth;
        _close(id);
        emit Cancelled(id);
    }

    /// @notice Closes a released purchase whose receipt never came, so the vault can work again. The bond pays the coins that were
    /// charged, as far as it goes; what it does not cover is recorded for ever. The owner may do this once the receipt deadline has
    /// passed; seven days later ANYONE may, so that an owner friendly to the purchaser cannot shelter the bond by doing nothing.
    function writeOff(uint256 id, bytes32 reasonRef) external nonReentrant {
        Purchase storage p = _purchases[id];
        if (p.status != Status.Released) revert WrongStatus();
        if (!_overdue(id, 0)) revert NotOverdue();
        if (msg.sender != registry.owner() && !_overdue(id, PUBLIC_WRITEOFF_AFTER)) revert NotOwner();
        p.status = Status.WrittenOff;
        uint256 lost = p.maxEth - p.returned;
        outstanding -= lost;
        outstandingOf[p.purchaser] -= lost;
        uint256 posted = bondOf[p.purchaser];
        uint256 slash = posted < lost ? posted : lost;
        if (slash != 0) {
            bond -= slash;
            bondOf[p.purchaser] = posted - slash;
            _giveBack(id, p.maxEth, slash);
            emit BondSlashed(id, slash);
        }
        totalWrittenOff += lost - slash;
        _close(id);
        emit WrittenOff(id, lost - slash, reasonRef);
    }

    // ---- the bond -------------------------------------------------------------------------------------------------------------

    /// @notice What all bonds together must cover right now: the registry's share of everything released and not yet settled.
    function requiredBond() public view returns (uint256) {
        return (outstanding * registry.bondBps()) / 10_000;
    }

    /// @notice What one purchaser's bond must cover: the same share of what is in THEIR hands.
    function requiredBondOf(address purchaser_) public view returns (uint256) {
        return (outstandingOf[purchaser_] * registry.bondBps()) / 10_000;
    }

    /// @notice Anyone may add to the CURRENT purchaser's bond. Only that purchaser can take it out, and only what nothing in their
    /// hands needs — including after they have been rotated out.
    function postBond() external payable {
        if (msg.value == 0) revert ZeroAmount();
        bond += msg.value;
        bondOf[registry.purchaser()] += msg.value;
        emit BondPosted(msg.sender, msg.value);
    }

    function withdrawBond(uint256 amount) external nonReentrant {
        uint256 posted = bondOf[msg.sender];
        if (posted == 0) revert NotPurchaser();
        for (uint256 i = 0; i < _open.length; ++i) {
            Purchase storage q = _purchases[_open[i]];
            if (q.purchaser == msg.sender && q.status == Status.Released && _overdue(_open[i], 0)) revert BlockedByOverdue(_open[i]);
        }
        uint256 required = requiredBondOf(msg.sender);
        if (amount == 0 || amount > posted || posted - amount < required) revert BondTooSmall(required, posted);
        bond -= amount;
        bondOf[msg.sender] = posted - amount;
        (bool ok,) = msg.sender.call{value: amount}("");
        if (!ok) revert SendFailed();
        emit BondWithdrawn(msg.sender, amount);
    }

    // ---- anyone ---------------------------------------------------------------------------------------------------------------

    /// @notice Sends a matured declaration's ETH to the purchaser named when it was declared, and charges the coins that pay for it,
    /// first in first out.
    function release(uint256 id) external nonReentrant {
        Purchase storage p = _purchases[id];
        if (p.status != Status.Declared) revert WrongStatus();
        if (!Wait.elapsed(p.releasableAt, p.releasableAtBlock)) revert TooEarly(p.releasableAt);
        if (registry.isPaused(listingId)) revert ListingPaused();
        p.status = Status.Released;
        p.deadline = uint64(block.timestamp + SETTLE_DEADLINE);
        deadlineBlock[id] = uint64(block.number + Wait.blocksFor(SETTLE_DEADLINE));
        reserved -= p.maxEth;

        uint256 left = p.maxEth;
        uint256 payers;
        Charge[] storage cs = _charges[id];
        while (left != 0) {
            if (++payers > MAX_PAYERS) revert TooManyPayers();
            address coin = _queue[queueHead];
            uint256 b = unspentOf[coin];
            uint256 take = b < left ? b : left;
            unspentOf[coin] = b - take;
            left -= take;
            // a coin is in the queue once, so it is charged once per release: no search, no duplicate
            cs.push(Charge(coin, take, 0, 0));
            chargedTo[coin] += take;
            emit Charged(id, coin, take);
            if (b == take) { queued[coin] = false; queueHead++; }
        }
        attributed -= p.maxEth;
        // the purchaser's own bond must cover its share of everything in their hands, this release included
        outstanding += p.maxEth;
        outstandingOf[p.purchaser] += p.maxEth;
        uint256 required = requiredBondOf(p.purchaser);
        if (bondOf[p.purchaser] < required) revert BondTooSmall(required, bondOf[p.purchaser]);

        (bool ok,) = p.purchaser.call{value: p.maxEth}("");
        if (!ok) revert SendFailed();
        emit Released(id, p.purchaser, p.maxEth, p.deadline);
    }

    /// @notice ETH the purchase did not need comes back to the coins that paid, in proportion, as fresh contributions.
    function returnUnspent(uint256 id) external payable nonReentrant {
        Purchase storage p = _purchases[id];
        if (p.status != Status.Released && p.status != Status.Settled) revert WrongStatus();
        if (msg.value == 0 || p.returned + msg.value > p.maxEth) revert BadQuantity();
        if (p.status == Status.Released) { outstanding -= msg.value; outstandingOf[p.purchaser] -= msg.value; }
        p.returned += msg.value;
        _giveBack(id, p.maxEth, msg.value);
        emit Returned(id, msg.value);
    }

    /// @dev Gives `amount` of a purchase back to the coins that paid it, in proportion, and NEVER gives a coin back more than that
    /// coin was charged. The clamp is the whole point. A purchase can be given back in instalments — a partial refund now, the rest
    /// later, or a bond slash after a partial refund — and a proportional share truncates, so a single instalment split by the old
    /// `last one takes the remainder` rule could hand the last coin more than its share. Do that twice and the coin has been given
    /// back more than it ever paid: its `chargedTo` underflows and every later instalment reverts, stranding the refund. So each
    /// charge carries what it has already had back, the proportional pass may give it no more than the rest, and a second pass hands
    /// whatever truncation left over to the coins that still have room. Those two passes always place the whole amount, because the
    /// charges sum to `maxEth` (release charges until nothing is left) and the callers never give back more than `maxEth` in total.
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

    // ---- the attestor ---------------------------------------------------------------------------------------------------------

    /// @notice One inscription of a released purchase, receipted. It must be in the listing's committed collection and not already held.
    /// It is charged to whichever paying coin has the most paid-for value not yet matched by an inscription.
    function settleInscription(uint256 id, string calldata inscriptionId, bytes32 btcTxid, uint64 satsPaid, bytes32[] calldata proof)
        external onlyAttestor returns (uint256 acquisitionId)
    {
        if (isRune) revert WrongKind();
        Purchase storage p = _purchases[id];
        if (p.status != Status.Released) revert WrongStatus();
        if (!registry.isMember(listingId, inscriptionId, proof)) revert NotInCollection();
        bytes32 leaf = registry.leafOf(inscriptionId);
        if (inscriptionHeld[leaf]) revert AlreadyHeld();
        inscriptionHeld[leaf] = true;

        Charge[] storage cs = _charges[id];
        uint256 best;
        uint256 bestGap;
        for (uint256 i = 0; i < cs.length; ++i) {
            uint256 gap = cs[i].weiCharged > cs[i].assignedWei ? cs[i].weiCharged - cs[i].assignedWei : 0;
            if (gap > bestGap) { bestGap = gap; best = i; }
        }
        cs[best].assignedWei += p.maxEth / p.quantity;

        acquisitionId = _record(id, cs[best].coin, 1, btcTxid, satsPaid, inscriptionId);
        if (++p.settledQuantity == p.quantity) _settled(id, p);
    }

    /// @notice A rune purchase, receipted in one go: the amount that actually arrived is split between the paying coins in
    /// proportion to what each paid; the last takes the rounding.
    function settleRune(uint256 id, uint256 amountReceived, bytes32 btcTxid, uint64 satsPaid) external onlyAttestor {
        if (!isRune) revert WrongKind();
        Purchase storage p = _purchases[id];
        if (p.status != Status.Released) revert WrongStatus();
        if (amountReceived == 0) revert BadQuantity();
        _requireUnitPrice(p, amountReceived);
        string memory runeId = registry.listing(listingId).ref;
        Charge[] storage cs = _charges[id];
        uint256 given;
        for (uint256 i = 0; i < cs.length; ++i) {
            uint256 share = i == cs.length - 1 ? amountReceived - given : (amountReceived * cs[i].weiCharged) / p.maxEth;
            given += share;
            if (share != 0) _record(id, cs[i].coin, share, btcTxid, satsPaid, runeId);
        }
        p.settledQuantity = amountReceived;
        _settled(id, p);
    }

    /// @notice An inscription purchase that bought fewer than it declared is closed by the attestor once what did arrive is receipted.
    function settleShort(uint256 id) external onlyAttestor {
        if (isRune) revert WrongKind();
        Purchase storage p = _purchases[id];
        if (p.status != Status.Released || p.settledQuantity == 0) revert WrongStatus();
        _requireUnitPrice(p, p.settledQuantity);
        _settled(id, p);
    }

    /// @notice docs/phase3.md H2. A purchase is closed only if what was kept is within the DECLARED price per unit for what
    /// actually arrived: (maxEth − returned) / received ≤ maxEth / quantity. Whatever was declared for units that never came has
    /// to be back in the vault first. Until then the purchase stays open, the bond stays held, and past the deadline it is
    /// written off against that bond. `quantity` is therefore a MINIMUM the keeper commits the purchaser to, not a hope.
    function _requireUnitPrice(Purchase storage p, uint256 received) private view {
        if (received >= p.quantity) return;
        uint256 mayKeep = (p.maxEth * received) / p.quantity;
        uint256 kept = p.maxEth - p.returned;
        if (kept > mayKeep) revert ReturnUnspentFirst(kept - mayKeep);
    }

    function _record(uint256 id, address coin, uint256 amount, bytes32 txid, uint64 sats, string memory ref) private returns (uint256 aid) {
        aid = _acquisitions.length;
        _acquisitions.push(IEtchVault.Acquisition(id, coin, amount, txid, sats, ref));
        emit Receipted(id, aid, coin, ref, amount, txid, sats);
    }

    function _settled(uint256 id, Purchase storage p) private {
        outstanding -= p.maxEth - p.returned;
        outstandingOf[p.purchaser] -= p.maxEth - p.returned;
        p.status = Status.Settled;
        _close(id);
        emit Settled(id);
    }

    function _close(uint256 id) private {
        uint256 n = _open.length;
        for (uint256 i = 0; i < n; ++i) {
            if (_open[i] == id) { _open[i] = _open[n - 1]; _open.pop(); return; }
        }
    }

    /// @dev Strictly past purchase `id`'s settle deadline plus `extra`, on both clocks: the timestamp's strict `>` is kept, and the
    /// parent chain must be past its half too. A sequencer stamping ahead cannot make a purchaser overdue early.
    function _overdue(uint256 id, uint256 extra) private view returns (bool) {
        return Wait.elapsed(uint64(uint256(_purchases[id].deadline) + extra + 1), uint64(deadlineBlock[id] + Wait.blocksFor(extra)));
    }

    // ---- views ----------------------------------------------------------------------------------------------------------------

    /// @notice True while any released purchase is past its deadline without a receipt. Nothing can be declared until it is settled or written off.
    function isBlocked() public view returns (bool, uint256) {
        for (uint256 i = 0; i < _open.length; ++i) {
            Purchase storage p = _purchases[_open[i]];
            if (p.status == Status.Released && _overdue(_open[i], 0)) return (true, _open[i]);
        }
        return (false, 0);
    }

    function purchase(uint256 id) external view returns (Purchase memory) { return _purchases[id]; }
    function purchaseCount() external view returns (uint256) { return _purchases.length; }
    function chargesOf(uint256 id) external view returns (Charge[] memory) { return _charges[id]; }
    function openPurchases() external view returns (uint256[] memory) { return _open; }
    function acquisition(uint256 id) external view returns (IEtchVault.Acquisition memory) { return _acquisitions[id]; }
    function acquisitionCount() external view returns (uint256) { return _acquisitions.length; }
    function queueLength() external view returns (uint256) { return _queue.length; }
    function queueAt(uint256 i) external view returns (address) { return _queue[i]; }
}
