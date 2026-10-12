// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {AutoBase} from "../utils/AutoBase.sol";
import {Clock} from "../utils/Clock.sol";
import {Merkle} from "../utils/Merkle.sol";
import {Wait} from "../../src/lib/Wait.sol";
import {AssetVault} from "../../src/AssetVault.sol";
import {AutoBuyerVault} from "../../src/AutoBuyerVault.sol";
import {Distributor} from "../../src/Distributor.sol";
import {ListingRegistry} from "../../src/ListingRegistry.sol";

/// docs/audit/README.md "Single-clock waits". The Distributor's rune `claimsEnd` and nine more waits were measured on the
/// sequencer's clock alone. Each test here is the attack as the chain allows it: both clocks move to ONE HOUR before the
/// deadline, then the sequencer stamps a block one hour ahead — legal on Nitro, and nothing more than it may do — so the
/// timestamp says the wait is over while the parent chain says it is not. The wait must hold, and must then end on the parent
/// chain's block, not before and not never.
contract SingleClockWaitsTest is AutoBase {
    string constant TAPROOT = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";
    bytes32 constant TXID = keccak256("a bitcoin txid");
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address carol = makeAddr("carol");
    bytes32[] tickets;
    bytes32[] shares;

    function setUp() public override {
        super.setUp();
        launcher.set(coinA, router, creatorA, 0);
        tickets.push(distributor.ticketLeaf(alice, 0, 60));
        tickets.push(distributor.ticketLeaf(bob, 60, 100));
        shares.push(distributor.shareLeaf(alice, 60_000));
        shares.push(distributor.shareLeaf(bob, 30_000));
        shares.push(distributor.shareLeaf(carol, 9_000));
    }

    /// `secs` from now until a deadline: both clocks to an hour short of it, then the sequencer's legal hour ahead.
    function _sequencerAhead(uint256 secs) internal {
        Clock.skip(secs - 1 hours);
        Clock.skipTimeOnly(1 hours);
    }

    // ---- the Distributor ------------------------------------------------------------------------------------------------------

    function _runes(uint256 amount) internal returns (uint256 aid) {
        aid = runeVault.acquisitionCount();
        _fund(runeVault, coinA, 1 ether);
        vm.prank(keeper); runeVault.postCap(1 ether);
        vm.prank(keeper); uint256 id = runeVault.declarePurchase(1 ether, amount, "f");
        Clock.skip(1 hours);
        runeVault.release(id);
        vm.prank(attestor);
        runeVault.settleRune(id, amount, TXID, 1);
    }

    function _openedDistribution() internal returns (uint256 aid) {
        launcher.set(coinA, router, creatorA, 1);
        aid = _runes(100_000);
        vm.prank(creatorA);
        distributor.choose(1, aid, Distributor.Mode.ProRata, address(0));
        vm.prank(keeper);
        distributor.openDistribution(1, aid, Merkle.root(shares), "ipfs://shares");
        Clock.skip(15 minutes);                                   // past the challenge window: claims are open
    }

    function test_theSequencerCannotCloseRuneClaimsEarly_theyCloseOnTheParentChain() public {
        uint256 aid = _openedDistribution();
        Distributor.Distribution memory d = distributor.distribution(1, aid);
        uint64 endBlock = distributor.claimsEndBlock(1, aid);
        _sequencerAhead(uint256(d.claimsEnd) - block.timestamp);
        assertGe(block.timestamp, d.claimsEnd, "the sequencer's clock says claims have closed");
        assertLt(block.number, endBlock, "the parent chain says they have not");

        distributor.claimShare(1, aid, alice, 60_000, Merkle.proof(shares, 0));   // still accepted
        vm.expectRevert(Distributor.TooEarly.selector);
        distributor.rollOver(1, aid);                                              // and nobody may take the rest yet

        vm.roll(uint256(endBlock) - 1);
        distributor.claimShare(1, aid, bob, 30_000, Merkle.proof(shares, 1));     // one block short is still open

        vm.roll(endBlock);
        vm.expectRevert(Distributor.ClaimsClosed.selector);
        distributor.claimShare(1, aid, carol, 9_000, Merkle.proof(shares, 2));   // both clocks: closed
        distributor.rollOver(1, aid);
        assertEq(distributor.carry(1, coinA), 100_000 - 90_000, "what nobody claimed rolls over, and only that");
        assertEq(distributor.distribution(1, aid).claimsEnd, d.claimsEnd, "the struct a view returns kept its shape and value");
    }

    function _pinned() internal returns (uint256 aid) {
        _fund(ordVault, coinA, 1 ether);
        vm.prank(keeper); ordVault.postCap(1 ether);
        vm.prank(keeper); uint256 id = ordVault.declarePurchase(1 ether, 1, "f");
        Clock.skip(1 hours);
        ordVault.release(id);
        vm.prank(attestor);
        aid = ordVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0));
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.DrawWeighted, address(0));
        vm.prank(keeper);
        distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://tickets");
        Clock.skip(15 minutes);
        distributor.requestDraw(0, aid);
    }

    function test_theSequencerCannotBringARedrawForward() public {
        uint256 aid = _pinned();
        uint64 at = distributor.lot(0, aid).pinnedAt;
        uint64 atBlock = distributor.pinnedAtBlock(0, aid);
        _sequencerAhead(24 hours);
        assertGe(block.timestamp, at + 24 hours);
        vm.expectRevert(Distributor.TooEarly.selector);
        distributor.redraw(0, aid);
        vm.roll(atBlock + Wait.blocksFor(24 hours));
        distributor.redraw(0, aid);
    }

    function test_theSequencerCannotLetTheOwnerVoidADrawEarly() public {
        uint256 aid = _pinned();
        distributor.resolve(0, aid, abi.encodePacked(bytes32(uint256(7))));      // resolved, and nobody claims it
        uint64 atBlock = distributor.resolvedAtBlock(0, aid);
        _sequencerAhead(7 days);
        vm.prank(owner);
        vm.expectRevert(Distributor.TooEarly.selector);
        distributor.voidDraw(0, aid);
        vm.roll(atBlock + Wait.blocksFor(7 days));
        vm.prank(owner);
        distributor.voidDraw(0, aid);
    }

    function test_theSequencerCannotMakeTheKeeperOverdueEarly_forALot() public {
        _fund(ordVault, coinA, 1 ether);
        vm.prank(keeper); ordVault.postCap(1 ether);
        vm.prank(keeper); uint256 id = ordVault.declarePurchase(1 ether, 1, "f");
        Clock.skip(1 hours);
        ordVault.release(id);
        vm.prank(attestor);
        uint256 aid = ordVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0));
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.Named, alice);
        vm.prank(alice);
        distributor.setDestination(0, aid, TAPROOT);
        Distributor.Lot memory lot = distributor.lot(0, aid);
        _sequencerAhead(7 days + 1);
        assertGt(block.timestamp, lot.destinationAt + 7 days, "the sequencer's clock says the week is over");
        vm.expectRevert(Distributor.NotOverdue.selector);
        distributor.flagOverdue(0, aid);
        vm.roll(lot.destinationAtBlock + Wait.blocksFor(7 days));
        distributor.flagOverdue(0, aid);
        assertTrue(distributor.lot(0, aid).overdue);
    }

    function test_theSequencerCannotMakeTheKeeperOverdueEarly_forAPayout() public {
        uint256 aid = _openedDistribution();
        distributor.claimShare(1, aid, carol, 9_000, Merkle.proof(shares, 2));
        vm.prank(keeper); distributor.postMinimum(1, 1_000);
        vm.prank(carol);
        uint256 pid = distributor.requestPayout(1, TAPROOT);
        uint64 atBlock = distributor.payoutRequestedAtBlock(pid);
        _sequencerAhead(7 days + 1);
        vm.expectRevert(Distributor.NotOverdue.selector);
        distributor.flagPayoutOverdue(pid);
        vm.roll(atBlock + Wait.blocksFor(7 days));
        distributor.flagPayoutOverdue(pid);
        assertTrue(distributor.payout(pid).overdue);
    }

    // ---- the vaults -----------------------------------------------------------------------------------------------------------

    function test_theSequencerCannotWriteOffAV1PurchaseEarly_orBlockItsVault() public {
        _fund(ordVault, coinA, 5 ether);
        vm.prank(keeper); ordVault.postCap(1 ether);
        vm.prank(keeper); uint256 id = ordVault.declarePurchase(1 ether, 1, "f");
        Clock.skip(1 hours);
        ordVault.release(id);
        uint64 deadline = ordVault.purchase(id).deadline;
        uint64 dBlock = ordVault.deadlineBlock(id);
        _sequencerAhead(3 days + 1);
        assertGt(block.timestamp, deadline);
        (bool blocked,) = ordVault.isBlocked();
        assertFalse(blocked, "not overdue on the parent chain, so not blocking the vault");
        vm.prank(owner);
        vm.expectRevert(AssetVault.NotOverdue.selector);
        ordVault.writeOff(id, "early");

        vm.roll(dBlock);
        (blocked,) = ordVault.isBlocked();
        assertTrue(blocked);
        // the public write-off, seven days later, is held the same way
        _sequencerAhead(7 days);
        vm.prank(stranger);
        vm.expectRevert(AssetVault.NotOwner.selector);
        ordVault.writeOff(id, "early and public");
        vm.roll(dBlock + Wait.blocksFor(7 days));
        vm.prank(stranger);
        ordVault.writeOff(id, "late and public");
        assertEq(uint8(ordVault.purchase(id).status), uint8(AssetVault.Status.WrittenOff));
    }

    function test_theSequencerCannotWriteOffAV2PurchaseEarly_orMarkItLate() public {
        _fundAuto(autoRune, coinA, 5 ether);
        uint256 early = _pulled(autoRune, 1 ether, 100);
        uint64 dBlock = autoRune.deadlineBlock(early);
        _sequencerAhead(3 days + 1);
        assertGt(block.timestamp, autoRune.purchase(early).deadline);
        vm.prank(owner);
        vm.expectRevert(AutoBuyerVault.NotOverdue.selector);
        autoRune.writeOff(early, "early");
        (bool blocked,) = autoRune.isBlocked();
        assertFalse(blocked);
        vm.prank(autoBuyer);
        autoRune.settleRune(early, 100, TXID, 1);
        assertFalse(autoRune.purchase(early).late, "on time on the parent chain is on time");

        uint256 late = _pulled(autoRune, 1 ether, 100);
        vm.roll(autoRune.deadlineBlock(late));
        Clock.skip(3 days + 1);
        vm.prank(autoBuyer);
        autoRune.settleRune(late, 100, TXID, 1);
        assertTrue(autoRune.purchase(late).late, "late on both clocks is late");
        dBlock;
    }

    function test_theSequencerCannotOpenAV2PublicCancelEarly() public {
        _fundAuto(autoOrd, coinA, 5 ether);
        vm.prank(keeper); autoOrd.postCap(1 ether);
        vm.prank(autoBuyer);
        uint256 id = autoOrd.announce(1 ether, 1, "cap-ref");
        AutoBuyerVault.Purchase memory p = autoOrd.purchase(id);
        _sequencerAhead(uint256(p.readyAt) + 7 days + 1 - block.timestamp);
        assertGt(block.timestamp, uint256(p.readyAt) + 7 days);
        vm.prank(stranger);
        vm.expectRevert(AutoBuyerVault.NotBuyer.selector);
        autoOrd.returnFunds(id);
        vm.roll(p.readyAtBlock + Wait.blocksFor(7 days));
        vm.prank(stranger);
        autoOrd.returnFunds(id);
        assertEq(uint8(autoOrd.purchase(id).status), uint8(AutoBuyerVault.Status.Returned));
    }

    // ---- the registry -------------------------------------------------------------------------------------------------------

    function test_theSequencerCannotBringAGovernanceChangeForward() public {
        address till = makeAddr("a new treasury");
        vm.prank(owner);
        registry.proposeTreasury(till);
        uint64 readyAt = registry.pendingTreasuryAt();
        uint64 readyBlock = registry.pendingTreasuryReadyBlock();
        address k = makeAddr("new keeper"); address pu = makeAddr("new purchaser"); address at = makeAddr("new attestor");
        vm.prank(owner);
        registry.proposeRoles(k, pu, at);
        uint64 rolesBlock = registry.pendingRolesReadyBlock();

        _sequencerAhead(7 days);
        assertGe(block.timestamp, readyAt);
        vm.startPrank(owner);
        vm.expectRevert(abi.encodeWithSelector(ListingRegistry.TooEarly.selector, readyAt));
        registry.acceptTreasury();
        vm.expectRevert();
        registry.acceptRoles();

        vm.roll(readyBlock > rolesBlock ? readyBlock : rolesBlock);
        registry.acceptTreasury();
        registry.acceptRoles();
        vm.stopPrank();
        assertEq(registry.treasury(), till);
        assertEq(registry.keeper(), k);
    }
}
