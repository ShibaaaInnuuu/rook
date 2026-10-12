// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {YoyoBase, MockYoyoVenue} from "./YoyoVault.t.sol";
import {SweepVault} from "../src/SweepVault.sol";
import {Wrapper} from "../src/Wrapper.sol";
import {CurveToken} from "../src/curve/CurveToken.sol";

interface IYoyoHooks {
    function sweepOne() external;
    function depositToVault() external;
}

/// Sweeps, relists, resolves and lists, sells, buys back, delists, refunds — in any order — and keeps the ghosts the invariants
/// read: what the coin ever bought, what was ever sold, what was ever burned back.
contract YoyoHandler is Test {
    SweepVault immutable v;
    Wrapper immutable w;
    CurveToken immutable coin;
    IYoyoHooks immutable base;
    address immutable custodian;
    address immutable keeper;
    uint256 constant ONE = 1e18;
    uint256 constant COIN_PRICE = 0.0001 ether;

    uint256 public everBought;      // wFROG units the coin's sweeps bought
    uint256 public soldCount;       // listings sold
    uint256 public returnedCount;   // listings returned or refunded
    uint256 public deadBefore;
    uint256 public burnedBack;      // coin sent to DEAD by buybacks, as measured
    uint256[] public open;          // relist tickets not yet closed
    uint256 public calls;

    constructor(SweepVault v_, Wrapper w_, CurveToken c, IYoyoHooks b, address cu, address k) {
        v = v_; w = w_; coin = c; base = b; custodian = cu; keeper = k;
        deadBefore = c.balanceOf(0x000000000000000000000000000000000000dEaD);
    }

    function sweep() external {
        calls++;
        base.sweepOne();
        everBought += ONE;
    }

    function relist() external {
        calls++;
        if (v.units(address(coin)) < ONE || !v.capFresh()) return;
        uint256 id = v.relist(address(coin));
        open.push(id);
    }

    function resolveAndList(uint256 i) external {
        calls++;
        if (open.length == 0) return;
        uint256 id = open[bound(i, 0, open.length - 1)];
        if (w.ticket(id).stage != Wrapper.Stage.Pinned) return;
        if (w.nextToResolve() != id) return;
        try w.resolve(id, abi.encodePacked(bytes32(uint256(i) | 1))) {} catch { return; }
        vm.prank(custodian);
        w.recordDelivery(id, keccak256(abi.encode("listing", id)));
    }

    function sell(uint256 i, uint256 over) external {
        calls++;
        if (open.length == 0) return;
        uint256 k = bound(i, 0, open.length - 1);
        uint256 id = open[k];
        SweepVault.Listing memory l = v.listing(id);
        if (l.stage != SweepVault.ListingStage.Listed) return;
        uint256 amount = l.ask * 95 / 100 + bound(over, 0, l.ask / 2);
        vm.deal(custodian, custodian.balance + amount);
        vm.prank(custodian);
        v.recordSale{value: amount}(id);
        soldCount++;
        _close(k);
    }

    function buyback(uint256 spend) external {
        calls++;
        uint256 p = v.proceeds(address(coin));
        if (p == 0) return;
        spend = bound(spend, 1, p);
        vm.prank(keeper);
        (uint256 spent, uint256 burned) = v.buyback(address(coin), spend, abi.encodeCall(MockYoyoVenue.buyCoin, (COIN_PRICE)), 1);
        burnedBack += burned;
        spent;
    }

    function delist(uint256 i) external {
        calls++;
        if (open.length == 0) return;
        uint256 k = bound(i, 0, open.length - 1);
        uint256 id = open[k];
        if (v.listing(id).stage != SweepVault.ListingStage.Listed) return;
        if (w.ticket(id).stage != Wrapper.Stage.Delivered) return;
        base.depositToVault();
        vm.prank(custodian);
        v.reclaimListing(id);
        returnedCount++;
        _close(k);
    }

    function refund(uint256 i) external {
        calls++;
        if (open.length == 0) return;
        uint256 k = bound(i, 0, open.length - 1);
        uint256 id = open[k];
        if (w.ticket(id).stage != Wrapper.Stage.Resolved) return;
        vm.warp(vm.getBlockTimestamp() + 7 days + 1);
        vm.roll(vm.getBlockNumber() + 7 days / 12 + 2);
        vm.prank(w.custodian());
        w.markUndeliverable(id);                             // the custodian's attestation (review2 R3 §4)
        w.refund(id);
        v.reclaim(id);
        returnedCount++;
        _close(k);
    }

    function _close(uint256 k) private {
        open[k] = open[open.length - 1];
        open.pop();
    }

    function openCount() external view returns (uint256) { return open.length; }
}

/// forge-config: default.invariant.fail-on-revert = true
contract YoyoInvariantTest is YoyoBase {
    YoyoHandler handler;

    function setUp() public override {
        super.setUp();
        for (uint256 i = 0; i < 40; ++i) _deposit(address(yoyoVenue));     // plenty for the venue to sell
        handler = new YoyoHandler(yoyo, wrapper, coinY, IYoyoHooks(address(this)), custodian, keeper);
        targetContract(address(handler));
    }

    function sweepOne() external { _sweepY(1); }
    function depositToVault() external { _deposit(address(yoyo)); }

    /// No exit but the burn: the vault holds every wei of credit, desk escrow and proceeds.
    function invariant_noExitButTheBurn() public view {
        assertGe(address(yoyo).balance, yoyo.totalCredit() + yoyo.atDesk() + yoyo.totalProceeds());
        assertEq(coinY.balanceOf(address(yoyo)), 0, "the vault keeps no coin");
        assertEq(coinY.balanceOf(DEAD) - handler.deadBefore(), handler.burnedBack(), "every coin bought back is at DEAD");
    }

    /// Each unit the coin bought is in exactly one place: held, listed, sold, or returned to it.
    function invariant_everyUnitIsInOnePlace() public view {
        address c = address(coinY);
        assertEq(yoyo.units(c) + (yoyo.listedOpen(c) + handler.soldCount()) * ONE, handler.everBought(),
            "held + open listings + sold == bought (returned ones are held again)");
        assertEq(yoyo.listedOpen(c), handler.openCount());
    }

    /// The markup stays within its bounds; every ask honoured it.
    function invariant_theMarkupIsBounded() public view {
        assertGe(yoyo.markupBps(), yoyo.MIN_MARKUP_BPS());
        assertLe(yoyo.markupBps(), yoyo.MAX_MARKUP_BPS());
    }

    function invariant_theWrapperLedgerHolds() public view {
        _assertLedger();
    }
}
