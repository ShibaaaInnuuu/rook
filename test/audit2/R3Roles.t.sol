// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {NftBase} from "../nft/NftBase.sol";
import {NftRegistry} from "../../src/nft/NftRegistry.sol";
import {ListingRegistry} from "../../src/ListingRegistry.sol";
import {Policy} from "../../src/nft/INft.sol";

/// Second review (R2) N-6 and N-8, docs/audit/2026-09-26/review2/fixes-R3-contracts.md §6: the guardian is a key of its own, and
/// the NFT registry's buyer check reads the keeper as named (not zero while paused).
contract R3Roles is NftBase {
    address guardian = makeAddr("guardian");

    /// N-8: the guardian may be no other role, now or pending, and no auto-buyer; zero (none) and a key of its own are fine.
    function test_N8_theGuardianIsAKeyOfItsOwn() public {
        vm.startPrank(owner);
        address[5] memory taken = [owner, keeper, purchaser, attestor, treasury];
        for (uint256 i = 0; i < taken.length; ++i) {
            vm.expectRevert(ListingRegistry.SameKey.selector);
            lr.setGuardian(taken[i]);
        }
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.setGuardian(canister);                                  // an auto-buyer (reserved by the NFT listing)
        lr.setGuardian(guardian);
        assertEq(lr.guardian(), guardian);
        lr.setGuardian(guardian);                                  // setting the same one again is not a collision
        // and no role may later be moved onto it
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.proposeRoles(guardian, purchaser, attestor);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.proposeRoles(keeper, guardian, attestor);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.transferOwnership(guardian);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.proposeTreasury(guardian);
        // a pending role counts too
        lr.proposeRoles(makeAddr("k2"), purchaser, attestor);
        vm.expectRevert(ListingRegistry.SameKey.selector);
        lr.setGuardian(makeAddr("k2"));
        lr.setGuardian(address(0));                                // none
        assertEq(lr.guardian(), address(0));
        vm.stopPrank();
    }

    /// N-6: while the keeper is paused `keeper()` is zero; the NFT registry still refuses the named keeper as a buyer.
    function test_N6_aPausedKeeperIsStillNotABuyer() public {
        vm.prank(owner);
        lr.pauseKeeper();
        assertEq(lr.keeper(), address(0));
        vm.prank(owner);
        vm.expectRevert(abi.encodeWithSelector(NftRegistry.BadBuyer.selector, keeper));
        nreg.listNft(1, address(0xBEEF), Policy.Burn, "keeper as buyer while paused", payable(keeper), 1 ether);
    }
}
