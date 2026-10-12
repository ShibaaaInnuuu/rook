// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {MerkleProof} from "@openzeppelin/contracts/utils/cryptography/MerkleProof.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {ListingRegistry} from "./ListingRegistry.sol";
import {IWrappedCollection} from "./interfaces/IWrappedCollection.sol";
import {ILauncher} from "./interfaces/ILauncher.sol";
import {IEtchVault} from "./interfaces/IEtchVault.sol";
import {IRandomnessBeacon} from "./interfaces/IRandomnessBeacon.sol";
import {Wait} from "./lib/Wait.sol";
import {MintDesk} from "./curve/MintDesk.sol";

/// @title SweepVault
/// @notice A sweep listing's vault: its coins' fees buy wFROG, and a policy fixed at construction decides where each whole wFROG
/// goes — to one holder of the coin that paid (RAFFLE), or to a provably unspendable Bitcoin output (BURN). docs/v3/sweep.md.
/// @dev It has no withdraw, no approve, no owner function that moves anything, and no policy setter. Every wei (`credit`) and
/// every wFROG unit (`units`, `reserved`) inside it is charged to exactly one coin; wFROG sent here by anyone else is stray and
/// stays. What this does NOT protect against:
///  - THE CAP AND THE SNAPSHOT ARE THE KEEPER'S WORD, within bounds. A sweep never pays above the posted cap, a cap never above
///    the owner's ceiling (which must exist), and an hour-old cap buys nothing. A draw's snapshot TIME is chosen by a drand round
///    nobody had seen when the draw was announced (a draw voided or cancelled before its pin hands that round to the coin's next
///    draw, so the time cannot be re-rolled), and its root commits the ticket count and that time (a sentinel leaf); the
///    ticket list itself is still the keeper's, and waits 15 minutes for anyone to recompute it and the owner or the guardian to
///    cancel it (either may cancel until the pin). Pinned rounds are final. docs/audit/2026-09-26/fixes-B-C-keeper-draws.md.
///    A snapshot with no eligible holder is opened as the sentinel alone and closes with no winner (`Empty`): that emptiness is
///    the keeper's word like any list, checked and cancellable the same way (docs/audit/2026-09-26/review2/fixes-R3-carry.md).
///  - fees earned from trades vs ETH paid into the coin's escrow: FeeRouter splits both alike. Paying in funds only that coin's
///    own raffle or burn and moves nothing about who wins.
///  - a custodian that does not deliver a burn ticket: `reclaim` restores the books after the wrapper's refund, not the exit.
/// @dev Both wrapped collections (Wrapper, NftWrapper) expose the drand beacon their redemptions use; the raffle uses the same one.
interface IBeaconOf {
    function beacon() external view returns (IRandomnessBeacon);
}

contract SweepVault is IEtchVault, ReentrancyGuard {
    using SafeERC20 for IERC20;
    enum Policy { RAFFLE, BURN, YOYO }
    enum ListingStage { None, Listed, Sold, Returned }

    /// @notice YOYO (docs/v5/policies-and-fees.md §1): one whole unit of a coin's, relisted at the markup over the keeper's cap.
    struct Listing {
        ListingStage stage;
        address coin;
        uint256 ask;          // wei: the cap at listing × (1 + markupBps)
        uint256 proceeds;     // wei the custodian reported the sale at
    }
    /// @dev `Announced` and `Empty` are last so the earlier stages keep their numbers. `Empty`: a draw opened over a snapshot with
    /// no eligible holder, closed with no winner (fixes-R3-carry); its unit went back to the coin.
    enum Stage { None, Open, Pinned, Resolved, Claimed, Cancelled, Voided, Announced, Empty }

    struct Draw {
        Stage stage;
        address coin;
        uint64 challengeEnds;
        uint64 challengeEndsBlock;
        uint64 round;
        uint64 pinnedAt;
        uint64 pinnedAtBlock;
        uint64 resolvedAt;
        uint64 resolvedAtBlock;
        bytes32 root;
        uint256 totalTickets;
        uint256 winningTicket;
        address winner;
        string uri;
        uint64 seedRound;       // the drand round, pinned at the announcement, that picks the snapshot time
        uint64 announcedAt;
        uint64 announcedAtBlock;
        uint64 snapshotAt;      // the snapshot is the last block whose timestamp is at most this (set when opened)
    }

    ListingRegistry public immutable registry;
    uint256 public immutable listingId;
    IWrappedCollection public immutable wrapper;
    IRandomnessBeacon public immutable beacon;
    Policy public immutable policy;
    /// @notice The one contract a pool-route sweep may call (zero: no pool route). Fixed here; there is no setter.
    address public immutable venue;
    /// @notice Where each coin's fee source is looked up: `routerOf(coin)` may deposit for `coin` if `listingOf(coin)` is this
    /// listing. Pons coins: the Launcher. Coins on our own curve: the CurveFactory (docs/v3/curve.md). Fixed here.
    ILauncher public immutable launcher;
    /// @notice The canister route (zero: none): the vault escrows at most one cap's worth per frog at the public mint desk.
    MintDesk public immutable desk;
    /// @notice The parent chain's block time on this chain (lib/Wait.sol): deployment config, fixed here.
    uint256 public immutable parentBlockSeconds;

    uint256 public constant ONE = 1e18;
    uint256 public constant CAP_TTL = 1 hours;
    /// @notice The snapshot time is a moment of the day before the announcement, picked by the seed round (C-04).
    uint256 public constant SNAPSHOT_WINDOW = 24 hours;
    /// @notice An announced draw the keeper has not opened this long after the announcement may be voided by anyone.
    uint256 public constant OPEN_TTL = 6 hours;
    uint256 public constant CHALLENGE = 15 minutes;
    uint64 public constant PIN_AHEAD = 20;
    /// @notice A pinned draw nobody resolved this long after its pin may be voided by anyone. The round is never replaced (C-02).
    uint256 public constant PIN_TIMEOUT = 7 days;
    uint256 public constant VOID_AFTER = 7 days;

    /// @notice ETH a coin's fees put here and no sweep has spent yet.
    mapping(address coin => uint256) public credit;
    /// @notice wFROG units a coin's fees bought or paid, not yet reserved by a draw or retired.
    mapping(address coin => uint256) public units;
    /// @notice BURN: the coin that retired each wrapper ticket, and whether a refund of it has been credited back.
    mapping(uint256 ticketId => address coin) public retiredBy;
    mapping(uint256 ticketId => bool) public reclaimed;
    /// @notice YOYO: each relist ticket's listing, and each coin's sale proceeds awaiting a buyback.
    mapping(uint256 ticketId => Listing) private _listings;
    mapping(address coin => uint256) public proceeds;
    mapping(address coin => uint256) public listedOpen;
    uint256 public totalProceeds;
    uint256 public totalBurnedBack;   // wei spent on buybacks, ever
    /// @notice YOYO: the markup over the cap a relisted item asks, bps; the registry's owner sets it within the bounds.
    uint256 public constant MIN_MARKUP_BPS = 1_000;
    uint256 public constant MAX_MARKUP_BPS = 2_000;
    uint256 public constant SALE_FEE_ALLOWANCE_BPS = 500;
    uint256 public markupBps = MIN_MARKUP_BPS;
    address public constant DEAD = 0x000000000000000000000000000000000000dEaD;
    /// @notice Canister route: the coin each desk request was made for, and the ETH escrowed there for it.
    mapping(uint256 requestId => address coin) public deskCoin;
    mapping(uint256 requestId => uint256) public deskAmount;
    uint256 public atDesk;

    uint256 public totalCredit;
    uint256 public totalUnits;
    uint256 public totalReserved;
    uint256 public retiredCount;

    uint256 public cap;              // wei per whole wFROG
    uint64 public capAt;
    uint64 public capAtBlock;

    // ---- H12 (docs/v5/custody-hardening.md): the keeper's cap moves in steps ----
    /// @notice While the last cap is fresh, a new one is within ±20% of it, and at most one post per `MIN_POST_GAP` (both clocks).
    uint256 public constant MAX_STEP_BPS = 2_000;
    uint256 public constant MIN_POST_GAP = 10 minutes;
    uint256 public constant CEILING_DELAY = 7 days;
    /// @notice No cap — fresh step or stale re-post — may exceed it, and no cap is taken until there is one (C-01). The first is
    /// the owner's at once (it only tightens an unbounded vault); a raise waits seven public days; a lowering is at once.
    uint256 public capCeiling;
    uint256 public pendingCeiling;
    uint64 public pendingCeilingAt;
    uint64 public pendingCeilingAtBlock;

    Draw[] private _draws;
    /// @notice A coin's draw before its pin (announced or open), id + 1; zero: none. One at a time per coin (E-02).
    mapping(address coin => uint256) public drawInFlight;
    /// @notice review2 N-2: a coin's seed left by a draw voided or cancelled before its pin (the origin draw's id + 1; 0: none).
    /// The coin's next announcement reuses it, so the snapshot time cannot be discarded and drawn again. A pin consumes it.
    mapping(address coin => uint256) public carriedSeed;
    /// @dev A draw's seed origin (id + 1) when it carries another draw's seed; 0: its own.
    mapping(uint256 id => uint256) private _seedOrigin;
    bool private _sweeping;
    bool private _collecting;

    event Deposited(address indexed coin, uint256 amount);
    event DepositedWrapped(address indexed coin, uint256 amount);
    event CapPosted(uint256 weiPerWrapped, uint64 at);
    event CapCeilingProposed(uint256 ceiling, uint64 readyAt);
    event CapCeilingSet(uint256 ceiling);
    event Swept(address indexed coin, address indexed caller, uint256 spent, uint256 bought);
    event DeskRequested(address indexed coin, uint256 indexed requestId, uint256 amount);
    event DeskCollected(address indexed coin, uint256 indexed requestId, uint256 ethBack, uint256 bought);
    event DrawAnnounced(uint256 indexed drawId, address indexed coin, uint64 seedRound);
    event SnapshotFixed(uint256 indexed drawId, uint64 snapshotAt);
    event SeedCarried(uint256 indexed drawId, uint256 indexed originId, uint64 seedRound);
    event DrawOpened(uint256 indexed drawId, address indexed coin, bytes32 root, uint256 totalTickets, string uri, uint64 challengeEnds);
    event DrawCancelled(uint256 indexed drawId);
    event DrawPinned(uint256 indexed drawId, uint64 round);
    event DrawResolved(uint256 indexed drawId, uint256 winningTicket);
    event Won(uint256 indexed drawId, address indexed holder);
    event DrawVoided(uint256 indexed drawId);
    event DrawEmpty(uint256 indexed drawId, address indexed coin, uint64 snapshotAt);
    event Retired(address indexed coin, uint256 indexed ticketId);
    event Reclaimed(address indexed coin, uint256 indexed ticketId);
    event Relisted(address indexed coin, uint256 indexed ticketId, uint256 ask);
    event SaleRecorded(address indexed coin, uint256 indexed ticketId, uint256 proceeds);
    event BoughtBack(address indexed coin, address indexed caller, uint256 spent, uint256 burned);
    event ListingReturned(address indexed coin, uint256 indexed ticketId);
    event MarkupSet(uint256 markupBps);

    error NotRouter();
    error NotKeeper();
    error NotOwner();
    error NotAccepted();
    error NoAcquisitions();
    error WrongPolicy();
    error WrongStage();
    error ListingPaused();
    error BadInput();
    error BadVenue();
    error CapStale();
    error CapStep(uint256 cap, uint256 lo, uint256 hi);
    error CapTooSoon();
    error OverCeiling(uint256 cap, uint256 ceiling);
    error OverCap(uint256 spent, uint256 bought, uint256 cap);
    error Slippage(uint256 bought, uint256 minOut);
    error Insufficient();
    error VenueFailed();
    error NoCeiling();
    error DrawInFlight();
    error KeeperPaused();
    error ChallengeWindow();
    error WindowOver();
    error TooEarly();
    error BadProof();
    error NotCustodian();
    error UnderAsk(uint256 proceeds, uint256 min);
    error BadMarkup();

    constructor(ListingRegistry registry_, uint256 listingId_, IWrappedCollection wrapper_, Policy policy_, address venue_, ILauncher launcher_,
        MintDesk desk_, uint256 parentBlockSeconds_)
    {
        Wait.checkBlockSeconds(parentBlockSeconds_);
        parentBlockSeconds = parentBlockSeconds_;
        if (address(registry_) == address(0) || address(wrapper_) == address(0) || address(launcher_) == address(0)) revert BadInput();
        // any wrapped collection (docs/v4/nft-wrapper.md §3): what is checked is that it has a custodian, i.e. is one
        if (wrapper_.custodian() == address(0)) revert BadInput();
        if (venue_ == address(0) && address(desk_) == address(0)) revert BadVenue();
        if (venue_ != address(0) && (venue_ == address(wrapper_) || venue_ == address(desk_) || venue_.code.length == 0)) revert BadVenue();
        if (address(desk_) != address(0) && desk_.wrapper() != wrapper_) revert BadVenue();
        registry = registry_;
        listingId = listingId_;
        wrapper = wrapper_;
        beacon = IBeaconOf(address(wrapper_)).beacon();
        policy = policy_;
        venue = venue_;
        launcher = launcher_;
        desk = desk_;
    }

    /// @dev The registry's keeper, read on every call: it follows a rotation at once, and is zero while the keeper is paused.
    modifier onlyKeeper() { if (msg.sender != registry.keeper()) revert NotKeeper(); _; }

    function _onlyRouterOf(address coin) private view {
        if (msg.sender != launcher.routerOf(coin) || launcher.listingOf(coin) != listingId || coin == address(0)) revert NotRouter();
    }

    function _notPaused() private view {
        if (registry.isPaused(listingId)) revert ListingPaused();
    }

    // ---- money in ------------------------------------------------------------------------------------------------------------

    /// @notice A coin's fees arrive in ETH (stage 4). Only that coin's router, only for a coin on this listing.
    function deposit(address coin) external payable nonReentrant {
        _onlyRouterOf(coin);
        credit[coin] += msg.value;
        totalCredit += msg.value;
        emit Deposited(coin, msg.value);
    }

    /// @notice A coin's fees arrive in wFROG (stage 3, a coin quoted in wFROG). Pulled from the router; credited as received.
    function depositWrapped(address coin, uint256 amount) external nonReentrant {
        _onlyRouterOf(coin);
        uint256 before = wrapper.balanceOf(address(this));
        wrapper.transferFrom(msg.sender, address(this), amount);
        uint256 got = wrapper.balanceOf(address(this)) - before;
        units[coin] += got;
        totalUnits += got;
        emit DepositedWrapped(coin, got);
    }

    /// @notice The Treasury's entry. Treasury money is not a coin's fees, and would mix with them here: refused.
    function depositFrom(address) external payable { revert NotAccepted(); }

    /// @notice Only the venue's refund of unspent input during a sweep, or the desk's change during a collect. A direct payment
    /// buys no say: refused.
    receive() external payable {
        if (!(_sweeping && msg.sender == venue) && !(_collecting && msg.sender == address(desk))) revert NotAccepted();
    }

    // ---- the sweep -----------------------------------------------------------------------------------------------------------

    /// @notice The keeper's ceiling, in wei per whole wFROG. Refused by `sweep` once either clock has passed `CAP_TTL`. Never
    /// above the owner's ceiling, and never taken without one (C-01); never more than `MAX_STEP_BPS` above the last cap, fresh or
    /// stale (a stale re-post may fall freely: lower is safer).
    function postCap(uint256 weiPerWrapped) external onlyKeeper {
        if (weiPerWrapped == 0) revert BadInput();
        uint256 ceiling = capCeiling;
        if (ceiling == 0) revert NoCeiling();
        if (capFresh()) {
            // H12: a step, never a jump, and not more often than every ten minutes on both clocks
            if (!Wait.elapsed(uint64(capAt + MIN_POST_GAP), uint64(capAtBlock + Wait.blocksFor(MIN_POST_GAP, parentBlockSeconds)))) {
                revert CapTooSoon();
            }
            uint256 lo = cap - (cap * MAX_STEP_BPS) / 10_000;
            uint256 hi = cap + (cap * MAX_STEP_BPS) / 10_000;
            if (weiPerWrapped < lo || weiPerWrapped > hi) revert CapStep(weiPerWrapped, lo, hi);
        } else if (cap != 0) {
            uint256 hi = cap + (cap * MAX_STEP_BPS) / 10_000;
            if (weiPerWrapped > hi) revert CapStep(weiPerWrapped, 0, hi);
        }
        if (weiPerWrapped > ceiling) revert OverCeiling(weiPerWrapped, ceiling);
        cap = weiPerWrapped;
        capAt = uint64(block.timestamp);
        capAtBlock = uint64(block.number);
        emit CapPosted(weiPerWrapped, capAt);
    }

    /// @notice The registry's owner: the first ceiling, or a lower one, at once (each only tightens); a higher one is proposed and
    /// anyone may accept it after `CEILING_DELAY` on both clocks. It bounds every cap, including the first post after the cap
    /// went stale, and without one no cap is taken.
    function proposeCapCeiling(uint256 ceiling) external {
        if (msg.sender != registry.owner()) revert NotOwner();
        if (ceiling == 0) revert BadInput();
        if (capCeiling == 0 || ceiling <= capCeiling) {
            capCeiling = ceiling;
            pendingCeiling = 0;
            emit CapCeilingSet(ceiling);
            return;
        }
        pendingCeiling = ceiling;
        pendingCeilingAt = uint64(block.timestamp + CEILING_DELAY);
        pendingCeilingAtBlock = uint64(block.number + Wait.blocksFor(CEILING_DELAY, parentBlockSeconds));
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

    /// @notice Whether the last cap may be used now: under an hour old on both clocks, and the keeper not paused.
    function capFresh() public view returns (bool) {
        return cap != 0 && !registry.keeperPaused() && block.timestamp < uint256(capAt) + CAP_TTL && block.number < uint256(capAtBlock) + Wait.blocksFor(CAP_TTL, parentBlockSeconds);
    }

    /// @notice Anyone: spend up to `spend` of `coin`'s ETH at the venue, with the caller's route, for at least `minOut` wFROG and
    /// never above the cap. The vault measures what it paid and what it got; it approves nothing, so no route can pull from it.
    function sweep(address coin, uint256 spend, bytes calldata swapData, uint256 minOut)
        external nonReentrant returns (uint256 spent, uint256 bought)
    {
        _notPaused();
        if (venue == address(0)) revert BadVenue();
        if (!capFresh()) revert CapStale();
        if (spend == 0 || minOut == 0) revert BadInput();
        if (spend > credit[coin]) revert Insufficient();

        uint256 ethBefore = address(this).balance;
        uint256 wBefore = wrapper.balanceOf(address(this));
        _sweeping = true;
        (bool ok,) = venue.call{value: spend}(swapData);
        _sweeping = false;
        if (!ok) revert VenueFailed();
        uint256 ethAfter = address(this).balance;
        if (ethAfter > ethBefore) revert VenueFailed();             // refunded more than it was sent: not a swap
        spent = ethBefore - ethAfter;
        bought = wrapper.balanceOf(address(this)) - wBefore;

        if (bought < minOut) revert Slippage(bought, minOut);
        if (spent * ONE > cap * bought) revert OverCap(spent, bought, cap);
        credit[coin] -= spent;                                      // spent <= spend <= credit[coin]
        totalCredit -= spent;
        units[coin] += bought;
        totalUnits += bought;
        emit Swept(coin, msg.sender, spent, bought);
    }

    /// @notice The keeper: escrow `amount` of `coin`'s ETH at the mint desk for one frog the canister buys on Bitcoin (the canister
    /// route). Never more than the cap for one frog, so `spent ≤ cap × bought` holds here too. Keeper-only (audit 2026-09-26 A-05 /
    /// C-10): an unserved desk would otherwise let anyone park a coin's credit there for the TTL, again and again.
    function sweepViaDesk(address coin, uint256 amount) external onlyKeeper nonReentrant returns (uint256 requestId) {
        _notPaused();
        if (address(desk) == address(0)) revert BadVenue();
        if (!capFresh()) revert CapStale();
        if (amount == 0 || amount > cap) revert BadInput();
        if (amount > credit[coin]) revert Insufficient();
        credit[coin] -= amount;
        totalCredit -= amount;
        atDesk += amount;
        requestId = desk.request{value: amount}();
        deskCoin[requestId] = coin;
        deskAmount[requestId] = amount;
        emit DeskRequested(coin, requestId, amount);
    }

    /// @notice Anyone: bring a filled (or expired) desk request home, charged to the coin it was made for. Measured, not trusted.
    function collectDesk(uint256 requestId) external nonReentrant {
        address coin = deskCoin[requestId];
        if (coin == address(0)) revert WrongStage();
        uint256 amount = deskAmount[requestId];
        delete deskCoin[requestId];
        delete deskAmount[requestId];
        uint256 ethBefore = address(this).balance;
        uint256 wBefore = wrapper.balanceOf(address(this));
        _collecting = true;
        desk.collect(requestId);
        _collecting = false;
        uint256 ethBack = address(this).balance - ethBefore;
        uint256 bought = wrapper.balanceOf(address(this)) - wBefore;
        if (ethBack > amount) revert VenueFailed();
        atDesk -= amount;
        credit[coin] += ethBack;
        totalCredit += ethBack;
        units[coin] += bought;
        totalUnits += bought;
        emit DeskCollected(coin, requestId, ethBack, bought);
    }

    // ---- RAFFLE --------------------------------------------------------------------------------------------------------------

    /// @notice The keeper announces a draw of one whole wFROG among `coin`'s holders (reserved now) and pins a drand round that does
    /// not exist yet: that round will pick the snapshot time, a moment of the `SNAPSHOT_WINDOW` before now (C-04). One draw per
    /// coin before its pin. If the coin's last draw was voided or cancelled before its pin, this one carries that draw's seed
    /// round and snapshot window instead (review2 N-2), so it gets the same snapshot time: nobody can discard a snapshot time
    /// after seeing it and draw another.
    function announceDraw(address coin) external onlyKeeper nonReentrant returns (uint256 id) {
        if (policy != Policy.RAFFLE) revert WrongPolicy();
        _notPaused();
        if (coin == address(0)) revert BadInput();
        if (drawInFlight[coin] != 0) revert DrawInFlight();
        if (units[coin] < ONE) revert Insufficient();
        units[coin] -= ONE;
        totalUnits -= ONE;
        totalReserved += ONE;

        id = _draws.length;
        Draw storage d = _draws.push();
        d.stage = Stage.Announced;
        d.coin = coin;
        uint256 carried = carriedSeed[coin];
        if (carried != 0) {
            _seedOrigin[id] = carried;
            d.seedRound = _draws[carried - 1].seedRound;
            emit SeedCarried(id, carried - 1, d.seedRound);
        } else {
            d.seedRound = beacon.roundAt(block.timestamp) + PIN_AHEAD;
        }
        d.announcedAt = uint64(block.timestamp);
        d.announcedAtBlock = uint64(block.number);
        drawInFlight[coin] = id + 1;
        emit DrawAnnounced(id, coin, d.seedRound);
    }

    /// @notice The draw whose seed `id` uses: itself, or the draw voided or cancelled before its pin whose seed it carries.
    function seedOrigin(uint256 id) public view returns (uint256) {
        uint256 o = _seedOrigin[id];
        return o == 0 ? id : o - 1;
    }

    /// @notice The snapshot time an announced draw's seed randomness picks: `announcedAt − SNAPSHOT_WINDOW + (r mod SNAPSHOT_WINDOW)`,
    /// hashed with and counted back from its seed origin (`seedOrigin(id)`), so a carried seed gives the same time.
    function snapshotTimeOf(uint256 id, bytes32 seedRandomness) public view returns (uint64) {
        uint256 o = seedOrigin(id);
        uint256 x = uint256(keccak256(abi.encode(seedRandomness, address(this), o, "snapshot"))) % SNAPSHOT_WINDOW;
        return uint64(uint256(_draws[o].announcedAt) - SNAPSHOT_WINDOW + x);
    }

    /// @dev review2 N-2: a draw ended before its pin leaves its seed to its coin's next announcement.
    function _carry(uint256 id, address coin) private {
        carriedSeed[coin] = seedOrigin(id) + 1;
    }

    /// @notice The keeper opens an announced draw within `OPEN_TTL`: it proves (or reads) the seed round, which fixes the snapshot
    /// time, and names the ticket list published at `uri` with root `root`. The root must hold the sentinel leaf
    /// `leaf(address(0), totalTickets, snapshotAt)` (C-03): the count and the time are committed with the list. Nothing can use it
    /// for 15 minutes. A snapshot with no eligible holder (fixes-R3-carry) is opened with `totalTickets` 0, and its tree is the
    /// sentinel alone: `root == leaf(address(0), 0, snapshotAt)`, no proof. After the window `requestDraw` closes it with no
    /// winner; until then the owner or the guardian may cancel it (a false empty), which carries the seed like any cancel. A list
    /// with tickets has at least one holder leaf beside its sentinel, so its `sentinelProof` is never empty (R3 check A3-I2). That
    /// is the only tree shape checkable here: the sentinel beside junk nodes (R4 check A4-I1), like any other false list, passes any
    /// structural rule the contract could state without the holder list; the monitor's recompute is the guard (a `bad` root: the
    /// guardian pauses the keeper and cancels the draw before the pin).
    function openDraw(uint256 id, bytes32 root, uint256 totalTickets, string calldata uri, bytes32[] calldata sentinelProof,
        bytes calldata seedSignature) external onlyKeeper nonReentrant
    {
        _notPaused();
        Draw storage d = _draws[id];
        if (d.stage != Stage.Announced) revert WrongStage();
        if (Wait.elapsed(uint64(d.announcedAt + OPEN_TTL), uint64(d.announcedAtBlock + Wait.blocksFor(OPEN_TTL, parentBlockSeconds)))) {
            revert WindowOver();
        }
        if (root == bytes32(0)) revert BadInput();
        bytes32 seed = beacon.randomnessOf(d.seedRound);
        if (seed == bytes32(0)) seed = beacon.prove(d.seedRound, seedSignature);
        uint64 at = snapshotTimeOf(id, seed);
        if (totalTickets == 0) {
            // the empty snapshot's whole tree is its sentinel: no holder leaf can sit beside it
            if (sentinelProof.length != 0 || root != leaf(address(0), 0, at)) revert BadProof();
        } else if (sentinelProof.length == 0 || !MerkleProof.verifyCalldata(sentinelProof, root, leaf(address(0), totalTickets, at))) {
            // R3 check A3-I2: a list with tickets has a holder leaf beside its sentinel, so the sentinel's proof is never empty
            revert BadProof();
        }

        d.stage = Stage.Open;
        d.root = root;
        d.totalTickets = totalTickets;
        d.uri = uri;
        d.snapshotAt = at;
        d.challengeEnds = uint64(block.timestamp + CHALLENGE);
        d.challengeEndsBlock = uint64(block.number + Wait.blocksFor(CHALLENGE, parentBlockSeconds));
        emit SnapshotFixed(id, at);
        emit DrawOpened(id, d.coin, root, totalTickets, uri, d.challengeEnds);
    }

    /// @notice The owner or the guardian, any time before the pin, or before the close of an empty draw (the winner is unknown then,
    /// and the coin's next draw carries this one's seed and so its snapshot time, review2 N-2: a cancel steers nothing; a false empty
    /// is answered here, fixes-R3-carry): the whole wFROG goes back to the coin.
    function cancelDraw(uint256 id) external nonReentrant {
        address g = registry.guardian();
        if (msg.sender != registry.owner() && (msg.sender != g || g == address(0))) revert NotOwner();
        Draw storage d = _draws[id];
        if (d.stage != Stage.Open && d.stage != Stage.Announced) revert WrongStage();
        d.stage = Stage.Cancelled;
        _carry(id, d.coin);
        delete drawInFlight[d.coin];
        _unreserve(d.coin);
        emit DrawCancelled(id);
    }

    /// @notice Anyone, once the window has passed on both clocks and the keeper is not paused (nor zero): pins a drand round that
    /// does not exist yet. The pin is final: there is no redraw. A pause holds the pin for as long as it lasts (the monitor pauses
    /// the keeper ahead of the pin of a draw whose root it could not verify). A draw opened with no tickets is closed instead, under
    /// the same gates (fixes-R3-carry): `Empty`, no winner, the unit back to the coin, and the seed used like a pinned one.
    function requestDraw(uint256 id) external {
        Draw storage d = _draws[id];
        if (d.stage != Stage.Open) revert WrongStage();
        if (!Wait.elapsed(d.challengeEnds, d.challengeEndsBlock)) revert ChallengeWindow();
        // while the keeper is paused (or there is none) nobody pins: the monitor's safety net pauses the keeper before the pin of a
        // draw whose root it could not verify, and relies on this refusal
        if (registry.keeperPaused() || registry.keeper() == address(0)) revert KeeperPaused();
        delete drawInFlight[d.coin];
        delete carriedSeed[d.coin];                              // the seed is used: the coin's next draw gets a new one
        if (d.totalTickets == 0) {
            d.stage = Stage.Empty;
            _unreserve(d.coin);
            emit DrawEmpty(id, d.coin, d.snapshotAt);
            return;
        }
        _pin(d);
        emit DrawPinned(id, d.round);
    }

    function _pin(Draw storage d) private {
        d.stage = Stage.Pinned;
        d.round = beacon.roundAt(block.timestamp) + PIN_AHEAD;
        d.pinnedAt = uint64(block.timestamp);
        d.pinnedAtBlock = uint64(block.number);
    }

    /// @notice Anyone: reads (or proves) the pinned round and fixes the winning ticket.
    function resolve(uint256 id, bytes calldata signature) external {
        Draw storage d = _draws[id];
        if (d.stage != Stage.Pinned) revert WrongStage();
        bytes32 randomness = beacon.randomnessOf(d.round);
        if (randomness == bytes32(0)) randomness = beacon.prove(d.round, signature);
        d.winningTicket = uint256(keccak256(abi.encode(randomness, address(this), id))) % d.totalTickets;
        d.stage = Stage.Resolved;
        d.resolvedAt = uint64(block.timestamp);
        d.resolvedAtBlock = uint64(block.number);
        emit DrawResolved(id, d.winningTicket);
    }

    /// @notice Anyone, for the holder whose published range holds the winning ticket: one whole wFROG to that holder.
    function claim(uint256 id, address holder, uint256 firstTicket, uint256 endTicket, bytes32[] calldata proof) external nonReentrant {
        Draw storage d = _draws[id];
        if (d.stage != Stage.Resolved) revert WrongStage();
        if (holder == address(0) || firstTicket > d.winningTicket || d.winningTicket >= endTicket || endTicket > d.totalTickets) {
            revert BadProof();
        }
        if (!MerkleProof.verifyCalldata(proof, d.root, leaf(holder, firstTicket, endTicket))) revert BadProof();
        d.stage = Stage.Claimed;
        d.winner = holder;
        totalReserved -= ONE;
        wrapper.transfer(holder, ONE);
        emit Won(id, holder);
    }

    /// @notice Anyone: the whole wFROG goes back to the coin for a later draw, from a draw resolved a week ago with no claim, a
    /// draw pinned `PIN_TIMEOUT` ago that nobody resolved (the round is never replaced), or a draw announced `OPEN_TTL` ago that
    /// the keeper never opened. Both clocks.
    function voidDraw(uint256 id) external nonReentrant {
        Draw storage d = _draws[id];
        uint64 at;
        uint64 atBlock;
        if (d.stage == Stage.Resolved) {
            (at, atBlock) = (uint64(d.resolvedAt + VOID_AFTER), uint64(d.resolvedAtBlock + Wait.blocksFor(VOID_AFTER, parentBlockSeconds)));
        } else if (d.stage == Stage.Pinned) {
            (at, atBlock) = (uint64(d.pinnedAt + PIN_TIMEOUT), uint64(d.pinnedAtBlock + Wait.blocksFor(PIN_TIMEOUT, parentBlockSeconds)));
        } else if (d.stage == Stage.Announced) {
            (at, atBlock) = (uint64(d.announcedAt + OPEN_TTL), uint64(d.announcedAtBlock + Wait.blocksFor(OPEN_TTL, parentBlockSeconds)));
            delete drawInFlight[d.coin];
            _carry(id, d.coin);                                  // not opening is a delay, never a new snapshot time (N-2)
        } else {
            revert WrongStage();
        }
        if (!Wait.elapsed(at, atBlock)) revert TooEarly();
        d.stage = Stage.Voided;
        _unreserve(d.coin);
        emit DrawVoided(id);
    }

    function _unreserve(address coin) private {
        totalReserved -= ONE;
        units[coin] += ONE;
        totalUnits += ONE;
    }

    /// @notice The Distributor's leaf: a holder's range of tickets `[firstTicket, endTicket)`.
    function leaf(address holder, uint256 firstTicket, uint256 endTicket) public pure returns (bytes32) {
        return keccak256(bytes.concat(keccak256(abi.encode(holder, firstTicket, endTicket))));
    }

    // ---- BURN ----------------------------------------------------------------------------------------------------------------

    /// @notice Anyone: redeem one whole wFROG of `coin`'s to the burn address. The drand round picks the frog; custody sends it
    /// where nobody can ever spend it. The wrapper's ledger is untouched by this being a burn: it is a redemption like any other.
    function retire(address coin) external nonReentrant returns (uint256 ticketId) {
        if (policy != Policy.BURN) revert WrongPolicy();
        if (units[coin] < ONE) revert Insufficient();
        units[coin] -= ONE;
        totalUnits -= ONE;
        ticketId = wrapper.redeemToBurn();
        retiredBy[ticketId] = coin;
        retiredCount++;
        emit Retired(coin, ticketId);
    }

    /// @notice Anyone, after the wrapper refunded a retirement nobody delivered: credit the returned wFROG back to its coin.
    function reclaim(uint256 ticketId) external nonReentrant {
        address coin = retiredBy[ticketId];
        if (coin == address(0) || reclaimed[ticketId]) revert WrongStage();
        if (!wrapper.refunded(ticketId)) revert WrongStage();
        reclaimed[ticketId] = true;
        Listing storage l = _listings[ticketId];
        if (l.stage == ListingStage.Listed) { l.stage = ListingStage.Returned; listedOpen[coin]--; }
        retiredCount--;
        units[coin] += ONE;
        totalUnits += ONE;
        emit Reclaimed(coin, ticketId);
    }

    // ---- YOYO (docs/v5/policies-and-fees.md §1) -------------------------------------------------------------------------------

    /// @notice Anyone: relist one whole unit of `coin`'s at the markup over the keeper's fresh cap. The drand round picks the
    /// item; it stays in custody, listed; the custodian's delivery is the listing.
    function relist(address coin) external nonReentrant returns (uint256 ticketId) {
        if (policy != Policy.YOYO) revert WrongPolicy();
        _notPaused();
        if (!capFresh()) revert CapStale();
        if (units[coin] < ONE) revert Insufficient();
        units[coin] -= ONE;
        totalUnits -= ONE;
        ticketId = wrapper.redeemToRelist();
        uint256 ask = (cap * (10_000 + markupBps)) / 10_000;
        _listings[ticketId] = Listing(ListingStage.Listed, coin, ask, 0);
        retiredBy[ticketId] = coin;                                 // a refunded ticket comes back through `reclaim`
        retiredCount++;
        listedOpen[coin]++;
        emit Relisted(coin, ticketId, ask);
    }

    /// @notice The custodian, having sold a listed item: its proceeds, in ETH, at least the ask less the sale allowance. They
    /// wait here for the keeper's buyback; nothing else can move them.
    function recordSale(uint256 ticketId) external payable nonReentrant {
        if (msg.sender != wrapper.custodian()) revert NotCustodian();
        Listing storage l = _listings[ticketId];
        if (l.stage != ListingStage.Listed) revert WrongStage();
        uint256 min = (l.ask * (10_000 - SALE_FEE_ALLOWANCE_BPS)) / 10_000;
        if (msg.value < min) revert UnderAsk(msg.value, min);
        l.stage = ListingStage.Sold;
        l.proceeds = msg.value;
        listedOpen[l.coin]--;
        retiredCount--;
        proceeds[l.coin] += msg.value;
        totalProceeds += msg.value;
        emit SaleRecorded(l.coin, ticketId, msg.value);
    }

    /// @notice The keeper: spend up to `spend` of `coin`'s sale proceeds at the venue, with the keeper's route, for at least
    /// `minOut` of the coin — every coin received goes to `DEAD`. Measured, not trusted: the vault approves nothing, and takes
    /// back any refund. The one way proceeds ever leave.
    function buyback(address coin, uint256 spend, bytes calldata swapData, uint256 minOut)
        external onlyKeeper nonReentrant returns (uint256 spent, uint256 burned)
    {
        if (policy != Policy.YOYO) revert WrongPolicy();
        _notPaused();
        if (venue == address(0)) revert BadVenue();
        if (spend == 0 || minOut == 0 || coin == address(0)) revert BadInput();
        if (spend > proceeds[coin]) revert Insufficient();
        IERC20 token = IERC20(coin);
        uint256 ethBefore = address(this).balance;
        uint256 cBefore = token.balanceOf(address(this));
        _sweeping = true;
        (bool ok,) = venue.call{value: spend}(swapData);
        _sweeping = false;
        if (!ok) revert VenueFailed();
        uint256 ethAfter = address(this).balance;
        if (ethAfter > ethBefore) revert VenueFailed();
        spent = ethBefore - ethAfter;
        burned = token.balanceOf(address(this)) - cBefore;
        if (burned < minOut) revert Slippage(burned, minOut);
        proceeds[coin] -= spent;
        totalProceeds -= spent;
        totalBurnedBack += spent;
        emit BoughtBack(coin, msg.sender, spent, burned);
        token.safeTransfer(DEAD, token.balanceOf(address(this)));   // everything the vault holds of the coin: never kept
    }

    /// @notice The custodian, having delisted an unsold item and deposited it back to this vault: the unit is the coin's again.
    function reclaimListing(uint256 ticketId) external nonReentrant {
        if (msg.sender != wrapper.custodian()) revert NotCustodian();
        Listing storage l = _listings[ticketId];
        if (l.stage != ListingStage.Listed) revert WrongStage();
        if (wrapper.balanceOf(address(this)) < totalUnits + totalReserved + ONE) revert Insufficient();
        l.stage = ListingStage.Returned;
        listedOpen[l.coin]--;
        retiredCount--;
        units[l.coin] += ONE;
        totalUnits += ONE;
        emit ListingReturned(l.coin, ticketId);
    }

    /// @notice The registry's owner (the timelock): the markup, within `[MIN_MARKUP_BPS, MAX_MARKUP_BPS]`, for listings from now.
    function setMarkup(uint256 bps) external {
        if (msg.sender != registry.owner()) revert NotOwner();
        if (bps < MIN_MARKUP_BPS || bps > MAX_MARKUP_BPS) revert BadMarkup();
        markupBps = bps;
        emit MarkupSet(bps);
    }

    function listing(uint256 ticketId) external view returns (Listing memory) { return _listings[ticketId]; }

    // ---- views ---------------------------------------------------------------------------------------------------------------

    function draw(uint256 id) external view returns (Draw memory) { return _draws[id]; }
    function drawCount() external view returns (uint256) { return _draws.length; }
    /// @notice wFROG here that no coin is charged with: sent directly, or refunded and not yet reclaimed. It stays.
    function stray() external view returns (uint256) { return wrapper.balanceOf(address(this)) - totalUnits - totalReserved; }

    /// @notice A sweep vault holds wFROG, not receipted inscriptions: nothing downstream may treat it as a purchase vault.
    function acquisition(uint256) external pure returns (Acquisition memory) { revert NoAcquisitions(); }
    function acquisitionCount() external pure returns (uint256) { return 0; }
    function isRune() external pure returns (bool) { return false; }
}
