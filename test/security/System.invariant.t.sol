// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Base} from "../utils/Base.sol";
import {Merkle} from "../utils/Merkle.sol";
import {MockEscrow, MockCurve, RefusesEth} from "../utils/Mocks.sol";
import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";
import {AssetVault} from "../../src/AssetVault.sol";
import {Distributor} from "../../src/Distributor.sol";
import {FeeRouter} from "../../src/FeeRouter.sol";
import {ListingRegistry} from "../../src/ListingRegistry.sol";
import {IPonsV2FeeEscrow, IPonsV2BondingCurve, IPonsV2MemeHook} from "../../src/interfaces/IPons.sol";
import {Clock} from "../utils/Clock.sol";

contract Feeder { constructor(address payable to) payable { selfdestruct(to); } }

/// docs/phase3.md §3. The whole money path under one handler: three coins with real routers, escrow and curve, one vault, a
/// treasury that sometimes refuses ETH, two purchasers (one rotated in half way), a bondsman, and somebody force-feeding the vault.
contract SystemHandler is Test {
    ListingRegistry registry; AssetVault vault; RefusesEth public treasury; MockEscrow escrow;
    address owner; address keeper; address attestor;
    address[2] public purchasers;
    FeeRouter[3] public routers; MockCurve[3] curves; address[3] public coins;

    uint256 public gIn;               // every wei that entered through a fee path (escrow credit, curve, plain send to a router)
    uint256 public gBond; uint256 public gBondOut; uint256 public gReturned; uint256 public gForced; uint256 public gReleased;
    uint256 public violations;
    uint256 public releases; uint256 public writeOffs; uint256 public settlements;

    constructor(ListingRegistry r, AssetVault v, RefusesEth t, MockEscrow e, address o, address k, address a, address p0, address p1,
        FeeRouter[3] memory rs, MockCurve[3] memory cs, address[3] memory coins_) {
        registry = r; vault = v; treasury = t; escrow = e; owner = o; keeper = k; attestor = a; purchasers = [p0, p1];
        routers = rs; curves = cs; coins = coins_;
    }

    function fees(uint8 which, uint96 amount, uint8 path) external {
        amount = uint96(bound(amount, 1, 3 ether));
        uint256 i = which % 3;
        vm.deal(address(this), amount);
        if (path % 3 == 0) escrow.credit{value: amount}(address(routers[i]));
        else if (path % 3 == 1) { (bool ok,) = address(curves[i]).call{value: amount}(""); require(ok); }
        else { (bool ok,) = address(routers[i]).call{value: amount}(""); require(ok); }
        gIn += amount;
    }

    /// the ordinary working paths, so that a campaign spends its calls on money that really moves
    function feeAndHarvest(uint8 which, uint96 amount, uint8 path) external { this.fees(which, amount, path); this.harvest(which); }
    function bondDeclareRelease(uint96 cap, uint96 maxEth, uint8 q) external {
        this.bond(10 ether);
        this.declare(cap, maxEth, q);
        uint256 n = vault.purchaseCount();
        if (n != 0) this.release(n - 1, true);
    }
    function settleLatest(uint64 amount, uint96 back) external {
        uint256 n = vault.purchaseCount(); if (n == 0) return;
        this.giveBack(n - 1, back);
        this.settle(n - 1, amount);
    }

    function dust(uint8 which) external { this.fees(which, 1, 2); this.harvest(which); }

    function harvest(uint8 which) external { routers[which % 3].harvest(); }

    function treasuryMood(bool open) external { treasury.setOpen(open); }

    function forceFeed(uint64 amount) external {
        uint256 a = bound(amount, 1, 1 ether);
        vm.deal(address(this), a);
        new Feeder{value: a}(payable(address(vault)));
        gForced += a;
    }

    function bond(uint96 amount) external {
        amount = uint96(bound(amount, 1, 10 ether));
        vm.deal(address(0xB07D), amount);
        vm.prank(address(0xB07D)); vault.postBond{value: amount}();
        gBond += amount;
    }

    function unbond(uint8 who, uint96 amount) external {
        address p = purchasers[who % 2];
        uint256 before = p.balance;
        vm.prank(p);
        try vault.withdrawBond(bound(amount, 1, 10 ether)) {
            gBondOut += p.balance - before;
            if (vault.bondOf(p) < vault.requiredBondOf(p)) violations++;      // never below what is in their hands
        } catch {}
    }

    function rotate() external {
        address next = registry.purchaser() == purchasers[0] ? purchasers[1] : purchasers[0];
        vm.startPrank(owner);
        registry.proposeRoles(keeper, next, attestor);
        Clock.skip(7 days);
        registry.acceptRoles();
        vm.stopPrank();
    }

    function declare(uint96 cap, uint96 maxEth, uint8 q) external {
        uint256 available = vault.attributed() - vault.reserved();
        // mostly a declaration that can succeed (within what is spendable, under a cap that allows it); sometimes one that cannot
        uint256 want = available == 0 || cap % 7 == 0 ? bound(maxEth, 1, 8 ether) : bound(maxEth, 1, available);
        vm.prank(keeper); vault.postCap(cap % 5 == 0 ? bound(cap, 0.01 ether, 5 ether) : want);
        vm.prank(keeper); try vault.declarePurchase(want, bound(q, 1, 5) * 100, "h") {} catch {}
    }

    function release(uint256 id, bool waitFirst) external {
        uint256 n = vault.purchaseCount(); if (n == 0) return;
        id = id % n;
        if (waitFirst) Clock.skip(1 hours);
        AssetVault.Purchase memory p = vault.purchase(id);
        uint256 before = p.purchaser.balance; uint256 vb = address(vault).balance;
        try vault.release(id) {
            releases++;
            gReleased += p.maxEth;
            if (p.purchaser.balance - before != p.maxEth || vb - address(vault).balance != p.maxEth) violations++;
            if (vault.bondOf(p.purchaser) < vault.requiredBondOf(p.purchaser)) violations++;
            AssetVault.Charge[] memory cs = vault.chargesOf(id);
            uint256 sum; for (uint256 i = 0; i < cs.length; ++i) sum += cs[i].weiCharged;
            if (sum != p.maxEth || cs.length > vault.MAX_PAYERS()) violations++;   // what the coins were charged IS what the purchase cost
        } catch {}
    }

    function giveBack(uint256 id, uint96 amount) external {
        uint256 n = vault.purchaseCount(); if (n == 0) return;
        amount = uint96(bound(amount, 1, 8 ether));
        vm.deal(address(this), amount);
        try vault.returnUnspent{value: amount}(id % n) { gReturned += amount; } catch {}
    }

    function settle(uint256 id, uint64 amount) external {
        uint256 n = vault.purchaseCount(); if (n == 0) return;
        vm.prank(attestor);
        try vault.settleRune(id % n, bound(amount, 1, 1_000), keccak256(abi.encode(id)), 1) { settlements++; } catch {}
    }

    function writeOff(uint256 id, bool late, bool byAnyone) external {
        uint256 n = vault.purchaseCount(); if (n == 0) return;
        if (late) Clock.skip((byAnyone ? 11 days : 4 days));
        vm.prank(byAnyone ? address(0xA11) : owner);
        try vault.writeOff(id % n, "w") { writeOffs++; } catch {}
    }

    function cancel(uint256 id) external {
        uint256 n = vault.purchaseCount(); if (n == 0) return;
        vm.prank(owner); try vault.cancel(id % n) {} catch {}
    }

    function wait(uint32 s) external { Clock.skip(bound(s, 1, 3 days)); }

    function tryToTake(uint8 who, uint8 how) external {
        address[6] memory actors = [owner, keeper, purchasers[0], purchasers[1], attestor, address(0xBAD)];
        bytes[5] memory calls = [abi.encodeWithSignature("withdraw(uint256)", 1 ether), abi.encodeWithSignature("withdraw()"),
            abi.encodeWithSignature("sweep(address)", actors[who % 6]), abi.encodeWithSignature("release(uint256)", type(uint256).max),
            abi.encodeWithSignature("withdrawBond(uint256)", type(uint256).max)];
        uint256 before = address(vault).balance;
        vm.prank(actors[who % 6]);
        (bool ok,) = address(vault).call(calls[how % 5]);
        if (ok || address(vault).balance != before) violations++;
    }

    function routerBalances() external view returns (uint256 sum) {
        for (uint256 i = 0; i < 3; ++i) sum += address(routers[i]).balance + address(curves[i]).balance + escrow.balanceOf(address(routers[i]));
    }
}

contract SystemInvariantTest is Base {
    SystemHandler handler; RefusesEth till; MockEscrow escrow;
    FeeRouter[3] routers; MockCurve[3] curves; address[3] coins;
    address p1 = makeAddr("purchaser2");

    function setUp() public override {
        super.setUp();
        till = new RefusesEth(); till.setOpen(true);
        vm.startPrank(owner);
        registry.proposeTreasury(address(till)); Clock.skip(7 days); registry.acceptTreasury();
        registry.setBondBps(5_000);                                   // half, so that both a covered and an uncovered loss happen
        vm.stopPrank();
        escrow = new MockEscrow();
        address impl = address(new FeeRouter());
        for (uint256 i = 0; i < 3; ++i) {
            coins[i] = address(uint160(0xC011 + i));
            curves[i] = new MockCurve(escrow);
            routers[i] = FeeRouter(payable(Clones.clone(impl)));
            routers[i].initialize(coins[i], registry, runeVault, IPonsV2FeeEscrow(address(escrow)), IPonsV2BondingCurve(address(curves[i])), IPonsV2MemeHook(address(0xB00C)));
            curves[i].setRecipient(address(routers[i]));
            launcher.set(coins[i], address(routers[i]), creatorA, 1);
        }
        handler = new SystemHandler(registry, runeVault, till, escrow, owner, keeper, attestor, purchaser, p1, routers, curves, coins);
        targetContract(address(handler));
    }

    /// Every wei that came in is somewhere it is allowed to be, and nowhere else.
    function invariant_conservation() public view {
        assertEq(handler.violations(), 0);
        uint256 inFlight = handler.routerBalances();
        uint256 out = purchaser.balance + p1.balance;                 // releases and bond refunds, to purchasers only
        assertEq(handler.gIn() + handler.gBond() + handler.gReturned() + handler.gForced(),
            inFlight + address(runeVault).balance + address(till).balance + out);
        assertEq(out, handler.gReleased() + handler.gBondOut());
    }

    /// ETH leaves the vault only as a release or a bond refund.
    function invariant_vaultOutflowIsReleasesAndBondRefunds() public view {
        uint256 vaultIn = address(runeVault).balance + handler.gReleased() + handler.gBondOut();
        uint256 fromRouters = 0;
        for (uint256 i = 0; i < 3; ++i) fromRouters += runeVault.contributedBy(coins[i]);
        assertEq(vaultIn, fromRouters + handler.gBond() + handler.gReturned() + handler.gForced());
    }

    /// The books: what the vault holds is what it owes the coins, the bonds, and whatever was forced in.
    function invariant_books() public view {
        assertEq(runeVault.attributed() + runeVault.waiting() + runeVault.bond() + handler.gForced(), address(runeVault).balance);
        assertLe(runeVault.reserved(), runeVault.attributed());
        assertEq(runeVault.bondOf(purchaser) + runeVault.bondOf(p1), runeVault.bond());
        assertEq(runeVault.outstandingOf(purchaser) + runeVault.outstandingOf(p1), runeVault.outstanding());
        uint256 unspent; uint256 queuedSum;
        for (uint256 i = 0; i < 3; ++i) {
            unspent += runeVault.unspentOf(coins[i]);
            if (runeVault.queued(coins[i])) queuedSum += runeVault.unspentOf(coins[i]);
        }
        assertEq(unspent, runeVault.attributed() + runeVault.waiting());
        assertEq(queuedSum, runeVault.attributed());
        assertLe(runeVault.queueLength() - runeVault.queueHead(), 3, "a coin is in the queue at most once");
    }

    /// 80/20, to the wei, whatever the treasury's mood: what the treasury has plus what it is still owed is a fifth of what was split.
    function invariant_split() public view {
        uint256 toVault; uint256 owed;
        for (uint256 i = 0; i < 3; ++i) { toVault += runeVault.contributedBy(coins[i]); owed += routers[i].treasuryOwed(); }
        uint256 split = toVault + address(till).balance + owed;
        assertLe(split * 8_000 / 10_000 >= toVault ? split * 8_000 / 10_000 - toVault : 0, 1_000_000, "rounding only, a wei a harvest");
        assertGe(split * 8_000 / 10_000 + 1, toVault, "the vault never gets more than its four fifths");
    }

    // Not vacuous: an invariant asserting `handler.releases() == 0` was added once and failed within two calls (docs/phase3.md §3).

    function test_theSystemHandlerReallyMovesMoney() public {
        handler.fees(0, 2 ether, 0); handler.fees(1, 1 ether, 1); handler.fees(2, 1, 2);
        handler.harvest(0); handler.harvest(1); handler.harvest(2);
        handler.bond(3 ether);
        handler.declare(1 ether, 2 ether, 3);
        handler.release(0, true);
        assertEq(handler.releases(), 1);
        handler.rotate();
        handler.writeOff(0, true, true);
        assertEq(handler.writeOffs(), 1);
        invariant_conservation(); invariant_vaultOutflowIsReleasesAndBondRefunds(); invariant_books(); invariant_split();
    }
}

/// A distribution claimed by everybody, in any order, any number of times, with right and wrong proofs.
contract ClaimHandler is Test {
    Distributor d; uint256 listingId; uint256 aid;
    address[5] public holders; uint256[5] public units; bytes32[] leaves;
    uint256 public claimedOk; uint256 public violations; uint256 public paidOut;
    mapping(address => uint256) public timesClaimed;

    constructor(Distributor d_, uint256 l, uint256 a, address[5] memory h, uint256[5] memory u, bytes32[] memory lv) {
        d = d_; listingId = l; aid = a; holders = h; units = u; leaves = lv;
    }

    function claim(uint8 who, uint8 asWhom, uint64 lie) external {
        uint256 i = who % 5; uint256 j = asWhom % 5;
        uint256 amount = lie % 3 == 0 ? units[i] : uint256(lie);
        try d.claimShare(listingId, aid, holders[j], amount, Merkle.proof(leaves, i)) {
            if (i != j || amount != units[i]) violations++;           // only the true (holder, units) pair is ever accepted
            if (++timesClaimed[holders[j]] > 1) violations++;         // and once
            claimedOk += amount;
        } catch {}
    }

    function payout(uint8 who) external {
        address h = holders[who % 5];
        uint256 before = d.accrued(listingId, h);
        vm.prank(h);
        try d.requestPayout(listingId, "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0") { paidOut += before; } catch {}
    }

    function wait(uint32 s) external { Clock.skip(bound(s, 1, 2 days)); }
}

contract ClaimInvariantTest is Base {
    ClaimHandler handler; uint256 aid;
    address[5] holders; uint256[5] units; bytes32[] leaves;

    function setUp() public override {
        super.setUp();
        launcher.set(coinA, router, creatorA, 1);
        _fund(runeVault, coinA, 1 ether);
        vm.prank(keeper); runeVault.postCap(1 ether);
        vm.prank(keeper); uint256 id = runeVault.declarePurchase(1 ether, 100, "f");
        Clock.skip(1 hours);
        runeVault.release(id);
        vm.prank(attestor); runeVault.settleRune(id, 100_000, bytes32(uint256(1)), 1);
        aid = 0;
        uint256[5] memory u = [uint256(40_000), 30_000, 15_000, 10_000, 5_000];
        for (uint256 i = 0; i < 5; ++i) {
            holders[i] = address(uint160(0xAA00 + i)); units[i] = u[i];
            leaves.push(distributor.shareLeaf(holders[i], u[i]));
        }
        vm.prank(creatorA); distributor.choose(1, aid, Distributor.Mode.ProRata, address(0));
        vm.prank(keeper); distributor.openDistribution(1, aid, Merkle.root(leaves), "ipfs://s");
        vm.prank(keeper); distributor.postMinimum(1, 1);
        handler = new ClaimHandler(distributor, 1, aid, holders, units, leaves);
        targetContract(address(handler));
    }

    /// A claim pays once, only the true pair, and never more than was bought.
    function invariant_aClaimPaysOnce() public view {
        assertEq(handler.violations(), 0);
        Distributor.Distribution memory dist = distributor.distribution(1, aid);
        assertEq(dist.claimed, handler.claimedOk());
        assertLe(dist.claimed, dist.total);
        uint256 accrued;
        for (uint256 i = 0; i < 5; ++i) accrued += distributor.accrued(1, holders[i]);
        assertEq(accrued + handler.paidOut(), dist.claimed, "what was claimed is either still accrued or was asked for, never both");
    }
}
