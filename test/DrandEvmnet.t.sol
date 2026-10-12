// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {DrandEvmnet} from "../src/rand/DrandEvmnet.sol";

contract DrandEvmnetTest is Test {
    DrandEvmnet beacon;
    bytes constant SIG_1000 = hex"06fd5996329504d3a56b482d9222bf7205857d0a9559ddd216ca31a286f6a8cc0a120f021aac2f13553fb164f62bc3a5ca32c76dea88a777b39bcf3cac5fdbd6";
    bytes32 constant RAND_1000 = 0x0e6745667465a6f9dce5d5f994656955080be14c469ff17fc4fc588c925a8504;
    bytes constant SIG_20823703 = hex"205ff8f2b2cb01f39deb17fe64eed496207c8fa1a490e5bf0db027ced4665bcf22504bf545a9880b36617734cede20622276674ec0c5599561ac68e8f5432ad6";
    bytes32 constant RAND_20823703 = 0x13b91b6a08f04051999beeca41f8ce3f99e1bf8004a128ba49e3fa27f0c4ba25;

    function setUp() public {
        beacon = new DrandEvmnet();
    }

    function test_realBeaconsVerify_andRandomnessIsTheNetworksOwn() public {
        assertEq(beacon.verify(1000, SIG_1000), RAND_1000);
        assertEq(beacon.verify(20823703, SIG_20823703), RAND_20823703);
        vm.prank(address(0xBEEF));
        assertEq(beacon.prove(1000, SIG_1000), RAND_1000);
        assertEq(beacon.randomnessOf(1000), RAND_1000);
        assertEq(beacon.randomnessOf(1001), bytes32(0));
    }

    function test_aRealSignatureForAnotherRoundIsRefused() public {
        vm.expectRevert(DrandEvmnet.InvalidSignature.selector);
        beacon.verify(1001, SIG_1000);
    }

    function test_aTamperedSignatureIsRefused() public {
        bytes memory bad = SIG_1000;
        bad[63] = bytes1(uint8(bad[63]) ^ 1);
        vm.expectRevert();      // off the curve: the pairing precompile itself fails
        beacon.verify(1000, bad);
    }

    function test_anotherValidPointIsRefused() public {
        // the G1 generator is on the curve and is nobody's signature
        bytes memory g = abi.encodePacked(uint256(1), uint256(2));
        vm.expectRevert(DrandEvmnet.InvalidSignature.selector);
        beacon.verify(1000, g);
    }

    function test_lengthAndZeroAreRefused() public {
        vm.expectRevert(DrandEvmnet.BadSignatureLength.selector);
        beacon.verify(1000, hex"00");
        vm.expectRevert(DrandEvmnet.SignatureNotOnCurve.selector);
        beacon.verify(1000, new bytes(64));
    }

    function test_provingTwiceChangesNothing_whoeverProves() public {
        vm.prank(address(1));
        bytes32 a = beacon.prove(1000, SIG_1000);
        vm.prank(address(2));
        bytes32 b = beacon.prove(1000, hex"deadbeef");   // already proven: the argument is not even read
        assertEq(a, b);
    }

    function test_roundClock() public view {
        assertEq(beacon.roundAt(1727521075), 1);
        assertEq(beacon.roundAt(1727521075 + 2), 1);
        assertEq(beacon.roundAt(1727521075 + 3), 2);
        assertEq(beacon.roundAt(1), 0);
        assertEq(beacon.timeOf(2), 1727521078);
        assertEq(beacon.roundAt(beacon.timeOf(20823703)), 20823703);
    }
}
