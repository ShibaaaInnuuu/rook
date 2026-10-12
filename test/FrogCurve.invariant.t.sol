// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {CurveBase} from "./utils/CurveBase.sol";
import {Merkle} from "./utils/Merkle.sol";
import {Clock} from "./utils/Clock.sol";
import {SweepVault} from "../src/SweepVault.sol";
import {FrogCurve} from "../src/curve/FrogCurve.sol";
import {CurveToken} from "../src/curve/CurveToken.sol";
import {Wrapper} from "../src/Wrapper.sol";
import {ItemQueue} from "../src/curve/ItemQueue.sol";
import {SweepVault as SV} from "../src/SweepVault.sol";

/// Buyers and sellers in any order, the canister buying frogs for the queue, graduation or refunds, fees flushed and claimed, and time on
/// both clocks — against one curve.
contract CurveHandler is Test {
    FrogCurve public c;
    CurveToken public tok;
    Wrapper public w;
    ItemQueue public q;
    SV public capVault;
    address keeper;
    address custodian; address creator;
    address[3] actors;
    string[] frogIds; bytes32[] frogLeaves;
    uint256 nextFrog; uint256 nextTx;
    uint256 public panics;
    uint256 public kLast;
    uint256 public kDropped;
    uint256 public priceAboveEnd;

    constructor(FrogCurve c_, Wrapper w_, ItemQueue q_, SV capVault_, address keeper_, address custodian_, address creator_,
        address[3] memory actors_, string[] memory ids, bytes32[] memory leaves)
    {
        c = c_; tok = CurveToken(address(c_.token())); w = w_; custodian = custodian_; creator = creator_; actors = actors_;
        q = q_; capVault = capVault_; keeper = keeper_;
        for (uint256 i = 0; i < ids.length; ++i) { frogIds.push(ids[i]); frogLeaves.push(leaves[i]); }
        kLast = c_.v() * c_.t();
    }

    function _check(bytes memory err) internal {
        if (err.length >= 4 && bytes4(err) == bytes4(0x4e487b71)) panics++;
    }

    /// k never falls on a trade — except the last buy, which takes exactly what is left on the curve (tEnd) rather than what
    /// the formula would round to; that can differ by base units, and it is the one trade that must end exactly at tEnd.
    function _afterTrade() internal {
        uint256 k = c.v() * c.t();
        if (k < kLast && c.state() == FrogCurve.State.Trading) kDropped++;
        kLast = k;
        if (c.price() > (c.vEnd() * 1e18) / c.tEnd() + 1) priceAboveEnd++;
    }

    function buy(uint256 who, uint96 eth) public {
        eth = uint96(bound(eth, 1e9, 3 ether));
        address a = actors[who % 3];
        vm.deal(a, a.balance + eth);
        vm.prank(a);
        try c.buy{value: eth}(0) {} catch (bytes memory err) { _check(err); }
        _afterTrade();
    }

    function sell(uint256 who, uint256 frac) public {
        address a = actors[who % 3];
        uint256 bal = tok.balanceOf(a);
        if (bal == 0) return;
        uint256 amount = bound(frac, 1, bal);
        vm.startPrank(a);
        tok.approve(address(c), amount);
        try c.sell(amount, 0) {} catch (bytes memory err) { _check(err); }
        vm.stopPrank();
        _afterTrade();
    }

    function buyFrog(uint96 cost) public {
        if (c.state() != FrogCurve.State.Graduating || q.isFilled(address(tok))) return;
        cost = uint96(bound(cost, 0.5 ether, 0.646 ether));
        if (cost > q.queued()) return;
        uint256 i = nextFrog++;
        if (i >= frogIds.length) return;
        bytes32[] memory proof = Merkle.proof(frogLeaves, i);
        vm.prank(custodian);
        w.wrapDeposit(frogIds[i], proof, keccak256(abi.encode("curve frog", nextTx++)), 0, address(q));
        if (!capVault.capFresh()) { vm.prank(keeper); capVault.postCap(0.646 ether); }
        vm.prank(custodian);
        try q.recordItem(cost) {} catch (bytes memory err) { _check(err); }
    }

    function graduate() public {
        try c.graduate() {} catch (bytes memory err) { _check(err); }
    }

    function openRefunds() public {
        try c.openRefunds() {} catch (bytes memory err) { _check(err); }
    }

    function refund(uint256 who, uint256 frac) public {
        address a = actors[who % 3];
        uint256 bal = tok.balanceOf(a);
        if (bal == 0 || c.state() != FrogCurve.State.Refunding) return;
        uint256 amount = bound(frac, 1, bal);
        vm.startPrank(a);
        tok.approve(address(c), amount);
        try c.refund(amount) {} catch (bytes memory err) { _check(err); }
        vm.stopPrank();
    }

    function fees(uint8 which) public {
        if (which % 3 == 0) { try c.flushVaultFees() {} catch (bytes memory err) { _check(err); } }
        else if (which % 3 == 1) { vm.prank(creator); try c.claimCreatorFees() {} catch (bytes memory err) { _check(err); } }
        else { try c.claimPlatformFees() {} catch (bytes memory err) { _check(err); } }
    }

    function passTime(uint32 secs) public {
        Clock.skip(bound(secs, 1, 2 days));
    }
}

contract FrogCurveInvariantTest is CurveBase {
    FrogCurve c;
    CurveToken tok;
    CurveHandler h;
    address[3] actors = [makeAddr("b0"), makeAddr("b1"), makeAddr("b2")];

    function setUp() public override {
        super.setUp();
        (c, tok) = _launch(SweepVault.Policy.RAFFLE);
        h = new CurveHandler(c, wrapper, queue, curveRaffle, keeper, custodian, creator, actors, frogIds, frogLeaves);
        targetContract(address(h));
    }

    function invariant_ethIsExactlyTheRaisePlusTheFeesOwed() public view {
        _curveBooks(c);
    }

    function invariant_theReserveMatchesTheCurveWhileTrading() public view {
        if (c.state() == FrogCurve.State.Trading) assertEq(c.reserveEth(), c.v() - c.v0());
        if (c.state() == FrogCurve.State.Graduating) assertEq(c.reserveEth() + c.itemBudget(), c.v() - c.v0(), "the rest is queued");
        assertLe(c.v(), c.vEnd());
        assertGe(c.t(), c.tEnd());
    }

    function invariant_everyCoinIsAccountedFor() public view {
        uint256 held;
        for (uint256 i = 0; i < 3; ++i) held += tok.balanceOf(actors[i]);
        held += tok.balanceOf(address(c)) + tok.balanceOf(address(seeder));
        assertEq(held, SUPPLY);
        if (c.state() == FrogCurve.State.Trading || c.state() == FrogCurve.State.Graduating) {
            assertEq(tok.balanceOf(address(c)), c.reserved() + (c.t() - c.tEnd()), "the curve holds the reserve and what is unsold");
        }
    }

    function invariant_theCurveOnlyEverGetsMoreExpensiveToDrain() public view {
        assertEq(h.kDropped(), 0, "k never falls: every rounding is the curve's");
        assertEq(h.priceAboveEnd(), 0, "the price never passes the pools' opening price");
    }

    function invariant_theQueueKeepsItsBooks() public view {
        _queueBooks();
        ItemQueue.Entry memory e = queue.entry(address(tok));
        assertEq(e.spent + e.remaining + (e.status == ItemQueue.Status.Cancelled || e.status == ItemQueue.Status.Released
            ? e.deposited - e.spent : 0), e.deposited, "a coin's ETH is spent, waiting, or back with its curve");
        assertEq(queue.itemsRecorded() * ONE, queue.unitsHeld() + (e.status == ItemQueue.Status.Cancelled
            || e.status == ItemQueue.Status.Released ? e.units : 0), "units assigned == items x 1e18");
    }

    function invariant_refundsNeverPayMoreThanThePools() public view {
        if (c.state() != FrogCurve.State.Refunding) return;
        assertLe(wrapper.balanceOf(address(c)), c.refundWrappedPool());
        assertGe(wrapper.balanceOf(address(c)) + 3, (c.refundWrappedPool() * (c.forSale() - c.returned())) / c.forSale());
    }

    function invariant_theWrappersLedgerHolds() public view {
        assertEq(wrapper.totalSupply(), (wrapper.held() - wrapper.openTickets()) * ONE);
    }

    function invariant_noPanics() public view {
        assertEq(h.panics(), 0);
    }
}
