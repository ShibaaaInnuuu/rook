// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {HomePolicy} from "./IHomeChain.sol";
import {HomeVaultBase} from "./HomeVaultBase.sol";

/// @title HomeVaultBurn
/// @notice A home-chain vault whose every purchase goes to 0x…dEaD, by anyone's call. docs/v6/home-chain-vault.md §3.3.
contract HomeVaultBurn is HomeVaultBase {
    event Burned(uint256 indexed acquisitionId, uint256 indexed tokenId);

    constructor(Config memory c) HomeVaultBase(c, HomePolicy.Burn) {}

    function burn(uint256 acquisitionId) external nonReentrant {
        Acquisition storage a = _acquisitions[acquisitionId];
        if (a.stage != Stage.Held) revert WrongStage();
        _stage(a, Stage.Burned);
        guard.release(collection, 1, a.pricePaid);
        _send(DEAD, a.tokenId);
        emit Burned(acquisitionId, a.tokenId);
    }
}
