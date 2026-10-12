// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title BN254HashToCurve
/// @notice RFC 9380 hash_to_curve for BN254 G1 (y^2 = x^3 + 3): expand_message_xmd over Keccak-256, then the
/// Shallue–van de Woestijne map, then a point addition. G1 has cofactor 1, so nothing is cleared.
/// @dev Written from the RFC for one purpose: the message point of drand's `bls-bn254-unchained-on-g1` scheme.
/// It is checked in the tests against real beacons of that network, which is the only evidence offered that it is right.
/// It does NOT run in constant time and must never touch a secret; here every input is public.
library BN254HashToCurve {
    uint256 internal constant P = 21888242871839275222246405745257275088696311157297823662689037894645226208583;
    // SvdW constants for A = 0, B = 3, Z = 1 (RFC 9380 §6.6.1), computed in docs/phase0.md
    uint256 private constant C1 = 4;                                                                     // g(Z)
    uint256 private constant C2 = 0x183227397098d014dc2822db40c0ac2ecbc0b548b438e5469e10460b6c3e7ea3;    // -Z / 2
    uint256 private constant C3 = 0x16789af3a83522eb353c98fc6b36d713d5d8d1cc5dffffffa;                   // sqrt(-g(Z)(3Z^2+4A)), sgn0 = 0
    uint256 private constant C4 = 0x10216f7ba065e00de81ac1e7808072c9dd2b2385cd7b438469602eb24829a9bd;    // -4g(Z) / (3Z^2+4A)

    error ModExpFailed();
    error EcAddFailed();
    error DstTooLong();

    /// @notice The G1 point for `message` under domain separation tag `dst`.
    function hashToPoint(bytes memory dst, bytes memory message) internal view returns (uint256 x, uint256 y) {
        (uint256 u0, uint256 u1) = hashToField(dst, message);
        (uint256 x0, uint256 y0) = mapToPoint(u0);
        (uint256 x1, uint256 y1) = mapToPoint(u1);
        uint256[4] memory input = [x0, y0, x1, y1];
        uint256[2] memory out;
        bool ok;
        assembly {
            ok := staticcall(gas(), 0x06, input, 0x80, out, 0x40)
        }
        if (!ok) revert EcAddFailed();
        return (out[0], out[1]);
    }

    /// @notice hash_to_field with count = 2, m = 1, L = 48 (RFC 9380 §5.2).
    function hashToField(bytes memory dst, bytes memory message) internal pure returns (uint256 u0, uint256 u1) {
        bytes memory uniform = expandMessageXmd(dst, message, 96);
        u0 = _reduce48(uniform, 0);
        u1 = _reduce48(uniform, 48);
    }

    /// @notice expand_message_xmd (RFC 9380 §5.3.1) with H = Keccak-256: b_in_bytes = 32, s_in_bytes = 136.
    function expandMessageXmd(bytes memory dst, bytes memory message, uint16 lenInBytes) internal pure returns (bytes memory out) {
        if (dst.length > 255) revert DstTooLong();
        uint256 ell = (uint256(lenInBytes) + 31) / 32;
        bytes memory dstPrime = abi.encodePacked(dst, uint8(dst.length));
        bytes32 b0 = keccak256(abi.encodePacked(new bytes(136), message, lenInBytes, uint8(0), dstPrime));
        bytes32 bi = keccak256(abi.encodePacked(b0, uint8(1), dstPrime));
        out = abi.encodePacked(bi);
        for (uint256 i = 2; i <= ell; ++i) {
            bi = keccak256(abi.encodePacked(b0 ^ bi, uint8(i), dstPrime));
            out = abi.encodePacked(out, bi);
        }
        assembly {
            mstore(out, lenInBytes)
        }
    }

    /// @notice The SvdW map, RFC 9380 §F.1 line for line. Always lands on the curve.
    function mapToPoint(uint256 u) internal view returns (uint256 x, uint256 y) {
        uint256 tv1 = mulmod(mulmod(u, u, P), C1, P);
        uint256 tv2 = addmod(1, tv1, P);
        tv1 = addmod(1, P - tv1, P);
        uint256 tv3 = _inv0(mulmod(tv1, tv2, P));
        uint256 tv4 = mulmod(mulmod(mulmod(u, tv1, P), tv3, P), C3, P);
        uint256 x1 = addmod(C2, P - tv4, P);
        uint256 x2 = addmod(C2, tv4, P);
        uint256 x3 = mulmod(mulmod(tv2, tv2, P), tv3, P);
        x3 = addmod(mulmod(mulmod(x3, x3, P), C4, P), 1, P);
        if (_isSquare(_g(x1))) x = x1;
        else if (_isSquare(_g(x2))) x = x2;
        else x = x3;
        y = _sqrt(_g(x));
        if ((u & 1) != (y & 1)) y = P - y;   // sgn0(u) == sgn0(y); for a prime field sgn0 is the parity
    }

    function _g(uint256 x) private pure returns (uint256) {
        return addmod(mulmod(mulmod(x, x, P), x, P), 3, P);
    }

    function _reduce48(bytes memory b, uint256 offset) private pure returns (uint256 r) {
        // 48 big-endian bytes = hi (16 bytes) * 2^256 + lo (32 bytes), reduced mod P
        uint256 hi;
        uint256 lo;
        assembly {
            hi := shr(128, mload(add(add(b, 0x20), offset)))
            lo := mload(add(add(b, 0x30), offset))
        }
        uint256 twoTo256 = addmod(mulmod(1 << 255, 2, P), 0, P);
        r = addmod(mulmod(hi % P, twoTo256, P), lo % P, P);
    }

    function _modexp(uint256 base, uint256 e) private view returns (uint256 r) {
        uint256[6] memory input = [uint256(32), 32, 32, base, e, P];
        uint256[1] memory out;
        bool ok;
        assembly {
            ok := staticcall(gas(), 0x05, input, 0xc0, out, 0x20)
        }
        if (!ok) revert ModExpFailed();
        r = out[0];
    }

    function _inv0(uint256 a) private view returns (uint256) {
        return _modexp(a, P - 2);            // 0 maps to 0, as inv0 requires
    }

    function _isSquare(uint256 a) private view returns (bool) {
        uint256 l = _modexp(a, (P - 1) / 2);
        return l == 0 || l == 1;
    }

    function _sqrt(uint256 a) private view returns (uint256) {
        return _modexp(a, (P + 1) / 4);      // P = 3 mod 4
    }
}
