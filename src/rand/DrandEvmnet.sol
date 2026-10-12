// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {BN254HashToCurve} from "./BN254HashToCurve.sol";
import {IRandomnessBeacon} from "../interfaces/IRandomnessBeacon.sol";

/// @title DrandEvmnet
/// @notice Verifies beacons of drand's `evmnet` network (scheme `bls-bn254-unchained-on-g1`, a round every 3 s) on chain and keeps
/// the randomness of every round somebody has proven. Anyone may prove any round; the result does not depend on who does.
/// No owner, no fee, no allowlist, nothing to upgrade: the network's public key and genesis are constants.
/// @dev What this does NOT protect against: drand itself. A threshold of the League of Entropy's nodes colluding could sign a round
/// early or choose between signatures; nothing on any chain can detect that. It also says nothing about whether a caller chose
/// WHEN to pin a round — that is the consumer's job (pin a round that does not exist yet, and never let it be changed).
contract DrandEvmnet is IRandomnessBeacon {
    /// drand evmnet: chain hash 04f1e9062b8a81f848fded9c12306733282b2727ecced50032187751166ec8c3
    uint64 public constant GENESIS = 1727521075;
    uint64 public constant PERIOD = 3;
    bytes public constant DST = "BLS_SIG_BN254G1_XMD:KECCAK-256_SVDW_RO_NUL_";

    // the network's public key, a G2 point, in the order the pairing precompile reads: x.c1, x.c0, y.c1, y.c0
    uint256 private constant PK_X1 = 0x07e1d1d335df83fa98462005690372c643340060d205306a9aa8106b6bd0b382;
    uint256 private constant PK_X0 = 0x0557ec32c2ad488e4d4f6008f89a346f18492092ccc0d594610de2732c8b808f;
    uint256 private constant PK_Y1 = 0x0095685ae3a85ba243747b1b2f426049010f6b73a0cf1d389351d5aaaa1047f6;
    uint256 private constant PK_Y0 = 0x297d3a4f9749b33eb2d904c9d9ebf17224150ddd7abd7567a9bec6c74480ee0b;
    // the G2 generator, same order
    uint256 private constant G2_X1 = 11559732032986387107991004021392285783925812861821192530917403151452391805634;
    uint256 private constant G2_X0 = 10857046999023057135944570762232829481370756359578518086990519993285655852781;
    uint256 private constant G2_Y1 = 4082367875863433681332203403145435568316851327593401208105741076214120093531;
    uint256 private constant G2_Y0 = 8495653923123431417604973247489272438418190587263600148770280649306958101930;

    mapping(uint64 round => bytes32 randomness) private _randomness;

    event RoundProven(uint64 indexed round, bytes32 randomness, address indexed by);

    error BadSignatureLength();
    error SignatureNotOnCurve();
    error InvalidSignature();
    error PairingFailed();

    /// @inheritdoc IRandomnessBeacon
    function roundAt(uint256 timestamp) public pure returns (uint64) {
        if (timestamp < GENESIS) return 0;
        return uint64((timestamp - GENESIS) / PERIOD) + 1;
    }

    /// @inheritdoc IRandomnessBeacon
    function timeOf(uint64 round) public pure returns (uint256) {
        return round == 0 ? 0 : GENESIS + uint256(round - 1) * PERIOD;
    }

    /// @inheritdoc IRandomnessBeacon
    function randomnessOf(uint64 round) external view returns (bytes32) {
        return _randomness[round];
    }

    /// @inheritdoc IRandomnessBeacon
    function prove(uint64 round, bytes calldata signature) external returns (bytes32 randomness) {
        randomness = _randomness[round];
        if (randomness != bytes32(0)) return randomness;        // a round has one valid signature; proving it twice changes nothing
        randomness = verify(round, signature);
        _randomness[round] = randomness;
        emit RoundProven(round, randomness, msg.sender);
    }

    /// @notice Reverts unless `signature` is evmnet's signature of `round`; returns the round's randomness, sha256(signature).
    function verify(uint64 round, bytes calldata signature) public view returns (bytes32) {
        if (signature.length != 64) revert BadSignatureLength();
        uint256 sx = uint256(bytes32(signature[0:32]));
        uint256 sy = uint256(bytes32(signature[32:64]));
        uint256 p = BN254HashToCurve.P;
        if (sx >= p || sy >= p || (sx == 0 && sy == 0)) revert SignatureNotOnCurve();
        // docs/phase3.md L5: a point off the curve makes the pairing precompile FAIL, and a failed precompile burns every unit of
        // gas it was given. Checking y^2 = x^3 + 3 here costs a few hundred gas and turns that into an ordinary revert.
        if (mulmod(sy, sy, p) != addmod(mulmod(mulmod(sx, sx, p), sx, p), 3, p)) revert SignatureNotOnCurve();
        (uint256 hx, uint256 hy) = BN254HashToCurve.hashToPoint(DST, abi.encodePacked(keccak256(abi.encodePacked(round))));
        // e(-sig, G2) * e(H(m), pk) == 1
        uint256[12] memory input = [sx, p - sy, G2_X1, G2_X0, G2_Y1, G2_Y0, hx, hy, PK_X1, PK_X0, PK_Y1, PK_Y0];
        uint256[1] memory out;
        bool ok;
        assembly {
            ok := staticcall(gas(), 0x08, input, 0x180, out, 0x20)
        }
        if (!ok) revert PairingFailed();                        // also what a signature off the curve produces
        if (out[0] != 1) revert InvalidSignature();
        return sha256(signature);
    }
}
