// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// A small Merkle tree for tests, with OpenZeppelin's sorted-pair hashing. An odd node is carried up unchanged.
library Merkle {
    function _pair(bytes32 a, bytes32 b) private pure returns (bytes32) {
        return a < b ? keccak256(abi.encodePacked(a, b)) : keccak256(abi.encodePacked(b, a));
    }

    function root(bytes32[] memory leaves) internal pure returns (bytes32) {
        bytes32[] memory level = leaves;
        while (level.length > 1) level = _up(level);
        return level[0];
    }

    function proof(bytes32[] memory leaves, uint256 index) internal pure returns (bytes32[] memory out) {
        bytes32[] memory level = leaves;
        bytes32[] memory acc = new bytes32[](32);
        uint256 n;
        while (level.length > 1) {
            uint256 sib = index ^ 1;
            if (sib < level.length) acc[n++] = level[sib];
            index >>= 1;
            level = _up(level);
        }
        out = new bytes32[](n);
        for (uint256 i = 0; i < n; ++i) out[i] = acc[i];
    }

    function _up(bytes32[] memory level) private pure returns (bytes32[] memory next) {
        next = new bytes32[]((level.length + 1) / 2);
        for (uint256 i = 0; i < level.length; i += 2) {
            next[i / 2] = i + 1 < level.length ? _pair(level[i], level[i + 1]) : level[i];
        }
    }
}
