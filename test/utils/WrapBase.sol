// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "./Base.sol";
import {Clock} from "./Clock.sol";
import {Distributor} from "../../src/Distributor.sol";
import {Wrapper} from "../../src/Wrapper.sol";

/// `Base`, plus a wrapper over the inscriptions listing (id 0). A frog reaches it the only way the design allows: bought and
/// receipted by the vault, ruled out of any draw by the creator of the coin that paid (`Mode.Never`), then wrapped by that
/// same creator. Its `ONE_ITEM` lands on `distribution`, which here is a plain address standing in for step 2's claim contract.
abstract contract WrapBase is Base {
    Wrapper wrapper;
    address distribution = makeAddr("distribution (step 2's pro-rata claims)");
    string constant TAPROOT = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";
    bytes32 constant TXID = keccak256("a bitcoin txid");
    uint256 constant ONE = 1e18;

    function setUp() public virtual override {
        super.setUp();
        wrapper = new Wrapper(registry, distributor, beacon, 0, distribution, custodian, CUSTODY_BTC, "Wrapped Test Frog", "wFROG", PBS);
        wrapper.initMintLimit(1_000);
    }

    /// Receipt inscription `ids[i]` into the v1 vault, paid for by `coinA`. Returns the acquisition id.
    function _receipt(uint256 i) internal returns (uint256 aid) {
        _fund(ordVault, coinA, 1 ether);
        vm.prank(keeper); ordVault.postCap(1 ether);
        vm.prank(keeper); uint256 id = ordVault.declarePurchase(1 ether, 1, "cap");
        Clock.skip(1 hours);
        ordVault.release(id);
        vm.prank(attestor);
        aid = ordVault.settleInscription(id, ids[i], TXID, 1, _proofOf(i));
    }

    /// The whole legitimate path: receipt, the creator's final `Never`, the creator's wrap.
    function _frog(uint256 i) internal returns (uint256 aid) {
        aid = _receipt(i);
        vm.prank(creatorA);
        distributor.choose(0, aid, Distributor.Mode.Never, address(0));
        vm.prank(creatorA);
        wrapper.wrap(aid);
    }

    /// Hand `n` whole tokens from the distribution contract to `to`, as a claim would.
    function _give(address to, uint256 n) internal {
        vm.prank(distribution);
        wrapper.transfer(to, n * ONE);
    }

    function _redeem(address who) internal returns (uint256 id) {
        vm.prank(who);
        id = wrapper.redeem(TAPROOT);
    }

    /// Resolve ticket `id` with randomness `r` (the mock beacon's "signature" is the randomness itself).
    function _resolve(uint256 id, bytes32 r) internal {
        if (r == bytes32(0)) r = bytes32(uint256(1));
        wrapper.resolve(id, abi.encodePacked(r));
    }

    /// docs/v3/wrapper.md, the ledger invariant as corrected: each held frog backs one whole token or one open ticket.
    function _assertLedger() internal view {
        assertEq(wrapper.totalSupply(), (wrapper.held() - wrapper.openTickets()) * ONE, "supply == (held - open) x ONE");
    }
}
