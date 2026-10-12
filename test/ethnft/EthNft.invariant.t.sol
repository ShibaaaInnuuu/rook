// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {EthNftBase} from "./EthNftBase.sol";
import {EthNftWrapper} from "../../src/nft/EthNftWrapper.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// Everyone acts at random on the Ethereum collection's 4663 side: holders mint and burn at the module, redeem, resolve and refund;
/// the canister advances, mints the bought unit to the module and settles (or returns), notes deliveries sent (D3-H-1), records
/// deliveries and marks some undeliverable; anyone tries the late-window refund; the poster posts; time passes on both clocks.
/// docs/v7/phase1-nft.md §3.
contract EthNftHandler is Test {
    EthNftWrapper w;
    FloorModule m;
    address custodian;
    address[3] holders;
    uint256 nextToken = 1;
    uint256 constant ONE = 1e18;
    uint256 constant PBS = 12;
    uint256 public settles;
    uint256 public deliveries;
    uint256 public refunds;
    uint256 public notedSent;
    uint256 public lateRefunds;
    /// D3-H-1: refunds of a ticket noted sent and not marked undeliverable. Must stay zero.
    uint256 public refundsOfNotedSent;

    constructor(EthNftWrapper w_, FloorModule m_, address custodian_) {
        w = w_;
        m = m_;
        custodian = custodian_;
        holders = [makeAddr("h0"), makeAddr("h1"), makeAddr("h2")];
    }

    function _h(uint256 i) internal view returns (address) { return holders[i % 3]; }

    function wait(uint256 secs) external {
        secs = bound(secs, 1, 3 days);
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
        uint256 a = m.accepted();
        if (a != 0) {
            vm.prank(custodian);
            m.postFloor(a, 10_000, 50);
        }
    }

    function mint(uint256 who, uint256 eth) external {
        eth = bound(eth, 0.01 ether, 3 ether);
        address a = _h(who);
        vm.deal(a, a.balance + eth);
        vm.prank(a);
        try m.mint{value: eth}(a, 0) {} catch {}
    }

    function burn(uint256 who, uint256 units) external {
        address a = _h(who);
        uint256 bal = w.balanceOf(a);
        if (bal == 0) return;
        units = bound(units, 1, bal);
        vm.startPrank(a);
        w.approve(address(m), units);
        try m.burn(units, 0, a) {} catch {}
        vm.stopPrank();
    }

    function advance() external {
        uint256 max = m.maxItemCost();
        if (max == 0 || m.transitUnits() != 0) return;
        vm.prank(custodian);
        try m.advance(max) {} catch {}
    }

    function settle(uint256 costBps) external {
        if (m.transitUnits() == 0) return;
        uint256 adv = m.advanced();
        uint256 cost = adv * bound(costBps, 9_000, 10_000) / 10_000;
        if (cost == 0) return;
        vm.startPrank(custodian);
        try w.mint(nextToken, keccak256(abi.encode(nextToken)), address(m)) { nextToken++; } catch { vm.stopPrank(); return; }
        m.settleAdvance(cost);
        vm.stopPrank();
        settles++;
    }

    function giveBack(uint256 bps) external {
        if (m.transitUnits() == 0) return;
        uint256 v = m.advanced() * bound(bps, 9_000, 10_000) / 10_000;
        vm.deal(custodian, custodian.balance + v);
        vm.prank(custodian);
        m.returnAdvance{value: v}();
    }

    function redeem(uint256 who) external {
        address a = _h(who);
        if (w.balanceOf(a) < ONE) return;
        vm.prank(a);
        try w.redeem(a) {} catch {}
    }

    function resolve(bytes32 r) external {
        uint256 n = w.nextToResolve();
        if (n >= w.ticketCount() || r == bytes32(0)) return;
        try w.resolve(n, abi.encodePacked(r)) {} catch {}
    }

    function deliver(uint256 pick) external {
        uint256 c = w.ticketCount();
        if (c == 0) return;
        uint256 id = pick % c;
        if (w.ticket(id).stage != EthNftWrapper.Stage.Resolved) return;
        vm.prank(custodian);
        w.recordDelivery(id, keccak256(abi.encode("d", id)));
        deliveries++;
    }

    /// The canister notes its delivery transaction mined, right after it transfers (before finality).
    function noteSent(uint256 pick) external {
        uint256 c = w.ticketCount();
        if (c == 0) return;
        uint256 id = pick % c;
        if (w.ticket(id).stage != EthNftWrapper.Stage.Resolved) return;
        vm.prank(custodian);
        w.noteSent(id, keccak256(abi.encode("sent", id)));
        notedSent++;
    }

    /// Anyone, without the canister's attestation: the late window.
    function lateRefund(uint256 pick) external {
        uint256 c = w.ticketCount();
        if (c == 0) return;
        uint256 id = pick % c;
        bool guarded = w.ticket(id).stage == EthNftWrapper.Stage.Resolved && w.sentTx(id) != bytes32(0) && !w.undeliverable(id);
        try w.refund(id) {
            refunds++;
            lateRefunds++;
            if (guarded) refundsOfNotedSent++;
        } catch {}
    }

    function markAndRefund(uint256 pick) external {
        uint256 c = w.ticketCount();
        if (c == 0) return;
        uint256 id = pick % c;
        if (w.ticket(id).stage == EthNftWrapper.Stage.Resolved) {
            vm.prank(custodian);
            w.markUndeliverable(id);
        }
        try w.refund(id) { refunds++; } catch {}
    }
}

contract EthNftInvariantTest is EthNftBase {
    EthNftHandler handler;

    function setUp() public override {
        reserveBps = 2_500;
        super.setUp();
        handler = new EthNftHandler(w, module, custodian);
        targetContract(address(handler));
    }

    /// (W1) the wrapper's ledger with ETH-backed units; (M) the module holds every wei it owes and its tiers add up.
    function invariant_booksHold() public view {
        _books();
    }

    /// A redemption always has an item: never more open tickets than items held.
    function invariant_everyOpenTicketHasAnItem() public view {
        assertLe(w.openTickets(), w.held());
        assertEq(w.held(), w.itemCount() - handler.deliveries(), "held == items minted - delivered");
    }

    /// D3-H-1: a delivery the canister noted sent is never refunded without `markUndeliverable`.
    function invariant_aNotedDeliveryIsNeverRefundedWithoutTheMark() public view {
        assertEq(handler.refundsOfNotedSent(), 0);
    }

    function test_aShortSequenceOfEveryAction() public {
        handler.mint(0, 3 ether);
        handler.advance();
        handler.settle(9_950);
        handler.redeem(0);
        handler.resolve(bytes32(uint256(7)));
        handler.noteSent(0);
        handler.deliver(0);
        handler.mint(1, 2 ether);
        handler.wait(1 hours);
        handler.advance();
        handler.giveBack(9_500);
        assertEq(handler.settles(), 1);
        assertEq(handler.deliveries(), 1);
        assertEq(handler.notedSent(), 1);
        _books();
    }

    /// The handler's guard is live: a noted ticket past the late window is refused, an unnoted one refunds.
    function test_lateRefundSkipsANotedTicket_andRefundsAnUnnotedOne() public {
        handler.mint(0, 3 ether);
        handler.advance();
        handler.settle(9_950);
        handler.mint(0, 3 ether);
        handler.advance();
        handler.settle(9_950);
        handler.redeem(0);
        handler.redeem(0);
        handler.resolve(bytes32(uint256(7)));
        handler.resolve(bytes32(uint256(8)));
        handler.noteSent(0);
        for (uint256 i = 0; i < 14; ++i) handler.wait(3 days);
        handler.lateRefund(0);
        assertEq(handler.lateRefunds(), 0, "the noted ticket is not refunded");
        handler.lateRefund(1);
        assertEq(handler.lateRefunds(), 1, "the unnoted one is");
        assertEq(handler.refundsOfNotedSent(), 0);
        _books();
    }
}
