// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {FloorBase} from "./FloorBase.sol";
import {Merkle} from "../utils/Merkle.sol";
import {Wrapper} from "../../src/Wrapper.sol";
import {NftWrapper} from "../../src/nft/NftWrapper.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {IFloorWrapped} from "../../src/interfaces/IFloorWrapped.sol";

/// H11 (docs/v5/custody-hardening.md): nothing on-chain limited how fast the custodian could mint. Now each wrapper lets at most
/// `maxMintsPerDay` items in (purchases, deposits, rentals, the creator's wrap) in any 24 hours on both clocks; the deployer (the
/// timelock) lowers it at once and raises it only through a proposal anyone can see for seven days. The floor module's own
/// item-arrival limit sits on top (`FloorModuleTest.test_itemArrivalsAreLimitedPerDay_onBothClocks`).
contract MintRateTest is FloorBase {
    Wrapper w2;
    NftWrapper apes;

    function setUp() public override {
        super.setUp();
        w2 = new Wrapper(registry, distributor, beacon, FROGS, distribution, custodian, CUSTODY_BTC, "w2", "w2", PBS);   // default
        apes = new NftWrapper(beacon, 1, makeAddr("BAYC"), custodian, "Wrapped Bored Ape", "wAPE");
    }

    function _dep(Wrapper w, uint256 i) internal {
        vm.prank(custodian);
        w.wrapDeposit(frogIds[i], Merkle.proof(frogLeaves, i), keccak256(abi.encode("dep", i, nextTx++)), 0, alice);
    }

    function _mintApe(uint256 tokenId) internal {
        vm.prank(custodian);
        apes.mint(tokenId, keccak256(abi.encode("buy", tokenId)), alice);
    }

    function test_theDefaultIsTwentyADay_andTheTwentyFirstWaits() public {
        assertEq(w2.maxMintsPerDay(), 20);
        for (uint256 i = 0; i < 20; ++i) _dep(w2, i);
        assertFalse(w2.mintRoom());
        vm.prank(custodian);
        vm.expectRevert(Wrapper.MintRateLimited.selector);
        w2.wrapDeposit(frogIds[20], Merkle.proof(frogLeaves, 20), keccak256("x"), 0, alice);
        _skipTimeOnly(1 days);                                 // a sequencer ahead cannot open room early
        assertFalse(w2.mintRoom());
        _skip(1 days);
        assertTrue(w2.mintRoom());
        _dep(w2, 20);
        assertEq(w2.totalSupply(), 21e18);
        assertEq(w2.totalSupply(), (w2.held() - w2.openTickets()) * ONE + w2.ethBacked(), "the ledger");
    }

    function test_theWindowSlides() public {
        for (uint256 i = 0; i < 10; ++i) _dep(w2, i);
        _skip(12 hours);
        for (uint256 i = 10; i < 20; ++i) _dep(w2, i);
        assertFalse(w2.mintRoom());
        _skip(12 hours);                                       // the first ten are a day old; the last ten are not
        for (uint256 i = 20; i < 30; ++i) _dep(w2, i);
        assertFalse(w2.mintRoom(), "exactly ten more");
    }

    function test_nftWrapperCountsPurchasesAndDeposits() public {
        for (uint256 i = 0; i < 10; ++i) _mintApe(i);
        for (uint256 i = 10; i < 20; ++i) {
            vm.prank(custodian);
            apes.mintDeposit(i, keccak256(abi.encode("deposit", i)), bob);
        }
        vm.prank(custodian);
        vm.expectRevert(NftWrapper.MintRateLimited.selector);
        apes.mint(99, keccak256("late"), alice);
        _skip(1 days);
        _mintApe(99);
    }

    function test_rentalsCount_throughTheModule() public {
        // the module's wrapper (FloorBase) was given 1,000; lower it to the number already used, then rent
        wrapper.proposeMintLimit(1);
        _mint(alice, 2 ether);
        _rent(32, renter);
        vm.prank(custodian);
        vm.expectRevert(Wrapper.MintRateLimited.selector);
        module.rentIn(renter, bytes(TAPROOT), Merkle.proof(rentLeaves, 1),
            abi.encode(frogIds[33], Merkle.proof(frogLeaves, 33), keccak256("r2"), uint32(0)));
    }

    function test_loweringIsImmediate_raisingWaitsSevenDaysOnBothClocks() public {
        w2.proposeMintLimit(5);
        assertEq(w2.maxMintsPerDay(), 5, "lower: at once");
        w2.proposeMintLimit(50);
        assertEq(w2.maxMintsPerDay(), 5, "raise: not yet");
        assertEq(w2.pendingMintLimit(), 50);
        vm.expectRevert(Wrapper.TooEarly.selector);
        w2.acceptMintLimit();
        _skipTimeOnly(8 days);
        vm.expectRevert(Wrapper.TooEarly.selector);
        w2.acceptMintLimit();                                  // the parent chain has not moved a week
        _skip(7 days);
        vm.prank(stranger);
        w2.acceptMintLimit();                                  // anyone, once it is due
        assertEq(w2.maxMintsPerDay(), 50);
        vm.expectRevert(Wrapper.WrongStage.selector);
        w2.acceptMintLimit();
    }

    function test_onlyTheDeployerSetsIt_andInitOnlyBeforeAnyMint() public {
        vm.prank(stranger);
        vm.expectRevert(Wrapper.NotDeployer.selector);
        w2.proposeMintLimit(100);
        vm.prank(stranger);
        vm.expectRevert(Wrapper.NotDeployer.selector);
        w2.initMintLimit(100);
        vm.expectRevert(Wrapper.BadInput.selector);
        w2.proposeMintLimit(0);
        _dep(w2, 0);
        vm.expectRevert(Wrapper.NotFresh.selector);
        w2.initMintLimit(100);
        vm.prank(stranger);
        vm.expectRevert(NftWrapper.NotDeployer.selector);
        apes.proposeMintLimit(100);
    }

    /// Refunds are not mints: a ticket nobody delivered puts its item back and its unit back, whatever the rate.
    function test_aRefundIsNotAMint() public {
        w2.proposeMintLimit(1);
        _dep(w2, 0);
        vm.prank(alice);
        uint256 id = w2.redeem(TAPROOT);
        w2.resolve(id, abi.encodePacked(bytes32(uint256(3))));
        _skip(7 days + 72 hours + 1 hours);                  // the deadline and the refund grace
        vm.prank(custodian);
        w2.markUndeliverable(id);                            // the custodian's attestation (review2 R3 §4)
        w2.refund(id);
        assertEq(w2.balanceOf(alice), 1e18);
    }
}
