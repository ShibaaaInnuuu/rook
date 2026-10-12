// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Base} from "../utils/Base.sol";
import {MockSeeder} from "../utils/CurveBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {PosterSource} from "../../src/floor/PosterSource.sol";
import {WakeDesk} from "../../src/floor/WakeDesk.sol";
import {EthNftWrapper, IListingPause} from "../../src/nft/EthNftWrapper.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {MintDesk} from "../../src/curve/MintDesk.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveFactory} from "../../src/curve/CurveFactory.sol";
import {CollectionDirectory} from "../../src/curve/CollectionDirectory.sol";
import {IKeeperSource} from "../../src/interfaces/IKeeperSource.sol";
import {IFloorWrapped} from "../../src/interfaces/IFloorWrapped.sol";
import {IItemQueue} from "../../src/interfaces/IItemQueue.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";
import {Wait} from "../../src/lib/Wait.sol";

/// A requester that refuses ETH: what it is paid back is credited to it (DI-L-2), and it withdraws to another address.
contract Refuser {
    function ask(WakeDesk d, address w) external payable returns (uint256) { return d.request{value: msg.value}(w); }
    function launch(CurveFactory f) external returns (address coin) { (coin,) = f.launch("R", "R", SweepVault.Policy.RAFFLE, false); }
    function pull(WakeDesk d, address payable to) external { d.withdraw(to); }
    receive() external payable { revert("no"); }
}

/// A contract wallet whose receive path needs more than the forfeit's 50,000-gas stipend (a proxy that logs, a 4337 account):
/// honest, and paid in full by a call with gas (DI-L-2).
contract HeavyWallet {
    uint256[] public log;
    function ask(WakeDesk d, address w) external payable returns (uint256) { return d.request{value: msg.value}(w); }
    function pull(WakeDesk d) external { d.withdraw(payable(address(this))); }
    receive() external payable {
        for (uint256 i; i < 3; i++) log.push(msg.value);     // three fresh slots: over 60,000 gas
    }
}

/// A requester that re-enters the desk when paid: it is paid once.
contract Reenterer {
    WakeDesk d;
    uint256 id;
    uint256 coinIndex;
    uint256 public calls;
    function ask(WakeDesk d_, address w) external payable returns (uint256) { d = d_; id = d_.request{value: msg.value}(w); return id; }
    function launch(CurveFactory f) external returns (address coin) { (coin,) = f.launch("E", "E", SweepVault.Policy.RAFFLE, false); }
    function set(uint256 i) external { coinIndex = i; }
    receive() external payable {
        calls++;
        try d.reclaim(id, coinIndex) {} catch {}
        try d.forfeit(id) {} catch {}
    }
}

/// docs/v7/dormant-collections.md §3.3: the bonded wake request. Real stacks (wrapper, poster, module, factory, vaults) listed in a
/// real CollectionDirectory, as DeployStack builds them; the canister is `custodian`, the module's poster.
contract WakeDeskTest is Base {
    struct Col {
        EthNftWrapper w;
        FloorModule m;
        CurveFactory f;
        MockSeeder s;
    }

    address payee = makeAddr("payee");
    address platform = makeAddr("platform");
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address whale = makeAddr("whale");
    address payable protocol = payable(makeAddr("protocol treasury"));
    uint256 constant BOND = 0.003 ether;
    uint256 constant WINDOW = 3 days;
    CollectionDirectory dir;
    WakeDesk desk;
    Col milady;
    Col doodles;

    function _params() internal pure returns (FloorModule.Params memory p) {
        p.baseSpreadBps = 15; p.volWeightBps = 10_000; p.staleSpreadBps = 100; p.stressSpreadBps = 300; p.maxSpreadBps = 1_000;
        p.maxMoveBps = 500; p.convBps = 300; p.bufferBps = 500; p.mintPerHour = 5e18; p.burnPerHour = 5e18;
        p.maxPublicEthBacked = 10e18; p.maxEthBacked = 40e18; p.revenueTarget = 1 ether; p.rentTerm = 30 days; p.premiumBps = 1_000;
        p.rentShareBps = 5_000; p.maxRentals = 20; p.lowConfidenceSpreadBps = 100; p.fullConfidenceBps = 8_000; p.minConfidenceBps = 3_000;
        p.maxItemsPerDay = 3; p.reserveBps = 2_500; p.fillBandBps = 300;
    }

    function _template() internal pure returns (FrogCurve.Config memory c) {
        c.supply = 1_000_000_000e18; c.reserved = 200_000_000e18;
        c.platformFeeCap = 0.25 ether; c.platformBps = 500; c.poolEthBps = 1_260; c.feeBps = 100;
        c.vaultShareBps = 4_000; c.creatorShareBps = 3_000; c.reserveShareBps = 500; c.liquidityShareBps = 500;
        c.window = 3 days; c.launchFeeBps = 5_000; c.launchFeeDecay = 10 minutes;
    }

    /// One allowlisted collection, deployed and listed but never posted: DORMANT.
    function _collection(address nft) internal returns (Col memory c) {
        c.w = new EthNftWrapper(IListingPause(address(registry)), 0, IRandomnessBeacon(address(beacon)), 1, nft, custodian, "w", "w", PBS);
        c.w.initMintLimit(1_000);
        PosterSource ps = new PosterSource(IKeeperSource(address(registry)), custodian);
        c.m = new FloorModule(IFloorWrapped(address(c.w)), IKeeperSource(address(ps)), payee, platform, address(0), _params(), PBS);
        c.w.wireFloorModule(address(c.m));
        c.s = new MockSeeder(IERC20(address(c.w)));
        c.f = new CurveFactory(c.w, c.s, IItemQueue(address(c.m)), platform, _template(), CurveFactory.Targets(5 ether, 5 ether, 2 ether, 30 ether, 100, 800), PBS);
        MintDesk md = new MintDesk(c.w, payee, 6 hours, PBS);
        SweepVault r = new SweepVault(registry, 0, c.w, SweepVault.Policy.RAFFLE, address(beacon), c.f, md, PBS);
        SweepVault b = new SweepVault(registry, 0, c.w, SweepVault.Policy.BURN, address(beacon), c.f, md, PBS);
        c.f.wire(r, b);
        c.m.wire(c.f);
        c.s.setDepth(10 ether);
        vm.prank(owner);
        dir.add(c.f);
    }

    function setUp() public override {
        super.setUp();
        dir = new CollectionDirectory(owner);
        milady = _collection(makeAddr("Milady"));
        doodles = _collection(makeAddr("Doodles"));
        desk = new WakeDesk(dir, protocol, BOND, WINDOW, PBS);
        vm.deal(alice, 1 ether);
        vm.deal(bob, 1 ether);
    }

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    function _post(Col memory c, uint256 floor) internal {
        vm.prank(custodian);
        c.m.postFloor(floor, 10_000, 50);
    }

    function _launch(Col memory c, address who) internal returns (address coin, address curve) {
        vm.prank(who);
        (coin, curve) = c.f.launch("Coin", "COIN", SweepVault.Policy.RAFFLE, false);
    }

    /// The coin at `i` is bought out on a fresh module: its curve reaches its raise (Graduating or Graduated).
    function _graduate(Col memory c, uint256 i) internal {
        FrogCurve fc = FrogCurve(payable(c.f.routerOf(c.f.coins(i))));
        _skip(11 minutes);
        _post(c, 0.92 ether);
        uint256 pay = fc.raise() * 2;
        vm.deal(whale, pay);
        vm.prank(whale);
        fc.buy{value: pay}(0);
        assertTrue(uint256(fc.state()) == 1 || uint256(fc.state()) == 2, "reached its raise");
    }

    // ---- construction --------------------------------------------------------------------------------------------------------------

    function test_constructor_bounds() public {
        vm.expectRevert(WakeDesk.BadInput.selector);
        new WakeDesk(CollectionDirectory(address(0)), protocol, BOND, WINDOW, PBS);
        vm.expectRevert(WakeDesk.BadInput.selector);
        new WakeDesk(dir, payable(address(0)), BOND, WINDOW, PBS);
        vm.expectRevert(WakeDesk.BadInput.selector);
        new WakeDesk(dir, protocol, 0.0004 ether, WINDOW, PBS);
        vm.expectRevert(WakeDesk.BadInput.selector);
        new WakeDesk(dir, protocol, 1.01 ether, WINDOW, PBS);
        vm.expectRevert(WakeDesk.BadInput.selector);
        new WakeDesk(dir, protocol, BOND, 23 hours, PBS);
        vm.expectRevert(WakeDesk.BadInput.selector);
        new WakeDesk(dir, protocol, BOND, 31 days, PBS);
        vm.expectRevert(abi.encodeWithSelector(Wait.BadParentBlockSeconds.selector, 0));
        new WakeDesk(dir, protocol, BOND, WINDOW, 0);
        assertEq(desk.bond(), BOND);
        assertEq(desk.window(), WINDOW);
        assertEq(address(desk.directory()), address(dir));
        assertEq(desk.treasury(), protocol);
    }

    // ---- the premise: a dormant (never posted) collection cannot be launched on until the canister's first post ---------------------

    function test_dormantCollection_refusesLaunches_untilItsFirstPost() public {
        assertEq(milady.m.accepted(), 0, "never posted: dormant");
        assertFalse(milady.f.launchesOpen(), "the site says launches are paused");
        vm.prank(alice);
        vm.expectRevert(CurveFactory.QueueFull.selector);
        milady.f.launch("Coin", "COIN", SweepVault.Policy.RAFFLE, false);
        _post(milady, 0.92 ether);                    // the wake's first confirmed post: ACTIVE
        assertTrue(milady.f.launchesOpen());
        _launch(milady, alice);
        assertEq(milady.f.coinCount(), 1);
    }

    // ---- request ---------------------------------------------------------------------------------------------------------------------

    function test_request_needsTheExactBond_andAListedWrapper() public {
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(WakeDesk.WrongBond.selector, BOND - 1, BOND));
        desk.request{value: BOND - 1}(address(milady.w));
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(WakeDesk.WrongBond.selector, BOND + 1, BOND));
        desk.request{value: BOND + 1}(address(milady.w));
        vm.prank(alice);
        vm.expectRevert(WakeDesk.NotListed.selector);
        desk.request{value: BOND}(makeAddr("an unlisted wrapper"));
        assertEq(desk.requestCount(), 0);
    }

    function test_request_recordsWhatTheCanisterReads_andOneOpenPerCollection() public {
        vm.prank(alice);
        uint256 id = desk.request{value: BOND}(address(milady.w));
        assertEq(id, 0);
        assertEq(desk.requestCount(), 1);
        (address w, address who, uint64 at, uint128 b, bool closed) = desk.requestAt(0);
        assertEq(w, address(milady.w));
        assertEq(who, alice);
        assertEq(at, uint64(block.timestamp));
        assertEq(b, BOND);
        assertFalse(closed);
        assertEq(address(desk).balance, BOND);
        (bool open, uint256 oid) = desk.openOf(address(milady.w));
        assertTrue(open);
        assertEq(oid, 0);
        // a second request for the same collection while one is open: refused (the wake is already asked for)
        vm.prank(bob);
        vm.expectRevert(abi.encodeWithSelector(WakeDesk.AlreadyRequested.selector, 0));
        desk.request{value: BOND}(address(milady.w));
        // another collection is its own
        vm.prank(bob);
        assertEq(desk.request{value: BOND}(address(doodles.w)), 1);
        // after the window (both clocks) the collection may be asked for again
        vm.warp(block.timestamp + WINDOW);            // the sequencer's clock alone is not enough
        vm.prank(bob);
        vm.expectRevert(abi.encodeWithSelector(WakeDesk.AlreadyRequested.selector, 0));
        desk.request{value: BOND}(address(milady.w));
        vm.roll(block.number + WINDOW / PBS + 1);
        assertFalse(desk.isOpen(0));
        vm.prank(bob);
        assertEq(desk.request{value: BOND}(address(milady.w)), 2);
    }

    // ---- reclaim by a launch ---------------------------------------------------------------------------------------------------------

    function test_reclaim_paysTheRequesterBack_whenTheyLaunch() public {
        _post(milady, 0.92 ether);
        _launch(milady, bob);                         // a coin launched BEFORE the request is not the requester's launch
        _skip(milady.m.FLOOR_TTL());                  // asleep again: its floor is stale (a fresh module refuses requests, DI-M-2)
        vm.prank(alice);
        uint256 id = desk.request{value: BOND}(address(milady.w));
        vm.expectRevert(WakeDesk.NotALaunch.selector);
        desk.reclaim(id, 0);
        vm.expectRevert(WakeDesk.NotALaunch.selector);
        desk.reclaim(id, 1);                          // no such coin yet
        _launch(milady, bob);                         // after the request, by someone else: not hers
        vm.expectRevert(WakeDesk.NotALaunch.selector);
        desk.reclaim(id, 1);
        _launch(milady, alice);
        // DR-H-1: a bare launch costs gas alone: the bond waits for the coin to reach its raise
        vm.expectRevert(WakeDesk.NotGraduated.selector);
        desk.reclaim(id, 2);
        _graduate(milady, 2);
        uint256 before = alice.balance;
        vm.prank(stranger);                           // anyone may send it back to her
        desk.reclaim(id, 2);
        assertTrue(desk.spent(milady.f.coins(2)));
        assertEq(alice.balance, before + BOND);
        assertEq(address(desk).balance, 0);
        (,,,, bool closed) = desk.requestAt(id);
        assertTrue(closed);
        vm.expectRevert(WakeDesk.Closed.selector);
        desk.reclaim(id, 2);
        _skip(WINDOW + 1 days + 1);
        vm.expectRevert(WakeDesk.Closed.selector);
        desk.forfeit(id);
        // a closed request frees the collection for a new one at once
        vm.prank(bob);
        assertEq(desk.request{value: BOND}(address(milady.w)), 1);
    }

    function test_reclaim_stillLandsInTheGrace_butNotAfter() public {
        vm.prank(alice);
        uint256 id = desk.request{value: BOND}(address(milady.w));
        _post(milady, 0.92 ether);
        _launch(milady, alice);
        _graduate(milady, 0);
        _skip(WINDOW + 12 hours);                     // the window is over, the grace is not
        vm.expectRevert(WakeDesk.TooEarly.selector);
        desk.forfeit(id);
        desk.reclaim(id, 0);
        assertEq(alice.balance, 1 ether);
        vm.prank(bob);
        uint256 id2 = desk.request{value: BOND}(address(milady.w));
        _launch(milady, bob);
        _graduate(milady, 1);
        _skip(WINDOW + 1 days + 1);
        vm.expectRevert(WakeDesk.TooLate.selector);
        desk.reclaim(id2, 1);
    }

    // ---- forfeit ------------------------------------------------------------------------------------------------------------------

    function test_forfeit_sendsTheBondToTheTreasury_onlyAfterTheWindowAndTheGrace_onBothClocks() public {
        vm.prank(alice);
        uint256 id = desk.request{value: BOND}(address(milady.w));
        _post(milady, 0.92 ether);                    // the wake was served: its first post came after the request
        vm.expectRevert(WakeDesk.TooEarly.selector);
        desk.forfeit(id);
        vm.warp(block.timestamp + WINDOW + 1 days + 1);   // the sequencer's clock alone
        vm.expectRevert(WakeDesk.TooEarly.selector);
        desk.forfeit(id);
        vm.roll(block.number + (WINDOW + 1 days) / PBS + 1);
        vm.prank(stranger);
        desk.forfeit(id);
        assertEq(protocol.balance, BOND);
        assertEq(address(desk).balance, 0);
        vm.expectRevert(WakeDesk.Closed.selector);
        desk.forfeit(id);
    }

    function test_aRequesterThatRefusesEth_isCreditedItsReclaim_andWithdrawsElsewhere() public {
        Refuser r = new Refuser();
        vm.deal(address(r), 1 ether);
        uint256 id = r.ask{value: BOND}(desk, address(milady.w));
        _post(milady, 0.92 ether);
        r.launch(milady.f);
        _graduate(milady, 0);
        vm.expectEmit(true, true, false, true);
        emit WakeDesk.Owed(id, address(r), BOND);
        desk.reclaim(id, 0);                          // DI-L-2: its refusal no longer blocks the reclaim; the bond is credited
        assertEq(desk.owed(address(r)), BOND);
        _skip(WINDOW + 1 days + 1);
        vm.expectRevert(WakeDesk.Closed.selector);
        desk.forfeit(id);
        vm.expectRevert(WakeDesk.TransferFailed.selector);
        r.pull(desk, payable(address(r)));            // to itself it still refuses
        r.pull(desk, payable(bob));
        assertEq(bob.balance, 1 ether + BOND);
        assertEq(desk.owed(address(r)), 0);
        assertEq(protocol.balance, 0);
        vm.expectRevert(WakeDesk.NothingOwed.selector);
        r.pull(desk, payable(bob));
    }

    function test_aReenteringRequester_isPaidOnce() public {
        Reenterer e = new Reenterer();
        vm.deal(address(e), 1 ether);
        uint256 id = e.ask{value: BOND}(desk, address(milady.w));
        _post(milady, 0.92 ether);
        e.launch(milady.f);
        _graduate(milady, 0);
        e.set(0);
        uint256 before = address(e).balance;
        desk.reclaim(id, 0);
        assertEq(address(e).balance, before + BOND, "one bond");
        assertEq(e.calls(), 1);
        assertEq(address(desk).balance, 0);
        assertEq(protocol.balance, 0);
    }

    function test_theDeskHoldsOnlyBonds_eachRequestItsOwn() public {
        vm.prank(alice);
        desk.request{value: BOND}(address(milady.w));
        vm.prank(bob);
        desk.request{value: BOND}(address(doodles.w));
        assertEq(address(desk).balance, 2 * BOND);
        _post(doodles, 0.25 ether);
        _launch(doodles, bob);
        _graduate(doodles, 0);
        desk.reclaim(1, 0);
        assertEq(address(desk).balance, BOND, "alice's bond is untouched");
        _skip(WINDOW + 1 days + 1);
        uint256 before = alice.balance;
        desk.forfeit(0);                              // Milady was never posted: alice's wake was not served, her bond comes back
        assertEq(address(desk).balance, 0);
        assertEq(alice.balance, before + BOND);
        assertEq(protocol.balance, 0);
    }

    // ---- dormant-review DR-H-1, DR-L-5, DR-H-3 ------------------------------------------------------------------------------------

    function test_reclaim_refusesAFloorBuysOnlyCoin_andOneCoinPaysBackOneBond() public {
        vm.prank(alice);
        uint256 id = desk.request{value: BOND}(address(milady.w));
        _post(milady, 0.92 ether);
        vm.prank(alice);
        milady.f.launch("S", "S", SweepVault.Policy.RAFFLE, true);   // sweep-only: gas alone, needs no floor
        vm.expectRevert(WakeDesk.NotALaunch.selector);
        desk.reclaim(id, 0);
        _launch(milady, alice);
        _graduate(milady, 1);
        desk.reclaim(id, 1);
        // a second request by the same requester: the same graduated coin pays back nothing more
        _skip(milady.m.FLOOR_TTL());
        vm.prank(alice);
        uint256 id2 = desk.request{value: BOND}(address(milady.w));
        vm.expectRevert(WakeDesk.NotALaunch.selector);   // launched before this request
        desk.reclaim(id2, 1);
        assertTrue(desk.spent(milady.f.coins(1)));
    }

    function test_reclaim_refusesTheSameCoinForTwoRequests() public {
        // DR-L-5: request a's window ends (its grace does not), request b follows: one coin launched after both qualifies for both
        _post(milady, 0.92 ether);
        _skip(milady.m.FLOOR_TTL());
        vm.prank(alice);
        uint256 a = desk.request{value: BOND}(address(milady.w));
        _skip(WINDOW + 1);
        vm.prank(alice);
        uint256 b = desk.request{value: BOND}(address(milady.w));
        _launch(milady, alice);
        _graduate(milady, 0);
        desk.reclaim(a, 0);
        vm.expectRevert(WakeDesk.AlreadySpent.selector);
        desk.reclaim(b, 0);
        assertEq(address(desk).balance, BOND, "one launch, one bond");
    }

    function test_forfeit_refundsAWakeTheCanisterNeverServed_andChargesOneItServed() public {
        vm.prank(alice);
        uint256 a = desk.request{value: BOND}(address(milady.w));   // queued behind the caps: never posted in the window
        vm.prank(bob);
        uint256 b = desk.request{value: BOND}(address(doodles.w));
        _post(doodles, 0.25 ether);                                 // Doodles woke: served
        assertFalse(desk.served(a));
        assertTrue(desk.served(b));
        _skip(WINDOW + 1 days + 1);
        uint256 ab = alice.balance;
        vm.expectEmit(true, true, false, true);
        emit WakeDesk.Refunded(a, alice, BOND);
        desk.forfeit(a);
        assertEq(alice.balance, ab + BOND);
        desk.forfeit(b);
        assertEq(protocol.balance, BOND);
        assertEq(address(desk).balance, 0);
        // a requester that refuses ETH: its unserved bond is credited to it (DI-L-2), never the treasury's, and the request still closes
        Refuser r = new Refuser();
        vm.deal(address(r), 1 ether);
        uint256 c = r.ask{value: BOND}(desk, address(milady.w));
        _skip(WINDOW + 1 days + 1);
        desk.forfeit(c);
        assertEq(protocol.balance, BOND);
        assertEq(desk.owed(address(r)), BOND);
        assertEq(address(desk).balance, BOND, "held for its withdrawal");
    }

    // ---- independent review 1: DI-M-1, DI-M-2, DI-L-2 ------------------------------------------------------------------------------

    /// DI-M-1, the reviewer's PoC (test_poc_lateServiceChargesARequesterWhoCouldNeverReclaim): a post after the window no longer
    /// turns an unserved request into a forfeit. The requester could not reclaim any more; the bond goes back to her.
    function test_diM1_aPostAfterTheWindow_doesNotChargeARequesterWhoCouldNeverReclaim() public {
        vm.prank(alice);
        uint256 id = desk.request{value: BOND}(address(milady.w));
        _skip(WINDOW + 1 days + 1);                   // a reclaim is now TooLate
        _post(milady, 0.9 ether);                     // a post long after the window (another wake's, or a stale queued one)
        assertFalse(desk.served(id));
        vm.expectRevert(WakeDesk.NotServed.selector);
        desk.markServed(id);                          // nobody can record a post outside the window
        uint256 t0 = protocol.balance;
        uint256 a0 = alice.balance;
        desk.forfeit(id);
        assertEq(protocol.balance, t0, "the treasury is paid nothing for a wake that came too late");
        assertEq(alice.balance, a0 + BOND);
    }

    /// DI-M-1, the treasury's side: a wake served inside the window stays served after later posts, once recorded (the canister sends
    /// markServed after the post). Unrecorded, the module's latest post decides, and a collection that stayed awake past the window
    /// looks unserved: that is why the canister records it.
    function test_diM1_aWakeServedInTheWindow_andRecorded_staysChargedWhateverIsPostedLater() public {
        vm.prank(alice);
        uint256 a = desk.request{value: BOND}(address(milady.w));
        vm.prank(bob);
        uint256 b = desk.request{value: BOND}(address(doodles.w));
        vm.expectRevert(WakeDesk.NotServed.selector);
        desk.markServed(a);                           // nothing posted since the request
        _skip(1 days);
        _post(milady, 0.92 ether);                    // both wakes served on day 1
        _post(doodles, 0.25 ether);
        vm.expectEmit(true, false, false, true);
        emit WakeDesk.Served(a, uint64(block.timestamp));
        vm.prank(stranger);                           // anyone may record it; the canister does
        desk.markServed(a);
        desk.markServed(a);                           // once is enough: a second call changes nothing
        assertEq(desk.servedAt(a), uint64(block.timestamp));
        // both collections stay awake: posts go on past the window
        for (uint256 i; i < 8; i++) {
            _skip(1 days);
            _post(milady, 0.92 ether);
            _post(doodles, 0.25 ether);
        }
        assertTrue(desk.served(a), "recorded in the window");
        assertFalse(desk.served(b), "unrecorded, and the latest post is past the window");
        vm.expectRevert(WakeDesk.NotServed.selector);
        desk.markServed(b);                           // too late to record: the latest post is outside the window
        uint256 b0 = bob.balance;
        desk.forfeit(a);
        assertEq(protocol.balance, BOND, "the served wake pays the treasury");
        desk.forfeit(b);
        assertEq(bob.balance, b0 + BOND);
        vm.expectRevert(WakeDesk.Closed.selector);
        desk.markServed(a);
    }

    /// DI-M-1: a post late in the window may still be recorded after the window while it is the module's latest (nothing posted since).
    function test_diM1_theLatestPostInsideTheWindow_countsWithoutARecord_andMayBeRecordedLater() public {
        vm.prank(alice);
        uint256 id = desk.request{value: BOND}(address(milady.w));
        _skip(WINDOW - 1 hours);
        _post(milady, 0.92 ether);
        _skip(2 hours);                               // the window is over; the post inside it is still the latest
        assertTrue(desk.served(id));
        desk.markServed(id);
        _skip(1 days);
        _post(milady, 0.92 ether);
        assertTrue(desk.served(id));
        desk.forfeit(id);
        assertEq(protocol.balance, BOND);
    }

    /// DI-M-2, the reviewer's PoC (poc_di_3): a request made while the collection is awake held the collection's one open slot for the
    /// whole window and asked the canister for nothing. A fresh module now refuses it: nothing is held, and once the collection sleeps
    /// (its floor goes stale) anyone may ask.
    function test_diM2_aRequestWhileTheCollectionIsAwake_isRefused_andOneAfterItSleepsIsTaken() public {
        _post(milady, 0.92 ether);
        assertTrue(milady.m.fresh());
        vm.prank(alice);
        vm.expectRevert(WakeDesk.Awake.selector);
        desk.request{value: BOND}(address(milady.w));
        (bool open,) = desk.openOf(address(milady.w));
        assertFalse(open, "no slot held");
        _skip(milady.m.FLOOR_TTL());                  // asleep: the floor is stale
        assertFalse(milady.m.fresh());
        vm.prank(bob);
        assertEq(desk.request{value: BOND}(address(milady.w)), 0);
    }

    /// DI-L-2: an honest contract wallet whose receive needs more than the 50,000-gas stipend is no longer treated as a refuser: its
    /// unserved bond is credited to it, never paid to the treasury, and it withdraws it with gas.
    function test_diL2_aHeavyWalletsUnservedBond_isCredited_andWithdrawn() public {
        HeavyWallet hw = new HeavyWallet();
        vm.deal(address(hw), 1 ether);
        uint256 id = hw.ask{value: BOND}(desk, address(milady.w));
        _skip(WINDOW + 1 days + 1);
        vm.expectEmit(true, true, false, true);
        emit WakeDesk.Owed(id, address(hw), BOND);
        desk.forfeit(id);
        assertEq(protocol.balance, 0, "the treasury is not paid an unserved bond");
        assertEq(desk.owed(address(hw)), BOND);
        uint256 before = address(hw).balance;
        hw.pull(desk);
        assertEq(address(hw).balance, before + BOND);
        assertEq(desk.owed(address(hw)), 0);
        assertEq(address(desk).balance, 0);
    }

    // ---- the Step::Fill premise (docs §5): a graduation queued at a stale module is filled by anyone once the module is fresh ----

    function test_aGraduationQueuedAtAStaleModule_isFilledByAnyone_onceTheFloorIsFresh() public {
        _post(milady, 0.92 ether);                    // once active: gradPrice() stays non-zero after the posts stop
        _skip(milady.m.FLOOR_TTL());                  // dormant again: the floor is stale
        assertFalse(milady.m.fresh());
        (address coin, address curve) = _launch(milady, alice);   // a launch on a dormant, once-active module is still accepted
        FrogCurve fc = FrogCurve(payable(curve));
        _skip(11 minutes);
        uint256 pay = fc.raise() * 2;
        vm.deal(whale, pay);
        vm.prank(whale);
        fc.buy{value: pay}(0);                        // bought out at once: the graduation reaches a stale module
        assertGt(milady.m.gradEth(), 0, "queued, not filled");
        assertFalse(milady.m.isFilled(coin));
        vm.expectRevert(FrogCurve.NotFilled.selector);
        fc.graduate();
        _post(milady, 0.92 ether);                    // the wake's first post: fresh again
        vm.prank(custodian);                          // the canister's Step::Fill (anyone may)
        milady.m.fill(coin);
        assertTrue(milady.m.isFilled(coin));
        assertEq(milady.m.gradEth(), 0);
        fc.graduate();
        assertEq(uint256(fc.state()), uint256(FrogCurve.State.Graduated));
    }

    // ---- independent review 2: D2-L-1 ----------------------------------------------------------------------------------------------

    /// D2-L-1, the reviewer's PoC (test_poc_d2_markServedInTheBlockTail_chargesAPostOutsideTheWindow): the parent blocks run slower
    /// than the desk's `parentBlockSeconds`, so the block clock's half of the window is still open after `at + window`. A post in that
    /// tail is outside the window, and `markServed` now refuses it on the timestamp alone: the requester is refunded, not charged.
    function test_d2L1_aPostInTheBlockClocksTail_isNotRecordedServed_andTheBondGoesBack() public {
        vm.prank(alice);
        uint256 id = desk.request{value: BOND}(address(milady.w));
        uint64 at = uint64(block.timestamp);
        uint256 t = WINDOW + 20 minutes;              // wall time, at 12.1 s a parent block
        vm.warp(block.timestamp + t);
        vm.roll(block.number + (t * 10) / 121);
        (bool open,) = desk.openOf(address(milady.w));
        assertTrue(open, "the block clock's half of the window is still running");
        _post(milady, 0.9 ether);                     // outside [at, at + window]
        assertGt(milady.m.postedAt(), at + WINDOW);
        assertFalse(desk.served(id));
        vm.prank(stranger);
        vm.expectRevert(WakeDesk.NotServed.selector);
        desk.markServed(id);                          // refused: past at + window, whatever the block clock says
        assertEq(desk.servedAt(id), 0);
        _skip(2 days);
        uint256 t0 = protocol.balance;
        uint256 a0 = alice.balance;
        desk.forfeit(id);
        assertEq(protocol.balance, t0, "the treasury is paid nothing for a post outside the window");
        assertEq(alice.balance, a0 + BOND);
    }

    /// D2-L-1, the other side: a post at the window's last second is still recorded in the block tail (and after it, while latest).
    function test_d2L1_aPostAtTheWindowsEnd_isStillRecordedServedInTheBlockTail() public {
        vm.prank(alice);
        uint256 id = desk.request{value: BOND}(address(milady.w));
        uint64 at = uint64(block.timestamp);
        vm.warp(at + WINDOW);
        vm.roll(block.number + (WINDOW * 10) / 121);
        _post(milady, 0.9 ether);                     // exactly at + window: inside
        vm.warp(at + WINDOW + 20 minutes);
        vm.prank(stranger);
        desk.markServed(id);
        assertEq(desk.servedAt(id), at + WINDOW);
        _skip(2 days);
        _post(milady, 0.9 ether);
        desk.forfeit(id);
        assertEq(protocol.balance, BOND);
    }
}
