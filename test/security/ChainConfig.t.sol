// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase} from "../utils/SweepBase.sol";
import {Wait} from "../../src/lib/Wait.sol";
import {MintDesk} from "../../src/curve/MintDesk.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {Distributor} from "../../src/Distributor.sol";
import {Wrapper} from "../../src/Wrapper.sol";

/// src/lib/Wait.sol: the parent chain's block time is deployment config. The same contracts, deployed for a chain whose
/// `block.number` ticks every 2 seconds, wait six times as many blocks for the same hour; and nobody can deploy with a block time
/// nobody measured.
contract ChainConfigTest is SweepBase {
    function test_theBlockHalfOfAWaitFollowsTheConfiguredBlockTime() public pure {
        assertEq(Wait.blocksFor(1 hours, 12), 300);
        assertEq(Wait.blocksFor(1 hours, 2), 1_800);
        assertEq(Wait.blocksFor(1 hours), 300, "the legacy form is 4663's, for the contracts this branch may not edit");
    }

    function test_aSecondChainsDeskWaitsItsOwnBlocks() public {
        MintDesk fast = new MintDesk(wrapper, payee, 6 hours, 2);
        assertEq(fast.parentBlockSeconds(), 2);
        assertEq(desk.parentBlockSeconds(), 12);
        vm.deal(address(this), 2 ether);
        uint256 a = desk.request{value: 1 ether}();
        uint256 b = fast.request{value: 1 ether}();
        // 3 hours of time and 1,800 parent blocks: at 12 s that is the 6-hour ttl's block count (1,800) already; at 2 s it is
        // 3 hours' worth of 2-second blocks, half of the 10,800 needed
        vm.roll(block.number + 1_800);
        assertTrue(desk.isExpired(a), "12 s blocks: 1,800 blocks is six hours");
        assertFalse(fast.isExpired(b), "2 s blocks: 1,800 blocks is one hour");
        vm.roll(block.number + 9_000);
        assertTrue(fast.isExpired(b));
    }

    function test_nobodyDeploysWithAnUnmeasuredBlockTime() public {
        vm.expectRevert(abi.encodeWithSelector(Wait.BadParentBlockSeconds.selector, 0));
        new MintDesk(wrapper, payee, 6 hours, 0);
        vm.expectRevert(abi.encodeWithSelector(Wait.BadParentBlockSeconds.selector, 61));
        new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(venue), launcher, desk, 61);
        vm.expectRevert(abi.encodeWithSelector(Wait.BadParentBlockSeconds.selector, 0));
        new Distributor(registry, beacon, 0);
        vm.expectRevert(abi.encodeWithSelector(Wait.BadParentBlockSeconds.selector, 0));
        new Wrapper(registry, distributor, beacon, FROGS, distribution, custodian, CUSTODY_BTC, "x", "x", 0);
    }
}
