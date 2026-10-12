// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "./utils/Base.sol";
import {ListingRegistry} from "../src/ListingRegistry.sol";
import {AssetVault} from "../src/AssetVault.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Clock} from "./utils/Clock.sol";

contract ListingRegistryTest is Base {
    function test_listingsAndTheirVaults() public view {
        assertEq(registry.count(), 2);
        ListingRegistry.Listing memory a = registry.listing(0);
        ListingRegistry.Listing memory b = registry.listing(1);
        assertEq(uint8(a.kind), uint8(ListingRegistry.Kind.Inscriptions));
        assertEq(uint8(b.kind), uint8(ListingRegistry.Kind.Rune));
        assertEq(b.divisibility, 2);
        assertEq(b.ref, "840000:1");
        assertFalse(ordVault.isRune());
        assertTrue(runeVault.isRune());
        assertEq(address(ordVault.registry()), address(registry));
        assertEq(runeVault.listingId(), 1);
    }

    function test_onlyTheOwnerLists() public {
        vm.prank(stranger);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, stranger));
        registry.listRune("X", "1:1", 0, "bc1p");
        vm.prank(keeper);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, keeper));
        registry.listInscriptions("X", "x", bytes32(uint256(1)), "bc1p");
    }

    function test_aListingNeedsItsFacts() public {
        vm.startPrank(owner);
        vm.expectRevert(ListingRegistry.Empty.selector);
        registry.listInscriptions("X", "x", bytes32(0), "bc1p");
        vm.expectRevert(ListingRegistry.Empty.selector);
        registry.listRune("", "1:1", 0, "bc1p");
        vm.expectRevert(ListingRegistry.Empty.selector);
        registry.listRune("X", "1:1", 39, "bc1p");
        vm.stopPrank();
    }

    function test_membership() public view {
        for (uint256 i = 0; i < ids.length; ++i) assertTrue(registry.isMember(0, ids[i], _proofOf(i)));
        assertFalse(registry.isMember(0, "eeee5555i0", _proofOf(0)));
        assertFalse(registry.isMember(1, ids[0], _proofOf(0)), "a rune listing has no members");
    }

    function test_aMembershipRootChangesOnlyAfterSevenPublicDays() public {
        bytes32 next = keccak256("another collection");
        vm.startPrank(owner);
        registry.proposeRoot(0, next);
        vm.expectRevert();
        registry.acceptRoot(0);
        Clock.skip(7 days - 1);
        vm.expectRevert();
        registry.acceptRoot(0);
        assertTrue(registry.isMember(0, ids[0], _proofOf(0)), "still the old collection");
        Clock.skip(1);
        registry.acceptRoot(0);
        vm.stopPrank();
        assertEq(registry.listing(0).membershipRoot, next);
        assertFalse(registry.isMember(0, ids[0], _proofOf(0)));
    }

    function test_aRuneHasNoRoot() public {
        vm.prank(owner);
        vm.expectRevert(ListingRegistry.WrongKind.selector);
        registry.proposeRoot(1, keccak256("x"));
    }

    function test_whoeverSpendsNeverVouches() public {
        vm.startPrank(owner);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        registry.proposeRoles(keeper, purchaser, purchaser);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        registry.proposeRoles(keeper, purchaser, keeper);
        vm.expectRevert(ListingRegistry.ZeroAddress.selector);
        registry.proposeRoles(keeper, address(0), attestor);
        vm.stopPrank();
    }

    function test_rolesChangeOnlyAfterSevenPublicDays() public {
        address k2 = makeAddr("k2"); address p2 = makeAddr("p2"); address a2 = makeAddr("a2");
        vm.startPrank(owner);
        registry.proposeRoles(k2, p2, a2);
        assertEq(registry.keeper(), keeper, "the first roles applied at once; these wait");
        vm.expectRevert();
        registry.acceptRoles();
        Clock.skip(7 days);
        registry.acceptRoles();
        vm.stopPrank();
        assertEq(registry.keeper(), k2);
        assertEq(registry.purchaser(), p2);
        assertEq(registry.attestor(), a2);
    }

    function test_wiredOnce() public {
        vm.prank(owner);
        vm.expectRevert(ListingRegistry.AlreadyWired.selector);
        registry.wire(address(1), address(2));
    }

    function test_theTreasuryAndTheBitcoinAddressChangeSlowlyToo() public {
        address t2 = makeAddr("t2");
        vm.startPrank(owner);
        registry.proposeTreasury(t2);
        vm.expectRevert();
        registry.acceptTreasury();
        registry.proposeBtcVault(0, "bc1p-new");
        vm.expectRevert();
        registry.acceptBtcVault(0);
        Clock.skip(7 days);
        registry.acceptTreasury();
        registry.acceptBtcVault(0);
        vm.stopPrank();
        assertEq(registry.treasury(), t2);
        assertEq(registry.listing(0).btcVault, "bc1p-new");
    }

    function test_pausingIsTheOwners() public {
        vm.prank(keeper);
        vm.expectRevert();
        registry.setPaused(0, true);
        vm.prank(owner);
        registry.setPaused(0, true);
        assertTrue(registry.isPaused(0));
        vm.expectRevert(ListingRegistry.NoListing.selector);
        registry.isPaused(7);
    }
}
