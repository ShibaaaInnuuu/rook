// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {PoolKey} from "@uniswap/v4-core/src/types/PoolKey.sol";
import {Currency} from "@uniswap/v4-core/src/types/Currency.sol";
import {IHooks} from "@uniswap/v4-core/src/interfaces/IHooks.sol";
import {PoolId, PoolIdLibrary} from "@uniswap/v4-core/src/types/PoolId.sol";
import {StateLibrary} from "@uniswap/v4-core/src/libraries/StateLibrary.sol";
import {LockSpotQuote} from "../../src/homechain/LockSpotQuote.sol";
import {LiquidityLock} from "../../src/pools/LiquidityLock.sol";

/// A PoolManager that answers `extsload` for slot0 of the pools a test sets, as StateLibrary reads it.
contract MockPoolManagerSlot0 {
    using PoolIdLibrary for PoolKey;
    mapping(bytes32 => bytes32) public slots;
    function setSqrtPrice(PoolKey memory key, uint160 sqrtPriceX96) external {
        bytes32 slot = keccak256(abi.encodePacked(PoolId.unwrap(key.toId()), StateLibrary.POOLS_SLOT));
        slots[slot] = bytes32(uint256(sqrtPriceX96));
    }
    function extsload(bytes32 slot) external view returns (bytes32) { return slots[slot]; }
}

/// The lock's reads that the quote needs, and nothing else.
contract MockLockKeys {
    MockPoolManagerSlot0 public poolManager;
    address public wrapped;
    PoolKey internal shared;
    mapping(address => PoolKey) internal keys;
    mapping(address => bool) internal quoteIsEthOf;
    mapping(address => address) internal curveOf;
    constructor(MockPoolManagerSlot0 pm, address wx) { poolManager = pm; wrapped = wx; }
    function setShared(PoolKey memory k) external { shared = k; }
    function setCoin(address coin, PoolKey memory k, bool quoteIsEth) external { keys[coin] = k; quoteIsEthOf[coin] = quoteIsEth; curveOf[coin] = address(1); }
    function coins(address coin) external view returns (address, bool, uint128) { return (curveOf[coin], quoteIsEthOf[coin], 0); }
    function coinKey(address coin) external view returns (PoolKey memory) { return keys[coin]; }
    function sharedKey() external view returns (PoolKey memory) { return shared; }
}

/// docs/v6/home-chain-vault.md §5.3: the spot-derived bound for a buyback's `minOut`, on the pools' sqrt prices.
contract LockSpotQuoteTest is Test {
    MockPoolManagerSlot0 pm;
    MockLockKeys lock;
    LockSpotQuote q;
    address wx = address(0x1000);
    address coinEth = address(0x2000);
    address coinWx = address(0x500);      // sorts before wx: currency0 = coin in its pool

    function setUp() public {
        pm = new MockPoolManagerSlot0();
        lock = new MockLockKeys(pm, wx);
        q = new LockSpotQuote(LiquidityLock(payable(address(lock))));
    }

    function _key(address a, address b) internal pure returns (PoolKey memory) {
        (address c0, address c1) = a < b ? (a, b) : (b, a);
        return PoolKey(Currency.wrap(c0), Currency.wrap(c1), 3000, 60, IHooks(address(0)));
    }

    /// ETH is currency0 (address 0 sorts first): at sqrtP = 2^96 × 10, one ETH buys 100 coin.
    function test_ethQuotedCoin() public {
        PoolKey memory k = _key(address(0), coinEth);
        lock.setCoin(coinEth, k, true);
        pm.setSqrtPrice(k, uint160(10 * 2 ** 96));
        assertEq(q.coinOutForEth(coinEth, 1 ether), 100 ether);
    }

    /// Two hops: ETH → wX in the shared pool (ETH currency0, sqrtP = 2^96 / 2: 1 ETH = 0.25 wX), then wX → coin where the coin
    /// sorts FIRST (currency0 = coin, currency1 = wX, sqrtP = 2^96 / 4: 1 coin = 1/16 wX, so 0.25 wX = 4 coin).
    function test_wxQuotedCoinComposesBothPools() public {
        PoolKey memory shared = _key(address(0), wx);
        lock.setShared(shared);
        pm.setSqrtPrice(shared, uint160(2 ** 96 / 2));
        PoolKey memory k = _key(coinWx, wx);
        assertEq(Currency.unwrap(k.currency0), coinWx);
        lock.setCoin(coinWx, k, false);
        pm.setSqrtPrice(k, uint160(2 ** 96 / 4));
        assertEq(q.coinOutForEth(coinWx, 1 ether), 4 ether);
    }

    function test_refusesAnUnknownCoinAndAnEmptyPool() public {
        vm.expectRevert(LockSpotQuote.NotAPool.selector);
        q.coinOutForEth(address(0x4000), 1 ether);
        PoolKey memory k = _key(address(0), coinEth);
        lock.setCoin(coinEth, k, true);
        vm.expectRevert(LockSpotQuote.NotAPool.selector);
        q.coinOutForEth(coinEth, 1 ether);
    }

    function testFuzz_linearInEthIn(uint96 ethIn, uint64 p) public {
        uint256 price = bound(p, 1, 1_000_000);
        PoolKey memory k = _key(address(0), coinEth);
        lock.setCoin(coinEth, k, true);
        pm.setSqrtPrice(k, uint160(price * 2 ** 96));
        uint256 x = bound(ethIn, 1, 100 ether);
        assertEq(q.coinOutForEth(coinEth, x), x * price * price);
    }
}
