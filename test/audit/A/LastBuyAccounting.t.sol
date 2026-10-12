// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {AuditABase} from "./AuditABase.sol";
import {SweepVault} from "../../../src/SweepVault.sol";
import {FrogCurve} from "../../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../../src/curve/CurveToken.sol";

/// Regression for A-02: the closing buy re-derives `gross = ceil(room·BPS/(BPS−rate))`, which could exceed `msg.value` in a
/// narrow band, so the curve booked more than it received and the last claimant was refused. The fix clamps gross to
/// `msg.value`; these are the audit's tests, now asserting the curve stays solvent and every claim is paid.
contract LastBuyAccountingTest is AuditABase {
    uint256 constant B = 10_000;

    /// the smallest msg.value that closes the curve now while under the re-derived gross (the audit's band); 0 if none
    function _badValue(FrogCurve c) internal view returns (uint256) {
        uint256 r = c.feeBps() + c.launchExtraBps();
        uint256 room = c.vEnd() - c.v();
        uint256 g = Math.mulDiv(room, B, B - r, Math.Rounding.Ceil);
        for (uint256 x = g - 1; x + 64 > g; --x) {
            if (x - (x * r) / B >= room) return x;
        }
        return 0;
    }

    function test_A02_aClosingBuyInTheBandIsSolvent_andEveryClaimIsPaid() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        vm.deal(buyer, 100 ether);
        vm.prank(buyer);
        c.buy{value: 1.234567 ether}(0);

        uint256 x = _badValue(c);
        assertGt(x, 0, "the band still exists as an input");
        uint256 before = address(c).balance;
        vm.deal(mallory, x);
        vm.prank(mallory);
        c.buy{value: x}(0);
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduating));
        assertEq(mallory.balance, 0, "no change: the whole value was taken");
        assertEq(address(c).balance + c.itemBudget(), before + x, "the curve received x, less the budget it passed on");
        assertGe(address(c).balance, _owed(c), "it holds what it owes");

        c.graduate();
        c.flushVaultFees();
        c.flushProtocolFees();
        vm.prank(creator);
        c.claimCreatorFees();
        c.claimPlatformFees();                                  // before the fix: TransferFailed for want of a wei
        assertEq(c.platformOwed(), 0);
        assertEq(_owed(c), 0);
    }

    function test_A02_duringTheLaunchFee_stillSolvent() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE, false);
        uint256 x = _badValue(c);
        assertGt(x, 0);
        vm.deal(mallory, x);
        vm.prank(mallory);
        c.buy{value: x}(0);
        assertGe(address(c).balance, _owed(c));
    }

    /// The audit's property, which failed on the old code within ~20 runs: for any closing buy, the curve holds what it owes.
    function testFuzz_A02_closingBuyIsSolvent(uint256 pre, uint256 extra, uint256 wait) public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(bound(wait, 0, 15 minutes));
        pre = bound(pre, 0, 4 ether);
        vm.deal(buyer, 100 ether);
        if (pre != 0) { vm.prank(buyer); c.buy{value: pre}(0); }
        uint256 r = c.feeBps() + c.launchExtraBps();
        uint256 room = c.vEnd() - c.v();
        uint256 xMin = Math.mulDiv(room, B, B - r) - 3;
        uint256 x = xMin + bound(extra, 0, 6);
        if (x - (x * r) / B < room) return;
        vm.deal(mallory, x);
        vm.prank(mallory);
        c.buy{value: x}(0);
        assertGe(address(c).balance, _owed(c), "insolvent after the closing buy");
    }

    /// "Curve ETH out ≤ ETH in": random buys and sells, a close in or near the band, graduation, every claim. The curve is
    /// solvent after every step, and what left it for traders never exceeds what they put in.
    function testFuzz_A02_curveEthOutNeverExceedsIn(uint256[6] memory amounts, uint256 extra, bool viaDeadline) public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        vm.deal(buyer, 1_000 ether);
        uint256 inTotal;
        uint256 outTotal;
        for (uint256 i = 0; i < amounts.length; ++i) {
            uint256 b0 = buyer.balance;
            vm.startPrank(buyer);
            if (i % 2 == 0) {
                uint256 v = bound(amounts[i], 1 gwei, 1 ether);
                if (viaDeadline) c.buy{value: v}(0, block.timestamp);
                else c.buy{value: v}(0);
            } else {
                uint256 bal = tok.balanceOf(buyer);
                uint256 amt = bound(amounts[i], 0, bal);
                if (amt != 0) {
                    tok.approve(address(c), amt);
                    try c.sell(amt, 0) {} catch {}
                }
            }
            vm.stopPrank();
            if (buyer.balance < b0) inTotal += b0 - buyer.balance;
            else outTotal += buyer.balance - b0;
            assertGe(address(c).balance, _owed(c));
        }
        uint256 r = c.feeBps() + c.launchExtraBps();
        uint256 x = Math.mulDiv(c.vEnd() - c.v(), B, B - r) - 3 + bound(extra, 0, 6);
        if (x - (x * r) / B < c.vEnd() - c.v()) x = Math.mulDiv(c.vEnd() - c.v(), B, B - r, Math.Rounding.Ceil) + bound(extra, 0, 1 ether);
        uint256 b1 = buyer.balance;
        vm.prank(buyer);
        c.buy{value: x}(0);
        inTotal += b1 - buyer.balance;
        assertGe(address(c).balance, _owed(c));
        c.graduate();
        c.flushVaultFees();
        c.flushProtocolFees();
        vm.prank(creator);
        c.claimCreatorFees();
        c.claimPlatformFees();
        assertEq(_owed(c), 0);
        assertLe(outTotal, inTotal, "traders took out no more than they paid in");
    }

    /// A-07: the deadline overloads refuse a late transaction and otherwise behave as the plain calls.
    function test_A07_buyAndSellTakeADeadline() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        vm.deal(buyer, 10 ether);
        vm.prank(buyer);
        vm.expectRevert(FrogCurve.Expired.selector);
        c.buy{value: 1 ether}(0, block.timestamp - 1);
        vm.prank(buyer);
        uint256 out = c.buy{value: 1 ether}(0, block.timestamp);
        assertGt(out, 0);
        vm.startPrank(buyer);
        tok.approve(address(c), out);
        vm.expectRevert(FrogCurve.Expired.selector);
        c.sell(out, 0, block.timestamp - 1);
        assertGt(c.sell(out, 0, block.timestamp), 0);
        vm.stopPrank();
    }
}
