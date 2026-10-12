// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {MerkleProof} from "@openzeppelin/contracts/utils/cryptography/MerkleProof.sol";
import {IRandomnessBeacon} from "../interfaces/IRandomnessBeacon.sol";
import {Wait} from "../lib/Wait.sol";
import {Policy, INftRegistry, INftLauncher, INftVault} from "./INft.sol";

/// @title NftVaultBase
/// @notice What both NFT vaults share (docs/v4/nft.md §3, §4, §7): money in from a coin's own router, attributed to that coin; the
/// AssetVault ledger (a first-in-first-out queue of coins, nobody able to stuff it with dust); a keeper's price cap that expires on
/// both clocks; and the fate of what is bought — a drand raffle over a challengeable snapshot of the paying coin's holders, or a
/// burn. The fate is the listing's, fixed at construction: one vault never holds both kinds of money.
/// @dev What differs between the two vaults — how a purchase happens and how an NFT physically leaves — is in `NftSweepVault` (4663,
/// Seaport, checked here) and `NftRemoteVault` (another chain, bought by the canister). Neither has a withdraw function.
abstract contract NftVaultBase is INftVault, ReentrancyGuard {
    INftRegistry public immutable registry;
    uint256 public immutable listingId;
    uint64 public immutable chainId;
    address public immutable collection;
    Policy public immutable policy;
    IRandomnessBeacon public immutable beacon;

    uint256 public constant CAP_TTL = 1 hours;
    uint256 public constant MIN_ENTRY = 0.01 ether;
    uint256 public constant MAX_PAYERS = 64;
    uint256 public constant CHALLENGE = 15 minutes;
    uint64 public constant PIN_AHEAD = 20;                  // Distributor.PIN_AHEAD, for the same reason
    uint256 public constant REDRAW_AFTER = 24 hours;
    uint256 public constant VOID_AFTER = 7 days;
    address public constant DEAD = 0x000000000000000000000000000000000000dEaD;

    // ---- the ledger: AssetVault's, unchanged in meaning ---------------------------------------------------------------------------

    address[] private _queue;
    uint256 public queueHead;
    mapping(address coin => uint256) public unspentOf;
    mapping(address coin => bool) public queued;
    uint256 public attributed;      // wei of queued coins: spendable, owed to the listing's purpose
    uint256 public waiting;         // wei of coins still under MIN_ENTRY: attributed to them, not yet spendable
    mapping(address coin => uint256) public contributedBy;
    mapping(address coin => uint256) public chargedTo;

    struct Charge { address coin; uint256 weiCharged; uint256 assignedWei; uint256 givenBackWei; }
    mapping(uint256 purchaseId => Charge[]) internal _charges;

    // ---- the cap --------------------------------------------------------------------------------------------------------------

    uint256 public capWei;          // the most that may be paid per NFT
    uint64 public capExpiry;
    uint64 public capExpiryBlock;   // the parent chain's half of the hour. See lib/Wait.sol.

    // ---- H12 (docs/v5/custody-hardening.md): the keeper's cap moves in steps ----
    /// @notice While the last cap is fresh, a new one is within ±20% of it, and at most one post per `MIN_POST_GAP` (both clocks).
    uint256 public constant MAX_STEP_BPS = 2_000;
    uint256 public constant MIN_POST_GAP = 10 minutes;
    uint256 public constant CEILING_DELAY = 7 days;
    /// @notice Once the registry's owner has set one (seven public days), no cap may exceed it, fresh or after a lapse.
    uint256 public capCeiling;
    uint256 public pendingCeiling;
    uint64 public pendingCeilingAt;
    uint64 public pendingCeilingAtBlock;

    // ---- what was bought, and its fate -----------------------------------------------------------------------------------------

    struct Acquisition {
        uint256 tokenId;
        address fundedBy;           // the coin that paid most of its price: whose holders its raffle is for
        uint256 pricePaid;
        uint256 purchaseId;
        bytes32 purchaseRef;        // 4663: the block number it was swept in. Elsewhere: the canister's purchase tx hash.
    }

    enum Stage { Held, Open, Pinned, Resolved, Won, Delivered, Burned }

    struct Lot {
        Stage stage;
        bool overdue;
        uint64 challengeEnds;
        uint64 challengeEndsBlock;
        uint64 round;
        uint64 at;                  // when the lot entered its current stage (acquired, pinned, resolved or won)
        uint64 atBlock;             // ...by the parent chain's clock
        address winner;
        address destination;        // where a remote NFT goes: the winner's own address unless they changed it in the first hour
        bytes32 root;
        uint256 totalTickets;
        uint256 winningTicket;
        bytes32 settleTx;           // the delivery or burn transaction on the collection's chain (remote only)
        string uri;
    }

    Acquisition[] private _acquisitions;
    mapping(uint256 acquisitionId => Lot) private _lots;

    event Deposited(address indexed coin, uint256 amount);
    event CapPosted(uint256 capWei, uint64 expiry, string capRef);
    event CapCeilingProposed(uint256 ceiling, uint64 readyAt);
    event CapCeilingSet(uint256 ceiling);
    event Charged(uint256 indexed purchaseId, address indexed coin, uint256 amount);
    event Acquired(uint256 indexed acquisitionId, uint256 indexed tokenId, address indexed fundedBy, uint256 pricePaid, uint256 purchaseId, bytes32 purchaseRef);
    event DrawOpened(uint256 indexed acquisitionId, bytes32 root, uint256 totalTickets, string uri, uint64 challengeEnds);
    event DrawCancelled(uint256 indexed acquisitionId);
    event DrawPinned(uint256 indexed acquisitionId, uint64 round);
    event DrawResolved(uint256 indexed acquisitionId, uint256 winningTicket);
    event DrawVoided(uint256 indexed acquisitionId, uint256 unclaimedTicket);
    event Won(uint256 indexed acquisitionId, address indexed winner);
    event Burned(uint256 indexed acquisitionId, uint256 indexed tokenId, bytes32 txHash);

    error NotKeeper();
    error NotOwner();
    error NotRouter();
    error ZeroAmount();
    error ListingPaused();
    error CapExpired();
    error TooManyPayers();
    error WrongPolicy();
    error WrongStage();
    error BadInput();
    error BadProof();
    error ChallengeWindow(uint64 until);
    error WindowOver();
    error TooEarly();
    error NoDirectPayments();
    error CapStep(uint256 cap, uint256 lo, uint256 hi);
    error CapTooSoon();
    error OverCeiling(uint256 cap, uint256 ceiling);

    constructor(INftRegistry registry_, uint256 listingId_, uint64 chainId_, address collection_, Policy policy_,
        IRandomnessBeacon beacon_)
    {
        registry = registry_;
        listingId = listingId_;
        chainId = chainId_;
        collection = collection_;
        policy = policy_;
        beacon = beacon_;
    }

    modifier onlyKeeper() { if (msg.sender != registry.keeper()) revert NotKeeper(); _; }
    modifier onlyOwner() { if (msg.sender != registry.owner()) revert NotOwner(); _; }

    // ---- money in -------------------------------------------------------------------------------------------------------------

    /// @notice A coin's fees arrive, from that coin's own router and nobody else, and only for a coin launched on THIS listing.
    /// @dev nonReentrant shares the lock with a sweep: a seller paid mid-sweep cannot harvest a router into this vault and make the
    /// ETH the sweep measures look smaller than what left.
    function deposit(address coin) external payable nonReentrant {
        INftLauncher l = INftLauncher(registry.launcher());
        if (l.routerOf(coin) != msg.sender || l.listingOf(coin) != listingId) revert NotRouter();
        if (msg.value == 0) revert ZeroAmount();
        _credit(coin, msg.value);
        contributedBy[coin] += msg.value;
        emit Deposited(coin, msg.value);
    }

    function _credit(address coin, uint256 amount) internal {
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

    /// @dev Charges `amount` to the queued coins, first in first out, and records who paid what under `purchaseId`.
    function _chargeFifo(uint256 purchaseId, uint256 amount) internal {
        uint256 left = amount;
        uint256 payers;
        Charge[] storage cs = _charges[purchaseId];
        while (left != 0) {
            if (++payers > MAX_PAYERS) revert TooManyPayers();
            address coin = _queue[queueHead];
            uint256 b = unspentOf[coin];
            uint256 take = b < left ? b : left;
            unspentOf[coin] = b - take;
            left -= take;
            cs.push(Charge(coin, take, 0, 0));
            chargedTo[coin] += take;
            emit Charged(purchaseId, coin, take);
            if (b == take) { queued[coin] = false; queueHead++; }
        }
        attributed -= amount;
    }

    /// @dev The coin that has paid most, under `purchaseId`, that no NFT has yet been assigned to. Its gap shrinks by `price`.
    /// Attribution by fees earned: whoever's money bought it is whose holders may win it.
    function _attribute(uint256 purchaseId, uint256 price) internal returns (address) {
        Charge[] storage cs = _charges[purchaseId];
        uint256 best;
        uint256 bestGap;
        for (uint256 i = 0; i < cs.length; ++i) {
            uint256 gap = cs[i].weiCharged > cs[i].assignedWei ? cs[i].weiCharged - cs[i].assignedWei : 0;
            if (gap > bestGap) { bestGap = gap; best = i; }
        }
        cs[best].assignedWei += price;
        return cs[best].coin;
    }

    function _record(uint256 tokenId, address coin, uint256 price, uint256 purchaseId, bytes32 ref) internal returns (uint256 aid) {
        aid = _acquisitions.length;
        _acquisitions.push(Acquisition(tokenId, coin, price, purchaseId, ref));
        Lot storage lot = _lots[aid];
        lot.at = uint64(block.timestamp);
        lot.atBlock = uint64(block.number);
        emit Acquired(aid, tokenId, coin, price, purchaseId, ref);
    }

    // ---- the cap --------------------------------------------------------------------------------------------------------------

    function postCap(uint256 capWei_, string calldata capRef) external onlyKeeper {
        if (capWei_ == 0) revert ZeroAmount();
        if (capFresh()) {
            // H12: a step, never a jump, and not more often than every ten minutes on both clocks
            uint64 at = capExpiry - uint64(CAP_TTL);
            uint64 atBlock = capExpiryBlock - uint64(Wait.blocksFor(CAP_TTL));
            if (!Wait.elapsed(uint64(at + MIN_POST_GAP), uint64(atBlock + Wait.blocksFor(MIN_POST_GAP)))) revert CapTooSoon();
            uint256 lo = capWei - (capWei * MAX_STEP_BPS) / 10_000;
            uint256 hi = capWei + (capWei * MAX_STEP_BPS) / 10_000;
            if (capWei_ < lo || capWei_ > hi) revert CapStep(capWei_, lo, hi);
        }
        if (capCeiling != 0 && capWei_ > capCeiling) revert OverCeiling(capWei_, capCeiling);
        capWei = capWei_;
        capExpiry = uint64(block.timestamp + CAP_TTL);
        capExpiryBlock = uint64(block.number + Wait.blocksFor(CAP_TTL));
        emit CapPosted(capWei_, capExpiry, capRef);
    }

    /// @notice The registry's owner (the timelock): lower the ceiling at once, or propose a higher one (or the first) that anyone
    /// may accept after `CEILING_DELAY` on both clocks.
    function proposeCapCeiling(uint256 ceiling) external onlyOwner {
        if (ceiling == 0) revert ZeroAmount();
        if (capCeiling != 0 && ceiling <= capCeiling) {
            capCeiling = ceiling;
            pendingCeiling = 0;
            emit CapCeilingSet(ceiling);
            return;
        }
        pendingCeiling = ceiling;
        pendingCeilingAt = uint64(block.timestamp + CEILING_DELAY);
        pendingCeilingAtBlock = uint64(block.number + Wait.blocksFor(CEILING_DELAY));
        emit CapCeilingProposed(ceiling, pendingCeilingAt);
    }

    function acceptCapCeiling() external {
        uint256 c = pendingCeiling;
        if (c == 0) revert WrongStage();
        if (!Wait.elapsed(pendingCeilingAt, pendingCeilingAtBlock)) revert TooEarly();
        pendingCeiling = 0;
        capCeiling = c;
        emit CapCeilingSet(c);
    }

    /// @notice Fresh on BOTH clocks: a sequencer stamping blocks an hour ahead cannot keep a stale cap alive, nor a stalled parent
    /// chain one whose hour has passed.
    function capFresh() public view returns (bool) {
        return block.timestamp <= capExpiry && block.number <= capExpiryBlock;
    }

    // ---- raffle: the Distributor's machinery, around one ERC-721 lot ---------------------------------------------------------------

    /// @notice The keeper publishes the ticket list of the paying coin's holders: `root` over (holder, firstTicket, endTicket).
    function openDraw(uint256 acquisitionId, bytes32 root, uint256 totalTickets, string calldata uri) external onlyKeeper {
        if (policy != Policy.Raffle) revert WrongPolicy();
        _acq(acquisitionId);
        Lot storage lot = _lots[acquisitionId];
        if (lot.stage != Stage.Held) revert WrongStage();
        if (root == bytes32(0) || totalTickets == 0 || bytes(uri).length == 0) revert BadInput();
        lot.stage = Stage.Open;
        lot.root = root;
        lot.totalTickets = totalTickets;
        lot.uri = uri;
        lot.challengeEnds = uint64(block.timestamp + CHALLENGE);
        lot.challengeEndsBlock = uint64(block.number + Wait.blocksFor(CHALLENGE));
        emit DrawOpened(acquisitionId, root, totalTickets, uri, lot.challengeEnds);
    }

    /// @notice The owner's only lever, and only while the snapshot is under challenge.
    function cancelDraw(uint256 acquisitionId) external onlyOwner {
        Lot storage lot = _lots[acquisitionId];
        if (lot.stage != Stage.Open) revert WrongStage();
        if (Wait.elapsed(lot.challengeEnds, lot.challengeEndsBlock)) revert WindowOver();
        _reopen(lot);
        emit DrawCancelled(acquisitionId);
    }

    /// @notice Anyone, once the challenge window is over on both clocks: pins a drand round that does not exist yet.
    function requestDraw(uint256 acquisitionId) external {
        Lot storage lot = _lots[acquisitionId];
        if (lot.stage != Stage.Open) revert WrongStage();
        if (!Wait.elapsed(lot.challengeEnds, lot.challengeEndsBlock)) revert ChallengeWindow(lot.challengeEnds);
        _pin(acquisitionId, lot);
    }

    /// @notice Anyone, if the pinned round is still unproven a day later (drand skipped it).
    function redraw(uint256 acquisitionId) external {
        Lot storage lot = _lots[acquisitionId];
        if (lot.stage != Stage.Pinned) revert WrongStage();
        if (!Wait.elapsed(uint64(lot.at + REDRAW_AFTER), uint64(lot.atBlock + Wait.blocksFor(REDRAW_AFTER)))
            || beacon.randomnessOf(lot.round) != bytes32(0)) revert TooEarly();
        _pin(acquisitionId, lot);
    }

    function _pin(uint256 acquisitionId, Lot storage lot) private {
        lot.stage = Stage.Pinned;
        lot.round = beacon.roundAt(block.timestamp) + PIN_AHEAD;
        _stamp(lot);
        emit DrawPinned(acquisitionId, lot.round);
    }

    /// @notice Anyone delivers the round's signature. The result is the same whoever does.
    function resolve(uint256 acquisitionId, bytes calldata signature) external {
        Lot storage lot = _lots[acquisitionId];
        if (lot.stage != Stage.Pinned) revert WrongStage();
        bytes32 randomness = beacon.randomnessOf(lot.round);
        if (randomness == bytes32(0)) randomness = beacon.prove(lot.round, signature);
        lot.winningTicket = uint256(keccak256(abi.encode(randomness, address(this), acquisitionId))) % lot.totalTickets;
        lot.stage = Stage.Resolved;
        _stamp(lot);
        emit DrawResolved(acquisitionId, lot.winningTicket);
    }

    /// @notice A winning ticket nobody can prove for seven days means the list was defective: the owner may send the lot back for a
    /// NEW public snapshot and a new future round. It cannot name a winner.
    function voidDraw(uint256 acquisitionId) external onlyOwner {
        Lot storage lot = _lots[acquisitionId];
        if (lot.stage != Stage.Resolved) revert WrongStage();
        if (!Wait.elapsed(uint64(lot.at + VOID_AFTER), uint64(lot.atBlock + Wait.blocksFor(VOID_AFTER)))) revert TooEarly();
        emit DrawVoided(acquisitionId, lot.winningTicket);
        _reopen(lot);
    }

    /// @notice Anyone proves whose ticket range holds the winning ticket. On 4663 the NFT goes to the winner in this call.
    function claimWin(uint256 acquisitionId, address winner, uint256 firstTicket, uint256 endTicket, bytes32[] calldata proof)
        external nonReentrant
    {
        Lot storage lot = _lots[acquisitionId];
        if (lot.stage != Stage.Resolved) revert WrongStage();
        if (winner == address(0) || lot.winningTicket < firstTicket || lot.winningTicket >= endTicket) revert BadProof();
        if (!MerkleProof.verifyCalldata(proof, lot.root, ticketLeaf(winner, firstTicket, endTicket))) revert BadProof();
        lot.winner = winner;
        lot.destination = winner;
        lot.stage = Stage.Won;
        _stamp(lot);
        emit Won(acquisitionId, winner);
        _onWin(acquisitionId, lot, winner);
    }

    /// @notice The same leaf as `Distributor.ticketLeaf`, so the keeper's snapshot tool serves both.
    function ticketLeaf(address holder, uint256 firstTicket, uint256 endTicket) public pure returns (bytes32) {
        return keccak256(bytes.concat(keccak256(abi.encode(holder, firstTicket, endTicket))));
    }

    /// @dev What a win does to the NFT itself. 4663: transfer it. Elsewhere: nothing yet; the canister delivers and records it.
    function _onWin(uint256 acquisitionId, Lot storage lot, address winner) internal virtual;

    function _reopen(Lot storage lot) private {
        lot.stage = Stage.Held;
        lot.root = bytes32(0);
        lot.totalTickets = 0;
        lot.winningTicket = 0;
        lot.round = 0;
        lot.uri = "";
    }

    function _stamp(Lot storage lot) internal {
        lot.at = uint64(block.timestamp);
        lot.atBlock = uint64(block.number);
    }

    /// @dev The burn half. The caller has checked the policy; this is shared so both vaults mark a burn the same way.
    function _markBurned(uint256 acquisitionId, bytes32 txHash) internal returns (Acquisition memory a) {
        if (policy != Policy.Burn) revert WrongPolicy();
        a = _acq(acquisitionId);
        Lot storage lot = _lots[acquisitionId];
        if (lot.stage != Stage.Held) revert WrongStage();
        lot.stage = Stage.Burned;
        lot.settleTx = txHash;
        emit Burned(acquisitionId, a.tokenId, txHash);
    }

    function _acq(uint256 acquisitionId) internal view returns (Acquisition memory) {
        return _acquisitions[acquisitionId];      // reverts (panic 0x32) if there is no such acquisition
    }

    function _lot(uint256 acquisitionId) internal view returns (Lot storage) {
        return _lots[acquisitionId];
    }

    // ---- views ----------------------------------------------------------------------------------------------------------------

    function acquisition(uint256 id) external view returns (Acquisition memory) { return _acquisitions[id]; }
    function acquisitionCount() external view returns (uint256) { return _acquisitions.length; }
    function lot(uint256 id) external view returns (Lot memory) { return _lots[id]; }
    function chargesOf(uint256 purchaseId) external view returns (Charge[] memory) { return _charges[purchaseId]; }
    function queueLength() external view returns (uint256) { return _queue.length; }
    function queueAt(uint256 i) external view returns (address) { return _queue[i]; }
    /// @notice Everything the vault owes to a purpose. Its balance is never below this (the invariant tests hold it to that).
    function accounted() public view virtual returns (uint256) { return attributed + waiting; }
}
