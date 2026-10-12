// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {AutoBase} from "../utils/AutoBase.sol";
import {Clock} from "../utils/Clock.sol";
import {Wait} from "../../src/lib/Wait.sol";
import {AutoBuyerVault} from "../../src/AutoBuyerVault.sol";
import {AssetVault} from "../../src/AssetVault.sol";
import {Distributor} from "../../src/Distributor.sol";
import {Merkle} from "../utils/Merkle.sol";

/// Chain 4663 is an Arbitrum Nitro L2, and Nitro lets the sequencer stamp a block up to ONE HOUR in the future. Every window
/// in this system in which a person is supposed to be able to see something and stop it was an hour or less, measured on that
/// clock alone. One hour of tolerance against a one-hour wait is not a margin; it is the whole wait. These tests move only
/// the sequencer's clock — `Clock.skipTimeOnly`, which is `vm.warp` with no `vm.roll`, and is exactly what a sequencer can do
/// — and assert that each window survives it. See `src/lib/Wait.sol` for the measurement behind the block counts.
contract SequencerClockTest is AutoBase {
    function test_S1_aTimestampJumpAloneNoLongerCollapsesTheV2PublicHour() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        vm.prank(keeper);
        autoOrd.postCap(5 ether);
        vm.prank(autoBuyer);
        uint256 id = autoOrd.announce(1 ether, 1, "cap-ref");
        AutoBuyerVault.Purchase memory p = autoOrd.purchase(id);

        // the sequencer stamps the very next block a full hour ahead. This is legal Nitro behaviour, not a clock violation.
        Clock.skipTimeOnly(1 hours);
        assertGe(block.timestamp, p.readyAt, "the sequencer's clock says the hour is up");
        assertLt(block.number, p.readyAtBlock, "the parent chain says it is not");

        vm.prank(autoBuyer);
        vm.expectRevert(abi.encodeWithSelector(AutoBuyerVault.TooEarly.selector, p.readyAt));
        autoOrd.pull(id);

        // one block short is still short
        vm.roll(uint256(p.readyAtBlock) - 1);
        vm.prank(autoBuyer);
        vm.expectRevert(abi.encodeWithSelector(AutoBuyerVault.TooEarly.selector, p.readyAt));
        autoOrd.pull(id);

        // and the parent chain, which the sequencer cannot run forward, is what finally releases it
        vm.roll(uint256(p.readyAtBlock));
        vm.prank(autoBuyer);
        autoOrd.pull(id);
        assertEq(autoBuyer.balance, 1 ether);
    }

    function test_S1_theSameHoldsForTheV1Release() public {
        _fund(ordVault, coinA, 5 ether);
        vm.prank(keeper);
        ordVault.postCap(5 ether);
        vm.prank(keeper);
        uint256 id = ordVault.declarePurchase(1 ether, 1, "cap-ref");
        AssetVault.Purchase memory p = ordVault.purchase(id);

        Clock.skipTimeOnly(1 hours);
        assertGe(block.timestamp, p.releasableAt);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.TooEarly.selector, p.releasableAt));
        ordVault.release(id);

        vm.roll(uint256(p.releasableAtBlock));
        ordVault.release(id);
        assertEq(purchaser.balance, 1 ether);
    }

    /// The block count is a real hour, not a token gesture: 300 parent blocks at the measured 12.06–12.14 s is 60.3–60.7
    /// minutes. If the parent chain kept Ethereum's nominal 12 s exactly it would be 60.0.
    function test_S1_theBlockCountIsAnHourAtTheMeasuredRate() public pure {
        assertEq(Wait.blocksFor(1 hours), 300);
        assertEq(Wait.blocksFor(15 minutes), 75);
        // at the slowest span measured (12.14 s), 300 blocks is 3642 s — longer than the hour it stands in for, never shorter
        assertGe(uint256(300 * 1206) / 100, uint256(3600));
    }

    /// The fifteen minutes in which the owner may cancel a draw is well inside the sequencer's hour of tolerance, so before
    /// this it could be closed before anybody saw the draw at all.
    function test_S1_aTimestampJumpCannotCloseTheDrawChallengeWindow() public {
        uint256 aid = _openedDraw();

        Clock.skipTimeOnly(1 hours);            // the sequencer says the fifteen minutes are long gone
        vm.prank(owner);
        distributor.cancelDraw(0, aid);         // the owner can still stop it, because the parent chain says otherwise
    }

    /// The other side of the same window: nobody may pin the randomness until it has really closed.
    function test_S1_aTimestampJumpCannotOpenTheDrawEarly() public {
        uint256 aid = _openedDraw();

        Clock.skipTimeOnly(1 hours);
        vm.expectRevert();                      // ChallengeWindow: the parent chain has not moved
        distributor.requestDraw(0, aid);

        Clock.skip(15 minutes);                 // both clocks
        distributor.requestDraw(0, aid);
    }

    address alice = makeAddr("alice");
    address bob = makeAddr("bob");

    /// One receipted inscription, with a draw opened on it, so there is a challenge window to attack.
    function _openedDraw() internal returns (uint256 aid) {
        aid = _anAcquisition();
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.DrawWeighted, address(0));
        bytes32[] memory leaves = new bytes32[](2);
        leaves[0] = keccak256(abi.encode(alice, uint256(1), uint256(0)));
        leaves[1] = keccak256(abi.encode(bob, uint256(1), uint256(1)));
        vm.prank(keeper);
        distributor.openDraw(0, aid, Merkle.root(leaves), 2, "ipfs://tickets");
    }

    /// One receipted inscription, so there is something to distribute.
    function _anAcquisition() internal returns (uint256 aid) {
        _fund(ordVault, coinA, 5 ether);
        vm.prank(keeper);
        ordVault.postCap(5 ether);
        vm.prank(keeper);
        uint256 id = ordVault.declarePurchase(1 ether, 1, "cap-ref");
        Clock.skip(1 hours);
        ordVault.release(id);
        vm.prank(attestor);
        aid = ordVault.settleInscription(id, ids[0], bytes32("tx"), 1000, _proofOf(0));
    }
}
