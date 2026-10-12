// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {IERC721} from "@openzeppelin/contracts/token/ERC721/IERC721.sol";
import {IPonsV2LaunchFactory, IPonsV2FeeEscrow} from "../../src/interfaces/IPons.sol";
import {ListingRegistry} from "../../src/ListingRegistry.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";
import {NftRegistry} from "../../src/nft/NftRegistry.sol";
import {NftLauncher} from "../../src/nft/NftLauncher.sol";
import {NftFeeRouter} from "../../src/nft/NftFeeRouter.sol";
import {NftSweepVault} from "../../src/nft/NftSweepVault.sol";
import {NftVaultBase} from "../../src/nft/NftVaultBase.sol";
import {Policy} from "../../src/nft/INft.sol";

import {SeaportGuard} from "../../src/nft/SeaportGuard.sol";
import {
    ItemType, OrderType, BasicOrderType, OfferItem, ConsiderationItem, OrderParameters, AdvancedOrder, CriteriaResolver,
    FulfillmentComponent, AdditionalRecipient, BasicOrderParameters, ISeaport16Sweep
} from "../../src/nft/Seaport16.sol";

// Seaport 1.6's own types that the sweep subset does not use: what an order's signature is over, and the plain fulfillOrder.
struct OrderComponents {
    address offerer; address zone; OfferItem[] offer; ConsiderationItem[] consideration; OrderType orderType; uint256 startTime;
    uint256 endTime; bytes32 zoneHash; uint256 salt; bytes32 conduitKey; uint256 counter;
}
struct Order { OrderParameters parameters; bytes signature; }

interface ISeaport {
    function getOrderHash(OrderComponents calldata order) external view returns (bytes32);
    function getCounter(address offerer) external view returns (uint256);
    function information() external view returns (string memory version, bytes32 domainSeparator, address conduitController);
    function fulfillOrder(Order calldata order, bytes32 fulfillerConduitKey) external payable returns (bool);
}

interface ICurve {
    function buy(uint256 quoteIn, uint256 minTokensOut, address recipient) external payable returns (uint256);
}

/// Robinhood Chain (4663), forked: the real Seaport 1.6, real collections (StonkBrokers, Chain Mancers), the real Pons factory
/// and escrow. An order is signed here by a test key for a token moved to it on the fork — nothing is sent to the real chain.
/// Run against a local fork:  anvil --fork-url https://rpc.mainnet.chain.robinhood.com --port 8623 --code-size-limit 98304
///                            forge test --match-path "test/fork/NftSweep*" --fork-url http://127.0.0.1:8623
contract NftSweepForkTest is Test {
    ISeaport constant SEAPORT = ISeaport(0x0000000000000068F116a894984e2DB1123eB395);
    IERC721 constant STONK = IERC721(0x539CdD042c2f3d93EbC5BE7DfFf0c79F3B4fAbF0);
    IERC721 constant MANCERS = IERC721(0x797a2e030B7e49107C8F07bF0300Ea9caE88cA57);
    IPonsV2LaunchFactory constant FACTORY = IPonsV2LaunchFactory(0x7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e);

    ListingRegistry lr;
    NftRegistry nreg;
    NftLauncher launcher;
    NftSweepVault vault;
    address owner = makeAddr("owner");
    address treasury = makeAddr("treasury");
    address keeper = makeAddr("keeper");
    address creator = makeAddr("creator");
    address sweeper = makeAddr("sweeper");
    address seller;
    uint256 sellerKey;
    bool forked;

    function setUp() public {
        forked = block.chainid == 4663;
        if (!forked) return;
        (seller, sellerKey) = makeAddrAndKey("seller");
        vm.startPrank(owner);
        lr = new ListingRegistry(owner, treasury);
        lr.proposeRoles(keeper, makeAddr("purchaser"), makeAddr("attestor"));
        nreg = new NftRegistry(lr, IRandomnessBeacon(address(0xBEAC0)), address(SEAPORT));
        launcher = new NftLauncher(FACTORY, nreg);
        nreg.wire(address(launcher));
        lr.wireNft(address(nreg));
        vault = NftSweepVault(payable(nreg.vaultOf(nreg.listNft(4663, address(STONK), Policy.Raffle, "StonkBrokers", payable(0), 0))));
        vm.stopPrank();
    }

    /// Moves a real token to the test seller (on the fork only), approves Seaport, and signs a FULL_OPEN order for `price` wei paid
    /// to the seller, plus `fee` wei to `feeTo` as a second SIGNED consideration item (a marketplace fee) when `fee` is not zero.
    function _signed(IERC721 nft, uint256 tokenId, uint256 price, address feeTo, uint256 fee) internal returns (Order memory o) {
        address holder = nft.ownerOf(tokenId);
        if (holder != seller) {
            vm.prank(holder);
            nft.transferFrom(holder, seller, tokenId);
        }
        vm.prank(seller);
        nft.setApprovalForAll(address(SEAPORT), true);
        OfferItem[] memory offer = new OfferItem[](1);
        offer[0] = OfferItem(ItemType.ERC721, address(nft), tokenId, 1, 1);
        ConsiderationItem[] memory cons = new ConsiderationItem[](fee == 0 ? 1 : 2);
        cons[0] = ConsiderationItem(ItemType.NATIVE, address(0), 0, price, price, payable(seller));
        if (fee != 0) cons[1] = ConsiderationItem(ItemType.NATIVE, address(0), 0, fee, fee, payable(feeTo));
        OrderComponents memory c = OrderComponents(seller, address(0), offer, cons, OrderType.FULL_OPEN, block.timestamp - 1,
            block.timestamp + 1 days, bytes32(0), uint256(keccak256(abi.encode(nft, tokenId))), bytes32(0), SEAPORT.getCounter(seller));
        (, bytes32 domain,) = SEAPORT.information();
        (uint8 v, bytes32 r, bytes32 s_) = vm.sign(sellerKey, keccak256(abi.encodePacked("\x19\x01", domain, SEAPORT.getOrderHash(c))));
        o.parameters = OrderParameters(seller, address(0), offer, cons, OrderType.FULL_OPEN, c.startTime, c.endTime, bytes32(0),
            c.salt, bytes32(0), cons.length);
        o.signature = abi.encodePacked(r, s_, v);
    }

    function _order(IERC721 nft, uint256 tokenId, uint256 price) internal returns (Order memory) {
        return _signed(nft, tokenId, price, address(0), 0);
    }

    function _advanced(Order memory o) internal pure returns (AdvancedOrder memory a) {
        a.parameters = o.parameters;
        a.numerator = 1;
        a.denominator = 1;
        a.signature = o.signature;
    }

    /// fulfillAvailableAdvancedOrders for `orders`, bought into the vault: OpenSea's sweep call.
    function _available(Order[] memory orders) internal view returns (bytes memory) {
        AdvancedOrder[] memory a = new AdvancedOrder[](orders.length);
        for (uint256 i = 0; i < orders.length; ++i) a[i] = _advanced(orders[i]);
        return abi.encodeCall(ISeaport16Sweep.fulfillAvailableAdvancedOrders, (a, new CriteriaResolver[](0),
            _offerFulfillments(orders.length), _considerationFulfillments(orders), bytes32(0), address(vault), orders.length));
    }

    /// Each order's one offer item, in a fulfillment of its own, as OpenSea's own sweep builds it.
    function _offerFulfillments(uint256 n) internal pure returns (FulfillmentComponent[][] memory f) {
        f = new FulfillmentComponent[][](n);
        for (uint256 i = 0; i < n; ++i) {
            f[i] = new FulfillmentComponent[](1);
            f[i][0] = FulfillmentComponent(i, 0);
        }
    }

    /// Seaport 1.6 requires every consideration item to be named in some fulfillment: one component per item here.
    function _considerationFulfillments(Order[] memory orders) internal pure returns (FulfillmentComponent[][] memory f) {
        uint256 n;
        for (uint256 i = 0; i < orders.length; ++i) n += orders[i].parameters.consideration.length;
        f = new FulfillmentComponent[][](n);
        uint256 k;
        for (uint256 i = 0; i < orders.length; ++i) {
            for (uint256 j = 0; j < orders[i].parameters.consideration.length; ++j) {
                f[k] = new FulfillmentComponent[](1);
                f[k++][0] = FulfillmentComponent(i, j);
            }
        }
    }

    function _one(Order memory o) internal pure returns (Order[] memory r) {
        r = new Order[](1);
        r[0] = o;
    }

    /// The same signed order as Seaport's basic parameters (its first consideration item is the offerer's, the rest are signed
    /// additional recipients).
    function _basic(Order memory o) internal pure returns (BasicOrderParameters memory p) {
        OrderParameters memory q = o.parameters;
        p.considerationAmount = q.consideration[0].startAmount;
        p.offerer = payable(q.offerer);
        p.offerToken = q.offer[0].token;
        p.offerIdentifier = q.offer[0].identifierOrCriteria;
        p.offerAmount = 1;
        p.basicOrderType = BasicOrderType.ETH_TO_ERC721_FULL_OPEN;
        p.startTime = q.startTime;
        p.endTime = q.endTime;
        p.salt = q.salt;
        p.totalOriginalAdditionalRecipients = q.consideration.length - 1;
        p.additionalRecipients = new AdditionalRecipient[](q.consideration.length - 1);
        for (uint256 j = 1; j < q.consideration.length; ++j) {
            p.additionalRecipients[j - 1] = AdditionalRecipient(q.consideration[j].startAmount, q.consideration[j].recipient);
        }
        p.signature = o.signature;
    }

    function _ids1(uint256 a) internal pure returns (uint256[] memory r) { r = new uint256[](1); r[0] = a; }

    function _launchAndFund(uint256 credit) internal returns (address coin, address router) {
        NftLauncher.LaunchParams memory p;
        p.name = "Stonk Floor"; p.symbol = "SFLOOR"; p.salt = keccak256("etch-nft-fork"); p.listingId = 0;
        uint256 fee = FACTORY.launchFee();
        vm.deal(creator, fee + 1 ether);
        address curve;
        vm.prank(creator);
        (coin, curve, router) = launcher.launch{value: fee}(p);
        // real trading fees, then a credit through Pons's own escrow (in Pons's record, so attributed), then a permissionless harvest
        vm.warp(block.timestamp + 60);
        address trader = makeAddr("trader");
        vm.deal(trader, 2 ether);
        vm.prank(trader);
        ICurve(curve).buy{value: 1 ether}(1 ether, 0, trader);
        IPonsV2FeeEscrow escrow = IPonsV2FeeEscrow(FACTORY.feeEscrow());
        vm.deal(address(this), credit);
        escrow.credit{value: credit}(router);
        NftFeeRouter(payable(router)).harvest();
    }

    function test_theForkIsRobinhoodChainWithSeaport16AndRealCollections() public view {
        if (!forked) return;
        (string memory version,,) = SEAPORT.information();
        assertEq(version, "1.6");
        assertGt(address(STONK).code.length, 0);
        assertGt(address(MANCERS).code.length, 0);
        assertTrue(nreg.isSweepTarget(address(SEAPORT)));
    }

    /// The real Seaport 1.6 fills a real signed order for a real StonkBroker through the vault's decoder: the vault checked the
    /// order, re-encoded it, and Seaport accepted the re-encoding (the order hash and signature still verify).
    function test_aRealSeaportOrderForARealStonkBrokerIsSweptAndChecked() public {
        if (!forked) return;
        (address coin,) = _launchAndFund(1 ether);
        assertGt(vault.contributedBy(coin), 0.8 ether, "80% of the escrow's payment reached the vault, attributed to the coin");
        vm.prank(keeper);
        vault.postCap(0.05 ether, "fork: a made-up floor");
        Order memory o = _order(STONK, 100, 0.04 ether);
        uint256 before = address(vault).balance;
        uint256 sellerBefore = seller.balance;
        vm.prank(sweeper);
        (uint256 bought, uint256 spent) = vault.sweep(address(SEAPORT), _available(_one(o)), _ids1(100), 1);
        assertEq(bought, 1);
        assertEq(spent, 0.04 ether, "the vault sent the order's price, no more");
        assertEq(STONK.ownerOf(100), address(vault));
        assertEq(address(vault).balance, before - 0.04 ether);
        assertEq(seller.balance - sellerBefore, 0.04 ether);
        assertEq(vault.acquisition(0).fundedBy, coin);
        assertEq(vault.acquisition(0).tokenId, 100);
    }

    /// Two real orders, one with a signed marketplace fee, in one fulfillAvailableAdvancedOrders call.
    function test_aRealTwoOrderSweepWithASignedFee() public {
        if (!forked) return;
        _launchAndFund(1 ether);
        vm.prank(keeper);
        vault.postCap(0.05 ether, "fork");
        address fee = makeAddr("marketplace fee");
        Order[] memory orders = new Order[](2);
        orders[0] = _order(STONK, 100, 0.04 ether);
        orders[1] = _signed(STONK, 101, 0.03 ether, fee, 0.002 ether);
        uint256[] memory ids = new uint256[](2);
        ids[0] = 100; ids[1] = 101;
        vm.prank(sweeper);
        (uint256 bought, uint256 spent) = vault.sweep(address(SEAPORT), _available(orders), ids, 2);
        assertEq(bought, 2);
        assertEq(spent, 0.072 ether);
        assertEq(fee.balance, 0.002 ether);
        assertEq(STONK.ownerOf(101), address(vault));
    }

    /// The same real order, as Seaport's basic parameters, through both basic selectors.
    function test_aRealBasicOrderThroughBothBasicSelectors() public {
        if (!forked) return;
        _launchAndFund(1 ether);
        vm.prank(keeper);
        vault.postCap(0.05 ether, "fork");
        vm.prank(sweeper);
        vault.sweep(address(SEAPORT), abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder, (_basic(_order(STONK, 100, 0.04 ether)))),
            _ids1(100), 1);
        vm.prank(sweeper);
        (uint256 bought, uint256 spent) = vault.sweep(address(SEAPORT),
            abi.encodeCall(ISeaport16Sweep.fulfillBasicOrder_efficient_6GL6yc, (_basic(_signed(STONK, 101, 0.03 ether, makeAddr("fee"), 0.001 ether)))),
            _ids1(101), 1);
        assertEq(bought, 1);
        assertEq(spent, 0.031 ether);
        assertEq(STONK.ownerOf(100), address(vault));
        assertEq(STONK.ownerOf(101), address(vault));
    }

    /// A real order with a tip appended by the sweeper: real Seaport would pay it (the signature does not cover it); the vault
    /// refuses it before any ETH moves.
    function test_aRealOrderWithAnAppendedTipIsRefused() public {
        if (!forked) return;
        _launchAndFund(1 ether);
        vm.prank(keeper);
        vault.postCap(0.05 ether, "fork");
        Order memory o = _order(STONK, 100, 0.03 ether);
        ConsiderationItem[] memory c = new ConsiderationItem[](2);
        c[0] = o.parameters.consideration[0];
        c[1] = ConsiderationItem(ItemType.NATIVE, address(0), 0, 0.02 ether, 0.02 ether, payable(sweeper));
        o.parameters.consideration = c;                         // totalOriginalConsiderationItems stays 1: a tip
        bytes memory data = _available(_one(o));
        vm.prank(sweeper);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.Tip.selector, 0));
        vault.sweep(address(SEAPORT), data, _ids1(100), 1);
        // the attack is real: the same tipped order, sent to Seaport directly, is filled and the tip paid
        AdvancedOrder[] memory direct = new AdvancedOrder[](1);
        direct[0] = _advanced(o);
        FulfillmentComponent[][] memory cf = new FulfillmentComponent[][](2);
        cf[0] = new FulfillmentComponent[](1); cf[0][0] = FulfillmentComponent(0, 0);
        cf[1] = new FulfillmentComponent[](1); cf[1][0] = FulfillmentComponent(0, 1);
        address buyer = makeAddr("a direct buyer");
        vm.deal(buyer, 0.05 ether);
        uint256 sweeperBefore = sweeper.balance;
        vm.prank(buyer);
        ISeaport16Sweep(address(SEAPORT)).fulfillAvailableAdvancedOrders{value: 0.05 ether}(direct, new CriteriaResolver[](0),
            _offerFulfillments(1), cf, bytes32(0), buyer, 1);
        assertEq(sweeper.balance - sweeperBefore, 0.02 ether, "real Seaport pays a fulfiller's tip");
        assertEq(STONK.ownerOf(100), buyer);
        // and the plain fulfillOrder the vault used to forward is no longer accepted at all
        Order memory plain = _order(STONK, 100, 0.03 ether);
        vm.prank(sweeper);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.UnsupportedCall.selector, ISeaport.fulfillOrder.selector));
        vault.sweep(address(SEAPORT), abi.encodeCall(ISeaport.fulfillOrder, (plain, bytes32(0))), _ids1(100), 1);
        assertEq(STONK.ownerOf(100), seller);
    }

    function test_aRealOrderAboveTheCapCannotBeFilled() public {
        if (!forked) return;
        _launchAndFund(1 ether);
        vm.prank(keeper);
        vault.postCap(0.05 ether, "fork");
        bytes memory data = _available(_one(_order(STONK, 100, 0.06 ether)));
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.OrderOverCap.selector, 0, 0.06 ether, 0.05 ether));
        vault.sweep(address(SEAPORT), data, _ids1(100), 1);
        assertEq(STONK.ownerOf(100), seller);
    }

    /// A real order for a DIFFERENT collection's token: refused by the decoder before Seaport is called.
    function test_aRealOrderForAnotherCollectionIsRefused() public {
        if (!forked) return;
        _launchAndFund(1 ether);
        vm.prank(keeper);
        vault.postCap(0.05 ether, "fork");
        bytes memory data = _available(_one(_order(MANCERS, 1, 0.01 ether)));
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        vault.sweep(address(SEAPORT), data, _ids1(1), 1);
    }

    /// A real order whose token is not among the candidates: refused by the decoder.
    function test_aRealOrderForAnUnlistedIdIsRefused() public {
        if (!forked) return;
        _launchAndFund(1 ether);
        vm.prank(keeper);
        vault.postCap(0.05 ether, "fork");
        bytes memory data = _available(_one(_order(STONK, 100, 0.01 ether)));
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.WrongOffer.selector, 0));
        vault.sweep(address(SEAPORT), data, _ids1(3), 1);
    }

    function test_aBurnListingOnARealCollectionBurnsToDead() public {
        if (!forked) return;
        vm.prank(owner);
        NftSweepVault bv = NftSweepVault(payable(nreg.vaultOf(nreg.listNft(4663, address(STONK), Policy.Burn, "StonkBrokers burn", payable(0), 0))));
        NftLauncher.LaunchParams memory p;
        p.name = "Stonk Burn"; p.symbol = "SBURN"; p.salt = keccak256("etch-nft-fork-burn"); p.listingId = 1;
        uint256 fee = FACTORY.launchFee();
        vm.deal(creator, fee);
        vm.prank(creator);
        (, , address router) = launcher.launch{value: fee}(p);
        IPonsV2FeeEscrow escrow = IPonsV2FeeEscrow(FACTORY.feeEscrow());
        vm.deal(address(this), 1 ether);
        escrow.credit{value: 1 ether}(router);
        NftFeeRouter(payable(router)).harvest();
        vm.prank(keeper);
        bv.postCap(0.05 ether, "fork");
        NftSweepVault saved = vault;
        vault = bv;                                            // _available names the vault as recipient
        bytes memory data = _available(_one(_order(STONK, 100, 0.03 ether)));
        vault = saved;
        vm.prank(sweeper);
        bv.sweep(address(SEAPORT), data, _ids1(100), 1);
        bv.burn(0);
        assertEq(STONK.ownerOf(100), bv.DEAD());
    }
}
