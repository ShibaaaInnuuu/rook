// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {DrandEvmnet} from "../../src/rand/DrandEvmnet.sol";
import {NftWrapper} from "../../src/nft/NftWrapper.sol";

/// The NFT wrapper against the real drand evmnet verifier and real beacons (test/fixtures/drand-evmnet-20.json): a burn pins the
/// round a real beacon later signed, anyone resolves it with that signature, and the NFT drawn is the formula's — no mock.
contract NftWrapperDrandTest is Test {
    DrandEvmnet beacon;
    NftWrapper w;
    address custodian = makeAddr("custodian");
    address alice = makeAddr("alice");

    function _beacon(uint256 i) internal view returns (uint64 round, bytes memory sig, bytes32 randomness) {
        string memory j = vm.readFile("test/fixtures/drand-evmnet-20.json");
        string memory at = string.concat(".beacons[", vm.toString(i), "]");
        round = uint64(vm.parseJsonUint(j, string.concat(at, ".round")));
        sig = vm.parseBytes(vm.parseJsonString(j, string.concat(at, ".signature")));
        randomness = bytes32(vm.parseBytes(vm.parseJsonString(j, string.concat(at, ".randomness"))));
    }

    function setUp() public {
        beacon = new DrandEvmnet();
        w = new NftWrapper(beacon, 1, makeAddr("BAYC"), custodian, "Wrapped Bored Ape", "wAPE");
    }

    function test_aRealBeaconDecidesTheNft_andAWrongSignatureCannot() public {
        for (uint256 i = 0; i < 5; ++i) {
            vm.prank(custodian);
            w.mint(100 + i, keccak256(abi.encode(i)), alice);
        }
        (uint64 round, bytes memory sig, bytes32 randomness) = _beacon(19);
        (, bytes memory otherSig,) = _beacon(18);
        vm.warp(beacon.timeOf(round - w.PIN_AHEAD()));
        vm.roll(26_000_000);
        vm.prank(alice);
        uint256 id = w.redeem(alice);
        assertEq(w.ticket(id).round, round, "the burn pinned the round the real beacon signed");

        vm.warp(beacon.timeOf(round) + 3);
        vm.expectRevert(DrandEvmnet.InvalidSignature.selector);
        w.resolve(id, otherSig);
        vm.prank(makeAddr("anyone"));
        w.resolve(id, sig);
        assertEq(beacon.randomnessOf(round), randomness);
        uint256 index = uint256(keccak256(abi.encode(randomness, address(w), id))) % 5;
        assertEq(w.ticket(id).itemId, index);
        assertEq(w.item(index).tokenId, 100 + index);
    }
}
