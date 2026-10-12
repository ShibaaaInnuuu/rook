// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {ListingRegistry} from "./ListingRegistry.sol";
import {ILauncher} from "./interfaces/ILauncher.sol";
import {Wait} from "./lib/Wait.sol";
import {IEtchVault} from "./interfaces/IEtchVault.sol";

/// @title AutoBuyerVault — etched v2
/// @notice Holds one listing's ETH and lets it out one way only: to ONE address fixed when the vault is built, which is the EVM
/// address of the autonomous buyer described in `docs/v2/autonomous-buyer.md` — a program on the Internet Computer whose signing
/// key is held by a subnet and by no person. The buyer ANNOUNCES a purchase in public, bounded by a fresh price cap the keeper
/// posted; an hour later it may PULL that ETH to itself and nowhere else; and it must SETTLE with a Bitcoin receipt inside a
/// deadline, or the vault announces nothing more. Money that never reached Bitcoin comes back by anybody's call. There is no
/// withdraw function.
///
/// @dev This is v1's `AssetVault` with the purchaser replaced. What is IDENTICAL: how a coin's fees are attributed to it, the
/// MIN_ENTRY queue, the price cap, the public hour, the receipt rules, the membership check, and the absence of any way out for
/// the ETH. What is GONE: the bond, and the separate attestor. A bond existed because a person could walk off with a release;
/// a threshold key cannot walk off, so the bound is structural instead — `maxInFlight` and `MAX_OPEN` cap what one bug can
/// cost, and `buyer` is immutable, so there is no key anybody can rotate to point the money somewhere else.
///
/// What this does NOT protect against, because no contract on this chain can:
///  - a bug in the buyer program, a bridge that takes the ETH and never pays out, or a venue that lies about what a UTXO
///    carries. Each is bounded by `maxInFlight`, not to zero, and ends as a written-off purchase that is on the record for ever.
///  - a FALSE RECEIPT. In v1 a second key vouched for the spending; here the buyer posts its own receipt, and the contract can
///    only check what is checkable on this chain: the inscription is in the listing's committed collection, it is not already
///    held, and money for units that never arrived is back in the vault. That the Bitcoin transaction is real, and paid the
///    listing's public address, is checkable by anyone against Bitcoin and by nothing here. See `docs/v2/trust-model.md`.
///  - a cap posted too high. The cap is the keeper's word about a market this chain cannot read; it expires after an hour.
///  - the buyer being lost — a canister deleted, a subnet gone. `buyer` is immutable by design (changing it would be the one
///    lever that could redirect the money), so a lost buyer strands whatever is in the vault for ever. That is the cost of the
///    immutability, stated out loud rather than hedged.
///  - ETH forced in without attribution (selfdestruct): never spendable, which is safe and wasteful.
contract AutoBuyerVault is IEtchVault, ReentrancyGuard {
    using Wait for uint64;
    ListingRegistry public immutable registry;
    uint256 public immutable listingId;
    bool public immutable isRune;
    uint8 public immutable divisibility;

    /// @notice The autonomous buyer's own EVM address, and the ONLY address `pull` can ever send to. Fixed here for the life of
    /// the vault: changing it means a new vault and a new listing, which is the point.
    address payable public immutable buyer;
    /// @notice The most that may be announced-or-pulled and not yet settled at any one moment. One bug costs at most this.
    uint256 public immutable maxInFlight;

    uint256 public constant CAP_TTL = 1 hours;
    uint256 public constant PUBLIC_WAIT = 1 hours;
    uint256 public constant SETTLE_DEADLINE = 3 days;
    uint256 public constant PUBLIC_WRITEOFF_AFTER = 7 days;
    /// @notice How long past its ready hour an announcement nobody pulled may sit before ANYONE may cancel it. A buyer that
    /// stops between `announce` and `pull` otherwise holds a MAX_OPEN slot and its share of `maxInFlight` until the owner acts,
    /// and if the owner is gone too, for ever. Nothing has left the vault at that point, so letting a stranger tidy it up costs
    /// nothing and removes a state only two keys could leave.
    uint256 public constant PUBLIC_CANCEL_AFTER = 7 days;
    uint256 public constant MAX_OPEN = 3;

    enum Status { None, Announced, Pulled, Settled, Returned, WrittenOff }

    struct Purchase {
        Status status;
        uint64 readyAt;            // when the announced pull may execute, by the sequencer's clock
        uint64 readyAtBlock;       // ...and by the parent chain's, which the sequencer cannot run forward. Both must pass.
        uint64 deadline;           // when an unsettled pull becomes overdue
        bool late;                 // settled, but after its deadline
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

    /// @notice docs/phase3.md H1, unchanged from v1. The queue holds COINS, each at most once, in the order their unspent fees
    /// first reached MIN_ENTRY. A coin's fees under that wait, attributed to it and not yet spendable, so nobody can lengthen
    /// the queue with dust.
    uint256 public constant MIN_ENTRY = 0.01 ether;
    uint256 public constant MAX_PAYERS = 64;
    address[] private _queue;
    uint256 public queueHead;
    mapping(address coin => uint256) public unspentOf;
    mapping(address coin => bool) public queued;
    uint256 public attributed;        // wei of queued coins: spendable, and owed to a coin's purpose
    uint256 public waiting;           // wei of coins still under MIN_ENTRY
    uint256 public inFlight;          // announced-or-pulled and not yet settled, returned or written off
    uint256 public totalWrittenOff;
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
    event Announced(uint256 indexed id, uint256 maxEth, uint256 quantity, uint64 readyAt, string capRef);
    event Charged(uint256 indexed id, address indexed coin, uint256 amount);
    event Pulled(uint256 indexed id, address indexed buyer, uint256 amount, uint64 deadline);
    event Receipted(uint256 indexed id, uint256 indexed acquisitionId, address indexed coin, string assetRef, uint256 amount, bytes32 btcTxid, uint64 satsPaid);
    event Settled(uint256 indexed id, bool late);
    event Returned(uint256 indexed id, uint256 amount, bool closed);
    event WrittenOff(uint256 indexed id, uint256 amount, bytes32 reasonRef);

    error NotKeeper();
    error NotBuyer();
    error NotOwner();
    error NotRouter();
    error NotTreasury();
    error UnknownCoin();
    error ListingPaused();
    error CapExpired();
    error OverCap(uint256 maxEth, uint256 bound);
    error Insufficient(uint256 maxEth, uint256 available);
    error OverInFlight(uint256 maxEth, uint256 headroom);
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
    error TooManyPayers();
    error ReturnUnspentFirst(uint256 atLeast);

    constructor(ListingRegistry registry_, uint256 listingId_, bool isRune_, uint8 divisibility_,
        address payable buyer_, uint256 maxInFlight_)
    {
        if (buyer_ == address(0) || maxInFlight_ == 0) revert ZeroAmount();
        registry = registry_;
        listingId = listingId_;
        isRune = isRune_;
        divisibility = divisibility_;
        buyer = buyer_;
        maxInFlight = maxInFlight_;
    }

    modifier onlyKeeper() { if (msg.sender != registry.keeper()) revert NotKeeper(); _; }
    modifier onlyBuyer() { if (msg.sender != buyer) revert NotBuyer(); _; }
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

    /// @notice The treasury's share of every coin's fees, attributed to the protocol token. docs/token/protocol-token-v1.md §2.
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

    // ---- the keeper: a bound on spending, and no way to move money --------------------------------------------------------------

    function postCap(uint256 capWei_) external onlyKeeper {
        if (capWei_ == 0) revert ZeroAmount();
        capWei = capWei_;
        capExpiry = uint64(block.timestamp + CAP_TTL);
        emit CapPosted(capWei_, capExpiry);
    }

    /// @notice The most an announcement for `quantity` may ask for under the posted cap.
    function boundFor(uint256 quantity) public view returns (uint256) {
        return isRune ? (capWei * quantity) / (10 ** uint256(divisibility)) : capWei * quantity;
    }

    /// @notice How many coins an announcement of `amount` would charge right now (0: not enough is spendable).
    function payersFor(uint256 amount) external view returns (uint256 payers) {
        uint256 left = amount;
        for (uint256 i = queueHead; left != 0; ++i) {
            if (i == _queue.length) return 0;
            uint256 b = unspentOf[_queue[i]];
            left -= b < left ? b : left;
            ++payers;
        }
    }

    // ---- the buyer: say it out loud, wait an hour, then pull to ITSELF ---------------------------------------------------------

    /// @notice The buyer states what it is about to buy and what it will spend. The coins that will pay are chosen and CHARGED
    /// here, not an hour later at `pull`.
    /// @dev The public hour is measured on BOTH clocks. On this chain the sequencer may stamp a block up to an hour in the
    /// future, which is exactly the length of this wait, so a timestamp-only check could be satisfied in the block after the
    /// announcement and the hour anybody has to object would be gone. See `Wait`.
    ///
    /// That the coins are charged here is deliberate, and it is the fix for the v1 defect where two matured declarations raced for the head of the
    /// queue and whichever lost could need more than MAX_PAYERS coins — permanently, because the queue only moves forward. With
    /// the charge list fixed at announcement there is no ordering between two open purchases to exploit: each already owns the
    /// coins that will pay for it, and `TooManyPayers` can only be hit here, where the buyer can simply announce less.
    function announce(uint256 maxEth, uint256 quantity, string calldata capRef) external onlyBuyer returns (uint256 id) {
        if (registry.isPaused(listingId)) revert ListingPaused();
        if (maxEth == 0 || quantity == 0) revert BadQuantity();
        if (block.timestamp > capExpiry) revert CapExpired();
        uint256 bound = boundFor(quantity);
        if (maxEth > bound) revert OverCap(maxEth, bound);
        (bool blocked, uint256 by) = isBlocked();
        if (blocked) revert BlockedByOverdue(by);
        if (_open.length >= MAX_OPEN) revert TooManyOpen();
        if (maxEth > maxInFlight - inFlight) revert OverInFlight(maxEth, maxInFlight - inFlight);
        if (maxEth > attributed) revert Insufficient(maxEth, attributed);

        id = _purchases.length;
        uint64 readyAt = uint64(block.timestamp + PUBLIC_WAIT);
        uint64 readyAtBlock = uint64(block.number + Wait.blocksFor(PUBLIC_WAIT));
        _purchases.push(Purchase(Status.Announced, readyAt, readyAtBlock, 0, false, maxEth, quantity, 0, 0, capRef));
        _open.push(id);
        inFlight += maxEth;
        _charge(id, maxEth);
        emit Announced(id, maxEth, quantity, readyAt, capRef);
    }

    /// @dev Walks the queue head-first and debits the coins that will pay. A coin drained to zero leaves the queue; new fees
    /// re-queue it at the tail. Bounded by MAX_PAYERS so every later loop over the charge list has a ceiling.
    function _charge(uint256 id, uint256 amount) private {
        uint256 left = amount;
        uint256 payers;
        Charge[] storage cs = _charges[id];
        while (left != 0) {
            if (++payers > MAX_PAYERS) revert TooManyPayers();
            address coin = _queue[queueHead];
            uint256 b = unspentOf[coin];
            uint256 take = b < left ? b : left;
            unspentOf[coin] = b - take;
            left -= take;
            cs.push(Charge(coin, take, 0, 0));
            chargedTo[coin] += take;
            emit Charged(id, coin, take);
            if (b == take) { queued[coin] = false; queueHead++; }
        }
        attributed -= amount;
    }

    /// @notice The ONLY function that sends a purchase's ETH out, and it can only send it to `buyer`.
    function pull(uint256 id) external onlyBuyer nonReentrant {
        if (registry.isPaused(listingId)) revert ListingPaused();
        Purchase storage p = _purchases[id];
        if (p.status != Status.Announced) revert WrongStatus();
        if (!Wait.elapsed(p.readyAt, p.readyAtBlock)) revert TooEarly(p.readyAt);
        p.status = Status.Pulled;
        p.deadline = uint64(block.timestamp + SETTLE_DEADLINE);
        deadlineBlock[id] = uint64(block.number + Wait.blocksFor(SETTLE_DEADLINE));
        emit Pulled(id, buyer, p.maxEth, p.deadline);
        (bool ok,) = buyer.call{value: p.maxEth}("");
        if (!ok) revert SendFailed();
    }

    // ---- the receipt ------------------------------------------------------------------------------------------------------------

    /// @notice One inscription of a pulled purchase, receipted. It must be in the listing's committed collection and not already
    /// held — the two things this chain can actually check about a Bitcoin delivery. It is charged to whichever paying coin has
    /// the most paid-for value not yet matched by an inscription.
    function settleInscription(uint256 id, string calldata inscriptionId, bytes32 btcTxid, uint64 satsPaid, bytes32[] calldata proof)
        external onlyBuyer returns (uint256 acquisitionId)
    {
        if (isRune) revert WrongKind();
        Purchase storage p = _purchases[id];
        if (p.status != Status.Pulled) revert WrongStatus();
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
    /// proportion to what each paid; the last takes the rounding. Nothing accumulates here — the purchase settles in this call.
    function settleRune(uint256 id, uint256 amountReceived, bytes32 btcTxid, uint64 satsPaid) external onlyBuyer {
        if (!isRune) revert WrongKind();
        Purchase storage p = _purchases[id];
        if (p.status != Status.Pulled) revert WrongStatus();
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

    /// @notice An inscription purchase that bought fewer than it declared is closed once what did arrive is receipted.
    function settleShort(uint256 id) external onlyBuyer {
        if (isRune) revert WrongKind();
        Purchase storage p = _purchases[id];
        if (p.status != Status.Pulled || p.settledQuantity == 0) revert WrongStatus();
        _requireUnitPrice(p, p.settledQuantity);
        _settled(id, p);
    }

    /// @notice docs/phase3.md H2. A purchase closes only if what was kept is within the ANNOUNCED price per unit for what
    /// actually arrived. Whatever was announced for units that never came has to be back in the vault first.
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

    /// @dev A receipt is accepted after the deadline too, and flagged `late`, because a delivery that really happened belongs on
    /// the record. What being late costs is only that the vault was frozen in the meantime, and that anyone could have written
    /// the purchase off first — after which this reverts, which is the race the deadline is for.
    function _settled(uint256 id, Purchase storage p) private {
        p.status = Status.Settled;
        p.late = _overdue(id, 0);
        inFlight -= p.maxEth - p.returned;
        _close(id);
        emit Settled(id, p.late);
    }

    // ---- money back -------------------------------------------------------------------------------------------------------------

    /// @notice ETH the purchase did not need comes back to the coins that paid, in proportion, as fresh contributions.
    /// ANYONE may send it: the buyer program exposes a public refund whose destination is fixed to this vault, so making the
    /// coins whole needs nobody's permission. An announcement that was never pulled is cancelled here instead — there is nothing
    /// to send back, the ETH never left — and only the buyer or the owner may do that, until PUBLIC_CANCEL_AFTER has passed,
    /// when anyone may.
    function returnFunds(uint256 id) external payable nonReentrant {
        Purchase storage p = _purchases[id];
        if (p.status == Status.Announced) {
            // anyone, once PUBLIC_CANCEL_AFTER has passed on both clocks (strictly, as before, on the timestamp)
            if (msg.sender != buyer && msg.sender != registry.owner() && !Wait.elapsed(
                uint64(uint256(p.readyAt) + PUBLIC_CANCEL_AFTER + 1), uint64(p.readyAtBlock + Wait.blocksFor(PUBLIC_CANCEL_AFTER)))) revert NotBuyer();
            if (msg.value != 0) revert BadQuantity();
            p.status = Status.Returned;
            inFlight -= p.maxEth;
            _giveBack(id, p.maxEth, p.maxEth);
            _close(id);
            emit Returned(id, 0, true);
            return;
        }
        if (p.status != Status.Pulled && p.status != Status.Settled && p.status != Status.WrittenOff) revert WrongStatus();
        if (msg.value == 0 || p.returned + msg.value > p.maxEth) revert BadQuantity();
        p.returned += msg.value;
        if (p.status == Status.Pulled) inFlight -= msg.value;   // a purchase that closed already left `inFlight` when it did
        // money that finds its way home after a write-off un-writes exactly that much of it. The loss was recorded as what the
        // coins had paid and not got back; if some of it comes back, that sentence is no longer true and the record should say
        // so. Bounded by the guard above: a purchase can never return more than it was charged, so this cannot underflow.
        if (p.status == Status.WrittenOff) totalWrittenOff -= msg.value;
        _giveBack(id, p.maxEth, msg.value);
        // Only the buyer or the owner may close a purchase by paying it off. A STRANGER may always make the coins whole —
        // that is the point of the permissionless refund — but if a stranger's payment also closed the purchase, anyone could
        // spend `maxEth` to race a receipt: the buyer really bought the asset, the purchase closes as Returned, and the
        // acquisition is never recorded, so the asset can never be distributed. Expensive griefing, and permanent. Left open,
        // a fully-repaid purchase simply settles as normal, or goes overdue and writes off the zero that is still outstanding.
        bool closed = p.status == Status.Pulled && p.returned == p.maxEth
            && (msg.sender == buyer || msg.sender == registry.owner());
        if (closed) { p.status = Status.Returned; _close(id); }
        emit Returned(id, msg.value, closed);
    }

    /// @dev Gives `amount` of a purchase back to the coins that paid it, in proportion, and NEVER gives a coin back more than
    /// that coin was charged. The clamp is the whole point, and the reason is v1's C1: a purchase can be given back in
    /// instalments, a proportional share truncates, and a `last one takes the remainder` rule hands the last coin more than its
    /// share. Do that twice and `chargedTo` underflows and every later instalment reverts, stranding the refund. So each charge
    /// carries what it has already had back, the proportional pass may give it no more than the rest, and a second pass hands
    /// whatever truncation left over to the coins that still have room. The two passes always place the whole amount, because
    /// `_charge` charges until nothing is left (the charges sum to `maxEth` exactly) and no caller gives back more in total.
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

    /// @notice Closes a pulled purchase whose receipt never came, so the vault can work again, and records the loss for ever.
    /// There is no bond to pay it: what the coins paid and did not get back stays on `chargedTo` as the true record that those
    /// coins paid for nothing. The owner may do this once the deadline has passed; seven days later ANYONE may, so that an
    /// owner who would rather not admit a loss cannot keep the vault frozen by doing nothing.
    function writeOff(uint256 id, bytes32 reasonRef) external nonReentrant {
        Purchase storage p = _purchases[id];
        if (p.status != Status.Pulled) revert WrongStatus();
        if (!_overdue(id, 0)) revert NotOverdue();
        if (msg.sender != registry.owner() && !_overdue(id, PUBLIC_WRITEOFF_AFTER)) revert NotOwner();
        uint256 lost = p.maxEth - p.returned;
        p.status = Status.WrittenOff;
        inFlight -= lost;
        totalWrittenOff += lost;
        _close(id);
        emit WrittenOff(id, lost, reasonRef);
    }

    function _close(uint256 id) private {
        uint256 n = _open.length;
        for (uint256 i = 0; i < n; ++i) {
            if (_open[i] == id) { _open[i] = _open[n - 1]; _open.pop(); return; }
        }
    }

    // ---- views ----------------------------------------------------------------------------------------------------------------

    /// @dev Strictly past purchase `id`'s settle deadline plus `extra`, on both clocks: the timestamp's strict `>` is kept, and the
    /// parent chain must be past its half too. A sequencer stamping ahead cannot make the buyer late, overdue or written off early.
    function _overdue(uint256 id, uint256 extra) private view returns (bool) {
        return Wait.elapsed(uint64(uint256(_purchases[id].deadline) + extra + 1), uint64(deadlineBlock[id] + Wait.blocksFor(extra)));
    }

    /// @notice True while any pulled purchase is past its deadline without a receipt. Nothing can be announced until it is
    /// settled, returned or written off.
    function isBlocked() public view returns (bool, uint256) {
        for (uint256 i = 0; i < _open.length; ++i) {
            Purchase storage p = _purchases[_open[i]];
            if (p.status == Status.Pulled && _overdue(_open[i], 0)) return (true, _open[i]);
        }
        return (false, 0);
    }

    function headroom() external view returns (uint256) { return maxInFlight - inFlight; }
    function purchase(uint256 id) external view returns (Purchase memory) { return _purchases[id]; }
    function purchaseCount() external view returns (uint256) { return _purchases.length; }
    function chargesOf(uint256 id) external view returns (Charge[] memory) { return _charges[id]; }
    function openPurchases() external view returns (uint256[] memory) { return _open; }
    function acquisition(uint256 id) external view returns (IEtchVault.Acquisition memory) { return _acquisitions[id]; }
    function acquisitionCount() external view returns (uint256) { return _acquisitions.length; }
    function queueLength() external view returns (uint256) { return _queue.length; }
    function queueAt(uint256 i) external view returns (address) { return _queue[i]; }
}
