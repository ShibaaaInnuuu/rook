// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase} from "./utils/SweepBase.sol";
import {Merkle} from "./utils/Merkle.sol";
import {Clock} from "./utils/Clock.sol";
import {Wrapper} from "../src/Wrapper.sol";

/// docs/v3/sweep.md, 1a: a holder's frog, sent to custody, mints one wFROG to the address in its OP_RETURN. The canister's
/// checks (depth, clean sats, the OP_RETURN) are not here; what a contract can check is.
contract WrapperDepositTest is SweepBase {
    address alice = makeAddr("alice");
    bytes32 constant TX = keccak256("deposit");

    function _args(uint256 i) internal view returns (string memory id, bytes32[] memory proof) {
        id = frogIds[i];
        proof = Merkle.proof(frogLeaves, i);
    }

    function test_aDepositMintsOneToTheNamedAddress() public {
        (string memory id, bytes32[] memory proof) = _args(0);
        vm.prank(custodian);
        uint256 frog = wrapper.wrapDeposit(id, proof, TX, 1, alice);
        assertEq(wrapper.balanceOf(alice), ONE);
        assertEq(wrapper.held(), 1);
        assertEq(frog, wrapper.DEPOSIT_FLAG(), "the first deposit is DEPOSIT_FLAG | 0");
        assertTrue(wrapper.inscriptionHeld(registry.leafOf(id)));
        _assertLedger();
    }

    function test_onlyTheCustodianAttests() public {
        (string memory id, bytes32[] memory proof) = _args(0);
        address[3] memory notCustodian = [keeper, alice, owner];
        for (uint256 i = 0; i < 3; ++i) {
            vm.prank(notCustodian[i]);
            vm.expectRevert(Wrapper.NotCustodian.selector);
            wrapper.wrapDeposit(id, proof, TX, 0, alice);
        }
    }

    function test_aFrogOutsideTheCollectionIsRefused() public {
        (, bytes32[] memory proof) = _args(0);
        vm.prank(custodian);
        vm.expectRevert(Wrapper.NotInCollection.selector);
        wrapper.wrapDeposit("ffff0000i0", proof, TX, 0, alice);
    }

    function test_anOutpointMintsOnceEver() public {
        (string memory id, bytes32[] memory proof) = _args(0);
        (string memory id1, bytes32[] memory proof1) = _args(1);
        vm.prank(custodian);
        wrapper.wrapDeposit(id, proof, TX, 0, alice);
        vm.prank(custodian);
        vm.expectRevert(Wrapper.AlreadyMinted.selector);     // a reorg re-including the same deposit
        wrapper.wrapDeposit(id1, proof1, TX, 0, alice);
    }

    function test_aFrogAlreadyHeldCannotMintAgain() public {
        (string memory id, bytes32[] memory proof) = _args(0);
        vm.prank(custodian);
        wrapper.wrapDeposit(id, proof, TX, 0, alice);
        vm.prank(custodian);
        vm.expectRevert(Wrapper.AlreadyHeld.selector);
        wrapper.wrapDeposit(id, proof, keccak256("another tx"), 0, alice);
    }

    function test_aDeliveredFrogMayBeDepositedAgain() public {
        (string memory id, bytes32[] memory proof) = _args(0);
        vm.prank(custodian);
        wrapper.wrapDeposit(id, proof, TX, 0, alice);
        vm.prank(alice);
        uint256 t = wrapper.redeem(TAPROOT);
        _resolveTicket(t, bytes32(uint256(5)));
        vm.prank(custodian);
        wrapper.recordDelivery(t, keccak256("out"));
        assertFalse(wrapper.inscriptionHeld(registry.leafOf(id)));
        vm.prank(custodian);
        wrapper.wrapDeposit(id, proof, keccak256("back in"), 0, alice);
        assertEq(wrapper.held(), 1);
        _assertLedger();
    }

    function test_aDepositedFrogIsDrawableAndRefundable() public {
        _deposit(alice);
        vm.prank(alice);
        uint256 t = wrapper.redeem(TAPROOT);
        _resolveTicket(t, bytes32(uint256(5)));
        assertEq(wrapper.ticket(t).acquisitionId, wrapper.DEPOSIT_FLAG());
        Clock.skip(7 days + 72 hours);                        // the deadline and the refund grace
        vm.prank(custodian);
        wrapper.markUndeliverable(t);                         // the custodian's attestation (review2 R3 §4)
        wrapper.refund(t);
        assertEq(wrapper.balanceOf(alice), ONE);
        _assertLedger();
    }

    function test_zeroRecipientOrTxidRefused() public {
        (string memory id, bytes32[] memory proof) = _args(0);
        vm.prank(custodian);
        vm.expectRevert(Wrapper.BadInput.selector);
        wrapper.wrapDeposit(id, proof, TX, 0, address(0));
        vm.prank(custodian);
        vm.expectRevert(Wrapper.BadInput.selector);
        wrapper.wrapDeposit(id, proof, bytes32(0), 0, alice);
    }

    function test_refusedWhileTheFrogListingIsPaused() public {
        (string memory id, bytes32[] memory proof) = _args(0);
        vm.prank(owner);
        registry.setPaused(FROGS, true);
        vm.prank(custodian);
        vm.expectRevert(Wrapper.ListingPaused.selector);
        wrapper.wrapDeposit(id, proof, TX, 0, alice);
    }

    function testFuzz_depositsKeepTheLedger(uint8 n, uint8 redeems) public {
        n = uint8(bound(n, 1, 40));
        redeems = uint8(bound(redeems, 0, n));
        for (uint256 i = 0; i < n; ++i) _deposit(alice);
        for (uint256 i = 0; i < redeems; ++i) {
            vm.prank(alice);
            wrapper.redeem(TAPROOT);
            _assertLedger();
        }
        assertEq(wrapper.held(), n);
        assertEq(wrapper.balanceOf(alice), uint256(n - redeems) * ONE);
    }
}
