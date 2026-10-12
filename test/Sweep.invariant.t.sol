// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {SweepBase, MockVenue} from "./utils/SweepBase.sol";
import {Merkle} from "./utils/Merkle.sol";
import {Clock} from "./utils/Clock.sol";
import {Wrapper} from "../src/Wrapper.sol";
import {SweepVault} from "../src/SweepVault.sol";

/// Every actor, in any order, across the wrapper, both sweep vaults, the raffle and the burn: holder deposits, fees in (ETH and
/// wFROG), caps, sweeps at fair and unfair prices, draws through every stage, retirements, deliveries, refunds, reclaims, stray
/// transfers, holder redemptions, and time on both clocks.
/// @dev As in Wrapper.invariant.t.sol, a Panic anywhere is a violation, not a skipped step.
contract SweepHandler is Test {
    Wrapper public w;
    SweepVault public raffle;
    SweepVault public burn;
    MockVenue public venue;
    address keeper; address owner; address custodian;
    address[3] actors;
    address[2] raffleCoins; address[2] raffleRouters;
    address burnCoin; address burnRouter;
    string[] frogIds;
    bytes32[] frogLeaves;
    uint256 nextTx;
    uint256 frogCursor;
    string constant TAPROOT = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";

    // ghosts
    uint256 public panics;
    mapping(address vault => uint256) public ethIn;
    mapping(address vault => uint256) public ethSpent;
    mapping(address vault => uint256) public capTimesBought;      // Σ cap × bought / 1e18, per vault
    uint256 public sweeps; uint256 public claims; uint256 public retirements; uint256 public deliveries;
    uint256 public refunds; uint256 public reclaims; uint256 public voids; uint256 public cancels;
    uint256[] public burnTickets;

    struct Env {
        Wrapper w; SweepVault raffle; SweepVault burn; MockVenue venue; address keeper; address owner; address custodian;
        address[3] actors; address[2] raffleCoins; address[2] raffleRouters; address burnCoin; address burnRouter;
    }

    constructor(Env memory e, string[] memory ids, bytes32[] memory leaves) {
        w = e.w; raffle = e.raffle; burn = e.burn; venue = e.venue; keeper = e.keeper; owner = e.owner; custodian = e.custodian;
        actors = e.actors; raffleCoins = e.raffleCoins; raffleRouters = e.raffleRouters; burnCoin = e.burnCoin; burnRouter = e.burnRouter;
        for (uint256 i = 0; i < ids.length; ++i) { frogIds.push(ids[i]); frogLeaves.push(leaves[i]); }
    }

    function _check(bytes memory err) internal {
        if (err.length >= 4 && bytes4(err) == bytes4(0x4e487b71)) panics++;
    }

    function _vault(uint256 s) internal view returns (SweepVault v, address coin, address router_) {
        if (s % 3 == 2) return (burn, burnCoin, burnRouter);
        uint256 k = s % 2;
        return (raffle, raffleCoins[k], raffleRouters[k]);
    }

    // ---- supply ----

    function depositFrog(uint256 who) public {
        address to = who % 4 == 3 ? address(venue) : actors[who % 3];
        for (uint256 tries = 0; tries < frogIds.length; ++tries) {
            uint256 i = frogCursor++ % frogIds.length;
            if (w.inscriptionHeld(frogLeaves[i])) continue;
            bytes32[] memory proof = Merkle.proof(frogLeaves, i);
            vm.prank(custodian);
            w.wrapDeposit(frogIds[i], proof, keccak256(abi.encode("tx", nextTx++)), 0, to);
            return;
        }
    }

    // ---- money in ----

    function feesInEth(uint256 s, uint96 amount) public {
        amount = uint96(bound(amount, 1, 5 ether));
        (SweepVault v, address coin, address r) = _vault(s);
        vm.deal(r, amount);
        vm.prank(r);
        v.deposit{value: amount}(coin);
        ethIn[address(v)] += amount;
    }

    function feesInWrapped(uint256 s, uint256 who) public {
        (SweepVault v, address coin, address r) = _vault(s);
        address a = actors[who % 3];
        if (w.balanceOf(a) < 1e18) return;
        vm.prank(a);
        w.transfer(r, 1e18);
        vm.prank(r);
        w.approve(address(v), 1e18);
        vm.prank(r);
        v.depositWrapped(coin, 1e18);
    }

    function stray(uint256 s, uint256 who) public {
        (SweepVault v,,) = _vault(s);
        address a = actors[who % 3];
        if (w.balanceOf(a) < 1e18) return;
        vm.prank(a);
        w.transfer(address(v), 1e18);
    }

    // ---- the sweep ----

    function postCap(uint256 s, uint64 cap) public {
        (SweepVault v,,) = _vault(s);
        cap = uint64(bound(cap, 0.001 ether, 1 ether));
        vm.prank(keeper);
        v.postCap(cap);
    }

    /// Mostly an honest sweep (funded, a fresh cap, a price near it), so the campaign reaches bought wFROG; a price above the
    /// cap, an empty venue and a stale cap still happen, and must revert cleanly.
    function sweep(uint256 s, uint96 spend, uint64 price, bool honestKeeper) public {
        (SweepVault v, address coin, address r) = _vault(s);
        if (honestKeeper) {
            if (v.credit(coin) == 0) feesInEth(s, 1 ether);
            if (!v.capFresh()) { vm.prank(keeper); v.postCap(0.05 ether); }
        }
        spend = uint96(bound(spend, 1, v.credit(coin) == 0 ? 1 : v.credit(coin)));
        uint256 cap = v.cap();
        price = uint64(bound(price, cap / 2 + 1e9, cap + cap / 10 + 1e9));
        r;
        try v.sweep(coin, spend, abi.encodeCall(MockVenue.buy, (price)), 1) returns (uint256 spent, uint256 bought) {
            ethSpent[address(v)] += spent;
            capTimesBought[address(v)] += (cap * bought) / 1e18 + 1;   // +1: rounding headroom for the ghost only
            sweeps++;
        } catch (bytes memory err) { _check(err); }
    }

    // ---- RAFFLE ----

    /// The draw's sentinel leaf (C-03), for the claim's one-step proof.
    mapping(uint256 id => bytes32) public sentinelOf;
    uint256 public emptyOpened;

    function openDraw(uint256 s, uint256 who, uint256 total) public {
        address coin = raffleCoins[s % 2];
        total = bound(total, 1, 1e24);
        vm.prank(keeper);
        try raffle.announceDraw(coin) returns (uint256 id) {
            // the list: one holder with every ticket, plus the sentinel committing the total and the snapshot time
            bytes32 seed = raffle.beacon().randomnessOf(raffle.draw(id).seedRound);   // a carried seed round may be proven
            if (seed == bytes32(0)) seed = keccak256(abi.encode("seed", id));
            bytes32 holderLeaf = raffle.leaf(actors[who % 3], 0, total);
            bytes32 sentinel = raffle.leaf(address(0), total, raffle.snapshotTimeOf(id, seed));
            sentinelOf[id] = sentinel;
            bytes32 root = holderLeaf < sentinel ? keccak256(abi.encodePacked(holderLeaf, sentinel)) : keccak256(abi.encodePacked(sentinel, holderLeaf));
            bytes32[] memory proof = new bytes32[](1);
            proof[0] = holderLeaf;
            vm.prank(keeper);
            try raffle.openDraw(id, root, total, "x", proof, abi.encodePacked(seed)) {} catch (bytes memory err) { _check(err); }
        } catch (bytes memory err) { _check(err); }
    }

    /// fixes-R3-carry: a snapshot with no eligible holder, opened as the sentinel alone; `requestDraw` closes it with no winner.
    function openEmptyDraw(uint256 s) public {
        address coin = raffleCoins[s % 2];
        vm.prank(keeper);
        try raffle.announceDraw(coin) returns (uint256 id) {
            bytes32 seed = raffle.beacon().randomnessOf(raffle.draw(id).seedRound);
            if (seed == bytes32(0)) seed = keccak256(abi.encode("seed", id));
            bytes32 sentinel = raffle.leaf(address(0), 0, raffle.snapshotTimeOf(id, seed));
            sentinelOf[id] = sentinel;
            vm.prank(keeper);
            try raffle.openDraw(id, sentinel, 0, "x", new bytes32[](0), abi.encodePacked(seed)) { emptyOpened++; } catch (bytes memory err) { _check(err); }
        } catch (bytes memory err) { _check(err); }
    }

    function _someDraw(uint256 i) internal view returns (bool, uint256) {
        uint256 n = raffle.drawCount();
        if (n == 0) return (false, 0);
        return (true, i % n);
    }

    function advanceDraw(uint256 i, uint8 action) public {
        (bool any, uint256 id) = _someDraw(i);
        if (!any) return;
        SweepVault.Draw memory d = raffle.draw(id);
        uint8 a = action % 4;
        if (a == 0) {
            vm.prank(owner);
            try raffle.cancelDraw(id) { cancels++; } catch (bytes memory err) { _check(err); }
        } else if (a == 1) {
            try raffle.requestDraw(id) {} catch (bytes memory err) { _check(err); }
        } else if (a == 2) {
            try raffle.resolve(id, abi.encodePacked(keccak256(abi.encode(id, d.round)))) {} catch (bytes memory err) { _check(err); }
        } else {
            try raffle.voidDraw(id) { voids++; } catch (bytes memory err) { _check(err); }
        }
    }

    function claim(uint256 i, uint256 who) public {
        (bool any, uint256 id) = _someDraw(i);
        if (!any) return;
        SweepVault.Draw memory d = raffle.draw(id);
        address holder = actors[who % 3];
        bytes32[] memory proof = new bytes32[](1);
        proof[0] = sentinelOf[id];
        try raffle.claim(id, holder, 0, d.totalTickets, proof) { claims++; } catch (bytes memory err) { _check(err); }
    }

    // ---- BURN, and the wrapper's tickets ----

    function retire() public {
        try burn.retire(burnCoin) returns (uint256 t) { burnTickets.push(t); retirements++; }
        catch (bytes memory err) { _check(err); }
    }

    function holderRedeem(uint256 who) public {
        address a = actors[who % 3];
        if (w.balanceOf(a) < 1e18) return;
        vm.prank(a);
        w.redeem(TAPROOT);
    }

    function resolveNext(bytes32 r) public {
        if (r == bytes32(0)) r = bytes32(uint256(1));
        uint256 id = w.nextToResolve();
        if (id >= w.ticketCount()) return;
        try w.resolve(id, abi.encodePacked(r)) {} catch (bytes memory err) { _check(err); }
    }

    function deliverOrRefund(uint256 i, bool deliver) public {
        uint256 n = w.ticketCount();
        if (n == 0) return;
        uint256 id = i % n;
        if (deliver) {
            vm.prank(custodian);
            try w.recordDelivery(id, keccak256(abi.encode("out", id))) { deliveries++; } catch (bytes memory err) { _check(err); }
        } else {
            if (w.ticket(id).stage == Wrapper.Stage.Resolved) {
                vm.prank(custodian);
                w.markUndeliverable(id);                 // the custodian's attestation (review2 R3 §4)
            }
            try w.refund(id) { refunds++; } catch (bytes memory err) { _check(err); }
        }
    }

    function reclaim(uint256 i) public {
        if (burnTickets.length == 0) return;
        try burn.reclaim(burnTickets[i % burnTickets.length]) { reclaims++; } catch (bytes memory err) { _check(err); }
    }

    // ---- time ----

    function passTime(uint32 secs, bool timeOnly) public {
        secs = uint32(bound(secs, 1, 8 days));
        if (timeOnly) Clock.skipTimeOnly(bound(secs, 1, 1 hours));
        else Clock.skip(secs);
    }
}

contract SweepInvariantTest is SweepBase {
    SweepHandler h;
    address[3] actors = [makeAddr("a0"), makeAddr("a1"), makeAddr("a2")];

    function setUp() public override {
        super.setUp();
        SweepHandler.Env memory e;
        e.w = wrapper; e.raffle = raffle; e.burn = burn; e.venue = venue; e.keeper = keeper; e.owner = owner;
        e.custodian = custodian; e.actors = actors; e.raffleCoins = [coinR, coinR2]; e.raffleRouters = [routerR, routerR2];
        e.burnCoin = coinX; e.burnRouter = routerX;
        _stockVenue(24);
        h = new SweepHandler(e, frogIds, frogLeaves);
        targetContract(address(h));
    }

    function _vaultBooks(SweepVault v, address[] memory coins) internal view {
        uint256 c; uint256 u;
        for (uint256 i = 0; i < coins.length; ++i) { c += v.credit(coins[i]); u += v.units(coins[i]); }
        assertEq(c, v.totalCredit(), "credit sums to its total");
        assertEq(u, v.totalUnits(), "units sum to their total");
        assertGe(wrapper.balanceOf(address(v)), v.totalUnits() + v.totalReserved(), "wFROG covers every coin's units and every reservation");
        assertEq(address(v).balance, v.totalCredit(), "ETH is exactly the unspent credit");
        assertEq(h.ethIn(address(v)) - h.ethSpent(address(v)), v.totalCredit(), "ETH out only by sweeps");
        assertLe(h.ethSpent(address(v)), h.capTimesBought(address(v)), "spent <= cap x bought, summed");
    }

    function invariant_theWrappersLedgerHoldsWithRetirements() public view {
        assertEq(wrapper.totalSupply(), (wrapper.held() - wrapper.openTickets()) * ONE, "supply == (held - open) x ONE");
    }

    function invariant_theVaultsBooksBalance() public view {
        address[] memory rc = new address[](2);
        rc[0] = coinR; rc[1] = coinR2;
        address[] memory bc = new address[](1);
        bc[0] = coinX;
        _vaultBooks(raffle, rc);
        _vaultBooks(burn, bc);
        assertEq(burn.totalReserved(), 0, "the burn vault never reserves");
    }

    function invariant_reservationsAreExactlyTheLiveDraws() public view {
        uint256 live;
        for (uint256 i = 0; i < raffle.drawCount(); ++i) {
            SweepVault.Stage s = raffle.draw(i).stage;
            if (s == SweepVault.Stage.Announced || s == SweepVault.Stage.Open || s == SweepVault.Stage.Pinned || s == SweepVault.Stage.Resolved) live++;
        }
        assertEq(raffle.totalReserved(), live * ONE);
    }

    function invariant_thePolicyNeverChanges() public view {
        assertEq(uint8(raffle.policy()), uint8(SweepVault.Policy.RAFFLE));
        assertEq(uint8(burn.policy()), uint8(SweepVault.Policy.BURN));
        assertEq(raffle.venue(), address(venue));
    }

    function invariant_noVaultEverApprovedAnyone() public view {
        address[5] memory who = [address(venue), keeper, owner, custodian, stranger];
        for (uint256 i = 0; i < who.length; ++i) {
            assertEq(wrapper.allowance(address(raffle), who[i]), 0);
            assertEq(wrapper.allowance(address(burn), who[i]), 0);
        }
    }

    function invariant_noPanics() public view {
        assertEq(h.panics(), 0);
    }

}

/// The handler's paths do reach the states the invariants are about: a sweep, a draw claimed, a retirement delivered, one
/// refunded and reclaimed. (Per-run coverage is not asserted in the campaign: a 64-call run can by chance never pick one.)
contract SweepHandlerReachesTest is SweepInvariantTest {
    function test_theHandlerReachesEveryState() public {
        h.sweep(0, 0.1 ether, 0.05 ether, true);                // raffle coinR: 2 wFROG
        h.sweep(2, 0.1 ether, 0.05 ether, true);                // burn coinX: 2 wFROG
        assertEq(h.sweeps(), 2);
        h.openDraw(0, 0, 10);
        h.passTime(15 minutes, false);
        h.advanceDraw(0, 1);                                    // request
        h.advanceDraw(0, 2);                                    // resolve
        h.claim(0, 0);
        assertEq(h.claims(), 1);
        h.openEmptyDraw(0);                                     // fixes-R3-carry: an empty snapshot, closed with no winner
        assertEq(h.emptyOpened(), 1);
        h.passTime(15 minutes, false);
        h.advanceDraw(1, 1);
        assertEq(uint8(raffle.draw(1).stage), uint8(SweepVault.Stage.Empty));
        h.retire();
        h.retire();
        assertEq(h.retirements(), 2);
        h.resolveNext(bytes32(uint256(1)));
        h.resolveNext(bytes32(uint256(2)));
        h.deliverOrRefund(0, true);
        h.passTime(7 days, false);
        h.passTime(3 days, false);                              // the refund grace
        h.deliverOrRefund(1, false);
        h.reclaim(1);
        assertEq(h.deliveries(), 1);
        assertEq(h.refunds(), 1);
        assertEq(h.reclaims(), 1);
        assertEq(h.panics(), 0);
        invariant_theWrappersLedgerHoldsWithRetirements();
        invariant_theVaultsBooksBalance();
        invariant_reservationsAreExactlyTheLiveDraws();
    }
}
