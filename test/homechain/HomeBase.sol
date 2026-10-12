// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {MockBeacon} from "../utils/Mocks.sol";
import {MockNft} from "../nft/NftMocks.sol";
import {Merkle} from "../utils/Merkle.sol";
import {Clock} from "../utils/Clock.sol";
import {MockPunks, MockSeaport, MockInbox, MockOutbox, MockGatewayRouter} from "./HomeMocks.sol";
import {HomeGuard} from "../../src/homechain/HomeGuard.sol";
import {HomeWrapper} from "../../src/homechain/HomeWrapper.sol";
import {HomeVaultBase} from "../../src/homechain/HomeVaultBase.sol";
import {HomeVaultRaffle} from "../../src/homechain/HomeVaultRaffle.sol";
import {HomeVaultBurn} from "../../src/homechain/HomeVaultBurn.sol";
import {HomeVaultYoyo} from "../../src/homechain/HomeVaultYoyo.sol";
import {HomeVaultWrap} from "../../src/homechain/HomeVaultWrap.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";
import {IHomeGuard, IArbInbox, IArbOutbox, IL1GatewayRouter} from "../../src/homechain/IHomeChain.sol";
import {
    ItemType, OrderType, BasicOrderType, OfferItem, ConsiderationItem, OrderParameters, AdvancedOrder, CriteriaResolver,
    FulfillmentComponent, BasicOrderParameters, ISeaport16Sweep
} from "../../src/nft/Seaport16.sol";

/// The home chain (Ethereum, 12-second blocks), forked in miniature: a mock ERC-721 floor and a mock CryptoPunksMarket, a mock
/// Seaport that fills honestly and validates the vault's own orders, a mock Inbox/Outbox pair, one guard owned by `timelock`, a
/// wrapper for each collection, and one vault per policy: raffle/burn/yoyo/wrap for the ERC-721, yoyo/raffle/wrap for the punks.
/// `escrow` and `receiver` are the 4663 addresses the vaults talk to; here they are only addresses.
abstract contract HomeBase is Test {
    MockBeacon beacon;
    MockNft nft;
    MockPunks punks;
    MockSeaport seaport;
    MockInbox inbox;
    MockOutbox outbox;
    MockGatewayRouter gateway;
    HomeGuard guard;
    HomeWrapper wNft;
    HomeWrapper wPunk;
    HomeVaultRaffle raffle;
    HomeVaultBurn burnVault;
    HomeVaultYoyo yoyo;
    HomeVaultWrap wrapVault;
    HomeVaultYoyo punkYoyo;
    HomeVaultRaffle punkRaffle;
    HomeVaultWrap punkWrap;

    address timelock = makeAddr("timelock (the owner)");
    address keeper = makeAddr("keeper");
    address escrow = makeAddr("FeeEscrow on 4663");
    address receiver = makeAddr("HomeReceiver on 4663");
    address seller = makeAddr("seller");
    address trigger = makeAddr("trigger (anyone)");
    address solver = makeAddr("fast bridge solver");
    address stranger = makeAddr("stranger");
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address carol = makeAddr("carol");

    uint256 constant CAP = 5 ether;
    uint256 constant REWARD = 0.01 ether;
    uint256 constant BUFFER = 0.02 ether;
    uint256 constant MARKUP = 12_000;
    uint256 constant TTL = 30 days;
    uint256 constant PARENT_SECONDS = 12;

    function setUp() public virtual {
        vm.warp(1_790_000_000);
        vm.roll(26_000_000);
        vm.fee(20 gwei);
        beacon = new MockBeacon();
        nft = new MockNft();
        punks = new MockPunks();
        seaport = new MockSeaport();
        inbox = new MockInbox();
        outbox = new MockOutbox();
        gateway = new MockGatewayRouter();
        guard = new HomeGuard(timelock, 7 days, PARENT_SECONDS);
        // the wrappers first: a Wrap vault names its wrapper, and the wrapper names its vault, whose address is computed ahead.
        uint64 nonce = vm.getNonce(address(this));
        address wrapVaultAt = vm.computeCreateAddress(address(this), nonce + 5);      // wNft, wPunk, raffle, burn, yoyo, THEN wrapVault
        address punkWrapAt = vm.computeCreateAddress(address(this), nonce + 8);      // ... punkYoyo, punkRaffle, THEN punkWrap
        wNft = new HomeWrapper(beacon, address(nft), false, guard, wrapVaultAt, PARENT_SECONDS, "Wrapped Floor", "wFLOOR");
        wPunk = new HomeWrapper(beacon, address(punks), true, guard, punkWrapAt, PARENT_SECONDS, "Wrapped Punk", "wPUNK");
        raffle = new HomeVaultRaffle(_config(address(nft), false));
        burnVault = new HomeVaultBurn(_config(address(nft), false));
        yoyo = new HomeVaultYoyo(_config(address(nft), false), MARKUP, TTL);
        wrapVault = new HomeVaultWrap(_config(address(nft), false), address(wNft), IL1GatewayRouter(address(gateway)));
        punkYoyo = new HomeVaultYoyo(_config(address(punks), true), MARKUP, TTL);
        punkRaffle = new HomeVaultRaffle(_config(address(punks), true));
        punkWrap = new HomeVaultWrap(_config(address(punks), true), address(wPunk), IL1GatewayRouter(address(gateway)));
        assertEq(address(wrapVault), wrapVaultAt, "the wrapper names its vault");
        assertEq(address(punkWrap), punkWrapAt);
        vm.startPrank(timelock);
        guard.register(address(nft), address(raffle));
        guard.register(address(nft), address(burnVault));
        guard.register(address(nft), address(yoyo));
        guard.register(address(nft), address(wrapVault));
        guard.register(address(nft), address(wNft));
        guard.register(address(punks), address(punkYoyo));
        guard.register(address(punks), address(punkRaffle));
        guard.register(address(punks), address(punkWrap));
        guard.register(address(punks), address(wPunk));
        // room for the tests: 3 items at stage 0 is tight; raise once (×4) so multi-item tests fit
        guard.raise(address(nft), 12, 40 ether, 10 ether);
        guard.raise(address(punks), 12, 40 ether, 10 ether);
        guard.raiseGlobal(48, 40 ether);
        guard.setLimits(6, 12, 40 ether);
        vm.stopPrank();
        vm.prank(seller);
        nft.setApprovalForAll(address(seaport), true);
        vm.deal(solver, 1_000 ether);
        vm.deal(address(outbox), 0);
    }

    function _config(address collection, bool isPunks) internal view returns (HomeVaultBase.Config memory c) {
        c.collection = collection;
        c.isPunks = isPunks;
        c.seaport = isPunks ? address(0) : address(seaport);
        c.beacon = IRandomnessBeacon(address(beacon));
        c.guard = IHomeGuard(address(guard));
        c.inbox = IArbInbox(address(inbox));
        c.outbox = IArbOutbox(address(outbox));
        c.escrow = escrow;
        c.receiver = receiver;
        c.owner = timelock;
        c.keeper = keeper;
        c.reward = REWARD;
        c.buffer = BUFFER;
        c.parentBlockSeconds = PARENT_SECONDS;
    }

    // ---- helpers -------------------------------------------------------------------------------------------------------------

    /// The native path: the Outbox executes the escrow's `fund(ticket)` message with `value`.
    function _fundNative(HomeVaultBase v, uint256 ticket, uint256 amount) internal {
        vm.deal(address(this), address(this).balance + amount);
        outbox.execute{value: amount}(escrow, address(v), abi.encodeCall(HomeVaultBase.fund, (ticket)));
    }

    /// The fast path: a solver's destination call.
    function _fundFast(HomeVaultBase v, uint256 ticket, uint256 amount) internal {
        vm.prank(solver);
        v.fund{value: amount}(ticket);
    }

    function _cap(HomeVaultBase v, uint256 capWei) internal {
        vm.prank(keeper);
        v.postCap(capWei, "https://example.invalid/floor");
    }

    mapping(uint256 id => address) internal askSeller;
    mapping(uint256 id => uint256) internal askPrice;

    /// Mints `ids` to the seller and notes an ask at `price` for the order builders.
    function _ask(uint256[] memory ids, uint256 price) internal {
        for (uint256 i = 0; i < ids.length; ++i) {
            nft.mint(seller, ids[i]);
            askSeller[ids[i]] = seller;
            askPrice[ids[i]] = price;
        }
    }

    /// Mints punk `id` to the seller and offers it to anyone at `price`.
    function _offerPunk(uint256 id, uint256 price) internal {
        punks.mint(seller, id);
        vm.prank(seller);
        punks.offerPunkForSale(id, price);
    }

    function _ids(uint256 a) internal pure returns (uint256[] memory r) { r = new uint256[](1); r[0] = a; }
    function _ids(uint256 a, uint256 b) internal pure returns (uint256[] memory r) { r = new uint256[](2); r[0] = a; r[1] = b; }
    function _ids(uint256 a, uint256 b, uint256 c) internal pure returns (uint256[] memory r) {
        r = new uint256[](3); r[0] = a; r[1] = b; r[2] = c;
    }

    function _orderFor(address collection, address offerer, uint256 id, uint256 price) internal pure returns (AdvancedOrder memory a) {
        OfferItem[] memory offer = new OfferItem[](1);
        offer[0] = OfferItem(ItemType.ERC721, collection, id, 1, 1);
        ConsiderationItem[] memory cons = new ConsiderationItem[](1);
        cons[0] = ConsiderationItem(ItemType.NATIVE, address(0), 0, price, price, payable(offerer));
        a.parameters = OrderParameters(offerer, address(0), offer, cons, OrderType.FULL_OPEN, 0, type(uint256).max, bytes32(0),
            id, bytes32(0), 1);
        a.numerator = 1;
        a.denominator = 1;
    }

    function _order(address offerer, uint256 id, uint256 price) internal view returns (AdvancedOrder memory) {
        return _orderFor(address(nft), offerer, id, price);
    }

    function _one(AdvancedOrder memory a) internal pure returns (AdvancedOrder[] memory orders) {
        orders = new AdvancedOrder[](1);
        orders[0] = a;
    }

    function _available(AdvancedOrder[] memory orders, address recipient) internal pure returns (bytes memory) {
        return abi.encodeCall(ISeaport16Sweep.fulfillAvailableAdvancedOrders, (orders, new CriteriaResolver[](0),
            new FulfillmentComponent[][](0), new FulfillmentComponent[][](0), bytes32(0), recipient, orders.length));
    }

    function _buyCall(uint256[] memory ids, address recipient) internal view returns (bytes memory) {
        AdvancedOrder[] memory orders = new AdvancedOrder[](ids.length);
        for (uint256 i = 0; i < ids.length; ++i) orders[i] = _order(askSeller[ids[i]], ids[i], askPrice[ids[i]]);
        return _available(orders, recipient);
    }

    function _basicParams(uint256 id) internal view returns (BasicOrderParameters memory p) {
        p.offerer = payable(askSeller[id]);
        p.offerToken = address(nft);
        p.offerIdentifier = id;
        p.offerAmount = 1;
        p.considerationAmount = askPrice[id];
        p.basicOrderType = BasicOrderType.ETH_TO_ERC721_FULL_OPEN;
        p.endTime = type(uint256).max;
        p.salt = id;
    }

    /// A funded, capped vault buys `ids` at `price` each through Seaport, by `trigger`. Returns the first new acquisition id.
    function _buy(HomeVaultBase v, uint256[] memory ids, uint256 price) internal returns (uint256 first) {
        first = v.acquisitionCount();
        _ask(ids, price);
        vm.prank(trigger, trigger);
        v.buySeaport(_buyCall(ids, address(v)), ids, ids.length);
    }

    function _buyPunk(HomeVaultBase v, uint256 id, uint256 price) internal returns (uint256 aid) {
        _offerPunk(id, price);
        vm.prank(trigger, trigger);
        aid = v.buyPunk(id);
    }

    // a three-holder ticket list: alice [0,50), bob [50,80), carol [80,100)
    function _tickets(HomeVaultRaffle v) internal view returns (bytes32[] memory leaves) {
        leaves = new bytes32[](3);
        leaves[0] = v.ticketLeaf(alice, 0, 50);
        leaves[1] = v.ticketLeaf(bob, 50, 80);
        leaves[2] = v.ticketLeaf(carol, 80, 100);
    }

    function _holderOf(uint256 ticket) internal view returns (address who, uint256 first, uint256 end, uint256 index) {
        if (ticket < 50) return (alice, 0, 50, 0);
        if (ticket < 80) return (bob, 50, 80, 1);
        return (carol, 80, 100, 2);
    }

    /// Opens, waits out the challenge, pins, resolves (with `randomness`) and claims. Returns the winner.
    function _raffle(HomeVaultRaffle v, uint256 acqId, bytes32 randomness) internal returns (address who) {
        bytes32[] memory leaves = _tickets(v);
        vm.prank(keeper);
        v.openDraw(acqId, Merkle.root(leaves), 100, "ipfs://tickets");
        Clock.skip(15 minutes);
        v.requestDraw(acqId);
        Clock.skip(2 minutes);
        v.resolve(acqId, abi.encode(randomness));
        uint256 first; uint256 end; uint256 index;
        (who, first, end, index) = _holderOf(v.draw(acqId).winningTicket);
        vm.prank(stranger);
        v.claimWin(acqId, who, first, end, Merkle.proof(leaves, index));
    }

    receive() external payable {}
}
