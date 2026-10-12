// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {HomeBase} from "./HomeBase.sol";
import {Clock} from "../utils/Clock.sol";
import {HomeWrapper} from "../../src/homechain/HomeWrapper.sol";
import {HomeGuard} from "../../src/homechain/HomeGuard.sol";
import {IHomeGuard} from "../../src/homechain/IHomeChain.sol";
import {IWrappedCollection} from "../../src/interfaces/IWrappedCollection.sol";

/// docs/v6/home-chain-vault.md §4: custody by the contract, a wrap by anyone who owns an NFT, a random redemption delivered in the
/// resolving call, and the ledger that is now checkable on the same chain.
contract HomeWrapperTest is HomeBase {
    uint256 constant ONE = 1e18;

    function _wrap(address who, uint256 id) internal {
        nft.mint(who, id);
        vm.startPrank(who);
        nft.approve(address(wNft), id);
        wNft.wrap(id, who);
        vm.stopPrank();
    }

    function _wrapPunk(address who, uint256 id) internal {
        punks.mint(who, id);
        vm.startPrank(who);
        punks.offerPunkForSaleToAddress(id, 0, address(wPunk));
        wPunk.wrap(id, who);
        vm.stopPrank();
    }

    function _resolve(HomeWrapper w, uint256 id, bytes32 r) internal {
        w.resolve(id, abi.encodePacked(r == bytes32(0) ? bytes32(uint256(1)) : r));
    }

    function test_constructorAndInterface() public {
        assertEq(wNft.custodian(), address(wNft));
        assertEq(wNft.ONE_ITEM(), ONE);
        IWrappedCollection i = IWrappedCollection(address(wNft));
        assertEq(i.held(), 0);
        vm.expectRevert(HomeWrapper.BadInput.selector);
        new HomeWrapper(beacon, makeAddr("no code"), false, IHomeGuard(address(guard)), address(0), 12, "x", "x");
        vm.expectRevert();
        new HomeWrapper(beacon, address(nft), false, IHomeGuard(address(guard)), address(0), 0, "x", "x");
    }

    function test_wrapMovesTheNftHereAndMints() public {
        _wrap(alice, 1);
        assertEq(nft.ownerOf(1), address(wNft));
        assertEq(wNft.balanceOf(alice), ONE);
        assertEq(wNft.held(), 1);
        assertEq(wNft.itemOf(1), 1);
        assertEq(wNft.pendingCount(), 1);
        HomeWrapper.Item memory it = wNft.item(0);
        assertEq(it.tokenId, 1);
        assertEq(it.from, alice);
        HomeGuard.Cap memory c = guard.capOf(address(nft));
        assertEq(c.items, 1);
        assertEq(c.valueWei, 0);
    }

    function test_wrapRefusals() public {
        nft.mint(alice, 1);
        vm.prank(alice);
        vm.expectRevert();                        // not approved: transferFrom reverts
        wNft.wrap(1, alice);
        vm.prank(alice);
        nft.approve(address(wNft), 1);
        vm.prank(bob);
        vm.expectRevert();                        // bob does not own it
        wNft.wrap(1, bob);
        vm.prank(alice);
        vm.expectRevert(HomeWrapper.BadInput.selector);
        wNft.wrap(1, address(0));
        vm.prank(alice);
        wNft.wrap(1, alice);
        vm.prank(alice);
        vm.expectRevert(HomeWrapper.WrongStage.selector);   // already held
        wNft.wrap(1, alice);
    }

    function test_wrapPunk_throughAZeroOfferToTheWrapper() public {
        punks.mint(alice, 5);
        vm.prank(alice);
        vm.expectRevert(HomeWrapper.NotArrived.selector);   // no offer
        wPunk.wrap(5, alice);
        vm.prank(alice);
        punks.offerPunkForSale(5, 0);                          // to anyone: refused (someone else could take it first)
        vm.prank(alice);
        vm.expectRevert(HomeWrapper.NotArrived.selector);
        wPunk.wrap(5, alice);
        vm.prank(alice);
        punks.offerPunkForSaleToAddress(5, 1, address(wPunk)); // a price: refused
        vm.prank(alice);
        vm.expectRevert(HomeWrapper.NotArrived.selector);
        wPunk.wrap(5, alice);
        vm.startPrank(alice);
        punks.offerPunkForSaleToAddress(5, 0, address(wPunk));
        wPunk.wrap(5, alice);
        vm.stopPrank();
        assertEq(punks.punkIndexToAddress(5), address(wPunk));
        assertEq(wPunk.balanceOf(alice), ONE);
    }

    function test_guardCapBindsWraps() public {
        vm.prank(timelock);
        guard.raise(address(nft), 2, 40 ether, 10 ether);     // lowering is instant
        _wrap(alice, 1);
        _wrap(alice, 2);
        nft.mint(alice, 3);
        vm.startPrank(alice);
        nft.approve(address(wNft), 3);
        vm.expectRevert(abi.encodeWithSelector(HomeGuard.OverCap.selector, "collection items", 3, 2));
        wNft.wrap(3, alice);
        vm.stopPrank();
        assertEq(nft.ownerOf(3), alice, "the NFT stays with its owner");
    }

    function test_redeemDeliversInTheResolvingCall() public {
        _wrap(alice, 1);
        _wrap(alice, 2);
        vm.prank(alice);
        uint256 t = wNft.redeem(bob);
        assertEq(wNft.balanceOf(alice), ONE);
        assertEq(wNft.openTickets(), 1);
        HomeWrapper.Ticket memory tk = wNft.ticket(t);
        assertEq(uint8(tk.stage), uint8(HomeWrapper.Stage.Pinned));
        assertEq(tk.destination, bob);
        assertEq(tk.round, beacon.roundAt(block.timestamp) + 20);
        Clock.skip(2 minutes);
        _resolve(wNft, t, keccak256("r"));
        tk = wNft.ticket(t);
        assertEq(uint8(tk.stage), uint8(HomeWrapper.Stage.Delivered), "delivered in the same call");
        uint256 tokenId = wNft.item(tk.itemId).tokenId;
        assertEq(nft.ownerOf(tokenId), bob);
        assertEq(wNft.held(), 1);
        assertEq(wNft.openTickets(), 0);
        assertEq(wNft.totalSupply(), ONE);
        assertEq(guard.capOf(address(nft)).items, 1);
    }

    function test_redeemToBurn() public {
        _wrap(alice, 1);
        vm.prank(alice);
        uint256 t = wNft.redeemToBurn();
        Clock.skip(2 minutes);
        _resolve(wNft, t, keccak256("r"));
        assertEq(nft.ownerOf(1), wNft.DEAD());
        assertEq(wNft.burned(), 1);
    }

    function test_redeemPunkDeliversByTransferPunk() public {
        _wrapPunk(alice, 5);
        vm.prank(alice);
        uint256 t = wPunk.redeem(carol);
        Clock.skip(2 minutes);
        _resolve(wPunk, t, keccak256("r"));
        assertEq(punks.punkIndexToAddress(5), carol);
    }

    function test_redeemRefusals() public {
        _wrap(alice, 1);
        vm.prank(alice);
        vm.expectRevert(HomeWrapper.BadDestination.selector);
        wNft.redeem(address(0));
        vm.prank(alice);
        vm.expectRevert(HomeWrapper.BadDestination.selector);
        wNft.redeem(address(wNft));
        vm.prank(bob);
        vm.expectRevert();                                      // no token
        wNft.redeem(bob);
    }

    function test_ticketsResolveStrictlyInOrder_andTheDrawIsTheSameWhoeverResolves() public {
        _wrap(alice, 1);
        _wrap(alice, 2);
        _wrap(alice, 3);
        vm.startPrank(alice);
        uint256 t0 = wNft.redeem(alice);
        uint256 t1 = wNft.redeem(alice);
        vm.stopPrank();
        Clock.skip(2 minutes);
        vm.expectRevert(abi.encodeWithSelector(HomeWrapper.OutOfOrder.selector, t0));
        wNft.resolve(t1, abi.encodePacked(keccak256("r")));
        uint256 expectedIndex = uint256(keccak256(abi.encode(keccak256("r"), address(wNft), t0))) % 3;
        uint256 expectedItem = expectedIndex;     // items 0,1,2 are promoted into the free set in order
        vm.prank(stranger);
        _resolve(wNft, t0, keccak256("r"));
        assertEq(wNft.ticket(t0).itemId, expectedItem);
        vm.prank(bob);
        _resolve(wNft, t1, keccak256("r"));
        assertEq(uint8(wNft.ticket(t1).stage), uint8(HomeWrapper.Stage.Delivered));
    }

    /// N1 of docs/v4/nft-wrapper.md: an item wrapped after the ticket was burned is not drawable by that ticket.
    function test_aWrapAfterTheBurnCannotBeDrawnByThatTicket() public {
        _wrap(alice, 1);
        vm.prank(alice);
        uint256 t = wNft.redeem(alice);
        _wrap(bob, 2);                            // stamped with tickets.length = 1 > t = 0
        Clock.skip(2 minutes);
        _resolve(wNft, t, keccak256("r"));
        assertEq(wNft.item(wNft.ticket(t).itemId).tokenId, 1, "only item 0 was drawable");
        assertEq(wNft.pendingCount(), 1);
    }

    function test_redrawAfterADayOnBothClocks() public {
        _wrap(alice, 1);
        vm.prank(alice);
        uint256 t = wNft.redeem(alice);
        uint64 r1 = wNft.ticket(t).round;
        vm.expectRevert(HomeWrapper.TooEarly.selector);
        wNft.redraw(t);
        Clock.skipTimeOnly(24 hours);
        vm.expectRevert(HomeWrapper.TooEarly.selector);
        wNft.redraw(t);
        vm.roll(block.number + 24 hours / 12 + 1);
        wNft.redraw(t);
        assertGt(wNft.ticket(t).round, r1);
    }

    /// A collection that refuses the destination: the ticket stays Resolved; `deliver` retries; `refund` after the deadline.
    function test_refusedDeliveryThenRefund() public {
        _wrapPunk(alice, 5);
        punks.setBlocked(bob);
        vm.prank(alice);
        uint256 t = wPunk.redeem(bob);
        Clock.skip(2 minutes);
        _resolve(wPunk, t, keccak256("r"));
        HomeWrapper.Ticket memory tk = wPunk.ticket(t);
        assertEq(uint8(tk.stage), uint8(HomeWrapper.Stage.Resolved));
        assertEq(punks.punkIndexToAddress(5), address(wPunk));
        assertEq(wPunk.held(), 1);
        assertEq(wPunk.openTickets(), 1);
        vm.expectRevert(HomeWrapper.DeliveryFailed.selector);
        wPunk.deliver(t);
        vm.expectRevert(HomeWrapper.TooEarly.selector);
        wPunk.refund(t);
        // unblocked: deliver works
        punks.setBlocked(address(0));
        wPunk.deliver(t);
        assertEq(punks.punkIndexToAddress(5), bob);
        assertEq(uint8(wPunk.ticket(t).stage), uint8(HomeWrapper.Stage.Delivered));
    }

    function test_refundAfterSevenDaysOnBothClocks() public {
        _wrapPunk(alice, 5);
        punks.setBlocked(bob);
        vm.prank(alice);
        uint256 t = wPunk.redeem(bob);
        Clock.skip(2 minutes);
        _resolve(wPunk, t, keccak256("r"));
        Clock.skipTimeOnly(7 days);
        vm.expectRevert(HomeWrapper.TooEarly.selector);
        wPunk.refund(t);
        vm.roll(block.number + 7 days / 12 + 1);
        wPunk.refund(t);
        assertEq(uint8(wPunk.ticket(t).stage), uint8(HomeWrapper.Stage.Refunded));
        assertTrue(wPunk.refunded(t));
        assertEq(wPunk.balanceOf(alice), 1e18);
        assertEq(wPunk.openTickets(), 0);
        assertEq(wPunk.pendingCount(), 1, "the item is drawable again, by a later ticket");
        vm.expectRevert(HomeWrapper.WrongStage.selector);
        wPunk.refund(t);
    }

    function testFuzz_ledgerHolds(uint8 wraps, uint8 redeems) public {
        uint256 w = bound(wraps, 1, 8);
        uint256 r = bound(redeems, 0, w);
        Clock.skip(3 days + 1);                    // setUp raised the limits once; the interval must pass
        vm.prank(timelock);
        guard.setLimits(24, 48, 40 ether);
        for (uint256 i = 1; i <= w; ++i) _wrap(alice, i);
        for (uint256 i = 0; i < r; ++i) { vm.prank(alice); wNft.redeem(bob); }
        Clock.skip(2 minutes);
        for (uint256 i = 0; i < r; ++i) _resolve(wNft, i, keccak256(abi.encode(i)));
        assertEq(wNft.totalSupply() + wNft.openTickets() * ONE, wNft.held() * ONE);
        assertEq(wNft.held(), w - r);
        assertEq(nft.balanceOf(address(wNft)), w - r, "backing: the wrapper owns exactly what it counts");
        assertEq(nft.balanceOf(bob), r);
    }
}
