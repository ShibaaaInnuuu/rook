// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";
import {ListingRegistry} from "../../src/ListingRegistry.sol";
import {NftRegistry} from "../../src/nft/NftRegistry.sol";
import {Policy} from "../../src/nft/INft.sol";
import {NftBase} from "./NftBase.sol";
import {Clock} from "../utils/Clock.sol";

/// docs/v4/nft.md §2.1, §10.6: the canister's NFT buyer is reserved in ListingRegistry the moment it is named, so no role, now or
/// after any later rotation, can ever be that address, and no address that is, or is about to be, a role can be named a buyer.
contract NftBuyerReservationTest is NftBase {
    address payable other = payable(makeAddr("another canister key"));

    function test_aRemoteListingReservesItsBuyer() public view {
        assertTrue(lr.isAutoBuyer(canister), "setUp listed two remote listings bought by the canister");
        assertEq(lr.nftRegistry(), address(nreg));
    }

    /// Failed before the hook: the rotation was accepted, and the canister held the keeper role next to its pull rights.
    function test_noRoleCanRotateOntoAnNftBuyer() public {
        vm.startPrank(owner);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.proposeRoles(canister, purchaser, attestor);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.proposeRoles(keeper, canister, attestor);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.proposeRoles(keeper, purchaser, canister);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.proposeTreasury(canister);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.transferOwnership(canister);
        vm.stopPrank();
        assertEq(lr.keeper(), keeper);
        assertEq(lr.pendingOwner(), address(0));
    }

    /// The reverse order: a rotation proposed FIRST, then the same address named as a buyer during the seven days. Refused at the
    /// naming, so the rotation can never land on a buyer.
    function test_anAddressPendingForARoleCannotBeNamedABuyer() public {
        vm.startPrank(owner);
        lr.proposeRoles(other, purchaser, attestor);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        nreg.listNft(10, address(0xBEEF), Policy.Burn, "pending keeper as buyer", other, 1 ether);
        // the same for a pending treasury and a pending owner
        address payable t2 = payable(makeAddr("next treasury"));
        lr.proposeTreasury(t2);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        nreg.listNft(10, address(0xBEEF), Policy.Burn, "pending treasury as buyer", t2, 1 ether);
        address payable o2 = payable(makeAddr("next owner"));
        lr.transferOwnership(o2);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        nreg.listNft(10, address(0xBEEF), Policy.Burn, "pending owner as buyer", o2, 1 ether);
        vm.stopPrank();
        assertFalse(lr.isAutoBuyer(other));
        // and the rotation still goes through after its seven days, untouched
        Clock.skip(7 days + 1);
        vm.prank(owner);
        lr.acceptRoles();
        assertEq(lr.keeper(), other);
    }

    /// The seven-day delay is unchanged for every rotation that does not touch a buyer.
    function test_rotationsToOtherAddressesStillWaitSevenDays() public {
        address k2 = makeAddr("k2");
        vm.prank(owner);
        lr.proposeRoles(k2, purchaser, attestor);
        Clock.skip(6 days);
        vm.prank(owner);
        vm.expectRevert();
        lr.acceptRoles();
        Clock.skip(1 days + 1);
        vm.prank(owner);
        lr.acceptRoles();
        assertEq(lr.keeper(), k2);
    }

    function test_onlyTheWiredNftRegistryReserves() public {
        vm.prank(stranger);
        vm.expectRevert(ListingRegistry.NotNftRegistry.selector);
        lr.reserveBuyer(other);
        vm.prank(owner);
        vm.expectRevert(ListingRegistry.NotNftRegistry.selector);
        lr.reserveBuyer(other);
        // and the NFT registry cannot reserve a live role either (it would first refuse it itself; call ListingRegistry directly)
        vm.prank(address(nreg));
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.reserveBuyer(keeper);
        vm.prank(address(nreg));
        vm.expectRevert(ListingRegistry.NotNftRegistry.selector);
        lr.reserveBuyer(address(0));
        assertFalse(lr.isAutoBuyer(other));
    }

    function test_wireNftOnceAndOwnerOnly() public {
        vm.prank(stranger);
        vm.expectRevert();
        lr.wireNft(stranger);
        vm.prank(owner);
        vm.expectRevert(ListingRegistry.AlreadyWired.selector);
        lr.wireNft(stranger);
    }

    /// Fail closed: a ListingRegistry nobody wired to the NFT registry refuses every remote listing.
    function test_anUnwiredRegistryRefusesRemoteListings() public {
        vm.startPrank(owner);
        ListingRegistry lr2 = new ListingRegistry(owner, treasury);
        lr2.proposeRoles(keeper, purchaser, attestor);
        NftRegistry n2 = new NftRegistry(lr2, IRandomnessBeacon(address(beacon)), address(market));
        vm.expectRevert(ListingRegistry.NotNftRegistry.selector);
        n2.listNft(1, address(0xBEEF), Policy.Burn, "unwired", canister, 1 ether);
        // a native listing names no buyer, and needs no wiring
        n2.listNft(uint64(block.chainid), address(nft), Policy.Burn, "native", payable(0), 0);
        lr2.wireNft(address(n2));
        n2.listNft(1, address(0xBEEF), Policy.Burn, "wired", canister, 1 ether);
        vm.stopPrank();
        assertTrue(lr2.isAutoBuyer(canister));
    }

    /// One canister key may buy for many listings: reserving it twice is fine.
    function test_theSameBuyerForTwoListingsIsFine() public {
        vm.prank(owner);
        nreg.listNft(10, address(0xBEEF), Policy.Raffle, "Optimism", canister, 1 ether);
        assertTrue(lr.isAutoBuyer(canister));
    }

    /// The v2 path has the same pending-role gap closed: a buyer cannot be an address about to become a role.
    function test_aV2BuyerCannotBeAPendingRoleEither() public {
        vm.startPrank(owner);
        lr.proposeRoles(keeper, other, attestor);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.listRuneAuto("Rune", "840000:1", 0, "bc1q-vault", other, 1 ether);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.listRuneAuto("Rune", "840000:1", 0, "bc1q-vault", payable(treasury), 1 ether);
        vm.stopPrank();
    }
}
