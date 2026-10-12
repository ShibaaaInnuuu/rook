// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {FloorBase} from "../../floor/FloorBase.sol";
import {FloorModule} from "../../../src/floor/FloorModule.sol";
import {Wrapper} from "../../../src/Wrapper.sol";
import {ListingRegistry} from "../../../src/ListingRegistry.sol";

/// Drives the module with honest and hostile moves: real mints and dust mints (B-01), burns of any size, fee money into the
/// surplus, keeper posts at any price and confidence (including none, E-02a; and a 1-wei target at full confidence, review2 N-1a),
/// the keeper paused and unpaused (review2 N-1b), a mint → pause → burn round trip (R3 check A3-M1), and time up to several days
/// at once.
contract LivenessHandler is Test {
    FloorModule public module;
    Wrapper public wrapper;
    address public keeper;
    ListingRegistry public registry;
    address public owner;
    address[3] public actors;
    /// A holder of ETH-backed wX, with the module fresh or escaped and room under the hourly cap, whose burn of a whole unit (or
    /// all it holds) reverted: the liveness failure B-01 was.
    uint256 public stuck;
    /// A burn that paid more per unit than its tier's backing (M4).
    bool public overpaid;
    uint256 public exits;
    uint256 public escapedExits;
    uint256 public pausedExits;
    uint256 public tinyPosts;
    /// A3-M1: a unit minted, the keeper paused (no post between), and burned inside the stale window paid more than it cost.
    bool public pauseProfit;
    /// A paused burn (not escaped) paid more per unit than the pause floor less the base spread.
    bool public abovePauseFloor;
    uint256 public roundTrips;
    /// R4 check A4-L1: round trips with a keeper post between the mint and the pause.
    uint256 public postsBetween;
    uint256 public tripStuck;
    address public tripper = makeAddr("tripper");

    constructor(FloorModule m, Wrapper w, address k, ListingRegistry r, address o) {
        module = m;
        wrapper = w;
        keeper = k;
        registry = r;
        owner = o;
        actors = [makeAddr("h1"), makeAddr("h2"), makeAddr("h3")];
    }

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / 12 + 1);
    }

    function post(uint256 seed, bool good) external {
        _skip(bound(seed, 1 minutes, module.FLOOR_TTL() + 10 minutes));   // past the TTL too: the stale branch
        if (registry.keeperPaused()) return;
        uint256 a = module.accepted();
        uint256 t = bound(seed >> 16, a / 2 + 1, a * 2);
        vm.prank(keeper);
        module.postFloor(t, good ? 10_000 : 0, 50);
    }

    /// N-1a: a compromised keeper keeps the module fresh with a 1-wei target.
    function postTiny(uint256 seed) external {
        _skip(bound(seed, 1 minutes, 55 minutes));
        if (registry.keeperPaused()) return;
        vm.prank(keeper);
        module.postFloor(1, 10_000, 50);
        tinyPosts++;
    }

    /// N-1b: the guardian's (here the owner's) incident switch, and its release.
    function pause(bool on) external {
        vm.prank(owner);
        if (on) registry.pauseKeeper();
        else registry.unpauseKeeper();
    }

    /// A3-M1: an outsider mints at whatever the keeper left, a delay, maybe a keeper post (R4 check A4-L1: any target from half to
    /// twice the floor, any confidence), a pause, a wait inside the stale window, and a burn of those units. Never a profit. The
    /// pause is then lifted again.
    function roundTrip(uint256 amount, uint256 delay, uint256 hold) external {
        if (registry.keeperPaused()) return;
        (uint256 mp, uint256 maxIn) = module.mintCapacity();
        if (mp == 0 || maxIn < 1e12) return;
        uint256 paid = bound(amount, 1e12, maxIn);
        vm.deal(tripper, tripper.balance + paid);
        vm.prank(tripper);
        uint256 units;
        try module.mint{value: paid}(tripper, 0) returns (uint256 u) { units = u; } catch { return; }
        _skip(bound(delay, 0, 3 hours));
        if ((amount >> 200) % 2 == 1) {
            uint256 a = module.accepted();
            vm.prank(keeper);
            module.postFloor(bound(delay >> 64, a / 2 + 1, a * 2), uint16(bound(hold >> 64, 0, 10_000)), 50);
            _skip(bound(delay >> 128, 0, 50 minutes));
            postsBetween++;
        }
        vm.prank(owner);
        registry.pauseKeeper();
        _skip(bound(hold, 0, 3 days));
        (, uint256 room) = module.burnCapacity();
        if (!module.escaped() && room >= units) {
            vm.startPrank(tripper);
            wrapper.approve(address(module), units);
            try module.burn(units, 0, tripper) returns (uint256 got) {
                roundTrips++;
                if (got > paid) pauseProfit = true;
            } catch {
                tripStuck++;
            }
            vm.stopPrank();
        }
        vm.prank(owner);
        registry.unpauseKeeper();
    }

    function mint(uint256 who, uint256 amount) external {
        (uint256 price, uint256 maxIn) = module.mintCapacity();
        if (price == 0 || maxIn == 0) return;
        amount = bound(amount, 1, maxIn);
        address a = actors[who % 3];
        vm.deal(a, a.balance + amount);
        vm.prank(a);
        try module.mint{value: amount}(a, 0) {} catch {}
    }

    /// B-01's grief: a few wei, re-arming a dust reserve tier.
    function dustMint(uint256 who, uint256 wei_) external {
        if (!module.fresh()) return;
        wei_ = bound(wei_, 1, 16);
        address a = actors[who % 3];
        vm.deal(a, a.balance + wei_);
        vm.prank(a);
        try module.mint{value: wei_}(a, 0) {} catch {}
    }

    function fundSurplus(uint256 amount) external {
        amount = bound(amount, 1, 2 ether);
        vm.deal(address(this), amount);
        module.fundReserve{value: amount}();
    }

    function wait(uint256 secs) external {
        _skip(bound(secs, 1 minutes, 4 days));
    }

    /// A holder exits: a whole unit, or all it holds if less, within the hourly cap. It must go through whenever the module is
    /// fresh or escaped and the amount is worth at least a wei.
    function exit(uint256 who) external {
        address a = actors[who % 3];
        uint256 bal = wrapper.balanceOf(a);
        if (bal == 0) return;
        if (!module.burnsOpen()) return;
        (uint256 price, uint256 room) = module.burnCapacity();
        uint256 u = bal < 1e18 ? bal : 1e18;
        if (room < u) return;                                   // the hourly cap is full: an orderly queue, not a freeze
        if (module.quoteBurn(u) == 0) return;                    // a dust amount worth nothing
        uint256 r = module.reserveUnits();
        uint256 rU = u < r ? u : r;
        uint256 c = module.netUnits();
        uint256 maxPay = (rU == 0 ? 0 : rU == r ? module.reserve() : module.reserve() * rU / r)
            + (u - rU == 0 ? 0 : u - rU == c ? module.backing() : module.backing() * (u - rU) / c);
        bool esc = module.escaped();
        bool paused = registry.keeperPaused();
        uint256 pf = module.pauseFloor();
        vm.startPrank(a);
        wrapper.approve(address(module), u);
        try module.burn(u, 0, a) returns (uint256 got) {
            exits++;
            if (esc) escapedExits++;
            if (paused) pausedExits++;
            if (got > maxPay) overpaid = true;
            if (paused && !esc && got > u * pf / 1e18 * (10_000 - module.baseSpreadBps()) / 10_000 + 2) abovePauseFloor = true;
        } catch {
            stuck++;
        }
        vm.stopPrank();
        price;
    }

    receive() external payable {}
}

/// Invariants (docs/audit/2026-09-26/fixes-B-C-keeper-draws.md): a holder can always burn when backing exists (B-01), and after
/// the stale window nothing is locked (B-04); burns never pay above the backing, and the books hold.
contract BurnLivenessInvariantTest is FloorBase {
    LivenessHandler handler;

    function _params() internal pure override returns (FloorModule.Params memory p) {
        p = super._params();
        p.reserveBps = 2_500;
        p.maxItemsPerDay = 3;
    }

    function setUp() public override {
        super.setUp();
        handler = new LivenessHandler(module, wrapper, keeper, registry, owner);
        targetContract(address(handler));
        bytes4[] memory s = new bytes4[](9);
        s[0] = LivenessHandler.post.selector;
        s[1] = LivenessHandler.mint.selector;
        s[2] = LivenessHandler.dustMint.selector;
        s[3] = LivenessHandler.fundSurplus.selector;
        s[4] = LivenessHandler.wait.selector;
        s[5] = LivenessHandler.exit.selector;
        s[6] = LivenessHandler.postTiny.selector;
        s[7] = LivenessHandler.pause.selector;
        s[8] = LivenessHandler.roundTrip.selector;
        targetSelector(FuzzSelector({addr: address(handler), selectors: s}));
    }

    /// B-01, N-1: while fresh, paused or escaped and under the hourly cap, a holder of ETH-backed wX can always burn, whatever the
    /// keeper posted.
    function invariant_aHolderCanAlwaysBurnWhenBackingExists() public view {
        assertEq(handler.stuck(), 0, "a holder's burn reverted with backing, room and a fresh or escaped module");
        assertEq(handler.tripStuck(), 0, "a paused burn reverted");
    }

    /// R3 check A3-M1: a pause never pays a mint → burn round trip, and a paused burn never pays above the pause floor.
    function invariant_aPauseNeverPaysARoundTrip() public view {
        assertFalse(handler.pauseProfit(), "a mint, a pause, a burn: a profit");
        assertFalse(handler.abovePauseFloor(), "a paused burn above the pause floor");
    }

    /// B-04, N-1b: once escaped, and at once while the keeper is paused, every ETH-backed unit has an exit at a non-zero price
    /// (within the hourly cap).
    function invariant_noLockupAfterTheStaleWindow() public view {
        if (!(module.exitAtBacking() || registry.keeperPaused()) || wrapper.ethBacked() == 0) return;
        (uint256 price, uint256 room) = module.burnCapacity();
        assertGt(price, 0, "escaped: a price with no floor");
        if (module.burnLevel() + 1e18 <= module.burnPerHour()) assertGt(room, 0, "escaped: room");
    }

    /// M4: never more per unit than the tier's backing.
    function invariant_burnsNeverPayAboveTheBacking() public view {
        assertFalse(handler.overpaid());
    }

    function invariant_theBooksHold() public view {
        _books();
    }

    /// Not vacuous: driven by hand, the handler really exits through a dust tier while fresh, and again once escaped.
    function test_theHandlerReachesAFreshAndAnEscapedExit() public {
        handler.mint(0, 3 ether);
        handler.dustMint(1, 4);
        handler.exit(0);
        assertEq(handler.exits(), 1);
        handler.wait(4 days);
        handler.wait(4 days);
        assertTrue(module.escaped());
        handler.exit(0);
        assertEq(handler.escapedExits(), 1);
        assertEq(handler.stuck(), 0);
        _books();
    }

    /// Not vacuous for N-1: a 1-wei poster keeps the module fresh and the exit goes through; then a pause, a day of it, an exit.
    function test_theHandlerExitsUnderATinyTargetAndThroughAPause() public {
        handler.mint(0, 3 ether);
        handler.postTiny(30 minutes);
        handler.postTiny(30 minutes);
        assertTrue(module.fresh());
        assertEq(module.target(), 1);
        handler.exit(0);
        assertEq(handler.exits(), 1);
        handler.pause(true);
        handler.wait(1 days);
        assertFalse(module.fresh());
        handler.exit(0);
        assertEq(handler.pausedExits(), 1);
        assertEq(handler.stuck(), 0);
        _books();
    }

    /// Not vacuous for A4-L1: a round trip with a post between the mint and the pause (the target back up) loses too.
    function test_theHandlerMakesAPausedRoundTripWithAPostBetween() public {
        handler.mint(0, 3 ether);
        handler.post(30 minutes, true);
        uint256 odd = uint256(1) << 200;
        handler.roundTrip(odd | 1 ether, (uint256(module.accepted() * 3 / 2) << 64) | 30 minutes, (uint256(10_000) << 64) | 1 days);
        assertEq(handler.roundTrips(), 1);
        assertEq(handler.postsBetween(), 1);
        assertFalse(handler.pauseProfit());
        assertEq(handler.tripStuck(), 0);
        _books();
    }

    /// Not vacuous for A3-M1: the round trip action really mints, pauses, burns and unpauses, and loses.
    function test_theHandlerMakesAPausedRoundTrip() public {
        handler.mint(0, 3 ether);
        handler.post(30 minutes, true);
        handler.roundTrip(1 ether, 1 hours, 1 days);
        assertEq(handler.roundTrips(), 1);
        assertFalse(handler.pauseProfit());
        assertFalse(registry.keeperPaused(), "lifted again");
        assertEq(handler.tripStuck(), 0);
        _books();
    }
}
