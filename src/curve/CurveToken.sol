// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

/// @title CurveToken
/// @notice A coin launched on a FrogCurve: its whole fixed supply is minted once, to its curve, at construction. No owner, no
/// mint, no burn, no pause, no fee on transfer.
contract CurveToken is ERC20 {
    constructor(string memory name_, string memory symbol_, uint256 supply_, address curve_) ERC20(name_, symbol_) {
        _mint(curve_, supply_);
    }
}
