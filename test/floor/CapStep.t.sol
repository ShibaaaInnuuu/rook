// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase} from "../utils/SweepBase.sol";
import {NftBase} from "../nft/NftBase.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {NftVaultBase} from "../../src/nft/NftVaultBase.sol";

/// H12 (docs/v5/custody-hardening.md): the keeper's `postCap` had no step bound, so one post could move a sweep's ceiling to any
/// price. Now, while the last cap is fresh, a new one is within ±20% of it and at least ten minutes (both clocks) after it; no cap
/// is taken without the registry owner's ceiling (the first applies at once, C-01) and nothing exceeds it; and even a lapsed cap
/// rises at most one step. The floor module's own floor has its move limit (FloorModuleTest).
contract CapStepSweepTest is SweepBase {
    function _post(uint256 c) internal {
        vm.prank(keeper);
        raffle.postCap(c);
    }

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    function test_whileFresh_aStepOfAtMostTwentyPercent_tenMinutesApart() public {
        _post(1 ether);
        vm.prank(keeper);
        vm.expectRevert(SweepVault.CapTooSoon.selector);
        raffle.postCap(1.1 ether);
        _skip(10 minutes);
        _post(1.2 ether);                                      // +20%: allowed
        _skip(10 minutes);
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(SweepVault.CapStep.selector, 1.45 ether, 0.96 ether, 1.44 ether));
        raffle.postCap(1.45 ether);
        _post(0.96 ether);                                     // −20%
        assertEq(raffle.cap(), 0.96 ether);
    }

    function test_theSequencerAloneCannotShortenTheTenMinutes() public {
        _post(1 ether);
        vm.warp(vm.getBlockTimestamp() + 10 minutes);         // no parent blocks
        vm.prank(keeper);
        vm.expectRevert(SweepVault.CapTooSoon.selector);
        raffle.postCap(1.1 ether);
    }

    /// C-01 (docs/audit/2026-09-26): without a ceiling a vault takes no cap at all; the owner's first ceiling applies at once (it
    /// only tightens); a lapsed cap is re-posted at most one step up; a lower ceiling is at once and a raise waits seven days.
    function test_noCapWithoutACeiling_theFirstIsAtOnce_andALapsedCapRisesOneStepAtMost() public {
        SweepVault v = new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(venue), launcher, desk, PBS);
        vm.prank(keeper);
        vm.expectRevert(SweepVault.NoCeiling.selector);
        v.postCap(1 ether);
        vm.prank(owner);
        v.proposeCapCeiling(2 ether);
        assertEq(v.capCeiling(), 2 ether, "the first ceiling is active at once");
        assertEq(v.pendingCeiling(), 0);
        vm.prank(keeper);
        v.postCap(1 ether);
        _skip(1 hours);
        assertFalse(v.capFresh());
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(SweepVault.CapStep.selector, 1.5 ether, 0, 1.2 ether));
        v.postCap(1.5 ether);                                  // stale: still at most one step up
        vm.prank(keeper);
        v.postCap(0.5 ether);                                  // a stale re-post may fall freely
        _skip(1 hours);
        vm.prank(keeper);
        v.postCap(0.6 ether);
        _skip(10 minutes);
        vm.prank(owner);
        v.proposeCapCeiling(0.65 ether);                       // lowering is immediate
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(SweepVault.OverCeiling.selector, 0.7 ether, 0.65 ether));
        v.postCap(0.7 ether);
        vm.prank(owner);
        v.proposeCapCeiling(5 ether);
        assertEq(v.capCeiling(), 0.65 ether, "a raise is only a proposal");
        vm.expectRevert(SweepVault.TooEarly.selector);
        v.acceptCapCeiling();
        _skip(7 days);
        v.acceptCapCeiling();                                  // anyone, once due
        assertEq(v.capCeiling(), 5 ether);
    }

    function test_onlyTheOwnerProposesACeiling() public {
        vm.prank(keeper);
        vm.expectRevert(SweepVault.NotOwner.selector);
        raffle.proposeCapCeiling(1 ether);
    }
}

contract CapStepNftTest is NftBase {
    function _post(uint256 c) internal {
        vm.prank(keeper);
        raffleVault.postCap(c, "ref");
    }

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / 12 + 1);
    }

    function test_theNftSweepVaultStepsToo() public {
        _post(1 ether);
        vm.prank(keeper);
        vm.expectRevert(NftVaultBase.CapTooSoon.selector);
        raffleVault.postCap(1 ether, "ref");
        _skip(10 minutes);
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(NftVaultBase.CapStep.selector, 2 ether, 0.8 ether, 1.2 ether));
        raffleVault.postCap(2 ether, "ref");
        _post(1.2 ether);
        vm.prank(owner);
        raffleVault.proposeCapCeiling(1.5 ether);
        _skip(7 days);
        raffleVault.acceptCapCeiling();
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(NftVaultBase.OverCeiling.selector, 1.6 ether, 1.5 ether));
        raffleVault.postCap(1.6 ether, "ref");
    }
}
