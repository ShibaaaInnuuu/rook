// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {NftBase} from "./NftBase.sol";
import {NftRegistry} from "../../src/nft/NftRegistry.sol";
import {NftSweepVault} from "../../src/nft/NftSweepVault.sol";
import {NftRemoteVault} from "../../src/nft/NftRemoteVault.sol";
import {Policy} from "../../src/nft/INft.sol";
import {Clock} from "../utils/Clock.sol";

contract NftRegistryTest is NftBase {
    function test_listingsNameAnyEvmChain() public view {
        NftRegistry.NftListing memory l0 = nreg.listing(0);
        assertEq(l0.chainId, block.chainid);
        assertEq(l0.collection, address(nft));
        assertEq(uint8(l0.policy), uint8(Policy.Raffle));
        assertEq(l0.buyer, address(0));
        assertTrue(nreg.isNative(0));
        NftRegistry.NftListing memory l2 = nreg.listing(2);
        assertEq(l2.chainId, 1);
        assertEq(l2.buyer, canister);
        assertFalse(nreg.isNative(2));
        assertEq(nreg.count(), 4);
        assertEq(nreg.ref(2), string.concat("eip155:1:", vm.toLowercase(vm.toString(remoteCollection))));
        assertEq(nreg.ref(3), string.concat("eip155:8453:", vm.toLowercase(vm.toString(remoteCollection))));
    }

    function test_vaultsCarryTheirListingForever() public view {
        assertEq(raffleVault.listingId(), 0);
        assertEq(uint8(raffleVault.policy()), uint8(Policy.Raffle));
        assertEq(uint8(burnVault.policy()), uint8(Policy.Burn));
        assertEq(raffleVault.collection(), address(nft));
        assertEq(raffleVault.chainId(), block.chainid);
        assertEq(remoteRaffle.buyer(), canister);
        assertEq(remoteRaffle.maxInFlight(), 5 ether);
        assertEq(address(raffleVault.registry()), address(nreg));
        assertEq(address(raffleVault.beacon()), address(beacon));
    }

    function test_rolesAreListingRegistrys() public view {
        assertEq(nreg.owner(), owner);
        assertEq(nreg.keeper(), keeper);
        assertEq(nreg.treasury(), treasury);
    }

    function test_onePolicyPerCollectionAndChain() public {
        vm.prank(owner);
        vm.expectRevert(NftRegistry.AlreadyListed.selector);
        nreg.listNft(uint64(block.chainid), address(nft), Policy.Raffle, "again", payable(0), 0);
        vm.prank(owner);
        vm.expectRevert(NftRegistry.AlreadyListed.selector);
        nreg.listNft(1, remoteCollection, Policy.Raffle, "again", canister, 1 ether);
        // the same address on a THIRD chain is a different collection
        vm.prank(owner);
        nreg.listNft(10, remoteCollection, Policy.Raffle, "Optimism floor", canister, 1 ether);
    }

    function test_listingRefusals() public {
        vm.startPrank(owner);
        vm.expectRevert(abi.encodeWithSelector(NftRegistry.NoCode.selector, stranger));
        nreg.listNft(uint64(block.chainid), stranger, Policy.Burn, "no code", payable(0), 0);
        vm.expectRevert(abi.encodeWithSelector(NftRegistry.BadBuyer.selector, canister));
        nreg.listNft(uint64(block.chainid), address(market), Policy.Burn, "native with a buyer", canister, 0);
        vm.expectRevert(abi.encodeWithSelector(NftRegistry.BadBuyer.selector, address(0)));
        nreg.listNft(1, address(0xBEEF), Policy.Burn, "no buyer", payable(0), 1 ether);
        vm.expectRevert(abi.encodeWithSelector(NftRegistry.BadBuyer.selector, keeper));
        nreg.listNft(1, address(0xBEEF), Policy.Burn, "keeper as buyer", payable(keeper), 1 ether);
        vm.expectRevert(abi.encodeWithSelector(NftRegistry.BadBuyer.selector, owner));
        nreg.listNft(1, address(0xBEEF), Policy.Burn, "owner as buyer", payable(owner), 1 ether);
        vm.expectRevert(abi.encodeWithSelector(NftRegistry.BadBuyer.selector, address(market)));
        nreg.listNft(1, address(0xBEEF), Policy.Burn, "contract as buyer", payable(address(market)), 1 ether);
        vm.expectRevert(NftRegistry.Empty.selector);
        nreg.listNft(1, address(0xBEEF), Policy.Burn, "no bound", canister, 0);
        vm.expectRevert(NftRegistry.Empty.selector);
        nreg.listNft(1, address(0xBEEF), Policy.Burn, "", canister, 1 ether);
        vm.expectRevert(NftRegistry.ZeroAddress.selector);
        nreg.listNft(0, address(0xBEEF), Policy.Burn, "chain 0", canister, 1 ether);
        vm.stopPrank();
        vm.prank(stranger);
        vm.expectRevert(NftRegistry.NotOwner.selector);
        nreg.listNft(1, address(0xBEEF), Policy.Burn, "stranger", canister, 1 ether);
    }

    function test_wireOnce() public {
        vm.prank(owner);
        vm.expectRevert(NftRegistry.AlreadyWired.selector);
        nreg.wire(stranger);
    }

    function test_aNewSweepTargetWaitsSevenPublicDaysOnBothClocks() public {
        address seaport17 = makeAddr("a future Seaport");
        vm.prank(stranger);
        vm.expectRevert(NftRegistry.NotOwner.selector);
        nreg.proposeSweepTarget(seaport17);
        vm.prank(owner);
        nreg.proposeSweepTarget(seaport17);
        Clock.skipTimeOnly(8 days);
        vm.prank(owner);
        vm.expectRevert();
        nreg.acceptSweepTarget(seaport17);
        Clock.skip(7 days);
        vm.prank(owner);
        nreg.acceptSweepTarget(seaport17);
        assertTrue(nreg.isSweepTarget(seaport17));
        vm.prank(owner);
        nreg.removeSweepTarget(seaport17);                // instant
        assertFalse(nreg.isSweepTarget(seaport17));
    }

    function test_roleRotationThroughListingRegistryMovesTheNftSide() public {
        address newKeeper = makeAddr("new keeper");
        vm.prank(owner);
        lr.proposeRoles(newKeeper, purchaser, attestor);
        assertEq(nreg.keeper(), keeper, "not before the seven days");
        Clock.skip(7 days + 1);
        vm.prank(owner);
        lr.acceptRoles();
        assertEq(nreg.keeper(), newKeeper);
    }

    function test_nativeAndRemoteGetTheRightVault() public view {
        assertEq(NftSweepVault(payable(nreg.vaultOf(1))).MAX_CANDIDATES(), 50);
        assertEq(NftRemoteVault(nreg.vaultOf(3)).MAX_QUANTITY(), 50);
    }
}
