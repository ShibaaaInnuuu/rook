// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "./utils/Base.sol";
import {Merkle} from "./utils/Merkle.sol";
import {AssetVault} from "../src/AssetVault.sol";
import {Distributor} from "../src/Distributor.sol";
import {DrandEvmnet} from "../src/rand/DrandEvmnet.sol";
import {Clock} from "./utils/Clock.sol";

contract DistributorTest is Base {
    string constant TAPROOT = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";
    bytes32 constant TXID = keccak256("a bitcoin txid");
    address alice = makeAddr("alice"); address bob = makeAddr("bob"); address carol = makeAddr("carol");
    bytes32[] tickets;     // alice [0,60) bob [60,90) carol [90,100)
    bytes32[] shares;      // alice 60_000, bob 30_000, carol 9_000 (of 100_000: a root that promises LESS than was bought is allowed)

    function setUp() public override {
        super.setUp();
        launcher.set(coinA, router, creatorA, 0);
        tickets.push(distributor.ticketLeaf(alice, 0, 60));
        tickets.push(distributor.ticketLeaf(bob, 60, 90));
        tickets.push(distributor.ticketLeaf(carol, 90, 100));
        shares.push(distributor.shareLeaf(alice, 60_000));
        shares.push(distributor.shareLeaf(bob, 30_000));
        shares.push(distributor.shareLeaf(carol, 9_000));
    }

    function _inscription() internal returns (uint256 aid) {
        _fund(ordVault, coinA, 1 ether);
        vm.prank(keeper); ordVault.postCap(1 ether);
        vm.prank(keeper); uint256 id = ordVault.declarePurchase(1 ether, 1, "f");
        Clock.skip(1 hours);
        ordVault.release(id);
        vm.prank(attestor);
        aid = ordVault.settleInscription(id, ids[0], TXID, 1, _proofOf(0));
    }

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

    function _openedDraw() internal returns (uint256 aid) {
        aid = _inscription();
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.DrawWeighted, address(0));
        vm.prank(keeper);
        distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://tickets");
    }

    // ---- the creator's choice ----

    function test_onlyThePayingCoinsCreatorChooses_andOnlyOnce() public {
        uint256 aid = _inscription();
        address[4] memory who = [creatorB, owner, keeper, stranger];
        for (uint256 i = 0; i < who.length; ++i) {
            vm.prank(who[i]);
            vm.expectRevert(Distributor.NotCreator.selector);
            distributor.choose(0, aid, Distributor.Mode.Hold, address(0));
        }
        vm.startPrank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.Hold, address(0));           // hold can be revisited…
        distributor.choose(0, aid, Distributor.Mode.Never, address(0));          // …anything else is final
        vm.expectRevert(Distributor.WrongStage.selector);
        distributor.choose(0, aid, Distributor.Mode.DrawWeighted, address(0));
        vm.stopPrank();
    }

    function test_anInscriptionCannotBeSharedProRata_andANamedChoiceNeedsAName() public {
        uint256 aid = _inscription();
        vm.startPrank(creatorA);
        vm.expectRevert(Distributor.WrongMode.selector);
        distributor.choose(0, aid, Distributor.Mode.ProRata, address(0));
        vm.expectRevert(Distributor.BadInput.selector);
        distributor.choose(0, aid, Distributor.Mode.Named, address(0));
        vm.expectRevert(Distributor.BadInput.selector);
        distributor.choose(0, aid, Distributor.Mode.Hold, alice);
        distributor.choose(0, aid, Distributor.Mode.Named, alice);
        vm.stopPrank();
        assertEq(distributor.lot(0, aid).winner, alice);
    }

    function test_aLotNobodyChoseForCannotBeDrawn() public {
        uint256 aid = _inscription();
        vm.prank(keeper);
        vm.expectRevert(Distributor.WrongMode.selector);
        distributor.openDraw(0, aid, Merkle.root(tickets), 100, "u");
    }

    // ---- a draw ----

    function test_noDrawDuringTheChallengeWindow_thenAnyonePins_aRoundThatDoesNotExistYet() public {
        uint256 aid = _openedDraw();
        vm.expectRevert();
        distributor.requestDraw(0, aid);
        Clock.skip(15 minutes - 1);
        vm.expectRevert();
        distributor.requestDraw(0, aid);
        Clock.skip(1);
        vm.prank(stranger);
        distributor.requestDraw(0, aid);
        Distributor.Lot memory l = distributor.lot(0, aid);
        assertGt(beacon.timeOf(l.round), vm.getBlockTimestamp(), "the pinned round is in the future");
        assertEq(beacon.randomnessOf(l.round), bytes32(0));
        // pinned: it cannot be pinned again, cancelled, or reopened
        vm.expectRevert(Distributor.WrongStage.selector);
        distributor.requestDraw(0, aid);
        vm.prank(owner);
        vm.expectRevert(Distributor.WrongStage.selector);
        distributor.cancelDraw(0, aid);
        vm.prank(keeper);
        vm.expectRevert(Distributor.WrongStage.selector);
        distributor.openDraw(0, aid, Merkle.root(tickets), 100, "u");
    }

    function test_theOwnerCancelsOnlyInsideTheWindow_andOnlyTheOwner() public {
        uint256 aid = _openedDraw();
        vm.prank(keeper);
        vm.expectRevert(Distributor.NotOwner.selector);
        distributor.cancelDraw(0, aid);
        Clock.skip(15 minutes);
        vm.prank(owner);
        vm.expectRevert(Distributor.WindowOver.selector);
        distributor.cancelDraw(0, aid);

        uint256 aid2 = _openedDrawSecond();
        vm.prank(owner);
        distributor.cancelDraw(0, aid2);
        vm.prank(keeper);                                        // the keeper may publish a corrected list
        distributor.openDraw(0, aid2, Merkle.root(tickets), 100, "ipfs://tickets-v2");
    }

    function _openedDrawSecond() internal returns (uint256 aid) {
        _fund(ordVault, coinA, 1 ether);
        vm.prank(keeper); ordVault.postCap(1 ether);
        vm.prank(keeper); uint256 id = ordVault.declarePurchase(1 ether, 1, "f");
        Clock.skip(1 hours);
        ordVault.release(id);
        vm.prank(attestor);
        aid = ordVault.settleInscription(id, ids[1], TXID, 1, _proofOf(1));
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.DrawPerWallet, address(0));
        vm.prank(keeper);
        distributor.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://tickets");
    }

    function test_anyoneDelivers_theWinnerIsWhoeverHoldsTheTicket_andNobodyElseCanClaim() public {
        uint256 aid = _openedDraw();
        Clock.skip(15 minutes);
        distributor.requestDraw(0, aid);
        bytes32 randomness = keccak256("the round's value");
        vm.prank(stranger);
        distributor.resolve(0, aid, abi.encodePacked(randomness));
        uint256 t = distributor.lot(0, aid).winningTicket;
        assertEq(t, uint256(keccak256(abi.encode(randomness, uint256(0), aid))) % 100);
        vm.expectRevert(Distributor.WrongStage.selector);        // no second roll
        distributor.resolve(0, aid, abi.encodePacked(keccak256("another")));

        (address w, uint256 a, uint256 b, uint256 i) = t < 60 ? (alice, uint256(0), uint256(60), uint256(0)) : t < 90 ? (bob, uint256(60), uint256(90), uint256(1)) : (carol, uint256(90), uint256(100), uint256(2));
        // a holder whose range does not contain the ticket cannot claim, even with a valid proof of their own leaf
        (address x, uint256 xa, uint256 xb, uint256 xi) = i == 0 ? (bob, uint256(60), uint256(90), uint256(1)) : (alice, uint256(0), uint256(60), uint256(0));
        vm.expectRevert(Distributor.BadProof.selector);
        distributor.claimWin(0, aid, x, xa, xb, Merkle.proof(tickets, xi));
        // a made-up range that does contain it has no proof
        vm.expectRevert(Distributor.BadProof.selector);
        distributor.claimWin(0, aid, stranger, 0, 100, Merkle.proof(tickets, i));
        vm.prank(stranger);                                      // anyone may submit the real winner's proof
        distributor.claimWin(0, aid, w, a, b, Merkle.proof(tickets, i));
        assertEq(distributor.lot(0, aid).winner, w);
    }

    function test_aRoundSomebodyAlreadyProvedNeedsNoSignature() public {
        uint256 aid = _openedDraw();
        Clock.skip(15 minutes);
        distributor.requestDraw(0, aid);
        beacon.prove(distributor.lot(0, aid).round, abi.encodePacked(keccak256("v")));
        distributor.resolve(0, aid, "");
        assertEq(uint8(distributor.lot(0, aid).stage), uint8(Distributor.Stage.Resolved));
    }

    function test_redrawOnlyAfterADayOfSilence_andNotIfTheRoundWasProven() public {
        uint256 aid = _openedDraw();
        Clock.skip(15 minutes);
        distributor.requestDraw(0, aid);
        uint64 first = distributor.lot(0, aid).round;
        vm.expectRevert(Distributor.TooEarly.selector);
        distributor.redraw(0, aid);
        Clock.skip(24 hours);
        vm.prank(stranger);
        distributor.redraw(0, aid);
        uint64 second = distributor.lot(0, aid).round;
        assertGt(second, first);
        Clock.skip(24 hours);
        beacon.prove(second, abi.encodePacked(keccak256("late but proven")));
        vm.expectRevert(Distributor.TooEarly.selector);          // a proven round is never abandoned: no fishing for a better one
        distributor.redraw(0, aid);
    }

    // ---- delivery ----

    function test_onlyTheWinnerNamesTheAddress_itMustBeARealTaprootAddress_andLatenessIsFlaggable() public {
        uint256 aid = _inscription();
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.Named, alice);
        vm.prank(bob);
        vm.expectRevert(Distributor.NotWinner.selector);
        distributor.setDestination(0, aid, TAPROOT);
        vm.prank(keeper);
        vm.expectRevert(Distributor.NotWinner.selector);
        distributor.setDestination(0, aid, TAPROOT);
        vm.startPrank(alice);
        vm.expectRevert(Distributor.BadAddress.selector);
        distributor.setDestination(0, aid, "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj1");
        distributor.setDestination(0, aid, TAPROOT);
        vm.stopPrank();

        vm.expectRevert(Distributor.NotOverdue.selector);
        distributor.flagOverdue(0, aid);
        Clock.skip(7 days + 1);
        vm.prank(stranger);
        distributor.flagOverdue(0, aid);
        assertTrue(distributor.lot(0, aid).overdue);

        vm.prank(stranger);
        vm.expectRevert(Distributor.NotKeeper.selector);
        distributor.recordDelivery(0, aid, TXID);
        vm.prank(keeper);
        distributor.recordDelivery(0, aid, TXID);
        assertEq(uint8(distributor.lot(0, aid).stage), uint8(Distributor.Stage.Delivered));
        vm.prank(alice);
        vm.expectRevert(Distributor.WrongStage.selector);        // sent is sent
        distributor.setDestination(0, aid, TAPROOT);
    }

    function test_nothingIsDeliveredToAnAddressNobodyChose() public {
        uint256 aid = _inscription();
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.Named, alice);
        vm.prank(keeper);
        vm.expectRevert(Distributor.WrongStage.selector);
        distributor.recordDelivery(0, aid, TXID);
    }

    // ---- rune units, pro rata ----

    function _openedDistribution(uint256 amount) internal returns (uint256 aid) {
        launcher.set(coinA, router, creatorA, 1);
        aid = _runes(amount);
        vm.prank(creatorA);
        distributor.choose(1, aid, Distributor.Mode.ProRata, address(0));
        vm.prank(keeper);
        distributor.openDistribution(1, aid, Merkle.root(shares), "ipfs://shares");
    }

    function test_claimBeforeTheWindowEnds_twice_orWithAWrongProof_allFail() public {
        uint256 aid = _openedDistribution(100_000);
        vm.expectRevert();
        distributor.claimShare(1, aid, alice, 60_000, Merkle.proof(shares, 0));
        Clock.skip(15 minutes);
        vm.expectRevert(Distributor.BadProof.selector);
        distributor.claimShare(1, aid, alice, 60_001, Merkle.proof(shares, 0));
        vm.expectRevert(Distributor.BadProof.selector);
        distributor.claimShare(1, aid, stranger, 60_000, Merkle.proof(shares, 0));
        vm.prank(stranger);                                      // anyone may submit it; it only ever credits alice
        distributor.claimShare(1, aid, alice, 60_000, Merkle.proof(shares, 0));
        assertEq(distributor.accrued(1, alice), 60_000);
        vm.expectRevert(Distributor.AlreadyClaimed.selector);
        distributor.claimShare(1, aid, alice, 60_000, Merkle.proof(shares, 0));
    }

    function test_aRootThatPromisesMoreThanWasBoughtCannotPayItOut() public {
        uint256 aid = _openedDistribution(80_000);                // the list adds to 99,000; only 80,000 arrived
        Clock.skip(15 minutes);
        distributor.claimShare(1, aid, alice, 60_000, Merkle.proof(shares, 0));
        vm.expectRevert(Distributor.OverTotal.selector);
        distributor.claimShare(1, aid, bob, 30_000, Merkle.proof(shares, 1));
        distributor.claimShare(1, aid, carol, 9_000, Merkle.proof(shares, 2));
    }

    function test_underTheMinimumNothingIsSent_itAccruesUntilItCrosses() public {
        uint256 aid = _openedDistribution(100_000);
        Clock.skip(15 minutes);
        distributor.claimShare(1, aid, carol, 9_000, Merkle.proof(shares, 2));
        vm.prank(carol);
        vm.expectRevert(Distributor.MinimumExpired.selector);     // no minimum posted: nothing is payable at all
        distributor.requestPayout(1, TAPROOT);
        vm.prank(keeper);
        distributor.postMinimum(1, 10_000);
        vm.prank(carol);
        vm.expectRevert(abi.encodeWithSelector(Distributor.BelowMinimum.selector, 9_000, 10_000));
        distributor.requestPayout(1, TAPROOT);

        // a second distribution for the same coin: carol's share accrues on top, and now crosses the minimum
        uint256 aid2 = _runes(100_000);
        vm.prank(creatorA); distributor.choose(1, aid2, Distributor.Mode.ProRata, address(0));
        vm.prank(keeper); distributor.openDistribution(1, aid2, Merkle.root(shares), "ipfs://shares-2");
        Clock.skip(15 minutes);
        distributor.claimShare(1, aid2, carol, 9_000, Merkle.proof(shares, 2));
        vm.prank(keeper); distributor.postMinimum(1, 10_000);
        vm.startPrank(carol);
        vm.expectRevert(Distributor.BadAddress.selector);
        distributor.requestPayout(1, "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4");
        uint256 pid = distributor.requestPayout(1, TAPROOT);
        vm.stopPrank();
        assertEq(distributor.payout(pid).amount, 18_000);
        assertEq(distributor.accrued(1, carol), 0);
        vm.prank(carol);
        vm.expectRevert();                                        // nothing left to ask for
        distributor.requestPayout(1, TAPROOT);

        vm.expectRevert(Distributor.NotOverdue.selector);
        distributor.flagPayoutOverdue(pid);
        vm.prank(keeper);
        distributor.recordPayout(pid, TXID);
        vm.prank(keeper);
        vm.expectRevert(Distributor.BadInput.selector);           // a payout is recorded once
        distributor.recordPayout(pid, TXID);
    }

    function test_anExpiredMinimumPaysNothing() public {
        uint256 aid = _openedDistribution(100_000);
        Clock.skip(15 minutes);
        distributor.claimShare(1, aid, alice, 60_000, Merkle.proof(shares, 0));
        vm.prank(keeper); distributor.postMinimum(1, 1_000);
        Clock.skip(24 hours + 1);
        vm.prank(alice);
        vm.expectRevert(Distributor.MinimumExpired.selector);
        distributor.requestPayout(1, TAPROOT);
    }

    function test_unclaimedUnitsRollIntoTheCoinsNextDistribution_afterClaimsClose_notBefore() public {
        uint256 aid = _openedDistribution(100_000);
        Clock.skip(15 minutes);
        distributor.claimShare(1, aid, alice, 60_000, Merkle.proof(shares, 0));
        vm.expectRevert(Distributor.TooEarly.selector);
        distributor.rollOver(1, aid);
        Clock.skip(30 days);
        vm.expectRevert(Distributor.ClaimsClosed.selector);
        distributor.claimShare(1, aid, bob, 30_000, Merkle.proof(shares, 1));
        vm.prank(stranger);
        distributor.rollOver(1, aid);
        assertEq(distributor.carry(1, coinA), 40_000);
        vm.expectRevert(Distributor.WrongStage.selector);
        distributor.rollOver(1, aid);

        uint256 aid2 = _runes(50_000);
        vm.prank(creatorA); distributor.choose(1, aid2, Distributor.Mode.ProRata, address(0));
        vm.prank(keeper); distributor.openDistribution(1, aid2, Merkle.root(shares), "ipfs://shares-2");
        assertEq(distributor.distribution(1, aid2).total, 90_000, "this lot plus what rolled over; the chain fixes the total, not the keeper");
        assertEq(distributor.carry(1, coinA), 0);
    }

    function test_aCancelledDistributionGivesTheCarryBack() public {
        uint256 aid = _openedDistribution(100_000);
        Clock.skip(15 minutes + 30 days);
        distributor.rollOver(1, aid);                             // 100,000 carried
        uint256 aid2 = _runes(10_000);
        vm.prank(creatorA); distributor.choose(1, aid2, Distributor.Mode.ProRata, address(0));
        vm.prank(keeper); distributor.openDistribution(1, aid2, Merkle.root(shares), "u");
        assertEq(distributor.carry(1, coinA), 0);
        vm.prank(owner);
        distributor.cancelDistribution(1, aid2);
        assertEq(distributor.carry(1, coinA), 100_000);
        vm.prank(keeper);
        distributor.openDistribution(1, aid2, Merkle.root(shares), "u2");
        assertEq(distributor.distribution(1, aid2).total, 110_000);
    }

    function test_aRuneLotCanBeRaffledWholeInstead() public {
        launcher.set(coinA, router, creatorA, 1);
        uint256 aid = _runes(100_000);
        vm.prank(creatorA);
        distributor.choose(1, aid, Distributor.Mode.DrawPerWallet, address(0));
        vm.prank(keeper);
        vm.expectRevert(Distributor.WrongMode.selector);          // and then it is not also shared out
        distributor.openDistribution(1, aid, Merkle.root(shares), "u");
        vm.prank(keeper);
        distributor.openDraw(1, aid, Merkle.root(tickets), 100, "u");
    }

    // ---- once, end to end, with the real verifier and a real beacon ----

    function test_aDrawSettledByARealDrandRound() public {
        DrandEvmnet real = new DrandEvmnet();
        Distributor d = new Distributor(registry, real, PBS);
        Clock.warpTo(real.timeOf(20823703 - 20) - 3 hours);            // so that the whole flow ends exactly where round 20,823,703 is PIN_AHEAD ahead
        uint256 aid = _inscription();
        vm.prank(creatorA);
        d.choose(0, aid, Distributor.Mode.DrawWeighted, address(0));
        vm.prank(keeper);
        d.openDraw(0, aid, Merkle.root(tickets), 100, "ipfs://tickets");
        // arrange the clock so that the round pinned (current + PIN_AHEAD, docs/phase3.md M2) is round 20,823,703, whose signature we hold
        Clock.warpTo(real.timeOf(20823703 - 20));
        d.requestDraw(0, aid);
        assertEq(d.lot(0, aid).round, 20823703);
        vm.expectRevert();                                        // round 1000's real signature is not this round's
        d.resolve(0, aid, hex"06fd5996329504d3a56b482d9222bf7205857d0a9559ddd216ca31a286f6a8cc0a120f021aac2f13553fb164f62bc3a5ca32c76dea88a777b39bcf3cac5fdbd6");
        vm.prank(stranger);
        d.resolve(0, aid, hex"205ff8f2b2cb01f39deb17fe64eed496207c8fa1a490e5bf0db027ced4665bcf22504bf545a9880b36617734cede20622276674ec0c5599561ac68e8f5432ad6");
        bytes32 r = 0x13b91b6a08f04051999beeca41f8ce3f99e1bf8004a128ba49e3fa27f0c4ba25;
        assertEq(d.lot(0, aid).winningTicket, uint256(keccak256(abi.encode(r, uint256(0), aid))) % 100);
    }
}
