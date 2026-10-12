// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "../utils/Base.sol";
import {Merkle} from "../utils/Merkle.sol";
import {MockEscrow, MockCurve} from "../utils/Mocks.sol";
import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";
import {AssetVault} from "../../src/AssetVault.sol";
import {Distributor} from "../../src/Distributor.sol";
import {FeeRouter} from "../../src/FeeRouter.sol";
import {BtcAddress} from "../../src/lib/BtcAddress.sol";
import {IPonsV2FeeEscrow, IPonsV2BondingCurve, IPonsV2MemeHook} from "../../src/interfaces/IPons.sol";
import {Clock} from "../utils/Clock.sol";

contract ForceFeeder {
    constructor(address payable to) payable { selfdestruct(to); }
}

/// A treasury that tries to come back in while it is being paid.
contract GreedyTreasury {
    FeeRouter public router;
    uint256 public attempts;
    bool public reenteredOk;
    function aim(FeeRouter r) external { router = r; }
    receive() external payable {
        attempts++;
        try router.harvest() { reenteredOk = true; } catch {}
    }
}

contract Addr { function ok(string memory a) external pure returns (bool) { return BtcAddress.isTaproot(a); } }

/// docs/phase3.md. One test per finding, written to fail on the code as it stood (the proofs of concept and their output are in
/// the phase document), plus the things that were attacked and held.
contract SecurityRegressionTest is Base {
    bytes32 constant TXID = bytes32(uint256(0xbeef));
    string constant TAPROOT = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";

    function _cap(AssetVault v, uint256 c) internal { vm.prank(keeper); v.postCap(c); }
    function _released(AssetVault v, uint256 maxEth, uint256 q) internal returns (uint256 id) {
        vm.prank(keeper); id = v.declarePurchase(maxEth, q, "ref");
        Clock.skip(1 hours);
        v.release(id);
    }
    function _coin(uint256 i) internal returns (address c) {
        c = address(uint160(0xC0100000 + i));
        launcher.set(c, router, creatorA, 0);
    }

    // ---- H1: the queue cannot be stuffed, and every loop has a ceiling -------------------------------------------------------

    function test_H1_dustNeverEntersTheQueue_andReleaseCostsTheSameBehindIt() public {
        _fund(ordVault, coinA, 1 ether);
        _cap(ordVault, 2 ether);
        vm.prank(keeper); uint256 clean = ordVault.declarePurchase(0.5 ether, 1, "ref");
        Clock.skip(1 hours);
        uint256 g = gasleft(); ordVault.release(clean); uint256 cleanGas = g - gasleft();

        for (uint256 i = 0; i < 1500; ++i) _fund(ordVault, _coin(i), 1);          // what bricked the old queue at ~3,400 entries
        assertEq(ordVault.queueLength(), 1, "one coin is queued; 1,500 dust coins are not");
        assertEq(ordVault.waiting(), 1500);
        assertEq(ordVault.attributed(), 0.5 ether);
        _cap(ordVault, 2 ether);
        vm.prank(keeper); uint256 id = ordVault.declarePurchase(0.5 ether, 1, "ref");
        Clock.skip(1 hours);
        g = gasleft(); ordVault.release(id); uint256 dusted = g - gasleft();
        assertLt(dusted, cleanGas + 20_000, "the dust costs a release nothing");
    }

    function test_H1_aCoinsDustIsStillItsOwn_andJoinsTheQueueWhenItAddsUp() public {
        address c = _coin(1);
        _fund(ordVault, c, 0.004 ether);
        _fund(ordVault, c, 0.004 ether);
        assertEq(ordVault.queueLength(), 0); assertEq(ordVault.waiting(), 0.008 ether); assertEq(ordVault.unspentOf(c), 0.008 ether);
        _fund(ordVault, c, 0.004 ether);                                            // 0.012: over MIN_ENTRY
        assertEq(ordVault.queueLength(), 1); assertEq(ordVault.waiting(), 0); assertEq(ordVault.attributed(), 0.012 ether);
        _fund(ordVault, c, 1);                                                      // and from then on everything is spendable at once
        assertEq(ordVault.attributed(), 0.012 ether + 1); assertEq(ordVault.queueLength(), 1, "a coin is in the queue once");
    }

    function test_H1_aReleaseNeedingMoreThanMaxPayersIsRefused_andASmallerOneIsNot() public {
        uint256 n = ordVault.MAX_PAYERS() + 2;
        for (uint256 i = 0; i < n; ++i) _fund(ordVault, _coin(i), 0.01 ether);
        _cap(ordVault, 5 ether);
        assertEq(ordVault.payersFor(0.64 ether), 64);
        assertEq(ordVault.payersFor(0.65 ether), 65);
        assertEq(ordVault.payersFor(99 ether), 0, "not enough is spendable");
        vm.prank(keeper); uint256 tooWide = ordVault.declarePurchase(0.65 ether, 1, "ref");
        Clock.skip(1 hours);
        vm.expectRevert(AssetVault.TooManyPayers.selector);
        ordVault.release(tooWide);
        vm.prank(owner); ordVault.cancel(tooWide);                                  // the way out is public and takes nothing
        _cap(ordVault, 5 ether);
        uint256 id = _released(ordVault, 0.64 ether, 1);
        assertEq(ordVault.chargesOf(id).length, 64);
        // the widest release there can be, and the widest refund of it, both fit a block many times over
        vm.deal(purchaser, 2 ether);
        uint256 g = gasleft();
        vm.prank(purchaser); ordVault.returnUnspent{value: 0.64 ether}(id);
        assertLt(g - gasleft(), 12_000_000);
    }

    // ---- H2: the declared price per unit holds at settlement ---------------------------------------------------------------------

    function test_H2_aShortPurchaseCannotBeClosedUntilTheRestIsBack_andTheBondStaysHeld() public {
        vm.prank(owner); registry.setBondBps(10_000);
        _fund(ordVault, coinA, 10 ether);
        _cap(ordVault, 2.5 ether);
        vm.deal(purchaser, 30 ether);
        vm.prank(purchaser); ordVault.postBond{value: 10 ether}();
        uint256 id = _released(ordVault, 10 ether, 4);                              // 2.5 ETH each, declared
        vm.prank(attestor); ordVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0));
        vm.prank(attestor);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.ReturnUnspentFirst.selector, 7.5 ether));
        ordVault.settleShort(id);
        vm.prank(purchaser); vm.expectRevert(); ordVault.withdrawBond(1);           // nothing of the bond is free
        vm.prank(purchaser); ordVault.returnUnspent{value: 7.5 ether}(id);
        vm.prank(attestor); ordVault.settleShort(id);
        assertEq(ordVault.outstanding(), 0);
        assertEq(ordVault.attributed(), 7.5 ether, "and it is the paying coin's again");
    }

    function test_H2_runes_theSameRule_andMoreThanDeclaredIsAlwaysFine() public {
        launcher.set(coinA, router, creatorA, 1);
        _fund(runeVault, coinA, 2 ether);
        _cap(runeVault, 0.01 ether);
        uint256 id = _released(runeVault, 1 ether, 10_000);                        // 100.00 runes at divisibility 2
        vm.prank(attestor);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.ReturnUnspentFirst.selector, 0.5 ether));
        runeVault.settleRune(id, 5_000, TXID, 1);
        _cap(runeVault, 0.01 ether);
        uint256 id2 = _released(runeVault, 1 ether, 10_000);
        vm.prank(attestor); runeVault.settleRune(id2, 12_345, TXID, 1);            // a better fill than promised needs nothing back
        assertEq(uint8(runeVault.purchase(id2).status), uint8(AssetVault.Status.Settled));
    }

    // ---- M1: a bond belongs to the purchaser who posted it ------------------------------------------------------------------------

    function test_M1_aNewPurchaserIsNotCoveredByTheOldOnesBond_andTheOldOneGetsTheirsBack() public {
        vm.prank(owner); registry.setBondBps(10_000);
        address next = makeAddr("nextPurchaser");
        vm.deal(purchaser, 5 ether);
        vm.prank(purchaser); ordVault.postBond{value: 3 ether}();
        vm.prank(owner); registry.proposeRoles(keeper, next, attestor);
        Clock.skip(7 days);
        vm.prank(owner); registry.acceptRoles();

        _fund(ordVault, coinA, 2 ether);
        _cap(ordVault, 1 ether);
        vm.prank(keeper); uint256 id = ordVault.declarePurchase(1 ether, 1, "ref");
        Clock.skip(1 hours);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.BondTooSmall.selector, 1 ether, 0));
        ordVault.release(id);                                                       // 3 ETH of bond is in the vault and none of it is theirs
        vm.prank(next); vm.expectRevert(AssetVault.NotPurchaser.selector); ordVault.withdrawBond(1 ether);
        uint256 before = purchaser.balance;
        vm.prank(purchaser); ordVault.withdrawBond(3 ether);                        // rotated out, still theirs
        assertEq(purchaser.balance - before, 3 ether);
        assertEq(ordVault.bond(), 0);
    }

    // ---- M2, M3, L3: draws --------------------------------------------------------------------------------------------------------

    bytes32[] tickets;
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");

    function _lot() internal returns (uint256 aid) {
        _fund(ordVault, coinA, 1 ether);
        _cap(ordVault, 1 ether);
        uint256 id = _released(ordVault, 1 ether, 1);
        vm.prank(attestor); aid = ordVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0));
        vm.prank(creatorA); distributor.choose(0, aid, Distributor.Mode.DrawWeighted, address(0));
    }

    function test_M2_aDrawIsPinnedAMinuteAhead() public {
        tickets.push(distributor.ticketLeaf(alice, 0, 60)); tickets.push(distributor.ticketLeaf(bob, 60, 100));
        uint256 aid = _lot();
        vm.prank(keeper); distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://t");
        Clock.skip(15 minutes);
        distributor.requestDraw(0, aid);
        uint64 round = distributor.lot(0, aid).round;
        assertGe(beacon.timeOf(round), vm.getBlockTimestamp() + 57, "due at least 57 s after the block that pinned it");
        assertEq(distributor.PIN_AHEAD(), 20);
    }

    function test_M3_aDrawNobodyCanProveIsVoidedByTheOwnerAfterAWeek_andNotBefore_andNotByAnyoneElse() public {
        tickets.push(distributor.ticketLeaf(alice, 0, 10)); tickets.push(distributor.ticketLeaf(bob, 10, 20));   // tickets 20..99 belong to nobody
        uint256 aid = _lot();
        vm.prank(keeper); distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://gappy");
        Clock.skip(15 minutes);
        distributor.requestDraw(0, aid);
        uint64 round = distributor.lot(0, aid).round;
        bytes32 seed = bytes32(uint256(1));
        for (uint256 i = 1; i < 50; ++i) {                                           // a round whose ticket falls in the gap
            seed = bytes32(i);
            if (uint256(keccak256(abi.encode(seed, uint256(0), aid))) % 100 >= 20) break;
        }
        distributor.resolve(0, aid, abi.encodePacked(seed));
        assertGe(distributor.lot(0, aid).winningTicket, 20);
        vm.prank(owner); vm.expectRevert(Distributor.TooEarly.selector); distributor.voidDraw(0, aid);
        Clock.skip(7 days);
        vm.prank(keeper); vm.expectRevert(Distributor.NotOwner.selector); distributor.voidDraw(0, aid);
        vm.prank(owner); distributor.voidDraw(0, aid);
        assertEq(uint8(distributor.lot(0, aid).stage), uint8(Distributor.Stage.None));
        assertEq(distributor.lot(0, aid).winner, address(0), "voiding names nobody");
        tickets[1] = distributor.ticketLeaf(bob, 10, 100);
        vm.prank(keeper); distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://whole");             // a new public snapshot
        round;
    }

    function test_M3_aClaimedDrawCannotBeVoided() public {
        tickets.push(distributor.ticketLeaf(alice, 0, 100)); tickets.push(distributor.ticketLeaf(bob, 100, 101));
        uint256 aid = _lot();
        vm.prank(keeper); distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://t");
        Clock.skip(15 minutes);
        distributor.requestDraw(0, aid);
        distributor.resolve(0, aid, abi.encodePacked(bytes32(uint256(7))));
        distributor.claimWin(0, aid, alice, 0, 100, Merkle.proof(tickets, 0));
        Clock.skip(30 days);
        vm.prank(owner); vm.expectRevert(Distributor.WrongStage.selector); distributor.voidDraw(0, aid);
    }

    function test_L3_anAddressCanBeCorrectedForAnHour_thenItIsTheAddress_andTheKeeperSendsOnlyAfterThat() public {
        uint256 aid = _lot0Named();
        vm.prank(alice); distributor.setDestination(0, aid, TAPROOT);
        vm.prank(keeper); vm.expectRevert(Distributor.TooEarly.selector); distributor.recordDelivery(0, aid, TXID);
        Clock.skip(30 minutes);
        vm.prank(alice); distributor.setDestination(0, aid, TAPROOT);                                           // a correction, inside the hour
        Clock.skip(31 minutes);
        vm.prank(alice); vm.expectRevert(Distributor.DestinationLocked.selector); distributor.setDestination(0, aid, TAPROOT);
        vm.prank(keeper); distributor.recordDelivery(0, aid, TXID);
    }

    function _lot0Named() internal returns (uint256 aid) {
        _fund(ordVault, coinA, 1 ether);
        _cap(ordVault, 1 ether);
        uint256 id = _released(ordVault, 1 ether, 1);
        vm.prank(attestor); aid = ordVault.settleInscription(id, ids[1], TXID, 1, _proofOf(1));
        vm.prank(creatorA); distributor.choose(0, aid, Distributor.Mode.Named, alice);
    }

    // ---- L2, and what held: the router ----------------------------------------------------------------------------------------------

    function _router(address treasury_) internal returns (FeeRouter r, MockEscrow escrow) {
        if (treasury_ != treasury) {
            vm.startPrank(owner); registry.proposeTreasury(treasury_); Clock.skip(7 days); registry.acceptTreasury(); vm.stopPrank();
        }
        escrow = new MockEscrow();
        MockCurve curve = new MockCurve(escrow);
        r = FeeRouter(payable(Clones.clone(address(new FeeRouter()))));
        r.initialize(coinA, registry, ordVault, IPonsV2FeeEscrow(address(escrow)), IPonsV2BondingCurve(address(curve)), IPonsV2MemeHook(address(0xB00C)));
        curve.setRecipient(address(r));
        launcher.set(coinA, address(r), creatorA, 0);
    }

    function test_L2_aTreasuryThatReentersGetsNothingExtra_andBlocksNothing() public {
        GreedyTreasury greedy = new GreedyTreasury();
        (FeeRouter r, MockEscrow escrow) = _router(address(greedy));
        greedy.aim(r);
        vm.deal(address(this), 10 ether);
        escrow.credit{value: 1 ether}(address(r));
        r.harvest();
        assertEq(greedy.attempts(), 1);
        assertFalse(greedy.reenteredOk(), "the second harvest was refused");
        assertEq(address(greedy).balance, 0.2 ether);
        assertEq(address(ordVault).balance, 0.8 ether);
        assertEq(address(r).balance, 0);
    }

    function test_held_threeThousandOddHarvestsNeverStrandOrMintAWei() public {
        (FeeRouter r, MockEscrow escrow) = _router(treasury);
        vm.deal(address(this), 100 ether);
        uint256 total;
        for (uint256 i = 1; i <= 3_000; ++i) {
            uint256 amount = (i * 7919) % 1013 + 1;                                  // awkward, mostly indivisible by five
            total += amount;
            if (i % 2 == 0) escrow.credit{value: amount}(address(r)); else { (bool ok,) = address(r).call{value: amount}(""); ok; }
            r.harvest();
            assertEq(address(r).balance, 0);
        }
        assertEq(address(ordVault).balance + treasury.balance, total);
        assertEq(ordVault.contributedBy(coinA), address(ordVault).balance);
        assertEq(ordVault.attributed() + ordVault.waiting(), address(ordVault).balance);
        assertGe(address(ordVault).balance * 10_000, total * 8_000 - 3_000 * 10_000, "80%, less at most a wei a harvest");
    }

    // ---- what held: the vault -------------------------------------------------------------------------------------------------------

    function test_held_forceFedEthIsNeverSpendable_andBreaksNothing() public {
        _fund(ordVault, coinA, 1 ether);
        new ForceFeeder{value: 5 ether}(payable(address(ordVault)));
        assertEq(address(ordVault).balance, 6 ether);
        assertEq(ordVault.attributed(), 1 ether);
        _cap(ordVault, 10 ether);
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(AssetVault.Insufficient.selector, 2 ether, 1 ether));
        ordVault.declarePurchase(2 ether, 1, "ref");
        _released(ordVault, 1 ether, 1);
        assertEq(address(ordVault).balance, 5 ether, "what was forced in stays, unattributed, for ever");
    }

    function test_held_everyIllegalTransitionIsRefused() public {
        _fund(ordVault, coinA, 3 ether);
        _cap(ordVault, 1 ether);
        vm.prank(keeper); uint256 id = ordVault.declarePurchase(1 ether, 1, "ref");
        vm.prank(attestor); vm.expectRevert(AssetVault.WrongStatus.selector); ordVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0));
        vm.expectRevert(AssetVault.WrongStatus.selector); ordVault.writeOff(id, bytes32(0));
        vm.expectRevert(AssetVault.WrongStatus.selector); ordVault.returnUnspent{value: 1}(id);
        Clock.skip(1 hours);
        ordVault.release(id);
        vm.expectRevert(AssetVault.WrongStatus.selector); ordVault.release(id);                                // not twice
        vm.prank(owner); vm.expectRevert(AssetVault.WrongStatus.selector); ordVault.cancel(id);               // not after the money left
        vm.prank(owner); vm.expectRevert(AssetVault.NotOverdue.selector); ordVault.writeOff(id, bytes32(0));  // not before the deadline
        vm.prank(attestor); ordVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0));
        vm.prank(attestor); vm.expectRevert(AssetVault.WrongStatus.selector); ordVault.settleInscription(id, ids[1], TXID, 1, _proofOf(1));
        Clock.skip(30 days);
        vm.expectRevert(AssetVault.WrongStatus.selector); ordVault.writeOff(id, bytes32(0));                   // a settled purchase is never written off
    }

    function test_held_aCapThatExpiresAfterDeclarationDoesNotStopTheRelease_whichIsBoundedByTheCapOfItsOwnHour() public {
        _fund(ordVault, coinA, 1 ether);
        _cap(ordVault, 1 ether);
        vm.prank(keeper); uint256 id = ordVault.declarePurchase(1 ether, 1, "ref");
        Clock.skip(3 hours);                                                             // the cap is long dead
        vm.prank(keeper); vm.expectRevert(AssetVault.CapExpired.selector); ordVault.declarePurchase(1, 1, "ref");
        ordVault.release(id);
        assertEq(purchaser.balance, 1 ether);
    }

    // ---- what held: Merkle leaves ---------------------------------------------------------------------------------------------------

    function test_held_anInnerNodeCannotBePassedOffAsALeaf_norATicketLeafAsAShareLeaf() public view {
        bytes32 a = distributor.ticketLeaf(address(0xA), 0, 10);
        bytes32 b = distributor.ticketLeaf(address(0xB), 10, 20);
        bytes32 node = a < b ? keccak256(abi.encodePacked(a, b)) : keccak256(abi.encodePacked(b, a));
        // a leaf is keccak(keccak(96 bytes)) = keccak of 32 bytes; a node is keccak of 64 bytes: no leaf preimage is a node preimage
        assertTrue(node != a && node != b);
        assertTrue(distributor.shareLeaf(address(0xA), 10) != distributor.ticketLeaf(address(0xA), 0, 10));
        assertTrue(registry.leafOf("aaaa1111i0") != distributor.shareLeaf(address(0xA), 10));
    }

    function test_held_aWinCannotBeClaimedTwice_orWithAnotherDrawsRange() public {
        tickets.push(distributor.ticketLeaf(alice, 0, 60)); tickets.push(distributor.ticketLeaf(bob, 60, 100));
        uint256 aid = _lot();
        vm.prank(keeper); distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://t");
        Clock.skip(15 minutes);
        distributor.requestDraw(0, aid);
        distributor.resolve(0, aid, abi.encodePacked(bytes32(uint256(3))));
        uint256 w = distributor.lot(0, aid).winningTicket;
        (address who, uint256 f, uint256 e, uint256 idx) = w < 60 ? (alice, uint256(0), uint256(60), uint256(0)) : (bob, uint256(60), uint256(100), uint256(1));
        (address other, uint256 ofirst, uint256 oe, uint256 oidx) = w < 60 ? (bob, uint256(60), uint256(100), uint256(1)) : (alice, uint256(0), uint256(60), uint256(0));
        vm.expectRevert(Distributor.BadProof.selector); distributor.claimWin(0, aid, other, ofirst, oe, Merkle.proof(tickets, oidx));
        vm.expectRevert(Distributor.BadProof.selector); distributor.claimWin(0, aid, who, f, e + 1, Merkle.proof(tickets, idx));   // a stretched range
        distributor.claimWin(0, aid, who, f, e, Merkle.proof(tickets, idx));
        vm.expectRevert(Distributor.WrongStage.selector); distributor.claimWin(0, aid, who, f, e, Merkle.proof(tickets, idx));
    }

    function test_held_aLateFulfilmentAndARedrawCannotBothHappen() public {
        tickets.push(distributor.ticketLeaf(alice, 0, 60)); tickets.push(distributor.ticketLeaf(bob, 60, 100));
        uint256 aid = _lot();
        vm.prank(keeper); distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://t");
        Clock.skip(15 minutes);
        distributor.requestDraw(0, aid);
        uint64 first = distributor.lot(0, aid).round;
        vm.expectRevert(Distributor.TooEarly.selector); distributor.redraw(0, aid);
        Clock.skip(24 hours);
        distributor.redraw(0, aid);
        assertGt(distributor.lot(0, aid).round, first);
        distributor.resolve(0, aid, abi.encodePacked(bytes32(uint256(9))));          // proves the NEW round; the old one no longer counts
        vm.expectRevert(Distributor.WrongStage.selector); distributor.redraw(0, aid);
    }

    // ---- L4: addresses ----------------------------------------------------------------------------------------------------------------

    function test_L4_anAddressWithNonZeroPaddingIsRefused() public {
        Addr a = new Addr();
        assertTrue(a.ok(TAPROOT));
        assertFalse(a.ok("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj1"));   // a wrong checksum
        assertFalse(a.ok("BC1P0XLXVLHEMJA6C4DQV22UAPCTQUPFHLXM9H8Z3K2E72Q4K9HCZ7VQZK5JJ0"));   // valid on Bitcoin, refused here: upper case
        assertFalse(a.ok("bc1qw508d4qejxtdg4y5r3zarvary0c5xw7kv8f3t4"));                       // valid on Bitcoin, refused here: segwit v0
    }
}
