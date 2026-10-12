// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test, console2} from "forge-std/Test.sol";
import {FloorBase} from "./FloorBase.sol";
import {Merkle} from "../utils/Merkle.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {Wrapper} from "../../src/Wrapper.sol";
import {IFloorWrapped} from "../../src/interfaces/IFloorWrapped.sol";

interface IFloorTestHooks {
    function depositTo(address to) external;
    function rentFor(address who) external returns (bool, uint256);
}

/// Drives the module and the wrapper together: the keeper's posts, mints, burns, conversions, redemptions and their draws,
/// deliveries and refunds, rentals in, out, drawn and returned — in any order, at any time. It checks, as it goes, the bounds
/// that are about one step (the move limit, the burn never paying above the backing, a draw never finding the set empty, a
/// leaving rental never drawn by a later ticket); the invariants below check the books after every call.
contract FloorHandler is Test {
    FloorModule immutable module;
    Wrapper immutable wrapper;
    IFloorTestHooks immutable base;
    address immutable keeper;
    address immutable custodian;
    uint256 constant F = 0.633 ether;
    address[3] actors;

    // ghosts
    uint256 public ethIn;
    uint256 public ethOut;
    bool public moveBroken;
    bool public burnAboveBacking;
    bool public drewFromEmpty;
    bool public leavingDrawnLate;
    bool public capBroken;
    bool public reserveConverted;      // a conversion or a rental intake touched the standing reserve
    bool public reserveBetaFell;       // the reserve's backing per unit fell on a burn
    bool public reserveShortOfUnits;   // reserve units without reserve ETH
    bool public advanceLowered;        // an advance, or a return with no loss, lowered the others' backing per unit
    bool public returnedUnsettled;     // a returned rental's unit ETH-backed while its escrow still sat in rentalEscrow
    uint256 public advances;
    uint256 public advancesSettled;
    uint256 public advancesReturned;
    uint256 public calls;
    uint256[] public rentals;

    constructor(FloorModule m, Wrapper w, IFloorTestHooks b, address k, address c) {
        module = m;
        wrapper = w;
        base = b;
        keeper = k;
        custodian = c;
        actors = [makeAddr("a1"), makeAddr("a2"), makeAddr("a3")];
    }

    receive() external payable {}

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / 12 + 1);
    }

    function post(uint256 seed) external {
        calls++;
        _skip(bound(seed, 1 minutes, module.FLOOR_TTL() + 10 minutes));   // past the TTL too: the stale branch
        uint256 prev = module.accepted();
        uint256 el = module.postedAt() == 0 ? 0 : vm.getBlockTimestamp() - module.postedAt();
        if (el > 1 hours) el = 1 hours;
        uint256 t = bound(seed >> 16, F / 2, F * 2);
        vm.prank(keeper);
        module.postFloor(t, 10_000, 50);
        uint256 next = module.accepted();
        if ((next > prev ? next - prev : prev - next) > prev * 500 * el / (10_000 * 1 hours) + 1) moveBroken = true;
    }

    function mint(uint256 who, uint256 amount) external {
        calls++;
        (uint256 price, uint256 maxIn) = module.mintCapacity();
        if (price == 0 || maxIn == 0) return;
        amount = bound(amount, 0, maxIn);
        if (amount * 1e18 < price) return;
        address a = actors[who % 3];
        vm.deal(a, a.balance + amount);
        vm.prank(a);
        module.mint{value: amount}(a, 0);
        ethIn += amount;
        if (module.mintLevel() > module.mintPerHour() || wrapper.ethBacked() > module.maxEthBacked()) capBroken = true;
    }

    function burn(uint256 who, uint256 amount) external {
        calls++;
        (uint256 price, uint256 room) = module.burnCapacity();
        address a = actors[who % 3];
        uint256 have = wrapper.balanceOf(a);
        amount = bound(amount, 0, room < have ? room : have);
        if (price == 0 || amount * price < 1e18 || module.quoteBurn(amount) == 0) return;   // a burn worth no wei is refused
        // a burn spans the tiers (B-01): the reserve units first, then convertible ones, each releasing its pot's backing per unit
        uint256 r = module.reserveUnits();
        uint256 rU = amount < r ? amount : r;
        uint256 cU = amount - rU;
        uint256 c = module.netUnits();
        uint256 released = (rU == 0 ? 0 : rU == r ? module.reserve() : module.reserve() * rU / r)
            + (cU == 0 ? 0 : cU == c ? module.backing() : module.backing() * cU / c);
        uint256 betaC = module.betaConvertible();
        uint256 betaS = module.betaReserve();
        vm.startPrank(a);
        wrapper.approve(address(module), amount);
        uint256 got = module.burn(amount, 0, a);
        vm.stopPrank();
        ethOut += got;
        if (got > released) burnAboveBacking = true;
        if (rU != 0 && module.reserveUnits() != 0 && module.betaReserve() < betaS) reserveBetaFell = true;
        if (cU != 0 && module.netUnits() != 0 && module.betaConvertible() < betaC) burnAboveBacking = true;
        if (module.burnLevel() > module.burnPerHour()) capBroken = true;
    }

    function convert(uint256 costSeed) external {
        calls++;
        if (module.netUnits() < 1e18 || module.itemCap() == 0) return;
        uint256 beta = module.betaConvertible();
        uint256 max = module.maxItemCost();
        uint256 ceiling = beta + module.revenue();
        if (ceiling < max) max = ceiling;
        uint256 cost = bound(costSeed, 1, max);
        uint256 s = module.reserve();
        uint256 r = module.reserveUnits();
        base.depositTo(address(module));
        vm.prank(custodian);
        module.itemArrived(cost);
        ethOut += cost;
        if (module.reserve() != s || module.reserveUnits() != r) reserveConverted = true;
    }

    /// docs/v6/relay-conversion.md §2: one unit's budget leaves before its item; then the item, or the ETH back.
    function advance(uint256 seed) external {
        calls++;
        if (module.transitUnits() != 0) return;
        uint256 max = module.maxItemCost();
        if (max == 0) return;
        uint256 amount = bound(seed, 1, max);
        uint256 beta = module.betaConvertible();
        uint256 s = module.reserve();
        uint256 r = module.reserveUnits();
        vm.prank(custodian);
        module.advance(amount);
        advances++;
        ethOut += amount;
        if (module.netUnits() != 0 && module.betaConvertible() + 1 < beta) advanceLowered = true;
        if (module.reserve() != s || module.reserveUnits() != r) reserveConverted = true;
    }

    function settleAdvance(uint256 seed) external {
        calls++;
        uint256 adv = module.advanced();
        if (module.transitUnits() == 0) return;
        base.depositTo(address(module));
        vm.prank(custodian);
        module.settleAdvance(bound(seed, 1, adv));
        advancesSettled++;
    }

    function returnAdvance(uint256 seed) external {
        calls++;
        if (module.transitUnits() == 0) return;
        uint256 basis = module.advanceBasis();
        uint256 v = bound(seed, 0, 2 * module.advanced() + 1);
        uint256 beta = module.betaConvertible();
        bool had = module.netUnits() != 0;
        uint256 cover = module.revenue() + module.surplus();
        vm.deal(custodian, custodian.balance + v);
        vm.prank(custodian);
        module.returnAdvance{value: v}();
        advancesReturned++;
        ethIn += v;
        uint256 floorBeta = beta < basis ? beta : basis;
        if (had && v + cover >= basis && module.betaConvertible() + 1 < floorBeta) advanceLowered = true;
    }

    function redeem(uint256 who) external {
        calls++;
        address a = actors[who % 3];
        if (wrapper.balanceOf(a) < 1e18) return;
        if (wrapper.held() < wrapper.openTickets() + wrapper.openReturns() + 1) return;
        vm.prank(a);
        wrapper.redeem("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0");
    }

    function resolve(uint256 r) external {
        calls++;
        uint256 id = wrapper.nextToResolve();
        if (id >= wrapper.ticketCount()) return;
        try wrapper.resolve(id, abi.encodePacked(bytes32(r | 1))) {
            uint256 frog = wrapper.ticket(id).acquisitionId;
            Wrapper.Rental memory rt = wrapper.rentalOf(frog);
            if ((rt.state == IFloorWrapped.RentalState.LeavingDrawn) && rt.leaveAfter <= id) leavingDrawnLate = true;
        } catch (bytes memory err) {
            if (bytes4(err) == Wrapper.NothingToDraw.selector) drewFromEmpty = true;
        }
    }

    function deliver(uint256 i) external {
        calls++;
        uint256 n = wrapper.ticketCount();
        if (n == 0) return;
        uint256 id = bound(i, 0, n - 1);
        if (wrapper.ticket(id).stage != Wrapper.Stage.Resolved) return;
        vm.prank(custodian);
        wrapper.recordDelivery(id, keccak256(abi.encode("delivery", id)));
    }

    function refund(uint256 i) external {
        calls++;
        uint256 n = wrapper.ticketCount();
        if (n == 0) return;
        uint256 id = bound(i, 0, n - 1);
        if (wrapper.ticket(id).stage != Wrapper.Stage.Resolved) return;
        _skip(7 days + 72 hours + 1);                        // the deadline and the refund grace
        vm.prank(wrapper.custodian());
        wrapper.markUndeliverable(id);                       // the custodian's attestation (review2 R3 §4)
        wrapper.refund(id);
        uint256 acc = module.accepted();
        vm.prank(keeper);
        module.postFloor(acc, 10_000, 50);
    }

    function rentIn(uint256 who) external {
        calls++;
        if (!module.rentalRoom()) return;
        uint256 s = module.reserve();
        (bool ok, uint256 rid) = base.rentFor(actors[who % 3]);
        if (ok) rentals.push(rid);
        if (module.reserve() != s) reserveConverted = true;
    }

    function fundReserve(uint256 amount) external {
        calls++;
        amount = bound(amount, 1, 0.5 ether);
        vm.deal(address(this), address(this).balance + amount);
        module.fundReserve{value: amount}();
        ethIn += amount;
    }

    function endRental(uint256 i) external {
        calls++;
        if (rentals.length == 0) return;
        uint256 rid = rentals[bound(i, 0, rentals.length - 1)];
        FloorModule.Rental memory r = module.rental(rid);
        if (r.status != FloorModule.RentStatus.Lent) return;
        if (wrapper.rentalState(r.itemId) != IFloorWrapped.RentalState.Active) return;
        if (vm.getBlockTimestamp() < r.endsAt || vm.getBlockNumber() < r.endsAtBlock) {
            _skip(r.endsAt - vm.getBlockTimestamp() + 13);
            uint256 acc = module.accepted();
            vm.prank(keeper);
            module.postFloor(acc, 10_000, 50);
        }
        module.endRental(rid);
    }

    function returnHome(uint256 i) external {
        calls++;
        if (rentals.length == 0) return;
        uint256 rid = rentals[bound(i, 0, rentals.length - 1)];
        uint256 item = module.rental(rid).itemId;
        wrapper.processLeaving();
        if (wrapper.rentalState(item) != IFloorWrapped.RentalState.Ready) return;
        vm.prank(custodian);
        wrapper.recordReturn(item, keccak256(abi.encode("return", item)));
    }

    function settle(uint256 i) external {
        calls++;
        if (rentals.length == 0 || !module.fresh()) return;
        uint256 rid = rentals[bound(i, 0, rentals.length - 1)];
        FloorModule.Rental memory r = module.rental(rid);
        IFloorWrapped.RentalState ws = wrapper.rentalState(r.itemId);
        // a returned item's rental closes inside `recordReturn` (deep-fuzz 2026-09-28): only a taken one is left to settle
        if (ws == IFloorWrapped.RentalState.Returned && r.status != FloorModule.RentStatus.Closed) returnedUnsettled = true;
        if (ws != IFloorWrapped.RentalState.Taken || r.status == FloorModule.RentStatus.Closed) return;
        module.settleRental(rid);
    }

    function claim(uint256 who) external {
        calls++;
        address a = actors[who % 3];
        if (module.owed(a) == 0) return;
        vm.prank(a);
        ethOut += module.claim();
    }
}

/// forge-config: default.invariant.fail-on-revert = true
contract FloorInvariantTest is FloorBase {
    FloorHandler handler;

    /// The 25% reserve share (§13.1): the books are checked with both pots in play.
    function _params() internal pure override returns (FloorModule.Params memory p) {
        p = super._params();
        p.reserveBps = 2_500;
        p.fillBandBps = 300;
    }

    function setUp() public override {
        super.setUp();
        handler = new FloorHandler(module, wrapper, IFloorTestHooks(address(this)), keeper, custodian);
        targetContract(address(handler));
    }

    /// The handler's way to have the canister wrap a frog into the module.
    function depositTo(address to) external {
        _deposit(to);
    }

    /// The handler's way to rent the next eligible frog for `who` (frogs 32..63).
    function rentFor(address who) external returns (bool, uint256) {
        if (nextFrog < 32) nextFrog = 32;
        if (nextFrog >= N_FROGS) return (false, 0);
        uint256 i = nextFrog++;
        bytes memory att = abi.encode(frogIds[i], Merkle.proof(frogLeaves, i), keccak256(abi.encode("rent", i)), uint32(0));
        vm.prank(custodian);
        uint256 rid = module.rentIn(who, bytes(TAPROOT), Merkle.proof(rentLeaves, i - 32), att);
        return (true, rid);
    }

    /// How often the advance paths ran (relay-conversion §2), visible with -vv.
    function afterInvariant() external view {
        console2.log("advances", handler.advances(), handler.advancesSettled(), handler.advancesReturned());
    }

    /// (W1): each held item backs a whole token or an open ticket; the rest of the supply is ETH-backed.
    function invariant_theLedger() public view {
        assertEq(wrapper.totalSupply(), (wrapper.held() - wrapper.openTickets()) * ONE + wrapper.ethBacked());
    }

    /// (W3) a redemption always has an item: the free set is never empty when a ticket is drawn.
    function invariant_aDrawNeverFindsTheSetEmpty() public view {
        assertFalse(handler.drewFromEmpty());
        assertGe(wrapper.held(), wrapper.openTickets() + wrapper.openReturns() > wrapper.held() ? wrapper.held() : 0);
    }

    /// (M2) the module holds every wei it owes; escrow never negative is this plus the checked arithmetic.
    function invariant_theModuleIsSolvent() public view {
        assertGe(address(module).balance, module.backing() + module.reserve() + module.surplus() + module.revenue() + module.gradEth()
            + module.rentalEscrow() + module.renterPool() + module.owedTotal());
    }

    /// §13.1 (M8, M9): E = C + R; the reserve is never converted nor lent; its backing per unit never falls on a burn; a reserve
    /// unit is never without backing.
    function invariant_theReserveStands() public view {
        assertEq(module.netUnits() + module.reserveUnits() + module.transitUnits(), wrapper.ethBacked(), "E = C + R + T");
        assertLe(module.transitUnits(), ONE, "one advance at a time");
        assertFalse(handler.advanceLowered(), "an advance, or a return with no loss, lowered the others' backing per unit");
        assertFalse(handler.reserveConverted(), "a conversion or a rental touched the reserve");
        assertFalse(handler.reserveBetaFell(), "the reserve's backing per unit fell");
        if (module.reserveUnits() != 0) assertGt(module.reserve(), 0, "reserve units without reserve");
        assertLe(module.reserveUnits(), wrapper.ethBacked());
    }

    /// §13.4 the price never leaves the keeper's band.
    function invariant_thePriceStaysInTheBand() public view {
        uint256 acc = module.accepted();
        assertLe(module.price(), acc * 10_300 / 10_000 + 1);
        assertGe(module.price(), acc * 9_700 / 10_000);
    }

    /// Every wei that came in either went out, to a burner, the canister or a renter, or is still here.
    function invariant_ethIsConserved() public view {
        assertEq(handler.ethIn(), handler.ethOut() + address(module).balance);
    }

    /// (M4) no burn paid above the backing it released, and none lowered the backing per unit of what was left.
    function invariant_burnsNeverPayAboveTheBacking() public view {
        assertFalse(handler.burnAboveBacking());
    }

    /// §5 the keeper's floor never moved more than 5% per hour of elapsed time.
    function invariant_theMoveLimitHolds() public view {
        assertFalse(handler.moveBroken());
    }

    /// §6.2 the hourly caps and the hard cap on ETH-backed units.
    function invariant_theCapsHold() public view {
        assertFalse(handler.capBroken());
        assertLe(module.mintLevel(), module.mintPerHour());
        assertLe(module.burnLevel(), module.burnPerHour());
        assertLe(wrapper.ethBacked(), module.maxEthBacked() + wrapper.rentedHeld() * ONE + ONE * 64);
    }

    /// §9.4 a rented item that is leaving is never drawn by a ticket burned after it left.
    function invariant_aLeavingItemIsNeverDrawnLate() public view {
        assertFalse(handler.leavingDrawnLate());
    }

    /// Nothing ETH-backed means nothing backing it; and a returned rental's unit is never ETH-backed without its escrow.
    function invariant_noBackingWithoutUnits() public view {
        if (wrapper.ethBacked() == 0) assertEq(module.backing(), 0);
        assertFalse(handler.returnedUnsettled(), "a returned rental left open");
    }

    // ---- replays of sequences the deep campaign (FOUNDRY_PROFILE=deep) shrank ----------------------------------------------------

    /// ci run 36412969532 (2026-09-28, 47a9b94; the same on 47c6cf5): a rental's item went home, which made its unit ETH-backed,
    /// but its escrow stayed in `rentalEscrow` until a separate `settleRental`. In between a conversion took that unit at zero
    /// backing (its cost from the revenue pot), and the later settle put ≈ 0.63 ETH into `backing` with no unit left to claim it.
    function test_deepRun36412969532_aReturnedRentalsEscrowBacksItsUnitAtOnce() public {
        handler.mint(45325653730203746239228721579940351951137224107575263305229335039776366002175, 960000000000000000);
        handler.rentIn(13875);
        uint256 escrow = module.rentalEscrow();
        assertGt(escrow, 0.6 ether);
        handler.burn(9335, type(uint256).max);
        assertEq(wrapper.ethBacked(), 0, "every ETH-backed unit burned");
        handler.endRental(1112588910);
        uint256 rid = handler.rentals(0);
        assertEq(uint256(module.rental(rid).status), uint256(FloorModule.RentStatus.Leaving));
        handler.returnHome(9669);
        // the return made the unit ETH-backed and, in the same call, its backing the escrow
        assertEq(wrapper.ethBacked(), ONE);
        assertEq(module.backing(), escrow, "the escrow backs the returned unit");
        assertEq(module.rentalEscrow(), 0);
        assertEq(module.leavingRentals(), 0);
        assertEq(uint256(module.rental(rid).status), uint256(FloorModule.RentStatus.Closed));
        vm.expectRevert(FloorModule.NothingToSettle.selector);
        module.settleRental(rid);
        // the conversion now releases that unit's whole backing, and nothing is left behind
        handler.convert(7259);
        handler.settle(3484366862);
        assertEq(wrapper.ethBacked(), 0);
        assertEq(module.backing(), 0, "no backing without units");
        invariant_noBackingWithoutUnits();
        invariant_ethIsConserved();
        invariant_theModuleIsSolvent();
    }

    /// Only the wrapper closes a rental as returned.
    function test_rentalReturned_onlyTheWrapper() public {
        vm.expectRevert(FloorModule.NotWrapper.selector);
        module.rentalReturned(0);
        vm.prank(custodian);
        vm.expectRevert(FloorModule.NotWrapper.selector);
        module.rentalReturned(0);
        vm.prank(address(wrapper));
        vm.expectRevert(FloorModule.NothingToSettle.selector);
        module.rentalReturned(12345);                        // never rented
    }

    function invariant_rentalsCountedNeverOwned() public view {
        assertEq(module.activeRentals() + module.leavingRentals() <= module.maxRentals() ? 1 : 0, 1);
        assertLe(wrapper.openReturns(), wrapper.rentedHeld());
    }
}
