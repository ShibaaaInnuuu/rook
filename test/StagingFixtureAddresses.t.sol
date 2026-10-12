// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {BtcAddress} from "../src/lib/BtcAddress.sol";

/// @title StagingFixtureAddressesTest
/// @notice docs/phase11.md §7. The staging simulator posts a winner's Bitcoin address for them, from a fixed list of test
/// vectors (staging/sim/spread.ts, FIXTURE_TAPROOT). `Distributor.setDestination` runs every one of them through
/// `BtcAddress.isTaproot`, so a vector the library refuses is a state the staging chain can never reach — which is exactly
/// what happened with 58 vaults: `bc1pw508d6qejxtdg4y5r3zarvary0c5xw7k…`, BIP-350's FORTY-byte v1 vector, is valid bech32m
/// and is not a taproot output, and every winner it was handed stalled at BadAddress().
/// These are the strings that file holds, pinned here so the two cannot drift apart again.
contract StagingFixtureAddressesTest is Test {
    function test_everyFixtureTaprootIsOneTheDistributorAccepts() public pure {
        string[3] memory fixtures = [
            "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0",
            "bc1p5d7rjq7g6rdk2yhzks9smlaqtedr4dekq08ge8ztwac72sfr9rusxg3297",
            "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr"
        ];
        for (uint256 i = 0; i < fixtures.length; ++i) {
            assertTrue(BtcAddress.isTaproot(fixtures[i]), fixtures[i]);
            assertEq(bytes(fixtures[i]).length, 62, "a taproot address is 62 characters; a longer v1 program is not one");
        }
    }

    function test_theVectorThatWasRemovedIsStillRefused() public pure {
        // BIP-350's 40-byte version-1 vector: valid bech32m, not a taproot output. It cost thirty stalled deliveries.
        assertFalse(BtcAddress.isTaproot("bc1pw508d6qejxtdg4y5r3zarvary0c5xw7kw508d6qejxtdg4y5r3zarvary0c5xw7kt5nd6y"));
    }
}
