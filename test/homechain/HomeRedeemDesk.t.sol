// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {HomeBase} from "./HomeBase.sol";
import {HomeRedeemDesk} from "../../src/homechain/HomeRedeemDesk.sol";
import {HomeWrapper} from "../../src/homechain/HomeWrapper.sol";
import {MockInbox} from "./HomeMocks.sol";
import {IArbInbox, IArbOutbox, IHomeAdapterAck} from "../../src/homechain/IHomeChain.sol";
import {IWrappedCollection} from "../../src/interfaces/IWrappedCollection.sol";

/// docs/v6/home-chain-vault.md §10.5: the home chain's side of a 4663 redeem intent. An intent only from the Outbox with the
/// adapter as its L2 sender; anyone's acknowledgement is a retryable to the adapter with refunds to the adapter; a completion
/// only once a unit's wX has landed, and one per intent, burning at `HomeWrapper` for the intent's kind.
contract HomeRedeemDeskTest is HomeBase {
    HomeRedeemDesk desk;
    address adapterL2 = makeAddr("HomeWrappedAdapter on 4663");

    function setUp() public override {
        super.setUp();
        desk = new HomeRedeemDesk(IArbOutbox(address(outbox)), IArbInbox(address(inbox)), adapterL2, IWrappedCollection(address(wNft)));
        vm.deal(alice, 10 ether);
    }

    function _intent(uint256 id, uint8 kind, address owner_) internal {
        outbox.execute(adapterL2, address(desk), abi.encodeCall(HomeRedeemDesk.intent, (id, kind, owner_)));
    }

    /// Alice wraps `tokenId` into the wrapper with the unit minted to the desk: what the gateway's release looks like from here.
    function _land(uint256 tokenId) internal {
        nft.mint(alice, tokenId);
        vm.startPrank(alice);
        nft.setApprovalForAll(address(wNft), true);
        wNft.wrap(tokenId, address(desk));
        vm.stopPrank();
    }

    function test_constructorRefusals() public {
        vm.expectRevert(HomeRedeemDesk.BadInput.selector);
        new HomeRedeemDesk(IArbOutbox(address(0)), IArbInbox(address(inbox)), adapterL2, IWrappedCollection(address(wNft)));
        vm.expectRevert(HomeRedeemDesk.BadInput.selector);
        new HomeRedeemDesk(IArbOutbox(address(outbox)), IArbInbox(address(inbox)), address(0), IWrappedCollection(address(wNft)));
    }

    function test_intent_onlyFromTheOutboxWithTheAdapterAsSender_once() public {
        vm.expectRevert(HomeRedeemDesk.NotAdapter.selector);
        desk.intent(0, 0, alice);                                                  // not the outbox
        vm.expectRevert(HomeRedeemDesk.NotAdapter.selector);
        outbox.execute(stranger, address(desk), abi.encodeCall(HomeRedeemDesk.intent, (0, 0, alice)));   // another L2 sender
        vm.expectRevert(HomeRedeemDesk.BadKind.selector);
        _intent(0, 2, alice);
        _intent(0, 0, alice);
        HomeRedeemDesk.Intent memory i = desk.intentOf(0);
        assertEq(uint8(i.stage), uint8(HomeRedeemDesk.Stage.Received));
        assertEq(i.kind, 0);
        assertEq(i.owner, alice);
        assertEq(desk.received(), 1);
        vm.expectRevert(HomeRedeemDesk.WrongStage.selector);
        _intent(0, 1, bob);                                                        // the same id again
    }

    function test_acknowledge_anyone_isARetryableToTheAdapterWithRefundsToTheAdapter() public {
        _intent(3, 1, alice);
        bytes memory data = abi.encodeCall(IHomeAdapterAck.acknowledge, (3));
        uint256 need = inbox.calculateRetryableSubmissionFee(data.length, block.basefee) + desk.L2_GAS_LIMIT() * desk.L2_MAX_FEE();
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(HomeRedeemDesk.Insufficient.selector, need, need - 1));
        desk.acknowledge{value: need - 1}(3);
        vm.expectRevert(HomeRedeemDesk.WrongStage.selector);
        desk.acknowledge{value: need}(9);                                          // no such intent
        vm.prank(alice);
        desk.acknowledge{value: need}(3);
        MockInbox.Retryable memory r = inbox.last();
        assertEq(r.to, adapterL2);
        assertEq(r.data, data);
        assertEq(r.l2CallValue, 0);
        assertEq(r.excessFeeRefund, adapterL2, "refunds never to a person");
        assertEq(r.callValueRefund, adapterL2);
        assertEq(r.gasLimit, desk.L2_GAS_LIMIT());
        assertEq(r.value, need);
        assertEq(uint8(desk.intentOf(3).stage), uint8(HomeRedeemDesk.Stage.Acknowledged));
        vm.prank(alice);
        vm.expectRevert(HomeRedeemDesk.WrongStage.selector);
        desk.acknowledge{value: need}(3);                                          // once
    }

    function test_complete_waitsForTheLanding_thenBurnsAtTheWrapperForTheKind_oncePerIntent() public {
        _intent(0, 0, alice);                                                      // burn
        _intent(1, 1, bob);                                                        // relist
        vm.expectRevert(HomeRedeemDesk.WrongStage.selector);                       // not acknowledged
        desk.complete(0);
        uint256 fee = 0.01 ether;
        vm.prank(alice);
        desk.acknowledge{value: fee}(0);
        vm.prank(alice);
        desk.acknowledge{value: fee}(1);
        vm.expectRevert(HomeRedeemDesk.NotArrived.selector);                       // the gateway has not released the wX yet
        desk.complete(0);
        _land(5);
        vm.prank(stranger);
        uint256 t0 = desk.complete(0);
        assertEq(wNft.balanceOf(address(desk)), 0, "the unit was burned");
        HomeWrapper.Ticket memory w = wNft.ticket(t0);
        assertEq(w.owner, address(desk));
        assertEq(w.destination, wNft.DEAD(), "a burn intent burns");
        assertEq(uint8(desk.intentOf(0).stage), uint8(HomeRedeemDesk.Stage.Completed));
        assertEq(desk.intentOf(0).homeTicket, t0);
        assertEq(desk.completed(), 1);
        vm.expectRevert(HomeRedeemDesk.WrongStage.selector);                       // once per intent
        desk.complete(0);
        vm.expectRevert(HomeRedeemDesk.NotArrived.selector);                       // the second intent waits for its own landing
        desk.complete(1);
        _land(6);
        uint256 t1 = desk.complete(1);
        assertEq(wNft.ticket(t1).destination, wNft.vault(), "a relist intent goes to the home vault's listing");
        assertEq(wNft.openTickets(), 2);
        assertEq(wNft.balanceOf(address(desk)), 0);
    }
}
