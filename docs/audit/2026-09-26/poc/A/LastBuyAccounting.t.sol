// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {AuditABase} from "./AuditABase.sol";
import {SweepVault} from "../../../src/SweepVault.sol";
import {FrogCurve} from "../../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../../src/curve/CurveToken.sol";

contract ForceSend {
    constructor(address payable to) payable { selfdestruct(to); }
}

/// A-02: the buy that reaches the raise re-derives `gross = ceil(room·BPS/(BPS−rate))` from `room`, but never checks it against
/// `msg.value`. When `msg.value` sits in a narrow band (net ≥ room, yet msg.value < ceil(room·BPS/(BPS−rate))) the curve books
/// `gross` of fees + reserve while receiving only `msg.value`: the books exceed the balance and the last claimant is short.
contract LastBuyAccountingTest is AuditABase {
    uint256 constant B = 10_000;

    /// the smallest msg.value that closes the curve now, if it books more than it pays; 0 if none exists in the search window
    function _badValue(FrogCurve c) internal view returns (uint256) {
        uint256 r = c.feeBps() + c.launchExtraBps();
        uint256 room = c.vEnd() - c.v();
        uint256 g = Math.mulDiv(room, B, B - r, Math.Rounding.Ceil);
        for (uint256 x = g - 1; x + 64 > g; --x) {
            if (x - (x * r) / B >= room) return x;             // closes the curve, yet x < g
        }
        return 0;
    }

    function test_A02_lastBuyBooksMoreThanItReceives() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(20 minutes);
        // an ordinary buy first, so `room` is not a round number
        vm.deal(buyer, 100 ether);
        vm.prank(buyer);
        c.buy{value: 1.234567 ether}(0);

        uint256 x = _badValue(c);
        assertGt(x, 0, "a closing value exists that under-pays");
        vm.prank(mallory);
        vm.deal(mallory, x);
        c.buy{value: x}(0);
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduating));
        uint256 owed = _owed(c);
        emit log_named_uint("owed - balance, wei", owed - address(c).balance);
        assertGt(owed, address(c).balance, "the curve owes more than it holds");

        // consequence: graduate (works: the platform's 0.25 ETH graduation fee is still in the balance), then everyone claims;
        // whoever claims last is refused for want of a few wei, however large their claim
        c.graduate();
        c.flushVaultFees();
        c.flushProtocolFees();
        vm.prank(creator);
        c.claimCreatorFees();
        uint256 plat = c.platformOwed();
        emit log_named_decimal_uint("platform's claim now blocked", plat, 18);
        vm.expectRevert(FrogCurve.TransferFailed.selector);
        c.claimPlatformFees();

        // nobody can top it up through `receive` (only the queue may pay the curve); a SELFDESTRUCT force-send is the only cure
        vm.deal(address(this), 1 ether);
        (bool ok,) = address(c).call{value: 1}("");
        assertFalse(ok);
        new ForceSend{value: plat - address(c).balance}(payable(address(c)));
        c.claimPlatformFees();
        assertEq(c.platformOwed(), 0);
    }

    /// The same during the launch fee (rate 51%): the per-close shortfall grows with BPS/(BPS−rate).
    function test_A02_duringLaunchFee_largerShortfall() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE, false);
        uint256 x = _badValue(c);
        assertGt(x, 0);
        vm.deal(mallory, x);
        vm.prank(mallory);
        c.buy{value: x}(0);
        emit log_named_uint("rate, bps", 100 + 5_000);
        emit log_named_uint("owed - balance, wei", _owed(c) - address(c).balance);
        assertGt(_owed(c), address(c).balance);
    }

    /// Fuzz: for any closing buy, the curve must hold at least what it owes. Fails whenever the band is hit.
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
        if (x - (x * r) / B < room) return;                     // does not close
        vm.deal(mallory, x);
        vm.prank(mallory);
        c.buy{value: x}(0);
        assertGe(address(c).balance, _owed(c), "insolvent after the closing buy");
    }
}
