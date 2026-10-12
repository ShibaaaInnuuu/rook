// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase, MockVenue} from "./utils/SweepBase.sol";
import {Merkle} from "./utils/Merkle.sol";
import {Clock} from "./utils/Clock.sol";
import {SweepVault} from "../src/SweepVault.sol";
import {Wrapper} from "../src/Wrapper.sol";
import {BtcAddress} from "../src/lib/BtcAddress.sol";
import {Wait} from "../src/lib/Wait.sol";

/// docs/v3/sweep.md: money in, the sweep, the raffle and the burn, one behaviour per test.
contract SweepVaultTest is SweepBase {
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address carol = makeAddr("carol");

    bytes32[] leaves;
    bytes32[] tree;                    // the published tree of the last draw opened: `leaves` then the sentinel
    uint256 constant TOTAL = 10;       // alice [0,5), bob [5,8), carol [8,10)

    function setUp() public override {
        super.setUp();
        _stockVenue(20);
        leaves.push(raffle.leaf(alice, 0, 5));
        leaves.push(raffle.leaf(bob, 5, 8));
        leaves.push(raffle.leaf(carol, 8, 10));
    }

    // ---- construction ----

    function test_constructorRefusesAVenueThatIsNotAContract() public {
        vm.expectRevert(SweepVault.BadVenue.selector);
        new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, stranger, launcher, desk, PBS);
    }

    function test_constructorRefusesTheWrapperAsVenue() public {
        vm.expectRevert(SweepVault.BadVenue.selector);
        new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(wrapper), launcher, desk, PBS);
    }

    /// Any wrapped collection is accepted (docs/v4/nft-wrapper.md §3): what the vault checks is that it has a custodian, and
    /// that the parent block time is one somebody measured.
    function test_constructorRefusesAWrappedTokenWithoutACustodianAndAnUnmeasuredBlockTime() public {
        vm.mockCall(address(wrapper), abi.encodeWithSignature("custodian()"), abi.encode(address(0)));
        vm.expectRevert(SweepVault.BadInput.selector);
        new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(venue), launcher, desk, PBS);
        vm.clearMockedCalls();
        vm.expectRevert(abi.encodeWithSignature("BadParentBlockSeconds(uint256)", 0));
        new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(venue), launcher, desk, 0);
    }

    function test_constructionFixesPolicyVenueAndBeacon() public view {
        assertEq(uint8(raffle.policy()), uint8(SweepVault.Policy.RAFFLE));
        assertEq(uint8(burn.policy()), uint8(SweepVault.Policy.BURN));
        assertEq(raffle.venue(), address(venue));
        assertEq(address(raffle.beacon()), address(beacon));
    }

    function test_burnAddressIsAValidTaprootAddress() public view {
        assertTrue(BtcAddress.isTaproot(wrapper.BURN_ADDRESS()));
    }

    // ---- money in ----

    function test_depositCreditsTheCoinFromItsOwnRouter() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        assertEq(raffle.credit(coinR), 1 ether);
        assertEq(raffle.totalCredit(), 1 ether);
    }

    function test_depositRefusesAnotherCoinsRouter() public {
        vm.prank(routerR2);
        vm.expectRevert(SweepVault.NotRouter.selector);
        raffle.deposit{value: 1 ether}(coinR);
    }

    function test_depositRefusesACoinOnAnotherListing() public {
        // coinX's router is real, but coinX is on the burn listing: raffle money and burn money never mix
        vm.prank(routerX);
        vm.expectRevert(SweepVault.NotRouter.selector);
        raffle.deposit{value: 1 ether}(coinX);
    }

    function test_depositFromTheTreasuryIsRefused() public {
        vm.deal(treasury, 1 ether);
        vm.prank(treasury);
        vm.expectRevert(SweepVault.NotAccepted.selector);
        raffle.depositFrom{value: 1 ether}(coinR);
    }

    function test_plainEthIsRefused() public {
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        (bool ok,) = address(raffle).call{value: 1 ether}("");
        assertFalse(ok);
        vm.deal(address(venue), 1 ether);
        vm.prank(address(venue));
        (ok,) = address(raffle).call{value: 1 ether}("");
        assertFalse(ok, "even the venue, outside a sweep");
    }

    function test_depositWrappedCreditsUnitsFromTheRouter() public {
        _deposit(routerR);
        vm.prank(routerR);
        wrapper.approve(address(raffle), ONE);
        vm.prank(routerR);
        raffle.depositWrapped(coinR, ONE);
        assertEq(raffle.units(coinR), ONE);
        assertEq(wrapper.balanceOf(address(raffle)), ONE);
    }

    function test_depositWrappedRefusesAStranger() public {
        _deposit(stranger);
        vm.prank(stranger);
        wrapper.approve(address(raffle), ONE);
        vm.prank(stranger);
        vm.expectRevert(SweepVault.NotRouter.selector);
        raffle.depositWrapped(coinR, ONE);
    }

    function test_directWrappedTransferIsStrayAndCreditsNobody() public {
        _deposit(stranger);
        vm.prank(stranger);
        wrapper.transfer(address(raffle), ONE);
        assertEq(raffle.units(coinR), 0);
        assertEq(raffle.stray(), ONE);
        vm.prank(keeper);
        vm.expectRevert(SweepVault.Insufficient.selector);
        raffle.announceDraw(coinR);
    }

    // ---- the cap ----

    function test_postCapIsTheKeepersOnly() public {
        vm.prank(stranger);
        vm.expectRevert(SweepVault.NotKeeper.selector);
        raffle.postCap(PRICE);
    }

    function test_capExpiresAfterAnHourOnTheSequencersClock() public {
        _fundCoin(raffle, coinR, routerR, PRICE);
        _postCap(raffle, PRICE);
        Clock.skipTimeOnly(1 hours);
        bytes memory data = abi.encodeCall(MockVenue.buy, (PRICE));
        vm.expectRevert(SweepVault.CapStale.selector);
        raffle.sweep(coinR, PRICE, data, ONE);
    }

    function test_capExpiresAfterAnHourOfParentBlocks() public {
        _fundCoin(raffle, coinR, routerR, PRICE);
        _postCap(raffle, PRICE);
        vm.roll(vm.getBlockNumber() + Wait.blocksFor(1 hours));   // the sequencer's clock stands still; the parent chain's moved on
        bytes memory data = abi.encodeCall(MockVenue.buy, (PRICE));
        vm.expectRevert(SweepVault.CapStale.selector);
        raffle.sweep(coinR, PRICE, data, ONE);
    }

    function test_noCapNoSweep() public {
        _fundCoin(raffle, coinR, routerR, PRICE);
        bytes memory data = abi.encodeCall(MockVenue.buy, (PRICE));
        vm.expectRevert(SweepVault.CapStale.selector);
        raffle.sweep(coinR, PRICE, data, ONE);
    }

    // ---- the sweep ----

    function test_sweepBuysAtOrBelowTheCapAndChargesTheCoin() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _fundCoin(raffle, coinR2, routerR2, 1 ether);
        _postCap(raffle, PRICE);
        vm.prank(stranger);
        (uint256 spent, uint256 bought) = raffle.sweep(coinR, 2 * PRICE, abi.encodeCall(MockVenue.buy, (PRICE)), 2 * ONE);
        assertEq(spent, 2 * PRICE);
        assertEq(bought, 2 * ONE);
        assertEq(raffle.credit(coinR), 1 ether - 2 * PRICE);
        assertEq(raffle.units(coinR), 2 * ONE);
        assertEq(raffle.credit(coinR2), 1 ether, "the other coin is untouched");
        assertEq(raffle.units(coinR2), 0);
    }

    function test_sweepAboveTheCapReverts() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, PRICE);
        bytes memory data = abi.encodeCall(MockVenue.buy, (PRICE + 1));
        vm.expectRevert();
        raffle.sweep(coinR, PRICE, data, 1);
    }

    function test_sweepBelowMinOutReverts() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, PRICE);
        bytes memory data = abi.encodeCall(MockVenue.buy, (PRICE));
        vm.expectRevert(abi.encodeWithSelector(SweepVault.Slippage.selector, ONE, ONE + 1));
        raffle.sweep(coinR, PRICE, data, ONE + 1);
    }

    function test_sweepCannotSpendMoreThanTheCoinsCredit() public {
        _fundCoin(raffle, coinR, routerR, PRICE);
        _fundCoin(raffle, coinR2, routerR2, 10 ether);
        _postCap(raffle, PRICE);
        bytes memory data = abi.encodeCall(MockVenue.buy, (PRICE));
        vm.expectRevert(SweepVault.Insufficient.selector);
        raffle.sweep(coinR, 2 * PRICE, data, ONE);
    }

    function test_sweepChargesOnlyWhatWasSpentWhenTheVenueRefunds() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, PRICE);
        (uint256 spent, uint256 bought) =
            raffle.sweep(coinR, 3 * PRICE, abi.encodeCall(MockVenue.buyAndRefund, (PRICE, PRICE)), ONE);
        assertEq(spent, PRICE);
        assertEq(bought, ONE);
        assertEq(raffle.credit(coinR), 1 ether - PRICE);
        assertEq(address(raffle).balance, 1 ether - PRICE);
    }

    function test_aVenueThatKeepsTheEthGetsNothingThrough() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, PRICE);
        bytes memory data = abi.encodeCall(MockVenue.keep, ());
        vm.expectRevert(abi.encodeWithSelector(SweepVault.Slippage.selector, 0, 1));
        raffle.sweep(coinR, PRICE, data, 1);
    }

    function test_aRouteThatSendsTheWrappedElsewhereReverts() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, PRICE);
        bytes memory data = abi.encodeCall(MockVenue.buyFor, (PRICE, stranger));
        vm.expectRevert(abi.encodeWithSelector(SweepVault.Slippage.selector, 0, 1));
        raffle.sweep(coinR, PRICE, data, 1);
    }

    function test_aVenueThatRefundsMoreThanItWasSentReverts() public {
        vm.deal(address(venue), 1 ether);
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, PRICE);
        bytes memory data = abi.encodeCall(MockVenue.overRefund, ());
        vm.expectRevert(SweepVault.VenueFailed.selector);
        raffle.sweep(coinR, PRICE, data, 1);
    }

    function test_theVenueCannotPullTheVaultsWrapped() public {
        _sweepWhole(raffle, coinR, routerR, 2);
        _fundCoin(raffle, coinR, routerR, PRICE);
        bytes memory data = abi.encodeCall(MockVenue.steal, (ONE));
        vm.expectRevert(SweepVault.VenueFailed.selector);   // no allowance: the vault approves nobody
        raffle.sweep(coinR, PRICE, data, 1);
        assertEq(wrapper.balanceOf(address(raffle)), 2 * ONE);
        assertEq(wrapper.allowance(address(raffle), address(venue)), 0);
    }

    function test_theVenueCannotReenter() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, PRICE);
        bytes memory data = abi.encodeCall(MockVenue.reenter, (coinR));
        vm.expectRevert(SweepVault.VenueFailed.selector);
        raffle.sweep(coinR, PRICE, data, 1);
    }

    function test_sweepOnlyEverCallsTheVenue() public {
        // swapData is the caller's, but the target is not: a call aimed at the wrapper's `transfer` goes to the venue, which
        // has no such function, and reverts
        _sweepWhole(raffle, coinR, routerR, 1);
        _fundCoin(raffle, coinR, routerR, PRICE);
        _postCap(raffle, PRICE);
        bytes memory data = abi.encodeWithSignature("transfer(address,uint256)", stranger, ONE);
        vm.expectRevert(SweepVault.VenueFailed.selector);
        raffle.sweep(coinR, PRICE, data, 1);
        assertEq(wrapper.balanceOf(stranger), 0);
    }

    function test_sweepRefusedWhileTheListingIsPaused() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, PRICE);
        vm.prank(owner);
        registry.setPaused(RAFFLE_LISTING, true);
        bytes memory data = abi.encodeCall(MockVenue.buy, (PRICE));
        vm.expectRevert(SweepVault.ListingPaused.selector);
        raffle.sweep(coinR, PRICE, data, ONE);
    }

    function testFuzz_spendNeverExceedsCapTimesBought(uint96 credit_, uint96 spend, uint64 price, uint64 cap_) public {
        vm.assume(credit_ > 0 && spend > 0 && price > 1e9 && cap_ > 0);
        vm.assume(uint256(spend) * 1e18 / price <= 20 * ONE && spend <= credit_);
        vm.deal(routerR, credit_);
        _fundCoin(raffle, coinR, routerR, credit_);
        _postCap(raffle, cap_);
        try raffle.sweep(coinR, spend, abi.encodeCall(MockVenue.buy, (price)), 1) returns (uint256 spent, uint256 bought) {
            assertLe(spent * ONE, uint256(cap_) * bought, "spent <= cap x bought");
            assertLe(spent, spend);
            assertEq(raffle.credit(coinR) + spent, credit_);
            assertEq(raffle.units(coinR), bought);
        } catch {
            assertEq(raffle.credit(coinR), credit_, "a refused sweep moves nothing");
            assertEq(raffle.units(coinR), 0);
        }
        _assertVaultBooks(raffle);
    }

    // ---- RAFFLE ----

    function _open(address coin) internal returns (uint256 id) {
        bytes32[] memory t;
        (id, t) = _openDraw(raffle, coin, leaves, TOTAL);
        tree = t;
    }

    function _toResolved(uint256 id, bytes32 r) internal {
        Clock.skip(15 minutes);
        raffle.requestDraw(id);
        raffle.resolve(id, abi.encodePacked(r));
    }

    function _winnerIndex(uint256 w) internal pure returns (uint256) {
        return w < 5 ? 0 : w < 8 ? 1 : 2;
    }

    function test_raffleEndToEnd() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        uint256 id = _open(coinR);
        assertEq(raffle.units(coinR), 0);
        assertEq(raffle.totalReserved(), ONE);
        _toResolved(id, bytes32(uint256(7)));
        uint256 w = raffle.draw(id).winningTicket;
        uint256 k = _winnerIndex(w);
        address[3] memory who = [alice, bob, carol];
        uint256[4] memory edges = [uint256(0), 5, 8, 10];
        bytes32[] memory proof = Merkle.proof(tree, k);
        vm.prank(stranger);                                   // anyone may claim for the holder; the holder is paid
        raffle.claim(id, who[k], edges[k], edges[k + 1], proof);
        assertEq(wrapper.balanceOf(who[k]), ONE);
        assertEq(raffle.totalReserved(), 0);
        _assertVaultBooks(raffle);
    }

    function test_theWinningTicketIsTheRoundsAndNobodyElses() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        uint256 id = _open(coinR);
        bytes32 r = keccak256("some round");
        _toResolved(id, r);
        assertEq(raffle.draw(id).winningTicket, uint256(keccak256(abi.encode(r, address(raffle), id))) % TOTAL);
    }

    function test_aLosingRangeCannotClaim() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        uint256 id = _open(coinR);
        _toResolved(id, bytes32(uint256(7)));
        uint256 k = (_winnerIndex(raffle.draw(id).winningTicket) + 1) % 3;
        address[3] memory who = [alice, bob, carol];
        uint256[4] memory edges = [uint256(0), 5, 8, 10];
        bytes32[] memory proof = Merkle.proof(tree, k);
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.claim(id, who[k], edges[k], edges[k + 1], proof);
    }

    function test_aForgedRangeCannotClaim() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        uint256 id = _open(coinR);
        _toResolved(id, bytes32(uint256(7)));
        bytes32[] memory proof = Merkle.proof(tree, 0);
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.claim(id, stranger, 0, TOTAL, proof);
    }

    function test_claimOnce() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        uint256 id = _open(coinR);
        _toResolved(id, bytes32(uint256(7)));
        uint256 k = _winnerIndex(raffle.draw(id).winningTicket);
        address[3] memory who = [alice, bob, carol];
        uint256[4] memory edges = [uint256(0), 5, 8, 10];
        bytes32[] memory proof = Merkle.proof(tree, k);
        raffle.claim(id, who[k], edges[k], edges[k + 1], proof);
        vm.expectRevert(SweepVault.WrongStage.selector);
        raffle.claim(id, who[k], edges[k], edges[k + 1], proof);
    }

    function test_announceAndOpenAreTheKeepersOnly() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        vm.prank(stranger);
        vm.expectRevert(SweepVault.NotKeeper.selector);
        raffle.announceDraw(coinR);
        vm.prank(keeper);
        uint256 id = raffle.announceDraw(coinR);
        bytes32[] memory none;
        vm.prank(stranger);
        vm.expectRevert(SweepVault.NotKeeper.selector);
        raffle.openDraw(id, bytes32(uint256(1)), TOTAL, "x", none, abi.encodePacked(bytes32(uint256(5))));
    }

    function test_announceNeedsAWholeWrappedOfThatCoin() public {
        _sweepWhole(raffle, coinR2, routerR2, 1);            // the other coin has one; this one has none
        vm.prank(keeper);
        vm.expectRevert(SweepVault.Insufficient.selector);
        raffle.announceDraw(coinR);
    }

    /// C-04: the snapshot time is picked by the seed round, pinned when the draw is announced and not yet in existence, and lies in
    /// the day before the announcement.
    function test_theSeedRoundPicksTheSnapshotTimeInTheDayBefore() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        vm.prank(keeper);
        uint256 id = raffle.announceDraw(coinR);
        SweepVault.Draw memory d = raffle.draw(id);
        assertEq(uint8(d.stage), uint8(SweepVault.Stage.Announced));
        assertEq(d.seedRound, beacon.roundAt(vm.getBlockTimestamp()) + raffle.PIN_AHEAD());
        assertEq(beacon.randomnessOf(d.seedRound), bytes32(0), "nobody has seen the seed");
        for (uint256 i = 0; i < 16; ++i) {
            uint64 at = raffle.snapshotTimeOf(id, keccak256(abi.encode(i)));
            assertLe(at, d.announcedAt);
            assertGt(uint256(at) + raffle.SNAPSHOT_WINDOW(), d.announcedAt);
        }
    }

    /// C-03: the root must commit the ticket count and the snapshot time (the sentinel leaf).
    function test_openRefusesARootWithoutTheSentinelOrWithAnotherTotal() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        vm.prank(keeper);
        uint256 id = raffle.announceDraw(coinR);
        bytes32 seed = keccak256("seed");
        uint64 at = raffle.snapshotTimeOf(id, seed);
        bytes32[] memory t = new bytes32[](4);
        (t[0], t[1], t[2]) = (leaves[0], leaves[1], leaves[2]);
        t[3] = raffle.leaf(address(0), TOTAL, at);
        bytes32 root = Merkle.root(t);
        bytes32[] memory proof = Merkle.proof(t, 3);
        vm.startPrank(keeper);
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.openDraw(id, root, 5, "x", proof, abi.encodePacked(seed));          // the honest list, a truncated total
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.openDraw(id, Merkle.root(leaves), TOTAL, "x", Merkle.proof(leaves, 0), abi.encodePacked(seed));   // no sentinel
        raffle.openDraw(id, root, TOTAL, "x", proof, abi.encodePacked(seed));
        vm.stopPrank();
        assertEq(raffle.draw(id).snapshotAt, at);
        assertEq(uint8(raffle.draw(id).stage), uint8(SweepVault.Stage.Open));
    }

    function test_oneDrawPerCoinBeforeItsPin_andAnUnopenedAnnouncementVoids() public {
        _sweepWhole(raffle, coinR, routerR, 2);
        vm.prank(keeper);
        uint256 id = raffle.announceDraw(coinR);
        vm.prank(keeper);
        vm.expectRevert(SweepVault.DrawInFlight.selector);
        raffle.announceDraw(coinR);
        vm.expectRevert(SweepVault.TooEarly.selector);
        raffle.voidDraw(id);
        Clock.skip(6 hours);
        bytes32[] memory none;
        vm.prank(keeper);
        vm.expectRevert(SweepVault.WindowOver.selector);
        raffle.openDraw(id, bytes32(uint256(1)), TOTAL, "x", none, abi.encodePacked(bytes32(uint256(5))));
        raffle.voidDraw(id);                                  // anyone
        assertEq(raffle.units(coinR), 2 * ONE);
        vm.prank(keeper);
        raffle.announceDraw(coinR);                           // and the coin may be drawn again
    }

    function test_announceRefusedOnTheBurnVault() public {
        _sweepWhole(burn, coinX, routerX, 1);
        vm.prank(keeper);
        vm.expectRevert(SweepVault.WrongPolicy.selector);
        burn.announceDraw(coinX);
    }

    function test_requestDrawWaitsForTheWindowOnBothClocks() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        uint256 id = _open(coinR);
        Clock.skipTimeOnly(1 hours);                          // the sequencer runs an hour ahead; the parent chain has not moved
        vm.expectRevert(SweepVault.ChallengeWindow.selector);
        raffle.requestDraw(id);
    }

    function test_theOwnerCancelsInsideTheWindowAndTheWrappedGoesBack() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        uint256 id = _open(coinR);
        vm.prank(owner);
        raffle.cancelDraw(id);
        assertEq(raffle.units(coinR), ONE);
        assertEq(raffle.totalReserved(), 0);
        vm.expectRevert(SweepVault.WrongStage.selector);
        raffle.requestDraw(id);
    }

    /// The owner or the guardian cancels any draw before its pin (nobody knows the outcome then); after the pin, nobody can.
    function test_cancelIsTheOwnersOrTheGuardiansAndOnlyBeforeThePin() public {
        address guardian = makeAddr("guardian");
        vm.prank(owner);
        registry.setGuardian(guardian);
        _sweepWhole(raffle, coinR, routerR, 2);
        uint256 id = _open(coinR);
        vm.prank(keeper);
        vm.expectRevert(SweepVault.NotOwner.selector);
        raffle.cancelDraw(id);
        Clock.skip(15 minutes);
        vm.prank(guardian);
        raffle.cancelDraw(id);                                // past the window, still before the pin
        uint256 id2 = _open(coinR);
        Clock.skip(15 minutes);
        raffle.requestDraw(id2);
        vm.prank(owner);
        vm.expectRevert(SweepVault.WrongStage.selector);
        raffle.cancelDraw(id2);
    }

    function test_thePinnedRoundIsInTheFuture() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        uint256 id = _open(coinR);
        Clock.skip(15 minutes);
        raffle.requestDraw(id);
        SweepVault.Draw memory d = raffle.draw(id);
        assertEq(d.round, beacon.roundAt(vm.getBlockTimestamp()) + raffle.PIN_AHEAD());
        assertEq(beacon.randomnessOf(d.round), bytes32(0), "nobody has seen it");
    }

    /// C-02: the pinned round is final. There is no redraw; a draw nobody resolves for a week voids (both clocks), and until then
    /// anyone resolves it to the same winner.
    function test_thePinnedRoundIsFinal_andAnUnresolvedDrawVoidsAfterAWeek() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        uint256 id = _open(coinR);
        Clock.skip(15 minutes);
        raffle.requestDraw(id);
        uint64 round = raffle.draw(id).round;
        (bool ok,) = address(raffle).call(abi.encodeWithSignature("redraw(uint256)", id));
        assertFalse(ok, "there is no redraw");
        Clock.skipTimeOnly(8 days);
        vm.expectRevert(SweepVault.TooEarly.selector);
        raffle.voidDraw(id);                                  // the parent chain has not waited a week
        Clock.skip(7 days);
        assertEq(raffle.draw(id).round, round, "never re-pinned");
        raffle.voidDraw(id);
        assertEq(raffle.units(coinR), ONE);
        assertEq(raffle.totalReserved(), 0);
    }

    function test_voidAfterAWeekReturnsTheWrappedToTheCoin() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        uint256 id = _open(coinR);
        _toResolved(id, bytes32(uint256(7)));
        vm.expectRevert(SweepVault.TooEarly.selector);
        raffle.voidDraw(id);
        Clock.skipTimeOnly(8 days);
        vm.expectRevert(SweepVault.TooEarly.selector);
        raffle.voidDraw(id);
        Clock.skip(7 days);
        raffle.voidDraw(id);
        assertEq(raffle.units(coinR), ONE);
        assertEq(raffle.totalReserved(), 0);
    }

    // ---- BURN ----

    function test_retireRedeemsToTheBurnAddress() public {
        _sweepWhole(burn, coinX, routerX, 2);
        uint256 supply = wrapper.totalSupply();
        uint256 t = burn.retire(coinX);
        Wrapper.Ticket memory tk = wrapper.ticket(t);
        assertEq(tk.owner, address(burn));
        assertEq(tk.destination, wrapper.BURN_ADDRESS());
        assertEq(wrapper.totalSupply(), supply - ONE);
        assertEq(burn.units(coinX), ONE);
        assertEq(burn.retiredBy(t), coinX);
        _assertLedger();
        _resolveTicket(t, bytes32(uint256(3)));
        vm.prank(custodian);
        wrapper.recordDelivery(t, keccak256("burn tx"));
        _assertLedger();
        assertEq(wrapper.held(), 19, "the frog left: delivered to the NUMS output");
    }

    function test_retireRefusedOnTheRaffleVault() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        vm.expectRevert(SweepVault.WrongPolicy.selector);
        raffle.retire(coinR);
    }

    function test_retireNeedsAWholeWrapped() public {
        _fundCoin(burn, coinX, routerX, 1 ether);
        _postCap(burn, PRICE);
        burn.sweep(coinX, PRICE / 2, abi.encodeCall(MockVenue.buy, (PRICE)), 1);
        vm.expectRevert(SweepVault.Insufficient.selector);
        burn.retire(coinX);
    }

    function test_anUndeliveredRetirementIsRefundedAndReclaimed() public {
        _sweepWhole(burn, coinX, routerX, 1);
        uint256 t = burn.retire(coinX);
        vm.expectRevert(SweepVault.WrongStage.selector);
        burn.reclaim(t);                                      // not refunded yet
        _resolveTicket(t, bytes32(uint256(3)));
        Clock.skip(7 days + 72 hours);
        vm.prank(custodian);
        wrapper.markUndeliverable(t);                         // the custodian's attestation (review2 R3 §4)
        wrapper.refund(t);
        assertEq(burn.stray(), ONE, "refunded to the vault, not yet charged to anyone");
        burn.reclaim(t);
        assertEq(burn.units(coinX), ONE);
        assertEq(burn.stray(), 0);
        vm.expectRevert(SweepVault.WrongStage.selector);
        burn.reclaim(t);
        _assertLedger();
        burn.retire(coinX);                                   // and it can be retired again
    }

    function test_reclaimRefusesATicketTheVaultDidNotRetire() public {
        _deposit(alice);
        vm.prank(alice);
        uint256 t = wrapper.redeem(TAPROOT);
        _resolveTicket(t, bytes32(uint256(3)));
        Clock.skip(7 days + 72 hours);
        vm.prank(custodian);
        wrapper.markUndeliverable(t);                         // the custodian's attestation (review2 R3 §4)
        wrapper.refund(t);
        vm.expectRevert(SweepVault.WrongStage.selector);
        burn.reclaim(t);
    }

    // ---- the creator gets nothing ----

    function test_theCreatorHasNoPower() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        uint64 at = uint64(vm.getBlockTimestamp());
        uint64 atBlock = uint64(vm.getBlockNumber());
        vm.startPrank(creatorA);
        vm.expectRevert(SweepVault.NotKeeper.selector);
        raffle.postCap(1);
        vm.expectRevert(SweepVault.NotKeeper.selector);
        raffle.announceDraw(coinR);
        vm.stopPrank();
        (at, atBlock);
    }
}
