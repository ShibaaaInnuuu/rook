// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Clock} from "../utils/Clock.sol";
import {HomeGuard} from "../../src/homechain/HomeGuard.sol";

/// docs/v6/home-chain-vault.md §6: caps at stage 0, ×4 raises with an interval and the bank-run check, a 24-hour outbound window on
/// both clocks, and the tripwire.
contract HomeGuardTest is Test {
    HomeGuard g;
    address timelock = makeAddr("timelock");
    address vault = makeAddr("vault");
    address wrapper = makeAddr("wrapper");
    address other = makeAddr("other vault");
    address bayc = makeAddr("BAYC");
    address punks = makeAddr("Punks");

    function setUp() public {
        vm.warp(1_790_000_000);
        vm.roll(26_000_000);
        g = new HomeGuard(timelock, 7 days, 12);
        vm.startPrank(timelock);
        g.register(bayc, vault);
        g.register(bayc, wrapper);
        g.register(punks, other);
        vm.stopPrank();
    }

    function test_stage0() public view {
        HomeGuard.Cap memory c = g.capOf(bayc);
        assertEq(c.maxItems, 3);
        assertEq(c.maxValueWei, 10 ether);
        assertEq(c.maxItemWei, 10 ether);
        (,uint256 gi, uint256 gv,,,,,) = g.global();
        assertEq(gi, 12);
        assertEq(gv, 10 ether);
        (uint256 pc, uint256 gl, uint256 gw,,) = g.limits();
        assertEq(pc, 6);
        assertEq(gl, 12);
        assertEq(gw, 10 ether);
    }

    function test_onlyRegisteredCallersCount() public {
        vm.expectRevert(HomeGuard.NotRegistered.selector);
        g.admit(bayc, 1, 1 ether);
        vm.prank(other);
        vm.expectRevert(HomeGuard.NotRegistered.selector);
        g.admit(bayc, 1, 1 ether);            // registered for punks, not for BAYC
        vm.prank(vault);
        g.admit(bayc, 1, 1 ether);
        vm.expectRevert(HomeGuard.NotOwner.selector);
        g.register(bayc, makeAddr("x"));
        vm.prank(timelock);
        vm.expectRevert(HomeGuard.AlreadyRegistered.selector);
        g.register(punks, vault);
    }

    function test_capsBindAtTheirEdge() public {
        vm.startPrank(vault);
        g.admit(bayc, 1, 4 ether);
        g.admit(bayc, 1, 4 ether);
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.OverCap.selector, "collection value", 12 ether, 10 ether));
        g.admit(bayc, 1, 4 ether);
        g.admit(bayc, 1, 2 ether);            // exactly 10
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.OverCap.selector, "collection items", 4, 3));
        g.admit(bayc, 1, 0);
        g.release(bayc, 1, 4 ether);
        g.admit(bayc, 1, 0);
        vm.stopPrank();
        // the global value cap binds across collections
        vm.prank(other);
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.OverCap.selector, "global value", 11 ether, 10 ether));
        g.admit(punks, 1, 5 ether);
        // per-item price, with room under the value caps so only the item price binds
        vm.startPrank(timelock);
        g.raise(punks, 3, 40 ether, 10 ether);
        g.raiseGlobal(12, 40 ether);
        vm.stopPrank();
        vm.prank(other);
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.OverCap.selector, "item price", 11 ether, 10 ether));
        g.admit(punks, 1, 11 ether);
    }

    function test_releaseClamps() public {
        vm.prank(vault);
        g.release(bayc, 5, 100 ether);
        HomeGuard.Cap memory c = g.capOf(bayc);
        assertEq(c.items, 0);
        assertEq(c.valueWei, 0);
    }

    function test_raiseAtMostTimesFourAndOncePerInterval() public {
        vm.startPrank(timelock);
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.RaiseTooLarge.selector, 13, 12));
        g.raise(bayc, 13, 40 ether, 10 ether);
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.RaiseTooLarge.selector, 41 ether, 40 ether));
        g.raise(bayc, 12, 41 ether, 10 ether);
        g.raise(bayc, 12, 40 ether, 10 ether);
        vm.expectRevert(HomeGuard.RaiseTooSoon.selector);
        g.raise(bayc, 13, 40 ether, 10 ether);
        // lowering is always fine
        g.raise(bayc, 6, 20 ether, 5 ether);
        Clock.skipTimeOnly(3 days);
        vm.expectRevert(HomeGuard.RaiseTooSoon.selector);
        g.raise(bayc, 12, 40 ether, 10 ether);
        Clock.skip(1);
        vm.roll(block.number + 3 days / 12);
        g.raise(bayc, 12, 40 ether, 10 ether);
        // per-item above value
        vm.expectRevert(HomeGuard.BadInput.selector);
        g.raise(bayc, 12, 40 ether, 41 ether);
        vm.stopPrank();
        vm.expectRevert(HomeGuard.NotOwner.selector);
        g.raise(bayc, 1, 1, 1);
    }

    function test_raiseGlobalAndLimits() public {
        vm.startPrank(timelock);
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.RaiseTooLarge.selector, 49, 48));
        g.raiseGlobal(49, 40 ether);
        g.raiseGlobal(48, 40 ether);
        vm.expectRevert(HomeGuard.RaiseTooSoon.selector);
        g.raiseGlobal(49, 40 ether);
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.RaiseTooLarge.selector, 25, 24));
        g.setLimits(25, 48, 40 ether);
        g.setLimits(24, 48, 40 ether);
        vm.expectRevert(HomeGuard.BadInput.selector);
        g.setLimits(24, 23, 40 ether);
        vm.stopPrank();
    }

    /// H3's bank-run constraint: a collection's whole cap must be able to leave within the timelock's delay less a day.
    function test_bankRunConstraint() public {
        vm.startPrank(timelock);
        g.setLimits(1, 12, 10 ether);          // 1 item a day: 6 days × 1 = 6 items at most
        g.raise(bayc, 6, 10 ether, 10 ether);
        Clock.skip(3 days + 1);
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.BankRun.selector, 7, 1, 7 days));
        g.raise(bayc, 7, 10 ether, 10 ether);
        vm.stopPrank();
        vm.expectRevert(HomeGuard.BadInput.selector);
        new HomeGuard(timelock, 1 hours, 12);
    }

    /// Six items spread over ten hours (no rush, no trip); the seventh waits until the first leaves the window on BOTH clocks.
    function test_windowBindsOnBothClocks() public {
        uint64 first = uint64(block.timestamp);
        vm.startPrank(vault);
        for (uint256 i = 0; i < 6; ++i) { g.out(bayc, 1, 0); Clock.skip(2 hours); }
        (uint256 items,, uint256 acts) = g.windowOf(bayc);
        assertEq(items, 6);
        assertEq(acts, 6);
        assertFalse(g.paused());
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.OutflowWait.selector, "collection items", first + 24 hours));
        g.out(bayc, 1, 0);
        // 24 h by timestamp alone is not enough
        Clock.skipTimeOnly(12 hours);
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.OutflowWait.selector, "collection items", first + 24 hours));
        g.out(bayc, 1, 0);
        vm.roll(block.number + 12 hours / 12 + 1);
        g.out(bayc, 1, 0);
        (items,, acts) = g.windowOf(bayc);
        assertEq(items, 6, "one pruned, one added");
        assertEq(acts, 6);
        vm.stopPrank();
    }

    function test_windowsAreSeparatePerCollectionAndSharedGlobally() public {
        for (uint256 i = 0; i < 6; ++i) {
            vm.prank(vault);
            g.out(bayc, 1, 0);
            vm.prank(other);
            g.out(punks, 1, 0);
            Clock.skip(2 hours);
        }
        assertFalse(g.paused());
        vm.prank(other);
        vm.expectRevert();                    // the global 12 is spent
        g.out(punks, 1, 0);
        vm.prank(vault);
        vm.expectRevert();
        g.out(bayc, 1, 0);
    }

    function test_valueWindow() public {
        vm.startPrank(vault);
        g.out(bayc, 0, 9 ether);
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.OutflowWait.selector, "global value", uint64(block.timestamp + 24 hours)));
        g.out(bayc, 0, 2 ether);
        g.out(bayc, 0, 1 ether);
        vm.stopPrank();
    }

    /// The tripwire: a whole day's items in six hours pauses the guard; only the owner unpauses.
    function test_tripwireOnARush() public {
        vm.startPrank(vault);
        for (uint256 i = 0; i < 5; ++i) g.out(bayc, 1, 0);
        assertFalse(g.paused());
        g.out(bayc, 1, 0);                   // the sixth within six hours
        assertTrue(g.paused());
        vm.expectRevert(HomeGuard.IsPaused.selector);
        g.admit(bayc, 1, 0);
        vm.expectRevert(HomeGuard.IsPaused.selector);
        g.out(bayc, 0, 1);
        vm.stopPrank();
        vm.expectRevert(HomeGuard.NotOwner.selector);
        g.setPaused(false);
        vm.prank(timelock);
        g.setPaused(false);
        assertFalse(g.paused());
    }

    /// The same six items spread over a day do not trip: a limit reached slowly is a limit, not an alarm.
    function test_noTripWhenTheDayIsSpent_slowly() public {
        vm.startPrank(vault);
        for (uint256 i = 0; i < 6; ++i) { g.out(bayc, 1, 0); Clock.skip(2 hours); }
        assertFalse(g.paused());
        vm.stopPrank();
    }

    function testFuzz_windowNeverExceedsTheLimit(uint8[16] memory gaps) public {
        vm.startPrank(vault);
        for (uint256 i = 0; i < gaps.length; ++i) {
            Clock.skip(uint256(gaps[i]) * 30 minutes);
            if (g.paused()) { vm.stopPrank(); vm.prank(timelock); g.setPaused(false); vm.startPrank(vault); }
            try g.out(bayc, 1, 0) {} catch {}
            (uint256 items,,) = g.windowOf(bayc);
            assertLe(items, 6);
        }
        vm.stopPrank();
    }
}
