// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {MerkleProof} from "@openzeppelin/contracts/utils/cryptography/MerkleProof.sol";
import {ListingRegistry} from "./ListingRegistry.sol";
import {AssetVault} from "./AssetVault.sol";
import {ILauncher} from "./interfaces/ILauncher.sol";
import {IRandomnessBeacon} from "./interfaces/IRandomnessBeacon.sol";
import {Wait} from "./lib/Wait.sol";
import {BtcAddress} from "./lib/BtcAddress.sol";

/// @title Distributor
/// @notice Decides who gets what the vaults acquired. The creator of the coin that paid for an acquisition chooses its fate:
/// hold it, never deliver it, draw a winner (tickets by balance, or one a wallet), name a recipient — or, for rune units, pay
/// holders pro rata by Merkle claim. Draws are settled by a drand round that did not exist when it was pinned.
/// @dev What this does NOT protect against, stated as plainly as the design does:
///  - THE HOLDER SNAPSHOT IS THE KEEPER'S WORD. A ticket list or a share list is published with its root and cannot be used for
///    15 minutes, during which anyone can recompute it and the owner can cancel it. No contract rejects a wrong root.
///  - Delivery happens on Bitcoin. This contract records the address the winner chose and the txid the keeper reports, and lets
///    anyone flag a delivery that is late. It cannot make the keeper send.
///  - A destination's checksum is verified; that somebody holds its key, or that their wallet will not spend an inscription as
///    ordinary bitcoin, is not.
contract Distributor {
    ListingRegistry public immutable registry;
    IRandomnessBeacon public immutable beacon;

    uint256 public constant CHALLENGE = 15 minutes;
    uint256 public constant REDRAW_AFTER = 24 hours;
    /// @notice docs/phase3.md M2. Twenty rounds, a minute. A draw is pinned to `roundAt(block.timestamp) + PIN_AHEAD`, and on an L2 the
    /// sequencer sets block.timestamp: measured here it runs ~1.3 s behind the wall clock, and nothing stops it running further.
    /// At two rounds (3–6 s) a timestamp six seconds stale would pin a round whose value is already public, and whoever calls
    /// first could choose when. A minute costs a draw nothing and puts that out of reach of any lag that would not be an outage.
    uint64 public constant PIN_AHEAD = 20;
    uint256 public constant DELIVERY_DEADLINE = 7 days;
    uint256 public constant CLAIM_PERIOD = 30 days;
    uint256 public constant MINIMUM_TTL = 24 hours;
    uint256 public constant DESTINATION_CORRECTION = 1 hours;   // docs/phase3.md L3
    uint256 public constant VOID_AFTER = 7 days;                // docs/phase3.md M3

    enum Mode { Unset, Hold, Never, DrawWeighted, DrawPerWallet, Named, ProRata }
    enum Stage { None, Open, Pinned, Resolved, Claimed, Delivered }

    struct Lot {
        Mode mode;
        Stage stage;
        bool overdue;
        uint64 challengeEnds;
        uint64 challengeEndsBlock;   // the parent chain's half of the same window. See lib/Wait.sol.
        uint64 round;
        uint64 pinnedAt;
        uint64 destinationAt;
        uint64 destinationAtBlock;   // ditto: a winner's hour to correct an address is inside the sequencer's tolerance
        address winner;
        bytes32 root;
        uint256 totalTickets;
        uint256 winningTicket;
        bytes32 deliveryTxid;
        string uri;
        string destination;
    }

    struct Distribution {
        bool open;
        bool rolled;
        uint64 challengeEnds;
        uint64 challengeEndsBlock;
        uint64 claimsEnd;
        bytes32 root;
        uint256 total;       // this acquisition's units plus whatever rolled over from the coin's earlier distributions
        uint256 claimed;
        string uri;
    }

    struct Payout {
        address holder;
        bool overdue;
        uint64 requestedAt;
        uint256 listingId;
        uint256 amount;
        bytes32 txid;
        string destination;
    }

    mapping(uint256 listingId => mapping(uint256 acquisitionId => Lot)) private _lots;
    mapping(uint256 listingId => mapping(uint256 acquisitionId => Distribution)) private _distributions;
    mapping(uint256 listingId => mapping(uint256 acquisitionId => mapping(address holder => bool))) public shareClaimed;
    mapping(uint256 listingId => mapping(address coin => uint256)) public carry;          // unclaimed units waiting for the coin's next distribution
    mapping(uint256 listingId => mapping(address holder => uint256)) public accrued;      // claimed units not yet big enough, or not yet requested
    mapping(uint256 listingId => uint256) public minimumUnits;
    mapping(uint256 listingId => uint64) public minimumExpiry;
    Payout[] private _payouts;
    /// @notice docs/phase3.md M3. Declared last so that no earlier variable's storage slot moved.
    mapping(uint256 listingId => mapping(uint256 acquisitionId => uint64)) public resolvedAt;
    /// @notice docs/audit/README.md "Single-clock waits": the parent chain's half of each wait below (lib/Wait.sol). Declared after
    /// everything else, and outside the structs, so no slot moved and nothing a view returns changed shape.
    mapping(uint256 listingId => mapping(uint256 acquisitionId => uint64)) public claimsEndBlock;
    mapping(uint256 listingId => mapping(uint256 acquisitionId => uint64)) public pinnedAtBlock;
    mapping(uint256 listingId => mapping(uint256 acquisitionId => uint64)) public resolvedAtBlock;
    mapping(uint256 payoutId => uint64) public payoutRequestedAtBlock;

    event Chosen(uint256 indexed listingId, uint256 indexed acquisitionId, Mode mode, address named);
    event DrawOpened(uint256 indexed listingId, uint256 indexed acquisitionId, bytes32 root, uint256 totalTickets, string uri, uint64 challengeEnds);
    event DrawCancelled(uint256 indexed listingId, uint256 indexed acquisitionId);
    event DrawPinned(uint256 indexed listingId, uint256 indexed acquisitionId, uint64 round);
    event DrawResolved(uint256 indexed listingId, uint256 indexed acquisitionId, uint256 winningTicket);
    event Won(uint256 indexed listingId, uint256 indexed acquisitionId, address indexed winner);
    event DrawVoided(uint256 indexed listingId, uint256 indexed acquisitionId, uint256 unclaimedTicket);
    event DestinationSet(uint256 indexed listingId, uint256 indexed acquisitionId, string destination);
    event Delivered(uint256 indexed listingId, uint256 indexed acquisitionId, bytes32 txid);
    event DeliveryOverdue(uint256 indexed listingId, uint256 indexed acquisitionId);
    event DistributionOpened(uint256 indexed listingId, uint256 indexed acquisitionId, bytes32 root, uint256 total, string uri, uint64 challengeEnds);
    event DistributionCancelled(uint256 indexed listingId, uint256 indexed acquisitionId);
    event ShareClaimed(uint256 indexed listingId, uint256 indexed acquisitionId, address indexed holder, uint256 amount);
    event RolledOver(uint256 indexed listingId, uint256 indexed acquisitionId, address indexed coin, uint256 amount);
    event MinimumPosted(uint256 indexed listingId, uint256 units, uint64 expiry);
    event PayoutRequested(uint256 indexed payoutId, uint256 indexed listingId, address indexed holder, uint256 amount, string destination);
    event PayoutRecorded(uint256 indexed payoutId, bytes32 txid);
    event PayoutOverdue(uint256 indexed payoutId);

    error NotCreator();
    error NotKeeper();
    error NotOwner();
    error NotWinner();
    error WrongMode();
    error WrongStage();
    error ChallengeWindow(uint64 until);
    error WindowOver();
    error TooEarly();
    error BadProof();
    error BadAddress();
    error BadInput();
    error AlreadyClaimed();
    error OverTotal();
    error BelowMinimum(uint256 accrued, uint256 minimum);
    error MinimumExpired();
    error NotOverdue();
    error ClaimsClosed();
    error DestinationLocked();

    /// @notice The parent chain's block time on this chain (lib/Wait.sol): deployment config, fixed here.
    uint256 public immutable parentBlockSeconds;

    constructor(ListingRegistry registry_, IRandomnessBeacon beacon_, uint256 parentBlockSeconds_) {
        Wait.checkBlockSeconds(parentBlockSeconds_);
        parentBlockSeconds = parentBlockSeconds_;
        registry = registry_;
        beacon = beacon_;
    }

    modifier onlyKeeper() { if (msg.sender != registry.keeper()) revert NotKeeper(); _; }
    modifier onlyOwner() { if (msg.sender != registry.owner()) revert NotOwner(); _; }

    function _acq(uint256 listingId, uint256 acquisitionId) private view returns (AssetVault vault, AssetVault.Acquisition memory a) {
        vault = AssetVault(registry.vaultOf(listingId));
        a = vault.acquisition(acquisitionId);      // reverts if there is no such acquisition
    }

    // ---- the creator's choice ---------------------------------------------------------------------------------------------------

    /// @notice Only the creator of the coin that paid. `Hold` can be revisited; every other choice is final.
    function choose(uint256 listingId, uint256 acquisitionId, Mode mode, address named) external {
        (AssetVault vault, AssetVault.Acquisition memory a) = _acq(listingId, acquisitionId);
        if (ILauncher(registry.launcher()).creatorOf(a.coin) != msg.sender) revert NotCreator();
        Lot storage lot = _lots[listingId][acquisitionId];
        if ((lot.mode != Mode.Unset && lot.mode != Mode.Hold) || lot.stage != Stage.None) revert WrongStage();
        if (mode == Mode.Unset) revert WrongMode();
        if (mode == Mode.ProRata && !vault.isRune()) revert WrongMode();
        if ((mode == Mode.Named) != (named != address(0))) revert BadInput();
        lot.mode = mode;
        emit Chosen(listingId, acquisitionId, mode, named);
        if (mode == Mode.Named) {
            lot.winner = named;
            lot.stage = Stage.Claimed;
            emit Won(listingId, acquisitionId, named);
        }
    }

    // ---- draws --------------------------------------------------------------------------------------------------------------------

    /// @notice The keeper publishes the ticket list: `root` over leaves (holder, firstTicket, endTicket), `uri` where the whole list is.
    function openDraw(uint256 listingId, uint256 acquisitionId, bytes32 root, uint256 totalTickets, string calldata uri) external onlyKeeper {
        Lot storage lot = _lots[listingId][acquisitionId];
        if (lot.mode != Mode.DrawWeighted && lot.mode != Mode.DrawPerWallet) revert WrongMode();
        if (lot.stage != Stage.None) revert WrongStage();
        if (root == bytes32(0) || totalTickets == 0 || bytes(uri).length == 0) revert BadInput();
        lot.stage = Stage.Open;
        lot.root = root;
        lot.totalTickets = totalTickets;
        lot.uri = uri;
        lot.challengeEnds = uint64(block.timestamp + CHALLENGE);
        lot.challengeEndsBlock = uint64(block.number + Wait.blocksFor(CHALLENGE, parentBlockSeconds));
        emit DrawOpened(listingId, acquisitionId, root, totalTickets, uri, lot.challengeEnds);
    }

    /// @notice The owner's only lever over a draw, and only while the snapshot is still under challenge.
    function cancelDraw(uint256 listingId, uint256 acquisitionId) external onlyOwner {
        Lot storage lot = _lots[listingId][acquisitionId];
        if (lot.stage != Stage.Open) revert WrongStage();
        if (Wait.elapsed(lot.challengeEnds, lot.challengeEndsBlock)) revert WindowOver();
        lot.stage = Stage.None;
        lot.root = bytes32(0);
        lot.totalTickets = 0;
        lot.uri = "";
        emit DrawCancelled(listingId, acquisitionId);
    }

    /// @notice Anyone. Pins the draw to a round that has not happened yet; once pinned it cannot be changed or cancelled.
    function requestDraw(uint256 listingId, uint256 acquisitionId) external {
        Lot storage lot = _lots[listingId][acquisitionId];
        if (lot.stage != Stage.Open) revert WrongStage();
        if (!Wait.elapsed(lot.challengeEnds, lot.challengeEndsBlock)) revert ChallengeWindow(lot.challengeEnds);
        _pin(listingId, acquisitionId, lot);
    }

    /// @notice Anyone, if a pinned round is still unproven a day later (drand skipped it): pins a fresh one.
    function redraw(uint256 listingId, uint256 acquisitionId) external {
        Lot storage lot = _lots[listingId][acquisitionId];
        if (lot.stage != Stage.Pinned) revert WrongStage();
        bool waited = Wait.elapsed(uint64(lot.pinnedAt + REDRAW_AFTER),
            uint64(pinnedAtBlock[listingId][acquisitionId] + Wait.blocksFor(REDRAW_AFTER, parentBlockSeconds)));
        if (!waited || beacon.randomnessOf(lot.round) != bytes32(0)) revert TooEarly();
        _pin(listingId, acquisitionId, lot);
    }

    function _pin(uint256 listingId, uint256 acquisitionId, Lot storage lot) private {
        lot.stage = Stage.Pinned;
        lot.round = beacon.roundAt(block.timestamp) + PIN_AHEAD;
        lot.pinnedAt = uint64(block.timestamp);
        pinnedAtBlock[listingId][acquisitionId] = uint64(block.number);
        emit DrawPinned(listingId, acquisitionId, lot.round);
    }

    /// @notice Anyone delivers the round's signature (or nothing, if somebody already proved that round). The result is the same whoever does.
    function resolve(uint256 listingId, uint256 acquisitionId, bytes calldata signature) external {
        Lot storage lot = _lots[listingId][acquisitionId];
        if (lot.stage != Stage.Pinned) revert WrongStage();
        bytes32 randomness = beacon.randomnessOf(lot.round);
        if (randomness == bytes32(0)) randomness = beacon.prove(lot.round, signature);
        lot.winningTicket = uint256(keccak256(abi.encode(randomness, listingId, acquisitionId))) % lot.totalTickets;
        lot.stage = Stage.Resolved;
        resolvedAt[listingId][acquisitionId] = uint64(block.timestamp);
        resolvedAtBlock[listingId][acquisitionId] = uint64(block.number);
        emit DrawResolved(listingId, acquisitionId, lot.winningTicket);
    }

    /// @notice docs/phase3.md M3. A winning ticket that nobody can prove — a list with a gap, or a total larger than the list —
    /// would leave the lot resolved for ever. Anyone can claim for a real winner, so seven days unclaimed means the list was
    /// defective: the owner may then void the draw, which returns the lot to the keeper for a NEW public snapshot and a new
    /// future round. It cannot name a winner, and it is slow and public on purpose: it is also a second draw.
    function voidDraw(uint256 listingId, uint256 acquisitionId) external onlyOwner {
        Lot storage lot = _lots[listingId][acquisitionId];
        if (lot.stage != Stage.Resolved) revert WrongStage();
        if (!Wait.elapsed(uint64(resolvedAt[listingId][acquisitionId] + VOID_AFTER),
            uint64(resolvedAtBlock[listingId][acquisitionId] + Wait.blocksFor(VOID_AFTER, parentBlockSeconds)))) revert TooEarly();
        emit DrawVoided(listingId, acquisitionId, lot.winningTicket);
        lot.stage = Stage.None;
        lot.root = bytes32(0);
        lot.totalTickets = 0;
        lot.winningTicket = 0;
        lot.round = 0;
        lot.uri = "";
    }

    /// @notice Anyone proves whose ticket range holds the winning ticket.
    function claimWin(uint256 listingId, uint256 acquisitionId, address winner, uint256 firstTicket, uint256 endTicket, bytes32[] calldata proof) external {
        Lot storage lot = _lots[listingId][acquisitionId];
        if (lot.stage != Stage.Resolved) revert WrongStage();
        if (winner == address(0) || lot.winningTicket < firstTicket || lot.winningTicket >= endTicket) revert BadProof();
        if (!MerkleProof.verifyCalldata(proof, lot.root, ticketLeaf(winner, firstTicket, endTicket))) revert BadProof();
        lot.winner = winner;
        lot.stage = Stage.Claimed;
        emit Won(listingId, acquisitionId, winner);
    }

    function ticketLeaf(address holder, uint256 firstTicket, uint256 endTicket) public pure returns (bytes32) {
        return keccak256(bytes.concat(keccak256(abi.encode(holder, firstTicket, endTicket))));
    }

    // ---- delivery of a won or named lot -------------------------------------------------------------------------------------------

    /// @notice Only the winner names where their asset goes, and may correct it until it is sent.
    function setDestination(uint256 listingId, uint256 acquisitionId, string calldata taprootAddress) external {
        Lot storage lot = _lots[listingId][acquisitionId];
        if (lot.stage != Stage.Claimed) revert WrongStage();
        if (msg.sender != lot.winner) revert NotWinner();
        if (!BtcAddress.isTaproot(taprootAddress)) revert BadAddress();
        // docs/phase3.md L3: a correction is allowed for an hour after the first address, and the keeper sends after that hour,
        // so the address it reads is the address that stays. The delivery clock runs from the first address.
        if (_destinationLocked(lot)) revert DestinationLocked();
        lot.destination = taprootAddress;
        if (lot.destinationAt == 0) { lot.destinationAt = uint64(block.timestamp); lot.destinationAtBlock = uint64(block.number); }
        emit DestinationSet(listingId, acquisitionId, taprootAddress);
    }

    /// @dev The winner's hour to correct a Bitcoin address, on both clocks. An hour is exactly the sequencer's permitted
    /// forward tolerance (see lib/Wait.sol), so a timestamp-only check could lock a wrong address in the next block and send
    /// the asset somewhere the winner cannot reach.
    function _destinationLocked(Lot storage lot) private view returns (bool) {
        if (lot.destinationAt == 0) return false;
        return Wait.elapsed(
            uint64(lot.destinationAt + DESTINATION_CORRECTION),
            uint64(lot.destinationAtBlock + Wait.blocksFor(DESTINATION_CORRECTION, parentBlockSeconds))
        );
    }

    function recordDelivery(uint256 listingId, uint256 acquisitionId, bytes32 txid) external onlyKeeper {
        Lot storage lot = _lots[listingId][acquisitionId];
        if (lot.stage != Stage.Claimed || lot.destinationAt == 0) revert WrongStage();
        if (!_destinationLocked(lot)) revert TooEarly();                 // the address can still change
        if (txid == bytes32(0)) revert BadInput();
        lot.deliveryTxid = txid;
        lot.stage = Stage.Delivered;
        emit Delivered(listingId, acquisitionId, txid);
    }

    function flagOverdue(uint256 listingId, uint256 acquisitionId) external {
        Lot storage lot = _lots[listingId][acquisitionId];
        if (lot.stage != Stage.Claimed || lot.destinationAt == 0) revert NotOverdue();
        // strictly after the deadline on the timestamp, as before, and past it on the parent chain too
        if (!Wait.elapsed(uint64(lot.destinationAt + DELIVERY_DEADLINE + 1),
            uint64(lot.destinationAtBlock + Wait.blocksFor(DELIVERY_DEADLINE, parentBlockSeconds)))) revert NotOverdue();
        lot.overdue = true;
        emit DeliveryOverdue(listingId, acquisitionId);
    }

    // ---- rune units, pro rata -----------------------------------------------------------------------------------------------------

    /// @notice The keeper publishes who gets how much: `root` over leaves (holder, units). The total is fixed by the chain, not by the
    /// keeper: this acquisition's units plus whatever the coin's earlier distributions left unclaimed.
    function openDistribution(uint256 listingId, uint256 acquisitionId, bytes32 root, string calldata uri) external onlyKeeper {
        (, AssetVault.Acquisition memory a) = _acq(listingId, acquisitionId);
        Lot storage lot = _lots[listingId][acquisitionId];
        Distribution storage d = _distributions[listingId][acquisitionId];
        if (lot.mode != Mode.ProRata) revert WrongMode();
        if (d.open || d.rolled) revert WrongStage();
        if (root == bytes32(0) || bytes(uri).length == 0) revert BadInput();
        d.open = true;
        d.root = root;
        d.uri = uri;
        d.total = a.amount + carry[listingId][a.coin];
        carry[listingId][a.coin] = 0;
        d.challengeEnds = uint64(block.timestamp + CHALLENGE);
        d.challengeEndsBlock = uint64(block.number + Wait.blocksFor(CHALLENGE, parentBlockSeconds));
        d.claimsEnd = uint64(block.timestamp + CHALLENGE + CLAIM_PERIOD);
        claimsEndBlock[listingId][acquisitionId] = uint64(block.number + Wait.blocksFor(CHALLENGE + CLAIM_PERIOD, parentBlockSeconds));
        emit DistributionOpened(listingId, acquisitionId, root, d.total, uri, d.challengeEnds);
    }

    function cancelDistribution(uint256 listingId, uint256 acquisitionId) external onlyOwner {
        (, AssetVault.Acquisition memory a) = _acq(listingId, acquisitionId);
        Distribution storage d = _distributions[listingId][acquisitionId];
        if (!d.open) revert WrongStage();
        if (Wait.elapsed(d.challengeEnds, d.challengeEndsBlock)) revert WindowOver();
        carry[listingId][a.coin] += d.total - a.amount;
        delete _distributions[listingId][acquisitionId];
        emit DistributionCancelled(listingId, acquisitionId);
    }

    /// @notice Anyone may submit a holder's proof: it only ever adds to that holder's own accrued units.
    function claimShare(uint256 listingId, uint256 acquisitionId, address holder, uint256 units, bytes32[] calldata proof) external {
        Distribution storage d = _distributions[listingId][acquisitionId];
        if (!d.open) revert WrongStage();
        if (!Wait.elapsed(d.challengeEnds, d.challengeEndsBlock)) revert ChallengeWindow(d.challengeEnds);
        if (d.rolled || Wait.elapsed(d.claimsEnd, claimsEndBlock[listingId][acquisitionId])) revert ClaimsClosed();
        if (shareClaimed[listingId][acquisitionId][holder]) revert AlreadyClaimed();
        if (holder == address(0) || units == 0) revert BadInput();
        if (!MerkleProof.verifyCalldata(proof, d.root, shareLeaf(holder, units))) revert BadProof();
        if (d.claimed + units > d.total) revert OverTotal();          // a root that promises more than was bought cannot pay it out
        shareClaimed[listingId][acquisitionId][holder] = true;
        d.claimed += units;
        accrued[listingId][holder] += units;
        emit ShareClaimed(listingId, acquisitionId, holder, units);
    }

    function shareLeaf(address holder, uint256 units) public pure returns (bytes32) {
        return keccak256(bytes.concat(keccak256(abi.encode(holder, units))));
    }

    /// @notice Anyone, once claims have closed: what nobody claimed waits for the same coin's next distribution.
    function rollOver(uint256 listingId, uint256 acquisitionId) external {
        (, AssetVault.Acquisition memory a) = _acq(listingId, acquisitionId);
        Distribution storage d = _distributions[listingId][acquisitionId];
        if (!d.open || d.rolled) revert WrongStage();
        if (!Wait.elapsed(d.claimsEnd, claimsEndBlock[listingId][acquisitionId])) revert TooEarly();
        d.rolled = true;
        uint256 unclaimed = d.total - d.claimed;
        carry[listingId][a.coin] += unclaimed;
        emit RolledOver(listingId, acquisitionId, a.coin, unclaimed);
    }

    /// @notice The least that is worth sending on Bitcoin: an output carrying runes needs its own dust (546 sats) and its share of a fee.
    function postMinimum(uint256 listingId, uint256 units) external onlyKeeper {
        if (units == 0) revert BadInput();
        minimumUnits[listingId] = units;
        minimumExpiry[listingId] = uint64(block.timestamp + MINIMUM_TTL);
        emit MinimumPosted(listingId, units, minimumExpiry[listingId]);
    }

    /// @notice A holder asks for everything they have accrued on a listing, if it has reached the posted minimum. Below it, it keeps accruing.
    function requestPayout(uint256 listingId, string calldata taprootAddress) external returns (uint256 payoutId) {
        uint256 amount = accrued[listingId][msg.sender];
        if (block.timestamp > minimumExpiry[listingId]) revert MinimumExpired();
        if (amount == 0 || amount < minimumUnits[listingId]) revert BelowMinimum(amount, minimumUnits[listingId]);
        if (!BtcAddress.isTaproot(taprootAddress)) revert BadAddress();
        accrued[listingId][msg.sender] = 0;
        payoutId = _payouts.length;
        _payouts.push(Payout(msg.sender, false, uint64(block.timestamp), listingId, amount, bytes32(0), taprootAddress));
        payoutRequestedAtBlock[payoutId] = uint64(block.number);
        emit PayoutRequested(payoutId, listingId, msg.sender, amount, taprootAddress);
    }

    function recordPayout(uint256 payoutId, bytes32 txid) external onlyKeeper {
        Payout storage p = _payouts[payoutId];
        if (p.txid != bytes32(0) || txid == bytes32(0)) revert BadInput();
        p.txid = txid;
        emit PayoutRecorded(payoutId, txid);
    }

    function flagPayoutOverdue(uint256 payoutId) external {
        Payout storage p = _payouts[payoutId];
        if (p.txid != bytes32(0)) revert NotOverdue();
        if (!Wait.elapsed(uint64(p.requestedAt + DELIVERY_DEADLINE + 1),
            uint64(payoutRequestedAtBlock[payoutId] + Wait.blocksFor(DELIVERY_DEADLINE, parentBlockSeconds)))) revert NotOverdue();
        p.overdue = true;
        emit PayoutOverdue(payoutId);
    }

    // ---- views --------------------------------------------------------------------------------------------------------------------

    function lot(uint256 listingId, uint256 acquisitionId) external view returns (Lot memory) { return _lots[listingId][acquisitionId]; }
    function distribution(uint256 listingId, uint256 acquisitionId) external view returns (Distribution memory) { return _distributions[listingId][acquisitionId]; }
    function payout(uint256 payoutId) external view returns (Payout memory) { return _payouts[payoutId]; }
    function payoutCount() external view returns (uint256) { return _payouts.length; }
}
