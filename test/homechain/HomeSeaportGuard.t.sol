// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {HomeBase} from "./HomeBase.sol";
import {MockNft} from "../nft/NftMocks.sol";
import {TriggerBot} from "./HomeMocks.sol";
import {HomeVaultBase} from "../../src/homechain/HomeVaultBase.sol";
import {SeaportGuard} from "../../src/nft/SeaportGuard.sol";
import {
    ItemType, OrderType, Side, BasicOrderType, OfferItem, ConsiderationItem, AdvancedOrder, CriteriaResolver, FulfillmentComponent,
    AdditionalRecipient, BasicOrderParameters, ISeaport16Sweep
} from "../../src/nft/Seaport16.sol";

/// test/nft/NftSeaportGuard.t.sol, ported to the home-chain vault: the same guard (imported, not copied), so every attack a
/// self-dealing trigger could hand `buySeaport` is refused the same way, and the outcome checks hold on top.
contract HomeSeaportGuardTest is HomeBase {
    address attacker = makeAddr("attacker");
    address accomplice = makeAddr("accomplice");

    function setUp() public override {
        super.setUp();
        _fundNative(raffle, 1, 20 ether);
        _cap(raffle, CAP);
    }

    function _buyAs(address who, bytes memory data, uint256[] memory ids) internal returns (uint256 bought, uint256 spent) {
        vm.prank(who, who);
        return raffle.buySeaport(data, ids, 1);
    }

    function _expectRefused(AdvancedOrder memory a, bytes memory err) internal {
        vm.expectRevert(err);
        _buyAs(trigger, _available(_one(a), address(raffle)), _ids(a.parameters.offer.length == 0 ? 1 : a.parameters.offer[0].identifierOrCriteria));
    }

    // ---- accepted ----------------------------------------------------------------------------------------------------------

    function test_aSignedFeeToAThirdPartyIsFine() public {
        _ask(_ids(1), 1 ether);
        BasicOrderParameters memory p = _basicParams(1);
        p.additionalRecipients = new AdditionalRecipient[](1);
        p.additionalRecipients[0] = AdditionalRecipient(0.025 ether, payable(makeAddr("OpenSea fee")));
        p.totalOriginalAdditionalRecipients = 1;
        (, uint256 spent) = _buyAs(trigger, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (p)), _ids(1));
        assertEq(spent, 1.025 ether);
    }

    function test_whatIsForwardedIsTheReencodingNotTheCallersBytes() public {
        _ask(_ids(1), 1 ether);
        bytes memory canonical = _buyCall(_ids(1), address(raffle));
        vm.expectCall(address(seaport), 1 ether, canonical);
        _buyAs(trigger, bytes.concat(canonical, hex"deadbeef"), _ids(1));
        assertEq(nft.ownerOf(1), address(raffle));
    }

    function test_aZeroRecipientBecomesTheVault() public {
        _ask(_ids(1), 1 ether);
        vm.expectCall(address(seaport), 1 ether, _buyCall(_ids(1), address(raffle)));
        _buyAs(trigger, _buyCall(_ids(1), address(0)), _ids(1));
    }

    function test_checkOrdersIsTheSameCheckAsAView() public {
        _ask(_ids(1), 1 ether);
        (bytes memory fwd, uint256 total) = raffle.checkOrders(_buyCall(_ids(1), address(0)), _ids(1));
        assertEq(total, 1 ether);
        assertEq(fwd, _buyCall(_ids(1), address(raffle)));
        _ask(_ids(2), 6 ether);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.OrderOverCap.selector, 0, 6 ether, CAP));
        raffle.checkOrders(_buyCall(_ids(2), address(0)), _ids(2));
    }

    // ---- attacks -------------------------------------------------------------------------------------------------------------

    function test_attack_selfListedOrder() public {
        nft.mint(attacker, 1);
        vm.prank(attacker);
        nft.setApprovalForAll(address(seaport), true);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.PaysTheCaller.selector, 0, attacker));
        _buyAs(attacker, _available(_one(_order(attacker, 1, CAP)), address(raffle)), _ids(1));
    }

    function test_attack_feeRecipientIsTheCaller() public {
        _ask(_ids(1), 1 ether);
        AdvancedOrder memory a = _order(seller, 1, 1 ether);
        ConsiderationItem[] memory c = new ConsiderationItem[](2);
        c[0] = a.parameters.consideration[0];
        c[1] = ConsiderationItem(ItemType.NATIVE, address(0), 0, 1 ether, 1 ether, payable(attacker));
        a.parameters.consideration = c;
        a.parameters.totalOriginalConsiderationItems = 2;
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.PaysTheCaller.selector, 0, attacker));
        _buyAs(attacker, _available(_one(a), address(raffle)), _ids(1));
    }

    function test_attack_paysTheTransactionSignerBehindAContract() public {
        TriggerBot bot = new TriggerBot();
        _ask(_ids(1), 1 ether);
        AdvancedOrder memory a = _order(seller, 1, 1 ether);
        ConsiderationItem[] memory c = new ConsiderationItem[](2);
        c[0] = a.parameters.consideration[0];
        c[1] = ConsiderationItem(ItemType.NATIVE, address(0), 0, 1 ether, 1 ether, payable(attacker));
        a.parameters.consideration = c;
        a.parameters.totalOriginalConsiderationItems = 2;
        vm.prank(address(this), attacker);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.PaysTheCaller.selector, 0, attacker));
        bot.call(address(raffle), abi.encodeCall(HomeVaultBase.buySeaport, (_available(_one(a), address(raffle)), _ids(1), 1)));
    }

    function test_attack_tipToAnAccomplice() public {
        _ask(_ids(1), 1 ether);
        AdvancedOrder memory a = _order(seller, 1, 1 ether);
        ConsiderationItem[] memory c = new ConsiderationItem[](2);
        c[0] = a.parameters.consideration[0];
        c[1] = ConsiderationItem(ItemType.NATIVE, address(0), 0, 1 ether, 1 ether, payable(accomplice));
        a.parameters.consideration = c;                            // totalOriginalConsiderationItems stays 1
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.Tip.selector, 0));
        _buyAs(attacker, _available(_one(a), address(raffle)), _ids(1));
        BasicOrderParameters memory p = _basicParams(1);
        p.additionalRecipients = new AdditionalRecipient[](1);
        p.additionalRecipients[0] = AdditionalRecipient(1 ether, payable(accomplice));
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.Tip.selector, 0));
        _buyAs(attacker, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder_efficient_6GL6yc, (p)), _ids(1));
        assertEq(accomplice.balance, 0);
    }

    function test_attack_anEmptyOfferPayingAnAccomplice() public {
        _ask(_ids(1), 0.1 ether);
        AdvancedOrder[] memory orders = new AdvancedOrder[](2);
        orders[0] = _order(seller, 1, 0.1 ether);
        orders[1] = _order(accomplice, 2, 4 ether);
        orders[1].parameters.offer = new OfferItem[](0);
        vm.prank(attacker, attacker);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 1));
        raffle.buySeaport(_available(orders, address(raffle)), _ids(1, 2), 1);
        assertEq(accomplice.balance, 0);
    }

    function test_attack_averagingOverTheCap() public {
        _ask(_ids(1), 7 ether);
        _ask(_ids(2), 1 ether);
        vm.prank(trigger, trigger);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.OrderOverCap.selector, 0, 7 ether, CAP));
        raffle.buySeaport(_buyCall(_ids(1, 2), address(raffle)), _ids(1, 2), 1);
    }

    function test_attack_anotherCollection() public {
        MockNft other = new MockNft();
        other.mint(seller, 1);
        vm.prank(seller);
        other.setApprovalForAll(address(seaport), true);
        nft.mint(seller, 1);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        _buyAs(trigger, _available(_one(_orderFor(address(other), seller, 1, 1 ether)), address(raffle)), _ids(1));
    }

    function test_attack_anIdThatIsNotACandidate() public {
        _ask(_ids(1, 2), 1 ether);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        _buyAs(trigger, _buyCall(_ids(2), address(raffle)), _ids(1));
    }

    function test_attack_disallowedSelectors() public {
        _ask(_ids(1), 1 ether);
        bytes memory available = _buyCall(_ids(1), address(raffle));
        bytes memory args = new bytes(available.length - 4);
        for (uint256 i = 4; i < available.length; ++i) args[i - 4] = available[i];
        bytes4[5] memory sels = [bytes4(0xf2d12b12), bytes4(0xe7acab24), bytes4(0xb3a34c4c), bytes4(0xfd9f1e10), bytes4(0x5b34b966)];
        for (uint256 i = 0; i < sels.length; ++i) {
            vm.expectRevert(abi.encodeWithSelector(SeaportGuard.UnsupportedCall.selector, sels[i]));
            _buyAs(trigger, bytes.concat(sels[i], args), _ids(1));
        }
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.UnsupportedCall.selector, bytes4(0)));
        _buyAs(trigger, "", _ids(1));
    }

    function test_offerAndConsiderationShapeRules() public {
        _ask(_ids(1), 1 ether);
        AdvancedOrder memory a;
        a = _order(seller, 1, 1 ether); a.parameters.offer[0].itemType = ItemType.ERC721_WITH_CRITERIA;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        a = _order(seller, 1, 1 ether); a.parameters.offer[0].endAmount = 2;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        a = _order(address(raffle), 1, 1 ether);
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.PaysTheCaller.selector, 0, address(raffle)));
        a = _order(seller, 1, 1 ether); a.parameters.consideration[0].itemType = ItemType.ERC20;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.WrongConsideration.selector, 0));
        a = _order(seller, 1, 1 ether); a.parameters.consideration[0].endAmount = 6 ether;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.OrderOverCap.selector, 0, 6 ether, CAP));
        a = _order(seller, 1, 1 ether); a.parameters.orderType = OrderType.CONTRACT;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.BadFill.selector));
        a = _order(seller, 1, 1 ether); a.numerator = 1; a.denominator = 2;
        _expectRefused(a, abi.encodeWithSelector(SeaportGuard.BadFill.selector));
        // a restricted order (a zone) passes the guard: OpenSea may use its signed zone
        a = _order(seller, 1, 1 ether); a.parameters.orderType = OrderType.FULL_RESTRICTED; a.parameters.zone = makeAddr("zone");
        _buyAs(trigger, _available(_one(a), address(raffle)), _ids(1));
        assertEq(nft.ownerOf(1), address(raffle));
    }

    function test_callLevelRules() public {
        _ask(_ids(1, 2), 1 ether);
        AdvancedOrder[] memory orders = _one(_order(seller, 1, 1 ether));
        CriteriaResolver[] memory crit = new CriteriaResolver[](1);
        crit[0] = CriteriaResolver(0, Side.OFFER, 0, 1, new bytes32[](0));
        FulfillmentComponent[][] memory none = new FulfillmentComponent[][](0);
        bytes memory d = abi.encodeCall(ISeaport16Sweep.fulfillAvailableAdvancedOrders, (orders, crit, none, none, bytes32(0), address(raffle), 1));
        vm.expectRevert(SeaportGuard.BadFill.selector);
        _buyAs(trigger, d, _ids(1));
        vm.expectRevert(SeaportGuard.BadFill.selector);
        _buyAs(trigger, _available(orders, attacker), _ids(1));           // bought for someone else
        BasicOrderParameters memory p = _basicParams(2);
        p.basicOrderType = BasicOrderType.ETH_TO_ERC1155_FULL_OPEN;
        vm.expectRevert(SeaportGuard.BadFill.selector);
        _buyAs(trigger, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (p)), _ids(2));
        p = _basicParams(2);
        p.fulfillerConduitKey = bytes32(uint256(1));
        vm.expectRevert(SeaportGuard.BadFill.selector);
        _buyAs(trigger, abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (p)), _ids(2));
    }

    /// Any bytes behind an accepted selector either decode to orders that pass every rule, or revert before Seaport is called.
    function testFuzz_arbitraryBytesNeverReachSeaportUnchecked(bytes memory junk, uint8 which) public {
        bytes4 sel = which % 3 == 0 ? ISeaport16Sweep.fulfillBasicOrder.selector
            : which % 3 == 1 ? ISeaport16Sweep.fulfillBasicOrder_efficient_6GL6yc.selector : ISeaport16Sweep.fulfillAvailableAdvancedOrders.selector;
        bytes memory data = bytes.concat(sel, junk);
        vm.prank(trigger, trigger);
        try raffle.buySeaport(data, _ids(1), 1) {
            assertEq(nft.ownerOf(1), address(raffle), "if it went through, the vault has candidate 1");
        } catch {}
        assertEq(seller.balance, 0, "nothing paid out by junk");
    }

    /// A byte overwritten or the data truncated anywhere in valid calldata: never a payout to anyone but the offerer at the ask.
    function testFuzz_corruptedValidCalldataPaysNobodyElse(uint16 at, uint8 b, bool truncate) public {
        _ask(_ids(1), 1 ether);
        bytes memory data = _buyCall(_ids(1), address(raffle));
        uint256 i = bound(at, 0, data.length - 1);
        if (truncate) {
            bytes memory t = new bytes(i);
            for (uint256 k = 0; k < i; ++k) t[k] = data[k];
            data = t;
        } else {
            data[i] = bytes1(b);
        }
        uint256 before = address(raffle).balance;
        vm.prank(trigger, trigger);
        try raffle.buySeaport(data, _ids(1), 1) {
            assertEq(nft.ownerOf(1), address(raffle));
            assertLe(before - address(raffle).balance, CAP + REWARD);
        } catch {
            assertEq(address(raffle).balance, before);
        }
        assertEq(attacker.balance, 0);
        assertEq(trigger.balance <= REWARD, true);
    }
}
