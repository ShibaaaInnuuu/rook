// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {DrandEvmnet} from "../../src/rand/DrandEvmnet.sol";
import {BN254HashToCurve} from "../../src/rand/BN254HashToCurve.sol";

/// docs/phase3.md: the verifier against twenty real beacons (the network's first rounds, powers of two, and ten spread back from
/// the day of the fetch), against every malformed input worth naming, and under a gas ceiling.
contract DrandRealTest is Test {
    DrandEvmnet beacon;
    uint256 constant P = 21888242871839275222246405745257275088696311157297823662689037894645226208583;

    function setUp() public { beacon = new DrandEvmnet(); }

    function _beacon(uint256 i) internal view returns (uint64 round, bytes memory sig, bytes32 randomness) {
        string memory j = vm.readFile("test/fixtures/drand-evmnet-20.json");
        string memory at = string.concat(".beacons[", vm.toString(i), "]");
        round = uint64(vm.parseJsonUint(j, string.concat(at, ".round")));
        sig = vm.parseBytes(vm.parseJsonString(j, string.concat(at, ".signature")));
        randomness = bytes32(vm.parseBytes(vm.parseJsonString(j, string.concat(at, ".randomness"))));
    }

    function test_twentyRealBeaconsVerify_eachUnderTheGasCeiling_andNoneVerifiesForItsNeighbour() public {
        uint256 worst;
        for (uint256 i = 0; i < 20; ++i) {
            (uint64 round, bytes memory sig, bytes32 randomness) = _beacon(i);
            uint256 g = gasleft();
            bytes32 got = beacon.verify(round, sig);
            uint256 used = g - gasleft();
            if (used > worst) worst = used;
            assertEq(got, randomness, "sha256(signature) is the network's own randomness");
            vm.expectRevert(DrandEvmnet.InvalidSignature.selector);
            beacon.verify(round + 1, sig);
        }
        emit log_named_uint("worst verify gas over 20 real beacons", worst);
        assertLt(worst, 250_000, "a draw anyone can afford to settle");
    }

    function test_theNegatedSignatureIsOnTheCurveAndIsRefused() public {
        (uint64 round, bytes memory sig,) = _beacon(3);
        (uint256 x, uint256 y) = abi.decode(sig, (uint256, uint256));
        vm.expectRevert(DrandEvmnet.InvalidSignature.selector);
        beacon.verify(round, abi.encodePacked(x, P - y));
    }

    function test_coordinatesAtOrAboveTheFieldAreRefused_soASignatureHasOneEncoding() public {
        (uint64 round, bytes memory sig,) = _beacon(3);
        (uint256 x, uint256 y) = abi.decode(sig, (uint256, uint256));
        vm.expectRevert(DrandEvmnet.SignatureNotOnCurve.selector);
        beacon.verify(round, abi.encodePacked(x + P, y));         // the same point, written a second way: would be a second "randomness"
        vm.expectRevert(DrandEvmnet.SignatureNotOnCurve.selector);
        beacon.verify(round, abi.encodePacked(x, y + P));
        vm.expectRevert(DrandEvmnet.SignatureNotOnCurve.selector);
        beacon.verify(round, abi.encodePacked(P, uint256(0)));
    }

    /// docs/phase3.md L5: before the fix this call burned every unit of gas it was given (1,040,430,373 in the test runner), because a
    /// failing precompile consumes all forwarded gas.
    function test_L5_aPointOffTheCurveIsAnOrdinaryCheapRevert() public {
        (uint64 round,,) = _beacon(3);
        uint256 g = gasleft();
        try beacon.verify(round, abi.encodePacked(uint256(1), uint256(3))) { fail(); } catch (bytes memory err) {
            assertEq(bytes4(err), DrandEvmnet.SignatureNotOnCurve.selector);
        }
        assertLt(g - gasleft(), 60_000);
    }

    function test_roundZero_andAWrongLength() public {
        (, bytes memory sig,) = _beacon(0);
        vm.expectRevert(DrandEvmnet.InvalidSignature.selector);
        beacon.verify(0, sig);
        vm.expectRevert(DrandEvmnet.BadSignatureLength.selector);
        beacon.verify(1, abi.encodePacked(sig, bytes1(0)));
    }

    /// BN254's G1 has prime order (cofactor 1): every point on the curve is in the subgroup, so there is no small-subgroup
    /// signature to try. What there is to check is that the hash lands ON the curve for any message.
    function testFuzz_hashToPointIsAlwaysOnTheCurve(uint64 round) public view {
        (uint256 x, uint256 y) = BN254HashToCurve.hashToPoint(beacon.DST(), abi.encodePacked(keccak256(abi.encodePacked(round))));
        assertLt(x, P); assertLt(y, P);
        assertEq(mulmod(y, y, P), addmod(mulmod(mulmod(x, x, P), x, P), 3, P));
    }
}
