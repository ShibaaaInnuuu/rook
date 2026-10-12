// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {QueueBase, MockCurve, QueueCapSource} from "./utils/QueueBase.sol";
import {Merkle} from "./utils/Merkle.sol";
import {Clock} from "./utils/Clock.sol";
import {ItemQueue} from "../src/curve/ItemQueue.sol";
import {Wrapper} from "../src/Wrapper.sol";
import {SweepVault} from "../src/SweepVault.sol";

/// Coins queue, the canister buys items at any cost it may, filled coins release, deadlines pass and refunds cancel — in any
/// order, on both clocks.
contract QueueHandler is Test {
    ItemQueue public q;
    QueueCapSource src;
    Wrapper w;
    SweepVault capVault;
    address custodian; address keeper;
    string[] frogIds; bytes32[] frogLeaves;
    uint256 nextFrog; uint256 nextTx;
    MockCurve[] public curves;
    uint256 public panics;
    uint256 public ghostIn;
    uint256 public ghostPaid;

    constructor(ItemQueue q_, QueueCapSource src_, Wrapper w_, SweepVault v_, address custodian_, address keeper_, string[] memory ids,
        bytes32[] memory leaves)
    {
        q = q_; src = src_; w = w_; capVault = v_; custodian = custodian_; keeper = keeper_;
        for (uint256 i = 0; i < ids.length; ++i) { frogIds.push(ids[i]); frogLeaves.push(leaves[i]); }
    }

    function _check(bytes memory err) internal {
        if (err.length >= 4 && bytes4(err) == bytes4(0x4e487b71)) panics++;
    }

    function count() external view returns (uint256) { return curves.length; }

    function enqueue(uint96 budget) public {
        if (curves.length >= 12) return;
        uint256 b = bound(budget, 0.05 ether, 8 ether);
        MockCurve c = new MockCurve(q);
        src.set(address(c), address(c));
        vm.deal(address(this), b);
        try c.enqueue{value: b}(uint64(block.timestamp + 3 days), uint64(block.number + 3 days / 12)) {
            curves.push(c);
            ghostIn += b;
        } catch (bytes memory err) { _check(err); }
    }

    function buyItem(uint96 cost) public {
        if (nextFrog >= frogIds.length) return;
        if (!capVault.capFresh()) { vm.prank(keeper); capVault.postCap(2 ether); }
        uint256 c = bound(cost, 0.01 ether, 2 ether);
        if (c > q.queued()) return;
        uint256 i = nextFrog++;
        bytes32[] memory proof = Merkle.proof(frogLeaves, i);
        vm.prank(custodian);
        w.wrapDeposit(frogIds[i], proof, keccak256(abi.encode("queue frog", nextTx++)), 0, address(q));
        vm.prank(custodian);
        try q.recordItem(c) { ghostPaid += c; } catch (bytes memory err) { _check(err); }
    }

    function release(uint256 which) public {
        if (curves.length == 0) return;
        MockCurve c = curves[which % curves.length];
        if (!q.isFilled(address(c))) return;
        try c.release() {} catch (bytes memory err) { _check(err); }
    }

    function refundExpired() public {
        try q.refundExpired() {} catch (bytes memory err) { _check(err); }
    }

    function passTime(uint32 secs) public {
        Clock.skip(bound(secs, 1, 2 days));
    }
}

contract ItemQueueInvariantTest is QueueBase {
    QueueHandler h;

    function setUp() public override {
        super.setUp();
        _postCap(raffle, 2 ether);
        h = new QueueHandler(queue, src, wrapper, raffle, custodian, keeper, frogIds, frogLeaves);
        targetContract(address(h));
    }

    function invariant_ethInIsSpentQueuedHeldReturnedOrReleased() public view {
        _books();
        assertEq(queue.totalIn(), h.ghostIn());
        assertEq(queue.totalSpent(), h.ghostPaid(), "the canister was paid exactly what was recorded");
    }

    function invariant_unitsAssignedAreExactlyItemsBought() public view {
        uint256 assigned;
        for (uint256 i = 0; i < h.count(); ++i) assigned += queue.entry(address(h.curves(i))).units;
        assertEq(assigned, queue.itemsRecorded() * 1e18);
    }

    function invariant_fifoNoCoinIsStarvedByALaterOne() public view {
        bool open;
        for (uint256 i = 0; i < h.count(); ++i) {
            ItemQueue.Entry memory e = queue.entry(address(h.curves(i)));
            if (e.spent != 0 && e.status != ItemQueue.Status.Cancelled) assertFalse(open, "a later coin spent before an earlier one");
            if (e.status == ItemQueue.Status.Queued) open = true;
        }
        uint256 hd = queue.head();
        for (uint256 i = 0; i < hd && i < h.count(); ++i) {
            assertTrue(queue.entry(address(h.curves(i))).status != ItemQueue.Status.Queued, "nothing queued behind the head");
        }
    }

    function invariant_everyCoinsEthIsAccountedFor() public view {
        for (uint256 i = 0; i < h.count(); ++i) {
            ItemQueue.Entry memory e = queue.entry(address(h.curves(i)));
            uint256 out = (e.status == ItemQueue.Status.Cancelled || e.status == ItemQueue.Status.Released) ? e.deposited - e.spent : 0;
            assertEq(e.spent + e.remaining + out, e.deposited);
            if (e.status == ItemQueue.Status.Filled) assertLe(e.remaining, queue.tailOf(e.deposited));
        }
    }

    function invariant_noPanics() public view {
        assertEq(h.panics(), 0);
    }
}
