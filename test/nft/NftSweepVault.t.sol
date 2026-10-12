// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {NftBase} from "./NftBase.sol";
import {NftSweepVault} from "../../src/nft/NftSweepVault.sol";
import {NftVaultBase} from "../../src/nft/NftVaultBase.sol";
import {Clock} from "../utils/Clock.sol";
import {Merkle} from "../utils/Merkle.sol";

/// The 4663 path: money in, the sweep's rules, attribution, the raffle and the burn. Attacks are in NftSecurity.t.sol.
contract NftSweepVaultTest is NftBase {
    function test_depositOnlyFromTheCoinsRouterAndOnlyForItsListing() public {
        _fund(raffleVault, coinA, 1 ether);
        assertEq(raffleVault.attributed(), 1 ether);
        assertEq(raffleVault.contributedBy(coinA), 1 ether);
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        vm.expectRevert(NftVaultBase.NotRouter.selector);
        raffleVault.deposit{value: 1 ether}(coinA);
        // coinC's router is real, but coinC launched on the burn listing: it cannot pay into the raffle vault
        vm.prank(router);
        vm.expectRevert(NftVaultBase.NotRouter.selector);
        raffleVault.deposit{value: 1 ether}(coinC);
    }

    function test_dustWaitsBelowMinEntry() public {
        _fund(raffleVault, coinA, 0.004 ether);
        assertEq(raffleVault.waiting(), 0.004 ether);
        assertEq(raffleVault.attributed(), 0);
        _fund(raffleVault, coinA, 0.006 ether);
        assertEq(raffleVault.waiting(), 0);
        assertEq(raffleVault.attributed(), 0.01 ether);
        assertEq(raffleVault.queueLength(), 1);
    }

    function test_sweepBuysChecksAndAttributes() public {
        _fund(raffleVault, coinA, 0.15 ether);
        _fund(raffleVault, coinB, 1 ether);
        _cap(raffleVault, CAP);
        uint256 first = _sweep(raffleVault, _ids(7, 8, 9), 0.09 ether);
        assertEq(nft.ownerOf(7), address(raffleVault));
        assertEq(nft.ownerOf(9), address(raffleVault));
        assertEq(raffleVault.acquisitionCount(), 3);
        assertEq(address(raffleVault).balance, 1.15 ether - 0.27 ether, "refund came back");
        assertEq(raffleVault.attributed(), 1.15 ether - 0.27 ether);
        // FIFO: coinA paid its 0.15, coinB the other 0.12; the first NFT is A's, the next two B's (largest gap wins)
        assertEq(raffleVault.chargedTo(coinA), 0.15 ether);
        assertEq(raffleVault.chargedTo(coinB), 0.12 ether);
        assertEq(raffleVault.acquisition(first).fundedBy, coinA);
        assertEq(raffleVault.acquisition(first + 1).fundedBy, coinB);
        assertEq(raffleVault.acquisition(first + 2).fundedBy, coinA);
        assertEq(raffleVault.acquisition(first).pricePaid, 0.09 ether);
    }

    function test_partialFillIsFineIfItMeetsMinBought() public {
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
        _ask(_ids(1), 0.05 ether);
        nft.mint(seller, 2);                       // candidate 2 is not for sale; the sweeper's order only buys 1
        vm.prank(sweeper);
        (uint256 bought, uint256 spent) = raffleVault.sweep(address(market), _buyCall(_ids(1), address(raffleVault)), _ids(1, 2), 1);
        assertEq(bought, 1);
        assertEq(spent, 0.05 ether);
    }

    function test_sweepRequiresMinBought() public {
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
        _ask(_ids(1), 0.05 ether);
        nft.mint(seller, 2);
        vm.prank(sweeper);
        vm.expectRevert(abi.encodeWithSelector(NftSweepVault.NotEnoughBought.selector, 1, 2));
        raffleVault.sweep(address(market), _buyCall(_ids(1), address(raffleVault)), _ids(1, 2), 2);
    }

    function test_candidateRules() public {
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
        uint256[] memory none = new uint256[](0);
        vm.expectRevert(NftSweepVault.BadCandidates.selector);
        raffleVault.sweep(address(market), "", none, 1);
        vm.expectRevert(NftSweepVault.BadCandidates.selector);
        raffleVault.sweep(address(market), "", _ids(1), 0);
        vm.expectRevert(NftSweepVault.BadCandidates.selector);
        raffleVault.sweep(address(market), "", _ids(1), 2);
        vm.expectRevert(abi.encodeWithSelector(NftSweepVault.DuplicateCandidate.selector, 1));
        raffleVault.sweep(address(market), "", _ids(1, 2, 1), 1);
        uint256[] memory many = new uint256[](51);
        for (uint256 i = 0; i < 51; ++i) many[i] = i + 100;
        vm.expectRevert(NftSweepVault.BadCandidates.selector);
        raffleVault.sweep(address(market), "", many, 1);
        _sweep(raffleVault, _ids(5), 0.05 ether);
        vm.expectRevert(abi.encodeWithSelector(NftSweepVault.AlreadyHeld.selector, 5));
        raffleVault.sweep(address(market), "", _ids(5), 1);
    }

    function test_valueSentIsCapTimesCandidatesBoundedBySpendable() public {
        _fund(raffleVault, coinA, 0.12 ether);
        _fund(raffleVault, coinB, 0.005 ether);           // waiting: never sent anywhere
        _cap(raffleVault, CAP);
        _ask(_ids(1, 2), 0.06 ether);
        vm.prank(sweeper);
        raffleVault.sweep(address(market), _buyCall(_ids(1, 2), address(raffleVault)), _ids(1, 2), 2);
        assertEq(raffleVault.attributed(), 0);
        assertEq(raffleVault.waiting(), 0.005 ether);
        assertEq(address(raffleVault).balance, 0.005 ether);
    }

    function test_pausedListingCannotSweepButStillSettles() public {
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
        uint256 a = _sweep(raffleVault, _ids(1), 0.05 ether);
        vm.prank(owner);
        nreg.setPaused(0, true);
        vm.expectRevert(NftVaultBase.ListingPaused.selector);
        raffleVault.sweep(address(market), "", _ids(2), 1);
        _fund(raffleVault, coinA, 1 ether);              // money still comes in
        address who = _raffle(raffleVault, a, keccak256("r"));
        assertEq(nft.ownerOf(1), who);
    }

    // ---- raffle ------------------------------------------------------------------------------------------------------------

    function test_raffleDeliversToTheWinnerOn4663() public {
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
        uint256 a = _sweep(raffleVault, _ids(42), 0.05 ether);
        address who = _raffle(raffleVault, a, keccak256("drand"));
        assertEq(nft.ownerOf(42), who);
        assertEq(uint8(raffleVault.lot(a).stage), uint8(NftVaultBase.Stage.Delivered));
        assertEq(raffleVault.lot(a).winner, who);
    }

    function test_drawWaitsOutTheChallengeOnBothClocks() public {
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
        uint256 a = _sweep(raffleVault, _ids(1), 0.05 ether);
        bytes32 root = Merkle.root(_tickets());      // before the prank: _tickets() makes external calls, and would spend it
        vm.prank(keeper);
        raffleVault.openDraw(a, root, 100, "ipfs://t");
        Clock.skipTimeOnly(1 hours);                  // a sequencer stamping ahead does not close the window
        vm.expectRevert();
        raffleVault.requestDraw(a);
        vm.prank(owner);
        raffleVault.cancelDraw(a);                    // and the owner can still cancel
        assertEq(uint8(raffleVault.lot(a).stage), uint8(NftVaultBase.Stage.Held));
    }

    function test_onlyKeeperOpensAndOnlyOwnerCancels() public {
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
        uint256 a = _sweep(raffleVault, _ids(1), 0.05 ether);
        bytes32 root = Merkle.root(_tickets());
        vm.prank(stranger);
        vm.expectRevert(NftVaultBase.NotKeeper.selector);
        raffleVault.openDraw(a, root, 100, "ipfs://t");
        vm.prank(keeper);
        raffleVault.openDraw(a, root, 100, "ipfs://t");
        vm.prank(keeper);
        vm.expectRevert(NftVaultBase.NotOwner.selector);
        raffleVault.cancelDraw(a);
        Clock.skip(15 minutes);
        vm.prank(owner);
        vm.expectRevert(NftVaultBase.WindowOver.selector);
        raffleVault.cancelDraw(a);
    }

    function test_wrongProofCannotClaim() public {
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
        uint256 a = _sweep(raffleVault, _ids(1), 0.05 ether);
        bytes32[] memory leaves = _tickets();
        vm.prank(keeper);
        raffleVault.openDraw(a, Merkle.root(leaves), 100, "ipfs://t");
        Clock.skip(15 minutes);
        raffleVault.requestDraw(a);
        raffleVault.resolve(a, abi.encode(keccak256("x")));
        uint256 t = raffleVault.lot(a).winningTicket;
        (address who,,, uint256 idx) = _holderOf(t);
        address other = who == alice ? bob : alice;
        uint256 otherIdx = who == alice ? 1 : 0;
        vm.expectRevert(NftVaultBase.BadProof.selector);
        raffleVault.claimWin(a, other, 0, 100, Merkle.proof(leaves, otherIdx));
        (,, uint256 end,) = _holderOf(t);
        (, uint256 first,,) = _holderOf(t);
        vm.expectRevert(NftVaultBase.BadProof.selector);
        raffleVault.claimWin(a, stranger, first, end, Merkle.proof(leaves, idx));
    }

    function test_voidAfterSevenDaysReturnsTheLotForANewSnapshot() public {
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
        uint256 a = _sweep(raffleVault, _ids(1), 0.05 ether);
        vm.prank(keeper);
        raffleVault.openDraw(a, keccak256("a list with a gap"), 100, "ipfs://t");
        Clock.skip(15 minutes);
        raffleVault.requestDraw(a);
        raffleVault.resolve(a, abi.encode(keccak256("x")));
        vm.prank(owner);
        vm.expectRevert(NftVaultBase.TooEarly.selector);
        raffleVault.voidDraw(a);
        Clock.skip(7 days);
        vm.prank(owner);
        raffleVault.voidDraw(a);
        assertEq(uint8(raffleVault.lot(a).stage), uint8(NftVaultBase.Stage.Held));
        assertEq(nft.ownerOf(1), address(raffleVault));
    }

    function test_redrawOnlyIfTheRoundNeverCame() public {
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
        uint256 a = _sweep(raffleVault, _ids(1), 0.05 ether);
        bytes32 root = Merkle.root(_tickets());      // before the prank: _tickets() makes external calls, and would spend it
        vm.prank(keeper);
        raffleVault.openDraw(a, root, 100, "ipfs://t");
        Clock.skip(15 minutes);
        raffleVault.requestDraw(a);
        uint64 r = raffleVault.lot(a).round;
        vm.expectRevert(NftVaultBase.TooEarly.selector);
        raffleVault.redraw(a);
        Clock.skip(24 hours);
        raffleVault.redraw(a);
        assertGt(raffleVault.lot(a).round, r);
    }

    // ---- burn --------------------------------------------------------------------------------------------------------------

    function test_burnSendsToDeadAndOnlyInABurnVault() public {
        _fund(burnVault, coinC, 1 ether);
        _cap(burnVault, CAP);
        uint256 a = _sweep(burnVault, _ids(3), 0.05 ether);
        vm.prank(stranger);
        burnVault.burn(a);
        assertEq(nft.ownerOf(3), burnVault.DEAD());
        vm.expectRevert(NftVaultBase.WrongStage.selector);
        burnVault.burn(a);
        // a raffle vault cannot burn, a burn vault cannot draw
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
        uint256 b = _sweep(raffleVault, _ids(4), 0.05 ether);
        vm.expectRevert(NftVaultBase.WrongPolicy.selector);
        raffleVault.burn(b);
        _sweep(burnVault, _ids(5), 0.05 ether);
        vm.prank(keeper);
        vm.expectRevert(NftVaultBase.WrongPolicy.selector);
        burnVault.openDraw(a + 1, bytes32(uint256(1)), 1, "x");
    }
}
