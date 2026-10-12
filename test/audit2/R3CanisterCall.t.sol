// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase} from "../utils/SweepBase.sol";
import {Wrapper} from "../../src/Wrapper.sol";

/// Integration R3 (docs/audit/2026-09-26/review2/integration-R3.md §1): the canister's `markUndeliverable` calldata, as the Rust
/// side builds it (`redeem::mark_undeliverable`, held to the same fixture by `autobuyer/canister-rs/tests/wrapper_abi.rs`), is
/// exactly the committed Wrapper's encoding, and those bytes do what the canister expects: accepted from the custodian on a
/// Resolved ticket, refused from anyone else and on any other stage.
contract R3CanisterCall is SweepBase {
    address alice = makeAddr("alice");

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    function _fixture() internal view returns (bytes memory) {
        string memory s = vm.readFile("test/fixtures/canister/mark-undeliverable-7.hex");
        return vm.parseBytes(vm.trim(s));
    }

    /// The canister's bytes for ticket 7, word for word.
    function _canisterCall(uint256 id) internal view returns (bytes memory) {
        bytes memory f = _fixture();
        bytes4 sel;
        assembly { sel := mload(add(f, 32)) }
        return abi.encodePacked(sel, id);
    }

    function test_theCanistersMarkUndeliverableBytesAreTheWrappersEncoding() public view {
        assertEq(_fixture(), abi.encodeCall(Wrapper.markUndeliverable, (7)), "fixture = abi.encodeCall(Wrapper.markUndeliverable, 7)");
        assertEq(Wrapper.markUndeliverable.selector, bytes4(0xacf756cc));
        assertEq(_canisterCall(7), _fixture());
    }

    function test_theCanistersBytesMarkAResolvedTicketFromTheCustodianOnly() public {
        _deposit(alice);
        _deposit(alice);
        vm.prank(alice);
        uint256 id = wrapper.redeem(TAPROOT);
        bytes memory call = _canisterCall(id);

        vm.prank(custodian);
        (bool ok, bytes memory ret) = address(wrapper).call(call);
        assertFalse(ok, "a pinned ticket cannot be marked");
        assertEq(bytes4(ret), Wrapper.WrongStage.selector);

        _skip(2 minutes);
        _resolveTicket(id, keccak256("r"));

        vm.prank(alice);
        (ok, ret) = address(wrapper).call(call);
        assertFalse(ok, "only the custodian");
        assertEq(bytes4(ret), Wrapper.NotCustodian.selector);

        vm.expectEmit(true, false, false, false, address(wrapper));
        emit Wrapper.Undeliverable(id);
        vm.prank(custodian);
        (ok,) = address(wrapper).call(call);
        assertTrue(ok, "the custodian marks a Resolved ticket");
        assertTrue(wrapper.undeliverable(id));
    }
}
