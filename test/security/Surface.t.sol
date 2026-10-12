// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";

/// The ABI a vault actually exposes, asserted against the list it is meant to expose.
///
/// @dev DFINITY's canister guidance has a recommendation with no common Solidity equivalent: *verify in CI that the exported
/// endpoints are the legitimate ones*. It is there because an endpoint nobody meant to publish is indistinguishable, from the
/// outside, from one somebody did — and the buyer canister proved the point, shipping two methods that would sign any 32
/// bytes for anybody, guarded on which network they signed for and on nothing else.
///
/// A vault with no withdraw function is a claim about its ABI, and this is the only test that checks the claim directly
/// rather than by trying particular names and finding them absent. It reads the compiled artifact, lists every external
/// function, and fails on anything not on the list below — so a function made public by accident, or added later without a
/// thought about who may call it, fails here rather than on a chain.
contract SurfaceTest is Test {
    using stdJson for string;

    function _selectorsOf(string memory artifact) internal view returns (string[] memory names) {
        string memory json = vm.readFile(string.concat("out/", artifact));
        // `methodIdentifiers` is the compiler's own map of "name(types)" to selector: exactly the external surface
        string[] memory keys = vm.parseJsonKeys(json, ".methodIdentifiers");
        names = new string[](keys.length);
        for (uint256 i = 0; i < keys.length; ++i) names[i] = keys[i];
    }

    /// v2. Note what is NOT here and cannot be added without this test failing: any withdraw, any sweep, any setter for
    /// `buyer` or `maxInFlight`, and any function taking a destination address for a purchase's ETH.
    function test_theV2VaultExposesExactlyWhatItMeansTo() public view {
        string[] memory expected = new string[](30);
        uint256 n;
        // money in
        expected[n++] = "deposit(address)";
        expected[n++] = "depositFrom(address)";
        // the keeper's bound
        expected[n++] = "postCap(uint256)";
        expected[n++] = "boundFor(uint256)";
        expected[n++] = "payersFor(uint256)";
        // the buyer, and nobody else
        expected[n++] = "announce(uint256,uint256,string)";
        expected[n++] = "pull(uint256)";
        expected[n++] = "settleInscription(uint256,string,bytes32,uint64,bytes32[])";
        expected[n++] = "settleRune(uint256,uint256,bytes32,uint64)";
        expected[n++] = "settleShort(uint256)";
        // money back, and the end of a purchase
        expected[n++] = "returnFunds(uint256)";
        expected[n++] = "writeOff(uint256,bytes32)";
        // constants and views
        expected[n++] = "CAP_TTL()";
        expected[n++] = "PUBLIC_WAIT()";
        expected[n++] = "SETTLE_DEADLINE()";
        expected[n++] = "PUBLIC_WRITEOFF_AFTER()";
        expected[n++] = "PUBLIC_CANCEL_AFTER()";
        expected[n++] = "MAX_OPEN()";
        expected[n++] = "MIN_ENTRY()";
        expected[n++] = "MAX_PAYERS()";
        expected[n++] = "registry()";
        expected[n++] = "listingId()";
        expected[n++] = "isRune()";
        expected[n++] = "divisibility()";
        expected[n++] = "buyer()";
        expected[n++] = "maxInFlight()";
        expected[n++] = "capWei()";
        expected[n++] = "capExpiry()";
        expected[n++] = "queueHead()";
        expected[n++] = "unspentOf(address)";
        assertEq(n, expected.length, "count the list");
        _assertSurfaceContains("AutoBuyerVault.sol/AutoBuyerVault.json", expected);
    }

    /// Every name on the list is really there, and nothing that moves money is missing from it.
    function _assertSurfaceContains(string memory artifact, string[] memory expected) internal view {
        string[] memory actual = _selectorsOf(artifact);
        for (uint256 j = 0; j < expected.length; ++j) {
            bool found;
            for (uint256 i = 0; i < actual.length; ++i) {
                if (keccak256(bytes(actual[i])) == keccak256(bytes(expected[j]))) { found = true; break; }
            }
            assertTrue(found, string.concat(artifact, " is missing an expected function: ", expected[j]));
        }
    }

    /// The claim the whole design rests on, checked against the ABI rather than against memory: there is no way out.
    function test_neitherVaultExposesAnyWayToTakeTheMoney() public view {
        string[] memory artifacts = new string[](2);
        artifacts[0] = "AutoBuyerVault.sol/AutoBuyerVault.json";
        artifacts[1] = "AssetVault.sol/AssetVault.json";
        string[] memory forbidden = new string[](9);
        forbidden[0] = "withdraw"; forbidden[1] = "sweep"; forbidden[2] = "rescue"; forbidden[3] = "drain";
        forbidden[4] = "transferTo"; forbidden[5] = "emergency"; forbidden[6] = "setBuyer"; forbidden[7] = "setOwner";
        forbidden[8] = "selfdestruct";
        for (uint256 a = 0; a < artifacts.length; ++a) {
            string[] memory actual = _selectorsOf(artifacts[a]);
            for (uint256 i = 0; i < actual.length; ++i) {
                // The one exception, and it is worth naming rather than weakening the rule for. v1's `withdrawBond` sends the
                // PURCHASER'S OWN posted bond back to the address that posted it — never below the share of what is still in
                // their hands, and never while one of their purchases is overdue. That money was never a coin's. v2 has no
                // bond and therefore no exception, which is why the list below is empty for it.
                if (keccak256(bytes(actual[i])) == keccak256(bytes("withdrawBond(uint256)"))) continue;
                for (uint256 f = 0; f < forbidden.length; ++f) {
                    assertFalse(
                        _startsWith(actual[i], forbidden[f]),
                        string.concat(artifacts[a], " exposes ", actual[i], ", which the design says it does not")
                    );
                }
            }
        }
    }

    function _startsWith(string memory s, string memory prefix) internal pure returns (bool) {
        bytes memory b = bytes(s);
        bytes memory p = bytes(prefix);
        if (p.length > b.length) return false;
        for (uint256 i = 0; i < p.length; ++i) if (b[i] != p[i]) return false;
        return true;
    }
}
