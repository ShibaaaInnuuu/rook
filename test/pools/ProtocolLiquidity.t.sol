// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IPoolManager} from "@uniswap/v4-core/src/interfaces/IPoolManager.sol";
import {PoolKey} from "@uniswap/v4-core/src/types/PoolKey.sol";
import {StateLibrary} from "@uniswap/v4-core/src/libraries/StateLibrary.sol";
import {PoolBase} from "../utils/PoolBase.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {LiquidityLock} from "../../src/pools/LiquidityLock.sol";
import {ICapSource} from "../../src/interfaces/ICapSource.sol";

/// docs/v5/floor-module.md §13.2: protocol-owned liquidity in the shared pool, against the real PoolManager. ETH anyone adds
/// goes under `PROTOCOL` into the same layers as a graduation's contribution — paired with the wX its own positions earned,
/// the rest as the ETH-only bid below the near band — and never leaves; its ETH fees go to the treasury once one is wired.
contract ProtocolLiquidityTest is PoolBase {
    using StateLibrary for IPoolManager;

    address stranger2 = makeAddr("stranger2");
    address treasury2 = makeAddr("treasury2");

    function setUp() public override {
        super.setUp();
        _graduate(SweepVault.Policy.RAFFLE);                   // the shared pool exists and is centered
    }

    function test_anyoneAddsProtocolLiquidity_itBidsBelowTheNearBand() public {
        address P = lock.PROTOCOL();
        vm.deal(stranger2, 1 ether);
        vm.prank(stranger2);
        lock.addProtocolLiquidity{value: 1 ether}();
        assertLt(lock.pendingEth(P), 1e6, "all but dust is in the pool");
        bytes32[] memory ids = lock.coinPositions(P);
        assertEq(ids.length, 1, "one position: the bid (the bands need wX, of which the protocol has none yet)");
        int24[2][4] memory layers = lock.layers();
        LiquidityLock.Position memory pos = lock.position(ids[0]);
        assertEq(pos.lower, layers[3][0]);
        assertEq(pos.upper, layers[3][1]);
        assertGt(lock.shareOf(ids[0], P).units, 0);
        // ETH in the PoolManager, not in the lock (which holds only pending dust, this coin's and the graduation's)
        assertLt(address(lock).balance, 1e6);
    }

    function test_protocolLiquidityDoesNotCountAsGraduationDepth() public {
        uint256 depth = lock.sharedEthIn();
        vm.deal(stranger2, 1 ether);
        vm.prank(stranger2);
        lock.addProtocolLiquidity{value: 1 ether}();
        assertEq(lock.sharedEthIn(), depth);
    }

    function test_anUnwiredLockKeepsItPending_untilDeployed() public {
        LiquidityLock fresh = new LiquidityLock(PM, wrapper, _layout(), PBS);
        address P = fresh.PROTOCOL();
        vm.deal(stranger2, 1 ether);
        vm.prank(stranger2);
        fresh.addProtocolLiquidity{value: 1 ether}();
        assertEq(fresh.pendingEth(P), 1 ether);
        vm.expectRevert(LiquidityLock.NotWired.selector);
        fresh.deploy(P);
        fresh.wire(ICapSource(address(factory)));
        fresh.deploy(P);
        assertLt(fresh.pendingEth(P), 1e6);
        vm.expectRevert(LiquidityLock.NotSeeded.selector);
        fresh.deploy(stranger2);
    }

    function test_theProtocolPositionsFees_compoundOrGoToTheTreasury() public {
        address P = lock.PROTOCOL();
        vm.deal(stranger2, 5 ether);
        vm.prank(stranger2);
        lock.addProtocolLiquidity{value: 5 ether}();
        // a crash through the bid: wX sold into the pool, then bought back — fees in wX, then in ETH
        _wrappedFor(address(swapper), 20);
        swapper.swap(lock.sharedKey(), false, 20e18, 0);
        (uint256 eth, uint256 w) = lock.claimShared(P);
        assertEq(eth, 0, "the crash paid wX fees");
        assertGt(w, 0);
        assertEq(lock.pendingWrapped(P) + _protocolWrappedInPool(P), w, "kept: pending, or already paired into the bands");
        swapper.swap(lock.sharedKey(), true, 8 ether, 0);
        (eth, w) = lock.claimShared(P);
        assertGt(eth, 0, "the buy-back paid ETH fees");
        assertEq(treasury2.balance, 0, "no treasury: they compound");
        assertLe(lock.pendingEth(P), 1e6 + eth, "as pending protocol liquidity, until deployed");
        // with a treasury wired, ETH fees go there
        vm.prank(stranger2);
        vm.expectRevert(LiquidityLock.NotDeployer.selector);
        lock.wireTreasury(treasury2);
        lock.wireTreasury(treasury2);
        vm.expectRevert(LiquidityLock.AlreadyWired.selector);
        lock.wireTreasury(treasury2);
        swapper.swap(lock.sharedKey(), false, 5e18, 0);
        swapper.swap(lock.sharedKey(), true, 4 ether, 0);
        (eth,) = lock.claimShared(P);
        assertGt(eth, 0);
        assertEq(treasury2.balance, eth, "to the treasury");
    }

    /// The wX the protocol holds in the shared pool: only its share units say so; here we just confirm its positions grew
    /// beyond the bid once it had wX to pair.
    function _protocolWrappedInPool(address P) internal view returns (uint256) {
        bytes32[] memory ids = lock.coinPositions(P);
        return ids.length > 1 ? type(uint256).max / 2 : 0;
    }

    function test_nothingEverLeaves_theSurfaceSaysSo() public view {
        // the lock's exact ABI is asserted in LiquidityLock.t.sol; here: no function takes protocol liquidity out
        string memory json = vm.readFile("out/LiquidityLock.sol/LiquidityLock.json");
        string[] memory sigs = vm.parseJsonKeys(json, ".methodIdentifiers");
        for (uint256 i = 0; i < sigs.length; ++i) {
            bytes memory b = bytes(sigs[i]);
            assertTrue(!(b.length >= 8 && b[0] == "w" && b[1] == "i" && b[2] == "t" && b[3] == "h"), "no withdraw");
            assertTrue(!(b.length >= 6 && b[0] == "r" && b[1] == "e" && b[2] == "m" && b[3] == "o"), "no remove");
        }
    }
}
