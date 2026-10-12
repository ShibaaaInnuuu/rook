// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {AuditABase} from "./AuditABase.sol";
import {SweepVault} from "../../../src/SweepVault.sol";
import {FrogCurve} from "../../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../../src/curve/CurveToken.sol";
import {MintDesk} from "../../../src/curve/MintDesk.sol";

/// Regression for A-05 / C-10: "MintDesk OFF" was policy only, and `sweepViaDesk` was anyone's, so a stranger could park a
/// coin's whole vault credit at the unserved desk for its 6-hour TTL, again and again. The route is now keeper-only; the
/// Phase-1 keeper service never calls it, so the desk is unreachable on chain.
contract DeskOffTest is AuditABase {
    function _credit() internal returns (FrogCurve c, CurveToken tok, uint256 credit) {
        (c, tok) = _launch(SweepVault.Policy.RAFFLE, false);
        vm.deal(buyer, 10 ether);
        vm.prank(buyer);
        c.buy{value: 3 ether}(0);                               // inside the launch fee: most of it goes to the vault
        c.flushVaultFees();
        credit = curveRaffle.credit(address(tok));
        assertGt(credit, 1 ether);
    }

    function test_A05_aStrangerCanNoLongerParkVaultCreditAtTheDesk() public {
        (, CurveToken tok, uint256 credit) = _credit();
        uint256 cap = curveRaffle.cap();
        vm.prank(mallory);
        vm.expectRevert(SweepVault.NotKeeper.selector);
        curveRaffle.sweepViaDesk(address(tok), cap);
        assertEq(curveRaffle.credit(address(tok)), credit, "the credit stays for the coin's sweeps");
        assertEq(curveRaffle.atDesk(), 0);
        assertEq(desk.requestCount(), 0);
    }

    /// Control: the route itself still works for the keeper (a later phase that serves the desk), and collecting stays open.
    function test_A05_control_theKeeperStillCanAndAnyoneCollects() public {
        (, CurveToken tok, uint256 credit) = _credit();
        uint256 cap = curveRaffle.cap();
        vm.prank(keeper);
        uint256 id = curveRaffle.sweepViaDesk(address(tok), cap);
        assertEq(curveRaffle.credit(address(tok)), credit - cap);
        _skipBoth(6 hours + 1 minutes);
        vm.prank(mallory);
        curveRaffle.collectDesk(id);
        assertEq(curveRaffle.credit(address(tok)), credit);
    }
}
