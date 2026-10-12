// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title BtcAddress
/// @notice Accepts exactly one shape of Bitcoin address: a mainnet taproot address (`bc1p…`, 62 characters, witness version 1),
/// with its bech32m checksum verified. Inscriptions and runes are conventionally held at taproot addresses, and a mistyped
/// address is an asset lost for ever, so the checksum is worth its gas.
/// @dev What this does NOT check: that anybody holds the key, that the wallet behind it understands ordinals or runes (a wallet
/// that does not may spend the asset as ordinary bitcoin), or anything about testnet, legacy or segwit-v0 addresses, which are
/// simply refused.
library BtcAddress {
    // value of each ASCII character in the bech32 alphabet "qpzry9x8gf2tvdw0s3jn54khce6mua7l"; 0xff: not in it (upper case included)
    bytes private constant REVERSE = hex"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff0fff0a1115141a1e0705ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff1dff180d19090817ff12161f1b13ff010003100b1c0c0e060402ffffffffff";
    uint256 private constant BECH32M = 0x2bc830a3;

    function isTaproot(string memory addr) internal pure returns (bool) {
        bytes memory a = bytes(addr);
        if (a.length != 62) return false;
        if (a[0] != "b" || a[1] != "c" || a[2] != "1" || a[3] != "p") return false;
        // polymod over: hrp expanded ("bc" -> 3,3,0,2,3) then the 59 data characters after the separator
        uint256 chk = 1;
        chk = _step(chk, 3);
        chk = _step(chk, 3);
        chk = _step(chk, 0);
        chk = _step(chk, 2);
        chk = _step(chk, 3);
        for (uint256 i = 3; i < 62; ++i) {
            uint256 v = _value(a[i]);
            if (v == 255) return false;
            chk = _step(chk, v);
        }
        // docs/phase3.md L4: 52 characters carry 260 bits for a 256-bit key; BIP-173 requires the four left over to be zero, and a
        // wallet that follows it refuses an address where they are not. The last program character is a[55].
        if (_value(a[55]) & 0x0f != 0) return false;
        return chk == BECH32M;
    }

    function _step(uint256 chk, uint256 v) private pure returns (uint256) {
        uint256 b = chk >> 25;
        chk = ((chk & 0x1ffffff) << 5) ^ v;
        if (b & 1 != 0) chk ^= 0x3b6a57b2;
        if (b & 2 != 0) chk ^= 0x26508e6d;
        if (b & 4 != 0) chk ^= 0x1ea119fa;
        if (b & 8 != 0) chk ^= 0x3d4233dd;
        if (b & 16 != 0) chk ^= 0x2a1462b3;
        return chk;
    }

    function _value(bytes1 c) private pure returns (uint256) {
        uint8 x = uint8(c);
        return x < 128 ? uint8(REVERSE[x]) : 255;
    }
}
