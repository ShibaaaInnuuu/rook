// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {WrapBase} from "./WrapBase.sol";
import {Clock} from "./Clock.sol";
import {Merkle} from "./Merkle.sol";
import {Distributor} from "../../src/Distributor.sol";
import {WrapClaim} from "../../src/WrapClaim.sol";

/// `WrapBase`, with step 2 in place: the wrapper is the one `WrapClaim` deployed for itself, so `distribution` is the claim
/// contract and every wrapped frog's `ONE_ITEM` lands there. A frog's holders are published by root and claimed by proof.
abstract contract ClaimBase is WrapBase {
    WrapClaim claim;

    function setUp() public virtual override {
        super.setUp();
        claim = new WrapClaim(registry, distributor, beacon, 0, custodian, CUSTODY_BTC, "Wrapped Test Frog", "wFROG", PBS);
        wrapper = claim.wrapper();
        distribution = address(claim);
    }

    /// Receipt inscription `ids[i]`, paid for by `coin`, ruled out of any draw and wrapped by `creator`.
    function _frogBy(uint256 i, address coin, address creator) internal returns (uint256 aid) {
        aid = _receiptBy(i, coin, creator);
        vm.prank(creator);
        wrapper.wrap(aid);
    }

    /// Receipt inscription `ids[i]`, paid for by `coin`, and have `creator` rule it out of any draw — wrappable, not wrapped.
    function _receiptBy(uint256 i, address coin, address creator) internal returns (uint256 aid) {
        _fund(ordVault, coin, 1 ether);
        vm.prank(keeper); ordVault.postCap(1 ether);
        vm.prank(keeper); uint256 id = ordVault.declarePurchase(1 ether, 1, "cap");
        Clock.skip(1 hours);
        ordVault.release(id);
        bytes32[] memory proof = _proofOf(i);
        string memory inscription = ids[i];
        vm.prank(attestor);
        aid = ordVault.settleInscription(id, inscription, TXID, 1, proof);
        vm.prank(creator);
        distributor.choose(0, aid, Distributor.Mode.Never, address(0));
    }

    /// The leaves of an allocation, in the Distributor's shape.
    function _leaves(address[] memory holders, uint256[] memory amounts) internal view returns (bytes32[] memory l) {
        l = new bytes32[](holders.length);
        for (uint256 i = 0; i < holders.length; ++i) l[i] = claim.leaf(holders[i], amounts[i]);
    }

    /// The keeper publishes `holders`/`amounts` for frog `aid`.
    function _open(uint256 aid, address[] memory holders, uint256[] memory amounts) internal {
        bytes32 root = Merkle.root(_leaves(holders, amounts));
        vm.prank(keeper);
        claim.open(aid, root, "ipfs://the-list");
    }

    function _claim(uint256 aid, address[] memory holders, uint256[] memory amounts, uint256 i) internal {
        bytes32[] memory proof = Merkle.proof(_leaves(holders, amounts), i);
        claim.claim(aid, holders[i], amounts[i], proof);
    }

    function _pair(address a, uint256 x, address b, uint256 y) internal pure returns (address[] memory h, uint256[] memory n) {
        h = new address[](2); h[0] = a; h[1] = b;
        n = new uint256[](2); n[0] = x; n[1] = y;
    }
}
