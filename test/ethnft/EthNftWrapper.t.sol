// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {EthNftBase} from "./EthNftBase.sol";
import {EthNftWrapper, IListingPause} from "../../src/nft/EthNftWrapper.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {IFloorWrapped} from "../../src/interfaces/IFloorWrapped.sol";
import {IKeeperSource} from "../../src/interfaces/IKeeperSource.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";

/// docs/v7/phase1-nft.md §3.1: `Wrapper`'s audited ticket machine keyed by ERC-721 token id.
contract EthNftWrapperTest is EthNftBase {
    // ---- construction and wiring ----

    function test_constructorRefusals() public {
        IListingPause r = IListingPause(address(registry));
        IRandomnessBeacon b = IRandomnessBeacon(address(beacon));
        vm.expectRevert(EthNftWrapper.BadInput.selector);
        new EthNftWrapper(IListingPause(address(0)), 0, b, 1, collection, custodian, "x", "x", PBS);
        vm.expectRevert(EthNftWrapper.BadInput.selector);
        new EthNftWrapper(r, 0, IRandomnessBeacon(address(0)), 1, collection, custodian, "x", "x", PBS);
        vm.expectRevert(EthNftWrapper.BadInput.selector);
        new EthNftWrapper(r, 0, b, 1, address(0), custodian, "x", "x", PBS);
        vm.expectRevert(EthNftWrapper.BadInput.selector);
        new EthNftWrapper(r, 0, b, 1, collection, address(0), "x", "x", PBS);
        vm.expectRevert(EthNftWrapper.BadInput.selector);
        new EthNftWrapper(r, 0, b, 0, collection, custodian, "x", "x", PBS);
        vm.expectRevert(EthNftWrapper.BadInput.selector);
        new EthNftWrapper(r, 0, b, uint64(block.chainid), collection, custodian, "x", "x", PBS);
        vm.expectRevert(EthNftWrapper.BadInput.selector);   // a custodian with code is not the canister's EOA
        new EthNftWrapper(r, 0, b, 1, collection, address(registry), "x", "x", PBS);
        assertEq(w.chainId(), 1);
        assertEq(w.collection(), collection);
        assertEq(w.custodian(), custodian);
        assertEq(w.parentBlockSeconds(), PBS);
    }

    function test_wireFloorModule_B05_theModuleMustNameThisWrapper() public {
        EthNftWrapper w2 = new EthNftWrapper(IListingPause(address(registry)), 0, IRandomnessBeacon(address(beacon)), 1, collection,
            custodian, "x", "x", PBS);
        vm.expectRevert(EthNftWrapper.NotItsModule.selector);
        w2.wireFloorModule(address(module));                  // built for `w`, not for w2
        vm.prank(stranger);
        vm.expectRevert(EthNftWrapper.NotDeployer.selector);
        w2.wireFloorModule(address(module));
        vm.expectRevert(EthNftWrapper.AlreadyWired.selector);
        w.wireFloorModule(address(module));
        // not fresh: a module for w3, after w3 minted
        EthNftWrapper w3 = new EthNftWrapper(IListingPause(address(registry)), 0, IRandomnessBeacon(address(beacon)), 1, collection,
            custodian, "x", "x", PBS);
        FloorModule m3 = new FloorModule(IFloorWrapped(address(w3)), IKeeperSource(address(poster)), payee, platform, address(0), _params(), PBS);
        vm.prank(custodian);
        w3.mint(7, _tx(7), alice);
        vm.expectRevert(EthNftWrapper.NotFresh.selector);
        w3.wireFloorModule(address(m3));
    }

    // ---- mints ----

    function test_onlyTheCustodianMints_onceperTokenAndSource() public {
        vm.prank(stranger);
        vm.expectRevert(EthNftWrapper.NotCustodian.selector);
        w.mint(1, _tx(1), alice);
        vm.prank(stranger);
        vm.expectRevert(EthNftWrapper.NotCustodian.selector);
        w.mintDeposit(1, _tx(1), alice);
        uint256 id = _mintItem(1, alice);
        assertEq(w.balanceOf(alice), ONE);
        assertEq(w.itemOf(1), id + 1);
        assertEq(w.item(id).tokenId, 1);
        assertEq(w.item(id).sourceTx, _tx(1));
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.AlreadyHeld.selector);
        w.mint(1, keccak256("another tx"), alice);
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.BadInput.selector);
        w.mint(2, bytes32(0), alice);
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.BadInput.selector);
        w.mint(2, _tx(2), address(0));
        // delivered, then the same source can never mint again (a replayed attestation)
        uint256 t = _redeem(alice, alice);
        _resolve(t, 0);
        _deliver(t);
        assertEq(w.itemOf(1), 0);
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.AlreadyMinted.selector);
        w.mint(1, _tx(1), alice);
        // a new purchase of the same token (it came back) is a new source
        vm.prank(custodian);
        w.mint(1, keccak256("bought again"), alice);
        _books();
    }

    function test_aPausedListingStopsMints_butNotRedemption() public {
        _mintItem(1, alice);
        vm.prank(owner);
        registry.setPaused(0, true);
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.ListingPaused.selector);
        w.mint(2, _tx(2), alice);
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.ListingPaused.selector);
        w.mintDeposit(2, _tx(2), alice);
        uint256 t = _redeem(alice, bob);
        _resolve(t, 0);
        _deliver(t);
        assertEq(uint8(w.ticket(t).stage), uint8(EthNftWrapper.Stage.Delivered));
    }

    function test_H11_mintRate() public {
        EthNftWrapper w2 = new EthNftWrapper(IListingPause(address(registry)), 0, IRandomnessBeacon(address(beacon)), 1, collection,
            custodian, "x", "x", PBS);
        w2.initMintLimit(3);
        vm.startPrank(custodian);
        for (uint256 i = 0; i < 3; ++i) w2.mint(i, _tx(i), alice);
        vm.expectRevert(EthNftWrapper.MintRateLimited.selector);
        w2.mint(3, _tx(3), alice);
        vm.stopPrank();
        vm.warp(vm.getBlockTimestamp() + 1 days);            // one clock only: still limited
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.MintRateLimited.selector);
        w2.mint(3, _tx(3), alice);
        vm.roll(vm.getBlockNumber() + 1 days / PBS + 1);
        vm.prank(custodian);
        w2.mint(3, _tx(3), alice);
        vm.expectRevert(EthNftWrapper.NotFresh.selector);
        w2.initMintLimit(5);
    }

    function test_rentalsAreNotSupported() public {
        vm.expectRevert(EthNftWrapper.NoRentals.selector);
        w.admitRental("", "");
        vm.expectRevert(EthNftWrapper.NoRentals.selector);
        w.returnRental(0);
        assertEq(w.openReturns(), 0);
        assertEq(w.rentedHeld(), 0);
        assertEq(uint8(w.rentalState(0)), uint8(IFloorWrapped.RentalState.None));
    }

    function test_noRedrawExists_C02() public {
        (bool ok,) = address(w).call(abi.encodeWithSignature("redraw(uint256)", 0));
        assertFalse(ok, "there is no redraw: a pinned round is never replaced");
    }

    // ---- redemption ----

    function test_redeemRefusals() public {
        vm.prank(alice);
        vm.expectRevert(EthNftWrapper.NoItem.selector);
        w.redeem(alice);
        _mintItem(1, alice);
        vm.prank(alice);
        vm.expectRevert(EthNftWrapper.BadDestination.selector);
        w.redeem(address(0));
        vm.prank(alice);
        vm.expectRevert(EthNftWrapper.BadDestination.selector);
        w.redeem(custodian);
        _redeem(alice, alice);
        _mintAtModule(bob, 2 ether);                          // ETH-backed units do not make an item redeemable
        vm.prank(bob);
        vm.expectRevert(EthNftWrapper.NoItem.selector);
        w.redeem(bob);
    }

    function test_resolveInOrder_drawsOnlyItemsFreeAtTheBurn() public {
        _mintItem(1, alice);
        _mintItem(2, alice);
        uint256 a = _redeem(alice, alice);
        _mintItem(3, bob);                                    // free only after ticket a was burned
        uint256 b = _redeem(alice, alice);
        vm.expectRevert(abi.encodeWithSelector(EthNftWrapper.OutOfOrder.selector, a));
        w.resolve(b, abi.encodePacked(bytes32(uint256(5))));
        for (uint256 r = 1; r < 40; ++r) {
            uint256 snap = vm.snapshotState();
            _resolve(a, bytes32(r));
            assertTrue(_tokenOf(a) == 1 || _tokenOf(a) == 2, "ticket a draws among 1 and 2");
            vm.revertToState(snap);
        }
        _resolve(a, bytes32(uint256(9)));
        _resolve(b, bytes32(uint256(9)));
        assertTrue(_tokenOf(a) != _tokenOf(b));
        _books();
    }

    function test_deliveryAndBurn() public {
        _mintItem(1, alice);
        uint256 t;
        vm.prank(alice);
        t = w.redeemToBurn();
        assertEq(w.ticket(t).destination, DEAD);
        _resolve(t, 0);
        vm.prank(stranger);
        vm.expectRevert(EthNftWrapper.NotCustodian.selector);
        w.recordDelivery(t, keccak256("x"));
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.BadInput.selector);
        w.recordDelivery(t, bytes32(0));
        _deliver(t);
        assertEq(w.burned(), 1);
        assertEq(w.held(), 0);
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.WrongStage.selector);
        w.recordDelivery(t, keccak256("again"));
        _books();
    }

    // ---- refunds: the gated refund (D M-1, review2 N-2) and the pinned timeout (E-03) ----

    function test_aResolvedTicketRefundsOnlyAfterTheGraceWithTheAttestation_orTheLateWindow() public {
        _mintItem(1, alice);
        uint256 t = _redeem(alice, alice);
        _resolve(t, 0);
        uint64 by = w.ticket(t).deliverBy;
        vm.expectRevert(EthNftWrapper.TooEarly.selector);
        w.refund(t);
        _skip(by - vm.getBlockTimestamp() + 72 hours + 1);
        vm.expectRevert(EthNftWrapper.NotUndeliverable.selector);
        w.refund(t);
        // a late delivery can still be recorded: the refund has not happened
        uint256 snap = vm.snapshotState();
        _deliver(t);
        vm.expectRevert(EthNftWrapper.WrongStage.selector);
        w.refund(t);
        vm.revertToState(snap);
        vm.prank(stranger);
        vm.expectRevert(EthNftWrapper.NotCustodian.selector);
        w.markUndeliverable(t);
        vm.prank(custodian);
        w.markUndeliverable(t);
        w.refund(t);
        assertTrue(w.refunded(t));
        assertEq(w.balanceOf(alice), ONE, "the unit is back");
        assertEq(w.held(), 1, "the NFT is back in the set");
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.WrongStage.selector);
        w.recordDelivery(t, keccak256("too late: refunded"));
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.WrongStage.selector);
        w.markUndeliverable(t);
        _books();
    }

    function test_withoutTheAttestationTheLateWindowIsThirtyDays_onBothClocks() public {
        _mintItem(1, alice);
        uint256 t = _redeem(alice, alice);
        _resolve(t, 0);
        uint64 by = w.ticket(t).deliverBy;
        vm.warp(by + 30 days + 1);                           // the sequencer alone
        vm.expectRevert();
        w.refund(t);
        vm.roll(vm.getBlockNumber() + (37 days) / PBS + 2);
        w.refund(t);
        assertTrue(w.refunded(t));
    }

    function test_aPinnedTicketRefundsAfterThePinTimeout_andResolutionSkipsIt() public {
        _mintItem(1, alice);
        _mintItem(2, alice);
        uint256 a = _redeem(alice, alice);
        uint256 b = _redeem(alice, bob);
        vm.expectRevert(EthNftWrapper.TooEarly.selector);
        w.refund(a);
        _skip(7 days + 1);
        w.refund(a);
        assertEq(w.nextToResolve(), 1, "resolution moves past the refunded ticket");
        _resolve(b, 0);
        _deliver(b);
        _books();
    }

    function testFuzz_theLedgerHoldsThroughAnyMixOfDeliveriesAndRefunds(uint8 n, uint256 seed) public {
        n = uint8(bound(n, 1, 12));
        for (uint256 i = 0; i < n; ++i) _mintItem(i + 100, alice);
        for (uint256 i = 0; i < n; ++i) _redeem(alice, alice);
        for (uint256 i = 0; i < n; ++i) _resolve(i, bytes32(uint256(keccak256(abi.encode(seed, i)))));
        _skip(8 days + 72 hours);
        for (uint256 i = 0; i < n; ++i) {
            if (uint256(keccak256(abi.encode(seed, "d", i))) % 2 == 0) {
                _deliver(i);
            } else {
                vm.prank(custodian);
                w.markUndeliverable(i);
                w.refund(i);
            }
            _books();
        }
        assertEq(w.openTickets(), 0);
    }

    /// Audit 2026-09-27-nft A-M-2 (PoC `test_A_M2_threeUnitsFreezeEveryRedemptionIndefinitely_atNoCost`, now red): the relist
    /// ticket was a free, repeatable option that kept every held NFT promised to its burner's own tickets. It is refused, so a
    /// holder of `held − openTickets` units can no longer close redemption to anyone else.
    function test_A_M2_redeemToRelistIsRefused_soNoHolderCanLockEveryItem() public {
        for (uint256 i = 0; i < 3; ++i) _mintItem(9_000 + i, alice);
        vm.prank(alice);
        vm.expectRevert(EthNftWrapper.NotSupported.selector);
        w.redeemToRelist();
        assertEq(w.openTickets(), 0);
        assertEq(w.balanceOf(alice), 3 * ONE, "nothing burned");
        // everyone else can still redeem every item
        vm.prank(alice);
        w.transfer(bob, ONE);
        vm.prank(bob);
        w.redeem(bob);
        assertEq(w.openTickets(), 1);
    }
}
