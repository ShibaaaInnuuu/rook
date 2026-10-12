// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {NftBase} from "./NftBase.sol";
import {NftRemoteVault} from "../../src/nft/NftRemoteVault.sol";
import {NftVaultBase} from "../../src/nft/NftVaultBase.sol";
import {Clock} from "../utils/Clock.sol";

/// The other-chain path: the canister announces, pulls after a public hour, receipts each NFT with its purchase tx, and records the
/// fate — delivery to the winner's same address on the collection's chain, or a burn — by tx hash.
contract NftRemoteVaultTest is NftBase {
    bytes32 constant BUY_TX = keccak256("purchase tx on chain 1");
    bytes32 constant SEND_TX = keccak256("delivery tx on chain 1");

    function _pulled(NftRemoteVault v, address coin, uint256 maxEth, uint256 qty) internal returns (uint256 id) {
        _fund(v, coin, maxEth);
        _cap(v, CAP);
        vm.prank(canister);
        id = v.announce(maxEth, qty, "https://example.invalid/floor");
        Clock.skip(1 hours);
        vm.prank(canister);
        v.pull(id);
    }

    function test_announcePullReceiptSettle() public {
        uint256 id = _pulled(remoteRaffle, coinD, 0.2 ether, 2);
        assertEq(canister.balance, 0.2 ether);
        assertEq(remoteRaffle.inFlight(), 0.2 ether);
        vm.startPrank(canister);
        uint256 a = remoteRaffle.receipt(id, 1001, BUY_TX);
        remoteRaffle.receipt(id, 1002, BUY_TX);
        vm.stopPrank();
        assertEq(uint8(remoteRaffle.purchase(id).status), uint8(NftRemoteVault.Status.Settled));
        assertEq(remoteRaffle.inFlight(), 0);
        assertEq(remoteRaffle.acquisition(a).fundedBy, coinD);
        assertEq(remoteRaffle.acquisition(a).purchaseRef, BUY_TX);
        assertEq(remoteRaffle.chainId(), 1);
    }

    function test_pullWaitsAPublicHourOnBothClocks() public {
        _fund(remoteRaffle, coinD, 1 ether);
        _cap(remoteRaffle, CAP);
        vm.prank(canister);
        uint256 id = remoteRaffle.announce(0.1 ether, 1, "r");
        Clock.skipTimeOnly(1 hours);
        vm.prank(canister);
        vm.expectRevert(NftVaultBase.TooEarly.selector);
        remoteRaffle.pull(id);
    }

    function test_onlyTheCanisterAnnouncesPullsAndRecords() public {
        _fund(remoteRaffle, coinD, 1 ether);
        _cap(remoteRaffle, CAP);
        vm.prank(keeper);
        vm.expectRevert(NftRemoteVault.NotBuyer.selector);
        remoteRaffle.announce(0.1 ether, 1, "r");
        vm.prank(owner);
        vm.expectRevert(NftRemoteVault.NotBuyer.selector);
        remoteRaffle.announce(0.1 ether, 1, "r");
    }

    function test_announceBounds() public {
        _fund(remoteRaffle, coinD, 10 ether);
        vm.prank(canister);
        vm.expectRevert(NftVaultBase.CapExpired.selector);
        remoteRaffle.announce(0.1 ether, 1, "r");
        _cap(remoteRaffle, CAP);
        vm.prank(canister);
        vm.expectRevert(abi.encodeWithSelector(NftRemoteVault.OverCap.selector, 0.21 ether, 0.2 ether));
        remoteRaffle.announce(0.21 ether, 2, "r");
        _cap(remoteRaffle, 1 ether);
        vm.prank(canister);
        vm.expectRevert(abi.encodeWithSelector(NftRemoteVault.OverInFlight.selector, 5.1 ether, 5 ether));
        remoteRaffle.announce(5.1 ether, 6, "r");
        _cap(remoteRaffle, CAP);
        vm.startPrank(canister);
        remoteRaffle.announce(0.1 ether, 1, "r");
        remoteRaffle.announce(0.1 ether, 1, "r");
        remoteRaffle.announce(0.1 ether, 1, "r");
        vm.expectRevert(NftRemoteVault.TooManyOpen.selector);
        remoteRaffle.announce(0.1 ether, 1, "r");
        vm.stopPrank();
    }

    function test_aTokenIsReceiptedOnce() public {
        uint256 id = _pulled(remoteRaffle, coinD, 0.2 ether, 2);
        vm.startPrank(canister);
        remoteRaffle.receipt(id, 7, BUY_TX);
        vm.expectRevert(abi.encodeWithSelector(NftRemoteVault.AlreadyReceipted.selector, 7));
        remoteRaffle.receipt(id, 7, BUY_TX);
        vm.expectRevert(NftVaultBase.BadInput.selector);
        remoteRaffle.receipt(id, 8, bytes32(0));
        vm.stopPrank();
    }

    function test_settleShortOnlyAfterTheMissingEthIsBack() public {
        uint256 id = _pulled(remoteRaffle, coinD, 0.3 ether, 3);
        vm.prank(canister);
        remoteRaffle.receipt(id, 1, BUY_TX);
        vm.prank(canister);
        vm.expectRevert(abi.encodeWithSelector(NftRemoteVault.ReturnFirst.selector, 0.2 ether));
        remoteRaffle.settleShort(id);
        vm.prank(canister);
        remoteRaffle.returnFunds{value: 0.2 ether}(id);
        assertEq(remoteRaffle.unspentOf(coinD), 0.2 ether);
        vm.prank(canister);
        remoteRaffle.settleShort(id);
        assertEq(remoteRaffle.inFlight(), 0);
        assertEq(remoteRaffle.chargedTo(coinD), 0.1 ether);
    }

    function test_overdueBlocksAndIsWrittenOffByOwnerThenAnyone() public {
        uint256 id = _pulled(remoteRaffle, coinD, 0.1 ether, 1);
        Clock.skip(3 days + 1);
        _fund(remoteRaffle, coinD, 1 ether);
        _cap(remoteRaffle, CAP);
        vm.prank(canister);
        vm.expectRevert(abi.encodeWithSelector(NftRemoteVault.BlockedByOverdue.selector, id));
        remoteRaffle.announce(0.1 ether, 1, "r");
        vm.prank(stranger);
        vm.expectRevert(NftVaultBase.NotOwner.selector);
        remoteRaffle.writeOff(id);
        Clock.skip(7 days);
        vm.prank(stranger);
        remoteRaffle.writeOff(id);
        assertEq(remoteRaffle.totalWrittenOff(), 0.1 ether);
        assertEq(remoteRaffle.inFlight(), 0);
    }

    function test_unpulledAnnouncementIsCancelledByOwnerOrLaterAnyone() public {
        _fund(remoteRaffle, coinD, 1 ether);
        _cap(remoteRaffle, CAP);
        vm.prank(canister);
        uint256 id = remoteRaffle.announce(0.1 ether, 1, "r");
        vm.prank(stranger);
        vm.expectRevert(NftVaultBase.NotOwner.selector);
        remoteRaffle.cancelAnnouncement(id);
        Clock.skip(8 days + 1 hours);
        vm.prank(stranger);
        remoteRaffle.cancelAnnouncement(id);
        assertEq(remoteRaffle.reserved(), 0);
        assertEq(remoteRaffle.attributed(), 1 ether);
    }

    function test_fullReturnClosesThePurchase() public {
        uint256 id = _pulled(remoteRaffle, coinD, 0.1 ether, 1);
        vm.prank(canister);
        remoteRaffle.returnFunds{value: 0.1 ether}(id);
        assertEq(uint8(remoteRaffle.purchase(id).status), uint8(NftRemoteVault.Status.Returned));
        assertEq(remoteRaffle.attributed(), 0.1 ether);
        assertEq(remoteRaffle.chargedTo(coinD), 0);
    }

    // ---- the fate, delivered elsewhere -------------------------------------------------------------------------------------

    function test_raffleWinnerIsDeliveredOnTheOtherChainAndRecordedByTxHash() public {
        uint256 id = _pulled(remoteRaffle, coinD, 0.1 ether, 1);
        vm.prank(canister);
        uint256 a = remoteRaffle.receipt(id, 77, BUY_TX);
        address who = _raffle(remoteRaffle, a, keccak256("drand"));
        assertEq(remoteRaffle.lot(a).destination, who, "the winner's same EVM address by default");
        vm.prank(canister);
        vm.expectRevert(NftVaultBase.TooEarly.selector);          // the winner's hour to redirect is still open
        remoteRaffle.recordDelivery(a, SEND_TX);
        address safeTwin = makeAddr("winner's account on chain 1");
        vm.prank(stranger);
        vm.expectRevert(NftRemoteVault.NotWinner.selector);
        remoteRaffle.setDestination(a, stranger);
        vm.prank(who);
        remoteRaffle.setDestination(a, safeTwin);
        Clock.skip(1 hours);
        vm.prank(who);
        vm.expectRevert(NftRemoteVault.DestinationLocked.selector);
        remoteRaffle.setDestination(a, who);
        vm.prank(canister);
        remoteRaffle.recordDelivery(a, SEND_TX);
        NftVaultBase.Lot memory l = remoteRaffle.lot(a);
        assertEq(uint8(l.stage), uint8(NftVaultBase.Stage.Delivered));
        assertEq(l.settleTx, SEND_TX);
        assertEq(l.destination, safeTwin);
    }

    function test_undeliveredWinIsFlaggedOverdueAfterSevenDays() public {
        uint256 id = _pulled(remoteRaffle, coinD, 0.1 ether, 1);
        vm.prank(canister);
        uint256 a = remoteRaffle.receipt(id, 77, BUY_TX);
        _raffle(remoteRaffle, a, keccak256("drand"));
        vm.expectRevert(NftRemoteVault.NotOverdue.selector);
        remoteRaffle.flagOverdue(a);
        Clock.skip(7 days + 1);
        remoteRaffle.flagOverdue(a);
        assertTrue(remoteRaffle.lot(a).overdue);
    }

    function test_remoteBurnIsRecordedByTxHash() public {
        uint256 id = _pulled(remoteBurn, coinE, 0.1 ether, 1);
        vm.prank(canister);
        uint256 a = remoteBurn.receipt(id, 5, BUY_TX);
        vm.prank(stranger);
        vm.expectRevert(NftRemoteVault.NotBuyer.selector);
        remoteBurn.recordBurn(a, SEND_TX);
        vm.prank(canister);
        remoteBurn.recordBurn(a, SEND_TX);
        assertEq(uint8(remoteBurn.lot(a).stage), uint8(NftVaultBase.Stage.Burned));
        assertEq(remoteBurn.lot(a).settleTx, SEND_TX);
        // and a raffle listing's NFT can never be recorded as burned
        uint256 id2 = _pulled(remoteRaffle, coinD, 0.1 ether, 1);
        vm.prank(canister);
        uint256 b = remoteRaffle.receipt(id2, 6, BUY_TX);
        vm.prank(canister);
        vm.expectRevert(NftVaultBase.WrongPolicy.selector);
        remoteRaffle.recordBurn(b, SEND_TX);
    }

    function test_unburnedIsFlaggedOverdue() public {
        uint256 id = _pulled(remoteBurn, coinE, 0.1 ether, 1);
        vm.prank(canister);
        uint256 a = remoteBurn.receipt(id, 5, BUY_TX);
        Clock.skip(7 days + 1);
        remoteBurn.flagOverdue(a);
        assertTrue(remoteBurn.lot(a).overdue);
    }
}
