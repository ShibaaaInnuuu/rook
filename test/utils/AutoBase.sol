// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "./Base.sol";
import {AutoBuyerVault} from "../../src/AutoBuyerVault.sol";
import {Merkle} from "./Merkle.sol";
import {Clock} from "./Clock.sol";

/// `Base`, plus the two v2 listings: an inscriptions listing (id 2) and a rune listing (id 3, divisibility 2), each with a
/// vault that can release ETH only to `autoBuyer` and never more than `MAX_IN_FLIGHT` at once.
abstract contract AutoBase is Base {
    AutoBuyerVault autoOrd;
    AutoBuyerVault autoRune;

    address payable autoBuyer = payable(makeAddr("the canister's EVM address"));
    uint256 constant MAX_IN_FLIGHT = 20 ether;

    function setUp() public virtual override {
        super.setUp();
        vm.startPrank(owner);
        registry.listInscriptionsAuto("Auto Collection", "auto-collection", Merkle.root(idLeaves), "bc1p-auto-ord",
            autoBuyer, MAX_IN_FLIGHT);
        registry.listRuneAuto("AUTO.RUNE", "840000:2", 2, "bc1p-auto-rune", autoBuyer, MAX_IN_FLIGHT);
        vm.stopPrank();
        autoOrd = AutoBuyerVault(payable(registry.vaultOf(2)));
        autoRune = AutoBuyerVault(payable(registry.vaultOf(3)));
    }

    function _fundAuto(AutoBuyerVault v, address coin, uint256 amount) internal {
        vm.prank(router);
        v.deposit{value: amount}(coin);
    }

    /// Cap, announce, wait the public hour, pull. Returns the purchase id.
    function _pulled(AutoBuyerVault v, uint256 maxEth, uint256 quantity) internal returns (uint256 id) {
        // read the vault BEFORE the prank: an external call in the argument list would consume it
        uint256 cap = v.isRune() ? maxEth * (10 ** uint256(v.divisibility())) / quantity : maxEth;
        vm.prank(keeper);
        v.postCap(cap);
        vm.prank(autoBuyer);
        id = v.announce(maxEth, quantity, "cap-ref");
        Clock.skip(1 hours);
        vm.prank(autoBuyer);
        v.pull(id);
    }
}
