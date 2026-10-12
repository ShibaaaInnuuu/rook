// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {MerkleProof} from "@openzeppelin/contracts/utils/cryptography/MerkleProof.sol";
import {IWrappedCollection} from "../interfaces/IWrappedCollection.sol";
import {IFloorWrapped} from "../interfaces/IFloorWrapped.sol";
import {IItemQueue} from "../interfaces/IItemQueue.sol";
import {ICapSource} from "../interfaces/ICapSource.sol";
import {ILiquiditySeeder} from "../interfaces/ILiquiditySeeder.sol";
import {IKeeperSource} from "../interfaces/IKeeperSource.sol";
import {Wait} from "../lib/Wait.sol";

/// @notice What the module asks of the protocol treasury (docs/v5/floor-module.md §13.3): its bootstrap loan, repaid first.
interface ITreasuryLoans {
    function outstanding(address module) external view returns (uint256);
    function repay() external payable;
}

/// @title FloorModule
/// @notice One collection's floor venue (docs/v5/floor-module.md). Anyone mints wX with ETH at the keeper's floor plus a spread,
/// or burns wX for ETH at the floor less a spread, in one transaction. Minted wX is ETH-backed: its ETH stays here (`backing`)
/// until the canister, asked by the netting signal (`netUnits` beyond one item plus `buffer`), wraps an item into this module and
/// `itemArrived` burns one whole ETH-backed unit against it, paying the canister's cost from that unit's backing. A graduating curve converts its item budget
/// here at once (`enqueue`, the `IItemQueue` a factory is wired to). Holders may lend floor items (`rentIn`) and get the same
/// item back at the end of the term, or the floor plus a premium if a burner's ticket takes it.
///
/// The books (§4, §13.1): `wrapper.ethBacked()` units are backed by two pots — the convertible `backing` (what the canister
/// converts, `netUnits()` of them) and the standing `reserve`, which keeps `reserveBps` of every mint's ETH and is never
/// converted. A burn pays at most the backing per unit of the pot it draws from, from the reserve first, so no pot's backing per
/// unit ever falls on an outflow and the escrow can never pay more than it holds; the rest is revenue. The module prices at the
/// time-weighted average of the protocol's own fills inside the keeper's band (§13.4), or at the keeper's floor alone before any
/// fill. Every price defence (§6) is a constant here: the move limit on the keeper's floor (per hour and per day), the spread
/// function, the hourly caps, the caps on ETH-backed units. There is no owner and no setter. The keeper is the registry's
/// (`registry.keeper()`, rotated by the owner in seven public days) and the registry's incident switch stops it, and every inflow
/// priced by its word, at once. Burns go on through the pause at the floor the keeper left (`pauseFloor()`, never above the
/// backing per unit) less the base spread, and a week without a good post opens the stale fallback at the backing per unit; only an
/// ordinary keeper outage shorter than that refuses them (`Stale`).
/// The treasury (§13.3) funds the reserve surplus and floats and is repaid its bootstrap loan first.
/// docs/audit/2026-09-26/fixes-B-C-keeper-draws.md; docs/audit/2026-09-26/review2/fixes-R3-contracts.md;
/// docs/audit/2026-09-26/review3/fixes-R4-contracts.md.
/// The advance (docs/v6/relay-conversion.md §2): the custodian may take one unit's item budget BEFORE its item arrives
/// (`advance`), to convert this module's own ETH into what the item's chain is paid in; that unit is then in transit
/// (`transitUnits`), outside every burn and conversion, until the item is wrapped here (`settleAdvance`) or the ETH comes back
/// (`returnAdvance`). One at a time, within `maxItemCost()` and the daily item count.
/// @dev What this does NOT protect against: the keeper's floor within the move limits until it is paused (a compromised keeper
/// costs at most `maxPublicEthBacked × floor × MAX_DAY_MOVE_BPS` a day through cheap mints; burns pay at most what the units paid
/// in, so nothing is drained through them); basis risk (ETH-backed wX does not rise with the floor); the custodian (it attests
/// every item, must deliver returns, and reports the fills that move the price within `fillBandBps`); a revenue pot and surplus
/// too small for a conversion or a rental premium (the conversion waits; the premium becomes debt).
contract FloorModule is IItemQueue, ReentrancyGuard {
    using SafeERC20 for IERC20;

    struct Params {
        uint16 baseSpreadBps;       // each side, calm market (15)
        uint16 volWeightBps;        // s_vol = weight × trailing-hour movement (10_000 = 1×)
        uint16 staleSpreadBps;      // added at a post FLOOR_TTL (3 h) old, ramping from STALE_GRACE (100)
        uint16 stressSpreadBps;     // added to minting at E = maxPublicEthBacked (300)
        uint16 maxSpreadBps;        // no side wider (1_000)
        uint16 maxMoveBps;          // the accepted floor moves at most this per hour (500)
        uint16 convBps;             // item cap and graduation price: floor × (1 + convBps) (300)
        uint16 bufferBps;           // the canister buys once E ≥ 1 + bufferBps of an item (500)
        uint256 mintPerHour;        // units (1e18 = one item), leaky bucket
        uint256 burnPerHour;
        uint256 maxPublicEthBacked; // a public mint may not push E past this
        uint256 maxEthBacked;       // nothing may (graduations included)
        uint256 revenueTarget;      // revenue beyond this may be skimmed to the platform (the treasury)
        uint64 rentTerm;            // 30 days
        uint16 premiumBps;          // a taken rental pays floor × (1 + premiumBps) (1_000)
        uint16 rentShareBps;        // of realized revenue, to active renters (5_000)
        uint16 maxRentals;          // lent or leaving at once
        uint16 lowConfidenceSpreadBps; // added at confidence 0, falling linearly to nothing at fullConfidenceBps (100)
        uint16 fullConfidenceBps;   // at or above this confidence, no addition (8_000)
        uint16 minConfidenceBps;    // a post below it leaves the module stale (3_000, the keeper's own minimum)
        uint16 maxItemsPerDay;      // item arrivals (conversions and rentals) the module pays for in any 24 h (H11)
        uint16 reserveBps;          // of every mint's ETH and units, kept as the standing reserve, never converted (2_500)
        uint16 fillBandBps;         // the fills price stays within this of the accepted floor (300)
    }

    struct Fill {
        uint192 price;              // wei per whole item: a purchase's cost or a sale's proceeds
        uint64 at;
    }

    enum GradStatus { None, Queued, Filled, Released, Cancelled }

    struct Grad {
        GradStatus status;
        address curve;
        uint256 eth;          // the item budget
        uint256 units;        // minted for it at the graduation price
    }

    enum RentStatus { None, Lent, Leaving, Closed }

    struct Rental {
        RentStatus status;
        address renter;
        uint64 endsAt;
        uint64 endsAtBlock;
        uint256 itemId;
        uint256 escrow;       // the backing of the unit the item took over, held for the rental
        uint256 accAt;        // the revenue accumulator when it started earning
        uint256 paid;         // compensation credited, if its item was taken
    }

    uint256 public constant BPS = 10_000;
    uint256 public constant ONE = 1e18;
    uint256 public constant FLOOR_TTL = 3 hours;          // the operator, 2026-10-09: NFT floors move slowly; a post up to 3 h old prices
    uint256 public constant STALE_GRACE = 10 minutes;
    uint256 public constant SNAPSHOT_TTL = 2 hours;
    uint256 private constant HOUR = 1 hours;
    uint256 private constant ACC = 1e18;
    /// @notice The fills price is the time-weighted average of the protocol's fills over this window (§13.4).
    uint256 public constant FILL_WINDOW = 24 hours;
    uint256 private constant FILLS = 8;
    uint256 private constant DAYS = 7;
    /// @notice The accepted floor stays within this of the day's anchor (the accepted floor when the current 24 h window opened).
    uint256 public constant MAX_DAY_MOVE_BPS = 2_000;
    uint256 public constant DAY_WINDOW = 1 days;
    /// @notice With no good post (confidence at least `minConfidenceBps`) for `FLOOR_TTL + STALE_ESCAPE` on both clocks, burns pay
    /// the backing per unit with no floor (`escaped()`).
    uint256 public constant STALE_ESCAPE = 7 days;
    /// @notice After the keeper is resumed, unfilled curves' refunds stay held this long on both clocks (R3 check A3-L1): the floor
    /// is stale at the unpause, so the keeper's first post and a `fill` must come before anyone can bind a held graduation to refund.
    uint256 public constant RESUME_GRACE = 6 hours;
    /// @notice R4-check A4-L1: the pause floor is capped by the least pre-spread price a public mint paid in the bucket of the last
    /// moment a mint was possible (the last post's `postedAt + FLOOR_TTL`) and the bucket before it: 12 to 24 hours of mints.
    uint256 public constant MINT_LOW_BUCKET = 12 hours;

    IWrappedCollection public immutable wrapper;
    IFloorWrapped private immutable _w;
    /// @notice Where the keeper is read, on every call (the ListingRegistry): no key is bound here.
    IKeeperSource public immutable registry;
    address public immutable payee;
    address public immutable platform;
    /// @notice The protocol treasury (§13.3): funds floats, and is repaid its bootstrap loan first. Zero: no treasury.
    address public immutable treasury;
    address public immutable deployer;
    uint256 public immutable parentBlockSeconds;

    uint256 public immutable baseSpreadBps;
    uint256 public immutable volWeightBps;
    uint256 public immutable staleSpreadBps;
    uint256 public immutable stressSpreadBps;
    uint256 public immutable maxSpreadBps;
    uint16 public immutable maxMoveBps;
    uint256 public immutable convBps;
    uint256 public immutable bufferBps;
    uint256 public immutable mintPerHour;
    uint256 public immutable burnPerHour;
    uint256 public immutable maxPublicEthBacked;
    uint256 public immutable maxEthBacked;
    uint256 public immutable revenueTarget;
    uint256 public immutable rentTerm;
    uint256 public immutable premiumBps;
    uint256 public immutable rentShareBps;
    uint256 public immutable maxRentals;
    uint256 public immutable lowConfidenceSpreadBps;
    uint256 public immutable fullConfidenceBps;
    uint256 public immutable minConfidenceBps;
    uint256 public immutable maxItemsPerDay;
    uint256 public immutable reserveBps;
    uint256 public immutable fillBandBps;

    ICapSource public factory;
    /// @notice The collection's liquidity lock, from the factory at wiring: where `fundLiquidity` goes.
    ILiquiditySeeder public seeder;

    // ---- the floor ----
    uint256 public accepted;              // wei per whole item, after the move limit
    uint256 public target;                // the keeper's last posted value
    uint64 public postedAt;
    uint64 public postedAtBlock;
    uint256 public moveAcc;               // bps moved, as of the last post (decays linearly over an hour)
    uint16 public confidenceBps;          // the keeper's confidence in its last post (docs/v5/floor-feed.md §1.5)
    uint32 public depthItems;             // listings within +10% of the floor at the last post: bounds public ETH-backed units
    uint256 public dayAnchor;             // the accepted floor when the current day window opened
    uint64 public dayAnchorAt;
    uint64 public dayAnchorAtBlock;
    uint64 public lastGoodAt;             // the last post with confidence ≥ minConfidenceBps
    uint64 public lastGoodAtBlock;

    // ---- the hourly caps (leaky buckets, units) ----
    uint256 public mintLevel;
    uint64 public mintAt;
    uint64 public mintAtBlock;
    uint256 public burnLevel;
    uint64 public burnAt;
    uint64 public burnAtBlock;

    // ---- the books: balance ≥ backing + reserve + surplus + revenue + gradEth + rentalEscrow + renterPool + owedTotal ----
    /// @notice The convertible pot: the ETH that backs the `netUnits()` ETH-backed units the canister converts.
    uint256 public backing;
    /// @notice The standing reserve's basis (§13.1): `reserveBps` of every mint's and graduation's ETH, what the reserve units
    /// paid in. It backs `reserveUnits`, pays burns first — never more per unit than it holds per unit — and is never converted.
    uint256 public reserve;
    /// @notice The reserve's surplus: the treasury's reserve allocation, the coins' reserve slices (`fundReserve`), the proceeds of
    /// the protocol's item sales and the spread reserve-tier burns keep. No unit claims it, so no burn pays from it: it tops up
    /// conversions, and after the stale window it goes to the platform (the treasury). B-02.
    uint256 public surplus;
    /// @notice The ETH-backed units whose backing is the reserve; the other `ethBacked − reserveUnits` are convertible.
    uint256 public reserveUnits;
    /// @notice Realized revenue kept to pay conversions above a unit's backing and rental premiums.
    uint256 public revenue;
    /// @notice Units the treasury's floats minted to this module (§13.3): the protocol's inventory, sold by `itemSold`.
    uint256 public floatUnits;
    /// @notice Item budgets of graduations not yet converted.
    uint256 public gradEth;
    /// @notice Units minted for graduations and not yet released to their curves.
    uint256 public gradUnits;
    uint256 public rentalEscrow;
    /// @notice Renters' share of revenue, accrued and not yet credited to a renter.
    uint256 public renterPool;
    mapping(address => uint256) public owed;
    uint256 public owedTotal;
    uint256 public debtTotal;
    mapping(uint256 rentalId => uint256) public debt;

    // ---- conversions (the canister) ----
    uint256 public itemsArrived;
    /// @dev H11: when each item the module paid for arrived (conversions and rentals), packed `timestamp << 64 | block`, by count.
    mapping(uint256 => uint256) private _arrivedAt;
    uint256 public arrivals;
    uint256 public snapCap;
    uint64 public snapAt;
    uint64 public snapAtBlock;

    // ---- the advance (docs/v6/relay-conversion.md §2): one unit whose backing left before its item arrived ----
    /// @notice 0, or ONE while an advance is open: that unit's backing is on its way to becoming an item (ETH at the payee, at
    /// the bridge, sats in the canister's float, the item in custody). No burn reaches it and no conversion counts it.
    uint256 public transitUnits;
    /// @notice The wei the open advance paid to `payee`.
    uint256 public advanced;
    /// @notice The backing the transit unit released when it was advanced (what `itemArrived` would have released for it).
    uint256 public advanceBasis;
    /// @notice When the open advance was paid (0 when none is open).
    uint64 public advancedAt;

    // ---- the fills (§13.4) and the volume (§13.3) ----
    Fill[FILLS] private _fills;           // a ring; `fillCount` fills ever
    uint256 public fillCount;
    uint256[DAYS] private _dayVolume;     // mint + burn ETH per day, by day number mod 7
    uint256[DAYS] private _dayOf;         // the day number each bucket holds
    /// @dev R4-check A4-L1: the least pre-spread price per unit (`max(price(), _upTarget())`) a public mint paid, per
    /// `MINT_LOW_BUCKET` of `block.timestamp`: two buckets (this one and the one before), each `price << 64 | bucket number`.
    uint256[2] private _mintLow;

    // ---- rentals ----
    bytes32 public rentalRoot;
    uint256 public activeRentals;         // Lent
    uint256 public leavingRentals;        // Leaving
    uint256 public accPerRental;          // revenue per lent rental, ever, × ACC
    Rental[] private _rentals;
    /// @dev The wrapper's item id → its latest rental's id + 1 (0: never rented), for `rentalReturned`.
    mapping(uint256 itemId => uint256) private _rentalOfItem;

    mapping(address coin => Grad) private _grads;

    event Wired(address factory);
    event FloorPosted(uint256 target, uint256 accepted, uint256 moveBps, uint16 confidenceBps, uint32 depthItems);
    event RentalRootPosted(bytes32 root);
    event Minted(address indexed payer, address indexed to, uint256 eth, uint256 units, uint256 price);
    event Burned(address indexed from, address indexed to, uint256 units, uint256 eth, uint256 released);
    event ItemArrived(uint256 indexed item, uint256 cost, uint256 released);
    event ItemSold(uint256 proceeds);
    event FillRecorded(uint256 price, bool sale);
    event ReserveFunded(address indexed from, uint256 amount);
    event LiquidityFunded(address indexed from, uint256 amount);
    event FloatFunded(uint256 eth, uint256 units, uint256 price);
    event LoanRepaid(uint256 amount);
    event CapSnapshot(uint256 cap);
    event GradQueued(address indexed coin, uint256 eth);
    event GradFilled(address indexed coin, uint256 units, uint256 price);
    event GradReleased(address indexed coin, uint256 units);
    event GradCancelled(address indexed coin, uint256 eth);
    event RentedIn(uint256 indexed rentalId, uint256 indexed itemId, address indexed renter, uint256 escrow);
    event RentalEnded(uint256 indexed rentalId);
    event RentalSettled(uint256 indexed rentalId, bool taken, uint256 paid, uint256 debt);
    event Revenue(uint256 amount, uint256 toRenters);
    event Claimed(address indexed who, uint256 amount);
    event Skimmed(uint256 amount);
    event StrayReclaimed(uint256 amount);
    event Advanced(uint256 amount, uint256 released);
    event AdvanceSettled(uint256 indexed item, uint256 advanced, uint256 cost);
    event AdvanceReturned(uint256 returned, uint256 basis, uint256 loss);

    error BadParams();
    error NotKeeper();
    error NotCustodian();
    error NotDeployer();
    error AlreadyWired();
    error NotWired();
    error NotCurve();
    error WrongStatus();
    error BadInput();
    error Stale();
    error Slippage();
    error OverCap();
    error OverBacked();
    error NotEnoughBacked();
    error OverBasis(uint256 need, uint256 revenue);
    error NotTreasury();
    error UnderSalePrice(uint256 proceeds, uint256 min);
    error NoInventory();
    error CapStale();
    error OverItemCap(uint256 cost, uint256 max);
    error NoWrapped();
    error ArrivalRateLimited();
    error NotEligible();
    error NoRoom();
    error TooEarly();
    error Drawn();
    error NothingToSettle();
    error TransferFailed();
    error Paused();
    error AdvanceOpen();
    error NoAdvance();
    error NotWrapper();

    /// @param treasury_ the protocol treasury (§13.3), or zero for none: then nothing funds floats and no loan is ever owed.
    /// @param registry_ where the keeper is read (the ListingRegistry): its `keeper()` and its incident switch.
    constructor(IFloorWrapped wrapper_, IKeeperSource registry_, address payee_, address platform_, address treasury_, Params memory p,
        uint256 parentBlockSeconds_)
    {
        if (address(wrapper_) == address(0) || address(registry_) == address(0) || payee_ == address(0) || platform_ == address(0)) revert BadParams();
        if (p.reserveBps > 5_000 || p.fillBandBps > 1_000) revert BadParams();
        Wait.checkBlockSeconds(parentBlockSeconds_);
        if (p.maxSpreadBps > 5_000 || p.baseSpreadBps > p.maxSpreadBps || p.maxMoveBps == 0 || p.maxMoveBps > 2_000) revert BadParams();
        if (p.convBps > 2_000 || p.bufferBps > 2_000 || p.staleSpreadBps > p.maxSpreadBps || p.stressSpreadBps > p.maxSpreadBps) revert BadParams();
        if (p.volWeightBps > 5 * BPS) revert BadParams();
        if (p.mintPerHour == 0 || p.burnPerHour == 0 || p.maxPublicEthBacked == 0 || p.maxPublicEthBacked > p.maxEthBacked) revert BadParams();
        if (p.rentTerm < 1 days || p.premiumBps > 5_000 || p.rentShareBps > BPS) revert BadParams();
        if (p.lowConfidenceSpreadBps > p.maxSpreadBps || p.fullConfidenceBps == 0 || p.fullConfidenceBps > BPS) revert BadParams();
        if (p.minConfidenceBps > p.fullConfidenceBps || p.maxItemsPerDay == 0) revert BadParams();
        wrapper = wrapper_;
        _w = wrapper_;
        registry = registry_;
        payee = payee_;
        platform = platform_;
        treasury = treasury_;
        deployer = msg.sender;
        parentBlockSeconds = parentBlockSeconds_;
        baseSpreadBps = p.baseSpreadBps;
        volWeightBps = p.volWeightBps;
        staleSpreadBps = p.staleSpreadBps;
        stressSpreadBps = p.stressSpreadBps;
        maxSpreadBps = p.maxSpreadBps;
        maxMoveBps = p.maxMoveBps;
        convBps = p.convBps;
        bufferBps = p.bufferBps;
        mintPerHour = p.mintPerHour;
        burnPerHour = p.burnPerHour;
        maxPublicEthBacked = p.maxPublicEthBacked;
        maxEthBacked = p.maxEthBacked;
        revenueTarget = p.revenueTarget;
        rentTerm = p.rentTerm;
        premiumBps = p.premiumBps;
        rentShareBps = p.rentShareBps;
        maxRentals = p.maxRentals;
        lowConfidenceSpreadBps = p.lowConfidenceSpreadBps;
        fullConfidenceBps = p.fullConfidenceBps;
        minConfidenceBps = p.minConfidenceBps;
        maxItemsPerDay = p.maxItemsPerDay;
        reserveBps = p.reserveBps;
        fillBandBps = p.fillBandBps;
    }

    /// @notice Once, by the deployer: the collection's factory (who each coin's curve is), for the graduation lane.
    function wire(ICapSource factory_) external {
        if (msg.sender != deployer) revert NotDeployer();
        if (address(factory) != address(0)) revert AlreadyWired();
        if (factory_.wrapped() != address(wrapper)) revert BadInput();
        factory = factory_;
        seeder = factory_.seeder();
        emit Wired(address(factory_));
    }

    // ---- the keeper -----------------------------------------------------------------------------------------------------------

    /// @notice The keeper's robust floor (docs/v5/floor-module.md §5, docs/v5/floor-feed.md), wei per whole item, with its
    /// confidence and the depth of the book at it. The accepted floor moves toward `priceWei` by at most `maxMoveBps` per hour of
    /// the slower clock since the last post, one hour's worth at most (`floorBounds()`); a price outside is clamped, not refused,
    /// and kept as `target` so the lag is priced against the trader. It also stays within `MAX_DAY_MOVE_BPS` of the day's anchor.
    /// A confidence under `minConfidenceBps` leaves the module stale.
    function postFloor(uint256 priceWei, uint16 confidenceBps_, uint32 depthItems_) external {
        if (msg.sender != registry.keeper()) revert NotKeeper();      // zero while the keeper is paused
        if (priceWei == 0 || confidenceBps_ > BPS) revert BadInput();
        uint256 prev = accepted;
        uint256 next = priceWei;
        uint256 moved;
        if (prev != 0) {
            (uint256 lo, uint256 hi) = floorBounds();
            if (next > hi) next = hi;
            if (next < lo) next = lo;
            moved = Math.mulDiv(next > prev ? next - prev : prev - next, BPS, prev, Math.Rounding.Ceil);
            moveAcc = volBps() + moved;
            (, bool reset) = _dayAnchorNow();
            if (reset) _setDayAnchor(prev);
        } else {
            _setDayAnchor(next);
        }
        if (confidenceBps_ >= minConfidenceBps) {
            lastGoodAt = uint64(block.timestamp);
            lastGoodAtBlock = uint64(block.number);
        }
        accepted = next;
        target = priceWei;
        confidenceBps = confidenceBps_;
        depthItems = depthItems_;
        postedAt = uint64(block.timestamp);
        postedAtBlock = uint64(block.number);
        emit FloorPosted(priceWei, next, moved, confidenceBps_, depthItems_);
    }

    /// @notice What the accepted floor may become if the keeper posts now: the last accepted floor ± `maxMoveBps` × the slower
    /// clock's time since the last post, one hour's worth at most, and within `MAX_DAY_MOVE_BPS` of the day's anchor (the accepted
    /// floor now, if the day window has passed). (0, max) before the first post.
    function floorBounds() public view returns (uint256 lo, uint256 hi) {
        uint256 prev = accepted;
        if (prev == 0) return (0, type(uint256).max);
        uint256 el = Math.min(_slowElapsed(postedAt, postedAtBlock), HOUR);
        uint256 step = Math.mulDiv(prev, uint256(maxMoveBps) * el, BPS * HOUR);
        (uint256 anchor,) = _dayAnchorNow();
        uint256 dLo = Math.mulDiv(anchor, BPS - MAX_DAY_MOVE_BPS, BPS, Math.Rounding.Ceil);
        uint256 dHi = Math.mulDiv(anchor, BPS + MAX_DAY_MOVE_BPS, BPS);
        lo = Math.max(prev - step, Math.min(dLo, prev));
        hi = Math.min(prev + step, Math.max(dHi, prev));
    }

    /// @dev The day's anchor, and whether a post now opens a new window (the last one is a day old on both clocks).
    function _dayAnchorNow() private view returns (uint256 anchor, bool reset) {
        if (dayAnchor == 0 || Wait.elapsed(uint64(dayAnchorAt + DAY_WINDOW),
            uint64(dayAnchorAtBlock + Wait.blocksFor(DAY_WINDOW, parentBlockSeconds)))) return (accepted, true);
        return (dayAnchor, false);
    }

    function _setDayAnchor(uint256 anchor) private {
        dayAnchor = anchor;
        dayAnchorAt = uint64(block.timestamp);
        dayAnchorAtBlock = uint64(block.number);
    }

    /// @notice The keeper: the Merkle root of items eligible for renting (floor items only), leaves `keccak256(abi.encode(itemKey))`.
    function postRentalRoot(bytes32 root) external {
        if (msg.sender != registry.keeper()) revert NotKeeper();
        rentalRoot = root;
        emit RentalRootPosted(root);
    }

    // ---- mint and burn --------------------------------------------------------------------------------------------------------

    /// @notice Pay `msg.value`; receive `msg.value / mintPrice()` wX (at least `minUnits`), ETH-backed until an item arrives.
    function mint(address to, uint256 minUnits) external payable nonReentrant returns (uint256 units) {
        _priced();
        if (to == address(0) || msg.value == 0) revert BadInput();
        (uint256 sMint,) = spreads();
        uint256 base = _mintBase();
        uint256 unitPrice = Math.mulDiv(base, BPS + sMint, BPS, Math.Rounding.Ceil);
        units = Math.mulDiv(msg.value, ONE, unitPrice);
        if (units == 0 || units < minUnits) revert Slippage();
        _noteMintBase(base);
        (, uint256 rUnits) = _split(msg.value, units);
        if (_stressUnits() + units - rUnits > publicCap()) revert OverBacked();
        if (units > _gradRoom()) revert OverBacked();                    // B-07: never the room a queued graduation waits for
        (uint256 lvl,) = _level(mintLevel, mintAt, mintAtBlock, mintPerHour);
        if (lvl + units > mintPerHour) revert OverCap();
        mintLevel = lvl + units;
        mintAt = uint64(block.timestamp);
        mintAtBlock = uint64(block.number);
        _intake(msg.value, units);
        _volume(msg.value);
        emit Minted(msg.sender, to, msg.value, units, unitPrice);
        _w.moduleMint(to, units);
    }

    /// @dev A mint's ETH and units, `reserveBps` of each into the standing reserve, the rest convertible (§13.1).
    function _intake(uint256 eth, uint256 units) private {
        (uint256 rEth, uint256 rUnits) = _split(eth, units);
        reserve += rEth;
        reserveUnits += rUnits;
        backing += eth - rEth;
    }

    /// @dev The reserve's share of a mint. A share too small to carry both a wei and a unit is all convertible: a reserve unit
    /// is never left without backing.
    function _split(uint256 eth, uint256 units) private view returns (uint256 rEth, uint256 rUnits) {
        rEth = (eth * reserveBps) / BPS;
        rUnits = (units * reserveBps) / BPS;
        if (rEth == 0 || rUnits == 0) { rEth = 0; rUnits = 0; }
    }

    /// @notice Return `units` wX (approved to this module) for at least `minEth`, paid to `to`. Never more per unit than the ETH
    /// the units of each tier paid in. The reserve units go first and a burn spans both tiers (B-01). While the floor is fresh
    /// each tier pays the lower of the floor (the keeper's target counted no lower than the floor may legally reach within the
    /// next hour, N-1a) less the spread and its backing per unit. While the keeper is paused (or zero), each pays the lower of its
    /// backing per unit and the floor the keeper left (`pauseFloor()`), less the base spread (N-1b, R3 check A3-M1): no pause
    /// stops an exit, and none pays a unit minted before it more than it cost. Once no good post has been made for `FLOOR_TTL +
    /// STALE_ESCAPE` (`exitAtBacking()`), each pays its backing per unit less the base spread, with no floor (B-04). Only a stale
    /// floor with the keeper not paused, for less than the stale window, refuses (`Stale`).
    function burn(uint256 units, uint256 minEth, address to) external nonReentrant returns (uint256 eth) {
        if (units == 0 || to == address(0)) revert BadInput();
        (uint256 pr, uint256 pc) = _tierPrices();                       // reverts Stale unless burnsOpen()
        uint256 e = _liveUnits();                                        // never the unit in transit (relay-conversion §2.2)
        if (units > e) revert NotEnoughBacked();
        (uint256 lvl,) = _level(burnLevel, burnAt, burnAtBlock, burnPerHour);
        if (lvl + units > burnPerHour) revert OverCap();
        burnLevel = lvl + units;
        burnAt = uint64(block.timestamp);
        burnAtBlock = uint64(block.number);
        uint256 released;
        (eth, released) = _release(units, e, pr, pc);
        if (eth == 0 || eth < minEth) revert Slippage();
        _volume(eth);
        emit Burned(msg.sender, to, units, eth, released);
        IERC20(address(wrapper)).safeTransferFrom(msg.sender, address(this), units);
        _w.moduleBurn(units);
        _send(to, eth);
    }

    /// @dev The price per whole unit of each tier now: `pr` for reserve units, `pc` for convertible ones (0 for a tier with no
    /// units). Escaped: the backing per unit less the base spread. Keeper down: the lower of the backing per unit and the pause
    /// floor, less the base spread (A3-M1). Fresh: the lower of the floor less the burn spread and the tier's backing per unit.
    /// Otherwise the module is stale.
    function _tierPrices() private view returns (uint256 pr, uint256 pc) {
        uint256 e = _liveUnits();
        uint256 r = reserveUnits;
        uint256 bR = r == 0 ? 0 : Math.mulDiv(reserve, ONE, r);
        uint256 bC = e > r ? Math.mulDiv(backing, ONE, e - r) : 0;
        if (escaped()) return (Math.mulDiv(bR, BPS - baseSpreadBps, BPS), Math.mulDiv(bC, BPS - baseSpreadBps, BPS));
        if (_keeperDown()) {
            uint256 f = _pauseFloor();
            return (Math.mulDiv(Math.min(bR, f), BPS - baseSpreadBps, BPS), Math.mulDiv(Math.min(bC, f), BPS - baseSpreadBps, BPS));
        }
        if (!fresh()) revert Stale();
        (, uint256 sBurn) = spreads();
        uint256 p = Math.mulDiv(Math.min(price(), _burnTarget()), BPS - sBurn, BPS);
        return (Math.min(p, bR), Math.min(p, bC));
    }

    /// @dev R3 check A3-M1, R4 check A4-L1: the floor a burn counts while the keeper is down. Nobody can post or mint then, so
    /// `accepted`, `target`, `postedAt` and the mint lows are the ones the pause froze. It is the least of
    /// - `_burnTarget()` (the fresh formula's target, never below the next hour's legal floor, N-1a);
    /// - the fill band's low edge `accepted × (1 − fillBandBps)`: `price()` never sits below it, so no mint under ANY post of this
    ///   accepted floor paid less per unit before its spread (A4-L1: a mint under a low target, then a post that raises it back);
    /// - the least a public mint paid per unit before its spread in the 12 to 24 hours before the last mint was possible (a mint
    ///   before a rise of `accepted`), when there was one.
    /// So a unit minted in that time never exits a pause above its cost. Positive once a floor was posted; it can only fall while
    /// the pause lasts (the day anchor in `_burnTarget()` is the only moving part).
    function _pauseFloor() private view returns (uint256) {
        uint256 f = Math.min(_burnTarget(), Math.mulDiv(accepted, BPS - fillBandBps, BPS));
        uint256 b = (uint256(postedAt) + FLOOR_TTL) / MINT_LOW_BUCKET;
        for (uint256 i = 0; i < 2; ++i) {
            uint256 v = _mintLow[i];
            uint256 at = uint64(v);
            if (v != 0 && (at == b || at + 1 == b)) f = Math.min(f, v >> 64);
        }
        return f;
    }

    /// @dev A mint's price per whole unit before its spread: the fills price, or the keeper's target as mints count it if higher.
    function _mintBase() private view returns (uint256) {
        return Math.max(price(), _upTarget());
    }

    /// @dev A4-L1: keep the least pre-spread price a public mint paid in this `MINT_LOW_BUCKET` (an older bucket in the slot is
    /// replaced: only this bucket and the one before are ever read).
    function _noteMintBase(uint256 base) private {
        uint256 b = block.timestamp / MINT_LOW_BUCKET;
        uint256 i = b % 2;
        uint256 v = _mintLow[i];
        if (v == 0 || uint64(v) != b || base < v >> 64) _mintLow[i] = (base << 64) | b;
    }

    /// @dev N-1a: the keeper's target as burns count it — never below what the accepted floor may legally become within the next
    /// hour of posts (`maxMoveBps` below it) nor below the day band, whichever is higher. The lag is priced against the trader
    /// as far as the floor can really move before the next post; a lower post (1 wei, say) says nothing the module would act on,
    /// so it cannot zero a burn while the module stays fresh.
    function _burnTarget() private view returns (uint256) {
        uint256 a = accepted;
        (uint256 anchor,) = _dayAnchorNow();
        uint256 dayLo = Math.mulDiv(anchor, BPS - MAX_DAY_MOVE_BPS, BPS, Math.Rounding.Ceil);
        uint256 lo = Math.max(Math.mulDiv(a, BPS - maxMoveBps, BPS, Math.Rounding.Ceil), Math.min(dayLo, a));
        return Math.max(target, lo);
    }

    /// @dev The keeper's target as mints and graduations count it: no higher than the top of the day band (A-10), so a far-off
    /// post cannot price them out either. It never makes a mint cheaper than `price()` plus the spread.
    function _upTarget() private view returns (uint256) {
        (uint256 anchor,) = _dayAnchorNow();
        return Math.min(target, Math.max(Math.mulDiv(anchor, BPS + MAX_DAY_MOVE_BPS, BPS), accepted));
    }

    /// @dev A burn of `units` (at most E): `min(units, R)` reserve units at `pr`, the rest convertible units at `pc`. Each tier
    /// releases exactly its backing per unit (⌊pot·u/n⌋; the last unit takes the rounding dust) and pays at most that; what the
    /// reserve tier keeps goes to the surplus (so the reserve's backing per unit never moves on a burn and no burn is ever paid
    /// from the surplus), what the convertible tier keeps is revenue.
    /// @return eth what the burner is paid
    /// @return released what left the backing pots
    function _release(uint256 units, uint256 e, uint256 pr, uint256 pc) private returns (uint256 eth, uint256 released) {
        uint256 r = reserveUnits;
        uint256 rU = Math.min(units, r);
        if (rU != 0) {
            uint256 rel = rU == r ? reserve : Math.mulDiv(reserve, rU, r);
            uint256 paid = Math.mulDiv(rU, pr, ONE);
            if (paid > rel) revert NotEnoughBacked();                        // unreachable: pr ≤ ⌊reserve·ONE/r⌋
            reserve -= rel;
            reserveUnits = r - rU;
            surplus += rel - paid;
            eth = paid;
            released = rel;
        }
        uint256 cU = units - rU;
        if (cU != 0) {
            uint256 c = e - r;                                               // cU ≤ e − r: units ≤ e
            uint256 rel = cU == c ? backing : Math.mulDiv(backing, cU, c);
            uint256 paid = Math.mulDiv(cU, pc, ONE);
            if (paid > rel) revert NotEnoughBacked();                        // unreachable: pc ≤ ⌊backing·ONE/c⌋
            backing -= rel;
            _revenue(rel - paid);
            eth += paid;
            released += rel;
        }
    }

    // ---- the canister: IFloorModule (docs/v5/floor-feed.md §5.1, autobuyer/canister-rs/src/floorsink.rs) -----------------------

    /// @notice The custodian, having wrapped one item into this module: one ETH-backed unit becomes item-backed. `cost` is paid from
    /// that unit's backing, and beyond it from the revenue pot, then the reserve surplus (never the reserve's basis); never above
    /// `maxItemCost()`; at most `maxItemsPerDay` in any 24 h.
    function itemArrived(uint256 cost) external nonReentrant {
        if (msg.sender != wrapper.custodian()) revert NotCustodian();
        if (cost == 0) revert BadInput();
        uint256 max = _capOrSnapshot();
        if (max == 0) revert CapStale();
        if (cost > max) revert OverItemCap(cost, max);
        if (wrapper.balanceOf(address(this)) < gradUnits + floatUnits + ONE) revert NoWrapped();
        uint256 c = netUnits();
        if (c < ONE) revert NotEnoughBacked();
        uint256 released = Math.mulDiv(backing, ONE, c);
        backing -= released;
        if (c == ONE && backing != 0) { released += backing; backing = 0; }
        if (cost <= released) {
            _revenue(released - cost);
        } else {
            uint256 need = cost - released;
            if (need > revenue + surplus) revert OverBasis(need, revenue + surplus);
            uint256 fromRevenue = Math.min(need, revenue);
            revenue -= fromRevenue;
            surplus -= need - fromRevenue;
        }
        _countArrival();
        _fill(Math.mulDiv(cost, BPS, BPS + convBps), false);        // the floor the cost implies, net of the conversion allowance
        uint256 item = itemsArrived++;
        emit ItemArrived(item, cost, released);
        _w.moduleBurn(ONE);
        _send(payee, cost);
    }

    /// @notice The custodian, buying one whole unit of the protocol's inventory (`floatUnits`) for `proceeds` ETH, the proceeds
    /// of an item sale on the protocol's behalf (§13.4): never under `minSaleProceeds()`. The ETH joins the reserve surplus;
    /// the price is a fill.
    function itemSold(uint256 proceeds) external payable nonReentrant {
        if (msg.sender != wrapper.custodian()) revert NotCustodian();
        if (proceeds == 0 || msg.value != proceeds) revert BadInput();
        _priced();
        uint256 min = minSaleProceeds();
        if (proceeds < min) revert UnderSalePrice(proceeds, min);
        if (floatUnits < ONE) revert NoInventory();
        floatUnits -= ONE;
        surplus += proceeds;
        _fill(Math.mulDiv(proceeds, BPS, BPS - convBps), true);      // the floor the proceeds imply, gross of the allowance
        emit ItemSold(proceeds);
        IERC20(address(wrapper)).safeTransfer(msg.sender, ONE);
    }

    // ---- the advance (docs/v6/relay-conversion.md §2): the item's budget first, the item after ------------------------------

    /// @notice The custodian, about to buy an item with this module's own ETH (converted to what the item's chain is paid in): one
    /// convertible unit goes into transit and `amount` is paid to `payee` now, instead of at `itemArrived`. The money side is
    /// `itemArrived`'s exactly: the unit releases its backing per unit, the rest is revenue, a shortfall comes from the revenue pot
    /// then the surplus (never the reserve); never above `maxItemCost()` (0 when stale, while the keeper is paused, without a whole
    /// convertible unit or without arrival room today). H11 counts here, when the ETH leaves. One advance at a time.
    function advance(uint256 amount) external nonReentrant {
        if (msg.sender != wrapper.custodian()) revert NotCustodian();
        if (transitUnits != 0) revert AdvanceOpen();
        if (amount == 0) revert BadInput();
        uint256 max = maxItemCost();
        if (amount > max) revert OverItemCap(amount, max);
        uint256 c = netUnits();
        uint256 released = Math.mulDiv(backing, ONE, c);
        backing -= released;
        if (c == ONE && backing != 0) { released += backing; backing = 0; }
        if (amount <= released) {
            _revenue(released - amount);
        } else {
            uint256 need = amount - released;
            if (need > revenue + surplus) revert OverBasis(need, revenue + surplus);   // unreachable: amount ≤ maxItemCost()
            uint256 fromRevenue = Math.min(need, revenue);
            revenue -= fromRevenue;
            surplus -= need - fromRevenue;
        }
        _countArrival();
        transitUnits = ONE;
        advanced = amount;
        advanceBasis = released;
        advancedAt = uint64(block.timestamp);
        emit Advanced(amount, released);
        _send(payee, amount);
    }

    /// @notice The custodian, having wrapped the advanced item into this module: the transit unit becomes item-backed. `cost` (at
    /// most what was advanced) is the item's share of the advance, recorded as a fill (not while the keeper is paused). No ETH
    /// moves and H11 is not counted again.
    function settleAdvance(uint256 cost) external nonReentrant {
        if (msg.sender != wrapper.custodian()) revert NotCustodian();
        if (transitUnits == 0) revert NoAdvance();
        uint256 adv = advanced;
        if (cost == 0 || cost > adv) revert BadInput();
        if (wrapper.balanceOf(address(this)) < gradUnits + floatUnits + ONE) revert NoWrapped();
        transitUnits = 0;
        advanced = 0;
        advanceBasis = 0;
        advancedAt = 0;
        if (!_keeperDown()) _fill(Math.mulDiv(cost, BPS, BPS + convBps), false);
        uint256 item = itemsArrived++;
        emit AdvanceSettled(item, adv, cost);
        _w.moduleBurn(ONE);
    }

    /// @notice The custodian: the advance could not become an item; what is left of it comes back with this call and the transit
    /// unit is ETH-backed again. Its backing is restored to what it released: a shortfall comes from the revenue pot, then the
    /// surplus, and only what they cannot cover is a loss shared by the convertible units; anything above it is revenue. The daily
    /// count is not given back.
    function returnAdvance() external payable nonReentrant {
        if (msg.sender != wrapper.custodian()) revert NotCustodian();
        if (transitUnits == 0) revert NoAdvance();
        uint256 v = msg.value;
        uint256 basis = advanceBasis;
        uint256 loss;
        transitUnits = 0;
        advanced = 0;
        advanceBasis = 0;
        advancedAt = 0;
        if (v >= basis) {
            backing += basis;
            _revenue(v - basis);
        } else {
            uint256 need = basis - v;
            uint256 fromRevenue = Math.min(need, revenue);
            revenue -= fromRevenue;
            uint256 fromSurplus = Math.min(need - fromRevenue, surplus);
            surplus -= fromSurplus;
            loss = need - fromRevenue - fromSurplus;
            backing += basis - loss;
        }
        emit AdvanceReturned(v, basis, loss);
    }

    function _fill(uint256 fillPrice, bool sale) private {
        _fills[fillCount % FILLS] = Fill(uint192(fillPrice), uint64(block.timestamp));
        fillCount++;
        emit FillRecorded(fillPrice, sale);
    }

    function _volume(uint256 eth) private {
        uint256 day = block.timestamp / 1 days;
        uint256 i = day % DAYS;
        if (_dayOf[i] != day) { _dayOf[i] = day; _dayVolume[i] = 0; }
        _dayVolume[i] += eth;
    }

    /// @notice Anyone (the canister, before it buys): remember the fresh item cap for `SNAPSHOT_TTL`. Refused while the keeper is
    /// paused (the item cap is then 0) and a snapshot does not count during a pause (N-7).
    function snapshotCap() external {
        uint256 cap = itemCap();
        if (cap == 0) revert CapStale();
        snapCap = cap;
        snapAt = uint64(block.timestamp);
        snapAtBlock = uint64(block.number);
        emit CapSnapshot(cap);
    }

    /// @dev H11 on the module's side: the (n − maxItemsPerDay)-th arrival must be a day old on both clocks.
    function _countArrival() private {
        uint256 n = arrivals;
        if (n >= maxItemsPerDay) {
            uint256 st = _arrivedAt[n - maxItemsPerDay];
            if (!Wait.elapsed(uint64(st >> 64) + 1 days, uint64(st) + uint64(Wait.blocksFor(1 days, parentBlockSeconds)))) {
                revert ArrivalRateLimited();
            }
        }
        _arrivedAt[n] = (block.timestamp << 64) | block.number;
        arrivals = n + 1;
    }

    /// @notice Whether an item arrival (a conversion or a rental) would be within the daily limit now.
    function arrivalRoom() public view returns (bool) {
        uint256 n = arrivals;
        if (n < maxItemsPerDay) return true;
        uint256 st = _arrivedAt[n - maxItemsPerDay];
        return Wait.elapsed(uint64(st >> 64) + 1 days, uint64(st) + uint64(Wait.blocksFor(1 days, parentBlockSeconds)));
    }

    /// @notice The custodian: units here that nobody is owed (a unit minted for a conversion that could not be recorded).
    function reclaimStray() external nonReentrant returns (uint256 amount) {
        address c = wrapper.custodian();
        if (msg.sender != c) revert NotCustodian();
        amount = wrapper.balanceOf(address(this)) - gradUnits - floatUnits;
        if (amount != 0) IERC20(address(wrapper)).safeTransfer(c, amount);
        emit StrayReclaimed(amount);
    }

    // ---- the graduation lane (IItemQueue) --------------------------------------------------------------------------------------

    function _onlyCurve(address coin) private view {
        if (address(factory) == address(0)) revert NotWired();
        if (coin == address(0) || msg.sender != factory.routerOf(coin)) revert NotCurve();
    }

    /// @inheritdoc IItemQueue
    /// @dev Converted at once at `gradPrice()` while the floor is fresh and `maxEthBacked` allows; otherwise it waits for `fill`.
    function enqueue(address coin, uint64, uint64) external payable nonReentrant {
        _onlyCurve(coin);
        Grad storage g = _grads[coin];
        if (g.status != GradStatus.None || msg.value == 0) revert BadInput();
        g.status = GradStatus.Queued;
        g.curve = msg.sender;
        g.eth = msg.value;
        gradEth += msg.value;
        emit GradQueued(coin, msg.value);
        _tryFill(coin);
    }

    /// @notice Anyone: convert a waiting graduation now.
    function fill(address coin) external nonReentrant {
        if (_grads[coin].status != GradStatus.Queued) revert WrongStatus();
        if (!_tryFill(coin)) revert NoRoom();
    }

    function _tryFill(address coin) private returns (bool) {
        if (!fresh() || _keeperDown()) return false;
        Grad storage g = _grads[coin];
        uint256 unitPrice = gradPrice();
        uint256 units = Math.mulDiv(g.eth, ONE, unitPrice);
        if (units == 0 || _w.ethBacked() + units > maxEthBacked) return false;
        g.status = GradStatus.Filled;
        g.units = units;
        gradEth -= g.eth;
        _intake(g.eth, units);
        gradUnits += units;
        emit GradFilled(coin, units, unitPrice);
        _w.moduleMint(address(this), units);
        return true;
    }

    /// @inheritdoc IItemQueue
    function isFilled(address coin) external view returns (bool) { return _grads[coin].status == GradStatus.Filled; }

    /// @inheritdoc IItemQueue
    /// @dev `_tryFill`'s room test for `eth` behind the budgets already queued, at today's `gradPrice()`. Freshness is not
    /// required: a stale floor is transient and the queued budget is then filled by `fill`. The keeper's pause is (N-3): it lasts
    /// the 7-day rotation, and nothing is filled meanwhile, so a curve keeps trading instead of closing into a refund.
    function hasRoomFor(uint256 eth) external view returns (bool) {
        if (_keeperDown()) return false;
        uint256 unitPrice = gradPrice();
        if (unitPrice == 0) return false;
        uint256 units = Math.mulDiv(gradEth + eth, ONE, unitPrice);
        return units != 0 && _w.ethBacked() + units <= maxEthBacked;
    }

    /// @inheritdoc IItemQueue
    /// @dev While the keeper is paused nothing is filled, so an unfilled curve waits (up to its escape hatch) for the unpause, and
    /// for `RESUME_GRACE` after it on both clocks (R3 check A3-L1): the floor is stale at the unpause and `fill` needs the keeper's
    /// first post. The grace does not end at that post, or the race would move to a back-run of it.
    function holdsRefunds() external view returns (bool) {
        if (_keeperDown()) return true;
        uint64 at = registry.keeperResumedAt();
        if (at == 0) return false;
        return !Wait.elapsed(uint64(at + RESUME_GRACE),
            uint64(registry.keeperResumedAtBlock() + Wait.blocksFor(RESUME_GRACE, parentBlockSeconds)));
    }

    /// @inheritdoc IItemQueue
    function release(address coin) external nonReentrant returns (uint256 units, uint256 spent, uint256 dust) {
        _onlyCurve(coin);
        Grad storage g = _grads[coin];
        if (g.status != GradStatus.Filled) revert WrongStatus();
        g.status = GradStatus.Released;
        units = g.units;
        spent = g.eth;
        dust = 0;
        gradUnits -= units;
        emit GradReleased(coin, units);
        IERC20(address(wrapper)).safeTransfer(msg.sender, units);
    }

    /// @inheritdoc IItemQueue
    function cancel(address coin) external nonReentrant returns (uint256 eth, uint256 units) {
        _onlyCurve(coin);
        Grad storage g = _grads[coin];
        if (g.status != GradStatus.Queued) revert WrongStatus();
        g.status = GradStatus.Cancelled;
        eth = g.eth;
        units = 0;
        gradEth -= eth;
        emit GradCancelled(coin, eth);
        _send(msg.sender, eth);
    }

    // ---- renting (docs/v5/floor-module.md §9) ------------------------------------------------------------------------------------

    /// @notice The custodian: admit a holder's floor item for `rentTerm`. It takes over one ETH-backed unit, whose backing waits
    /// here for the rental: back into the backing when the item goes home, or toward the renter's compensation if a ticket takes it.
    function rentIn(address renter, bytes calldata returnTo, bytes32[] calldata eligibility, bytes calldata attestation)
        external nonReentrant returns (uint256 rentalId)
    {
        if (msg.sender != wrapper.custodian()) revert NotCustodian();
        if (renter == address(0)) revert BadInput();
        if (_keeperDown()) revert Paused();
        if (!rentalRoom()) revert NoRoom();
        _countArrival();
        (uint256 itemId, bytes32 key) = _w.admitRental(attestation, returnTo);
        if (!MerkleProof.verifyCalldata(eligibility, rentalRoot, keccak256(abi.encode(key)))) revert NotEligible();
        uint256 c = netUnits();
        if (c < ONE) revert NotEnoughBacked();
        uint256 escrow = Math.mulDiv(backing, ONE, c);
        backing -= escrow;
        rentalEscrow += escrow;
        rentalId = _rentals.length;
        _rentals.push(Rental(RentStatus.Lent, renter, uint64(block.timestamp + rentTerm),
            uint64(block.number + Wait.blocksFor(rentTerm, parentBlockSeconds)), itemId, escrow, accPerRental, 0));
        _rentalOfItem[itemId] = rentalId + 1;
        activeRentals++;
        emit RentedIn(rentalId, itemId, renter, escrow);
        _w.moduleBurn(ONE);
    }

    /// @notice Anyone, once the term is over on both clocks and the item is not on a ticket: send it home. It leaves the random set
    /// after every ticket burned before now has resolved; the custodian returns it to the address fixed at intake.
    function endRental(uint256 rentalId) external nonReentrant {
        Rental storage r = _rentals[rentalId];
        if (r.status != RentStatus.Lent) revert WrongStatus();
        if (!Wait.elapsed(r.endsAt, r.endsAtBlock)) revert TooEarly();
        if (_w.rentalState(r.itemId) != IFloorWrapped.RentalState.Active) revert Drawn();
        _accrue(r);
        activeRentals--;
        leavingRentals++;
        r.status = RentStatus.Leaving;
        emit RentalEnded(rentalId);
        _w.returnRental(r.itemId);
    }

    /// @notice The wrapper, as the custodian records a rented item's return (`recordReturn`): that call makes the unit the item
    /// backed ETH-backed again, so here, in the same transaction, the rental closes and its escrow backs that unit again. Until
    /// 2026-09-28 this waited for a separate `settleRental`, and in between the unit counted in every backing-per-unit division
    /// with no backing: a conversion could take it for the revenue pot's money and strand the escrow, or burns paid it short.
    function rentalReturned(uint256 itemId) external nonReentrant {
        if (msg.sender != address(wrapper)) revert NotWrapper();
        uint256 id = _rentalOfItem[itemId];
        if (id == 0) revert NothingToSettle();
        Rental storage r = _rentals[id - 1];
        if (r.status != RentStatus.Leaving) revert WrongStatus();         // the wrapper's Ready → Returned implies Leaving here
        leavingRentals--;
        r.status = RentStatus.Closed;
        rentalEscrow -= r.escrow;
        backing += r.escrow;
        emit RentalSettled(id - 1, false, 0, 0);
    }

    /// @notice Anyone: close a rental whose item was taken by a ticket and delivered (the renter is credited the floor plus the
    /// premium). A returned item's rental is closed by the wrapper's `recordReturn` itself (`rentalReturned`).
    function settleRental(uint256 rentalId) external nonReentrant {
        Rental storage r = _rentals[rentalId];
        IFloorWrapped.RentalState ws = _w.rentalState(r.itemId);
        if (ws == IFloorWrapped.RentalState.Taken && (r.status == RentStatus.Lent || r.status == RentStatus.Leaving)) {
            if (!fresh()) revert Stale();
            if (r.status == RentStatus.Lent) { _accrue(r); activeRentals--; } else { leavingRentals--; }
            r.status = RentStatus.Closed;
            uint256 comp = Math.mulDiv(accepted, BPS + premiumBps, BPS);
            uint256 escrow = r.escrow;
            rentalEscrow -= escrow;
            uint256 pay = comp;
            uint256 short;
            if (escrow >= comp) {
                revenue += escrow - comp;
            } else {
                uint256 need = comp - escrow;
                uint256 fromRevenue = Math.min(need, revenue);
                revenue -= fromRevenue;
                short = need - fromRevenue;
                pay = comp - short;
                debt[rentalId] = short;
                debtTotal += short;
            }
            r.paid = pay;
            owed[r.renter] += pay;
            owedTotal += pay;
            emit RentalSettled(rentalId, true, pay, short);
        } else {
            revert NothingToSettle();
        }
    }

    /// @notice Anyone: pay what the revenue pot can of a taken rental's unpaid compensation.
    function payDebt(uint256 rentalId) external nonReentrant {
        uint256 d = debt[rentalId];
        uint256 amount = Math.min(d, revenue);
        if (amount == 0) revert NothingToSettle();
        debt[rentalId] = d - amount;
        debtTotal -= amount;
        revenue -= amount;
        Rental storage r = _rentals[rentalId];
        r.paid += amount;
        owed[r.renter] += amount;
        owedTotal += amount;
    }

    /// @notice What you are owed: rental earnings and compensation.
    function claim() external nonReentrant returns (uint256 amount) {
        amount = owed[msg.sender];
        if (amount == 0) revert BadInput();
        owed[msg.sender] = 0;
        owedTotal -= amount;
        emit Claimed(msg.sender, amount);
        _send(msg.sender, amount);
    }

    /// @notice Anyone: the revenue beyond `revenueTarget` (and any unpaid rental debt) to the platform — the treasury (§13.3).
    /// Once `escaped()` (no good post for a week, so no conversion can use them), all the revenue beyond the debt and the whole
    /// reserve surplus go with it: no ETH is ever locked here for good (B-04).
    function skim() external nonReentrant returns (uint256 amount) {
        bool esc = escaped();
        uint256 keep = (esc ? 0 : revenueTarget) + debtTotal;
        if (revenue > keep) {
            amount = revenue - keep;
            revenue = keep;
        }
        if (surplus != 0 && esc) {
            amount += surplus;
            surplus = 0;
        }
        if (amount == 0) revert NothingToSettle();
        emit Skimmed(amount);
        _send(platform, amount);
    }

    // ---- funding (docs/v5/floor-module.md §13.2, §13.3) ------------------------------------------------------------------------

    /// @notice Anyone, with ETH: the collection's reserve slice of a coin's fees, or the treasury's reserve allocation. A bootstrap
    /// loan is repaid first; the rest joins the reserve surplus (it tops up conversions; no burn is paid from it, B-02).
    function fundReserve() external payable nonReentrant {
        if (msg.value == 0) revert BadInput();
        uint256 left = _repay(msg.value);
        surplus += left;
        emit ReserveFunded(msg.sender, left);
    }

    /// @notice Anyone, with ETH: a coin's protocol-liquidity slice. A bootstrap loan is repaid first; the rest goes to the
    /// collection's liquidity lock, protocol-owned, never withdrawable.
    function fundLiquidity() external payable nonReentrant {
        if (msg.value == 0) revert BadInput();
        if (address(seeder) == address(0)) revert NotWired();
        uint256 left = _repay(msg.value);
        emit LiquidityFunded(msg.sender, left);
        if (left != 0) seeder.addProtocolLiquidity{value: left}();
    }

    /// @notice The treasury: a float (§13.3) — a mint to this module at the mint price with no reserve share, so the canister
    /// buys items ahead of demand; the units are the protocol's inventory (`floatUnits`), sold back by `itemSold`.
    function fundFloat() external payable nonReentrant returns (uint256 units) {
        if (msg.sender != treasury || treasury == address(0)) revert NotTreasury();
        _priced();
        if (msg.value == 0) revert BadInput();
        uint256 unitPrice = mintPrice();
        units = Math.mulDiv(msg.value, ONE, unitPrice);
        if (units == 0) revert Slippage();
        if (units > _gradRoom()) revert OverBacked();                    // maxEthBacked, after the queued graduations (B-07)
        backing += msg.value;
        floatUnits += units;
        emit FloatFunded(msg.value, units, unitPrice);
        _w.moduleMint(address(this), units);
    }

    /// @dev What the treasury is owed of its bootstrap loan, paid from `amount`; returns what is left.
    function _repay(uint256 amount) private returns (uint256) {
        if (treasury == address(0)) return amount;
        uint256 due = ITreasuryLoans(treasury).outstanding(address(this));
        uint256 pay = Math.min(due, amount);
        if (pay == 0) return amount;
        emit LoanRepaid(pay);
        ITreasuryLoans(treasury).repay{value: pay}();
        return amount - pay;
    }

    /// @notice What the treasury still holds of this module's bootstrap loan (0 without a treasury or a loan).
    function loanOutstanding() external view returns (uint256) {
        return treasury == address(0) ? 0 : ITreasuryLoans(treasury).outstanding(address(this));
    }

    /// @notice Mint and burn volume, ETH, over the trailing seven days (today included): the treasury's allocation weight.
    function volume7d() external view returns (uint256 v) {
        uint256 today = block.timestamp / 1 days;
        for (uint256 i = 0; i < DAYS; ++i) {
            if (_dayOf[i] + DAYS > today && _dayOf[i] <= today) v += _dayVolume[i];
        }
    }

    function _accrue(Rental storage r) private {
        uint256 earned = (accPerRental - r.accAt) / ACC;
        r.accAt = accPerRental;
        if (earned == 0) return;
        if (earned > renterPool) earned = renterPool;
        renterPool -= earned;
        owed[r.renter] += earned;
        owedTotal += earned;
    }

    function _revenue(uint256 amount) private {
        if (amount == 0) return;
        uint256 toRenters;
        if (activeRentals != 0 && rentShareBps != 0) {
            toRenters = (amount * rentShareBps) / BPS;
            accPerRental += (toRenters * ACC) / activeRentals;
            renterPool += toRenters;
        }
        revenue += amount - toRenters;
        emit Revenue(amount, toRenters);
    }

    // ---- prices ---------------------------------------------------------------------------------------------------------------

    /// @notice Whether the keeper's floor is fresh on both clocks.
    function fresh() public view returns (bool) {
        return accepted != 0 && confidenceBps >= minConfidenceBps && block.timestamp < uint256(postedAt) + FLOOR_TTL
            && block.number < uint256(postedAtBlock) + Wait.blocksFor(FLOOR_TTL, parentBlockSeconds);
    }

    /// @notice The stale fallback (B-04): not fresh, and no good post (confidence ≥ `minConfidenceBps`) for `FLOOR_TTL +
    /// STALE_ESCAPE` on both clocks. Burns then pay the backing per unit with no floor, and `skim` takes the surplus.
    function escaped() public view returns (bool) {
        if (fresh() || lastGoodAt == 0) return false;
        uint256 wait = FLOOR_TTL + STALE_ESCAPE;
        return Wait.elapsed(uint64(lastGoodAt + wait), uint64(lastGoodAtBlock + Wait.blocksFor(wait, parentBlockSeconds)));
    }

    /// @dev An inflow priced by the keeper's word (a mint, a float, a sale): a fresh floor, and a keeper (not paused, not zero).
    function _priced() private view {
        if (!fresh()) revert Stale();
        if (_keeperDown()) revert Paused();
    }

    /// @dev The keeper is paused, or there is none: no word of it may price anything, and burns exit at the backing.
    function _keeperDown() private view returns (bool) {
        return registry.keeperPaused() || registry.keeper() == address(0);
    }

    /// @notice Whether burns pay each tier's backing per unit less the base spread, with no floor: once `escaped()` (B-04). While
    /// the keeper is paused they pay no more than the pause floor instead (`pauseFloor()`, R3 check A3-M1).
    function exitAtBacking() public view returns (bool) {
        return escaped();
    }

    /// @notice Whether a burn is priced now: a fresh floor, the keeper paused (or zero), or the stale fallback. Only an ordinary
    /// keeper outage shorter than the stale window refuses burns (`Stale`).
    function burnsOpen() public view returns (bool) {
        return fresh() || _keeperDown() || escaped();
    }

    /// @notice The floor paused burns count (R3 check A3-M1): while the keeper is paused (or zero) and the module has not escaped,
    /// the floor the keeper left, capped at the fill band's low edge and at the least a public mint paid per unit before its spread
    /// in the 12 to 24 hours before the pause (R4 check A4-L1); 0 otherwise. A paused burn
    /// pays each tier the lower of this and its backing per unit, less the base spread.
    function pauseFloor() external view returns (uint256) {
        if (!_keeperDown() || escaped()) return 0;
        return _pauseFloor();
    }

    /// @notice The keeper the module obeys now: the registry's (zero while paused).
    function keeper() external view returns (address) { return registry.keeper(); }

    /// @notice The accepted floor's movement over the trailing hour, bps (decaying linearly on the slower clock).
    function volBps() public view returns (uint256) {
        uint256 el = Math.min(_slowElapsed(postedAt, postedAtBlock), HOUR);
        return Math.mulDiv(moveAcc, HOUR - el, HOUR, Math.Rounding.Ceil);
    }

    /// @notice The spread each side pays now, bps (docs/v5/floor-module.md §6.1).
    function spreads() public view returns (uint256 sMint, uint256 sBurn) {
        uint256 s = baseSpreadBps + (volBps() * volWeightBps) / BPS;
        if (confidenceBps < fullConfidenceBps) {
            s += Math.mulDiv(lowConfidenceSpreadBps, fullConfidenceBps - confidenceBps, fullConfidenceBps, Math.Rounding.Ceil);
        }
        uint256 age = _fastElapsed(postedAt, postedAtBlock);
        if (age > STALE_GRACE) {
            s += Math.mulDiv(staleSpreadBps, Math.min(age - STALE_GRACE, FLOOR_TTL - STALE_GRACE), FLOOR_TTL - STALE_GRACE, Math.Rounding.Ceil);
        }
        uint256 pc = publicCap();
        uint256 stress = pc == 0 ? stressSpreadBps : Math.mulDiv(stressSpreadBps, Math.min(_stressUnits(), pc), pc, Math.Rounding.Ceil);
        sMint = Math.min(maxSpreadBps, s + stress);
        sBurn = Math.min(maxSpreadBps, s);
    }

    /// @notice The floor the module prices at (§13.4): the time-weighted average of the protocol's own fills over the trailing
    /// `FILL_WINDOW` — each purchase's cost and each sale's proceeds, normalised by the conversion allowance to the floor they
    /// imply — clamped to within `fillBandBps` of the accepted keeper floor; the accepted floor alone with no fill in the
    /// window. Before the first fill in the window the accepted floor stands in for the price.
    function price() public view returns (uint256) {
        uint256 acc = accepted;
        uint256 n = fillCount;
        if (acc == 0 || n == 0) return acc;
        uint256 start = block.timestamp > FILL_WINDOW ? block.timestamp - FILL_WINDOW : 0;
        uint256 kept = Math.min(n, FILLS);
        Fill memory newest = _fills[(n - 1) % FILLS];
        if (newest.at < start) return acc;
        // walk the kept fills oldest first; each holds its price until the next
        uint256 weighted;
        uint256 span;
        uint256 prevPrice = acc;
        uint256 prevAt = start;
        for (uint256 i = 0; i < kept; ++i) {
            Fill memory f = _fills[(n - kept + i) % FILLS];
            if (f.at <= start) continue;                        // outside the window: the accepted floor stands in
            weighted += prevPrice * (f.at - prevAt);
            span += f.at - prevAt;
            prevPrice = f.price;
            prevAt = f.at;
        }
        weighted += prevPrice * (block.timestamp - prevAt);
        span += block.timestamp - prevAt;
        uint256 twap = span == 0 ? prevPrice : weighted / span;
        uint256 lo = Math.mulDiv(acc, BPS - fillBandBps, BPS);
        uint256 hi = Math.mulDiv(acc, BPS + fillBandBps, BPS, Math.Rounding.Ceil);
        return twap < lo ? lo : (twap > hi ? hi : twap);
    }

    /// @notice Wei per whole wX minted now: the higher of the fills price and the posted floor, plus the mint spread.
    function mintPrice() public view returns (uint256) {
        (uint256 sMint,) = spreads();
        return Math.mulDiv(_mintBase(), BPS + sMint, BPS, Math.Rounding.Ceil);
    }

    /// @notice Wei per whole wX the next unit burned is paid now: the lower of the fills price and the posted floor (bounded as
    /// `burn` says) less the burn spread, and never more than the backing per unit of its tier (the reserve's while it has units,
    /// else the convertible pot's); while the keeper is paused, the lower of that backing and `pauseFloor()` less the base
    /// spread; while `exitAtBacking()`, that backing less the base spread. 0 when nothing can be burned.
    function burnPrice() public view returns (uint256 p) {
        if (!burnsOpen()) return 0;
        (uint256 pr, uint256 pc) = _tierPrices();
        return reserveUnits != 0 ? pr : pc;
    }

    /// @notice What a graduation pays per whole unit: the all-in floor the canister will pay for the item. The keeper's raw
    /// `target` counts (the lag is priced against the curve, not the module) only up to the top of the day's band (A-10): a keeper
    /// posting a far-off target cannot shrink a graduation's units by more than the band allows.
    function gradPrice() public view returns (uint256) {
        return Math.mulDiv(Math.max(price(), _upTarget()), BPS + convBps, BPS, Math.Rounding.Ceil);
    }

    /// @notice The burn cap: the backing per whole unit of the tier burns draw now — the reserve's basis while it has units, else
    /// the convertible pot's (0 when no unit is ETH-backed). The surplus is never part of it.
    function beta() public view returns (uint256) {
        uint256 e = _liveUnits();
        if (e == 0) return 0;
        uint256 r = reserveUnits;
        return r != 0 ? Math.mulDiv(reserve, ONE, r) : Math.mulDiv(backing, ONE, e);
    }

    /// @notice The convertible pot's backing per whole convertible unit: what a conversion releases (0 when none).
    function betaConvertible() external view returns (uint256) {
        uint256 c = netUnits();
        return c == 0 ? 0 : Math.mulDiv(backing, ONE, c);
    }

    /// @notice The reserve's basis per whole reserve unit: what a reserve unit paid in, on average (0 when none).
    function betaReserve() external view returns (uint256) {
        uint256 r = reserveUnits;
        return r == 0 ? 0 : Math.mulDiv(reserve, ONE, r);
    }

    /// @notice The least a sale may report (§13.4): the accepted floor less the conversion allowance.
    function minSaleProceeds() public view returns (uint256) {
        return Math.mulDiv(accepted, BPS - convBps, BPS, Math.Rounding.Ceil);
    }

    /// @dev B-07: the ETH-backed units a public mint may still add under `maxEthBacked` after the graduations already queued (their
    /// budgets at today's `gradPrice()`), so a mint never takes the room a `Queued` graduation is waiting for.
    function _gradRoom() private view returns (uint256) {
        uint256 queued = gradEth == 0 ? 0 : Math.mulDiv(gradEth, ONE, gradPrice(), Math.Rounding.Ceil);
        uint256 used = _w.ethBacked() + queued;
        return used >= maxEthBacked ? 0 : maxEthBacked - used;
    }

    /// @dev The units the stress term and the public cap count: the convertible units (what the canister has still to convert),
    /// less the floats the module itself holds. Reserve units are never converted, so they neither stress nor fill the cap.
    function _stressUnits() private view returns (uint256) {
        uint256 c = netUnits();
        uint256 f = floatUnits;
        return c > f ? c - f : 0;
    }

    // ---- the canister's reads (IFloorModule) ---------------------------------------------------------------------------------

    /// @notice The keeper's floor as the canister and the keeper read it: the accepted price, when it was posted, how long it lives
    /// (on both clocks here; `fresh()` is the authority).
    function floor() external view returns (uint256 priceWei, uint64 postedAt_, uint64 ttl) {
        return (accepted, postedAt, uint64(FLOOR_TTL));
    }

    /// @notice Net convertible ETH-backed units (1e18 = one item): minted for ETH (and by graduations and floats), less burned
    /// for ETH, less items arrived, less units rented items took over, less the reserve's units (§13.1). The canister buys one
    /// item per whole unit beyond `buffer()`.
    function netUnits() public view returns (uint256) {
        uint256 e = _liveUnits();
        uint256 r = reserveUnits;
        return e > r ? e - r : 0;
    }

    /// @dev The ETH-backed units whose backing is here: the wrapper's `ethBacked` less the unit in transit (its backing left as
    /// an advance, docs/v6/relay-conversion.md §2.2). Burns, tier prices and conversions divide by these.
    function _liveUnits() private view returns (uint256) {
        uint256 e = _w.ethBacked();
        uint256 t = transitUnits;
        return e > t ? e - t : 0;
    }

    /// @notice The wei backing the convertible units: what item costs are paid from (the revenue pot tops them up).
    function escrow() external view returns (uint256) { return backing; }

    /// @notice Units (1e18 scale) beyond one item before a purchase is due.
    function buffer() public view returns (uint256) { return (ONE * bufferBps) / BPS; }

    /// @notice The per-item cap: the accepted floor plus the conversion allowance while fresh; 0 when stale or while the keeper is
    /// paused (N-7: no conversion is priced by the keeper's word during an incident).
    function itemCap() public view returns (uint256) {
        return fresh() && !_keeperDown() ? Math.mulDiv(accepted, BPS + convBps, BPS) : 0;
    }

    /// @notice The most `itemArrived` pays now: the item cap (or a snapshot of it within `SNAPSHOT_TTL`), and never more than one
    /// unit's backing plus the reserve; 0 with no whole ETH-backed unit or no arrival room today.
    function maxItemCost() public view returns (uint256 max) {
        uint256 c = netUnits();
        if (c < ONE || !arrivalRoom()) return 0;
        max = Math.min(_capOrSnapshot(), Math.mulDiv(backing, ONE, c) + revenue + surplus);
    }

    function _capOrSnapshot() private view returns (uint256 max) {
        max = itemCap();
        if (_keeperDown()) return 0;
        if (snapCap > max && !Wait.elapsed(uint64(snapAt + SNAPSHOT_TTL),
            uint64(snapAtBlock + Wait.blocksFor(SNAPSHOT_TTL, parentBlockSeconds)))) max = snapCap;
    }

    /// @notice Public mints may not push ETH-backed units past this: the smaller of the constant cap and the book's depth
    /// (`depthItems` whole items), so net minting never runs ahead of what the canister could buy near the floor.
    function publicCap() public view returns (uint256) {
        return Math.min(maxPublicEthBacked, uint256(depthItems) * ONE);
    }

    // ---- quotes, for routers and pages ----------------------------------------------------------------------------------------

    /// @notice Minting now: the price per whole unit and the most ETH it takes (0, 0 when it cannot mint).
    function mintCapacity() public view returns (uint256 unitPrice, uint256 maxEthIn) {
        if (!fresh() || _keeperDown()) return (0, 0);
        unitPrice = mintPrice();
        (uint256 lvl,) = _level(mintLevel, mintAt, mintAtBlock, mintPerHour);
        uint256 e = _stressUnits();
        uint256 pc = publicCap();
        // the cap is in convertible units; a mint's units are convertible less the reserve share
        uint256 capRoom = e >= pc ? 0 : Math.mulDiv(pc - e, BPS, BPS - reserveBps);
        uint256 room = Math.min(Math.min(mintPerHour - lvl, capRoom), _gradRoom());
        maxEthIn = Math.mulDiv(room, unitPrice, ONE);
    }

    /// @notice Burning now: the lower of the two tiers' prices per whole unit (a burn spans them; `quoteBurn` is exact) and the most
    /// units it takes: every ETH-backed unit, within the hourly cap (0, 0 when it cannot burn). Works while the keeper is paused and
    /// in the stale fallback too.
    function burnCapacity() public view returns (uint256 unitPrice, uint256 maxUnits) {
        if (!burnsOpen()) return (0, 0);
        (uint256 pr, uint256 pc) = _tierPrices();
        uint256 e = _liveUnits();
        uint256 r = reserveUnits;
        if (r == 0) unitPrice = pc;
        else if (e == r) unitPrice = pr;
        else unitPrice = Math.min(pr, pc);
        (uint256 lvl,) = _level(burnLevel, burnAt, burnAtBlock, burnPerHour);
        maxUnits = Math.min(burnPerHour - lvl, e);
        if (unitPrice == 0) maxUnits = 0;
    }

    function quoteMint(uint256 ethIn) external view returns (uint256 units) {
        (uint256 unitPrice, uint256 maxIn) = mintCapacity();
        if (unitPrice == 0 || ethIn > maxIn) return 0;
        units = Math.mulDiv(ethIn, ONE, unitPrice);
    }

    /// @notice What `burn(units)` pays now, exactly (0 when it would not be taken).
    function quoteBurn(uint256 units) external view returns (uint256 eth) {
        (uint256 unitPrice, uint256 maxUnits) = burnCapacity();
        if (units > maxUnits || unitPrice == 0) return 0;
        (uint256 pr, uint256 pc) = _tierPrices();
        uint256 rU = Math.min(units, reserveUnits);
        eth = Math.mulDiv(rU, pr, ONE) + Math.mulDiv(units - rU, pc, ONE);
    }

    /// @notice Whether a rental can be taken now: a fresh floor, an eligibility list, an ETH-backed unit for it, and room.
    function rentalRoom() public view returns (bool) {
        return fresh() && !_keeperDown() && rentalRoot != bytes32(0) && netUnits() >= ONE
            && activeRentals + leavingRentals < maxRentals && arrivalRoom();
    }

    // ---- the web and indexer adapters' reads (docs/v5/ux-and-reserves.md §4; `floor()` answers their `floor() → uint256` too,
    // its first word being the price) ----
    function wrapped() external view returns (address) { return address(wrapper); }
    /// @notice The wider of the two sides now, bps, for display; quotes (`quoteMint`, `quoteBurn`) are exact.
    function spreadBps() external view returns (uint256) {
        (uint256 sMint, uint256 sBurn) = spreads();
        return Math.max(sMint, sBurn);
    }
    function ethBackedUnits() external view returns (uint256) { return _w.ethBacked(); }
    /// @notice Every wei backing ETH-backed units: the convertible pot and the reserve's basis (`surplus` is the protocol's, apart).
    function escrowBalance() external view returns (uint256) { return backing + reserve; }
    /// @notice The `i`-th most recent fill kept (0 = the newest), price and time; reverts past what is kept.
    function fill(uint256 i) external view returns (Fill memory) {
        uint256 n = fillCount;
        if (i >= Math.min(n, FILLS)) revert BadInput();
        return _fills[(n - 1 - i) % FILLS];
    }

    function grad(address coin) external view returns (Grad memory) { return _grads[coin]; }
    function rental(uint256 id) external view returns (Rental memory) { return _rentals[id]; }
    function rentalCount() external view returns (uint256) { return _rentals.length; }
    /// @notice What a rental has earned from revenue so far and not yet been credited.
    function earned(uint256 id) external view returns (uint256) {
        Rental storage r = _rentals[id];
        return r.status == RentStatus.Lent ? (accPerRental - r.accAt) / ACC : 0;
    }

    // ---- clocks ---------------------------------------------------------------------------------------------------------------

    /// @dev The smaller of the two clocks' elapsed time: what a refill or a decay may count (a sequencer running ahead gains nothing).
    function _slowElapsed(uint64 at, uint64 atBlock) private view returns (uint256) {
        uint256 dt = block.timestamp > at ? block.timestamp - at : 0;
        uint256 db = block.number > atBlock ? (block.number - atBlock) * parentBlockSeconds : 0;
        return Math.min(dt, db);
    }

    /// @dev The larger: how stale a post looks (a stalled parent chain or a running sequencer both widen the spread).
    function _fastElapsed(uint64 at, uint64 atBlock) private view returns (uint256) {
        uint256 dt = block.timestamp > at ? block.timestamp - at : 0;
        uint256 db = block.number > atBlock ? (block.number - atBlock) * parentBlockSeconds : 0;
        return Math.max(dt, db);
    }

    /// @dev A leaky bucket's level now: it drains `cap` per hour of the slower clock.
    function _level(uint256 level, uint64 at, uint64 atBlock, uint256 cap) private view returns (uint256 lvl, uint256 leak) {
        leak = Math.mulDiv(cap, Math.min(_slowElapsed(at, atBlock), HOUR), HOUR);
        lvl = level > leak ? level - leak : 0;
    }

    function _send(address to, uint256 amount) private {
        if (amount == 0) return;
        (bool ok,) = to.call{value: amount}("");
        if (!ok) revert TransferFailed();
    }
}
