// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {IERC721} from "@openzeppelin/contracts/token/ERC721/IERC721.sol";

/// docs/v7/phase1-nft.md §4.3–4.4, on an Ethereum mainnet fork taken at block 26,055,690, the block BEFORE a real buyer filled
/// Milady #9180's OpenSea listing (tx 0x404e2c74…, 0.9999 ETH incl. OpenSea's 1 %). The canister's own calldata for that listing
/// (`ethnft::tests::the_canisters_own_calldata_for_the_real_listing_is_the_forge_fixture` writes it: its re-encoding, fulfiller conduit
/// zeroed, nothing appended) is sent from the canister's address with exactly the total: the NFT arrives at the canister (custody),
/// and the canister's delivery (`transferFrom(us, redeemer, id)`, no hook) moves it to a redeemer. Tampered copies revert. Nothing is
/// sent to any public chain.
/// Run:  anvil --fork-url https://eth.drpc.org --fork-block-number 26055690 --port 8642
///       forge test --match-path "test/fork/EthNftSeaport.fork.t.sol" --fork-url http://127.0.0.1:8642
contract EthNftSeaportForkTest is Test {
    address constant SEAPORT = 0x0000000000000068F116a894984e2DB1123eB395;
    IERC721 constant MILADY = IERC721(0x5Af0D9827E0c53E4799BB226655A1de152A425a5);
    uint256 constant TOKEN = 9180;

    bool forked;
    address canister;
    address offerer;
    uint256 value;
    bytes fill;

    function setUp() public {
        forked = block.chainid == 1;
        if (!forked) return;
        assertEq(block.number, 26_055_690, "fork at the block before the real fill");
        string memory json = vm.readFile("test/fixtures/ethnft/milady-9180-canister-fill.json");
        fill = vm.parseJsonBytes(json, ".calldata");
        canister = vm.parseJsonAddress(json, ".canister");
        offerer = vm.parseJsonAddress(json, ".offerer");
        value = vm.parseUint(vm.parseJsonString(json, ".valueWei"));
        assertEq(vm.parseJsonUint(json, ".tokenId"), TOKEN);
        assertEq(bytes4(fill), bytes4(0xfb0f3ee1), "fulfillBasicOrder, the canister's own selector");
        assertEq(canister.code.length, 0, "the canister's address is an EOA (a threshold key): no hook runs for it");
        vm.deal(canister, 2 ether);
    }

    function test_theCanistersCalldataBuysTheRealListingIntoCustody_andItsTransferDelivers() public {
        if (!forked) return;
        assertEq(MILADY.ownerOf(TOKEN), offerer, "listed and owned by the offerer at the fork");
        uint256 before = offerer.balance;
        vm.prank(canister);
        (bool ok,) = SEAPORT.call{value: value}(fill);
        assertTrue(ok, "the fill succeeds from the canister's address");
        assertEq(MILADY.ownerOf(TOKEN), canister, "custody: the canister's Ethereum address holds the NFT");
        assertEq(canister.balance, 2 ether - value, "exactly the total left; nothing else was approved or moved");
        assertEq(offerer.balance - before, 0.98990099901 ether, "the offerer was paid its consideration");
        // the delivery the canister signs for a redeemer (ethnft::transfer_from_call)
        address redeemer = makeAddr("redeemer");
        vm.prank(canister);
        (ok,) = address(MILADY).call(abi.encodeWithSelector(0x23b872dd, canister, redeemer, TOKEN));
        assertTrue(ok);
        assertEq(MILADY.ownerOf(TOKEN), redeemer, "delivered");
    }

    function test_tamperedCopiesRevert() public {
        if (!forked) return;
        // one wei less: the offerer's signature no longer covers the order
        bytes memory cheaper = fill;
        _addToWord(cheaper, 2, -1);
        vm.prank(canister);
        (bool ok,) = SEAPORT.call{value: value - 1}(cheaper);
        assertFalse(ok, "a cheaper copy is not the offerer's order");
        // another token id
        bytes memory other = fill;
        _addToWord(other, 6, 1);
        vm.prank(canister);
        (ok,) = SEAPORT.call{value: value}(other);
        assertFalse(ok, "another token is not the listed one");
        // the right order with less ETH than its total
        vm.prank(canister);
        (ok,) = SEAPORT.call{value: value - 1}(fill);
        assertFalse(ok, "underpaid");
        assertEq(MILADY.ownerOf(TOKEN), offerer, "nothing moved");
    }

    /// Word `i` of the BasicOrderParameters tuple (after the selector and the head offset).
    function _addToWord(bytes memory b, uint256 i, int256 delta) internal pure {
        uint256 off = 4 + 32 + 32 * i;
        uint256 v;
        assembly { v := mload(add(add(b, 32), off)) }
        v = delta < 0 ? v - uint256(-delta) : v + uint256(delta);
        assembly { mstore(add(add(b, 32), off), v) }
    }
}
