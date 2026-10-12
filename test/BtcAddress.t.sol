// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {BtcAddress} from "../src/lib/BtcAddress.sol";

contract BtcAddressTest is Test {
    // BIP-350's own valid mainnet taproot test vector
    string constant VALID = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";

    function test_bip350Vector() public pure {
        assertTrue(BtcAddress.isTaproot(VALID));
    }

    function test_oneWrongCharacterFailsTheChecksum() public pure {
        assertFalse(BtcAddress.isTaproot("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj1"));
        assertFalse(BtcAddress.isTaproot("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jjq"));
    }

    function test_otherShapesAreRefused() public pure {
        assertFalse(BtcAddress.isTaproot("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"));                          // segwit v0
        assertFalse(BtcAddress.isTaproot("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"));                                  // legacy
        assertFalse(BtcAddress.isTaproot("tb1pqqqqp399et2xygdj5xreqhjjvcmzhxw4aywxecjdzew6hylgvsesf3hn0c"));      // testnet
        assertFalse(BtcAddress.isTaproot("BC1P0XLXVLHEMJA6C4DQV22UAPCTQUPFHLXM9H8Z3K2E72Q4K9HCZ7VQZK5JJ0"));      // upper case
        assertFalse(BtcAddress.isTaproot(""));
        assertFalse(BtcAddress.isTaproot("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj"));       // one short
    }

    function testFuzz_aFlippedCharacterNeverPasses(uint8 pos, uint8 to) public pure {
        bytes memory a = bytes(VALID);
        uint256 i = 4 + (uint256(pos) % 58);
        bytes memory cs = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
        bytes1 c = cs[to % 32];
        vm.assume(c != a[i]);
        a[i] = c;
        assertFalse(BtcAddress.isTaproot(string(a)));
    }
}
