// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {NftBase} from "./NftBase.sol";
import {ItemType, ConsiderationItem, AdvancedOrder} from "../../src/nft/Seaport16.sol";
import {NftSweepVault} from "../../src/nft/NftSweepVault.sol";
import {NftRemoteVault} from "../../src/nft/NftRemoteVault.sol";
import {Clock} from "../utils/Clock.sol";

contract NftFuzzTest is NftBase {
    /// Any price, any count, any cap, any tip: a sweep either stands, with every order at most the cap, no tip paid, the price split
    /// exactly between the NFTs and exactly charged to the coins, or it does not happen at all.
    function testFuzz_sweepStandsOnlyWithinTheCap(uint96 fundA, uint96 fundB, uint64 capWei, uint64 price, uint8 count, uint64 tip)
        public
    {
        fundA = uint96(bound(fundA, 0.01 ether, 20 ether));
        fundB = uint96(bound(fundB, 0.01 ether, 20 ether));
        capWei = uint64(bound(capWei, 1 gwei, 5 ether));
        price = uint64(bound(price, 1, 5 ether));
        count = uint8(bound(count, 1, 8));
        tip = uint64(bound(tip, 0, 1 ether));
        _fund(raffleVault, coinA, fundA);
        _fund(raffleVault, coinB, fundB);
        _cap(raffleVault, capWei);
        uint256[] memory ids = new uint256[](count);
        for (uint256 i = 0; i < count; ++i) ids[i] = 500 + i;
        _ask(ids, price);
        AdvancedOrder[] memory orders = new AdvancedOrder[](count);
        for (uint256 i = 0; i < count; ++i) orders[i] = _order(seller, ids[i], price);
        if (tip != 0) {   // the sweeper appends a tip to a friend on the last order, past the signed count
            ConsiderationItem[] memory c = new ConsiderationItem[](2);
            c[0] = orders[count - 1].parameters.consideration[0];
            c[1] = ConsiderationItem(ItemType.NATIVE, address(0), 0, tip, tip, payable(stranger));
            orders[count - 1].parameters.consideration = c;
        }
        bytes memory data = _available(orders, address(raffleVault));
        uint256 before = address(raffleVault).balance;
        vm.prank(sweeper);
        try raffleVault.sweep(address(market), data, ids, count) returns (uint256 bought, uint256 spent) {
            assertEq(tip, 0, "a tip never goes through");
            assertEq(stranger.balance, 0);
            assertEq(bought, count);
            assertEq(spent, uint256(price) * count);
            assertLe(price, capWei);
            assertEq(address(raffleVault).balance, before - spent);
            uint256 sum;
            for (uint256 i = 0; i < bought; ++i) sum += raffleVault.acquisition(i).pricePaid;
            assertEq(sum, spent);
            assertEq(raffleVault.chargedTo(coinA) + raffleVault.chargedTo(coinB), spent);
        } catch {
            assertEq(address(raffleVault).balance, before);
            assertEq(raffleVault.acquisitionCount(), 0);
            // it failed for a reason: a tip, an order over the cap, or more than was spendable
            assertTrue(tip != 0 || price > capWei || uint256(price) * count > uint256(fundA) + fundB);
        }
    }

    /// Returning a remote purchase's ETH in any instalments never gives a coin back more than it was charged, and puts every wei back.
    function testFuzz_remoteGiveBackIsExactAndClamped(uint96 a, uint96 b, uint96 first, uint96 second) public {
        address coinD2 = makeAddr("coinD2");
        launcher.set(coinD2, router, makeAddr("creatorD2"), 2);
        a = uint96(bound(a, 0.01 ether, 2 ether));
        b = uint96(bound(b, 0.01 ether, 2 ether));
        _fund(remoteRaffle, coinD, a);
        _fund(remoteRaffle, coinD2, b);
        uint256 maxEth = uint256(a) + b;
        if (maxEth > 5 ether) maxEth = 5 ether;
        _cap(remoteRaffle, maxEth);
        vm.prank(canister);
        uint256 id = remoteRaffle.announce(maxEth, 1, "r");
        Clock.skip(1 hours);
        vm.prank(canister);
        remoteRaffle.pull(id);
        first = uint96(bound(first, 1, maxEth));
        second = uint96(bound(second, 0, maxEth - first));
        vm.prank(canister);
        remoteRaffle.returnFunds{value: first}(id);
        if (second != 0) {
            vm.prank(canister);
            remoteRaffle.returnFunds{value: second}(id);
        }
        uint256 back = uint256(first) + second;
        assertEq(remoteRaffle.chargedTo(coinD) + remoteRaffle.chargedTo(coinD2), maxEth - back);
        assertEq(remoteRaffle.attributed() + remoteRaffle.waiting(), uint256(a) + b - maxEth + back);
        assertEq(address(remoteRaffle).balance, uint256(a) + b - maxEth + back);
    }

    /// Stale on either clock means no sweep, whatever the timing.
    function testFuzz_capFreshnessIsBothClocks(uint32 secs, uint32 blocks) public {
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
        secs = uint32(bound(secs, 0, 3 hours));
        blocks = uint32(bound(blocks, 0, 900));
        vm.warp(block.timestamp + secs);
        vm.roll(block.number + blocks);
        bool fresh = secs <= 1 hours && blocks <= 300;
        assertEq(raffleVault.capFresh(), fresh);
    }
}
