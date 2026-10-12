// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20Errors} from "@openzeppelin/contracts/interfaces/draft-IERC6093.sol";
import {NftWrapBase} from "./NftWrapBase.sol";
import {Clock} from "../utils/Clock.sol";
import {MockBeacon} from "../utils/Mocks.sol";
import {NftWrapper} from "../../src/nft/NftWrapper.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";

/// docs/v4/nft-wrapper.md §2: mint per NFT in custody, burn to a ticket, resolve by beacon, deliver, refund on deadline.
contract NftWrapperTest is NftWrapBase {
    // ---- what the wrapper is ----

    function test_oneWholeTokenIsOneNft() public view {
        assertEq(w.decimals(), 18);
        assertEq(w.ONE_ITEM(), 1e18);
        assertEq(w.symbol(), "wAPE");
        assertEq(w.chainId(), 1);
        assertEq(w.collection(), collection);
        assertEq(w.custodian(), custodian);
        assertEq(address(w.beacon()), address(beacon));
        assertEq(w.DEAD(), DEAD);
        assertEq(w.DEPOSIT_MAGIC(), bytes4("EWN1"));
    }

    function test_theConstructorRefusesWhatCannotWork() public {
        IRandomnessBeacon b = beacon;
        vm.expectRevert(NftWrapper.BadInput.selector);
        new NftWrapper(IRandomnessBeacon(address(0)), 1, collection, custodian, "x", "x");
        vm.expectRevert(NftWrapper.BadInput.selector);
        new NftWrapper(b, 1, address(0), custodian, "x", "x");
        vm.expectRevert(NftWrapper.BadInput.selector);
        new NftWrapper(b, 1, collection, address(0), "x", "x");
        vm.expectRevert(NftWrapper.BadInput.selector);
        new NftWrapper(b, 0, collection, custodian, "x", "x");
    }

    function test_aCollectionOnThisChainIsNotWrappedHere_itBelongsInContractCustody() public {
        vm.chainId(4663);
        vm.expectRevert(NftWrapper.BadInput.selector);
        new NftWrapper(beacon, 4663, collection, custodian, "x", "x");
    }

    function test_theCustodianMustBeCodeless_theCanistersKeyNotAContract() public {
        vm.expectRevert(NftWrapper.BadInput.selector);
        new NftWrapper(beacon, 1, collection, address(beacon), "x", "x");
    }

    // ---- minting ----

    function test_theCanisterMintsOneWholeTokenPerNft_toWhomeverTheRequestNames() public {
        vm.expectEmit(address(w));
        emit NftWrapper.Wrapped(0, 42, _tx(42), curve);
        uint256 itemId = _mint(42, curve);
        assertEq(itemId, 0);
        assertEq(w.balanceOf(curve), ONE);
        assertEq(w.totalSupply(), ONE);
        assertEq(w.held(), 1);
        assertEq(w.itemOf(42), 1, "itemId + 1 while held");
        NftWrapper.Item memory it = w.item(0);
        assertEq(it.tokenId, 42);
        assertEq(it.sourceTx, _tx(42));
        assertFalse(it.deposit);
        assertTrue(w.sourceUsed(keccak256(abi.encode(uint256(42), _tx(42)))));
        assertEq(w.pendingCount(), 1, "waiting for the next ticket");
        _assertLedger();
    }

    function test_onlyTheCanisterMints() public {
        address[4] memory others = [alice, curve, collection, address(this)];
        for (uint256 k = 0; k < others.length; ++k) {
            vm.prank(others[k]);
            vm.expectRevert(NftWrapper.NotCustodian.selector);
            w.mint(1, _tx(1), others[k]);
            vm.prank(others[k]);
            vm.expectRevert(NftWrapper.NotCustodian.selector);
            w.mintDeposit(1, _tx(1), others[k]);
        }
    }

    function test_aMintNeedsARecipientAndATransaction() public {
        vm.startPrank(custodian);
        vm.expectRevert(NftWrapper.BadInput.selector);
        w.mint(1, _tx(1), address(0));
        vm.expectRevert(NftWrapper.BadInput.selector);
        w.mint(1, bytes32(0), curve);
        vm.stopPrank();
    }

    function test_aTokenIdHeldIsNeverMintedAgain_byEitherPathOrAnyTransaction() public {
        _mint(7, curve);
        vm.startPrank(custodian);
        vm.expectRevert(NftWrapper.AlreadyHeld.selector);
        w.mint(7, keccak256("another purchase"), curve);
        vm.expectRevert(NftWrapper.AlreadyHeld.selector);
        w.mintDeposit(7, keccak256("a deposit"), alice);
        vm.stopPrank();
        assertEq(w.held(), 1);
        assertEq(w.totalSupply(), ONE);
    }

    function test_aDeliveredTokenIdCanComeBack_onlyByANewTransaction() public {
        _mint(7, alice);
        uint256 t = _redeem(alice);
        _resolve(t, keccak256("r"));
        _deliver(t);
        assertEq(w.itemOf(7), 0, "no longer held");
        // the same attestation replayed: refused for ever
        vm.prank(custodian);
        vm.expectRevert(NftWrapper.AlreadyMinted.selector);
        w.mint(7, _tx(7), curve);
        // bought back later by a new transaction: a new item
        vm.prank(custodian);
        uint256 again = w.mint(7, keccak256("bought back"), curve);
        assertEq(again, 1);
        assertEq(w.itemOf(7), 2);
        assertEq(w.item(0).tokenId, 7, "the old item keeps its record");
        _assertLedger();
    }

    function test_aHolderDepositMintsToTheBoundRecipient_andIsMarkedAsOne() public {
        bytes32 depositTx = keccak256("alice's safeTransferFrom");
        vm.expectEmit(address(w));
        emit NftWrapper.Deposited(0, 99, depositTx, alice);
        vm.prank(custodian);
        uint256 itemId = w.mintDeposit(99, depositTx, alice);
        assertEq(w.balanceOf(alice), ONE);
        assertTrue(w.item(itemId).deposit);
        vm.prank(custodian);
        vm.expectRevert(NftWrapper.AlreadyHeld.selector);
        w.mintDeposit(99, depositTx, alice);
        _assertLedger();
    }

    // ---- redeeming ----

    function test_aBurnPinsAFutureRoundAndFixesTheDestination() public {
        _mint(1, alice);
        address elsewhere = makeAddr("alice's address on Ethereum");
        uint64 expected = beacon.roundAt(block.timestamp) + w.PIN_AHEAD();
        vm.expectEmit(address(w));
        emit NftWrapper.Redeemed(0, alice, expected, elsewhere);
        vm.prank(alice);
        uint256 id = w.redeem(elsewhere);
        NftWrapper.Ticket memory t = w.ticket(id);
        assertEq(uint8(t.stage), uint8(NftWrapper.Stage.Pinned));
        assertEq(t.owner, alice);
        assertEq(t.destination, elsewhere);
        assertEq(t.round, expected);
        assertEq(beacon.randomnessOf(t.round), bytes32(0), "the round does not exist yet");
        assertEq(w.balanceOf(alice), 0);
        assertEq(w.openTickets(), 1);
        _assertLedger();
    }

    function test_theDestinationMustBeARealAddressAndNotCustody() public {
        _mint(1, alice);
        vm.startPrank(alice);
        vm.expectRevert(NftWrapper.BadDestination.selector);
        w.redeem(address(0));
        vm.expectRevert(NftWrapper.BadDestination.selector);
        w.redeem(custodian);
        vm.stopPrank();
    }

    function test_aRedeemNeedsOneWholeToken() public {
        _mint(1, alice);
        vm.prank(alice);
        w.transfer(bob, 1);
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(IERC20Errors.ERC20InsufficientBalance.selector, alice, ONE - 1, ONE));
        w.redeem(alice);
    }

    function test_resolveDrawsAHeldNftAndStartsTheDeadline() public {
        _mint(1, alice); _mint(2, alice); _mint(3, alice);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        NftWrapper.Ticket memory t = w.ticket(id);
        assertEq(uint8(t.stage), uint8(NftWrapper.Stage.Resolved));
        assertEq(t.deliverBy, block.timestamp + 7 days);
        uint256 tok = _tokenOf(id);
        assertTrue(tok >= 1 && tok <= 3);
        assertEq(w.itemOf(tok), t.itemId + 1, "still held until delivered");
        assertEq(w.freeCount(), 2);
        assertEq(w.nextToResolve(), 1);
        _assertLedger();
    }

    function test_theDrawIsTheFormula_notAnyonesChoice() public {
        for (uint256 i = 1; i <= 5; ++i) _mint(i, alice);
        uint256 id = _redeem(alice);
        bytes32 r = keccak256("the round");
        _resolve(id, r);
        uint256 index = uint256(keccak256(abi.encode(r, address(w), id))) % 5;
        assertEq(w.ticket(id).itemId, index, "items 0..4 in order in the free set, swap-and-pop from the draw");
    }

    function test_ticketsResolveStrictlyInOrder() public {
        _mint(1, alice); _mint(2, alice);
        _redeem(alice);
        uint256 later = _redeem(alice);
        vm.expectRevert(abi.encodeWithSelector(NftWrapper.OutOfOrder.selector, 0));
        w.resolve(later, abi.encodePacked(keccak256("r")));
    }

    function test_aTicketResolvesOnce() public {
        _mint(1, alice);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        vm.expectRevert(abi.encodeWithSelector(NftWrapper.OutOfOrder.selector, 1));
        w.resolve(id, abi.encodePacked(keccak256("r")));
    }

    // ---- delivering ----

    function test_theCanisterRecordsTheDelivery_andTheNftLeavesTheBooks() public {
        _mint(1, alice);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        bytes32 txh = keccak256("transferFrom on chain 1");
        vm.expectEmit(address(w));
        emit NftWrapper.Delivered(id, 1, alice, txh);
        vm.prank(custodian);
        w.recordDelivery(id, txh);
        assertEq(uint8(w.ticket(id).stage), uint8(NftWrapper.Stage.Delivered));
        assertEq(w.ticket(id).deliveryTx, txh);
        assertEq(w.held(), 0);
        assertEq(w.openTickets(), 0);
        assertEq(w.itemOf(1), 0);
        assertEq(w.burned(), 0);
        _assertLedger();
    }

    function test_onlyTheCanisterRecordsADelivery() public {
        _mint(1, alice);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        address[3] memory others = [alice, curve, makeAddr("keeper")];
        for (uint256 k = 0; k < others.length; ++k) {
            vm.prank(others[k]);
            vm.expectRevert(NftWrapper.NotCustodian.selector);
            w.recordDelivery(id, keccak256("tx"));
        }
    }

    function test_aDeliveryIsRecordedOnlyForAResolvedTicket_withARealHash() public {
        _mint(1, alice); _mint(2, alice);
        uint256 id = _redeem(alice);
        vm.startPrank(custodian);
        vm.expectRevert(NftWrapper.WrongStage.selector);
        w.recordDelivery(id, keccak256("tx"));         // pinned, not drawn
        vm.stopPrank();
        _resolve(id, keccak256("r"));
        vm.startPrank(custodian);
        vm.expectRevert(NftWrapper.BadInput.selector);
        w.recordDelivery(id, bytes32(0));
        w.recordDelivery(id, keccak256("tx"));
        vm.expectRevert(NftWrapper.WrongStage.selector);
        w.recordDelivery(id, keccak256("tx again"));   // twice
        vm.stopPrank();
    }

    function test_aDeliveryIsStillRecordedAfterTheDeadline_ifNobodyRefunded() public {
        _mint(1, alice);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        Clock.skip(8 days);
        _deliver(id);
        _assertLedger();
    }

    // ---- burning ----

    function test_aBurnIsARedemptionToDead() public {
        _mint(1, curve);
        vm.prank(curve);
        uint256 id = w.redeemToBurn();
        assertEq(w.ticket(id).destination, DEAD);
        _resolve(id, keccak256("r"));
        _deliver(id);
        assertEq(w.burned(), 1);
        assertEq(w.held(), 0);
        _assertLedger();
    }

    function test_redeemingToDeadByHandIsTheSameAsABurn() public {
        _mint(1, alice);
        vm.prank(alice);
        uint256 id = w.redeem(DEAD);
        _resolve(id, keccak256("r"));
        _deliver(id);
        assertEq(w.burned(), 1);
    }

    // ---- refunds ----

    function test_anUndeliveredTicketIsRefundedByAnyone_theNftBackInTheSet() public {
        _mint(1, alice); _mint(2, alice);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        uint256 itemId = w.ticket(id).itemId;
        vm.expectRevert(NftWrapper.TooEarly.selector);
        w.refund(id);
        Clock.skip(7 days);
        vm.expectEmit(address(w));
        emit NftWrapper.Refunded(id, itemId, alice);
        vm.prank(carol);
        w.refund(id);
        assertTrue(w.refunded(id));
        assertEq(w.balanceOf(alice), 2 * ONE, "the redeemer has her token back");
        assertEq(w.openTickets(), 0);
        assertEq(w.held(), 2);
        assertEq(w.pendingCount(), 1, "the NFT waits for the next ticket");
        vm.prank(custodian);
        vm.expectRevert(NftWrapper.WrongStage.selector);
        w.recordDelivery(id, keccak256("too late"));
        _assertLedger();
    }

    function test_aRefundedNftIsDrawableByTheNextTicket() public {
        _mint(1, alice);
        uint256 a = _redeem(alice);
        _resolve(a, keccak256("r"));
        Clock.skip(7 days);
        w.refund(a);
        uint256 b = _redeem(alice);
        _resolve(b, keccak256("s"));
        assertEq(_tokenOf(b), 1);
        _deliver(b);
        _assertLedger();
    }

    function test_onlyAResolvedTicketRefunds() public {
        _mint(1, alice);
        uint256 id = _redeem(alice);
        Clock.skip(30 days);
        vm.expectRevert(NftWrapper.WrongStage.selector);
        w.refund(id);
        assertFalse(w.refunded(id));
    }

    function test_aDeliveredTicketNeverRefunds() public {
        _mint(1, alice);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        _deliver(id);
        Clock.skip(8 days);
        vm.expectRevert(NftWrapper.WrongStage.selector);
        w.refund(id);
    }

    // ---- drand skipped a round ----

    function test_anUnprovenRoundIsRepinnedADayLater() public {
        _mint(1, alice);
        uint256 id = _redeem(alice);
        uint64 first = w.ticket(id).round;
        vm.expectRevert(NftWrapper.TooEarly.selector);
        w.redraw(id);
        Clock.skip(24 hours);
        w.redraw(id);
        assertGt(w.ticket(id).round, first);
        _resolve(id, keccak256("r"));
        assertEq(_tokenOf(id), 1);
    }

    function test_aProvenRoundIsNeverRepinned() public {
        _mint(1, alice);
        uint256 id = _redeem(alice);
        beacon.prove(w.ticket(id).round, abi.encodePacked(keccak256("r")));
        Clock.skip(2 days);
        vm.expectRevert(NftWrapper.TooEarly.selector);
        w.redraw(id);
    }

    function test_aResolvedTicketIsNeverRepinned() public {
        _mint(1, alice);
        uint256 id = _redeem(alice);
        _resolve(id, keccak256("r"));
        Clock.skip(2 days);
        vm.expectRevert(NftWrapper.WrongStage.selector);
        w.redraw(id);
    }

    // ---- fuzz ----

    /// Any number of mints, any number of burns, resolved with any randomness: every draw is a distinct held NFT, and the
    /// ledger holds after every step.
    function testFuzz_drawsAreDistinctHeldNfts(uint8 nMint, uint8 nBurn, bytes32 seed) public {
        uint256 m = bound(nMint, 1, 30);
        uint256 b = bound(nBurn, 1, m);
        for (uint256 i = 0; i < m; ++i) _mint(1000 + i, alice);
        for (uint256 i = 0; i < b; ++i) { _redeem(alice); _assertLedger(); }
        bool[] memory drawn = new bool[](m);
        for (uint256 i = 0; i < b; ++i) {
            _resolve(i, keccak256(abi.encode(seed, i)));
            uint256 itemId = w.ticket(i).itemId;
            assertFalse(drawn[itemId], "one NFT drawn twice");
            drawn[itemId] = true;
            assertEq(w.itemOf(w.item(itemId).tokenId), itemId + 1, "a drawn NFT is still held");
            _assertLedger();
        }
        assertEq(w.freeCount(), m - b);
    }

    /// Any mix of deliveries and refunds keeps the books.
    function testFuzz_deliverOrRefund_theLedgerHolds(uint8 n, uint256 choices) public {
        uint256 k = bound(n, 1, 20);
        for (uint256 i = 0; i < k; ++i) _mint(i, alice);
        for (uint256 i = 0; i < k; ++i) _redeem(alice);
        for (uint256 i = 0; i < k; ++i) _resolve(i, keccak256(abi.encode(i)));
        Clock.skip(7 days);
        uint256 delivered;
        for (uint256 i = 0; i < k; ++i) {
            if ((choices >> i) & 1 == 1) { _deliver(i); delivered++; }
            else w.refund(i);
            _assertLedger();
        }
        assertEq(w.held(), k - delivered);
        assertEq(w.balanceOf(alice), (k - delivered) * ONE);
    }

    /// No second mint of a held token id, whatever the transaction hash or path.
    function testFuzz_aHeldTokenIdNeverMintsTwice(uint256 tokenId, bytes32 tx1, bytes32 tx2, bool deposit) public {
        vm.assume(tx1 != bytes32(0) && tx2 != bytes32(0));
        vm.prank(custodian);
        w.mint(tokenId, tx1, alice);
        vm.prank(custodian);
        vm.expectRevert(NftWrapper.AlreadyHeld.selector);
        if (deposit) w.mintDeposit(tokenId, tx2, bob); else w.mint(tokenId, tx2, bob);
        assertEq(w.totalSupply(), ONE);
    }

    /// The same (token id, transaction) never mints twice, even after the NFT has left and the id is free again.
    function testFuzz_anAttestationMintsOnceEver(uint256 tokenId, bytes32 txh, bytes32 r) public {
        vm.assume(txh != bytes32(0));
        vm.prank(custodian);
        w.mint(tokenId, txh, alice);
        uint256 id = _redeem(alice);
        _resolve(id, r);
        _deliver(id);
        vm.prank(custodian);
        vm.expectRevert(NftWrapper.AlreadyMinted.selector);
        w.mintDeposit(tokenId, txh, alice);
    }

    function test_aRealBeaconShapeIsAllItNeeds() public {
        // the mock and drand's evmnet contract share one interface: the wrapper reads nothing else
        MockBeacon other = new MockBeacon();
        NftWrapper w2 = new NftWrapper(other, 8453, collection, custodian, "Wrapped Base NFT", "wBASE");
        assertEq(w2.chainId(), 8453);
    }
}
