// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {MockBeacon} from "../utils/Mocks.sol";
import {NftWrapper} from "../../src/nft/NftWrapper.sol";

/// A wrapper over a collection on Ethereum (chain 1), held by the canister's address there. The canister is a plain address
/// here: what it attests on the other chain is its word, and this contract only keeps the books (docs/v4/nft-wrapper.md §2.4).
abstract contract NftWrapBase is Test {
    MockBeacon beacon;
    NftWrapper w;
    address custodian = makeAddr("custodian (the canister's EVM address)");
    address collection = makeAddr("BAYC on chain 1");
    address curve = makeAddr("curve or desk whose request a mint fills");
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address carol = makeAddr("carol");
    uint64 constant ETHEREUM = 1;
    uint256 constant ONE = 1e18;
    address constant DEAD = 0x000000000000000000000000000000000000dEaD;

    function setUp() public virtual {
        vm.warp(1_750_000_000);
        vm.roll(26_000_000);
        beacon = new MockBeacon();
        w = new NftWrapper(beacon, ETHEREUM, collection, custodian, "Wrapped Bored Ape", "wAPE");
        w.initMintLimit(1_000);
    }

    function _tx(uint256 tokenId) internal pure returns (bytes32) { return keccak256(abi.encode("purchase", tokenId)); }

    /// The canister mints token `tokenId`, bought by `_tx(tokenId)`, to `to`.
    function _mint(uint256 tokenId, address to) internal returns (uint256 itemId) {
        vm.prank(custodian);
        itemId = w.mint(tokenId, _tx(tokenId), to);
    }

    function _redeem(address who) internal returns (uint256 id) {
        vm.prank(who);
        id = w.redeem(who);
    }

    /// Resolve ticket `id` with randomness `r` (the mock beacon's "signature" is the randomness itself).
    function _resolve(uint256 id, bytes32 r) internal {
        if (r == bytes32(0)) r = bytes32(uint256(1));
        w.resolve(id, abi.encodePacked(r));
    }

    function _deliver(uint256 id) internal {
        vm.prank(custodian);
        w.recordDelivery(id, keccak256(abi.encode("delivery", id)));
    }

    function _tokenOf(uint256 ticketId) internal view returns (uint256) {
        return w.item(w.ticket(ticketId).itemId).tokenId;
    }

    /// Each held NFT backs one whole token or one open ticket, never both.
    function _assertLedger() internal view {
        assertEq(w.totalSupply() + w.openTickets() * ONE, w.held() * ONE, "supply + open tickets == held x ONE");
    }
}
