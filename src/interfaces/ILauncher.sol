// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface ILauncher {
    function routerOf(address coin) external view returns (address);
    function creatorOf(address coin) external view returns (address);
    function listingOf(address coin) external view returns (uint256);
}
