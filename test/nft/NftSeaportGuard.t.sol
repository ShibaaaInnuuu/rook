// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {NftBase} from "./NftBase.sol";
import {MockMarket} from "./NftMocks.sol";
import {MockNft} from "./NftMocks.sol";
import {NftSweepVault} from "../../src/nft/NftSweepVault.sol";
import {SeaportGuard} from "../../src/nft/SeaportGuard.sol";
import {
    ItemType, OrderType, Side, BasicOrderType, OfferItem, ConsiderationItem, OrderParameters, AdvancedOrder, CriteriaResolver,
    FulfillmentComponent, AdditionalRecipient, BasicOrderParameters, ISeaport16Sweep
} from "../../src/nft/Seaport16.sol";

/// The library, exposed, so the fuzzer can hand it any bytes. `msg.sender` inside it is whoever calls `check`.
contract GuardHarness {
    function check(bytes calldata data, address collection, uint256[] calldata candidates, uint256 capWei)
        external view returns (bytes memory forward, uint256 total)
    {
        return SeaportGuard.check(data, collection, candidates, capWei);
    }
}

/// A sweeper that is a contract: `msg.sender` to the vault is this, `tx.origin` is the EOA driving it.
contract SweeperContract {
    function sweep(NftSweepVault v, address target, bytes calldata data, uint256[] calldata ids) external {
        v.sweep(target, data, ids, 1);
    }
}

/// docs/v4/nft.md §5.1, §10.1: the Seaport 1.6 subset a sweep vault accepts, and every order a self-dealing sweeper could hand it.
/// Each `test_attack_*` succeeded against the vault before it decoded calldata (it forwarded any bytes and checked only balances),
/// and is refused now.
contract NftSeaportGuardTest is NftBase {
    address attacker = makeAddr("attacker");
    address accomplice = makeAddr("accomplice");
    GuardHarness harness;

    function setUp() public override {
        super.setUp();
        harness = new GuardHarness();
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
    }

    function _one(AdvancedOrder memory a) internal pure returns (AdvancedOrder[] memory orders) {
        orders = new AdvancedOrder[](1);
        orders[0] = a;
    }

    function _sweepAs(address who, bytes memory data, uint256[] memory ids) internal returns (uint256 bought, uint256 spent) {
        vm.prank(who, who);
        return raffleVault.sweep(address(market), data, ids, 1);
    }

    // ---- what is accepted ----------------------------------------------------------------------------------------------------

    function test_fulfillAvailableAdvancedOrdersBuysSeveral() public {
        _ask(_ids(1, 2, 3), 0.07 ether);
        (uint256 bought, uint256 spent) = _sweepAs(sweeper, _buyCall(_ids(1, 2, 3), address(raffleVault)), _ids(1, 2, 3));
        assertEq(bought, 3);
        assertEq(spent, 0.21 ether);
        assertEq(seller.balance, 0.21 ether);
    }

    function test_bothBasicSelectorsBuyOne() public {
        _ask(_ids(1, 2), 0.05 ether);
        _sweepAs(sweeper, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (_basicParams(1))), _ids(1));
        _sweepAs(sweeper, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder_efficient_6GL6yc, (_basicParams(2))), _ids(2));
        assertEq(nft.ownerOf(1), address(raffleVault));
        assertEq(nft.ownerOf(2), address(raffleVault));
        assertEq(raffleVault.acquisitionCount(), 2);
        assertEq(ISeaport16Sweep.fulfillBasicOrder_efficient_6GL6yc.selector, bytes4(0));
        assertEq(ISeaport16Sweep.fulfillBasicOrder.selector, bytes4(0xfb0f3ee1));
        assertEq(ISeaport16Sweep.fulfillAvailableAdvancedOrders.selector, bytes4(0x87201b41));
    }

    /// A basic order's original additional recipients (a marketplace fee, a royalty) are the offerer's signed terms: allowed.
    function test_aSignedFeeToAThirdPartyIsFine() public {
        _ask(_ids(1), 0.05 ether);
        BasicOrderParameters memory p = _basicParams(1);
        p.additionalRecipients = new AdditionalRecipient[](1);
        p.additionalRecipients[0] = AdditionalRecipient(0.005 ether, payable(makeAddr("marketplace fee")));
        p.totalOriginalAdditionalRecipients = 1;
        (, uint256 spent) = _sweepAs(sweeper, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (p)), _ids(1));
        assertEq(spent, 0.055 ether);
    }

    /// The vault never forwards the caller's bytes: trailing junk (or any non-canonical encoding) reaches Seaport canonical.
    function test_whatIsForwardedIsTheReencodingNotTheCallersBytes() public {
        _ask(_ids(1), 0.05 ether);
        bytes memory canonical = _buyCall(_ids(1), address(raffleVault));
        bytes memory junk = bytes.concat(canonical, hex"deadbeefdeadbeef");
        vm.expectCall(address(market), 0.05 ether, canonical);
        _sweepAs(sweeper, junk, _ids(1));
        assertEq(nft.ownerOf(1), address(raffleVault));
    }

    /// `recipient = 0` means "the caller" to Seaport; the vault writes itself in.
    function test_aZeroRecipientBecomesTheVault() public {
        _ask(_ids(1), 0.05 ether);
        vm.expectCall(address(market), 0.05 ether, _buyCall(_ids(1), address(raffleVault)));
        _sweepAs(sweeper, _buyCall(_ids(1), address(0)), _ids(1));
    }

    // ---- attacks that went through before, refused now -----------------------------------------------------------------------

    /// The sweeper lists its own NFT at the cap and fills it through the vault from the same address.
    function test_attack_selfListedOrder() public {
        nft.mint(attacker, 1);
        vm.prank(attacker);
        nft.setApprovalForAll(address(market), true);
        bytes memory data = _available(_one(_order(attacker, 1, CAP)), address(raffleVault));
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.PaysTheCaller.selector, 0, attacker));
        _sweepAs(attacker, data, _ids(1));
        // and as a basic order
        _note(1, attacker, CAP);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.PaysTheCaller.selector, 0, attacker));
        _sweepAs(attacker, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (_basicParams(1))), _ids(1));
    }

    /// A real seller's order whose (signed) fee recipient is the sweeper: the sweeper colluded with the seller on the terms.
    function test_attack_feeRecipientIsTheCaller() public {
        _ask(_ids(1), 0.04 ether);
        AdvancedOrder memory a = _order(seller, 1, 0.04 ether);
        ConsiderationItem[] memory c = new ConsiderationItem[](2);
        c[0] = a.parameters.consideration[0];
        c[1] = ConsiderationItem(ItemType.NATIVE, address(0), 0, 0.05 ether, 0.05 ether, payable(attacker));
        a.parameters.consideration = c;
        a.parameters.totalOriginalConsiderationItems = 2;          // signed: not a tip, a fee
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.PaysTheCaller.selector, 0, attacker));
        _sweepAs(attacker, _available(_one(a), address(raffleVault)), _ids(1));
        // the basic-order form of the same thing
        BasicOrderParameters memory p = _basicParams(1);
        p.additionalRecipients = new AdditionalRecipient[](1);
        p.additionalRecipients[0] = AdditionalRecipient(0.05 ether, payable(attacker));
        p.totalOriginalAdditionalRecipients = 1;
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.PaysTheCaller.selector, 0, attacker));
        _sweepAs(attacker, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (p)), _ids(1));
    }

    /// Through a contract: the vault sees the contract as caller, but the EOA signing the transaction is paid. Caught by tx.origin.
    function test_attack_paysTheTransactionSignerBehindAContract() public {
        SweeperContract bot = new SweeperContract();
        _ask(_ids(1), 0.04 ether);
        AdvancedOrder memory a = _order(seller, 1, 0.04 ether);
        ConsiderationItem[] memory c = new ConsiderationItem[](2);
        c[0] = a.parameters.consideration[0];
        c[1] = ConsiderationItem(ItemType.NATIVE, address(0), 0, 0.05 ether, 0.05 ether, payable(attacker));
        a.parameters.consideration = c;
        a.parameters.totalOriginalConsiderationItems = 2;
        vm.prank(address(this), attacker);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.PaysTheCaller.selector, 0, attacker));
        bot.sweep(raffleVault, address(market), _available(_one(a), address(raffleVault)), _ids(1));
    }

    /// A tip past the signed count, to a friend rather than to the caller: refused all the same, in both order kinds.
    function test_attack_tipToAnAccomplice() public {
        _ask(_ids(1), 0.04 ether);
        AdvancedOrder memory a = _order(seller, 1, 0.04 ether);
        ConsiderationItem[] memory c = new ConsiderationItem[](2);
        c[0] = a.parameters.consideration[0];
        c[1] = ConsiderationItem(ItemType.NATIVE, address(0), 0, 0.06 ether, 0.06 ether, payable(accomplice));
        a.parameters.consideration = c;                            // totalOriginalConsiderationItems stays 1
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.Tip.selector, 0));
        _sweepAs(attacker, _available(_one(a), address(raffleVault)), _ids(1));
        BasicOrderParameters memory p = _basicParams(1);
        p.additionalRecipients = new AdditionalRecipient[](1);
        p.additionalRecipients[0] = AdditionalRecipient(0.06 ether, payable(accomplice));
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.Tip.selector, 0));
        _sweepAs(attacker, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder_efficient_6GL6yc, (p)), _ids(1));
        assertEq(accomplice.balance, 0);
    }

    /// An order that offers NOTHING, paying an accomplice, riding next to a cheap real one inside the old aggregate bound.
    function test_attack_anEmptyOfferPayingAnAccomplice() public {
        _ask(_ids(1), 0.01 ether);
        AdvancedOrder[] memory orders = new AdvancedOrder[](2);
        orders[0] = _order(seller, 1, 0.01 ether);
        orders[1] = _order(accomplice, 2, 0.09 ether);
        orders[1].parameters.offer = new OfferItem[](0);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 1));
        _sweepAs(attacker, _available(orders, address(raffleVault)), _ids(1, 2));
        assertEq(accomplice.balance, 0);
    }

    /// One NFT above the cap, averaged down by a cheap one: stood under the old `spent ≤ cap × arrived`.
    function test_attack_averagingOverTheCap() public {
        _ask(_ids(1), 0.15 ether);
        _ask(_ids(2), 0.03 ether);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.OrderOverCap.selector, 0, 0.15 ether, CAP));
        _sweepAs(sweeper, _buyCall(_ids(1, 2), address(raffleVault)), _ids(1, 2));
    }

    function test_attack_anotherCollection() public {
        MockNft other = new MockNft();
        other.mint(seller, 1);
        vm.prank(seller);
        other.setApprovalForAll(address(market), true);
        nft.mint(seller, 1);                                       // candidate 1 of THIS collection exists, but is not what is sold
        bytes memory data = _available(_one(_orderFor(address(other), seller, 1, 0.01 ether)), address(raffleVault));
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        _sweepAs(sweeper, data, _ids(1));
        BasicOrderParameters memory p = _basicParams(1);
        p.offerToken = address(other);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        _sweepAs(sweeper, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (p)), _ids(1));
    }

    function test_attack_anIdThatIsNotACandidate() public {
        _ask(_ids(1, 2), 0.05 ether);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        _sweepAs(sweeper, _buyCall(_ids(2), address(raffleVault)), _ids(1));
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        _sweepAs(sweeper, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (_basicParams(2))), _ids(1));
    }

    /// Every other Seaport entry point, and anything that is not Seaport at all.
    function test_attack_disallowedSelectors() public {
        _ask(_ids(1), 0.05 ether);
        bytes memory available = _buyCall(_ids(1), address(raffleVault));
        bytes memory args = _slice(available, 4);
        string[9] memory sigs = [
            "fulfillOrder(((address,address,(uint8,address,uint256,uint256,uint256)[],(uint8,address,uint256,uint256,uint256,address)[],uint8,uint256,uint256,bytes32,uint256,bytes32,uint256),bytes),bytes32)",
            "fulfillAdvancedOrder(((address,address,(uint8,address,uint256,uint256,uint256)[],(uint8,address,uint256,uint256,uint256,address)[],uint8,uint256,uint256,bytes32,uint256,bytes32,uint256),uint120,uint120,bytes,bytes),(uint256,uint8,uint256,uint256,bytes32[])[],bytes32,address)",
            "fulfillAvailableOrders(((address,address,(uint8,address,uint256,uint256,uint256)[],(uint8,address,uint256,uint256,uint256,address)[],uint8,uint256,uint256,bytes32,uint256,bytes32,uint256),bytes)[],(uint256,uint256)[][],(uint256,uint256)[][],bytes32,uint256)",
            "matchOrders(((address,address,(uint8,address,uint256,uint256,uint256)[],(uint8,address,uint256,uint256,uint256,address)[],uint8,uint256,uint256,bytes32,uint256,bytes32,uint256),bytes)[],((uint256,uint256)[],(uint256,uint256)[])[])",
            "matchAdvancedOrders(((address,address,(uint8,address,uint256,uint256,uint256)[],(uint8,address,uint256,uint256,uint256,address)[],uint8,uint256,uint256,bytes32,uint256,bytes32,uint256),uint120,uint120,bytes,bytes)[],(uint256,uint8,uint256,uint256,bytes32[])[],((uint256,uint256)[],(uint256,uint256)[])[],address)",
            "cancel((address,address,(uint8,address,uint256,uint256,uint256)[],(uint8,address,uint256,uint256,uint256,address)[],uint8,uint256,uint256,bytes32,uint256,bytes32,uint256)[])",
            "validate(((address,address,(uint8,address,uint256,uint256,uint256)[],(uint8,address,uint256,uint256,uint256,address)[],uint8,uint256,uint256,bytes32,uint256,bytes32,uint256),bytes)[])",
            "incrementCounter()",
            "setApprovalForAll(address,bool)"
        ];
        for (uint256 i = 0; i < sigs.length; ++i) {
            bytes4 sel = bytes4(keccak256(bytes(sigs[i])));
            vm.expectRevert(abi.encodeWithSelector(SeaportGuard.UnsupportedCall.selector, sel));
            _sweepAs(sweeper, bytes.concat(sel, args), _ids(1));
        }
        assertEq(bytes4(keccak256(bytes(sigs[4]))), bytes4(0xf2d12b12), "matchAdvancedOrders, by its real selector");
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.UnsupportedCall.selector, bytes4(0)));
        _sweepAs(sweeper, "", _ids(1));
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.UnsupportedCall.selector, bytes4(0x87201b00)));
        _sweepAs(sweeper, hex"87201b", _ids(1));
    }

    // ---- the rest of the subset's rules --------------------------------------------------------------------------------------

    function test_offerShapeRules() public {
        _ask(_ids(1), 0.05 ether);
        AdvancedOrder memory a;
        a = _order(seller, 1, 0.05 ether); a.parameters.offer[0].itemType = ItemType.ERC721_WITH_CRITERIA;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        a = _order(seller, 1, 0.05 ether); a.parameters.offer[0].itemType = ItemType.ERC1155;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        a = _order(seller, 1, 0.05 ether); a.parameters.offer[0].endAmount = 2;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        a = _order(seller, 1, 0.05 ether);
        OfferItem[] memory two = new OfferItem[](2);
        two[0] = a.parameters.offer[0]; two[1] = a.parameters.offer[0];
        a.parameters.offer = two;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        // the vault itself as offerer (Seaport would skip the signature for its caller)
        a = _order(address(raffleVault), 1, 0.05 ether);
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.PaysTheCaller.selector, 0, address(raffleVault)));
    }

    function test_considerationShapeRules() public {
        _ask(_ids(1), 0.05 ether);
        AdvancedOrder memory a;
        a = _order(seller, 1, 0.05 ether); a.parameters.consideration[0].itemType = ItemType.ERC20;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.WrongConsideration.selector, 0));
        a = _order(seller, 1, 0.05 ether); a.parameters.consideration[0].token = address(nft);
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.WrongConsideration.selector, 0));
        a = _order(seller, 1, 0.05 ether); a.parameters.consideration[0].identifierOrCriteria = 1;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.WrongConsideration.selector, 0));
        // an ascending price counts at its highest
        a = _order(seller, 1, 0.05 ether); a.parameters.consideration[0].endAmount = 0.11 ether;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.OrderOverCap.selector, 0, 0.11 ether, CAP));
        // fewer items than the signed count is not a tip, but it is not a valid order either
        a = _order(seller, 1, 0.05 ether); a.parameters.totalOriginalConsiderationItems = 2;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.Tip.selector, 0));
    }

    function test_fillRules() public {
        _ask(_ids(1), 0.05 ether);
        AdvancedOrder memory a;
        a = _order(seller, 1, 0.05 ether); a.parameters.orderType = OrderType.CONTRACT;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.BadFill.selector));
        a = _order(seller, 1, 0.05 ether); a.numerator = 1; a.denominator = 2;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.BadFill.selector));
        a = _order(seller, 1, 0.05 ether); a.numerator = 0; a.denominator = 0;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.BadFill.selector));
        // a restricted order (a zone) is fine: OpenSea may use its signed zone
        a = _order(seller, 1, 0.05 ether); a.parameters.orderType = OrderType.FULL_RESTRICTED;
        a.parameters.zone = makeAddr("zone");
        _sweepAs(sweeper, _available(_one(a), address(raffleVault)), _ids(1));
        assertEq(nft.ownerOf(1), address(raffleVault));
    }

    function test_callLevelRules() public {
        _ask(_ids(1, 2), 0.05 ether);
        AdvancedOrder[] memory orders = _one(_order(seller, 1, 0.05 ether));
        CriteriaResolver[] memory crit = new CriteriaResolver[](1);
        crit[0] = CriteriaResolver(0, Side.OFFER, 0, 1, new bytes32[](0));
        FulfillmentComponent[][] memory none = new FulfillmentComponent[][](0);
        bytes memory d = abi.encodeCall(ISeaport16Sweep.fulfillAvailableAdvancedOrders,
            (orders, crit, none, none, bytes32(0), address(raffleVault), 1));
        vm.expectRevert(SeaportGuard.BadFill.selector);
        _sweepAs(sweeper, d, _ids(1));
        d = abi.encodeCall(ISeaport16Sweep.fulfillAvailableAdvancedOrders,
            (orders, new CriteriaResolver[](0), none, none, bytes32(uint256(1)), address(raffleVault), 1));
        vm.expectRevert(SeaportGuard.BadFill.selector);
        _sweepAs(sweeper, d, _ids(1));
        // NFTs bought for someone else
        vm.expectRevert(SeaportGuard.BadFill.selector);
        _sweepAs(sweeper, _available(orders, attacker), _ids(1));
        // no orders, or more orders than candidates
        vm.expectRevert(SeaportGuard.BadFill.selector);
        _sweepAs(sweeper, _available(new AdvancedOrder[](0), address(raffleVault)), _ids(1));
        AdvancedOrder[] memory twoOrders = new AdvancedOrder[](2);
        twoOrders[0] = _order(seller, 1, 0.05 ether);
        twoOrders[1] = _order(seller, 1, 0.05 ether);
        vm.expectRevert(SeaportGuard.BadFill.selector);
        _sweepAs(sweeper, _available(twoOrders, address(raffleVault)), _ids(1));
        // basic: a route that is not ETH for an ERC-721, and a fulfiller conduit
        BasicOrderParameters memory p = _basicParams(2);
        p.basicOrderType = BasicOrderType.ETH_TO_ERC1155_FULL_OPEN;
        vm.expectRevert(SeaportGuard.BadFill.selector);
        _sweepAs(sweeper, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (p)), _ids(2));
        p = _basicParams(2);
        p.fulfillerConduitKey = bytes32(uint256(1));
        vm.expectRevert(SeaportGuard.BadFill.selector);
        _sweepAs(sweeper, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (p)), _ids(2));
        p = _basicParams(2);
        p.considerationToken = address(nft);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.WrongConsideration.selector, 0));
        _sweepAs(sweeper, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (p)), _ids(2));
        p = _basicParams(2);
        p.offerAmount = 2;
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        _sweepAs(sweeper, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (p)), _ids(2));
    }

    /// The residual, stated as a test so it cannot be forgotten (docs/v4/nft.md §10.1): the sweeper's OTHER address lists an NFT of
    /// the collection at the cap, and the sweeper fills it. The vault buys a real NFT at no more than the cap. Bounded, not closed.
    function test_residual_aSecondAddressSellingAtTheCapIsIndistinguishableFromAnHonestSeller() public {
        nft.mint(accomplice, 1);
        vm.prank(accomplice);
        nft.setApprovalForAll(address(market), true);
        (uint256 bought, uint256 spent) = _sweepAs(attacker, _available(_one(_order(accomplice, 1, CAP)), address(raffleVault)), _ids(1));
        assertEq(bought, 1);
        assertEq(spent, CAP, "at most the cap per NFT; the loss is cap minus the floor");
        assertEq(nft.ownerOf(1), address(raffleVault), "and the vault holds a real NFT of its collection");
    }

    function _expectRefused(AdvancedOrder memory a, bytes memory err) internal {
        vm.expectRevert(err);
        _sweepAs(sweeper, _available(_one(a), address(raffleVault)), _ids(1));
    }

    function _slice(bytes memory b, uint256 from) internal pure returns (bytes memory r) {
        r = new bytes(b.length - from);
        for (uint256 i = 0; i < r.length; ++i) r[i] = b[from + i];
    }

    // ---- the decoder, fuzzed -------------------------------------------------------------------------------------------------

    /// Whatever the guard lets through, checked again from the bytes it would FORWARD, independently of the guard's own code.
    function _assertForwardIsSafe(bytes memory fwd, uint256 total, uint256[] memory cands, uint256 capWei) internal view {
        bytes4 sel = bytes4(fwd);
        bytes memory args = _slice(fwd, 4);
        uint256 sum;
        if (sel == ISeaport16Sweep.fulfillAvailableAdvancedOrders.selector) {
            (AdvancedOrder[] memory orders, CriteriaResolver[] memory crit,,, bytes32 conduit, address recipient,) = abi.decode(args,
                (AdvancedOrder[], CriteriaResolver[], FulfillmentComponent[][], FulfillmentComponent[][], bytes32, address, uint256));
            assertEq(crit.length, 0);
            assertEq(conduit, bytes32(0));
            assertEq(recipient, address(harness));
            assertTrue(orders.length >= 1 && orders.length <= cands.length);
            for (uint256 i = 0; i < orders.length; ++i) {
                OrderParameters memory o = orders[i].parameters;
                assertTrue(orders[i].numerator == orders[i].denominator && orders[i].numerator != 0);
                assertTrue(o.orderType != OrderType.CONTRACT);
                assertEq(o.offer.length, 1);
                assertTrue(o.offer[0].itemType == ItemType.ERC721 && o.offer[0].token == address(nft));
                assertTrue(_in(o.offer[0].identifierOrCriteria, cands));
                assertEq(o.consideration.length, o.totalOriginalConsiderationItems, "no tip");
                assertTrue(o.offerer != address(this) && o.offerer != tx.origin);
                uint256 price;
                for (uint256 j = 0; j < o.consideration.length; ++j) {
                    assertTrue(o.consideration[j].itemType == ItemType.NATIVE);
                    assertTrue(o.consideration[j].recipient != address(this) && o.consideration[j].recipient != tx.origin);
                    price += o.consideration[j].startAmount > o.consideration[j].endAmount
                        ? o.consideration[j].startAmount : o.consideration[j].endAmount;
                }
                assertLe(price, capWei);
                sum += price;
            }
        } else {
            assertTrue(sel == ISeaport16Sweep.fulfillBasicOrder.selector || sel == bytes4(0));
            BasicOrderParameters memory p = abi.decode(args, (BasicOrderParameters));
            assertLe(uint8(p.basicOrderType), 3);
            assertTrue(p.offerToken == address(nft) && p.offerAmount == 1 && _in(p.offerIdentifier, cands));
            assertEq(p.additionalRecipients.length, p.totalOriginalAdditionalRecipients, "no tip");
            assertTrue(p.offerer != address(this) && p.offerer != tx.origin);
            sum = p.considerationAmount;
            for (uint256 j = 0; j < p.additionalRecipients.length; ++j) {
                assertTrue(p.additionalRecipients[j].recipient != address(this));
                sum += p.additionalRecipients[j].amount;
            }
            assertLe(sum, capWei);
        }
        assertEq(sum, total);
        assertLe(total, capWei * cands.length);
    }

    function _in(uint256 id, uint256[] memory cands) internal pure returns (bool) {
        for (uint256 i = 0; i < cands.length; ++i) if (cands[i] == id) return true;
        return false;
    }

    /// Arbitrary bytes, with each accepted selector in front: the guard reverts, or what it forwards is safe.
    function testFuzz_garbageNeverGetsThrough(bytes calldata body, uint8 which) public view {
        bytes4[3] memory sels = [ISeaport16Sweep.fulfillBasicOrder.selector, bytes4(0),
            ISeaport16Sweep.fulfillAvailableAdvancedOrders.selector];
        bytes memory data = bytes.concat(sels[which % 3], body);
        try harness.check(data, address(nft), _ids(1, 2), CAP) returns (bytes memory fwd, uint256 total) {
            _assertForwardIsSafe(fwd, total, _ids(1, 2), CAP);
        } catch {}
    }

    /// A valid sweep's calldata with one byte overwritten anywhere, or truncated anywhere: the guard reverts, or forwards a call
    /// that is still safe. It never forwards the damaged bytes themselves.
    function testFuzz_corruptedValidCalldata(uint256 pos, uint8 value, uint256 cut, bool basic) public view {
        uint256[] memory cands = _ids(1, 2, 3);
        bytes memory data;
        if (basic) {
            BasicOrderParameters memory p;
            p.offerer = payable(seller); p.offerToken = address(nft); p.offerIdentifier = 2; p.offerAmount = 1;
            p.considerationAmount = 0.05 ether; p.endTime = type(uint256).max;
            p.additionalRecipients = new AdditionalRecipient[](1);
            p.additionalRecipients[0] = AdditionalRecipient(0.001 ether, payable(address(0xFEE)));
            p.totalOriginalAdditionalRecipients = 1;
            p.signature = hex"0102030405";
            data = abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (p));
        } else {
            AdvancedOrder[] memory orders = new AdvancedOrder[](2);
            orders[0] = _order(seller, 1, 0.05 ether);
            orders[1] = _order(seller, 3, 0.07 ether);
            orders[1].signature = hex"aabbcc";
            data = _available(orders, address(harness));
        }
        bytes memory bad = bytes.concat(data);
        bad[pos % bad.length] = bytes1(value);
        _tryAndCheck(bad, cands);
        cut = cut % (data.length + 1);
        bytes memory short = new bytes(cut);
        for (uint256 i = 0; i < cut; ++i) short[i] = data[i];
        _tryAndCheck(short, cands);
    }

    function _tryAndCheck(bytes memory data, uint256[] memory cands) internal view {
        try harness.check(data, address(nft), cands, CAP) returns (bytes memory fwd, uint256 total) {
            _assertForwardIsSafe(fwd, total, cands, CAP);
        } catch {}
    }

    /// Structured: a random order, with random deviations. The guard accepts exactly when no rule is broken.
    function testFuzz_acceptsExactlyTheSafeOrders(uint96 price, uint96 capWei, uint8 flags, uint96 extra) public view {
        price = uint96(bound(price, 0, 2 ether));
        capWei = uint96(bound(capWei, 1, 2 ether));
        extra = uint96(bound(extra, 0, 1 ether));
        AdvancedOrder memory a = _order(seller, 1, price);
        bool tip = flags & 1 != 0;
        bool feeToCaller = flags & 2 != 0;
        bool otherCollection = flags & 4 != 0;
        bool notCandidate = flags & 8 != 0;
        bool contractOrder = flags & 16 != 0;
        if (tip || feeToCaller) {
            ConsiderationItem[] memory c = new ConsiderationItem[](2);
            c[0] = a.parameters.consideration[0];
            c[1] = ConsiderationItem(ItemType.NATIVE, address(0), 0, extra, extra, payable(feeToCaller ? address(this) : accomplice));
            a.parameters.consideration = c;
            if (!tip) a.parameters.totalOriginalConsiderationItems = 2;
        }
        if (otherCollection) a.parameters.offer[0].token = accomplice;
        if (notCandidate) a.parameters.offer[0].identifierOrCriteria = 9;
        if (contractOrder) a.parameters.orderType = OrderType.CONTRACT;
        uint256 total = uint256(price) + ((tip || feeToCaller) ? extra : 0);
        bool shouldPass = !tip && !feeToCaller && !otherCollection && !notCandidate && !contractOrder && total <= capWei;
        try harness.check(_available(_one(a), address(0)), address(nft), _ids(1, 2), capWei) returns (bytes memory fwd, uint256 t) {
            assertTrue(shouldPass, "accepted an order that breaks a rule");
            assertEq(t, total);
            _assertForwardIsSafe(fwd, t, _ids(1, 2), capWei);
        } catch {
            assertFalse(shouldPass, "refused a safe order");
        }
    }
}
