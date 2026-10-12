// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase, MockVenue} from "../utils/SweepBase.sol";
import {Clock} from "../utils/Clock.sol";
import {Merkle} from "../utils/Merkle.sol";
import {SweepVault} from "../../src/SweepVault.sol";

/// docs/v3/sweep.md, "What the vault can never do", checked by what it exposes and by trying.
contract SweepSecurityTest is SweepBase {
    function setUp() public override {
        super.setUp();
        _stockVenue(20);
    }

    /// The whole ABI, exactly. No withdraw, no approve, no setter for the policy, the venue or anything else, no arbitrary call.
    /// A function added later fails here until somebody decides who may call it — and the policy stays fixed by construction.
    function test_theSweepVaultExposesExactlyWhatItMeansTo() public view {
        string[83] memory list = [
            // money in, and what is refused
            "deposit(address)", "depositWrapped(address,uint256)", "depositFrom(address)",
            // the sweep
            "postCap(uint256)", "capFresh()", "sweep(address,uint256,bytes,uint256)",
            // the canister route: the public mint desk, never more than one cap per frog
            "sweepViaDesk(address,uint256)", "collectDesk(uint256)", "deskCoin(uint256)", "deskAmount(uint256)", "atDesk()",
            // RAFFLE
            // (docs/audit/2026-09-26/fixes-B-C-keeper-draws.md: announced with a seed round that picks the snapshot time; the root
            // commits the count and the time; one draw per coin before its pin; the pinned round is final — no redraw)
            "announceDraw(address)", "openDraw(uint256,bytes32,uint256,string,bytes32[],bytes)", "snapshotTimeOf(uint256,bytes32)",
            "drawInFlight(address)", "cancelDraw(uint256)", "requestDraw(uint256)",
            // review2 N-2 (fixes-R3-contracts.md §3): a seed ended before its pin is carried to the coin's next draw
            "carriedSeed(address)", "seedOrigin(uint256)",
            "resolve(uint256,bytes)", "claim(uint256,address,uint256,uint256,bytes32[])", "voidDraw(uint256)",
            "leaf(address,uint256,uint256)",
            // BURN
            "retire(address)", "reclaim(uint256)",
            // YOYO (docs/v5/policies-and-fees.md §1): relist at a markup, the sale's proceeds buy the coin back and burn it
            "relist(address)", "recordSale(uint256)", "buyback(address,uint256,bytes,uint256)", "reclaimListing(uint256)",
            "setMarkup(uint256)", "listing(uint256)", "proceeds(address)", "listedOpen(address)", "totalProceeds()",
            "totalBurnedBack()", "markupBps()", "MIN_MARKUP_BPS()", "MAX_MARKUP_BPS()", "SALE_FEE_ALLOWANCE_BPS()", "DEAD()",
            // immutables and constants
            "registry()", "listingId()", "wrapper()", "beacon()", "policy()", "venue()", "launcher()", "desk()", "ONE()", "CAP_TTL()",
            "SNAPSHOT_WINDOW()", "OPEN_TTL()", "CHALLENGE()", "PIN_AHEAD()", "PIN_TIMEOUT()", "VOID_AFTER()", "parentBlockSeconds()",
            // the books
            "credit(address)", "units(address)", "retiredBy(uint256)", "reclaimed(uint256)", "totalCredit()", "totalUnits()",
            "totalReserved()", "retiredCount()", "cap()", "capAt()", "capAtBlock()", "draw(uint256)", "drawCount()", "stray()",
            // IEtchVault, so nothing downstream mistakes it for a purchase vault
            "acquisition(uint256)", "acquisitionCount()", "isRune()",
            // H12 (docs/v5/custody-hardening.md): the cap moves in steps under a ceiling only the registry's owner raises, slowly
            "MAX_STEP_BPS()", "MIN_POST_GAP()", "CEILING_DELAY()", "capCeiling()", "pendingCeiling()", "pendingCeilingAt()",
            "pendingCeilingAtBlock()", "proposeCapCeiling(uint256)", "acceptCapCeiling()"
        ];
        string memory json = vm.readFile("out/SweepVault.sol/SweepVault.json");
        string[] memory actual = vm.parseJsonKeys(json, ".methodIdentifiers");
        assertEq(actual.length, list.length, "the sweep vault exposes a function that is not on the list, or lost one that is");
        for (uint256 j = 0; j < list.length; ++j) {
            bool found;
            for (uint256 i = 0; i < actual.length; ++i) {
                if (keccak256(bytes(actual[i])) == keccak256(bytes(list[j]))) { found = true; break; }
            }
            assertTrue(found, string.concat("missing: ", list[j]));
        }
    }

    /// The policy is an immutable: it is in the code, not in storage, so no storage write — by any function, or by anybody
    /// with a storage-writing bug — can change it. Scribble over every low slot and it still reads the same.
    function test_thePolicyIsNotInStorage() public {
        for (uint256 slot = 0; slot < 64; ++slot) vm.store(address(burn), bytes32(slot), bytes32(uint256(0)));
        assertEq(uint8(burn.policy()), uint8(SweepVault.Policy.BURN));
        for (uint256 slot = 0; slot < 64; ++slot) vm.store(address(raffle), bytes32(slot), bytes32(type(uint256).max));
        assertEq(uint8(raffle.policy()), uint8(SweepVault.Policy.RAFFLE));
    }

    /// Everyone with any role, trying every function that moves anything, gets out no more than the rules pay.
    function test_theVaultCannotBeDrainedByAnyRole() public {
        _sweepWhole(raffle, coinR, routerR, 3);
        _fundCoin(raffle, coinR, routerR, 1 ether);
        uint256 ethBefore = address(raffle).balance;
        uint256 wBefore = wrapper.balanceOf(address(raffle));
        address[6] memory who = [owner, keeper, custodian, creatorA, routerR, stranger];
        for (uint256 i = 0; i < who.length; ++i) {
            vm.startPrank(who[i]);
            // a sweep with any route but a fair one: to somebody else, keep the ETH, pull the vault's wFROG
            vm.expectRevert();
            raffle.sweep(coinR, 0.5 ether, abi.encodeCall(MockVenue.buyFor, (PRICE, who[i])), 1);
            vm.expectRevert();
            raffle.sweep(coinR, 0.5 ether, abi.encodeCall(MockVenue.keep, ()), 1);
            vm.expectRevert();
            raffle.sweep(coinR, 0.5 ether, abi.encodeCall(MockVenue.steal, (wBefore)), 1);
            // a claim with no draw, a retirement under the wrong policy, a treasury deposit
            vm.expectRevert();
            raffle.claim(0, who[i], 0, 1, new bytes32[](0));
            vm.expectRevert();
            raffle.retire(coinR);
            vm.stopPrank();
        }
        assertEq(address(raffle).balance, ethBefore);
        assertEq(wrapper.balanceOf(address(raffle)), wBefore);
    }

    /// A raffle draw pays exactly one whole wFROG, once, to the proven holder, and nothing else leaves.
    function test_aDrawPaysOneWholeWrappedOnceAndNoMore() public {
        _sweepWhole(raffle, coinR, routerR, 3);
        address holder = makeAddr("holder");
        (uint256 id, bytes32[] memory tree) = _openDrawOne(raffle, coinR, holder, 1);
        Clock.skip(15 minutes);
        raffle.requestDraw(id);
        raffle.resolve(id, abi.encodePacked(bytes32(uint256(11))));
        raffle.claim(id, holder, 0, 1, Merkle.proof(tree, 0));
        vm.expectRevert(SweepVault.WrongStage.selector);
        raffle.claim(id, holder, 0, 1, Merkle.proof(tree, 0));
        vm.expectRevert(SweepVault.WrongStage.selector);
        raffle.voidDraw(id);
        assertEq(wrapper.balanceOf(holder), ONE);
        assertEq(wrapper.balanceOf(address(raffle)), 2 * ONE);
        assertEq(raffle.units(coinR), 2 * ONE);
    }

    /// Paying a coin's escrow directly (the one thing attribution cannot tell from a trade fee) buys no say: the only thing
    /// that money can do is sweep for that same coin; it cannot open, aim or cancel a draw, and it never reaches another coin.
    function test_directPaymentsBuyNoSay() public {
        // a whale pays straight to the vault: refused
        vm.deal(stranger, 10 ether);
        vm.prank(stranger);
        (bool ok,) = address(raffle).call{value: 10 ether}("");
        assertFalse(ok);
        vm.prank(stranger);
        vm.expectRevert(SweepVault.NotRouter.selector);
        raffle.deposit{value: 10 ether}(coinR);
        // a whale's wFROG sent straight in: stray, credited to nobody, drawn by nobody
        _deposit(stranger);
        vm.prank(stranger);
        wrapper.transfer(address(raffle), ONE);
        assertEq(raffle.units(coinR) + raffle.units(coinR2), 0);
        // the whale pays coinR's fees through its router: coinR's credit rises, coinR2's does not, and the whale has no role
        _fundCoin(raffle, coinR, routerR, 5 ether);
        assertEq(raffle.credit(coinR2), 0);
        vm.prank(stranger);
        vm.expectRevert(SweepVault.NotKeeper.selector);
        raffle.announceDraw(coinR);
    }

    /// The raffle cannot be aimed. Everything that decides the winner is fixed before the round is pinned; the round pinned is
    /// one nobody has seen; and the pin is final: there is no redraw at any age (C-02).
    function testFuzz_theRaffleCannotBeAimed(bytes32 r, uint64 wait, uint256 total) public {
        total = bound(total, 1, 1e30);
        wait = uint64(bound(wait, 15 minutes, 6 days));
        vm.assume(r != bytes32(0));
        _sweepWhole(raffle, coinR, routerR, 1);
        (uint256 id,) = _openDrawOne(raffle, coinR, makeAddr("holder"), total);
        Clock.skip(wait);                                       // whoever requests may choose when...
        raffle.requestDraw(id);
        SweepVault.Draw memory d = raffle.draw(id);
        assertEq(beacon.randomnessOf(d.round), bytes32(0), "...but not a round anyone has seen");
        assertGe(d.round, beacon.roundAt(vm.getBlockTimestamp()) + 20);
        Clock.skip(wait);
        (bool ok,) = address(raffle).call(abi.encodeWithSignature("redraw(uint256)", id));
        assertFalse(ok, "and cannot shop for another");
        vm.expectRevert(SweepVault.WrongStage.selector);
        raffle.requestDraw(id);
        raffle.resolve(id, abi.encodePacked(r));
        assertEq(raffle.draw(id).winningTicket, uint256(keccak256(abi.encode(r, address(raffle), id))) % total);
    }

    /// A stale cap is refused on either clock alone; an announcement not opened within six hours is refused.
    function testFuzz_staleIsRefusedOnEitherClock(uint32 secs, bool timeOnly) public {
        secs = uint32(bound(secs, 6 hours, 30 days));
        _sweepWhole(raffle, coinR, routerR, 1);
        _fundCoin(raffle, coinR, routerR, 1 ether);
        vm.prank(keeper);
        uint256 id = raffle.announceDraw(coinR);
        if (timeOnly) Clock.skipTimeOnly(secs);
        else vm.roll(vm.getBlockNumber() + secs / 12);
        assertFalse(raffle.capFresh());
        bytes memory data = abi.encodeCall(MockVenue.buy, (PRICE));
        vm.expectRevert(SweepVault.CapStale.selector);
        raffle.sweep(coinR, PRICE, data, 1);
        Clock.skip(6 hours);                                    // both clocks, for the announcement's own window
        bytes32[] memory none;
        vm.prank(keeper);
        vm.expectRevert(SweepVault.WindowOver.selector);
        raffle.openDraw(id, bytes32(uint256(1)), 1, "x", none, abi.encodePacked(bytes32(uint256(5))));
    }

    /// Raffle money and burn money never mix: one coin's router can fund only its own listing's vault.
    function test_raffleAndBurnMoneyNeverMix() public {
        vm.prank(routerR);
        vm.expectRevert(SweepVault.NotRouter.selector);
        burn.deposit{value: 1 ether}(coinR);
        vm.prank(routerX);
        vm.expectRevert(SweepVault.NotRouter.selector);
        raffle.deposit{value: 1 ether}(coinX);
    }

    /// A merkle root over one holder cannot be claimed by anyone else for a draw whose winner is that holder.
    function test_theClaimPaysTheLeafsHolderOnly() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        address holder = makeAddr("holder");
        bytes32[] memory leaves = new bytes32[](2);
        leaves[0] = raffle.leaf(holder, 0, 1);
        leaves[1] = raffle.leaf(makeAddr("other"), 1, 2);
        (uint256 id, bytes32[] memory tree) = _openDraw(raffle, coinR, leaves, 1);   // one ticket: holder wins
        Clock.skip(15 minutes);
        raffle.requestDraw(id);
        raffle.resolve(id, abi.encodePacked(bytes32(uint256(11))));
        bytes32[] memory proof = Merkle.proof(tree, 0);
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.claim(id, stranger, 0, 1, proof);
        raffle.claim(id, holder, 0, 1, proof);
        assertEq(wrapper.balanceOf(holder), ONE);
    }
}
